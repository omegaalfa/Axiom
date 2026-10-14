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
    PropertyHook,
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
    pipe_operator_ranges: Vec<Range<usize>>,
}

/// Timings for one syntax update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SyntaxUpdateProfile {
    pub incremental: bool,
    pub edit_span_bytes: usize,
    pub parse_us: u128,
    pub derived_us: u128,
    pub total_us: u128,
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
        let (asymmetric_text, _) = normalize_asymmetric_visibility(&text);
        let (hooks_text, _) = normalize_property_hooks(&text, &asymmetric_text);
        let (parse_text, pipe_operator_ranges) = normalize_php85_pipe_operator(&text, &hooks_text);
        let tree = parser
            .parse(parse_text.as_bytes(), None)
            .ok_or(SyntaxError::ParseCancelled)?;
        let mut syntax = Self {
            parser,
            query,
            tree,
            text,
            highlights: Vec::new(),
            highlight_prefix_max_end: Vec::new(),
            symbols: Vec::new(),
            pipe_operator_ranges,
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

    /// Tree-sitter represents many PHP keywords as anonymous nodes. In those
    /// cases `token_at_byte` returns the containing construct; use its first
    /// named child to distinguish the keyword prefix from a real name.
    pub fn is_keyword_at_byte(&self, offset: usize) -> bool {
        let Some(token) = self.token_at_byte(offset) else {
            return false;
        };
        let Some(node) = self
            .tree
            .root_node()
            .descendant_for_byte_range(token.range.start, token.range.end)
        else {
            return false;
        };
        node.named_child(0)
            .is_some_and(|child| offset < child.start_byte())
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

    /// Byte ranges of PHP 8.5 pipe operators in the original, unnormalized
    /// source. Tree-sitter-php 0.24 parses these as same-width `||` tokens.
    pub fn pipe_operator_ranges(&self) -> Vec<Range<usize>> {
        self.pipe_operator_ranges.clone()
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
        self.apply_edit_profiled(old_range, replacement).map(|_| ())
    }

    pub fn apply_edit_profiled(
        &mut self,
        old_range: Range<usize>,
        replacement: &str,
    ) -> Result<SyntaxUpdateProfile, SyntaxError> {
        let total_started = std::time::Instant::now();
        // `old_range` is deliberately expressed in UTF-8 bytes, matching
        // DocumentEdit.  Keep this operation separate from `update_text`:
        // callers that already have the edit must not make us rediscover it
        // by comparing two complete documents.
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
        let edit_span_bytes = old_range.end.saturating_sub(old_range.start);
        self.text.replace_range(old_range, replacement);
        let parse_started = std::time::Instant::now();
        let (asymmetric_text, _) = normalize_asymmetric_visibility(&self.text);
        let (hooks_text, _) = normalize_property_hooks(&self.text, &asymmetric_text);
        let (parse_text, pipe_operator_ranges) =
            normalize_php85_pipe_operator(&self.text, &hooks_text);
        self.tree = self
            .parser
            .parse(parse_text.as_bytes(), Some(&self.tree))
            .ok_or(SyntaxError::ParseCancelled)?;
        self.pipe_operator_ranges = pipe_operator_ranges;
        let parse_us = parse_started.elapsed().as_micros();
        let derived_started = std::time::Instant::now();
        self.refresh_derived_data();
        let derived_us = derived_started.elapsed().as_micros();
        Ok(SyntaxUpdateProfile {
            incremental: true,
            edit_span_bytes,
            parse_us,
            derived_us,
            total_us: total_started.elapsed().as_micros(),
        })
    }

    pub fn update_text(&mut self, new_text: &str) -> Result<(), SyntaxError> {
        self.update_text_profiled(new_text).map(|_| ())
    }

    pub fn update_text_profiled(
        &mut self,
        new_text: &str,
    ) -> Result<SyntaxUpdateProfile, SyntaxError> {
        if self.text == new_text {
            return Ok(SyntaxUpdateProfile::default());
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
        let mut profile = self.apply_edit_profiled(prefix..old_end, &new_text[prefix..new_end])?;
        profile.incremental = false;
        Ok(profile)
    }

    fn refresh_derived_data(&mut self) {
        self.highlights = collect_highlights(&self.query, &self.tree, &self.text);
        let (asymmetric_text, asymmetric_visibility_ranges) =
            normalize_asymmetric_visibility(&self.text);
        self.highlights.extend(asymmetric_visibility_ranges.into_iter().map(|range| {
            HighlightSpan {
                start_byte: range.start,
                end_byte: range.end,
                kind: HighlightKind::Keyword,
            }
        }));
        let (_, property_hook_ranges) = normalize_property_hooks(&self.text, &asymmetric_text);
        self.highlights.extend(property_hook_ranges.into_iter().map(|range| {
            HighlightSpan {
                start_byte: range.start,
                end_byte: range.end,
                kind: HighlightKind::PropertyHook,
            }
        }));
        self.highlights.sort_by_key(|span| (span.start_byte, span.end_byte));
        self.highlights.dedup();
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

/// Tree-sitter-php 0.24 predates PHP 8.4 asymmetric property visibility.
/// Replace only the `(set)` suffix in PHP code with equal-length whitespace so
/// its existing repeated visibility-modifier production can parse the property
/// while every node range continues to address the original source.
fn normalize_asymmetric_visibility(text: &str) -> (String, Vec<Range<usize>>) {
    const VISIBILITIES: [&[u8]; 3] = [b"public", b"protected", b"private"];
    let source = text.as_bytes();
    let mut normalized = source.to_vec();
    let mut ranges = Vec::new();
    let mut offset = 0;
    let mut quote = None;
    let mut line_comment = false;
    let mut block_comment = false;
    let has_php_open_tag = source.windows(2).any(|window| window == b"<?");
    let mut in_php = !has_php_open_tag;

    while offset < source.len() {
        let byte = source[offset];
        if !in_php {
            if source[offset..].starts_with(b"<?") {
                in_php = true;
                offset += 2;
            } else {
                offset += 1;
            }
            continue;
        }
        if source[offset..].starts_with(b"?>") {
            in_php = false;
            offset += 2;
            continue;
        }
        if line_comment {
            if byte == b'\n' {
                line_comment = false;
            }
            offset += 1;
            continue;
        }
        if block_comment {
            if source[offset..].starts_with(b"*/") {
                block_comment = false;
                offset += 2;
            } else {
                offset += 1;
            }
            continue;
        }
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                offset = (offset + 2).min(source.len());
            } else if byte == delimiter {
                quote = None;
                offset += 1;
            } else {
                offset += 1;
            }
            continue;
        }
        if source[offset..].starts_with(b"//") || (byte == b'#' && !source[offset..].starts_with(b"#[")) {
            line_comment = true;
            offset += if byte == b'#' { 1 } else { 2 };
            continue;
        }
        if source[offset..].starts_with(b"/*") {
            block_comment = true;
            offset += 2;
            continue;
        }
        if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
            offset += 1;
            continue;
        }

        let visibility = VISIBILITIES.iter().find(|word| {
            source[offset..].starts_with(word)
                && (offset == 0
                    || !source[offset - 1].is_ascii_alphanumeric() && source[offset - 1] != b'_')
        });
        if let Some(visibility) = visibility {
            let suffix_start = offset + visibility.len();
            let suffix_end = suffix_start + b"(set)".len();
            let next_is_boundary = source
                .get(suffix_end)
                .is_none_or(|next| next.is_ascii_whitespace() || matches!(next, b'$' | b';' | b'='));
            if source.get(suffix_start..suffix_end) == Some(b"(set)")
                && next_is_boundary
            {
                normalized[suffix_start..suffix_end].fill(b' ');
                ranges.push(suffix_start..suffix_end);
                offset = suffix_end;
                continue;
            }
        }
        offset += 1;
    }
    (String::from_utf8(normalized).expect("replacing ASCII bytes preserves UTF-8"), ranges)
}

/// Tree-sitter-php 0.24 predates PHP 8.5's pipe token. Preserve all byte
/// offsets and expression structure by parsing `|>` as the same-width `||`;
/// the source text remains untouched for highlighting and semantic consumers.
fn normalize_php85_pipe_operator(
    original: &str,
    parse_text: &str,
) -> (String, Vec<Range<usize>>) {
    let source = original.as_bytes();
    let mut normalized = parse_text.as_bytes().to_vec();
    let mut offset = 0;
    let mut quote = None;
    let mut line_comment = false;
    let mut block_comment = false;
    let mut in_php = !source.windows(2).any(|window| window == b"<?");
    let mut ranges = Vec::new();

    while offset < source.len() {
        let byte = source[offset];
        if !in_php {
            if source[offset..].starts_with(b"<?") {
                in_php = true;
                offset += 2;
            } else {
                offset += 1;
            }
            continue;
        }
        if source[offset..].starts_with(b"?>") {
            in_php = false;
            offset += 2;
            continue;
        }
        if line_comment {
            line_comment = byte != b'\n';
            offset += 1;
            continue;
        }
        if block_comment {
            if source[offset..].starts_with(b"*/") {
                block_comment = false;
                offset += 2;
            } else {
                offset += 1;
            }
            continue;
        }
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                offset = (offset + 2).min(source.len());
            } else {
                if byte == delimiter {
                    quote = None;
                }
                offset += 1;
            }
            continue;
        }
        if source[offset..].starts_with(b"//")
            || (byte == b'#' && !source[offset..].starts_with(b"#["))
        {
            line_comment = true;
            offset += if byte == b'#' { 1 } else { 2 };
        } else if source[offset..].starts_with(b"/*") {
            block_comment = true;
            offset += 2;
        } else if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
            offset += 1;
        } else if source[offset..].starts_with(b"|>") {
            normalized[offset + 1] = b'|';
            ranges.push(offset..offset + 2);
            offset += 2;
        } else {
            offset += 1;
        }
    }
    (
        String::from_utf8(normalized).expect("replacing ASCII bytes preserves UTF-8"),
        ranges,
    )
}

