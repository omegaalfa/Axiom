//! Project-local PHP symbol index.
//!
//! This crate is deliberately headless. It owns no UI or LSP state and can be
//! queried from completion/navigation providers without blocking rendering.

use axiom_syntax::PhpSyntax;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use tree_sitter::Node;

mod semantic;
mod source;

pub use semantic::{
    BuiltinType, DeclaredType, DefinitionCandidate, DefinitionConfidence, DefinitionLocation,
    DefinitionQueryContext, DefinitionResult, DefinitionSyntaxContext, Expression,
    ExpressionResolver, FileId, FileRecord, FindUsagesOptions, FindUsagesResult, FindUsagesStatus,
    ImportBinding, ImportKind, ImportTable, InaccessibilityInfo, InaccessibilityReason,
    InterfaceRelationIndexes, MemberAccess, MemberKind, MemberResolution, MemberResolver,
    PersistentFileKey, PersistentSymbolKey, ReferenceConfidence, ReferenceId, ReferenceLocation,
    ReferenceProvider, ReferenceRole, ReferenceTarget, Scope, ScopeId, ScopeKind, ScopeStore,
    SemanticDefinitionOutcome, SemanticDefinitionResult, SemanticEngine, SemanticParameter,
    SemanticReference, SemanticRevision, SemanticSnapshot, SemanticSymbol, SnapshotBuilder,
    SourceOrigin, SymbolId, TypeCompatibility, UsageLocation, VariableBinding,
    declared_type_compatibility, declared_type_label,
};
pub use source::{
    ComposerSource, DeferredSource, RuntimeSource, SourceCandidate, SourceError, SourceFile,
    SourceFingerprint, SourceKind, SourceProvider, SourceRegistry, SourceResolution,
    SourceResolutionPolicy, WorkspaceSource,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
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
    EnumCase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Visibility {
    Public,
    Protected,
    Private,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSymbol {
    pub name: String,
    pub fully_qualified_name: String,
    pub kind: ProjectSymbolKind,
    pub file: PathBuf,
    pub range: std::ops::Range<usize>,
    pub namespace: String,
    pub visibility: Visibility,
    pub modifiers: Vec<String>,
    pub parameters: Option<String>,
    pub return_type: Option<String>,
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
    prefix_names: BTreeMap<String, Vec<usize>>,
    prefix_fqns: BTreeMap<String, Vec<usize>>,
    ready: bool,
}

// Bumped when symbol FQNs/owner indexing change so stale caches cannot keep
// invalid member names (for example the old `\\Base::save` global FQN).
const PROJECT_CACHE_SCHEMA: u32 = 3;

#[derive(Debug, Serialize, Deserialize)]
struct ProjectCacheFile {
    schema_version: u32,
    files: BTreeMap<PathBuf, (u64, u128, Vec<ProjectSymbol>)>,
}

/// Composer metadata index. It records class locations without walking all of
/// `vendor/`; declarations are parsed only when a class is queried.
#[derive(Debug, Default, Clone)]
pub struct VendorSymbolIndex {
    /// Canonical project vendor directory. Composer mappings outside this
    /// directory belong to the workspace and must not be exposed as Vendor.
    vendor_root: Option<PathBuf>,
    classmap: BTreeMap<String, PathBuf>,
    class_names: BTreeMap<String, Vec<String>>,
    psr4: Vec<(String, Vec<PathBuf>)>,
    parsed: BTreeMap<String, Vec<ProjectSymbol>>,
    parsed_files: BTreeMap<String, (PathBuf, u64, u128)>,
}

const VENDOR_CACHE_SCHEMA: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VendorMetadataCache {
    schema_version: u32,
    classmap: BTreeMap<String, PathBuf>,
    psr4: Vec<(String, Vec<PathBuf>)>,
    fingerprint: Vec<(PathBuf, u64, u128)>,
}

impl VendorSymbolIndex {
    pub fn load(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = root.as_ref();
        let started = Instant::now();
        let composer = root.join("vendor/composer");
        let mut index = Self {
            vendor_root: Some(canonical_or(root.join("vendor"))),
            ..Self::default()
        };
        let classmap = composer.join("autoload_classmap.php");
        if let Ok(text) = fs::read_to_string(&classmap) {
            for line in text.lines() {
                let Some((left, right)) = line.split_once("=>") else {
                    continue;
                };
                let Some(fqn) = left.split('\'').nth(1) else {
                    continue;
                };
                let Some(file) = right.split('\'').nth(1) else {
                    continue;
                };
                let path = if right.contains("$vendorDir") || right.contains("$baseDir") {
                    composer_path(root, right.trim())
                } else {
                    composer_path(root, file)
                };
                if path.is_file() {
                    index
                        .classmap
                        .insert(fqn.replace("\\\\", "\\"), canonical_or(path));
                }
            }
        }
        for metadata in ["autoload_psr4.php", "autoload_static.php"] {
            let psr4 = composer.join(metadata);
            if let Ok(text) = fs::read_to_string(&psr4) {
                index.parse_psr4_php(root, &text);
            }
        }
        // Composer metadata is PHP syntax, not data; the JSON fallback keeps
        // this loader safe when generated files are absent or stale.
        if let Ok(text) = fs::read_to_string(root.join("composer.json")) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(map) = json.pointer("/autoload/psr-4").and_then(|v| v.as_object()) {
                    for (prefix, value) in map {
                        let values: Vec<&str> = value
                            .as_array()
                            .map(|array| array.iter().filter_map(|v| v.as_str()).collect())
                            .unwrap_or_else(|| value.as_str().into_iter().collect());
                        for base in values {
                            let path = root.join(base);
                            if path.is_dir()
                                && !index.psr4.iter().any(|(p, bases)| {
                                    p == prefix && bases.contains(&canonical_or(path.clone()))
                                })
                            {
                                index.psr4.push((prefix.clone(), vec![canonical_or(path)]));
                            }
                        }
                    }
                }
            }
        }
        if std::env::var_os("AXIOM_DEBUG_COMPOSER").is_some() {
            eprintln!(
                "[COMPOSER LOAD] project_root={:?} vendor_dir={:?} classmap_file={:?} psr4_file={:?} classmap_entries={} psr4_prefixes={}",
                root,
                composer.parent().unwrap_or(&composer),
                classmap,
                composer.join("autoload_psr4.php"),
                index.classmap.len(),
                index.psr4.len()
            );
            let metadata_files = [
                "autoload_classmap.php",
                "autoload_psr4.php",
                "autoload_static.php",
            ]
            .into_iter()
            .filter(|name| composer.join(name).is_file())
            .count();
            eprintln!(
                "[VENDOR STARTUP] metadata_files={metadata_files} php_files_parsed=0 elapsed_ms={}",
                started.elapsed().as_millis()
            );
        }
        index.rebuild_class_name_index();
        Ok(index)
    }

    pub fn load_cached(root: impl AsRef<Path>, cache_path: impl AsRef<Path>) -> io::Result<Self> {
        let root = root.as_ref();
        let cache_path = cache_path.as_ref();
        let fingerprint = metadata_fingerprint(root);
        let started = Instant::now();
        let cache_exists = cache_path.is_file();
        if let Ok(text) = fs::read_to_string(cache_path)
            && let Ok(cache) = serde_json::from_str::<VendorMetadataCache>(&text)
            && cache.schema_version == VENDOR_CACHE_SCHEMA
            && cache.fingerprint == fingerprint
        {
            if std::env::var_os("AXIOM_DEBUG_COMPOSER").is_some() {
                eprintln!(
                    "[VENDOR CACHE] hit=true reason=metadata-unchanged load_ms={}",
                    started.elapsed().as_millis()
                );
            }
            let mut index = Self {
                vendor_root: Some(canonical_or(root.join("vendor"))),
                classmap: cache.classmap,
                class_names: BTreeMap::new(),
                psr4: cache.psr4,
                parsed: BTreeMap::new(),
                parsed_files: BTreeMap::new(),
            };
            index.rebuild_class_name_index();
            return Ok(index);
        }
        let index = Self::load(root)?;
        if let Some(parent) = cache_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let cache = VendorMetadataCache {
            schema_version: VENDOR_CACHE_SCHEMA,
            classmap: index.classmap.clone(),
            psr4: index.psr4.clone(),
            fingerprint,
        };
        let _ = fs::write(cache_path, serde_json::to_vec(&cache).unwrap_or_default());
        if std::env::var_os("AXIOM_DEBUG_COMPOSER").is_some() {
            eprintln!(
                "[VENDOR CACHE] hit=false reason={} load_ms={}",
                if cache_exists {
                    "metadata-changed-or-schema"
                } else {
                    "missing"
                },
                started.elapsed().as_millis()
            );
        }
        Ok(index)
    }

    fn parse_psr4_php(&mut self, root: &Path, text: &str) {
        for line in text.lines() {
            let Some((left, right)) = line.split_once("=>") else {
                continue;
            };
            let Some(prefix) = left.split('\'').nth(1) else {
                continue;
            };
            let paths: Vec<PathBuf> = if right.contains("$vendorDir") || right.contains("$baseDir")
            {
                right
                    .split(',')
                    .filter(|part| part.contains("$vendorDir") || part.contains("$baseDir"))
                    .map(|part| composer_path(root, part))
                    .collect()
            } else {
                right
                    .split('\'')
                    .skip(1)
                    .step_by(2)
                    .map(|base| composer_path(root, base))
                    .collect()
            };
            let prefix = prefix.replace("\\\\", "\\");
            for path in paths {
                if path.is_dir() {
                    let path = canonical_or(path);
                    if std::env::var_os("AXIOM_DEBUG_COMPOSER").is_some() {
                        eprintln!("[COMPOSER PREFIX] prefix={prefix} directories=[{:?}]", path);
                    }
                    if let Some((_, bases)) = self.psr4.iter_mut().find(|(p, _)| *p == prefix) {
                        if !bases.contains(&path) {
                            bases.push(path);
                        }
                    } else {
                        self.psr4.push((prefix.clone(), vec![path]));
                    }
                }
            }
        }
    }

    /// Returns every Composer metadata candidate for a class. The most
    /// specific PSR-4 prefix is selected, while all directories registered
    /// for that prefix are retained so callers can surface ambiguity.
    pub fn resolve_class_candidates(&self, fqn: &str) -> Vec<PathBuf> {
        let is_vendor_path = |path: &Path| {
            let Some(root) = &self.vendor_root else {
                return true;
            };
            canonical_or(path.to_path_buf()).starts_with(root)
        };
        if let Some(path) = self.classmap.get(fqn) {
            if std::env::var_os("AXIOM_DEBUG_COMPOSER").is_some() {
                eprintln!(
                    "[VENDOR RESOLVE] fqn={fqn} classmap_match=true psr4_prefix_match= candidate_path={path:?} exists={} result={}",
                    path.is_file(),
                    path.is_file()
                );
            }
            return is_vendor_path(path)
                .then(|| path.clone())
                .into_iter()
                .collect();
        }
        let Some((prefix, tail, _)) = self
            .psr4
            .iter()
            .filter_map(|(prefix, bases)| {
                fqn.strip_prefix(prefix).map(|tail| (prefix, tail, bases))
            })
            .max_by_key(|(prefix, _, _)| prefix.len())
        else {
            return Vec::new();
        };
        let relative = tail
            .trim_start_matches('\\')
            .replace('\\', std::path::MAIN_SEPARATOR_STR);
        let mut candidates = Vec::new();
        let Some((_, bases)) = self.psr4.iter().find(|(p, _)| p == prefix) else {
            return candidates;
        };
        for base in bases {
            let path = base.join(format!("{relative}.php"));
            if std::env::var_os("AXIOM_DEBUG_COMPOSER").is_some() {
                eprintln!(
                    "[VENDOR RESOLVE] fqn={fqn} classmap_match=false psr4_prefix_match={prefix} candidate_path={:?} exists={} result={}",
                    path,
                    path.is_file(),
                    path.is_file()
                );
            }
            if path.is_file() && is_vendor_path(&path) {
                let path = canonical_or(path);
                if !candidates.contains(&path) {
                    candidates.push(path);
                }
            }
        }
        candidates
    }

    pub fn resolve_class(&self, fqn: &str) -> Option<PathBuf> {
        self.resolve_class_candidates(fqn).into_iter().next()
    }

    /// Reports whether Composer metadata can account for a class without
    /// probing or parsing its source file. This is suitable for lightweight
    /// editor diagnostics while the actual resolution remains asynchronous.
    pub fn has_class_metadata(&self, fqn: &str) -> bool {
        self.classmap.contains_key(fqn)
            || self.psr4.iter().any(|(prefix, _)| {
                fqn == prefix.trim_end_matches('\\')
                    || fqn.starts_with(&format!("{}\\", prefix.trim_end_matches('\\')))
            })
    }

    /// Stable metadata-only fingerprint used by source providers to decide
    /// whether Composer resolution needs to be refreshed.
    pub fn metadata_fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.classmap.hash(&mut hasher);
        self.psr4.hash(&mut hasher);
        hasher.finish()
    }

    pub fn symbols_of(&mut self, fqn: &str) -> Vec<ProjectSymbol> {
        let mut in_progress = HashSet::new();
        self.symbols_of_inner(fqn, &mut in_progress)
    }

    fn symbols_of_inner(
        &mut self,
        fqn: &str,
        in_progress: &mut HashSet<String>,
    ) -> Vec<ProjectSymbol> {
        if !in_progress.insert(fqn.to_owned()) {
            return Vec::new();
        }
        let result = self.symbols_of_inner_impl(fqn, in_progress);
        in_progress.remove(fqn);
        result
    }

    fn symbols_of_inner_impl(
        &mut self,
        fqn: &str,
        in_progress: &mut HashSet<String>,
    ) -> Vec<ProjectSymbol> {
        let started = Instant::now();
        let Some(path) = self.resolve_class(fqn) else {
            return Vec::new();
        };
        if let Some((cached_path, size, modified)) = self.parsed_files.get(fqn)
            && cached_path == &path
            && let Ok(metadata) = fs::metadata(&path)
            && metadata.len() == *size
            && metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|value| value.as_nanos())
                == Some(*modified)
            && let Some(symbols) = self.parsed.get(fqn)
        {
            if std::env::var_os("AXIOM_DEBUG_COMPOSER").is_some() {
                eprintln!(
                    "[VENDOR SYMBOL CACHE] hit=true fqn={fqn} elapsed_ms={}",
                    started.elapsed().as_millis()
                );
            }
            return symbols.clone();
        }
        if std::env::var_os("AXIOM_DEBUG_COMPOSER").is_some() {
            eprintln!("[DEFINITION VENDOR PARSE START] fqn={fqn} path={:?}", path);
        }
        let Ok(text) = fs::read_to_string(&path) else {
            return Vec::new();
        };
        let mut symbols = Vec::new();
        let namespace = text
            .lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("namespace ")
                    .map(|v| v.trim_end_matches(';').trim().to_owned())
            })
            .unwrap_or_default();
        let (_, imports) = vendor_trait_info(&text);
        for (offset, line) in text.split_inclusive('\n').scan(0usize, |offset, line| {
            let start = *offset;
            *offset += line.len();
            Some((start, line))
        }) {
            let trimmed = line.trim_start();
            for (keyword, kind) in [
                ("class ", ProjectSymbolKind::Class),
                ("interface ", ProjectSymbolKind::Interface),
                ("trait ", ProjectSymbolKind::Trait),
                ("enum ", ProjectSymbolKind::Enum),
            ] {
                if let Some(pos) = trimmed.find(keyword) {
                    let name = trimmed[pos + keyword.len()..]
                        .split(|c: char| !c.is_alphanumeric() && c != '_')
                        .next()
                        .unwrap_or_default();
                    if !name.is_empty() {
                        let start = offset + line.len() - trimmed.len() + pos + keyword.len();
                        symbols.push(ProjectSymbol {
                            name: name.to_owned(),
                            fully_qualified_name: fqn.to_owned(),
                            kind,
                            file: path.clone(),
                            range: start..start + name.len(),
                            namespace: namespace.clone(),
                            visibility: Visibility::Unknown,
                            modifiers: vec!["composer".into()],
                            parameters: None,
                            return_type: None,
                        });
                    }
                    break;
                }
            }
            if let Some(pos) = trimmed.find("function ") {
                let start = offset + line.len() - trimmed.len() + pos + 9;
                let function_tail = &trimmed[pos + 9..];
                let name = function_tail
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()
                    .unwrap_or_default();
                if !name.is_empty() {
                    // Keep the declaration's return type in the vendor
                    // symbol metadata.  This is intentionally syntax-only:
                    // resolving imports, self/static/parent and unions is
                    // done by the semantic resolver with the file context.
                    let return_type = vendor_method_return_type(function_tail, name.len())
                        .map(|raw| normalize_vendor_type(&raw, &namespace, &imports));
                    symbols.push(ProjectSymbol {
                        name: name.to_owned(),
                        fully_qualified_name: format!("{fqn}::{name}"),
                        kind: ProjectSymbolKind::Method,
                        file: path.clone(),
                        range: start..start + name.len(),
                        namespace: namespace.clone(),
                        visibility: Visibility::Public,
                        modifiers: if trimmed[..pos].contains("static") {
                            vec!["static".into()]
                        } else {
                            Vec::new()
                        },
                        parameters: trimmed[pos + 9 + name.len()..].find('(').and_then(|open| {
                            trimmed[pos + 9 + name.len() + open..]
                                .find(')')
                                .map(|close| {
                                    trimmed[pos + 9 + name.len() + open
                                        ..=pos + 9 + name.len() + open + close]
                                        .to_owned()
                                })
                        }),
                        return_type,
                    });
                }
            }
            // Capture declared properties as well as methods. This is needed
            // for `$this->nextId` and similar accesses inside vendor traits.
            let has_visibility = ["public", "protected", "private", "var"]
                .iter()
                .any(|modifier| trimmed.split_whitespace().any(|word| word == *modifier));
            if has_visibility {
                if let Some(dollar) = trimmed.find('$') {
                    let name = trimmed[dollar + 1..]
                        .chars()
                        .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                        .collect::<String>();
                    if !name.is_empty() {
                        let start = offset + line.len() - trimmed.len() + dollar + 1;
                        symbols.push(ProjectSymbol {
                            name: name.clone(),
                            fully_qualified_name: format!("{fqn}::{name}"),
                            kind: ProjectSymbolKind::Property,
                            file: path.clone(),
                            range: start..start + name.len(),
                            namespace: namespace.clone(),
                            visibility: if trimmed.contains("private") {
                                Visibility::Private
                            } else if trimmed.contains("protected") {
                                Visibility::Protected
                            } else {
                                Visibility::Public
                            },
                            modifiers: vec!["composer".into()],
                            parameters: None,
                            return_type: None,
                        });
                    }
                }
            }
        }
        // Methods supplied by traits are callable on the consuming class.
        // Load only explicitly used traits, on the background worker, and
        // retain their symbols in the same class result for member lookup.
        let (trait_names, _) = vendor_trait_info(&text);
        for trait_name in trait_names {
            let trait_fqn = if trait_name.starts_with('\\') || trait_name.contains('\\') {
                trait_name.trim_start_matches('\\').to_owned()
            } else if let Some(imported) = imports.get(&trait_name) {
                imported.clone()
            } else if namespace.is_empty() {
                trait_name
            } else {
                format!("{namespace}\\{trait_name}")
            };
            let trait_symbols = self.symbols_of_inner(&trait_fqn, in_progress);
            symbols.extend(
                trait_symbols
                    .into_iter()
                    .filter(|symbol| symbol.kind == ProjectSymbolKind::Method),
            );
        }
        self.parsed.insert(fqn.to_owned(), symbols.clone());
        if let Ok(metadata) = fs::metadata(&path) {
            let modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|value| value.as_nanos())
                .unwrap_or_default();
            self.parsed_files
                .insert(fqn.to_owned(), (path, metadata.len(), modified));
        }
        if std::env::var_os("AXIOM_DEBUG_COMPOSER").is_some() {
            eprintln!(
                "[DEFINITION VENDOR PARSE END] fqn={fqn} symbols={} elapsed_ms={}",
                symbols.len(),
                started.elapsed().as_millis()
            );
            eprintln!("[VENDOR SYMBOL CACHE] hit=false fqn={fqn}");
        }
        symbols
    }

    /// Publishes parsed entries from an off-lock snapshot. Only cache maps are
    /// copied; Composer metadata remains unchanged and is read-only here.
    pub fn merge_parsed_cache(&mut self, snapshot: &Self) {
        self.parsed.extend(snapshot.parsed.clone());
        self.parsed_files.extend(snapshot.parsed_files.clone());
    }

    /// Returns symbols already parsed for a class without touching the
    /// filesystem. This is safe for UI-side completion when the lock is held
    /// only briefly by the caller.
    pub fn cached_symbols(&self, fqn: &str) -> Vec<ProjectSymbol> {
        self.parsed.get(fqn).cloned().unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.classmap.is_empty() && self.psr4.is_empty()
    }

    pub fn classes_matching(&self, prefix: &str) -> Vec<String> {
        let upper = format!("{prefix}\u{10ffff}");
        self.class_names
            .range(prefix.to_owned()..=upper)
            .flat_map(|(_, fqns)| fqns.iter())
            .cloned()
            .collect()
    }

    fn rebuild_class_name_index(&mut self) {
        self.class_names.clear();
        for fqn in self.classmap.keys() {
            let name = fqn.rsplit('\\').next().unwrap_or(fqn);
            self.class_names
                .entry(name.to_owned())
                .or_default()
                .push(fqn.clone());
        }
    }
}

