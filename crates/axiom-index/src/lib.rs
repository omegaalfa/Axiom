//! Project-local PHP symbol index.
//!
//! This crate is deliberately headless. It owns no UI or LSP state and can be
//! queried from completion/navigation providers without blocking rendering.

use axiom_syntax::PhpSyntax;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use tree_sitter::Node;

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
    ready: bool,
}

const PROJECT_CACHE_SCHEMA: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct ProjectCacheFile {
    schema_version: u32,
    files: BTreeMap<PathBuf, (u64, u128, Vec<ProjectSymbol>)>,
}

/// Composer metadata index. It records class locations without walking all of
/// `vendor/`; declarations are parsed only when a class is queried.
#[derive(Debug, Default)]
pub struct VendorSymbolIndex {
    classmap: BTreeMap<String, PathBuf>,
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
        let mut index = Self::default();
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
                    composer_path(
                        root,
                        right
                            .trim()
                            .trim_end_matches(|c| matches!(c, ',' | ']' | ';'))
                            .trim(),
                    )
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
            return Ok(Self {
                classmap: cache.classmap,
                psr4: cache.psr4,
                parsed: BTreeMap::new(),
                parsed_files: BTreeMap::new(),
            });
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

    pub fn resolve_class(&self, fqn: &str) -> Option<PathBuf> {
        if let Some(path) = self.classmap.get(fqn) {
            if std::env::var_os("AXIOM_DEBUG_COMPOSER").is_some() {
                eprintln!(
                    "[VENDOR RESOLVE] fqn={fqn} classmap_match=true psr4_prefix_match= candidate_path={path:?} exists={} result={}",
                    path.is_file(),
                    path.is_file()
                );
            }
            return Some(path.clone());
        }
        let (prefix, tail, _) = self
            .psr4
            .iter()
            .filter_map(|(prefix, bases)| {
                fqn.strip_prefix(prefix).map(|tail| (prefix, tail, bases))
            })
            .max_by_key(|(prefix, _, _)| prefix.len())?;
        let relative = tail
            .trim_start_matches('\\')
            .replace('\\', std::path::MAIN_SEPARATOR_STR);
        for base in &self.psr4.iter().find(|(p, _)| p == prefix)?.1 {
            let path = base.join(format!("{relative}.php"));
            if std::env::var_os("AXIOM_DEBUG_COMPOSER").is_some() {
                eprintln!(
                    "[VENDOR RESOLVE] fqn={fqn} classmap_match=false psr4_prefix_match={prefix} candidate_path={:?} exists={} result={}",
                    path,
                    path.is_file(),
                    path.is_file()
                );
            }
            if path.is_file() {
                return Some(path);
            }
        }
        None
    }

    pub fn symbols_of(&mut self, fqn: &str) -> Vec<ProjectSymbol> {
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
                let name = trimmed[pos + 9..]
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()
                    .unwrap_or_default();
                if !name.is_empty() {
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
                        return_type: None,
                    });
                }
            }
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

    pub fn is_empty(&self) -> bool {
        self.classmap.is_empty() && self.psr4.is_empty()
    }

    pub fn classes_matching(&self, prefix: &str) -> Vec<String> {
        self.classmap
            .keys()
            .filter(|fqn| {
                fqn.rsplit('\\')
                    .next()
                    .is_some_and(|name| name.starts_with(prefix))
            })
            .cloned()
            .collect()
    }
}

fn canonical_or(path: PathBuf) -> PathBuf {
    fs::canonicalize(&path).unwrap_or(path)
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
    if !std::env::var_os("AXIOM_DEBUG_COMPLETION").is_some_and(|value| {
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
        collect_php_files(root.as_ref(), &mut paths)?;
        for path in paths {
            let _ = self.index_file_with_source(&path, "InitialProjectScan");
        }
        self.ready = true;
        Ok(self.report())
    }

    pub fn index_project_cached(
        &mut self,
        root: impl AsRef<Path>,
        cache_path: impl AsRef<Path>,
    ) -> io::Result<IndexReport> {
        let started = Instant::now();
        let root = root.as_ref();
        let mut discovered = Vec::new();
        collect_php_files(root, &mut discovered)?;
        let discovered: Vec<PathBuf> = discovered
            .into_iter()
            .filter_map(|path| fs::canonicalize(path).ok())
            .collect();
        let cached = fs::read_to_string(cache_path.as_ref())
            .ok()
            .and_then(|text| serde_json::from_str::<ProjectCacheFile>(&text).ok())
            .filter(|cache| cache.schema_version == PROJECT_CACHE_SCHEMA);
        let cached_files = cached
            .as_ref()
            .map(|cache| cache.files.len())
            .unwrap_or_default();
        self.files.clear();
        self.symbols.clear();
        self.ready = false;
        let mut reparsed = 0usize;
        for path in &discovered {
            let metadata = fs::metadata(path)?;
            let modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|v| v.as_nanos())
                .unwrap_or_default();
            let reused = cached
                .as_ref()
                .and_then(|cache| cache.files.get(path))
                .filter(|(size, stamp, _)| *size == metadata.len() && *stamp == modified);
            if let Some((_, _, symbols)) = reused {
                self.files.insert(path.clone(), Arc::from(""));
                self.symbols.extend(symbols.clone());
            } else {
                self.index_file_with_source(path, "InitialProjectScan")?;
                reparsed += 1;
            }
        }
        let removed = cached_files.saturating_sub(discovered.len());
        self.ready = true;
        let mut files = BTreeMap::new();
        for path in &discovered {
            let metadata = fs::metadata(path)?;
            let modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|v| v.as_nanos())
                .unwrap_or_default();
            let symbols = self
                .symbols
                .iter()
                .filter(|symbol| &symbol.file == path)
                .cloned()
                .collect();
            files.insert(path.clone(), (metadata.len(), modified, symbols));
        }
        if let Some(parent) = cache_path.as_ref().parent() {
            let _ = fs::create_dir_all(parent);
        }
        let cache = ProjectCacheFile {
            schema_version: PROJECT_CACHE_SCHEMA,
            files,
        };
        let _ = fs::write(cache_path, serde_json::to_vec(&cache).unwrap_or_default());
        if std::env::var_os("AXIOM_DEBUG_COMPLETION").is_some() {
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
            source,
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
    source: &str,
) {
    if let Some((kind, name_node)) = symbol_node(node) {
        if let Ok(name) = name_node.utf8_text(text.as_bytes()) {
            let name = name.to_owned();
            let (visibility, modifiers) = declaration_modifiers(node, text);
            let parameters = node.child_by_field_name("parameters").map(|node| {
                node.utf8_text(text.as_bytes())
                    .unwrap_or_default()
                    .to_owned()
            });
            let return_type = node.child_by_field_name("return_type").map(|node| {
                node.utf8_text(text.as_bytes())
                    .unwrap_or_default()
                    .to_owned()
            });
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
    }

    #[test]
    fn composer_json_psr4_is_used_as_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("src/Thing.php");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, "<?php namespace Acme; class Thing {}").unwrap();
        fs::write(
            dir.path().join("composer.json"),
            r#"{"autoload":{"psr-4":{"Acme\\":"src/"}}}"#,
        )
        .unwrap();
        let index = VendorSymbolIndex::load(dir.path()).unwrap();
        assert_eq!(
            index.resolve_class("Acme\\Thing").unwrap(),
            fs::canonicalize(source).unwrap()
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
}
