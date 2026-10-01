use std::{
    env,
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
    time::Instant,
};

use crate::{
    EmbeddedStubArtifact, LoadReport, RuntimeSymbolIndex, StubProvider, StubProviderError,
    STUB_PARSER_VERSION,
};

mod generated {
    include!(concat!(env!("OUT_DIR"), "/embedded_stub_artifact.rs"));
}

use generated::EMBEDDED_STUB_ARTIFACT;

pub use crate::artifact::EMBEDDED_STUB_SCHEMA_VERSION;

#[derive(Debug)]
pub enum EmbeddedStubError {
    InvalidArtifact(String),
    UnsupportedSchema { expected: u32, actual: u32 },
    UnsupportedParser { expected: u32, actual: u32 },
}

impl fmt::Display for EmbeddedStubError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArtifact(error) => write!(formatter, "invalid embedded stub artifact: {error}"),
            Self::UnsupportedSchema { expected, actual } => write!(
                formatter,
                "unsupported embedded stub schema {actual}; expected {expected}"
            ),
            Self::UnsupportedParser { expected, actual } => write!(
                formatter,
                "unsupported embedded stub parser {actual}; expected {expected}"
            ),
        }
    }
}

impl std::error::Error for EmbeddedStubError {}

#[derive(Debug, Clone, Copy)]
pub struct EmbeddedStubProvider {
    artifact: &'static [u8],
}

impl EmbeddedStubProvider {
    pub fn bundled() -> Self {
        Self {
            artifact: EMBEDDED_STUB_ARTIFACT,
        }
    }

    pub fn from_static(artifact: &'static [u8]) -> Self {
        Self { artifact }
    }

    pub fn load(
        &self,
    ) -> Result<(RuntimeSymbolIndex, LoadReport), EmbeddedStubError> {
        let started = Instant::now();
        let artifact: EmbeddedStubArtifact =
            bincode::deserialize(self.artifact).map_err(|error| EmbeddedStubError::InvalidArtifact(error.to_string()))?;
        if artifact.schema_version != EMBEDDED_STUB_SCHEMA_VERSION {
            return Err(EmbeddedStubError::UnsupportedSchema {
                expected: EMBEDDED_STUB_SCHEMA_VERSION,
                actual: artifact.schema_version,
            });
        }
        if artifact.parser_version != STUB_PARSER_VERSION {
            return Err(EmbeddedStubError::UnsupportedParser {
                expected: STUB_PARSER_VERSION,
                actual: artifact.parser_version,
            });
        }

        let mut index = RuntimeSymbolIndex::default();
        let mut report = LoadReport {
            files_discovered: 1,
            files_parsed: 1,
            ..Default::default()
        };
        for symbol in artifact.symbols {
            report.symbols_indexed += 1;
            index.insert(symbol);
        }
        report.elapsed = started.elapsed();
        Ok((index, report))
    }
}

#[derive(Debug)]
pub enum RuntimeStubLoadError {
    External(StubProviderError),
    Embedded(EmbeddedStubError),
}

