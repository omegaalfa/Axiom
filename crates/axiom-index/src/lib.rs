//! Project-local PHP symbol index.
//!
//! This crate is deliberately headless. It owns no UI or LSP state and can be
//! queried from completion/navigation providers without blocking rendering.

use axiom_syntax::PhpSyntax;
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};
use tree_sitter::Node;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub enum ProjectSymbolKind {
    Class,
    Interface,
    Trait,
    Enum,
    Function,
    Method,
    Property,
    Constant,
    ClassConstant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Visibility {
    Public,
    Protected,
    Private,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSymbol {
    pub name: String,
    pub fully_qualified_name: String,
    pub kind: ProjectSymbolKind,
    pub file: PathBuf,
    pub range: std::ops::Range<usize>,
    pub namespace: String,
    pub visibility: Visibility,
    pub modifiers: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct IndexReport {
    pub files: usize,
    pub symbols: usize,
    pub duplicates: usize,
}

#[derive(Debug, Default)]
pub struct ProjectSymbolIndex {
    files: BTreeMap<PathBuf, Arc<str>>,
    symbols: Vec<ProjectSymbol>,
    ready: bool,
}

impl ProjectSymbolIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn index_project(&mut self, root: impl AsRef<Path>) -> io::Result<IndexReport> {
        self.files.clear();
        self.symbols.clear();
        self.ready = false;
        let mut paths = Vec::new();
        collect_php_files(root.as_ref(), &mut paths)?;
        for path in paths {
            let _ = self.index_file(&path);
        }
        self.index_composer_classmap(root.as_ref());
        self.ready = true;
        Ok(self.report())
    }

    /// Reads Composer's generated classmap as data only. The PHP file is never
    /// executed; this intentionally supports the common `__DIR__ . '/file.php'`
    /// form emitted by Composer.
    fn index_composer_classmap(&mut self, root: &Path) {
        let path = root.join("vendor/composer/autoload_classmap.php");
        let Ok(text) = fs::read_to_string(&path) else {
            return;
        };
        for line in text.lines() {
            let Some((left, right)) = line.split_once("=>") else {
                continue;
            };
            let Some(name) = left.split('\'').nth(1) else {
                continue;
            };
            let Some(file) = right.split('\'').nth(1) else {
                continue;
            };
            let file = file.trim_start_matches('/');
            let target = path
                .parent()
                .map(|composer| composer.join(file))
                .filter(|candidate| candidate.is_file());
            let Some(file) = target else { continue };
            self.symbols.push(ProjectSymbol {
                name: name.rsplit('\\').next().unwrap_or(name).to_owned(),
                fully_qualified_name: name.to_owned(),
                kind: ProjectSymbolKind::Class,
                file,
                range: 0..0,
                namespace: name
                    .rsplit_once('\\')
                    .map(|(ns, _)| ns)
                    .unwrap_or("")
                    .to_owned(),
                visibility: Visibility::Unknown,
                modifiers: vec!["composer".to_owned()],
            });
        }
    }

    pub fn index_file(&mut self, path: impl AsRef<Path>) -> io::Result<usize> {
        let path = fs::canonicalize(path.as_ref())?;
        let text = fs::read_to_string(&path)?;
        self.index_file_text(path, text)
    }

    /// Incrementally replaces one indexed file from an in-memory document.
    /// This is used for dirty buffers and avoids a project-wide traversal.
    pub fn index_file_text(
        &mut self,
        path: impl AsRef<Path>,
        text: impl Into<String>,
    ) -> io::Result<usize> {
        let path = fs::canonicalize(path.as_ref()).unwrap_or_else(|_| path.as_ref().to_path_buf());
        let text = text.into();
        self.remove_file(&path);
        let syntax = PhpSyntax::parse(text.clone())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        let mut output = Vec::new();
        let namespace = namespace_of(&syntax);
        walk(
            syntax.tree().root_node(),
            &text,
            &path,
            &namespace,
            None,
            &mut output,
        );
        let count = output.len();
        self.files.insert(path, Arc::from(text));
        self.symbols.extend(output);
        Ok(count)
    }

    pub fn remove_file(&mut self, path: impl AsRef<Path>) -> usize {
        let path = fs::canonicalize(path.as_ref()).unwrap_or_else(|_| path.as_ref().to_path_buf());
        self.files.remove(&path);
        let before = self.symbols.len();
        self.symbols.retain(|symbol| symbol.file != path);
        before - self.symbols.len()
    }

    pub fn is_ready(&self) -> bool {
        self.ready
    }
    pub fn symbols(&self) -> &[ProjectSymbol] {
        &self.symbols
    }
    pub fn report(&self) -> IndexReport {
        let mut names = std::collections::HashSet::new();
        let duplicates = self
            .symbols
            .iter()
            .filter(|s| !names.insert((s.fully_qualified_name.clone(), s.kind)))
            .count();
        IndexReport {
            files: self.files.len(),
            symbols: self.symbols.len(),
            duplicates,
        }
    }
    pub fn find_fqn(&self, fqn: &str) -> Option<&ProjectSymbol> {
        self.symbols.iter().find(|s| s.fully_qualified_name == fqn)
    }
    pub fn find_class(&self, name: &str) -> Option<&ProjectSymbol> {
        self.symbols.iter().find(|s| {
            matches!(
                s.kind,
                ProjectSymbolKind::Class
                    | ProjectSymbolKind::Interface
                    | ProjectSymbolKind::Trait
                    | ProjectSymbolKind::Enum
            ) && (s.name == name || s.fully_qualified_name == name)
        })
    }
    pub fn find_methods(&self, class_fqn: &str) -> Vec<&ProjectSymbol> {
        let prefix = format!("{class_fqn}::");
        self.symbols
            .iter()
            .filter(|s| {
                s.kind == ProjectSymbolKind::Method && s.fully_qualified_name.starts_with(&prefix)
            })
            .collect()
    }
    pub fn search_prefix(&self, prefix: &str) -> Vec<&ProjectSymbol> {
        let mut result: Vec<_> = self
            .symbols
            .iter()
            .filter(|s| s.name.starts_with(prefix) || s.fully_qualified_name.starts_with(prefix))
            .collect();
        result.sort_by_key(|s| (!s.name.starts_with(prefix), s.name.to_lowercase()));
        result
    }
}

fn collect_php_files(root: &Path, output: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| matches!(n, ".git" | "target" | "node_modules" | "vendor"))
        {
            continue;
        }
        if path.is_dir() {
            collect_php_files(&path, output)?;
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("php"))
        {
            output.push(path);
        }
    }
    Ok(())
}