/// Tree-sitter-php 0.24 does not parse PHP 8.4 property hook blocks. Keep the
/// property declaration in the tree by replacing each hook block with an
/// equal-length whitespace span ending in a semicolon. Original byte offsets
/// remain valid for indexing and diagnostics.
fn normalize_property_hooks(original: &str, parse_text: &str) -> (String, Vec<Range<usize>>) {
    let source = original.as_bytes();
    let mut normalized = parse_text.as_bytes().to_vec();
    let mut hook_ranges = Vec::new();
    let mut offset = 0;
    let mut quote = None;
    let mut line_comment = false;
    let mut block_comment = false;
    let mut in_php = !source.windows(2).any(|window| window == b"<?");

    while offset < source.len() {
        let byte = source[offset];
        if !in_php {
            if source[offset..].starts_with(b"<?") {
                in_php = true;
                offset += 2;
            } else {
                offset += 1;
            }
            continue;
        }
        if source[offset..].starts_with(b"?>") {
            in_php = false;
            offset += 2;
            continue;
        }
        if line_comment {
            if byte == b'\n' {
                line_comment = false;
            }
            offset += 1;
            continue;
        }
        if block_comment {
            if source[offset..].starts_with(b"*/") {
                block_comment = false;
                offset += 2;
            } else {
                offset += 1;
            }
            continue;
        }
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                offset = (offset + 2).min(source.len());
            } else if byte == delimiter {
                quote = None;
                offset += 1;
            } else {
                offset += 1;
            }
            continue;
        }
        if source[offset..].starts_with(b"//")
            || (byte == b'#' && !source[offset..].starts_with(b"#["))
        {
            line_comment = true;
            offset += if byte == b'#' { 1 } else { 2 };
            continue;
        }
        if source[offset..].starts_with(b"/*") {
            block_comment = true;
            offset += 2;
            continue;
        }
        if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
            offset += 1;
            continue;
        }

        if byte == b'{' && property_hook_prefix(&original[..offset]) {
            if let Some(close) = matching_brace(source, offset)
                && let Some(ranges) = property_hook_keywords(source, offset + 1, close)
            {
                hook_ranges.extend(ranges);
                normalized[offset..=close].fill(b' ');
                normalized[close] = b';';
                offset = close + 1;
                continue;
            }
        }
        offset += 1;
    }

    (
        String::from_utf8(normalized).expect("replacing ASCII bytes preserves UTF-8"),
        hook_ranges,
    )
}

