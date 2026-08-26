//! Source location and loading adapters.
//!
//! Providers answer where source code comes from and how to load it. They do
//! not parse PHP or attach semantic meaning to the loaded text. The existing
//! project/vendor/runtime indexes remain compatible consumers alongside this
//! layer until later semantic milestones migrate to it.

use crate::{
    PersistentFileKey, ProjectSymbolIndex, ProjectSymbolKind, SourceOrigin, VendorSymbolIndex,
};
use axiom_php::RuntimeSymbolIndex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
    fs,
    hash::{Hash, Hasher},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, RwLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::UNIX_EPOCH,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SourceKind {
    Workspace,
    Vendor,
    Runtime,
    Generated,
}

impl SourceOrigin {
    pub fn kind(&self) -> SourceKind {
        match self {
            Self::Workspace => SourceKind::Workspace,
            Self::Vendor { .. } => SourceKind::Vendor,
            Self::Runtime => SourceKind::Runtime,
            Self::Generated => SourceKind::Generated,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceFingerprint(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCandidate {
    pub key: PersistentFileKey,
    pub path: PathBuf,
    pub origin: SourceOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    pub key: PersistentFileKey,
    pub path: PathBuf,
    pub origin: SourceOrigin,
    pub text: Arc<str>,
    pub fingerprint: SourceFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeferredSource {
    pub key: PersistentFileKey,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceResolution {
    Found(SourceCandidate),
    Candidates(Vec<SourceCandidate>),
    NotFound,
    Deferred(DeferredSource),
}

impl SourceResolution {
    pub fn candidates(&self) -> &[SourceCandidate] {
        match self {
            Self::Found(candidate) => std::slice::from_ref(candidate),
            Self::Candidates(candidates) => candidates,
            Self::NotFound | Self::Deferred(_) => &[],
        }
    }
}

#[derive(Debug)]
pub enum SourceError {
    WrongOrigin {
        expected: SourceKind,
        actual: SourceOrigin,
    },
    UnknownFile(PersistentFileKey),
    Io(io::Error),
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongOrigin { expected, actual } => {
                write!(
                    formatter,
                    "source origin mismatch: expected {expected:?}, got {actual:?}"
                )
            }
            Self::UnknownFile(key) => write!(formatter, "source file is not registered: {key:?}"),
            Self::Io(error) => write!(formatter, "source I/O failed: {error}"),
        }
    }
}

impl std::error::Error for SourceError {}

impl From<io::Error> for SourceError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub trait SourceProvider: Send + Sync {
    fn origin(&self) -> SourceOrigin;
    fn resolve_name(&self, name: &str) -> SourceResolution;
    fn load_source(&self, key: &PersistentFileKey) -> Result<SourceFile, SourceError>;
    fn fingerprint(&self) -> SourceFingerprint;

    /// Lets a registry with multiple providers of one kind route a persistent
    /// key to its owner without probing or parsing unrelated sources.
    fn owns(&self, key: &PersistentFileKey) -> bool {
        self.origin().kind() == key.origin.kind()
    }
}

#[derive(Debug, Clone, Default)]
struct WorkspaceState {
    paths: HashMap<PersistentFileKey, PathBuf>,
    names: HashMap<String, Vec<PersistentFileKey>>,
    buffers: HashMap<PersistentFileKey, Arc<str>>,
}

pub struct WorkspaceSource {
    root: PathBuf,
    state: RwLock<WorkspaceState>,
}

impl std::fmt::Debug for WorkspaceSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkspaceSource")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl WorkspaceSource {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: canonical_or(root.as_ref()),
            state: RwLock::new(WorkspaceState::default()),
        }
    }

    pub fn from_project_index(root: impl AsRef<Path>, index: &ProjectSymbolIndex) -> Self {
        let source = Self::new(root);
        let mut state = source
            .state
            .write()
            .expect("workspace source lock poisoned");
        for symbol in index.symbols() {
            let key = PersistentFileKey::workspace(&symbol.file);
            state
                .paths
                .entry(key.clone())
                .or_insert_with(|| symbol.file.clone());
            if is_top_level_symbol(symbol.kind) {
                state
                    .names
                    .entry(symbol.fully_qualified_name.clone())
                    .or_default()
                    .push(key);
            }
        }
        dedup_name_keys(&mut state.names);
        drop(state);
        source
    }

    pub fn register_file(&self, path: impl AsRef<Path>) -> PersistentFileKey {
        let path = canonical_or(path.as_ref());
        let key = PersistentFileKey::workspace(&path);
        self.state
            .write()
            .expect("workspace source lock poisoned")
            .paths
            .insert(key.clone(), path);
        key
    }

    /// Registers a future in-memory document. It takes precedence over disk
    /// when `load_source` is called for the same persistent file key.
    pub fn set_buffer(&self, path: impl AsRef<Path>, text: impl Into<Arc<str>>) {
        let key = self.register_file(path);
        self.state
            .write()
            .expect("workspace source lock poisoned")
            .buffers
            .insert(key, text.into());
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl SourceProvider for WorkspaceSource {
    fn origin(&self) -> SourceOrigin {
        SourceOrigin::Workspace
    }

    fn resolve_name(&self, name: &str) -> SourceResolution {
        let state = self.state.read().expect("workspace source lock poisoned");
        let mut candidates = state
            .names
            .get(name)
            .into_iter()
            .flatten()
            .filter_map(|key| {
                state.paths.get(key).map(|path| SourceCandidate {
                    key: key.clone(),
                    path: path.clone(),
                    origin: SourceOrigin::Workspace,
                })
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| left.path.cmp(&right.path));
        match candidates.len() {
            0 => SourceResolution::NotFound,
            1 => SourceResolution::Found(candidates.remove(0)),
            _ => SourceResolution::Candidates(candidates),
        }
    }

    fn load_source(&self, key: &PersistentFileKey) -> Result<SourceFile, SourceError> {
        if key.origin.kind() != SourceKind::Workspace {
            return Err(SourceError::WrongOrigin {
                expected: SourceKind::Workspace,
                actual: key.origin.clone(),
            });
        }
        let (path, buffer) = {
            let state = self.state.read().expect("workspace source lock poisoned");
            (
                state.paths.get(key).cloned(),
                state.buffers.get(key).cloned(),
            )
        };
        let path = path.ok_or_else(|| SourceError::UnknownFile(key.clone()))?;
        let text: Arc<str> = match buffer {
            Some(text) => text,
            None => fs::read_to_string(&path)?.into(),
        };
        Ok(SourceFile {
            key: key.clone(),
            path,
            origin: SourceOrigin::Workspace,
            fingerprint: text_fingerprint(&text),
            text,
        })
    }

    fn owns(&self, key: &PersistentFileKey) -> bool {
        key.origin.kind() == SourceKind::Workspace
            && self
                .state
                .read()
                .map(|state| state.paths.contains_key(key))
                .unwrap_or(false)
    }

    fn fingerprint(&self) -> SourceFingerprint {
        fingerprint_paths(
            &self
                .state
                .read()
                .expect("workspace source lock poisoned")
                .paths,
        )
    }
}

pub struct ComposerSource {
    index: Arc<RwLock<VendorSymbolIndex>>,
    paths: RwLock<HashMap<PersistentFileKey, PathBuf>>,
    loaded: RwLock<HashMap<PersistentFileKey, Arc<str>>>,
    loads: AtomicUsize,
}

impl std::fmt::Debug for ComposerSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ComposerSource")
            .field("index", &"VendorSymbolIndex")
            .finish_non_exhaustive()
    }
}

impl ComposerSource {
    pub fn new(index: Arc<RwLock<VendorSymbolIndex>>) -> Self {
        Self {
            index,
            paths: RwLock::new(HashMap::new()),
            loaded: RwLock::new(HashMap::new()),
            loads: AtomicUsize::new(0),
        }
    }

    pub fn load_count(&self) -> usize {
        self.loads.load(Ordering::Relaxed)
    }
}

impl SourceProvider for ComposerSource {
    fn origin(&self) -> SourceOrigin {
        SourceOrigin::Vendor { package: None }
    }

    fn resolve_name(&self, name: &str) -> SourceResolution {
        let paths = self
            .index
            .read()
            .ok()
            .map(|index| index.resolve_class_candidates(name))
            .unwrap_or_default();
        let mut candidates = Vec::new();
        for path in paths {
            let path = canonical_or(&path);
            let key = PersistentFileKey::new(SourceOrigin::Vendor { package: None }, &path);
            self.paths
                .write()
                .expect("composer source lock poisoned")
                .insert(key.clone(), path.clone());
            candidates.push(SourceCandidate {
                key,
                path,
                origin: SourceOrigin::Vendor { package: None },
            });
        }
        candidates.sort_by(|left, right| left.path.cmp(&right.path));
        candidates.dedup_by(|left, right| left.key == right.key);
        match candidates.len() {
            0 => SourceResolution::NotFound,
            1 => SourceResolution::Found(candidates.remove(0)),
            _ => SourceResolution::Candidates(candidates),
        }
    }

    fn load_source(&self, key: &PersistentFileKey) -> Result<SourceFile, SourceError> {
        if key.origin.kind() != SourceKind::Vendor {
            return Err(SourceError::WrongOrigin {
                expected: SourceKind::Vendor,
                actual: key.origin.clone(),
            });
        }
        if let Some(text) = self
            .loaded
            .read()
            .expect("composer source lock poisoned")
            .get(key)
            .cloned()
        {
            let path = self
                .paths
                .read()
                .expect("composer source lock poisoned")
                .get(key)
                .cloned()
                .ok_or_else(|| SourceError::UnknownFile(key.clone()))?;
            return Ok(SourceFile {
                key: key.clone(),
                path,
                origin: key.origin.clone(),
                fingerprint: text_fingerprint(&text),
                text,
            });
        }
        let path = self
            .paths
            .read()
            .expect("composer source lock poisoned")
            .get(key)
            .cloned()
            .ok_or_else(|| SourceError::UnknownFile(key.clone()))?;
        let text: Arc<str> = fs::read_to_string(&path)?.into();
        self.loads.fetch_add(1, Ordering::Relaxed);
        self.loaded
            .write()
            .expect("composer source lock poisoned")
            .insert(key.clone(), text.clone());
        Ok(SourceFile {
            key: key.clone(),
            path,
            origin: key.origin.clone(),
            fingerprint: text_fingerprint(&text),
            text,
        })
    }

    fn owns(&self, key: &PersistentFileKey) -> bool {
        key.origin.kind() == SourceKind::Vendor
            && self
                .paths
                .read()
                .map(|paths| paths.contains_key(key))
                .unwrap_or(false)
    }

    fn fingerprint(&self) -> SourceFingerprint {
        self.index
            .read()
            .map(|index| SourceFingerprint(index.metadata_fingerprint()))
            .unwrap_or(SourceFingerprint(0))
    }
}

pub struct RuntimeSource {
    index: Arc<RuntimeSymbolIndex>,
    paths: RwLock<HashMap<PersistentFileKey, PathBuf>>,
}

impl std::fmt::Debug for RuntimeSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeSource")
            .finish_non_exhaustive()
    }
}