fn namespace_of(syntax: &PhpSyntax) -> String {
    let text = syntax.text();
    let mut found = String::new();
    let mut stack = vec![syntax.tree().root_node()];
    while let Some(node) = stack.pop() {
        if node.kind() == "namespace_definition" {
            if let Some(name) = node.child_by_field_name("name") {
                found = name.utf8_text(text.as_bytes()).unwrap_or("").to_owned();
                break;
            }
        }
        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor));
    }
    found.trim_matches('\\').to_owned()
}

fn walk(
    node: Node<'_>,
    text: &str,
    file: &Path,
    namespace: &str,
    class: Option<&str>,
    out: &mut Vec<ProjectSymbol>,
) {
    if let Some((kind, name_node)) = symbol_node(node) {
        if let Ok(name) = name_node.utf8_text(text.as_bytes()) {
            let name = name.to_owned();
            let (visibility, modifiers) = declaration_modifiers(node, text);
            let fqn = match (class, kind) {
                (
                    Some(parent),
                    ProjectSymbolKind::Method
                    | ProjectSymbolKind::Property
                    | ProjectSymbolKind::ClassConstant,
                ) => format!("{namespace}\\{parent}::{name}"),
                _ if namespace.is_empty() => name.clone(),
                _ => format!("{namespace}\\{name}"),
            };
            out.push(ProjectSymbol {
                name,
                fully_qualified_name: fqn,
                kind,
                file: file.to_path_buf(),
                range: name_node.byte_range(),
                namespace: namespace.to_owned(),
                visibility,
                modifiers,
            });
            let next_class = matches!(
                kind,
                ProjectSymbolKind::Class
                    | ProjectSymbolKind::Interface
                    | ProjectSymbolKind::Trait
                    | ProjectSymbolKind::Enum
            )
            .then_some(name_node.utf8_text(text.as_bytes()).unwrap_or(""));
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                walk(child, text, file, namespace, next_class.or(class), out);
            }
            return;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, text, file, namespace, class, out);
    }
}

fn declaration_modifiers(node: Node<'_>, text: &str) -> (Visibility, Vec<String>) {
    let source = &text[node.start_byte()..node.end_byte().min(text.len())];
    let mut modifiers = Vec::new();
    for word in [
        "public",
        "protected",
        "private",
        "static",
        "abstract",
        "final",
        "readonly",
    ] {
        if source
            .split_whitespace()
            .any(|part| part.trim_matches(|c: char| !c.is_ascii_alphabetic()) == word)
        {
            modifiers.push(word.to_owned());
        }
    }
    let visibility = if modifiers.iter().any(|m| m == "private") {
        Visibility::Private
    } else if modifiers.iter().any(|m| m == "protected") {
        Visibility::Protected
    } else if modifiers.iter().any(|m| m == "public") {
        Visibility::Public
    } else {
        Visibility::Unknown
    };
    (visibility, modifiers)
}

fn symbol_node(node: Node<'_>) -> Option<(ProjectSymbolKind, Node<'_>)> {
    let kind = match node.kind() {
        "class_declaration" => ProjectSymbolKind::Class,
        "interface_declaration" => ProjectSymbolKind::Interface,
        "trait_declaration" => ProjectSymbolKind::Trait,
        "enum_declaration" => ProjectSymbolKind::Enum,
        "function_definition" => ProjectSymbolKind::Function,
        "method_declaration" => ProjectSymbolKind::Method,
        "property_declaration" => ProjectSymbolKind::Property,
        "const_declaration" => ProjectSymbolKind::Constant,
        "class_constant_declaration" => ProjectSymbolKind::ClassConstant,
        _ => return None,
    };
    let field = node.child_by_field_name("name").or_else(|| {
        node.named_children(&mut node.walk())
            .find(|child| child.kind() == "variable_name" || child.kind() == "name")
    })?;
    Some((kind, field))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indexes_php_symbols_and_fqns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("User.php");
        fs::write(
            &path,
            "<?php namespace App\\Service; class User { public function find() {} }",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        let report = index.index_project(dir.path()).unwrap();
        assert_eq!(report.files, 1);
        assert!(index.find_fqn("App\\Service\\User").is_some());
        assert!(index.find_fqn("App\\Service\\User::find").is_some());
    }
    #[test]
    fn incremental_update_removes_old_symbols() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("A.php");
        fs::write(&path, "<?php class Before {}").unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_file(&path).unwrap();
        fs::write(&path, "<?php class After {}").unwrap();
        index.index_file(&path).unwrap();
        assert!(index.find_class("Before").is_none());
        assert!(index.find_class("After").is_some());
    }
}
