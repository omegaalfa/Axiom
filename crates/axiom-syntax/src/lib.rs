//! Incremental PHP parsing, highlighting, and local symbol extraction.

use std::{collections::HashSet, fmt, ops::Range};

use tree_sitter::{InputEdit, Parser, Point, Query, QueryCursor, StreamingIterator, Tree};

const RUSTSTORM_HIGHLIGHTS_QUERY: &str = r#"
(class_declaration name: (name) @type)
(interface_declaration name: (name) @type)
(trait_declaration name: (name) @type)
(enum_declaration name: (name) @type)
(attribute_group) @attribute
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HighlightKind {
    Keyword,
    String,
    Comment,
    Number,
    Type,
    Function,
    Method,
    Property,
    Variable,
    Constant,
    Namespace,
    Operator,
    Punctuation,
    Attribute,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightSpan {
    pub start_byte: usize,
    pub end_byte: usize,
    pub kind: HighlightKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Namespace,
    Class,
    Interface,
    Trait,
    Enum,
    Function,
    Method,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxSymbol {
    pub kind: SymbolKind,
    pub name: String,
    pub range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxToken {
    pub text: String,
    pub kind: String,
    pub range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxDiagnostic {
    pub range: Range<usize>,
    pub message: String,
}

#[derive(Debug)]
pub enum SyntaxError {
    Language(tree_sitter::LanguageError),
    Query(tree_sitter::QueryError),
    ParseCancelled,
    InvalidEdit(Range<usize>),
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Language(error) => write!(f, "failed to load PHP grammar: {error}"),
            Self::Query(error) => write!(f, "failed to compile PHP highlight query: {error}"),
            Self::ParseCancelled => f.write_str("PHP parse was cancelled"),
            Self::InvalidEdit(range) => write!(f, "invalid UTF-8 edit range: {range:?}"),
        }
    }
}

impl std::error::Error for SyntaxError {}

pub struct PhpSyntax {
    parser: Parser,
    query: Query,
    tree: Tree,
    text: String,
    highlights: Vec<HighlightSpan>,
    highlight_prefix_max_end: Vec<usize>,
    symbols: Vec<SyntaxSymbol>,
}

impl PhpSyntax {
    pub fn parse(text: impl Into<String>) -> Result<Self, SyntaxError> {
        let text = text.into();
        let language = tree_sitter_php::LANGUAGE_PHP.into();
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(SyntaxError::Language)?;
        let highlight_query = format!(
            "{}\n{}",
            tree_sitter_php::HIGHLIGHTS_QUERY,
            RUSTSTORM_HIGHLIGHTS_QUERY
        );
        let query = Query::new(&language, &highlight_query).map_err(SyntaxError::Query)?;
        let tree = parser
            .parse(text.as_bytes(), None)
            .ok_or(SyntaxError::ParseCancelled)?;
        let mut syntax = Self {
            parser,
            query,
            tree,
            text,
            highlights: Vec::new(),
            highlight_prefix_max_end: Vec::new(),
            symbols: Vec::new(),
        };
        syntax.refresh_derived_data();
        Ok(syntax)
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// Returns the smallest named Tree-sitter token containing a UTF-8 byte
    /// offset. This is intentionally syntax-only and never performs an LSP
    /// request, making it suitable for Ctrl-hover feedback.
    pub fn token_at_byte(&self, offset: usize) -> Option<SyntaxToken> {
        let offset = offset.min(self.text.len());
        let probe = if offset == self.text.len() {
            offset.saturating_sub(1)
        } else {
            offset
        };
        if !self.text.is_char_boundary(probe) {
            return None;
        }
        let mut node = self
            .tree
            .root_node()
            .descendant_for_byte_range(probe, probe + 1)?;
        while !node.is_named() {
            node = node.parent()?;
        }
        let range = node.byte_range();
        let text = node.utf8_text(self.text.as_bytes()).ok()?.to_owned();
        Some(SyntaxToken {
            text,
            kind: node.kind().to_owned(),
            range,
        })
    }

    pub fn diagnostics(&self) -> Vec<SyntaxDiagnostic> {
        let mut result = Vec::new();
        let mut stack = vec![self.tree.root_node()];
        while let Some(node) = stack.pop() {
            if node.is_error() || node.is_missing() {
                let range = node.byte_range();
                if range.start < self.text.len() || node.is_missing() {
                    result.push(SyntaxDiagnostic {
                        range,
                        message: if node.is_missing() {
                            format!("Expected {}", node.kind())
                        } else {
                            "Syntax error".into()
                        },
                    });
                }
            }
            let mut cursor = node.walk();
            stack.extend(node.children(&mut cursor));
        }
        result.sort_by_key(|diagnostic| diagnostic.range.start);
        result.dedup_by(|a, b| a.range == b.range);
        result
    }

    pub fn has_errors(&self) -> bool {
        self.tree.root_node().has_error()
    }

    pub fn highlights(&self) -> &[HighlightSpan] {
        &self.highlights
    }

    pub fn highlights_in(&self, range: Range<usize>) -> impl Iterator<Item = &HighlightSpan> {
        let start = self
            .highlight_prefix_max_end
            .partition_point(|maximum_end| *maximum_end <= range.start);
        let end = self
            .highlights
            .partition_point(|span| span.start_byte < range.end);
        self.highlights[start.min(end)..end]
            .iter()
            .filter(move |span| span.end_byte > range.start && span.start_byte < range.end)
    }

    pub fn symbols(&self) -> &[SyntaxSymbol] {
        &self.symbols
    }

    pub fn apply_edit(
        &mut self,
        old_range: Range<usize>,
        replacement: &str,
    ) -> Result<(), SyntaxError> {
        if old_range.start > old_range.end
            || old_range.end > self.text.len()
            || !self.text.is_char_boundary(old_range.start)
            || !self.text.is_char_boundary(old_range.end)
        {
            return Err(SyntaxError::InvalidEdit(old_range));
        }

        let start_position = point_at(&self.text, old_range.start);
        let old_end_position = point_at(&self.text, old_range.end);
        let new_end_position = advance_point(start_position, replacement);
        let new_end_byte = old_range.start + replacement.len();
        self.tree.edit(&InputEdit {
            start_byte: old_range.start,
            old_end_byte: old_range.end,
            new_end_byte,
            start_position,
            old_end_position,
            new_end_position,
        });
        self.text.replace_range(old_range, replacement);
        self.tree = self
            .parser
            .parse(self.text.as_bytes(), Some(&self.tree))
            .ok_or(SyntaxError::ParseCancelled)?;
        self.refresh_derived_data();
        Ok(())
    }

    pub fn update_text(&mut self, new_text: &str) -> Result<(), SyntaxError> {
        if self.text == new_text {
            return Ok(());
        }
        let prefix = self
            .text
            .chars()
            .zip(new_text.chars())
            .take_while(|(old, new)| old == new)
            .map(|(ch, _)| ch.len_utf8())
            .sum::<usize>();
        let old_tail = &self.text[prefix..];
        let new_tail = &new_text[prefix..];
        let suffix = old_tail
            .chars()
            .rev()
            .zip(new_tail.chars().rev())
            .take_while(|(old, new)| old == new)
            .map(|(ch, _)| ch.len_utf8())
            .sum::<usize>();
        let old_end = self.text.len() - suffix;
        let new_end = new_text.len() - suffix;
        self.apply_edit(prefix..old_end, &new_text[prefix..new_end])
    }

    fn refresh_derived_data(&mut self) {
        self.highlights = collect_highlights(&self.query, &self.tree, &self.text);
        self.highlight_prefix_max_end = self
            .highlights
            .iter()
            .scan(0, |maximum, span| {
                *maximum = (*maximum).max(span.end_byte);
                Some(*maximum)
            })
            .collect();
        self.symbols = collect_symbols(&self.tree, &self.text);
    }
}

fn point_at(text: &str, byte: usize) -> Point {
    let prefix = &text[..byte];
    let row = prefix.bytes().filter(|byte| *byte == b'\n').count();
    let column = prefix
        .rfind('\n')
        .map_or(prefix.len(), |index| prefix.len() - index - 1);
    Point::new(row, column)
}

fn advance_point(start: Point, inserted: &str) -> Point {
    let newline_count = inserted.bytes().filter(|byte| *byte == b'\n').count();
    if newline_count == 0 {
        Point::new(start.row, start.column + inserted.len())
    } else {
        Point::new(
            start.row + newline_count,
            inserted.rsplit('\n').next().map_or(0, str::len),
        )
    }
}

fn collect_highlights(query: &Query, tree: &Tree, text: &str) -> Vec<HighlightSpan> {
    let capture_names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), text.as_bytes());
    let mut spans = Vec::new();
    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            if let Some(kind) = highlight_kind(capture_names[capture.index as usize]) {
                spans.push(HighlightSpan {
                    start_byte: capture.node.start_byte(),
                    end_byte: capture.node.end_byte(),
                    kind,
                });
            }
        }
    }
    spans.sort_by_key(|span| (span.start_byte, span.end_byte));
    spans.dedup();
    spans
}