fn canonical_or(path: PathBuf) -> PathBuf {
    fs::canonicalize(&path).unwrap_or(path)
}

/// Extracts only trait-use declarations from class-like bodies. Namespace
/// imports are represented by a different syntax node and must not trigger
/// recursive Vendor parsing.
/// Extracts a method return type without resolving it. Vendor parsing is kept
/// deliberately lightweight, but dropping this piece of declaration metadata
/// prevents semantic member chains from discovering the next receiver.
fn vendor_method_return_type(signature: &str, name_len: usize) -> Option<String> {
    let open = signature.get(name_len..)?.find('(')? + name_len;
    let bytes = signature.as_bytes();
    let mut depth = 0usize;
    let mut close = None;
    for (index, byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    close = Some(index);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    let suffix = signature.get(close + 1..)?.trim_start();
    let suffix = suffix.strip_prefix(':')?.trim_start();
    let end = suffix.find(['{', ';']).unwrap_or(suffix.len());
    let return_type = suffix[..end].trim();
    (!return_type.is_empty()).then(|| return_type.to_owned())
}

fn normalize_vendor_type(raw: &str, namespace: &str, imports: &BTreeMap<String, String>) -> String {
    let raw = raw.trim();
    if let Some(inner) = raw.strip_prefix('?') {
        return format!("?{}", normalize_vendor_type(inner, namespace, imports));
    }
    for separator in ['|', '&'] {
        if raw.contains(separator) {
            return raw
                .split(separator)
                .map(|part| normalize_vendor_type(part, namespace, imports))
                .collect::<Vec<_>>()
                .join(&separator.to_string());
        }
    }
    if raw.starts_with('\\')
        || raw.contains('\\')
        || matches!(
            raw.to_ascii_lowercase().as_str(),
            "int"
                | "string"
                | "bool"
                | "float"
                | "array"
                | "object"
                | "callable"
                | "iterable"
                | "mixed"
                | "void"
                | "never"
                | "null"
                | "false"
                | "true"
                | "self"
                | "static"
                | "parent"
        )
    {
        return raw.to_owned();
    }
    if let Some(imported) = imports.get(raw) {
        return imported.clone();
    }
    if namespace.is_empty() {
        raw.to_owned()
    } else {
        format!("{namespace}\\{raw}")
    }
}

fn vendor_trait_info(text: &str) -> (Vec<String>, BTreeMap<String, String>) {
    fn walk(node: Node<'_>, text: &str, in_class_like: bool, output: &mut Vec<String>) {
        let class_like = in_class_like
            || matches!(
                node.kind(),
                "class_declaration"
                    | "interface_declaration"
                    | "trait_declaration"
                    | "enum_declaration"
            );
        if class_like && node.kind() == "use_declaration" {
            let value = text[node.start_byte()..node.end_byte()]
                .trim()
                .trim_start_matches("use")
                .trim_end_matches(';')
                .trim();
            for name in value.split(',').map(str::trim) {
                let name = name
                    .split_once(" as ")
                    .map(|(name, _)| name.trim())
                    .unwrap_or(name);
                if !name.is_empty()
                    && name
                        .chars()
                        .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '\\'))
                {
                    output.push(name.to_owned());
                }
            }
            return;
        }
        for child in node.named_children(&mut node.walk()) {
            walk(child, text, class_like, output);
        }
    }

    let Ok(syntax) = PhpSyntax::parse(text.to_owned()) else {
        return (Vec::new(), BTreeMap::new());
    };
    let mut output = Vec::new();
    walk(syntax.tree().root_node(), text, false, &mut output);
    let mut imports = BTreeMap::new();
    fn collect_imports(node: Node<'_>, text: &str, imports: &mut BTreeMap<String, String>) {
        if node.kind() == "namespace_use_declaration" {
            let value = text[node.start_byte()..node.end_byte()]
                .trim()
                .trim_start_matches("use")
                .trim_end_matches(';')
                .trim();
            for item in value.split(',').map(str::trim) {
                let (name, alias) = item
                    .split_once(" as ")
                    .map(|(name, alias)| (name.trim(), alias.trim().to_owned()))
                    .unwrap_or_else(|| {
                        let name = item.trim();
                        let alias = name.rsplit('\\').next().unwrap_or(name).to_owned();
                        (name, alias)
                    });
                if !name.is_empty() {
                    imports.insert(alias, name.trim_start_matches('\\').to_owned());
                }
            }
        }
        for child in node.named_children(&mut node.walk()) {
            collect_imports(child, text, imports);
        }
    }
    collect_imports(syntax.tree().root_node(), text, &mut imports);
    (output, imports)
}

