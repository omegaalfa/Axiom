//! Bounded in-process memory worker.
//!
//! The worker owns the persistent backend. Callers only hold a synchronous
//! proxy implementing `MemoryService`; potentially blocking JSON/SQLite work
//! therefore never runs while the proxy is waiting on the UI thread.

use crate::{
    AxiomMemoryBackend, AxiomMemoryError, AxiomMemoryScopeIds, MemoryHandoff, MemoryManagerError,
    MemoryObservation, MemoryReadError, MemoryResult, MemoryScope, MemoryService, MemorySessionId,
    MemorySnapshot, MemorySnapshotError, MemoryStatus, MemoryUnavailableReason, MemoryWriteError,
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex, OnceLock, Weak},
    time::Duration,
};

const COMMAND_CAPACITY: usize = 64;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);

type ReadReply<T> = mpsc::SyncSender<Result<T, MemoryReadError>>;
type WriteReply = mpsc::SyncSender<Result<(), MemoryWriteError>>;

enum MemoryCommand {
    SessionStart {
        scope: MemoryScope,
        reply: ReadReply<MemorySessionId>,
    },
    Observe {
        session: MemorySessionId,
        observation: MemoryObservation,
    },
    SessionEnd {
        session: MemorySessionId,
    },
    Query {
        scope: MemoryScope,
        query: String,
        limit: usize,
        reply: ReadReply<Vec<MemoryResult>>,
    },
    Briefing {
        scope: MemoryScope,
        limit: usize,
        reply: ReadReply<Vec<MemoryResult>>,
    },
    Recent {
        scope: MemoryScope,
        limit: usize,
        reply: ReadReply<Vec<MemoryResult>>,
    },
    History {
        scope: MemoryScope,
        limit: usize,
        reply: ReadReply<Vec<MemoryResult>>,
    },
    Snapshot {
        scope: MemoryScope,
        limit: usize,
        reply: ReadReply<MemorySnapshot>,
    },
    Get {
        scope: MemoryScope,
        id: String,
        reply: ReadReply<Option<MemoryResult>>,
    },
    Remember {
        scope: MemoryScope,
        content: String,
        reply: WriteReply,
    },
    Shutdown {
        reply: mpsc::SyncSender<Result<(), MemoryManagerError>>,
    },
}

struct WorkerInner {
    sender: mpsc::SyncSender<MemoryCommand>,
    status: Mutex<MemoryStatus>,
    last_error: Mutex<Option<MemoryManagerError>>,
    timeout: Duration,
    registry_key: String,
}

static WORKERS: OnceLock<Mutex<HashMap<String, Weak<WorkerInner>>>> = OnceLock::new();

/// Observable non-fatal diagnostics for best-effort operations and queue
/// pressure. It intentionally contains no memory content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryWorkerDiagnostics {
    pub last_error: Option<MemoryManagerError>,
}

/// Cloneable proxy to one backend-owning worker.
#[derive(Clone)]
pub struct BackgroundMemoryService {
    inner: Arc<WorkerInner>,
}

impl BackgroundMemoryService {
    pub fn start(
        data_directory: impl Into<PathBuf>,
        scope_ids: AxiomMemoryScopeIds,
    ) -> Result<Self, MemoryManagerError> {
        Self::start_with_timeout(data_directory.into(), scope_ids, DEFAULT_TIMEOUT)
    }