impl fmt::Display for RuntimeStubLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::External(error) => error.fmt(formatter),
            Self::Embedded(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for RuntimeStubLoadError {}

impl From<StubProviderError> for RuntimeStubLoadError {
    fn from(value: StubProviderError) -> Self {
        Self::External(value)
    }
}

impl From<EmbeddedStubError> for RuntimeStubLoadError {
    fn from(value: EmbeddedStubError) -> Self {
        Self::Embedded(value)
    }
}

#[derive(Debug, Clone)]
pub enum RuntimeStubProvider {
    External(StubProvider),
    Embedded(EmbeddedStubProvider),
}

impl RuntimeStubProvider {
    pub fn from_env_or_embedded() -> Self {
        Self::from_overrides(
            env::var_os("AXIOM_PHP_STUBS"),
            env::var_os("RUSTSTORM_PHP_STUBS"),
        )
    }

    pub fn from_overrides(
        axiom_php_stubs: Option<OsString>,
        ruststorm_php_stubs: Option<OsString>,
    ) -> Self {
        if let Some(root) = axiom_php_stubs {
            return Self::External(StubProvider::new(PathBuf::from(root)));
        }
        if let Some(root) = ruststorm_php_stubs {
            return Self::External(StubProvider::new(PathBuf::from(root)));
        }
        Self::Embedded(EmbeddedStubProvider::bundled())
    }

    pub fn external(root: impl Into<PathBuf>) -> Self {
        Self::External(StubProvider::new(root))
    }

    pub fn is_embedded(&self) -> bool {
        matches!(self, Self::Embedded(_))
    }

    pub fn root(&self) -> Option<&Path> {
        match self {
            Self::External(provider) => Some(provider.root()),
            Self::Embedded(_) => None,
        }
    }

    pub fn load(&self) -> Result<(RuntimeSymbolIndex, LoadReport), RuntimeStubLoadError> {
        match self {
            Self::External(provider) => provider.load().map_err(Into::into),
            Self::Embedded(provider) => provider.load().map_err(Into::into),
        }
    }

    pub fn load_incremental(
        &self,
        cache_path: &Path,
    ) -> Result<(RuntimeSymbolIndex, LoadReport), RuntimeStubLoadError> {
        match self {
            Self::External(provider) => provider.load_incremental(cache_path).map_err(Into::into),
            Self::Embedded(provider) => provider.load().map_err(Into::into),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{SymbolKind, SymbolOrigin};

    #[test]
    fn embedded_provider_indexes_symbols_without_a_filesystem_root() {
        let provider = EmbeddedStubProvider::bundled();
        let (index, report) = provider.load().expect("embedded runtime artifact must load");

        assert!(provider.load().is_ok());
        assert_eq!(report.files_discovered, 1);
        assert_eq!(report.files_parsed, 1);
        assert!(report.symbols_indexed > 1, "real embedded dataset should contain multiple symbols");
        assert!(index.find_class("DateTime").is_some(), "DateTime must be embedded");
        assert!(index.find_class("PDO").is_some(), "PDO must be embedded");
        assert!(index.find_function("strlen").is_some(), "strlen must be embedded");
        assert!(index.find_function("array_map").is_some(), "array_map must be embedded");
    }

    #[test]
    fn resident_index_supports_prefix_and_member_lookup_without_reloading_provider() {
        let (index, _) = EmbeddedStubProvider::bundled().load().expect("embedded runtime artifact must load");
        let resident = Arc::new(index);

        assert!(resident.search_prefix_limited("DateTime", 16).iter().any(|symbol| {
            symbol.name.starts_with("DateTime") || symbol.fqn.starts_with("DateTime")
        }));
        assert!(!resident.members_of("PDO").is_empty(), "PDO should have indexed members");
    }

    #[test]
    fn environment_overrides_prefer_axiom_then_ruststorm_then_embedded() {
        let axiom = RuntimeStubProvider::from_overrides(
            Some(OsString::from("axiom-stubs")),
            Some(OsString::from("ruststorm-stubs")),
        );
        assert_eq!(axiom.root(), Some(Path::new("axiom-stubs")));

        let ruststorm = RuntimeStubProvider::from_overrides(
            None,
            Some(OsString::from("ruststorm-stubs")),
        );
        assert_eq!(ruststorm.root(), Some(Path::new("ruststorm-stubs")));

        let embedded = RuntimeStubProvider::from_overrides(None, None);
        assert!(embedded.is_embedded());
        assert_eq!(embedded.root(), None);
    }

    #[test]
    fn compact_artifact_preserves_existing_symbol_model() {
        let (index, _) = EmbeddedStubProvider::bundled().load().expect("embedded runtime artifact must load");
        let method = index
            .methods_of("PDO")
            .find(|symbol| symbol.signature.is_some())
            .expect("PDO should contain a method with a signature");
        let signature = method.signature.as_ref().expect("selected method signature");

        assert_eq!(method.kind, SymbolKind::Method);
        assert_eq!(method.origin, SymbolOrigin::PhpRuntime);
        assert!(!method.name.is_empty());
        assert!(method.location.file.to_string_lossy().ends_with(".php"));
        assert!(signature.parameters.iter().all(|parameter| !parameter.name.is_empty()));
    }

}