impl RuntimeSource {
    pub fn new(index: Arc<RuntimeSymbolIndex>) -> Self {
        Self {
            index,
            paths: RwLock::new(HashMap::new()),
        }
    }
}

impl SourceProvider for RuntimeSource {
    fn origin(&self) -> SourceOrigin {
        SourceOrigin::Runtime
    }

    fn resolve_name(&self, name: &str) -> SourceResolution {
        let mut candidates = Vec::new();
        for symbol in self.index.symbols() {
            if symbol.fqn.eq_ignore_ascii_case(name) || symbol.name.eq_ignore_ascii_case(name) {
                let path = canonical_or(&symbol.location.file);
                let key = PersistentFileKey::new(SourceOrigin::Runtime, &path);
                self.paths
                    .write()
                    .expect("runtime source lock poisoned")
                    .insert(key.clone(), path.clone());
                candidates.push(SourceCandidate {
                    key,
                    path,
                    origin: SourceOrigin::Runtime,
                });
            }
        }
        candidates.sort_by(|left, right| left.path.cmp(&right.path));
        candidates.dedup_by(|left, right| left.key == right.key);
        match candidates.len() {
            0 => SourceResolution::NotFound,
            1 => SourceResolution::Found(candidates.remove(0)),
            _ => SourceResolution::Candidates(candidates),
        }
    }

