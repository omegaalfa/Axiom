use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io,
    path::{Component, Path, PathBuf},
};

use serde::Serialize;

use crate::{extract_symbols, EmbeddedStubArtifact, Symbol, SymbolKind};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PhpStormStubsMap {
    pub classes: BTreeMap<String, String>,
    pub functions: BTreeMap<String, String>,
    pub constants: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct GenerationReport {
    pub files_processed: usize,
    pub symbols_generated: usize,
    pub duplicate_symbols_removed: usize,
    pub classes: usize,
    pub interfaces: usize,
    pub traits: usize,
    pub enums: usize,
    pub methods: usize,
    pub properties: usize,
    pub class_constants: usize,
    pub functions: usize,
    pub global_constants: usize,
}

#[derive(Debug)]
pub enum StubGenerationError {
    Io { path: PathBuf, source: io::Error },
    InvalidMap { section: &'static str, message: String },
    InvalidStubPath(String),
    MissingStubFile(PathBuf),
    ParseStub { path: PathBuf, message: String },
    Serialize(serde_json::Error),
    BinarySerialize(String),
    Write { path: PathBuf, source: io::Error },
}

impl std::fmt::Display for StubGenerationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(formatter, "failed to read {}: {source}", path.display())
            }
            Self::InvalidMap { section, message } => {
                write!(formatter, "invalid PhpStormStubsMap {section}: {message}")
            }
            Self::InvalidStubPath(path) => {
                write!(formatter, "invalid phpstorm-stubs path: {path}")
            }
            Self::MissingStubFile(path) => {
                write!(formatter, "phpstorm-stubs file is missing: {}", path.display())
            }
            Self::ParseStub { path, message } => {
                write!(formatter, "failed to parse {}: {message}", path.display())
            }
            Self::Serialize(error) => write!(formatter, "failed to serialize stub artifact: {error}"),
            Self::BinarySerialize(error) => write!(formatter, "failed to serialize binary stub artifact: {error}"),
            Self::Write { path, source } => {
                write!(formatter, "failed to write {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for StubGenerationError {}

pub fn parse_phpstorm_stubs_map(content: &str) -> Result<PhpStormStubsMap, StubGenerationError> {
    Ok(PhpStormStubsMap {
        classes: parse_map_section(content, "CLASSES")?,
        functions: parse_map_section(content, "FUNCTIONS")?,
        constants: parse_map_section(content, "CONSTANTS")?,
    })
}

pub fn generate_runtime_stub_artifact(
    source_root: &Path,
    map_path: &Path,
    output_path: &Path,
) -> Result<GenerationReport, StubGenerationError> {
    let map_content = fs::read_to_string(map_path).map_err(|source| StubGenerationError::Io {
        path: map_path.to_path_buf(),
        source,
    })?;
    let map = parse_phpstorm_stubs_map(&map_content)?;
    let (artifact, report) = generate_runtime_stub_artifact_bytes(source_root, map)?;
    let bytes = serde_json::to_vec(&artifact).map_err(StubGenerationError::Serialize)?;
    write_atomic(output_path, &bytes)?;
    Ok(report)
}

pub fn generate_runtime_stub_artifact_binary(
    source_root: &Path,
    map_path: &Path,
    output_path: &Path,
) -> Result<GenerationReport, StubGenerationError> {
    let map_content = fs::read_to_string(map_path).map_err(|source| StubGenerationError::Io {
        path: map_path.to_path_buf(),
        source,
    })?;
    let map = parse_phpstorm_stubs_map(&map_content)?;
    let (artifact, report) = generate_runtime_stub_artifact_bytes(source_root, map)?;
    let bytes = bincode::serialize(&artifact)
        .map_err(|error| StubGenerationError::BinarySerialize(error.to_string()))?;
    write_atomic(output_path, &bytes)?;
    Ok(report)
}

pub fn generate_runtime_stub_artifact_bytes(
    source_root: &Path,
    map: PhpStormStubsMap,
) -> Result<(EmbeddedStubArtifact, GenerationReport), StubGenerationError> {
    let mut referenced_files = BTreeSet::new();
    for path in map
        .classes
        .values()
        .chain(map.functions.values())
        .chain(map.constants.values())
    {
        let relative = validate_stub_path(path)?;
        referenced_files.insert(relative);
    }

    let mut candidates = BTreeMap::new();
    let mut files_processed = 0;
    let mut symbols_seen: usize = 0;
    for relative in referenced_files {
        let full_path = source_root.join(&relative);
        let text = fs::read_to_string(&full_path).map_err(|source| {
            if source.kind() == io::ErrorKind::NotFound {
                StubGenerationError::MissingStubFile(full_path.clone())
            } else {
                StubGenerationError::Io {
                    path: full_path.clone(),
                    source,
                }
            }
        })?;
        let extension = relative
            .components()
            .next()
            .and_then(|component| component.as_os_str().to_str())
            .unwrap_or("Core")
            .to_owned();
        let symbols = extract_symbols(&text, &relative, &extension)
            .map_err(|message| StubGenerationError::ParseStub {
                path: full_path,
                message,
            })?;
        files_processed += 1;
        for symbol in symbols {
            symbols_seen += 1;
            candidates
                .entry(symbol_identity(&symbol))
                .and_modify(|current| {
                    if symbol_candidate_rank(&symbol) < symbol_candidate_rank(current) {
                        *current = symbol.clone();
                    }
                })
                .or_insert(symbol);
        }
    }

    let duplicate_symbols_removed = symbols_seen.saturating_sub(candidates.len());
    let mut symbols: Vec<Symbol> = candidates.into_values().collect();
    symbols.sort_by(|left, right| symbol_sort_key(left).cmp(&symbol_sort_key(right)));
    let mut report = GenerationReport {
        files_processed,
        symbols_generated: symbols.len(),
        duplicate_symbols_removed,
        ..Default::default()
    };
    for symbol in &symbols {
        match symbol.kind {
            SymbolKind::Class => report.classes += 1,
            SymbolKind::Interface => report.interfaces += 1,
            SymbolKind::Trait => report.traits += 1,
            SymbolKind::Enum => report.enums += 1,
            SymbolKind::Method => report.methods += 1,
            SymbolKind::Property => report.properties += 1,
            SymbolKind::ClassConstant => report.class_constants += 1,
            SymbolKind::Function => report.functions += 1,
            SymbolKind::GlobalConstant => report.global_constants += 1,
        }
    }
    Ok((EmbeddedStubArtifact::new(symbols), report))
}

fn parse_map_section(
    content: &str,
    section: &'static str,
) -> Result<BTreeMap<String, String>, StubGenerationError> {
    let marker = format!("const {section} = array (");
    let start = content.find(&marker).ok_or_else(|| {
        StubGenerationError::InvalidMap {
            section,
            message: "section marker is missing".into(),
        }
    })? + marker.len();
    let mut entries = BTreeMap::new();
    for line in content[start..].lines() {
        let trimmed = line.trim();
        if trimmed == ");" {
            return Ok(entries);
        }
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("/*") {
            continue;
        }
        if let Some((name, path)) = parse_map_entry(trimmed) {
            entries.insert(name, path);
        } else {
            return Err(StubGenerationError::InvalidMap {
                section,
                message: format!("unsupported entry: {trimmed}"),
            });
        }
    }
    Err(StubGenerationError::InvalidMap {
        section,
        message: "section terminator is missing".into(),
    })
}

fn parse_map_entry(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim().trim_end_matches(',');
    let (name, path) = trimmed.split_once(" => ")?;
    let name = name.trim().strip_prefix('\'')?.strip_suffix('\'')?;
    let path = path.trim().strip_prefix('\'')?.strip_suffix('\'')?;
    Some((php_unescape_single_quoted(name), php_unescape_single_quoted(path)))
}

fn php_unescape_single_quoted(value: &str) -> String {
    value
        .replace("\\\\", "\u{0}")
        .replace("\\'", "'")
        .replace('\u{0}', "\\")
}

fn validate_stub_path(value: &str) -> Result<PathBuf, StubGenerationError> {
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(StubGenerationError::InvalidStubPath(value.to_owned()));
    }
    Ok(path.to_path_buf())
}

fn symbol_identity(symbol: &Symbol) -> (u8, String, String) {
    let exact = matches!(
        symbol.kind,
        SymbolKind::ClassConstant | SymbolKind::GlobalConstant
    );
    let fqn_key = if exact {
        symbol.fqn.clone()
    } else {
        symbol.fqn.to_lowercase()
    };
    (symbol_kind_rank(symbol.kind), fqn_key, symbol.name.clone())
}

fn symbol_candidate_rank(symbol: &Symbol) -> (String, usize, usize, String, String) {
    (
        symbol.location.file.to_string_lossy().into_owned(),
        symbol.location.range.start,
        symbol.location.range.end,
        symbol.name.clone(),
        format!("{symbol:?}"),
    )
}

fn symbol_sort_key(symbol: &Symbol) -> (u8, String, String, String, String, usize, usize) {
    (
        symbol_kind_rank(symbol.kind),
        symbol.fqn.to_lowercase(),
        symbol.fqn.clone(),
        symbol.name.clone(),
        symbol.location.file.to_string_lossy().into_owned(),
        symbol.location.range.start,
        symbol.location.range.end,
    )
}

fn symbol_kind_rank(kind: SymbolKind) -> u8 {
    match kind {
        SymbolKind::Class => 0,
        SymbolKind::Interface => 1,
        SymbolKind::Trait => 2,
        SymbolKind::Enum => 3,
        SymbolKind::Function => 4,
        SymbolKind::Method => 5,
        SymbolKind::Property => 6,
        SymbolKind::ClassConstant => 7,
        SymbolKind::GlobalConstant => 8,
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), StubGenerationError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| StubGenerationError::Write {
        path: parent.to_path_buf(),
        source,
    })?;
    let temp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("runtime-stubs.json"),
        std::process::id()
    ));
    fs::write(&temp, bytes).map_err(|source| StubGenerationError::Write {
        path: temp.clone(),
        source,
    })?;
    if path.exists() {
        fs::remove_file(path).map_err(|source| StubGenerationError::Write {
            path: path.to_path_buf(),
            source,
        })?;
    }
    fs::rename(&temp, path).map_err(|source| StubGenerationError::Write {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::{Parameter, Signature, SourceLocation, SymbolOrigin};

    #[test]
    fn generation_is_deterministic_and_deduplicates_by_stable_source_order() {
        let root = tempdir().unwrap();
        fs::write(
            root.path().join("z.php"),
            "<?php class Duplicate { public function z(): void {} }",
        )
        .unwrap();
        fs::write(
            root.path().join("a.php"),
            "<?php class Duplicate { public function a(): void {} }",
        )
        .unwrap();
        let map = parse_phpstorm_stubs_map(concat!(
            "const CLASSES = array (\n",
            "  'DuplicateZ' => 'z.php',\n",
            "  'DuplicateA' => 'a.php',\n",
            ");\n",
            "const FUNCTIONS = array (\n",
            ");\n",
            "const CONSTANTS = array (\n",
            ");\n",
        ))
        .unwrap();

        let (first, first_report) =
            generate_runtime_stub_artifact_bytes(root.path(), map.clone()).unwrap();
        let (second, second_report) =
            generate_runtime_stub_artifact_bytes(root.path(), map).unwrap();

        assert_eq!(first.symbols, second.symbols);
        assert_eq!(first_report, second_report);
        assert_eq!(first.symbols.len(), 3);
        assert_eq!(first_report.duplicate_symbols_removed, 1);
        assert_eq!(first.symbols[0].location.file, PathBuf::from("a.php"));
    }

    #[test]
    fn failed_generation_does_not_replace_existing_artifact() {
        let root = tempdir().unwrap();
        let output = root.path().join("runtime-stubs.json");
        fs::write(&output, "existing").unwrap();
        let map = root.path().join("invalid-map.php");
        fs::write(&map, "const CLASSES = array (\n").unwrap();

        assert!(generate_runtime_stub_artifact(root.path(), &map, &output).is_err());
        assert_eq!(fs::read_to_string(&output).unwrap(), "existing");
    }

    #[test]
    fn axiom_map_parses_sections_deduplicates_and_orders_paths() {
        let map = parse_phpstorm_stubs_map(concat!(
            "namespace AxiomStub; final class AxiomStubsMap {\n",
            "const CLASSES = array (\n",
            "  'B' => 'z/B.php',\n",
            "  'A' => 'a/A.php',\n",
            "  'A2' => 'a/A.php',\n",
            ");\n",
            "const FUNCTIONS = array (\n",
            "  'f' => 'z/B.php',\n",
            ");\n",
            "const CONSTANTS = array (\n",
            "  'C' => 'c/C.php',\n",
            ");\n",
            "}"
        ))
        .unwrap();
        let paths = map
            .classes
            .values()
            .chain(map.functions.values())
            .chain(map.constants.values())
            .map(PathBuf::from)
            .collect::<BTreeSet<_>>();
        assert_eq!(paths.into_iter().collect::<Vec<_>>(), vec![
            PathBuf::from("a/A.php"),
            PathBuf::from("c/C.php"),
            PathBuf::from("z/B.php"),
        ]);
    }

    #[test]
    fn generator_processes_only_files_referenced_by_map_and_rejects_missing_files() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("mapped.php"), "<?php class Mapped {}") .unwrap();
        fs::write(root.path().join("unmapped.php"), "<?php class Unmapped {}") .unwrap();
        let map = parse_phpstorm_stubs_map(concat!(
            "const CLASSES = array (\n",
            "  'Mapped' => 'mapped.php',\n",
            ");\n",
            "const FUNCTIONS = array (\n",
            ");\n",
            "const CONSTANTS = array (\n",
            ");\n",
        )).unwrap();
        let (_, report) = generate_runtime_stub_artifact_bytes(root.path(), map).unwrap();
        assert_eq!(report.files_processed, 1);
        let invalid = parse_phpstorm_stubs_map(concat!(
            "const CLASSES = array (\n",
            "  'Missing' => 'missing.php',\n",
            ");\n",
            "const FUNCTIONS = array (\n",
            ");\n",
            "const CONSTANTS = array (\n",
            ");\n",
        )).unwrap();
        assert!(matches!(
            generate_runtime_stub_artifact_bytes(root.path(), invalid),
            Err(StubGenerationError::MissingStubFile(_))
        ));
    }

    #[test]
    fn diagnostic_generator_preserves_pdo_from_map_to_json_round_trip() {
        let root = tempdir().unwrap();
        let pdo_dir = root.path().join("PDO");
        fs::create_dir_all(&pdo_dir).unwrap();
        fs::write(
            root.path().join("PhpStormStubsMap.php"),
            concat!(
                "const CLASSES = array (\n",
                "  'PDO' => 'PDO/PDO.php',\n",
                ");\n",
                "const FUNCTIONS = array (\n",
                ");\n",
                "const CONSTANTS = array (\n",
                ");\n",
            ),
        )
        .unwrap();
        let pdo_source = r#"<?php
namespace {
    const PDO_ATTR_AUTOCOMMIT = 0;
    class PDOException extends RuntimeException {}
    class PDO
    {
        public const ATTR_AUTOCOMMIT = 0;
        public function __construct(string $dsn = "") {}
    }
}
"#;
        let pdo_path = pdo_dir.join("PDO.php");
        fs::write(&pdo_path, pdo_source).unwrap();

        let extracted = crate::extract_symbols(pdo_source, Path::new("PDO/PDO.php"), "PDO")
            .expect("PDO fixture should extract");
        assert!(extracted
            .iter()
            .any(|symbol| symbol.fqn == "PDO" && symbol.kind == SymbolKind::Class));

        let output = root.path().join("runtime-stubs.json");
        let report = generate_runtime_stub_artifact(
            root.path(),
            &root.path().join("PhpStormStubsMap.php"),
            &output,
        )
        .unwrap();
        assert_eq!(report.files_processed, 1);
        let bytes = fs::read(&output).unwrap();
        let artifact: EmbeddedStubArtifact = serde_json::from_slice(&bytes).unwrap();
        assert!(artifact
            .symbols
            .iter()
            .any(|symbol| symbol.fqn == "PDO" && symbol.kind == SymbolKind::Class));

        let round_trip = serde_json::from_slice::<EmbeddedStubArtifact>(&bytes).unwrap();
        assert!(round_trip
            .symbols
            .iter()
            .any(|symbol| symbol.fqn == "PDO" && symbol.kind == SymbolKind::Class));
    }

    #[test]
    fn symbol_round_trip_preserves_signature_doc_and_availability() {
        let symbol = Symbol {
            name: "run".into(),
            fqn: "RoundTrip::run".into(),
            kind: SymbolKind::Method,
            origin: SymbolOrigin::PhpRuntime,
            extension: "Core".into(),
            location: SourceLocation {
                file: PathBuf::from("Core/Core.php"),
                range: 10..13,
            },
            declared_type: None,
            signature: Some(Signature {
                parameters: vec![Parameter {
                    name: "name".into(),
                    declared_type: Some("string".into()),
                    phpdoc_type: Some("non-empty-string".into()),
                    optional: false,
                    variadic: false,
                    by_reference: false,
                }],
                declared_return_type: Some("bool".into()),
                phpdoc_return_type: Some("true".into()),
            }),
            documentation: Some(crate::PhpDoc {
                description: Some("Run.".into()),
                ..Default::default()
            }),
            availability: crate::Availability {
                since: Some("8.0".into()),
                until: None,
            },
            is_static: true,
        };
        let artifact = EmbeddedStubArtifact::new(vec![symbol.clone()]);
        let bytes = serde_json::to_vec(&artifact).unwrap();
        let decoded: EmbeddedStubArtifact = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded.symbols, vec![symbol]);
    }
}