fn property_hook_prefix(prefix: &str) -> bool {
    let declaration_start = prefix
        .rfind([';', '{', '}'])
        .map_or(0, |offset| offset + 1);
    let declaration = &prefix[declaration_start..];
    let Some(variable_start) = declaration.rfind('$') else {
        return false;
    };
    let variable_tail = &declaration[variable_start + 1..];
    let name_len = variable_tail
        .bytes()
        .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        .count();
    name_len > 0
        && variable_tail[name_len..].trim().is_empty()
        && !declaration[..variable_start].contains("function")
        && !declaration[..variable_start].contains("fn(")
}

fn matching_brace(source: &[u8], open: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut offset = open + 1;
    let mut quote = None;
    while offset < source.len() {
        let byte = source[offset];
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                offset = (offset + 2).min(source.len());
            } else {
                if byte == delimiter {
                    quote = None;
                }
                offset += 1;
            }
        } else if source[offset..].starts_with(b"//")
            || (byte == b'#' && !source[offset..].starts_with(b"#["))
        {
            offset += if byte == b'#' { 1 } else { 2 };
            while offset < source.len() && source[offset] != b'\n' {
                offset += 1;
            }
        } else if source[offset..].starts_with(b"/*") {
            offset += 2;
            while offset + 1 < source.len() && !source[offset..].starts_with(b"*/") {
                offset += 1;
            }
            offset = (offset + 2).min(source.len());
        } else if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
            offset += 1;
        } else {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(offset);
                    }
                }
                _ => {}
            }
            offset += 1;
        }
    }
    None
}