    fn load_source(&self, key: &PersistentFileKey) -> Result<SourceFile, SourceError> {
        if key.origin.kind() != SourceKind::Runtime {
            return Err(SourceError::WrongOrigin {
                expected: SourceKind::Runtime,
                actual: key.origin.clone(),
            });
        }
        let path = self
            .paths
            .read()
            .expect("runtime source lock poisoned")
            .get(key)
            .cloned()
            .ok_or_else(|| SourceError::UnknownFile(key.clone()))?;
        let text: Arc<str> = fs::read_to_string(&path)?.into();
        Ok(SourceFile {
            key: key.clone(),
            path,
            origin: SourceOrigin::Runtime,
            fingerprint: text_fingerprint(&text),
            text,
        })
    }

    fn owns(&self, key: &PersistentFileKey) -> bool {
        key.origin.kind() == SourceKind::Runtime
            && self
                .paths
                .read()
                .map(|paths| paths.contains_key(key))
                .unwrap_or(false)
    }

    fn fingerprint(&self) -> SourceFingerprint {
        SourceFingerprint(self.index.len() as u64)
    }
}

#[derive(Debug, Clone)]
pub struct SourceResolutionPolicy {
    pub precedence: Vec<SourceKind>,
}

impl Default for SourceResolutionPolicy {
    fn default() -> Self {
        // This matches native definition navigation today: project symbols
        // are checked first, then Composer/vendor, then runtime fallback.
        Self {
            precedence: vec![
                SourceKind::Workspace,
                SourceKind::Vendor,
                SourceKind::Runtime,
            ],
        }
    }
}

