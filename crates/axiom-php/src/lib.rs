//! Native PHP symbol model and in-memory runtime stub index.

use std::{
    collections::HashMap,
    env, fmt, fs, io,
    ops::Range,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use axiom_syntax::PhpSyntax;
use tree_sitter::Node;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Function,
    Class,
    Interface,
    Trait,
    Enum,
    Method,
    Property,
    ClassConstant,
    GlobalConstant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolOrigin {
    Project,
    Composer,
    PhpRuntime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    pub file: PathBuf,
    pub range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Availability {
    pub since: Option<String>,
    pub until: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameter {
    pub name: String,
    pub declared_type: Option<String>,
    pub phpdoc_type: Option<String>,
    pub optional: bool,
    pub variadic: bool,
    pub by_reference: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Signature {
    pub parameters: Vec<Parameter>,
    pub declared_return_type: Option<String>,
    pub phpdoc_return_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhpDocParam {
    pub name: String,
    pub declared_type: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PhpDoc {
    pub description: Option<String>,
    pub params: Vec<PhpDocParam>,
    pub return_type: Option<String>,
    pub var_type: Option<String>,
    pub throws: Vec<String>,
    pub deprecated: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub fqn: String,
    pub kind: SymbolKind,
    pub origin: SymbolOrigin,
    pub extension: String,
    pub location: SourceLocation,
    /// Declared type for typed properties and typed constants.
    pub declared_type: Option<String>,
    pub signature: Option<Signature>,
    pub documentation: Option<PhpDoc>,
    pub availability: Availability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StubError {
    pub file: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct LoadReport {
    pub files_discovered: usize,
    pub files_parsed: usize,
    pub symbols_indexed: usize,
    pub errors: Vec<StubError>,
    pub elapsed: Duration,
}

#[derive(Debug)]
pub enum StubProviderError {
    MissingDirectory(PathBuf),
    Io(io::Error),
}

impl fmt::Display for StubProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingDirectory(path) => {
                write!(f, "stub directory not found: {}", path.display())
            }
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for StubProviderError {}

impl From<io::Error> for StubProviderError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Debug, Clone)]
pub struct StubProvider {
    root: PathBuf,
}

impl StubProvider {
    pub fn from_env() -> Option<Self> {
        env::var_os("AXIOM_PHP_STUBS")
            .or_else(|| env::var_os("RUSTSTORM_PHP_STUBS"))
            .map(PathBuf::from)
            .map(Self::new)
    }

    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn discover(&self) -> Result<Vec<StubFile>, StubProviderError> {
        if !self.root.is_dir() {
            return Err(StubProviderError::MissingDirectory(self.root.clone()));
        }
        let mut files = Vec::new();
        discover_php_files(&self.root, &self.root, &mut files)?;
        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(files)
    }

    pub fn load(&self) -> Result<(RuntimeSymbolIndex, LoadReport), StubProviderError> {
        let started = Instant::now();
        let files = self.discover()?;
        let mut report = LoadReport {
            files_discovered: files.len(),
            ..Default::default()
        };
        let mut index = RuntimeSymbolIndex::default();
        for stub in files {
            match fs::read_to_string(&stub.path) {
                Ok(text) => match extract_symbols(&text, &stub.path, &stub.extension) {
                    Ok(symbols) => {
                        report.files_parsed += 1;
                        report.symbols_indexed += symbols.len();
                        for symbol in symbols {
                            index.insert(symbol);
                        }
                    }
                    Err(message) => report.errors.push(StubError {
                        file: stub.path,
                        message,
                    }),
                },
                Err(error) => report.errors.push(StubError {
                    file: stub.path,
                    message: error.to_string(),
                }),
            }
        }
        report.elapsed = started.elapsed();
        tracing::info!(
            files_discovered = report.files_discovered,
            files_parsed = report.files_parsed,
            symbols = report.symbols_indexed,
            errors = report.errors.len(),
            elapsed_ms = report.elapsed.as_millis(),
            "PHP runtime stubs loaded"
        );
        Ok((index, report))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StubFile {
    pub path: PathBuf,
    pub extension: String,
}

#[derive(Debug, Default)]
pub struct RuntimeSymbolIndex {
    classes: HashMap<String, Vec<Symbol>>,
    functions: HashMap<String, Vec<Symbol>>,
    constants: HashMap<String, Vec<Symbol>>,
    members: HashMap<String, Vec<Symbol>>,
    symbol_count: usize,
}

impl RuntimeSymbolIndex {
    pub fn insert(&mut self, symbol: Symbol) {
        self.symbol_count += 1;
        match symbol.kind {
            SymbolKind::Class | SymbolKind::Interface | SymbolKind::Trait | SymbolKind::Enum => {
                self.classes
                    .entry(fold_name(&symbol.fqn))
                    .or_default()
                    .push(symbol);
            }
            SymbolKind::Function => {
                self.functions
                    .entry(fold_name(&symbol.fqn))
                    .or_default()
                    .push(symbol);
            }
            SymbolKind::GlobalConstant => {
                self.constants
                    .entry(symbol.fqn.clone())
                    .or_default()
                    .push(symbol);
            }
            SymbolKind::Method | SymbolKind::Property | SymbolKind::ClassConstant => {
                if let Some((owner, _)) = symbol.fqn.rsplit_once("::") {
                    self.members
                        .entry(fold_name(owner))
                        .or_default()
                        .push(symbol);
                }
            }
        }
    }

    pub fn find_class(&self, fqn: &str) -> Option<&Symbol> {
        self.class_definitions(fqn).first()
    }

    pub fn class_definitions(&self, fqn: &str) -> &[Symbol] {
        self.classes
            .get(&fold_name(fqn))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn find_function(&self, fqn: &str) -> Option<&Symbol> {
        self.function_definitions(fqn).first()
    }

    pub fn function_definitions(&self, fqn: &str) -> &[Symbol] {
        self.functions
            .get(&fold_name(fqn))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn find_constant(&self, fqn: &str) -> Option<&Symbol> {
        self.constants.get(fqn).and_then(|symbols| symbols.first())
    }

    pub fn members_of(&self, class_fqn: &str) -> &[Symbol] {
        self.members
            .get(&fold_name(class_fqn))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn search_prefix(&self, prefix: &str) -> Vec<&Symbol> {
        let prefix = fold_name(prefix);
        self.classes
            .values()
            .chain(self.functions.values())
            .chain(self.constants.values())
            .flat_map(|items| items.iter())
            .filter(|symbol| {
                fold_name(&symbol.name).starts_with(&prefix)
                    || fold_name(&symbol.fqn).starts_with(&prefix)
            })
            .collect()
    }

    pub fn methods_of(&self, class_fqn: &str) -> impl Iterator<Item = &Symbol> {
        self.members_of(class_fqn)
            .iter()
            .filter(|symbol| symbol.kind == SymbolKind::Method)
    }

    pub fn len(&self) -> usize {
        self.symbol_count
    }

    pub fn is_empty(&self) -> bool {
        self.symbol_count == 0
    }
}

fn fold_name(name: &str) -> String {
    name.trim_start_matches('\\').to_lowercase()
}

fn discover_php_files(root: &Path, directory: &Path, files: &mut Vec<StubFile>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            let name = entry.file_name();
            if matches!(name.to_str(), Some(".git" | ".idea" | "tests" | "vendor")) {
                continue;
            }
            discover_php_files(root, &path, files)?;
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("php"))
        {
            let extension = path
                .strip_prefix(root)
                .ok()
                .and_then(|relative| relative.components().next())
                .and_then(|component| component.as_os_str().to_str())
                .unwrap_or("Core")
                .to_owned();
            files.push(StubFile { path, extension });
        }
    }
    Ok(())
}

fn extract_symbols(text: &str, file: &Path, extension: &str) -> Result<Vec<Symbol>, String> {
    let syntax = PhpSyntax::parse(text.to_owned()).map_err(|error| error.to_string())?;
    if syntax.has_errors() {
        return Err("stub contains PHP syntax errors".into());
    }
    let namespace = syntax
        .symbols()
        .iter()
        .find(|symbol| symbol.kind == axiom_syntax::SymbolKind::Namespace)
        .map(|symbol| symbol.name.as_str());
    let root = syntax.tree().root_node();
    let mut symbols = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        visit_node(child, text, file, extension, namespace, None, &mut symbols);
    }
    Ok(symbols)
}

fn visit_node(
    node: Node<'_>,
    text: &str,
    file: &Path,
    extension: &str,
    namespace: Option<&str>,
    owner: Option<&str>,
    symbols: &mut Vec<Symbol>,
) {
    let declaration_kind = match node.kind() {
        "class_declaration" => Some(SymbolKind::Class),
        "interface_declaration" => Some(SymbolKind::Interface),
        "trait_declaration" => Some(SymbolKind::Trait),
        "enum_declaration" => Some(SymbolKind::Enum),
        "function_definition" => Some(SymbolKind::Function),
        "method_declaration" => Some(SymbolKind::Method),
        _ => None,
    };
    if let Some(kind) = declaration_kind
        && let Some(name_node) = node.child_by_field_name("name")
    {
        let name = node_text(name_node, text).to_owned();
        let fqn = if kind == SymbolKind::Method {
            format!("{}::{name}", owner.unwrap_or(""))
        } else {
            qualify(namespace, &name)
        };
        let documentation = preceding_phpdoc(node, text).map(parse_phpdoc);
        let signature = matches!(kind, SymbolKind::Function | SymbolKind::Method)
            .then(|| extract_signature(node, text, documentation.as_ref()));
        symbols.push(Symbol {
            name: name.clone(),
            fqn: fqn.clone(),
            kind,
            origin: SymbolOrigin::PhpRuntime,
            extension: extension.to_owned(),
            location: SourceLocation {
                file: file.to_path_buf(),
                range: name_node.byte_range(),
            },
            declared_type: None,
            signature,
            documentation,
            availability: extract_availability(node_text(node, text)),
        });
        if matches!(
            kind,
            SymbolKind::Class | SymbolKind::Interface | SymbolKind::Trait | SymbolKind::Enum
        ) {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                visit_node(child, text, file, extension, namespace, Some(&fqn), symbols);
            }
            return;
        }
    }

    if node.kind() == "property_declaration" {
        let declared_type = node
            .child_by_field_name("type")
            .map(|node| node_text(node, text).to_owned());
        let documentation = preceding_phpdoc(node, text).map(parse_phpdoc);
        let mut cursor = node.walk();
        for child in node
            .children(&mut cursor)
            .filter(|child| child.kind() == "property_element")
        {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = node_text(name_node, text)
                    .trim_start_matches('$')
                    .to_owned();
                symbols.push(member_symbol(
                    SymbolKind::Property,
                    owner,
                    name,
                    name_node,
                    file,
                    extension,
                    documentation.clone(),
                    declared_type.clone(),
                ));
            }
        }
    } else if node.kind() == "const_declaration" {
        let kind = if owner.is_some() {
            SymbolKind::ClassConstant
        } else {
            SymbolKind::GlobalConstant
        };
        let mut cursor = node.walk();
        for child in node
            .children(&mut cursor)
            .filter(|child| child.kind() == "const_element")
        {
            let mut child_cursor = child.walk();
            if let Some(name_node) = child
                .named_children(&mut child_cursor)
                .find(|candidate| candidate.kind() == "name")
            {
                let name = node_text(name_node, text).to_owned();
                let fqn = owner.map_or_else(
                    || qualify(namespace, &name),
                    |owner| format!("{owner}::{name}"),
                );
                symbols.push(Symbol {
                    name,
                    fqn,
                    kind,
                    origin: SymbolOrigin::PhpRuntime,
                    extension: extension.to_owned(),
                    location: SourceLocation {
                        file: file.to_path_buf(),
                        range: name_node.byte_range(),
                    },
                    declared_type: node
                        .child_by_field_name("type")
                        .map(|node| node_text(node, text).to_owned()),
                    signature: None,
                    documentation: preceding_phpdoc(node, text).map(parse_phpdoc),
                    availability: extract_availability(node_text(node, text)),
                });
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit_node(child, text, file, extension, namespace, owner, symbols);
    }
}

#[allow(clippy::too_many_arguments)]
fn member_symbol(
    kind: SymbolKind,
    owner: Option<&str>,
    name: String,
    name_node: Node<'_>,
    file: &Path,
    extension: &str,
    documentation: Option<PhpDoc>,
    declared_type: Option<String>,
) -> Symbol {
    Symbol {
        fqn: format!("{}::{name}", owner.unwrap_or("")),
        name,
        kind,
        origin: SymbolOrigin::PhpRuntime,
        extension: extension.to_owned(),
        location: SourceLocation {
            file: file.to_path_buf(),
            range: name_node.byte_range(),
        },
        declared_type,
        signature: None,
        documentation,
        availability: Availability::default(),
    }
}

fn extract_signature(node: Node<'_>, text: &str, phpdoc: Option<&PhpDoc>) -> Signature {
    let mut parameters = Vec::new();
    if let Some(parameter_list) = node.child_by_field_name("parameters") {
        collect_parameters(parameter_list, text, phpdoc, &mut parameters);
    }
    Signature {
        parameters,
        declared_return_type: node
            .child_by_field_name("return_type")
            .map(|node| node_text(node, text).to_owned()),
        phpdoc_return_type: phpdoc.and_then(|doc| doc.return_type.clone()),
    }
}

fn collect_parameters(
    node: Node<'_>,
    text: &str,
    phpdoc: Option<&PhpDoc>,
    output: &mut Vec<Parameter>,
) {
    if matches!(
        node.kind(),
        "simple_parameter" | "variadic_parameter" | "property_promotion_parameter"
    ) {
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = node_text(name_node, text)
                .trim_start_matches('$')
                .to_owned();
            output.push(Parameter {
                phpdoc_type: phpdoc
                    .and_then(|doc| doc.params.iter().find(|parameter| parameter.name == name))
                    .and_then(|parameter| parameter.declared_type.clone()),
                name,
                declared_type: node
                    .child_by_field_name("type")
                    .map(|node| node_text(node, text).to_owned()),
                optional: node.child_by_field_name("default_value").is_some(),
                variadic: node.kind() == "variadic_parameter"
                    || node_text(node, text).contains("..."),
                by_reference: node_text(node, text).contains('&'),
            });
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_parameters(child, text, phpdoc, output);
    }
}

fn qualify(namespace: Option<&str>, name: &str) -> String {
    namespace
        .filter(|namespace| !namespace.is_empty())
        .map_or_else(
            || name.to_owned(),
            |namespace| format!("{}\\{name}", namespace.trim_matches('\\')),
        )
}

fn node_text<'a>(node: Node<'_>, text: &'a str) -> &'a str {
    &text[node.byte_range()]
}

fn preceding_phpdoc<'a>(node: Node<'_>, text: &'a str) -> Option<&'a str> {
    let comment = node.prev_named_sibling()?;
    (comment.kind() == "comment" && node_text(comment, text).trim_start().starts_with("/**"))
        .then(|| node_text(comment, text))
}

pub fn parse_phpdoc(comment: &str) -> PhpDoc {
    let mut doc = PhpDoc::default();
    let mut description = Vec::new();
    for raw_line in comment.lines() {
        let line = raw_line
            .trim()
            .trim_start_matches("/**")
            .trim_start_matches("/*")
            .trim_start_matches('*')
            .trim_end_matches("*/")
            .trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("@param ") {
            let variable_start = rest
                .char_indices()
                .find(|(offset, character)| {
                    *character == '$'
                        && (*offset == 0
                            || rest[..*offset]
                                .chars()
                                .next_back()
                                .is_some_and(char::is_whitespace))
                })
                .map(|(offset, _)| offset);
            let (declared_type, name, remainder) = variable_start.map_or_else(
                || (None, String::new(), String::new()),
                |offset| {
                    let declared_type = rest[..offset].trim();
                    let variable_and_description = &rest[offset + 1..];
                    let name_end = variable_and_description
                        .find(char::is_whitespace)
                        .unwrap_or(variable_and_description.len());
                    (
                        (!declared_type.is_empty()).then(|| declared_type.to_owned()),
                        variable_and_description[..name_end].to_owned(),
                        variable_and_description[name_end..].trim().to_owned(),
                    )
                },
            );
            doc.params.push(PhpDocParam {
                name,
                declared_type,
                description: (!remainder.is_empty()).then_some(remainder),
            });
        } else if let Some(rest) = line.strip_prefix("@return ") {
            doc.return_type = phpdoc_type_expression(rest);
        } else if let Some(rest) = line.strip_prefix("@var ") {
            doc.var_type = phpdoc_type_expression(rest);
        } else if let Some(rest) = line.strip_prefix("@throws ") {
            if let Some(kind) = rest.split_whitespace().next() {
                doc.throws.push(kind.to_owned());
            }
        } else if let Some(rest) = line.strip_prefix("@deprecated") {
            doc.deprecated = Some(rest.trim().to_owned());
        } else if !line.starts_with('@') {
            description.push(line);
        }
    }
    if !description.is_empty() {
        doc.description = Some(description.join(" "));
    }
    doc
}

fn phpdoc_type_expression(text: &str) -> Option<String> {
    let mut nesting = 0_u32;
    let end = text
        .char_indices()
        .find_map(|(offset, character)| match character {
            '<' | '(' | '[' | '{' => {
                nesting += 1;
                None
            }
            '>' | ')' | ']' | '}' => {
                nesting = nesting.saturating_sub(1);
                None
            }
            character if character.is_whitespace() && nesting == 0 => Some(offset),
            _ => None,
        })
        .unwrap_or(text.len());
    let value = text[..end].trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn extract_availability(declaration: &str) -> Availability {
    Availability {
        since: attribute_argument(declaration, "Since"),
        until: attribute_argument(declaration, "Until"),
    }
}

fn attribute_argument(text: &str, name: &str) -> Option<String> {
    let marker = format!("#[{name}(");
    let rest = text.split_once(&marker)?.1;
    let argument = rest.split_once(')')?.0.trim().trim_matches(['\'', '"']);
    (!argument.is_empty()).then(|| argument.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stubs")
    }

    fn load_fixture() -> (RuntimeSymbolIndex, LoadReport) {
        StubProvider::new(fixture_root()).load().unwrap()
    }

    #[test]
    fn discovers_extensions_and_php_files() {
        let files = StubProvider::new(fixture_root()).discover().unwrap();
        assert!(files.iter().any(|file| file.extension == "Core"));
        assert!(files.iter().any(|file| file.extension == "date"));
        assert!(
            files
                .iter()
                .all(|file| file.path.extension().unwrap() == "php")
        );
    }

    #[test]
    fn indexes_runtime_symbols_and_signatures() {
        let (index, report) = load_fixture();
        assert_eq!(report.files_discovered, 9);
        assert_eq!(report.files_parsed, 8);
        assert_eq!(report.symbols_indexed, 24);
        assert_eq!(report.errors.len(), 1);
        assert!(!report.elapsed.is_zero());
        let strlen = index.find_function("STRLEN").unwrap();
        assert_eq!(strlen.origin, SymbolOrigin::PhpRuntime);
        assert_eq!(
            strlen
                .signature
                .as_ref()
                .unwrap()
                .declared_return_type
                .as_deref(),
            Some("int")
        );
        let parameter = &strlen.signature.as_ref().unwrap().parameters[0];
        assert_eq!(
            (parameter.name.as_str(), parameter.declared_type.as_deref()),
            ("string", Some("string"))
        );
        assert!(index.find_function("array_map").is_some());
        assert_eq!(
            index
                .find_function("array_map")
                .unwrap()
                .availability
                .since
                .as_deref(),
            Some("7.0")
        );
        assert!(index.find_function("json_encode").is_some());
        assert!(index.find_class("PDO").is_some());
        assert!(index.find_class("PDOStatement").is_some());
        assert!(index.find_class("ReflectionClass").is_some());
        assert!(index.find_class("ArrayIterator").is_some());
    }

    #[test]
    fn indexes_classes_members_properties_and_constants() {
        let (index, _) = load_fixture();
        let date_time = index.find_class("datetime").unwrap();
        assert_eq!(date_time.extension, "date");
        assert!(index.find_class("DateTimeImmutable").is_some());
        assert!(index.find_class("DateInterval").is_some());
        let members = index.members_of("DateTime");
        assert!(
            members
                .iter()
                .any(|symbol| symbol.kind == SymbolKind::Method && symbol.name == "format")
        );
        assert!(
            members
                .iter()
                .any(|symbol| symbol.kind == SymbolKind::Property && symbol.name == "timezone")
        );
        let timezone = members
            .iter()
            .find(|symbol| symbol.kind == SymbolKind::Property && symbol.name == "timezone")
            .unwrap();
        assert_eq!(timezone.declared_type.as_deref(), Some("string"));
        assert_eq!(
            timezone
                .documentation
                .as_ref()
                .and_then(|doc| doc.var_type.as_deref()),
            Some("non-empty-string")
        );
        assert!(
            members
                .iter()
                .any(|symbol| symbol.kind == SymbolKind::ClassConstant && symbol.name == "ATOM")
        );
        assert!(index.find_constant("PHP_VERSION_ID").is_some());
    }

    #[test]
    fn indexes_all_declaration_kinds_and_namespaces() {
        let (index, _) = load_fixture();
        assert_eq!(
            index.find_class("foo\\bar\\baz").unwrap().kind,
            SymbolKind::Class
        );
        assert_eq!(
            index.find_class("Foo\\Bar\\Contract").unwrap().kind,
            SymbolKind::Interface
        );
        assert_eq!(
            index.find_class("Foo\\Bar\\Shared").unwrap().kind,
            SymbolKind::Trait
        );
        assert_eq!(
            index.find_class("Foo\\Bar\\Status").unwrap().kind,
            SymbolKind::Enum
        );
    }

    #[test]
    fn phpdoc_preserves_structured_information() {
        let (index, _) = load_fixture();
        let symbol = index.find_function("strlen").unwrap();
        let doc = symbol.documentation.as_ref().unwrap();
        assert_eq!(doc.params[0].declared_type.as_deref(), Some("string"));
        assert_eq!(doc.return_type.as_deref(), Some("int<0, max>"));
        assert_eq!(doc.throws, vec!["ValueError"]);
        assert!(doc.deprecated.is_some());
        assert!(doc.description.as_deref().unwrap().contains("length"));
        assert_eq!(
            symbol
                .signature
                .as_ref()
                .unwrap()
                .phpdoc_return_type
                .as_deref(),
            Some("int<0, max>")
        );
    }

    #[test]
    fn duplicates_are_retained_and_constants_remain_case_sensitive() {
        let (index, _) = load_fixture();
        assert_eq!(index.function_definitions("strlen").len(), 2);
        assert!(index.find_function("StRlEn").is_some());
        assert!(index.find_constant("php_version_id").is_none());
    }

    #[test]
    fn missing_stub_directory_is_controlled() {
        let error = StubProvider::new(fixture_root().join("missing"))
            .load()
            .unwrap_err();
        assert!(matches!(error, StubProviderError::MissingDirectory(_)));
    }

    #[test]
    fn unicode_phpdoc_and_symbols_are_preserved() {
        let symbols = extract_symbols(
            "<?php namespace Olá; /** Ação 👋 */ function informação(string $ação): void {}",
            Path::new("unicode.php"),
            "Core",
        )
        .unwrap();
        assert!(symbols.iter().any(|symbol| symbol.fqn == "Olá\\informação"));
    }
}
