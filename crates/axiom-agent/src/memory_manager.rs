//! Lifecycle/configuration boundary for a future persistent memory backend.
//!
//! The manager does not launch processes or perform health checks itself. A
//! caller may execute those operations on a background worker and feed their
//! results back through this state machine.

use std::path::{Path, PathBuf};

use crate::{MemoryService, NoMemory};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemoryBackendConfig {
    Disabled,
    External {
        executable: PathBuf,
        data_directory: PathBuf,
    },
}

impl MemoryBackendConfig {
    pub fn external(executable: impl Into<PathBuf>, data_directory: impl Into<PathBuf>) -> Self {
        Self::External {
            executable: executable.into(),
            data_directory: data_directory.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryLaunchRequest {
    pub executable: PathBuf,
    pub data_directory: PathBuf,
}

impl MemoryLaunchRequest {
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn data_directory(&self) -> &Path {
        &self.data_directory
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryStatus {
    Disabled,
    Starting,
    Ready,
    Unavailable(MemoryUnavailableReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryUnavailableReason {
    ExecutableMissing,
    LaunchFailed,
    HealthCheckFailed,
    ShutdownFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryManagerError {
    ExecutableMissing,
    LaunchFailed,
    HealthCheckFailed,
    ShutdownFailed,
}

impl MemoryManagerError {
    const fn status(self) -> MemoryUnavailableReason {
        match self {
            Self::ExecutableMissing => MemoryUnavailableReason::ExecutableMissing,
            Self::LaunchFailed => MemoryUnavailableReason::LaunchFailed,
            Self::HealthCheckFailed => MemoryUnavailableReason::HealthCheckFailed,
            Self::ShutdownFailed => MemoryUnavailableReason::ShutdownFailed,
        }
    }
}

pub trait MemoryHealthCheck: Send {
    fn shutdown(&mut self) -> Result<(), MemoryManagerError>;
}

/// Process-launch seam. Implementations are expected to be called by a
/// background worker, never from a GPUI/render/input callback.
pub trait MemoryBackendLauncher: Send {
    fn launch(
        &mut self,
        request: &MemoryLaunchRequest,
    ) -> Result<Box<dyn MemoryHealthCheck>, MemoryManagerError>;
}

pub fn default_data_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|path| path.join("Axiom").join("ai-memory"))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|path| path.join(".local").join("share"))
            })
            .map(|path| path.join("Axiom").join("ai-memory"))
    }
}

pub struct MemoryManager {
    config: MemoryBackendConfig,
    status: MemoryStatus,
    service: Box<dyn MemoryService>,
    health: Option<Box<dyn MemoryHealthCheck>>,
}

impl MemoryManager {
    pub fn new(config: MemoryBackendConfig) -> Self {
        let status = match config {
            MemoryBackendConfig::Disabled => MemoryStatus::Disabled,
            MemoryBackendConfig::External { .. } => MemoryStatus::Disabled,
        };
        Self {
            config,
            status,
            service: Box::new(NoMemory),
            health: None,
        }
    }

    pub fn config(&self) -> &MemoryBackendConfig {
        &self.config
    }

    pub fn status(&self) -> MemoryStatus {
        self.status
    }

    pub fn service(&self) -> &dyn MemoryService {
        self.service.as_ref()
    }

    /// Installs a read/write-neutral Axiom memory service after the external
    /// backend has reached `Ready`. Backend construction remains outside the
    /// manager because protocol details belong to the backend adapter.
    pub fn install_service(&mut self, service: Box<dyn MemoryService>) -> bool {
        if self.status != MemoryStatus::Ready {
            return false;
        }
        self.service = service;
        true
    }

    pub fn begin_start(&mut self) -> Option<MemoryLaunchRequest> {
        let MemoryBackendConfig::External {
            executable,
            data_directory,
        } = &self.config
        else {
            self.status = MemoryStatus::Disabled;
            return None;
        };
        self.status = MemoryStatus::Starting;
        Some(MemoryLaunchRequest {
            executable: executable.clone(),
            data_directory: data_directory.clone(),
        })
    }

    pub fn complete_launch(
        &mut self,
        result: Result<Box<dyn MemoryHealthCheck>, MemoryManagerError>,
    ) {
        match result {
            Ok(health) => {
                self.health = Some(health);
                self.status = MemoryStatus::Starting;
            }
            Err(error) => {
                self.health = None;
                self.service = Box::new(NoMemory);
                self.status = MemoryStatus::Unavailable(error.status());
            }
        }
    }

    pub fn complete_health_check(&mut self, result: Result<(), MemoryManagerError>) {
        match result {
            Ok(()) => self.status = MemoryStatus::Ready,
            Err(error) => {
                self.status = MemoryStatus::Unavailable(error.status());
                self.health = None;
                self.service = Box::new(NoMemory);
            }
        }
    }

    pub fn shutdown(&mut self) {
        let result = self
            .health
            .take()
            .map(|mut health| health.shutdown())
            .unwrap_or(Ok(()));
        self.status = match result {
            Ok(()) => {
                self.service = Box::new(NoMemory);
                MemoryStatus::Disabled
            }
            Err(error) => {
                self.service = Box::new(NoMemory);
                MemoryStatus::Unavailable(error.status())
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryScope;

    struct FakeHealth {
        shutdown: Result<(), MemoryManagerError>,
    }

    impl MemoryHealthCheck for FakeHealth {
        fn shutdown(&mut self) -> Result<(), MemoryManagerError> {
            self.shutdown
        }
    }

    #[test]
    fn disabled_does_not_create_a_start_request() {
        let mut manager = MemoryManager::new(MemoryBackendConfig::Disabled);
        assert_eq!(manager.status(), MemoryStatus::Disabled);
        assert!(manager.begin_start().is_none());
        assert_eq!(manager.status(), MemoryStatus::Disabled);
    }

    #[test]
    fn configured_backend_reaches_ready_after_launch_and_health_check() {
        let mut manager = MemoryManager::new(MemoryBackendConfig::external(
            "memory-backend.exe",
            "test-data",
        ));
        let request = manager.begin_start().unwrap();
        assert_eq!(manager.status(), MemoryStatus::Starting);
        assert_eq!(request.executable(), Path::new("memory-backend.exe"));
        manager.complete_launch(Ok(Box::new(FakeHealth { shutdown: Ok(()) })));
        assert_eq!(manager.status(), MemoryStatus::Starting);
        manager.complete_health_check(Ok(()));
        assert_eq!(manager.status(), MemoryStatus::Ready);
    }

    #[test]
    fn launch_and_health_failures_are_sanitized_and_non_fatal() {
        let mut manager = MemoryManager::new(MemoryBackendConfig::external("missing.exe", "data"));
        manager.begin_start();
        manager.complete_launch(Err(MemoryManagerError::ExecutableMissing));
        assert_eq!(
            manager.status(),
            MemoryStatus::Unavailable(MemoryUnavailableReason::ExecutableMissing)
        );

        let mut manager = MemoryManager::new(MemoryBackendConfig::external("backend", "data"));
        manager.begin_start();
        manager.complete_launch(Ok(Box::new(FakeHealth { shutdown: Ok(()) })));
        manager.complete_health_check(Err(MemoryManagerError::HealthCheckFailed));
        assert_eq!(
            manager.status(),
            MemoryStatus::Unavailable(MemoryUnavailableReason::HealthCheckFailed)
        );
        assert!(
            manager
                .service()
                .query(MemoryScope::Project, "anything", 1)
                .is_empty()
        );
    }

    #[test]
    fn launch_request_preserves_injected_data_directory() {
        let mut manager = MemoryManager::new(MemoryBackendConfig::external(
            "backend",
            "/isolated/test-data",
        ));
        let request = manager.begin_start().unwrap();
        assert_eq!(request.data_directory(), Path::new("/isolated/test-data"));
    }

    #[test]
    fn shutdown_is_safe_and_uses_no_memory_fallback() {
        let mut manager = MemoryManager::new(MemoryBackendConfig::external("backend", "data"));
        manager.begin_start();
        manager.complete_launch(Ok(Box::new(FakeHealth {
            shutdown: Err(MemoryManagerError::ShutdownFailed),
        })));
        manager.complete_health_check(Ok(()));
        manager.shutdown();
        assert_eq!(
            manager.status(),
            MemoryStatus::Unavailable(MemoryUnavailableReason::ShutdownFailed)
        );
        assert!(
            manager
                .service()
                .briefing(MemoryScope::Workspace, 1)
                .is_empty()
        );
    }
}