fn highlight_kind(capture: &str) -> Option<HighlightKind> {
    if capture == "function.method" || capture == "constructor" {
        return Some(HighlightKind::Method);
    }
    let base = capture.split('.').next().unwrap_or(capture);
    Some(match base {
        "keyword" | "tag" => HighlightKind::Keyword,
        "string" => HighlightKind::String,
        "comment" => HighlightKind::Comment,
        "number" => HighlightKind::Number,
        "type" => HighlightKind::Type,
        "function" => HighlightKind::Function,
        "method" => HighlightKind::Method,
        "property" => HighlightKind::Property,
        "variable" => HighlightKind::Variable,
        "constant" => HighlightKind::Constant,
        "module" => HighlightKind::Namespace,
        "operator" => HighlightKind::Operator,
        "punctuation" => HighlightKind::Punctuation,
        "attribute" => HighlightKind::Attribute,
        _ => return None,
    })
}

fn collect_symbols(tree: &Tree, text: &str) -> Vec<SyntaxSymbol> {
    let mut symbols = Vec::new();
    let mut stack = vec![tree.root_node()];
    let mut seen = HashSet::new();
    while let Some(node) = stack.pop() {
        if let Some((kind, name_node)) = symbol_for_node(node) {
            let range = name_node.byte_range();
            if let Ok(name) = name_node.utf8_text(text.as_bytes()) {
                if seen.insert((kind as u8, range.start, range.end)) {
                    symbols.push(SyntaxSymbol {
                        kind,
                        name: name.to_owned(),
                        range,
                    });
                }
            }
        }
        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor));
    }
    symbols.sort_by_key(|symbol| symbol.range.start);
    symbols
}