pub struct SourceRegistry {
    providers: Vec<Arc<dyn SourceProvider>>,
    pub policy: SourceResolutionPolicy,
}

impl std::fmt::Debug for SourceRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SourceRegistry")
            .field("providers", &self.providers.len())
            .field("policy", &self.policy)
            .finish()
    }
}

impl SourceRegistry {
    pub fn new(policy: SourceResolutionPolicy) -> Self {
        Self {
            providers: Vec::new(),
            policy,
        }
    }

    pub fn with_default_policy() -> Self {
        Self::new(SourceResolutionPolicy::default())
    }

    pub fn register(&mut self, provider: Arc<dyn SourceProvider>) {
        self.providers.push(provider);
    }

    pub fn resolve_name(&self, name: &str) -> SourceResolution {
        for kind in &self.policy.precedence {
            let mut candidates = Vec::new();
            for provider in &self.providers {
                if provider.origin().kind() != *kind {
                    continue;
                }
                match provider.resolve_name(name) {
                    SourceResolution::Found(candidate) => candidates.push(candidate),
                    SourceResolution::Candidates(mut found) => candidates.append(&mut found),
                    SourceResolution::Deferred(deferred) => {
                        if candidates.is_empty() {
                            return SourceResolution::Deferred(deferred);
                        }
                    }
                    SourceResolution::NotFound => {}
                }
            }
            candidates.sort_by(|left, right| left.path.cmp(&right.path));
            candidates.dedup_by(|left, right| left.key == right.key);
            match candidates.len() {
                0 => continue,
                1 => return SourceResolution::Found(candidates.remove(0)),
                _ => return SourceResolution::Candidates(candidates),
            }
        }
        SourceResolution::NotFound
    }

    pub fn load_source(&self, candidate: &SourceCandidate) -> Result<SourceFile, SourceError> {
        for provider in &self.providers {
            if provider.owns(&candidate.key) {
                return provider.load_source(&candidate.key);
            }
        }
        Err(SourceError::UnknownFile(candidate.key.clone()))
    }