fn property_hook_keywords(source: &[u8], start: usize, end: usize) -> Option<Vec<Range<usize>>> {
    let mut ranges = Vec::new();
    let mut offset = start;
    let mut nested = 0usize;
    let mut quote = None;
    while offset < end {
        let byte = source[offset];
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                offset = (offset + 2).min(end);
            } else {
                if byte == delimiter {
                    quote = None;
                }
                offset += 1;
            }
            continue;
        }
        if source[offset..].starts_with(b"//")
            || (byte == b'#' && !source[offset..].starts_with(b"#["))
        {
            while offset < end && source[offset] != b'\n' {
                offset += 1;
            }
            continue;
        }
        if source[offset..].starts_with(b"/*") {
            offset += 2;
            while offset + 1 < end && !source[offset..].starts_with(b"*/") {
                offset += 1;
            }
            offset = (offset + 2).min(end);
            continue;
        }
        if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
            offset += 1;
            continue;
        }
        match byte {
            b'{' => nested += 1,
            b'}' => nested = nested.saturating_sub(1),
            _ => {}
        }
        if nested == 0 {
            for keyword in [&b"get"[..], &b"set"[..]] {
                let keyword_end = offset + keyword.len();
                let previous_boundary = offset == start
                    || !(source[offset - 1].is_ascii_alphanumeric()
                        || source[offset - 1] == b'_');
                let next_boundary = keyword_end <= end
                    && (keyword_end == end
                        || !(source[keyword_end].is_ascii_alphanumeric()
                            || source[keyword_end] == b'_'));
                if source[offset..].starts_with(keyword) && previous_boundary && next_boundary {
                    let mut after = keyword_end;
                    while after < end && source[after].is_ascii_whitespace() {
                        after += 1;
                    }
                    if after < end && (source[after..].starts_with(b"=>")
                        || source[after] == b'{' || source[after] == b'(')
                    {
                        ranges.push(offset..keyword_end);
                        offset = keyword_end;
                        break;
                    }
                }
            }
        }
        offset += 1;
    }
    (!ranges.is_empty()).then_some(ranges)
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
    fn token_lookup_keeps_php_keywords_out_of_name_resolution() {
        let source = "<?php return new Foo; function f() { if (true) { foreach ([] as $x) {} } } class Bar {}";
        let syntax = assert_clean(source);
        for keyword in ["return", "new", "function", "if", "foreach", "class"] {
            let offset = source.find(keyword).unwrap();
            assert!(syntax.is_keyword_at_byte(offset), "keyword={keyword}");
        }
        let await_source = "<?php $future->await();";
        let await_syntax = assert_clean(await_source);
        let await_offset = await_source.find("await").unwrap();
        assert!(!await_syntax.is_keyword_at_byte(await_offset));
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
    fn parses_and_highlights_php84_asymmetric_property_visibility() {
        let source = "<?php class Asymmetric { public private(set) string $name; protected private(set) ?int $id; public(set) private string $title; public readonly private(set) string $code; }";
        let syntax = assert_clean(source);
        let setter_tokens = source.match_indices("(set)").collect::<Vec<_>>();
        assert_eq!(setter_tokens.len(), 4);
        for (start, token) in setter_tokens {
            assert!(syntax.highlights().iter().any(|span| {
                span.kind == HighlightKind::Keyword
                    && span.start_byte == start
                    && span.end_byte == start + token.len()
            }));
        }
        assert_eq!(syntax.symbols().iter().filter(|symbol| symbol.kind == SymbolKind::Class).count(), 1);
    }

    #[test]
    fn asymmetric_visibility_text_in_comments_and_strings_is_not_highlighted_as_modifier() {
        let source = "<p>public(set)</p><?php // private(set)\n$value = 'protected(set)'; class Clean {}";
        let syntax = assert_clean(source);
        assert!(!syntax.highlights().iter().any(|span| {
            span.kind == HighlightKind::Keyword
                && source.get(span.start_byte..span.end_byte) == Some("(set)")
        }));
    }

    #[test]
    fn incremental_edit_reparses_asymmetric_visibility_without_errors() {
        let mut syntax = assert_clean("<?php class C { public string $name; }");
        let start = syntax.text().find("public").unwrap();
        syntax
            .apply_edit(start..start + "public".len(), "public private(set)")
            .unwrap();
        assert!(!syntax.has_errors(), "{}", syntax.tree().root_node().to_sexp());
        assert!(syntax.highlights().iter().any(|span| {
            span.kind == HighlightKind::Keyword
                && syntax.text().get(span.start_byte..span.end_byte) == Some("(set)")
        }));
    }

    #[test]
    fn parses_php84_property_hooks_and_highlights_hook_keywords() {
        let source = "<?php class C { public string $name { get => $this->_name; set => strtolower($value); } public string $title { get { return ucfirst($this->_title); } } public private(set) string $code { get => $this->_code; } public $virtual { get => 42; } }";
        let syntax = assert_clean(source);
        let hooks = source
            .match_indices("get")
            .chain(source.match_indices("set =>" ).map(|(offset, _)| (offset, "set")))
            .collect::<Vec<_>>();
        assert_eq!(hooks.len(), 5);
        for (offset, keyword) in hooks {
            assert!(syntax.highlights().iter().any(|span| {
                span.kind == HighlightKind::PropertyHook
                    && span.start_byte == offset
                    && span.end_byte == offset + keyword.len()
            }));
        }
        let mut properties = Vec::new();
        let mut nodes = vec![syntax.tree().root_node()];
        while let Some(node) = nodes.pop() {
            if node.kind() == "property_declaration" {
                properties.push(node);
            }
            nodes.extend((0..node.child_count()).filter_map(|index| node.child(index)));
        }
        assert_eq!(properties.len(), 4);
        assert!(properties.iter().all(|node| {
            source
                .get(node.start_byte()..node.end_byte())
                .is_some_and(|raw| raw.contains("get") || raw.contains("set"))
        }));
    }

    #[test]
    fn property_hook_words_in_strings_and_comments_are_not_highlighted() {
        let source = "<?php class C { public string $name { get => 'set'; /* get => */ } }";
        let syntax = assert_clean(source);
        let expected = source.find("get => '").unwrap();
        assert!(syntax.highlights().iter().any(|span| {
            span.kind == HighlightKind::PropertyHook && span.start_byte == expected
        }));
        let string_set = source.find("'set'").unwrap() + 1;
        let comment_get = source.find("/* get").unwrap() + 3;
        assert!(!syntax.highlights().iter().any(|span| {
            span.kind == HighlightKind::PropertyHook
                && (span.start_byte == string_set || span.start_byte == comment_get)
        }));
    }

    #[test]
    fn parses_php_mixed_with_html() {
        assert_clean(
            "<!DOCTYPE html><html><body><?php if ($user): ?><h1><?= htmlspecialchars($user->name) ?></h1><?php endif; ?></body></html>",
        );
    }

    #[test]
    fn parses_arrow_function_expression_without_syntax_errors() {
        let syntax = assert_clean(
            "<?php $callback = fn($value) => $value + 1; consume(fn($item) => $item); return fn() => true;",
        );
        let mut stack = vec![syntax.tree().root_node()];
        let mut arrow_functions = 0;
        while let Some(node) = stack.pop() {
            arrow_functions += usize::from(node.kind() == "arrow_function");
            stack.extend((0..node.child_count()).filter_map(|index| node.child(index)));
        }
        assert_eq!(arrow_functions, 3);
    }

    #[test]
    fn parses_php85_pipe_chains_without_errors_and_ignores_pipe_text() {
        let text = "<?php $title = ' PHP 8.5 Released '; $slug = $title |> trim(...) |> (fn($str) => str_replace(' ', '-', $str)) |> strtolower(...); // |> in comment\n$literal = '|>';";
        let syntax = PhpSyntax::parse(text).unwrap();
        assert!(!syntax.has_errors(), "{:?}", syntax.diagnostics());
        assert_eq!(syntax.text(), text);
        assert_eq!(syntax.pipe_operator_ranges().len(), 3);
    }

    #[test]
    fn recognizes_pipe_operator_before_incomplete_rhs_completion() {
        let text = "<?php $value = 'text'\n    |> ";
        let syntax = PhpSyntax::parse(text).unwrap();
        assert_eq!(syntax.pipe_operator_ranges().len(), 1);
        let range = &syntax.pipe_operator_ranges()[0];
        assert_eq!(&text[range.clone()], "|>");
    }

    #[test]
    fn parses_closures_and_first_class_callables_in_attribute_arguments() {
        let text = "<?php final class PostsController { #[AccessControl(static function (Request $request, Post $post): bool { return $request->user === $post->getAuthor(); })] public function update(Request $request, Post $post): Response {} #[Callback(trim(...))] public const CALLBACK = static function () {}; }";
        let syntax = PhpSyntax::parse(text).unwrap();
        assert!(!syntax.has_errors(), "{:?}", syntax.diagnostics());
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
    fn incremental_multiline_edit_uses_utf8_bytes_and_matches_full_parse() {
        let initial = "<?php\n// ação 😀\nclass User {\n    public function run(): void {}\n}\n";
        let mut syntax = PhpSyntax::parse(initial).unwrap();
        let start = initial.find("public function").unwrap();
        let replacement = "public function prepare(): void {}\n    public function run(): void {}";
        syntax
            .apply_edit(
                start..start + "public function run(): void {}".len(),
                replacement,
            )
            .unwrap();

        let full = PhpSyntax::parse(syntax.text()).unwrap();
        assert_eq!(
            syntax.tree().root_node().to_sexp(),
            full.tree().root_node().to_sexp()
        );
        assert_eq!(syntax.text().match_indices("function").count(), 2);
    }

    #[test]
    fn incremental_delete_and_append_around_emoji_match_full_parse() {
        let initial = "<?php\n$label = \"ação ç ã 😀\";\nclass User {}\n";
        let mut syntax = PhpSyntax::parse(initial).unwrap();
        let emoji = initial.find('😀').unwrap();
        syntax
            .apply_edit(emoji..emoji + '😀'.len_utf8(), "")
            .unwrap();
        let end = syntax.text().len();
        syntax
            .apply_edit(end..end, "final class Added {}\n")
            .unwrap();

        let full = PhpSyntax::parse(syntax.text()).unwrap();
        assert_eq!(
            syntax.tree().root_node().to_sexp(),
            full.tree().root_node().to_sexp()
        );
        assert!(!syntax.text().contains('😀'));
        assert!(syntax.text().contains("final class Added"));
    }

    #[test]
    fn incremental_edit_rejects_ranges_inside_utf8_codepoints() {
        let initial = "<?php $label = \"ação 😀\";";
        let mut syntax = PhpSyntax::parse(initial).unwrap();
        let start = initial.find('ç').unwrap();
        let before = syntax.text().to_owned();
        let error = syntax.apply_edit(start + 1..start + 1, "x").unwrap_err();
        assert!(matches!(error, SyntaxError::InvalidEdit(_)));
        assert_eq!(syntax.text(), before);
    }

    #[test]
    fn incremental_multiline_edit_matches_full_parse() {
        let initial = "<?php\nfunction run(): void {\n    return;\n}\n";
        let mut syntax = PhpSyntax::parse(initial).unwrap();
        let start = initial.find("return;").unwrap();
        syntax
            .apply_edit(
                start..start + "return;".len(),
                "// ação\n    return;\n    // 😀",
            )
            .unwrap();
        let full = PhpSyntax::parse(syntax.text()).unwrap();
        assert_eq!(
            syntax.tree().root_node().to_sexp(),
            full.tree().root_node().to_sexp()
        );
        assert!(syntax.text().contains("ação"));
        assert!(syntax.text().contains("😀"));
    }

    #[test]
    fn incremental_edit_rejects_non_boundary_byte_offsets() {
        let mut syntax = PhpSyntax::parse("<?php $value = \"ç\";").unwrap();
        let start = syntax.text().find('ç').unwrap();
        assert!(matches!(
            syntax.apply_edit(start + 1..start + 1, "x"),
            Err(SyntaxError::InvalidEdit(_))
        ));
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