    pub fn start_with_timeout(
        data_directory: PathBuf,
        scope_ids: AxiomMemoryScopeIds,
        timeout: Duration,
    ) -> Result<Self, MemoryManagerError> {
        let key = format!(
            "{}|{}|{}|{}",
            data_directory.to_string_lossy(),
            scope_ids.user,
            scope_ids.workspace,
            scope_ids.project
        );
        let registry = WORKERS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut registry_guard = registry
            .lock()
            .map_err(|_| MemoryManagerError::WorkerFailed)?;
        if let Some(existing) = registry_guard.get(&key).and_then(Weak::upgrade) {
            return Ok(Self { inner: existing });
        }
        let (sender, receiver) = mpsc::sync_channel(COMMAND_CAPACITY);
        let inner = Arc::new(WorkerInner {
            sender,
            status: Mutex::new(MemoryStatus::Starting),
            last_error: Mutex::new(None),
            timeout,
            registry_key: key.clone(),
        });
        registry_guard.insert(key.clone(), Arc::downgrade(&inner));
        drop(registry_guard);

        let weak = Arc::downgrade(&inner);
        if std::thread::Builder::new()
            .name("axiom-memory-worker".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    worker_loop(data_directory, scope_ids, receiver, weak.clone())
                }));
                if result.is_err() {
                    if let Some(inner) = weak.upgrade() {
                        set_error(&inner, MemoryManagerError::WorkerFailed);
                        set_status(
                            &inner,
                            MemoryStatus::Unavailable(MemoryUnavailableReason::WorkerFailed),
                        );
                    }
                    remove_registry_entry(&weak);
                }
            })
            .is_err()
        {
            remove_registry_entry(&Arc::downgrade(&inner));
            set_error(&inner, MemoryManagerError::LaunchFailed);
            set_status(
                &inner,
                MemoryStatus::Unavailable(MemoryUnavailableReason::LaunchFailed),
            );
            return Err(MemoryManagerError::LaunchFailed);
        }
        Ok(Self { inner })
    }

    pub fn start_ready(
        data_directory: impl Into<PathBuf>,
        scope_ids: AxiomMemoryScopeIds,
        timeout: Duration,
    ) -> Result<Self, MemoryManagerError> {
        let service = Self::start_with_timeout(data_directory.into(), scope_ids, timeout)?;
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match service.status() {
                MemoryStatus::Ready => return Ok(service),
                MemoryStatus::Unavailable(_) | MemoryStatus::Disabled => {
                    return Err(MemoryManagerError::HealthCheckFailed);
                }
                MemoryStatus::Starting | MemoryStatus::Stopping => {
                    if std::time::Instant::now() >= deadline {
                        set_status(
                            &service.inner,
                            MemoryStatus::Unavailable(MemoryUnavailableReason::Timeout),
                        );
                        return Err(MemoryManagerError::Timeout);
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }

    pub fn status(&self) -> MemoryStatus {
        self.inner
            .status
            .lock()
            .map(|status| *status)
            .unwrap_or(MemoryStatus::Unavailable(
                MemoryUnavailableReason::WorkerFailed,
            ))
    }

    pub fn diagnostics(&self) -> MemoryWorkerDiagnostics {
        MemoryWorkerDiagnostics {
            last_error: self.inner.last_error.lock().ok().and_then(|error| *error),
        }
    }

    pub fn shutdown(&self) -> Result<(), MemoryManagerError> {
        if self.status() == MemoryStatus::Disabled {
            return Ok(());
        }
        set_status(&self.inner, MemoryStatus::Stopping);
        let (reply, receiver) = mpsc::sync_channel(1);
        send_command(&self.inner, MemoryCommand::Shutdown { reply })
            .map_err(|_| MemoryManagerError::ShutdownFailed)?;
        match receiver.recv_timeout(self.inner.timeout) {
            Ok(result) => result,
            Err(_) => {
                set_error(&self.inner, MemoryManagerError::Timeout);
                set_status(
                    &self.inner,
                    MemoryStatus::Unavailable(MemoryUnavailableReason::Timeout),
                );
                Err(MemoryManagerError::Timeout)
            }
        }
    }

    fn read<T>(
        &self,
        command: MemoryCommand,
        receiver: mpsc::Receiver<Result<T, MemoryReadError>>,
    ) -> Result<T, MemoryReadError> {
        send_command(&self.inner, command)?;
        match receiver.recv_timeout(self.inner.timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => Err(MemoryReadError::Timeout),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(MemoryReadError::Unavailable),
        }
    }

    fn write(
        &self,
        command: MemoryCommand,
        receiver: mpsc::Receiver<Result<(), MemoryWriteError>>,
    ) -> Result<(), MemoryWriteError> {
        send_command(&self.inner, command).map_err(|error| match error {
            MemoryReadError::Timeout => MemoryWriteError::Timeout,
            _ => MemoryWriteError::Unavailable,
        })?;
        match receiver.recv_timeout(self.inner.timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => Err(MemoryWriteError::Timeout),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(MemoryWriteError::Unavailable),
        }
    }
}

impl Drop for WorkerInner {
    fn drop(&mut self) {
        if let Ok(mut status) = self.status.lock() {
            if *status != MemoryStatus::Disabled {
                *status = MemoryStatus::Stopping;
            }
        }
        let (reply, _receiver) = mpsc::sync_channel(1);
        let _ = self.sender.try_send(MemoryCommand::Shutdown { reply });
    }
}

impl MemoryService for BackgroundMemoryService {
    fn request_shutdown(&mut self) {
        if self.status() == MemoryStatus::Disabled {
            return;
        }
        set_status(&self.inner, MemoryStatus::Stopping);
        let (reply, _receiver) = mpsc::sync_channel(1);
        if self
            .inner
            .sender
            .try_send(MemoryCommand::Shutdown { reply })
            .is_err()
        {
            set_error(&self.inner, MemoryManagerError::ShutdownFailed);
        }
    }

    fn is_available(&self) -> bool {
        self.status() == MemoryStatus::Ready
    }

    fn can_remember(&self) -> bool {
        self.status() != MemoryStatus::Disabled
    }

    fn session_start(&mut self, scope: MemoryScope) -> MemorySessionId {
        let (reply, receiver) = mpsc::sync_channel(1);
        match self.read(MemoryCommand::SessionStart { scope, reply }, receiver) {
            Ok(session) => session,
            Err(error) => {
                set_error(&self.inner, read_error_to_manager(error));
                MemorySessionId::new(0)
            }
        }
    }

    fn observe(&mut self, session: MemorySessionId, observation: MemoryObservation) {
        if send_command(
            &self.inner,
            MemoryCommand::Observe {
                session,
                observation,
            },
        )
        .is_err()
        {
            set_error(&self.inner, MemoryManagerError::Timeout);
        }
    }

    fn session_end(&mut self, session: MemorySessionId) {
        if send_command(&self.inner, MemoryCommand::SessionEnd { session }).is_err() {
            set_error(&self.inner, MemoryManagerError::Timeout);
        }
    }

    fn query(&self, scope: MemoryScope, query: &str, limit: usize) -> Vec<MemoryResult> {
        self.query_checked(scope, query, limit).unwrap_or_default()
    }

    fn query_checked(
        &self,
        scope: MemoryScope,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MemoryResult>, MemoryReadError> {
        let (reply, receiver) = mpsc::sync_channel(1);
        self.read(
            MemoryCommand::Query {
                scope,
                query: query.to_owned(),
                limit,
                reply,
            },
            receiver,
        )
    }

    fn briefing(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        self.briefing_checked(scope, limit).unwrap_or_default()
    }

    fn briefing_checked(
        &self,
        scope: MemoryScope,
        limit: usize,
    ) -> Result<Vec<MemoryResult>, MemoryReadError> {
        let (reply, receiver) = mpsc::sync_channel(1);
        self.read(
            MemoryCommand::Briefing {
                scope,
                limit,
                reply,
            },
            receiver,
        )
    }

    fn recent(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        self.recent_checked(scope, limit).unwrap_or_default()
    }

    fn recent_checked(
        &self,
        scope: MemoryScope,
        limit: usize,
    ) -> Result<Vec<MemoryResult>, MemoryReadError> {
        let (reply, receiver) = mpsc::sync_channel(1);
        self.read(
            MemoryCommand::Recent {
                scope,
                limit,
                reply,
            },
            receiver,
        )
    }

    fn history(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        self.history_checked(scope, limit).unwrap_or_default()
    }

    fn history_checked(
        &self,
        scope: MemoryScope,
        limit: usize,
    ) -> Result<Vec<MemoryResult>, MemoryReadError> {
        let (reply, receiver) = mpsc::sync_channel(1);
        self.read(
            MemoryCommand::History {
                scope,
                limit,
                reply,
            },
            receiver,
        )
    }

    fn snapshot(
        &self,
        scope: MemoryScope,
        limit: usize,
    ) -> Result<MemorySnapshot, MemorySnapshotError> {
        let (reply, receiver) = mpsc::sync_channel(1);
        self.read(
            MemoryCommand::Snapshot {
                scope,
                limit,
                reply,
            },
            receiver,
        )
        .map_err(|_| MemorySnapshotError)
    }

    fn get(&self, scope: MemoryScope, id: &str) -> Option<MemoryResult> {
        self.get_checked(scope, id).ok().flatten()
    }

    fn get_checked(
        &self,
        scope: MemoryScope,
        id: &str,
    ) -> Result<Option<MemoryResult>, MemoryReadError> {
        let (reply, receiver) = mpsc::sync_channel(1);
        self.read(
            MemoryCommand::Get {
                scope,
                id: id.to_owned(),
                reply,
            },
            receiver,
        )
    }

    fn handoff(&self, _session: MemorySessionId) -> Option<MemoryHandoff> {
        None
    }

    fn remember(&self, scope: MemoryScope, content: &str) -> Result<(), MemoryWriteError> {
        let (reply, receiver) = mpsc::sync_channel(1);
        self.write(
            MemoryCommand::Remember {
                scope,
                content: content.to_owned(),
                reply,
            },
            receiver,
        )
    }
}

fn send_command(inner: &WorkerInner, command: MemoryCommand) -> Result<(), MemoryReadError> {
    inner.sender.try_send(command).map_err(|error| match error {
        mpsc::TrySendError::Full(_) => MemoryReadError::Timeout,
        mpsc::TrySendError::Disconnected(_) => MemoryReadError::Unavailable,
    })
}

fn set_status(inner: &WorkerInner, status: MemoryStatus) {
    if let Ok(mut current) = inner.status.lock() {
        *current = status;
    }
}

fn set_error(inner: &WorkerInner, error: MemoryManagerError) {
    if let Ok(mut current) = inner.last_error.lock() {
        *current = Some(error);
    }
}

fn read_error_to_manager(error: MemoryReadError) -> MemoryManagerError {
    match error {
        MemoryReadError::Timeout => MemoryManagerError::Timeout,
        _ => MemoryManagerError::WorkerFailed,
    }
}

fn memory_error_to_manager(_error: AxiomMemoryError) -> MemoryManagerError {
    MemoryManagerError::WorkerFailed
}

fn worker_loop(
    data_directory: PathBuf,
    scope_ids: AxiomMemoryScopeIds,
    receiver: mpsc::Receiver<MemoryCommand>,
    weak: Weak<WorkerInner>,
) {
    let Some(inner) = weak.upgrade() else {
        return;
    };
    let mut backend = AxiomMemoryBackend::new(data_directory, scope_ids);
    if backend.initialize_storage().is_err() {
        set_status(
            &inner,
            MemoryStatus::Unavailable(MemoryUnavailableReason::HealthCheckFailed),
        );
    } else {
        set_status(&inner, MemoryStatus::Ready);
    }
    drop(inner);

    while let Ok(command) = receiver.recv() {
        let Some(inner) = weak.upgrade() else {
            return;
        };
        match command {
            MemoryCommand::SessionStart { scope, reply } => {
                let result = if is_ready(&inner) {
                    Ok(MemoryService::session_start(&mut backend, scope))
                } else {
                    Err(MemoryReadError::Unavailable)
                };
                let _ = reply.send(result);
            }
            MemoryCommand::Observe {
                session,
                observation,
            } => {
                if is_ready(&inner) {
                    if let Err(error) = backend.observe_checked(session, observation) {
                        set_error(&inner, memory_error_to_manager(error));
                    }
                }
            }
            MemoryCommand::SessionEnd { session } => {
                if is_ready(&inner) {
                    if let Err(error) = backend.session_end_checked(session) {
                        set_error(&inner, memory_error_to_manager(error));
                    }
                }
            }
            MemoryCommand::Query {
                scope,
                query,
                limit,
                reply,
            } => {
                let result = if is_ready(&inner) {
                    backend.query_checked(scope, &query, limit)
                } else {
                    Err(MemoryReadError::Unavailable)
                };
                let _ = reply.send(result);
            }
            MemoryCommand::Briefing {
                scope,
                limit,
                reply,
            } => {
                let result = if is_ready(&inner) {
                    backend.briefing_checked(scope, limit)
                } else {
                    Err(MemoryReadError::Unavailable)
                };
                let _ = reply.send(result);
            }
            MemoryCommand::Recent {
                scope,
                limit,
                reply,
            } => {
                let result = if is_ready(&inner) {
                    backend.recent_checked(scope, limit)
                } else {
                    Err(MemoryReadError::Unavailable)
                };
                let _ = reply.send(result);
            }
            MemoryCommand::History {
                scope,
                limit,
                reply,
            } => {
                let result = if is_ready(&inner) {
                    backend.history_checked(scope, limit)
                } else {
                    Err(MemoryReadError::Unavailable)
                };
                let _ = reply.send(result);
            }
            MemoryCommand::Snapshot {
                scope,
                limit,
                reply,
            } => {
                let result = if is_ready(&inner) {
                    MemoryService::snapshot(&backend, scope, limit)
                        .map_err(|_| MemoryReadError::StorageFailure)
                } else {
                    Err(MemoryReadError::Unavailable)
                };
                let _ = reply.send(result);
            }
            MemoryCommand::Get { scope, id, reply } => {
                let result = if is_ready(&inner) {
                    backend.get_checked(scope, &id)
                } else {
                    Err(MemoryReadError::Unavailable)
                };
                let _ = reply.send(result);
            }
            MemoryCommand::Remember {
                scope,
                content,
                reply,
            } => {
                let result = if is_ready(&inner) {
                    MemoryService::remember(&backend, scope, &content)
                } else {
                    Err(MemoryWriteError::Unavailable)
                };
                let _ = reply.send(result);
            }
            MemoryCommand::Shutdown { reply } => {
                set_status(&inner, MemoryStatus::Disabled);
                let _ = reply.send(Ok(()));
                break;
            }
        }
    }
    if let Some(inner) = weak.upgrade() {
        if is_ready(&inner) {
            set_error(&inner, MemoryManagerError::WorkerFailed);
            set_status(
                &inner,
                MemoryStatus::Unavailable(MemoryUnavailableReason::WorkerFailed),
            );
        }
    }
    remove_registry_entry(&weak);
}

fn is_ready(inner: &WorkerInner) -> bool {
    inner
        .status
        .lock()
        .map(|status| *status == MemoryStatus::Ready)
        .unwrap_or(false)
}

fn remove_registry_entry(weak: &Weak<WorkerInner>) {
    let Some(inner) = weak.upgrade() else {
        return;
    };
    if let Some(registry) = WORKERS.get()
        && let Ok(mut entries) = registry.lock()
        && entries
            .get(&inner.registry_key)
            .and_then(Weak::upgrade)
            .is_some_and(|current| Arc::ptr_eq(&current, &inner))
    {
        entries.remove(&inner.registry_key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, sync::Barrier, time::SystemTime};

    fn test_backend() -> (PathBuf, AxiomMemoryScopeIds) {
        let id = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        (
            std::env::temp_dir().join(format!("axiom-background-memory-{id}")),
            AxiomMemoryScopeIds::new("user", "workspace", "project"),
        )
    }

    #[test]
    fn worker_persists_and_recovers_through_proxy() {
        let (path, scopes) = test_backend();
        let service = BackgroundMemoryService::start_ready(
            path.clone(),
            scopes.clone(),
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(service.status(), MemoryStatus::Ready);
        service
            .remember(MemoryScope::Project, "worker durable fact")
            .unwrap();
        assert_eq!(
            service
                .query_checked(MemoryScope::Project, "durable fact", 10)
                .unwrap()
                .len(),
            1
        );
        service.shutdown().unwrap();
        drop(service);

        for _ in 0..100 {
            let registry_empty = WORKERS
                .get()
                .and_then(|registry| registry.lock().ok())
                .is_none_or(|registry| registry.is_empty());
            if registry_empty {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let service =
            BackgroundMemoryService::start_ready(path.clone(), scopes, Duration::from_secs(2))
                .unwrap();
        assert_eq!(
            service
                .query_checked(MemoryScope::Project, "durable fact", 10)
                .unwrap()
                .len(),
            1
        );
        service.shutdown().unwrap();
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn concurrent_starts_share_one_worker() {
        let (path, scopes) = test_backend();
        let barrier = Arc::new(Barrier::new(2));
        let first_path = path.clone();
        let first_scopes = scopes.clone();
        let first_barrier = barrier.clone();
        let first = std::thread::spawn(move || {
            first_barrier.wait();
            BackgroundMemoryService::start(first_path, first_scopes).unwrap()
        });
        let second_path = path.clone();
        let second_scopes = scopes.clone();
        let second_barrier = barrier;
        let second = std::thread::spawn(move || {
            second_barrier.wait();
            BackgroundMemoryService::start(second_path, second_scopes).unwrap()
        });
        let first = first.join().unwrap();
        let second = second.join().unwrap();
        assert!(Arc::ptr_eq(&first.inner, &second.inner));
        let ready =
            BackgroundMemoryService::start_ready(path.clone(), scopes, Duration::from_secs(2))
                .unwrap();
        assert_eq!(ready.status(), MemoryStatus::Ready);
        ready.shutdown().unwrap();
        drop(first);
        drop(second);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn request_shutdown_is_cooperative_and_releases_worker() {
        let (path, scopes) = test_backend();
        let mut service = BackgroundMemoryService::start_ready(
            path.clone(),
            scopes,
            Duration::from_secs(2),
        )
        .unwrap();

        MemoryService::request_shutdown(&mut service);
        assert!(matches!(
            service.status(),
            MemoryStatus::Stopping | MemoryStatus::Disabled | MemoryStatus::Unavailable(_)
        ));

        for _ in 0..100 {
            if matches!(service.status(), MemoryStatus::Disabled | MemoryStatus::Unavailable(_)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(matches!(
            service.status(),
            MemoryStatus::Disabled | MemoryStatus::Unavailable(_)
        ));
        drop(service);
        let _ = fs::remove_dir_all(path);
    }
}