fn metadata_fingerprint(root: &Path) -> Vec<(PathBuf, u64, u128)> {
    [
        "composer.lock",
        "vendor/composer/autoload_classmap.php",
        "vendor/composer/autoload_psr4.php",
        "vendor/composer/autoload_static.php",
    ]
    .into_iter()
    .filter_map(|relative| {
        let path = root.join(relative);
        let metadata = fs::metadata(&path).ok()?;
        let modified = metadata
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_nanos();
        Some((path, metadata.len(), modified))
    })
    .collect()
}

fn composer_path(root: &Path, value: &str) -> PathBuf {
    let value = value.trim().replace("\\\\", "\\");
    if let Some(start) = value.find("$vendorDir . ") {
        let relative = &value[start..];
        let relative = relative
            .strip_prefix("$vendorDir . ")
            .or_else(|| relative.strip_prefix("$baseDir . "))
            .unwrap_or(relative);
        return root
            .join("vendor")
            .join(relative.trim_matches(['\'', '"', '/', '\\', ')', ']', ';']));
    }
    if let Some(start) = value.find("$baseDir . ") {
        let relative = &value[start + "$baseDir . ".len()..];
        return root.join(relative.trim_matches(['\'', '"', '/', '\\', ')', ']', ';']));
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

/// Emits path provenance diagnostics without changing path behavior.
#[cfg(debug_assertions)]
pub fn trace_path(stage: &str, source: &str, path: &Path) {
    if !std::env::var_os("AXIOM_DEBUG_PATHS").is_some_and(|value| {
        !matches!(value.to_string_lossy().as_ref(), "" | "0" | "false" | "off")
    }) {
        return;
    }
    let display = path.display().to_string();
    let exists = path.exists();
    let canonical = fs::canonicalize(path);
    let suspicious = display.contains("wsl$")
        || display
            .chars()
            .any(|character| ('\u{e000}'..='\u{f8ff}').contains(&character));
    eprintln!(
        "[PATH TRACE] stage={stage} source={source} path_debug={path:?} path_display={display:?} exists={exists} canonical={:?} canonicalize_error={:?}",
        canonical.as_ref().ok(),
        canonical.as_ref().err().map(ToString::to_string),
    );
    if suspicious || canonical.is_err() || !exists {
        let units = path_utf16_units(path)
            .into_iter()
            .map(|unit| format!("U+{unit:04X}"))
            .collect::<Vec<_>>();
        eprintln!("[PATH TRACE UTF16] stage={stage} source={source} units={units:?}");
        if let Some(backtrace) = std::env::var_os("AXIOM_DEBUG_PATH_BACKTRACE") {
            if !matches!(
                backtrace.to_string_lossy().as_ref(),
                "" | "0" | "false" | "off"
            ) {
                eprintln!(
                    "[PATH TRACE BACKTRACE] {:?}",
                    std::backtrace::Backtrace::capture()
                );
            }
        }
    }
}

#[cfg(not(debug_assertions))]
pub fn trace_path(_: &str, _: &str, _: &Path) {}

#[cfg(debug_assertions)]
fn trace_symbol_insert(symbol: &ProjectSymbol, source: &str) {
    if !std::env::var_os("AXIOM_DEBUG_PATHS").is_some_and(|value| {
        !matches!(value.to_string_lossy().as_ref(), "" | "0" | "false" | "off")
    }) {
        return;
    }
    let path = &symbol.file;
    let display = path.display().to_string();
    let canonical = fs::canonicalize(path);
    let suspicious = display.contains("wsl$")
        || display
            .chars()
            .any(|character| ('\u{e000}'..='\u{f8ff}').contains(&character));
    eprintln!(
        "[SYMBOL INSERT] symbol={} source={} path_debug={path:?} exists={} canonical={:?}",
        symbol.fully_qualified_name,
        source,
        path.exists(),
        canonical.as_ref().ok(),
    );
    if suspicious || canonical.is_err() || !path.exists() {
        let units = path_utf16_units(path)
            .into_iter()
            .map(|unit| format!("U+{unit:04X}"))
            .collect::<Vec<_>>();
        eprintln!(
            "[SUSPICIOUS SYMBOL INSERT] symbol={} source={} path_debug={path:?} exists={} canonical={:?} canonicalize_error={:?} UTF16={units:?} backtrace={:?}",
            symbol.fully_qualified_name,
            source,
            path.exists(),
            canonical.as_ref().ok(),
            canonical.as_ref().err().map(ToString::to_string),
            std::backtrace::Backtrace::capture(),
        );
    }
}

#[cfg(not(debug_assertions))]
fn trace_symbol_insert(_: &ProjectSymbol, _: &str) {}

#[cfg(debug_assertions)]
pub fn trace_path_join(root: &Path, child: &Path, result: &Path, source: &str) {
    if !std::env::var_os("AXIOM_DEBUG_PATHS").is_some_and(|value| {
        !matches!(value.to_string_lossy().as_ref(), "" | "0" | "false" | "off")
    }) {
        return;
    }
    eprintln!(
        "[PATH JOIN] source={source} root={root:?} child={child:?} child_is_absolute={} result={result:?}",
        child.is_absolute(),
    );
}

#[cfg(not(debug_assertions))]
pub fn trace_path_join(_: &Path, _: &Path, _: &Path, _: &str) {}

#[cfg(all(debug_assertions, windows))]
fn path_utf16_units(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().collect()
}

#[cfg(all(debug_assertions, not(windows)))]
fn path_utf16_units(path: &Path) -> Vec<u16> {
    path.to_string_lossy().encode_utf16().collect()
}

impl ProjectSymbolIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn index_project(&mut self, root: impl AsRef<Path>) -> io::Result<IndexReport> {
        trace_path("project_root", "InitialProjectScan", root.as_ref());
        self.files.clear();
        self.symbols.clear();
        self.ready = false;
        let mut paths = Vec::new();
        collect_php_files(root.as_ref(), &mut paths, &mut DiscoveryStats::default())?;
        let mut seen = HashSet::new();
        for discovered in paths {
            let path = canonical_or(discovered.path);
            if seen.insert(path.clone()) {
                let _ = self.index_file_with_source(&path, "InitialProjectScan");
            }
        }
        self.rebuild_prefix_index();
        self.ready = true;
        Ok(self.report())
    }

    pub fn index_project_cached(
        &mut self,
        root: impl AsRef<Path>,
        cache_path: impl AsRef<Path>,
    ) -> io::Result<IndexReport> {
        let started = Instant::now();
        let root = canonical_or(root.as_ref().to_path_buf());
        let mut discovered = Vec::new();
        let mut discovery = DiscoveryStats::default();
        let discovery_started = Instant::now();
        collect_php_files(&root, &mut discovered, &mut discovery)?;
        let discovery_us = discovery_started.elapsed().as_micros();
        let mut seen = HashSet::new();
        let canonicalize_started = Instant::now();
        let discovered: Vec<DiscoveredFile> = discovered
            .into_iter()
            .filter(|file| seen.insert(file.path.clone()))
            .collect();
        let canonicalize_us = canonicalize_started.elapsed().as_micros();
        let cache_read_started = Instant::now();
        let cache_text = fs::read_to_string(cache_path.as_ref()).ok();
        let cache_read_us = cache_read_started.elapsed().as_micros();
        let cache_deserialize_started = Instant::now();
        let cached = cache_text
            .and_then(|text| serde_json::from_str::<ProjectCacheFile>(&text).ok())
            .filter(|cache| cache.schema_version == PROJECT_CACHE_SCHEMA);
        let cache_deserialize_us = cache_deserialize_started.elapsed().as_micros();
        let cache_validation_started = Instant::now();
        let cached_files = cached
            .as_ref()
            .map(|cache| cache.files.len())
            .unwrap_or_default();
        let cache_validation_us = cache_validation_started.elapsed().as_micros();
        let snapshot_started = Instant::now();
        self.files.clear();
        self.symbols.clear();
        self.ready = false;
        let mut reparsed = 0usize;
        let metadata_us = discovery.metadata_us;
        let metadata_calls = discovery.metadata_calls;
        let mut changed_detection_us = 0u128;
        let mut files_read = 1usize;
        for file in &discovered {
            let path = &file.path;
            let modified = file.modified;
            let changed_started = Instant::now();
            let reused = cached
                .as_ref()
                .and_then(|cache| cache.files.get(path))
                .filter(|(size, stamp, _)| *size == file.size && *stamp == modified);
            changed_detection_us += changed_started.elapsed().as_micros();
            if let Some((_, _, symbols)) = reused {
                self.files.insert(path.clone(), Arc::from(""));
                self.symbols.extend(symbols.clone());
            } else {
                let text = fs::read_to_string(path)?;
                files_read += 1;
                self.index_file_text_at_path(path.clone(), text, "InitialProjectScan")?;
                reparsed += 1;
            }
        }
        let snapshot_restore_us = snapshot_started.elapsed().as_micros();
        let removed = cached_files.saturating_sub(discovered.len());
        self.rebuild_prefix_index();
        self.ready = true;
        let mut files = BTreeMap::new();
        for file in &discovered {
            let path = &file.path;
            let modified = file.modified;
            let symbols = self
                .symbols
                .iter()
                .filter(|symbol| &symbol.file == path)
                .cloned()
                .collect();
            files.insert(path.clone(), (file.size, modified, symbols));
        }
        if let Some(parent) = cache_path.as_ref().parent() {
            let _ = fs::create_dir_all(parent);
        }
        let cache = ProjectCacheFile {
            schema_version: PROJECT_CACHE_SCHEMA,
            files,
        };
        let _ = fs::write(cache_path, serde_json::to_vec(&cache).unwrap_or_default());
        if std::env::var_os("AXIOM_DEBUG_INPUT").is_some() {
            eprintln!(
                "[PROJECT CACHE] hit={} reason={} load_ms={}",
                cached.is_some() && reparsed == 0,
                if cached.is_some() {
                    "metadata"
                } else {
                    "missing"
                },
                started.elapsed().as_millis()
            );
            eprintln!(
                "[PROJECT STARTUP] discovered_files={} cached_files={} reparsed_files={} removed_files={} elapsed_ms={}",
                discovered.len(),
                cached_files,
                reparsed,
                removed,
                started.elapsed().as_millis()
            );
            eprintln!(
                "[PROJECT STARTUP PROFILE] cache_read_us={} cache_deserialize_us={} discovery_us={} metadata_us={} cache_validation_us={} changed_file_detection_us={} snapshot_restore_us={} publish_ui_us=0 canonicalize_us={} directories_visited={} metadata_calls={} canonicalize_calls={} files_opened_read={} total_us={}",
                cache_read_us,
                cache_deserialize_us,
                discovery_us,
                metadata_us,
                cache_validation_us,
                changed_detection_us,
                snapshot_restore_us,
                canonicalize_us,
                discovery.directories_visited,
                metadata_calls,
                discovery.canonicalize_calls,
                files_read,
                started.elapsed().as_micros(),
            );
        }
        Ok(self.report())
    }

    pub fn index_file(&mut self, path: impl AsRef<Path>) -> io::Result<usize> {
        self.index_file_with_source(path, "Other")
    }

    fn index_file_with_source(
        &mut self,
        path: impl AsRef<Path>,
        source: &str,
    ) -> io::Result<usize> {
        trace_path("index_file_input", source, path.as_ref());
        let path = fs::canonicalize(path.as_ref())?;
        trace_path("index_file_canonical", source, &path);
        let text = fs::read_to_string(&path)?;
        self.index_file_text_with_source(path, text, source)
    }

    /// Incrementally replaces one indexed file from an in-memory document.
    /// This is used for dirty buffers and avoids a project-wide traversal.
    pub fn index_file_text(
        &mut self,
        path: impl AsRef<Path>,
        text: impl Into<String>,
    ) -> io::Result<usize> {
        self.index_file_text_with_source(path, text, "Other")
    }

    pub fn index_file_text_with_source(
        &mut self,
        path: impl AsRef<Path>,
        text: impl Into<String>,
        source: &str,
    ) -> io::Result<usize> {
        trace_path("index_file_text_input", source, path.as_ref());
        let path = fs::canonicalize(path.as_ref()).unwrap_or_else(|_| path.as_ref().to_path_buf());
        trace_path("index_file_text_stored", source, &path);
        self.index_file_text_at_path(path, text.into(), source)
    }

    fn index_file_text_at_path(
        &mut self,
        path: PathBuf,
        text: String,
        source: &str,
    ) -> io::Result<usize> {
        self.files.remove(&path);
        self.symbols.retain(|symbol| symbol.file != path);
        let syntax = PhpSyntax::parse(text.clone())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        let mut output = Vec::new();
        walk(
            syntax.tree().root_node(),
            &text,
            &path,
            "",
            None,
            &mut output,
            source,
        );
        let count = output.len();
        self.files.insert(path, Arc::from(text));
        self.symbols.extend(output);
        if self.ready {
            self.rebuild_prefix_index();
        }
        Ok(count)
    }

    pub fn remove_file(&mut self, path: impl AsRef<Path>) -> usize {
        let path = fs::canonicalize(path.as_ref()).unwrap_or_else(|_| path.as_ref().to_path_buf());
        self.files.remove(&path);
        let before = self.symbols.len();
        self.symbols.retain(|symbol| symbol.file != path);
        self.rebuild_prefix_index();
        before - self.symbols.len()
    }

    pub fn is_ready(&self) -> bool {
        self.ready
    }
    pub fn symbols(&self) -> &[ProjectSymbol] {
        &self.symbols
    }
    pub fn indexed_files(&self) -> impl Iterator<Item = &Path> {
        self.files.keys().map(PathBuf::as_path)
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
        let upper = format!("{prefix}\u{10ffff}");
        let mut indexes = std::collections::BTreeSet::new();
        for (_, matches) in self.prefix_names.range(prefix.to_owned()..=upper.clone()) {
            indexes.extend(matches.iter().copied());
        }
        for (_, matches) in self.prefix_fqns.range(prefix.to_owned()..=upper) {
            indexes.extend(matches.iter().copied());
        }
        let mut result: Vec<&ProjectSymbol> = indexes
            .into_iter()
            .filter_map(|index| self.symbols.get(index))
            .collect::<Vec<_>>();
        result.sort_by_key(|s| (!s.name.starts_with(prefix), s.name.to_lowercase()));
        result
    }

    fn rebuild_prefix_index(&mut self) {
        self.prefix_names.clear();
        self.prefix_fqns.clear();
        for (index, symbol) in self.symbols.iter().enumerate() {
            self.prefix_names
                .entry(symbol.name.clone())
                .or_default()
                .push(index);
            self.prefix_fqns
                .entry(symbol.fully_qualified_name.clone())
                .or_default()
                .push(index);
        }
    }
}

#[derive(Default)]
struct DiscoveryStats {
    directories_visited: usize,
    metadata_calls: usize,
    canonicalize_calls: usize,
    metadata_us: u128,
}

struct DiscoveredFile {
    path: PathBuf,
    size: u64,
    modified: u128,
}

fn collect_php_files(
    root: &Path,
    output: &mut Vec<DiscoveredFile>,
    stats: &mut DiscoveryStats,
) -> io::Result<()> {
    stats.directories_visited += 1;
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
        // Inspect the directory entry itself before following its target.
        // This avoids following ordinary symlink directories without adding a
        // second metadata call to the warm-cache traversal.
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let metadata_started = Instant::now();
        let metadata = entry.metadata()?;
        stats.metadata_calls += 1;
        stats.metadata_us += metadata_started.elapsed().as_micros();
        if metadata.is_dir() {
            collect_php_files(&path, output, stats)?;
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("php"))
        {
            let modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|value| value.as_nanos())
                .unwrap_or_default();
            output.push(DiscoveredFile {
                path,
                size: metadata.len(),
                modified,
            });
        }
    }
    Ok(())
}