fn symbol_for_node(node: tree_sitter::Node<'_>) -> Option<(SymbolKind, tree_sitter::Node<'_>)> {
    let kind = match node.kind() {
        "namespace_definition" => SymbolKind::Namespace,
        "class_declaration" => SymbolKind::Class,
        "interface_declaration" => SymbolKind::Interface,
        "trait_declaration" => SymbolKind::Trait,
        "enum_declaration" => SymbolKind::Enum,
        "function_definition" => SymbolKind::Function,
        "method_declaration" => SymbolKind::Method,
        _ => return None,
    };
    node.child_by_field_name("name").map(|name| (kind, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_clean(source: &str) -> PhpSyntax {
        let syntax = PhpSyntax::parse(source).unwrap();
        assert!(
            !syntax.has_errors(),
            "{}",
            syntax.tree().root_node().to_sexp()
        );
        syntax
    }

    #[test]
    fn parses_empty_and_simple_php() {
        assert_clean("<?php\n");
        assert_clean("<?php class User {} function helper(): void {}\n");
    }

    #[test]
    fn token_lookup_returns_smallest_named_node() {
        let syntax = assert_clean("<?php class UserService {}\n");
        let offset = syntax.text().find("UserService").unwrap() + 2;
        let token = syntax.token_at_byte(offset).unwrap();
        assert_eq!(token.text, "UserService");
        assert_eq!(token.kind, "name");
    }

    #[test]
    fn parses_representative_php() {
        assert_clean(
            "<?php\nnamespace App\\Service;\nuse App\\Repository\\UserRepository;\nfinal class UserService { public function __construct(private readonly UserRepository $repository) {} public function find(string $email): ?User { return $this->repository->findByEmail($email); } }\n",
        );
    }

    #[test]
    fn parses_modern_php() {
        assert_clean(
            "<?php\n#[Route('/users')]\nreadonly class Controller {}\nenum Status: string { case Active = 'active'; }\ninterface A {} interface B {} interface Contract {} trait Shared {} abstract class Base { protected const X = 1; private static string|int|null $value; public function both(A&B $value): A&B {} }\n$result = match ($status) { Status::Active => true, default => false, };\n",
        );
    }

    #[test]
    fn parses_php_mixed_with_html() {
        assert_clean(
            "<!DOCTYPE html><html><body><?php if ($user): ?><h1><?= htmlspecialchars($user->name) ?></h1><?php endif; ?></body></html>",
        );
    }

    #[test]
    fn tolerates_incomplete_code() {
        let syntax =
            PhpSyntax::parse("<?php final class UserService { public function find( $").unwrap();
        assert!(syntax.has_errors());
        assert_eq!(syntax.tree().root_node().kind(), "program");
    }

    #[test]
    fn highlights_required_categories_and_comments() {
        let source = "<?php final class User { private string $name = \"Gabriel\"; // line\n# hash\n/* block */\n/** doc */\npublic function find(int $id = 42): void {} }";
        let syntax = assert_clean(source);
        for kind in [
            HighlightKind::Keyword,
            HighlightKind::String,
            HighlightKind::Comment,
            HighlightKind::Number,
            HighlightKind::Type,
            HighlightKind::Method,
            HighlightKind::Variable,
        ] {
            assert!(
                syntax.highlights().iter().any(|span| span.kind == kind),
                "missing {kind:?}"
            );
        }
        let class_start = source.find("User").unwrap();
        assert!(syntax.highlights().iter().any(|span| {
            span.kind == HighlightKind::Type
                && span.start_byte == class_start
                && span.end_byte == class_start + "User".len()
        }));
    }

    #[test]
    fn highlights_attributes() {
        let syntax = assert_clean("<?php #[Route('/users')] final class Controller {}");
        assert!(
            syntax
                .highlights()
                .iter()
                .any(|span| span.kind == HighlightKind::Attribute)
        );
    }

    #[test]
    fn extracts_local_symbols() {
        let source = "<?php namespace App\\Service; interface Contract {} trait Shared {} enum Status { case Active; } final class UserService { public function find(): void {} } function helper(): void {}";
        let syntax = assert_clean(source);
        let found: Vec<_> = syntax
            .symbols()
            .iter()
            .map(|symbol| (symbol.kind, symbol.name.as_str()))
            .collect();
        assert!(found.contains(&(SymbolKind::Namespace, "App\\Service")));
        assert!(found.contains(&(SymbolKind::Interface, "Contract")));
        assert!(found.contains(&(SymbolKind::Trait, "Shared")));
        assert!(found.contains(&(SymbolKind::Enum, "Status")));
        assert!(found.contains(&(SymbolKind::Class, "UserService")));
        assert!(found.contains(&(SymbolKind::Method, "find")));
        assert!(found.contains(&(SymbolKind::Function, "helper")));
    }

    #[test]
    fn incremental_insert_matches_full_parse() {
        let initial = "<?php\n\nclass User\n{\n}\n";
        let mut syntax = PhpSyntax::parse(initial).unwrap();
        let offset = initial.find("class").unwrap();
        syntax.apply_edit(offset..offset, "final ").unwrap();
        let full = PhpSyntax::parse(syntax.text()).unwrap();
        assert_eq!(
            syntax.tree().root_node().to_sexp(),
            full.tree().root_node().to_sexp()
        );
        assert!(!syntax.has_errors());
    }

    #[test]
    fn incremental_delete_matches_full_parse() {
        let initial = "<?php final class User {}";
        let mut syntax = PhpSyntax::parse(initial).unwrap();
        let start = initial.find("final ").unwrap();
        syntax
            .apply_edit(start..start + "final ".len(), "")
            .unwrap();
        let full = PhpSyntax::parse(syntax.text()).unwrap();
        assert_eq!(
            syntax.tree().root_node().to_sexp(),
            full.tree().root_node().to_sexp()
        );
    }

    #[test]
    fn incremental_unicode_preserves_byte_positions() {
        let initial = "<?php $cidade = \"São Paulo\"; // Olá 👋\n";
        let mut syntax = PhpSyntax::parse(initial).unwrap();
        let start = initial.find("São").unwrap();
        syntax
            .apply_edit(start..start + "São".len(), "João")
            .unwrap();
        let full = PhpSyntax::parse(syntax.text()).unwrap();
        assert_eq!(
            syntax.tree().root_node().to_sexp(),
            full.tree().root_node().to_sexp()
        );
        assert!(syntax.text().contains("João Paulo"));
    }

    #[test]
    fn update_text_finds_a_unicode_safe_incremental_edit() {
        let mut syntax = PhpSyntax::parse("<?php $name = \"João\";").unwrap();
        syntax.update_text("<?php $name = \"José 👋\";").unwrap();
        let full = PhpSyntax::parse(syntax.text()).unwrap();
        assert_eq!(
            syntax.tree().root_node().to_sexp(),
            full.tree().root_node().to_sexp()
        );
    }

    #[test]
    fn visible_range_filters_cached_highlights() {
        let syntax = assert_clean("<?php $one = 1;\n$two = \"two\";\n");
        let second = syntax.text().find("$two").unwrap();
        assert!(
            syntax
                .highlights_in(second..syntax.text().len())
                .all(|span| span.end_byte > second)
        );
        for range in [0..5, 6..15, second..syntax.text().len()] {
            let indexed: Vec<_> = syntax.highlights_in(range.clone()).collect();
            let exhaustive: Vec<_> = syntax
                .highlights()
                .iter()
                .filter(|span| span.end_byte > range.start && span.start_byte < range.end)
                .collect();
            assert_eq!(indexed, exhaustive);
        }
    }
}