    pub fn fingerprints(&self) -> Vec<(SourceKind, SourceFingerprint)> {
        self.providers
            .iter()
            .map(|provider| (provider.origin().kind(), provider.fingerprint()))
            .collect()
    }
}

fn is_top_level_symbol(kind: ProjectSymbolKind) -> bool {
    matches!(
        kind,
        ProjectSymbolKind::Class
            | ProjectSymbolKind::Interface
            | ProjectSymbolKind::Trait
            | ProjectSymbolKind::Enum
            | ProjectSymbolKind::Function
            | ProjectSymbolKind::Constant
    )
}

fn dedup_name_keys(names: &mut HashMap<String, Vec<PersistentFileKey>>) {
    for keys in names.values_mut() {
        keys.sort_by(|left, right| left.normalized_path.cmp(&right.normalized_path));
        keys.dedup();
    }
}

fn canonical_or(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn text_fingerprint(text: &str) -> SourceFingerprint {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    SourceFingerprint(hasher.finish())
}

fn fingerprint_paths(paths: &HashMap<PersistentFileKey, PathBuf>) -> SourceFingerprint {
    let mut entries = BTreeSet::new();
    for (key, path) in paths {
        let metadata = fs::metadata(path).ok();
        let size = metadata.as_ref().map_or(0, |value| value.len());
        let modified = metadata
            .and_then(|value| value.modified().ok())
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |value| value.as_nanos());
        entries.insert((key.normalized_path.clone(), size, modified));
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    entries.hash(&mut hasher);
    SourceFingerprint(hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProjectSymbolIndex;
    use std::sync::Arc;

    #[test]
    fn workspace_resolves_known_file_and_prefers_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("User.php");
        fs::write(&path, "<?php namespace App; class User {}").unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let source = WorkspaceSource::from_project_index(dir.path(), &index);
        let candidate = match source.resolve_name("App\\User") {
            SourceResolution::Found(candidate) => candidate,
            other => panic!("unexpected resolution: {other:?}"),
        };
        source.set_buffer(&path, "<?php namespace App; class User { }");
        let loaded = source.load_source(&candidate.key).unwrap();
        assert_eq!(loaded.origin, SourceOrigin::Workspace);
        assert!(loaded.text.contains("class User"));
        assert_eq!(candidate.key, PersistentFileKey::workspace(&path));
    }

    #[test]
    fn workspace_missing_name_is_not_found_and_path_is_normalized() {
        let dir = tempfile::tempdir().unwrap();
        let source = WorkspaceSource::new(dir.path());
        assert_eq!(source.resolve_name("Missing"), SourceResolution::NotFound);
        let key = source.register_file(dir.path().join("a/../Thing.php"));
        assert_eq!(
            key,
            PersistentFileKey::workspace(dir.path().join("Thing.php"))
        );
    }

    #[test]
    fn composer_resolves_classmap_and_deduplicates_file_loads() {
        let dir = tempfile::tempdir().unwrap();
        let composer = dir.path().join("vendor/composer");
        let file = dir.path().join("vendor/pkg/src/Shared.php");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::create_dir_all(&composer).unwrap();
        fs::write(&file, "<?php namespace Pkg; class First {} class Second {}").unwrap();
        fs::write(
            composer.join("autoload_classmap.php"),
            "<?php\n'Pkg\\\\First' => $vendorDir . '/pkg/src/Shared.php'\n'Pkg\\\\Second' => $vendorDir . '/pkg/src/Shared.php'\n",
        )
        .unwrap();
        let index = Arc::new(RwLock::new(VendorSymbolIndex::load(dir.path()).unwrap()));
        let source = ComposerSource::new(index);
        let first = source.resolve_name("Pkg\\First");
        let second = source.resolve_name("Pkg\\Second");
        let first = first.candidates()[0].clone();
        let second = second.candidates()[0].clone();
        assert_eq!(first.key, second.key);
        source.load_source(&first.key).unwrap();
        source.load_source(&second.key).unwrap();
        assert_eq!(source.load_count(), 1);
    }

    #[test]
    fn composer_psr4_uses_specific_prefix_and_multiple_directories() {
        let dir = tempfile::tempdir().unwrap();
        let composer = dir.path().join("vendor/composer");
        let broad = dir.path().join("vendor/pkg/src/Thing.php");
        let specific = dir.path().join("vendor/pkg/src/Deep/Thing.php");
        fs::create_dir_all(broad.parent().unwrap()).unwrap();
        fs::create_dir_all(specific.parent().unwrap()).unwrap();
        fs::create_dir_all(&composer).unwrap();
        fs::write(&broad, "<?php namespace Pkg; class Thing {}").unwrap();
        fs::write(&specific, "<?php namespace Pkg\\Deep; class Thing {}").unwrap();
        fs::write(
            composer.join("autoload_psr4.php"),
            "'Pkg\\\\Deep\\\\' => array($vendorDir . '/pkg/src/Deep'),\n'Pkg\\\\' => array($vendorDir . '/pkg/src'),",
        )
        .unwrap();
        let source = ComposerSource::new(Arc::new(RwLock::new(
            VendorSymbolIndex::load(dir.path()).unwrap(),
        )));
        let candidate = source.resolve_name("Pkg\\Deep\\Thing");
        assert_eq!(candidate.candidates().len(), 1);
        assert_eq!(
            candidate.candidates()[0].path,
            fs::canonicalize(specific).unwrap()
        );
    }

    #[test]
    fn runtime_source_marks_origin_and_resolves_known_symbol() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Runtime.php");
        fs::write(&file, "<?php class RuntimeThing {}").unwrap();
        let mut runtime = RuntimeSymbolIndex::default();
        runtime.insert(axiom_php::Symbol {
            name: "RuntimeThing".into(),
            fqn: "RuntimeThing".into(),
            kind: axiom_php::SymbolKind::Class,
            origin: axiom_php::SymbolOrigin::PhpRuntime,
            extension: String::new(),
            location: axiom_php::SourceLocation {
                file: file.clone(),
                range: 0..10,
            },
            declared_type: None,
            signature: None,
            documentation: None,
            availability: axiom_php::Availability::default(),
            is_static: false,
        });
        let source = RuntimeSource::new(Arc::new(runtime));
        let candidate = source.resolve_name("RuntimeThing");
        assert_eq!(candidate.candidates()[0].origin, SourceOrigin::Runtime);
        assert!(source.load_source(&candidate.candidates()[0].key).is_ok());
        assert_eq!(source.resolve_name("Missing"), SourceResolution::NotFound);
    }

    #[test]
    fn registry_preserves_workspace_vendor_runtime_precedence_and_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Thing.php");
        fs::write(&path, "<?php namespace App; class Thing {}").unwrap();
        let mut project = ProjectSymbolIndex::new();
        project.index_project(dir.path()).unwrap();
        let workspace = Arc::new(WorkspaceSource::from_project_index(dir.path(), &project));
        let runtime = Arc::new(RuntimeSource::new(Arc::new(RuntimeSymbolIndex::default())));
        let mut registry = SourceRegistry::with_default_policy();
        registry.register(workspace);
        registry.register(runtime);
        let result = registry.resolve_name("App\\Thing");
        assert_eq!(result.candidates().len(), 1);
        assert_eq!(result.candidates()[0].origin, SourceOrigin::Workspace);

        let duplicate_a = dir.path().join("A.php");
        let duplicate_b = dir.path().join("B.php");
        fs::write(&duplicate_a, "<?php namespace App; class Duplicate {}").unwrap();
        fs::write(&duplicate_b, "<?php namespace App; class Duplicate {}").unwrap();
        let mut duplicate_index = ProjectSymbolIndex::new();
        duplicate_index.index_project(dir.path()).unwrap();
        let duplicate_source = WorkspaceSource::from_project_index(dir.path(), &duplicate_index);
        assert_eq!(
            duplicate_source
                .resolve_name("App\\Duplicate")
                .candidates()
                .len(),
            2
        );
    }
}