/// Returns whether a path belongs to the editable Workspace source set.
/// This is the same root/exclusion policy used by project discovery.
pub fn is_workspace_source(path: impl AsRef<Path>, project_root: impl AsRef<Path>) -> bool {
    let path = canonical_or(path.as_ref().to_path_buf());
    let root = canonical_or(project_root.as_ref().to_path_buf());
    if !path.starts_with(&root) {
        return false;
    }
    path.strip_prefix(&root)
        .ok()
        .into_iter()
        .flat_map(Path::components)
        .all(|component| {
            let name = component.as_os_str().to_string_lossy();
            !matches!(
                name.to_ascii_lowercase().as_str(),
                ".git" | "target" | "node_modules" | "vendor"
            )
        })
}

/// Purely lexical variant for UI classification when the path and project
/// root have already been obtained from resident workspace state. Unlike
/// [`is_workspace_source`], this never canonicalizes or otherwise touches the
/// filesystem.
pub fn is_workspace_source_lexical(path: impl AsRef<Path>, project_root: impl AsRef<Path>) -> bool {
    let path = PersistentFileKey::workspace_lexical(path).normalized_path;
    let root = PersistentFileKey::workspace_lexical(project_root).normalized_path;
    let Some(relative) = path
        .strip_prefix(&root)
        .and_then(|rest| rest.strip_prefix('/'))
    else {
        return false;
    };
    relative.split('/').all(|component| {
        !matches!(
            component.to_ascii_lowercase().as_str(),
            ".git" | "target" | "node_modules" | "vendor"
        )
    })
}

fn walk(
    node: Node<'_>,
    text: &str,
    file: &Path,
    namespace: &str,
    class: Option<&str>,
    out: &mut Vec<ProjectSymbol>,
    source: &str,
) {
    if node.kind() == "program" {
        let mut current_namespace = namespace.to_owned();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "namespace_definition" {
                current_namespace = child
                    .child_by_field_name("name")
                    .and_then(|name| name.utf8_text(text.as_bytes()).ok())
                    .unwrap_or("")
                    .trim_matches('\\')
                    .to_owned();
            } else {
                walk(child, text, file, &current_namespace, class, out, source);
            }
        }
        return;
    }
    // Namespace declarations are lexical scopes. Carry the namespace from
    // each declaration's own byte range instead of selecting the first one in
    // the file; PHP permits multiple namespaces in a single file.
    if node.kind() == "namespace_definition" {
        let namespace = node
            .child_by_field_name("name")
            .and_then(|name| name.utf8_text(text.as_bytes()).ok())
            .unwrap_or("")
            .trim_matches('\\')
            .to_owned();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            walk(child, text, file, &namespace, None, out, source);
        }
        return;
    }
    if let Some((kind, name_node)) = symbol_node(node) {
        if let Ok(name) = name_node.utf8_text(text.as_bytes()) {
            let name = name.to_owned();
            let (visibility, modifiers) = declaration_modifiers(node, text);
            let parameters = node.child_by_field_name("parameters").map(|node| {
                node.utf8_text(text.as_bytes())
                    .unwrap_or_default()
                    .to_owned()
            });
            let return_type = node
                .child_by_field_name("return_type")
                .map(|node| {
                    node.utf8_text(text.as_bytes())
                        .unwrap_or_default()
                        .to_owned()
                })
                .or_else(|| {
                    (kind == ProjectSymbolKind::Property)
                        .then(|| property_declared_type(node, name_node, text))
                        .flatten()
                });
            let fqn = match (class, kind) {
                (
                    Some(parent),
                    ProjectSymbolKind::Method
                    | ProjectSymbolKind::Property
                    | ProjectSymbolKind::Constant
                    | ProjectSymbolKind::ClassConstant
                    | ProjectSymbolKind::EnumCase,
                ) if namespace.is_empty() => format!("{parent}::{name}"),
                (
                    Some(parent),
                    ProjectSymbolKind::Method
                    | ProjectSymbolKind::Property
                    | ProjectSymbolKind::Constant
                    | ProjectSymbolKind::ClassConstant
                    | ProjectSymbolKind::EnumCase,
                ) => format!("{namespace}\\{parent}::{name}"),
                _ if namespace.is_empty() => name.clone(),
                _ => format!("{namespace}\\{name}"),
            };
            let symbol = ProjectSymbol {
                name,
                fully_qualified_name: fqn,
                kind,
                file: file.to_path_buf(),
                range: name_node.byte_range(),
                namespace: namespace.to_owned(),
                visibility,
                modifiers,
                parameters,
                return_type,
            };
            trace_symbol_insert(&symbol, source);
            out.push(symbol);
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
                walk(
                    child,
                    text,
                    file,
                    namespace,
                    next_class.or(class),
                    out,
                    source,
                );
            }
            return;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, text, file, namespace, class, out, source);
    }
}

fn property_declared_type(node: Node<'_>, name: Node<'_>, text: &str) -> Option<String> {
    let source = &text[node.start_byte()..name.start_byte().min(text.len())];
    let ty = source.split_whitespace().last()?.trim_matches(['?', '&']);
    (ty.chars()
        .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '\\' | '|'))
        && !ty.is_empty())
    .then(|| ty.to_owned())
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
        "enum_case" => ProjectSymbolKind::EnumCase,
        _ => return None,
    };
    let field = node.child_by_field_name("name").or_else(|| {
        if kind == ProjectSymbolKind::Property {
            if let Some(element) = node
                .named_children(&mut node.walk())
                .find(|child| child.kind() == "property_element")
            {
                return element.child_by_field_name("name").or_else(|| {
                    element
                        .named_children(&mut element.walk())
                        .find(|child| child.kind() == "variable_name" || child.kind() == "name")
                });
            }
        }
        node.named_children(&mut node.walk()).find_map(|child| {
            if child.kind() == "variable_name" || child.kind() == "name" {
                Some(child)
            } else {
                child.child_by_field_name("name").or_else(|| {
                    child
                        .named_children(&mut child.walk())
                        .find(|nested| nested.kind() == "variable_name" || nested.kind() == "name")
                })
            }
        })
    })?;
    Some((kind, field))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_prefix_index_tracks_initial_incremental_and_removed_symbols() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Service.php");
        fs::write(
            &path,
            "<?php namespace App; class AlphaService {} function alpha_helper() {}",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();

        assert!(
            index
                .search_prefix("Alpha")
                .iter()
                .any(|symbol| { symbol.fully_qualified_name == "App\\AlphaService" })
        );
        assert!(
            index
                .search_prefix("App\\Alpha")
                .iter()
                .any(|symbol| { symbol.fully_qualified_name == "App\\AlphaService" })
        );

        index
            .index_file_text(&path, "<?php namespace App; class BetaService {}")
            .unwrap();
        assert!(index.search_prefix("Alpha").is_empty());
        assert_eq!(index.search_prefix("Beta").len(), 1);

        index.remove_file(&path);
        assert!(index.search_prefix("Beta").is_empty());
    }

    #[test]
    fn workspace_source_uses_discovery_exclusions_and_root_boundary() {
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("src/User.php");
        let vendor = root.path().join("vendor/pkg/Foo.php");
        let target = root.path().join("target/generated.php");
        let external = root.path().join(r"..\external.php");
        fs::create_dir_all(src.parent().unwrap()).unwrap();
        fs::create_dir_all(vendor.parent().unwrap()).unwrap();
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&src, "<?php").unwrap();
        fs::write(&vendor, "<?php").unwrap();
        fs::write(&target, "<?php").unwrap();
        fs::write(&external, "<?php").unwrap();
        assert!(is_workspace_source(&src, root.path()));
        assert!(!is_workspace_source(&vendor, root.path()));
        assert!(!is_workspace_source(&target, root.path()));
        assert!(!is_workspace_source(external, root.path()));
        let lexical_root = root.path().to_string_lossy().replace('\\', "/");
        assert!(is_workspace_source_lexical(
            format!("{lexical_root}/src/User.php"),
            &lexical_root
        ));
        assert!(!is_workspace_source_lexical(
            format!("{lexical_root}/vendor/pkg/Foo.php"),
            &lexical_root
        ));
        assert!(!is_workspace_source_lexical(
            "C:/other/External.php",
            &lexical_root
        ));
    }
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

    #[test]
    fn indexes_typed_property_type() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Client.php");
        fs::write(
            &path,
            "<?php\nclass Client {\n    private FiberEventLoop $loop;\n}\n",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_file(&path).unwrap();
        let property = index
            .symbols()
            .iter()
            .find(|symbol| symbol.kind == ProjectSymbolKind::Property)
            .unwrap();
        assert_eq!(property.return_type.as_deref(), Some("FiberEventLoop"));
    }

    #[test]
    fn composer_classmap_resolves_without_executing_php() {
        let dir = tempfile::tempdir().unwrap();
        let composer = dir.path().join("vendor/composer");
        fs::create_dir_all(&composer).unwrap();
        let file = dir.path().join("vendor/pkg/src/Widget.php");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(
            &file,
            "<?php namespace Acme; class Widget { public function run(string $x): bool {} }",
        )
        .unwrap();
        fs::write(
            composer.join("autoload_classmap.php"),
            format!("<?php return ['Acme\\\\Widget' => $vendorDir . '/pkg/src/Widget.php'];"),
        )
        .unwrap();
        let mut index = VendorSymbolIndex::load(dir.path()).unwrap();
        assert_eq!(
            index.resolve_class("Acme\\Widget").unwrap(),
            fs::canonicalize(file).unwrap()
        );
        assert!(
            index
                .symbols_of("Acme\\Widget")
                .iter()
                .any(|s| s.name == "run" && s.parameters.as_deref() == Some("(string $x)"))
        );
        assert_eq!(
            index
                .symbols_of("Acme\\Widget")
                .iter()
                .find(|s| s.name == "run")
                .and_then(|s| s.return_type.as_deref()),
            Some("bool")
        );
    }

    #[test]
    fn composer_root_psr4_is_not_misclassified_as_vendor() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("src/Thing.php");
        let vendor_source = dir.path().join("vendor/acme/pkg/src/Thing.php");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::create_dir_all(vendor_source.parent().unwrap()).unwrap();
        fs::write(&source, "<?php namespace Acme; class Thing {}").unwrap();
        fs::write(&vendor_source, "<?php namespace Vendor; class Thing {}").unwrap();
        fs::write(
            dir.path().join("composer.json"),
            r#"{"autoload":{"psr-4":{"Acme\\":"src/","Vendor\\":"vendor/acme/pkg/src/"}}}"#,
        )
        .unwrap();
        let index = VendorSymbolIndex::load(dir.path()).unwrap();
        assert!(index.resolve_class("Acme\\Thing").is_none());
        assert_eq!(
            index.resolve_class("Vendor\\Thing"),
            Some(fs::canonicalize(vendor_source).unwrap())
        );
    }

    #[test]
    fn composer_metadata_cache_reuses_unchanged_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let composer = dir.path().join("vendor/composer");
        let source = dir.path().join("vendor/pkg/src/Cached.php");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, "<?php class Cached {}").unwrap();
        fs::create_dir_all(&composer).unwrap();
        fs::write(
            composer.join("autoload_classmap.php"),
            "<?php return ['Pkg\\\\Cached' => $vendorDir . '/pkg/src/Cached.php'];",
        )
        .unwrap();
        let cache = dir.path().join("cache/vendor.json");
        let first = VendorSymbolIndex::load_cached(dir.path(), &cache).unwrap();
        let second = VendorSymbolIndex::load_cached(dir.path(), &cache).unwrap();
        assert_eq!(
            first.resolve_class("Pkg\\Cached"),
            second.resolve_class("Pkg\\Cached")
        );
    }

    #[test]
    fn generated_psr4_array_resolves_vendor_dir_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let composer = dir.path().join("vendor/composer");
        let source = dir
            .path()
            .join("vendor/omegaalfa/fiber-event-loop/src/FiberEventLoop.php");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::create_dir_all(&composer).unwrap();
        fs::write(
            &source,
            "<?php namespace Omegaalfa\\FiberEventLoop; class FiberEventLoop {}",
        )
        .unwrap();
        fs::write(
            composer.join("autoload_psr4.php"),
            "'Omegaalfa\\\\FiberEventLoop\\\\' => array($vendorDir . '/omegaalfa/fiber-event-loop/src'),",
        )
        .unwrap();
        let index = VendorSymbolIndex::load(dir.path()).unwrap();
        assert_eq!(
            index.resolve_class("Omegaalfa\\FiberEventLoop\\FiberEventLoop"),
            Some(fs::canonicalize(source).unwrap())
        );
    }

    #[test]
    fn vendor_symbols_include_methods_from_used_traits() {
        let dir = tempfile::tempdir().unwrap();
        let composer = dir.path().join("vendor/composer");
        let src = dir.path().join("vendor/pkg/src");
        fs::create_dir_all(&composer).unwrap();
        fs::create_dir_all(&src).unwrap();
        fs::write(
            src.join("FiberEventLoop.php"),
            "<?php\nnamespace Pkg;\nuse Pkg\\Traits\\LoopTrait;\nclass FiberEventLoop\n{\n    protected $loop;\n    use LoopTrait;\n}\n",
        )
        .unwrap();
        fs::create_dir_all(src.join("Traits")).unwrap();
        fs::write(
            src.join("Traits/LoopTrait.php"),
            "<?php\nnamespace Pkg\\Traits;\ntrait LoopTrait\n{\n    public function next(): void {}\n}\n",
        )
        .unwrap();
        fs::write(
            composer.join("autoload_psr4.php"),
            "'Pkg\\\\' => array($vendorDir . '/pkg/src'),",
        )
        .unwrap();
        let mut index = VendorSymbolIndex::load(dir.path()).unwrap();
        let symbols = index.symbols_of("Pkg\\FiberEventLoop");
        assert!(
            symbols.iter().any(|symbol| {
                symbol.kind == ProjectSymbolKind::Method && symbol.name == "next"
            })
        );
        assert!(
            symbols.iter().any(|symbol| {
                symbol.kind == ProjectSymbolKind::Property && symbol.name == "loop"
            })
        );
    }

    #[test]
    fn vendor_method_return_types_are_preserved_verbatim() {
        for (signature, expected) in [
            ("getUri(): UriInterface {}", "UriInterface"),
            (
                "getUri(): \\Psr\\Http\\Message\\UriInterface;",
                "\\Psr\\Http\\Message\\UriInterface",
            ),
            ("getUri(): ?UriInterface {}", "?UriInterface"),
            ("getUri(): A|B {}", "A|B"),
            ("getUri(): A&B {}", "A&B"),
            ("getUri(): self {}", "self"),
            ("getUri(): static {}", "static"),
            ("getUri(): parent {}", "parent"),
        ] {
            assert_eq!(
                vendor_method_return_type(signature, 6).as_deref(),
                Some(expected)
            );
        }
    }

    #[test]
    fn vendor_method_return_types_resolve_namespace_and_aliases() {
        let mut imports = BTreeMap::new();
        imports.insert(
            "UriInterface".to_owned(),
            "Psr\\Http\\Message\\UriInterface".to_owned(),
        );
        assert_eq!(
            normalize_vendor_type("UriInterface", "Laminas\\Diactoros", &imports),
            "Psr\\Http\\Message\\UriInterface"
        );
        assert_eq!(
            normalize_vendor_type("?UriInterface|self", "Laminas\\Diactoros", &imports),
            "?Psr\\Http\\Message\\UriInterface|self"
        );
        assert_eq!(
            normalize_vendor_type("\\Psr\\Http\\Message\\UriInterface", "Laminas", &imports),
            "\\Psr\\Http\\Message\\UriInterface"
        );
    }

    #[test]
    fn vendor_parser_distinguishes_imports_from_trait_uses() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("A.php");
        let b = dir.path().join("B.php");
        fs::write(
            &a,
            "<?php namespace Pkg; use Pkg\\B; class A { public function own() {} }",
        )
        .unwrap();
        fs::write(
            &b,
            "<?php namespace Pkg; class B { public function imported() {} }",
        )
        .unwrap();
        let mut index = VendorSymbolIndex::default();
        index.classmap.insert("Pkg\\A".into(), a.clone());
        index.classmap.insert("Pkg\\B".into(), b.clone());
        let symbols = index.symbols_of("Pkg\\A");
        assert!(symbols.iter().any(|symbol| symbol.name == "own"));
        assert!(!index.parsed.contains_key("Pkg\\B"));
    }

    #[test]
    fn vendor_trait_cycle_terminates_and_keeps_own_symbols() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("A.php");
        let b = dir.path().join("B.php");
        fs::write(
            &a,
            "<?php\nnamespace Pkg;\ntrait A {\n    use B;\n    public function a() {}\n}\n",
        )
        .unwrap();
        fs::write(
            &b,
            "<?php\nnamespace Pkg;\ntrait B {\n    use A;\n    public function b() {}\n}\n",
        )
        .unwrap();
        let mut index = VendorSymbolIndex::default();
        index.classmap.insert("Pkg\\A".into(), a);
        index.classmap.insert("Pkg\\B".into(), b);
        let symbols = index.symbols_of("Pkg\\A");
        assert!(symbols.iter().any(|symbol| symbol.name == "a"));
        assert!(symbols.iter().any(|symbol| symbol.name == "b"));
    }

    #[test]
    fn vendor_trait_indirect_and_self_cycles_terminate() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ["A", "B", "C"]
            .into_iter()
            .map(|name| dir.path().join(format!("{name}.php")))
            .collect::<Vec<_>>();
        fs::write(
            &paths[0],
            "<?php\nnamespace Pkg;\ntrait A { use B; public function a() {} }\n",
        )
        .unwrap();
        fs::write(
            &paths[1],
            "<?php\nnamespace Pkg;\ntrait B { use C; public function b() {} }\n",
        )
        .unwrap();
        fs::write(
            &paths[2],
            "<?php\nnamespace Pkg;\ntrait C { use A; public function c() {} }\n",
        )
        .unwrap();
        let mut index = VendorSymbolIndex::default();
        for (name, path) in ["A", "B", "C"].into_iter().zip(paths) {
            index.classmap.insert(format!("Pkg\\{name}"), path);
        }
        let symbols = index.symbols_of("Pkg\\A");
        assert!(symbols.iter().any(|symbol| symbol.name == "a"));
        assert!(symbols.iter().any(|symbol| symbol.name == "b"));
        assert!(symbols.iter().any(|symbol| symbol.name == "c"));

        let self_cycle = dir.path().join("Self.php");
        fs::write(
            &self_cycle,
            "<?php\nnamespace Pkg;\ntrait SelfTrait { use SelfTrait; public function own() {} }\n",
        )
        .unwrap();
        index.classmap.insert("Pkg\\SelfTrait".into(), self_cycle);
        assert!(
            index
                .symbols_of("Pkg\\SelfTrait")
                .iter()
                .any(|symbol| symbol.name == "own")
        );
    }

    #[test]
    fn vendor_trait_use_helper_ignores_namespace_imports_and_aliases() {
        let text = "<?php namespace Pkg; use Pkg\\Imported; use Pkg\\Aliased as Alias; class A { use TraitB; }";
        assert_eq!(vendor_trait_info(text).0, vec!["TraitB"]);
    }
}
