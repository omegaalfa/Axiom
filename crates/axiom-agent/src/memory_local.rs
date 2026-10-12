//! Axiom-owned durable memory backend.
//!
//! The JSON files in this module are the canonical representation. The
//! backend has no provider, MCP, HTTP, SQLite, or GPUI dependency. Callers
//! must use it from a background worker when persistence is involved.

use crate::{
    MemoryCategory, MemoryHandoff, MemoryObservation, MemoryReadError, MemoryResult, MemoryScope,
    MemoryMutationError, MemoryService, MemorySnapshot, MemorySnapshotError, MemoryWriteError,
};
use serde::{Deserialize, Serialize};
use rusqlite::{Connection, OptionalExtension, params};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

pub const LOCAL_MEMORY_SCHEMA_VERSION: u32 = 1;
pub const MAX_LOCAL_MEMORY_RESULTS: usize = 100;
pub const MAX_RECORD_CONTENT_CHARS: usize = 4096;
/// Maximum episodic observations retained per scope, including incomplete sessions.
const MAX_EPISODIC_RECORDS: usize = 64;
/// Maximum completed, unprotected durable records retained per scope.
const MAX_DURABLE_RECORDS: usize = 128;
/// Maximum durable decisions and gotchas admitted per scope; existing protected records are never pruned.
const MAX_PROTECTED_RECORDS: usize = 64;
/// Maximum sessions accepted concurrently by one backend.
const MAX_ACTIVE_SESSIONS: usize = 8;
/// Maximum observations admitted for any one session.
const MAX_OBSERVATIONS_PER_SESSION: usize = 16;
const MAX_MEMORY_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MEMORY_INDEX_SCHEMA_VERSION: &str = "1";
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AxiomMemoryScopeIds {
    pub user: String,
    pub workspace: String,
    pub project: String,
}

impl AxiomMemoryScopeIds {
    pub fn new(
        user: impl Into<String>,
        workspace: impl Into<String>,
        project: impl Into<String>,
    ) -> Self {
        Self {
            user: user.into(),
            workspace: workspace.into(),
            project: project.into(),
        }
    }

    fn id(&self, scope: MemoryScope) -> &str {
        match scope {
            MemoryScope::User => &self.user,
            MemoryScope::Workspace => &self.workspace,
            MemoryScope::Project => &self.project,
        }
    }
}

#[derive(Debug)]
pub enum AxiomMemoryError {
    Io(io::Error),
    Serialization(serde_json::Error),
    CorruptStore,
    SensitiveContent,
    InvalidScopeId,
    ProtectedRecordLimit,
}

impl std::fmt::Display for AxiomMemoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(_) => f.write_str("memory storage I/O failed"),
            Self::Serialization(_) => f.write_str("memory storage serialization failed"),
            Self::CorruptStore => f.write_str("memory storage is corrupt"),
            Self::SensitiveContent => f.write_str("memory content rejected as sensitive"),
            Self::InvalidScopeId => f.write_str("memory scope identifier is invalid"),
            Self::ProtectedRecordLimit => f.write_str("protected memory record limit reached"),
        }
    }
}

impl std::error::Error for AxiomMemoryError {}

impl From<io::Error> for AxiomMemoryError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for AxiomMemoryError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

fn memory_read_error(error: AxiomMemoryError) -> MemoryReadError {
    match error {
        AxiomMemoryError::CorruptStore | AxiomMemoryError::Serialization(_) => {
            MemoryReadError::CorruptStore
        }
        AxiomMemoryError::Io(_)
        | AxiomMemoryError::SensitiveContent
        | AxiomMemoryError::InvalidScopeId
        | AxiomMemoryError::ProtectedRecordLimit => MemoryReadError::StorageFailure,
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
enum LocalRecordKind {
    Remember,
    Decision,
    Gotcha,
    Handoff,
    Mutation,
    ValidationOutcome,
    Outcome,
    SessionOutcome,
    FailedApproach,
    OpenQuestion,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct LocalRecord {
    id: u64,
    kind: LocalRecordKind,
    content: String,
    #[serde(default)]
    session_id: Option<u64>,
    #[serde(default)]
    session_completed: bool,
    #[serde(default = "one_occurrence")]
    occurrence_count: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct LocalMemoryFile {
    schema_version: u32,
    scope: String,
    scope_id: String,
    #[serde(default)]
    next_record_id: u64,
    records: Vec<LocalRecord>,
}

const fn one_occurrence() -> u32 {
    1
}

#[derive(Clone, Copy)]
struct SessionState {
    scope: MemoryScope,
    completed: bool,
    observations: usize,
}

pub struct AxiomMemoryBackend {
    base_directory: PathBuf,
    scope_ids: AxiomMemoryScopeIds,
    next_session_id: AtomicU64,
    sessions: Mutex<HashMap<crate::MemorySessionId, SessionState>>,
    #[cfg(test)]
    fail_write_after: AtomicUsize,
}

impl AxiomMemoryBackend {
    pub fn new(base_directory: impl Into<PathBuf>, scope_ids: AxiomMemoryScopeIds) -> Self {
        let session_seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        Self {
            base_directory: base_directory.into(),
            scope_ids,
            next_session_id: AtomicU64::new(session_seed),
            sessions: Mutex::new(HashMap::new()),
            #[cfg(test)]
            fail_write_after: AtomicUsize::new(usize::MAX),
        }
    }

    pub fn base_directory(&self) -> &Path {
        &self.base_directory
    }

    pub fn initialize_storage(&self) -> Result<(), AxiomMemoryError> {
        fs::create_dir_all(&self.base_directory)?;
        for scope in [
            MemoryScope::User,
            MemoryScope::Workspace,
            MemoryScope::Project,
        ] {
            let path = self.file_path(scope)?;
            let parent = path.parent().ok_or(AxiomMemoryError::InvalidScopeId)?;
            fs::create_dir_all(parent)?;
            let _ = self.read_file(scope)?;
            let probe = parent.join(format!(
                ".memory-init-{}-{}.tmp",
                std::process::id(),
                TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            if let Err(error) = fs::write(&probe, b"axiom memory ready") {
                let _ = fs::remove_file(&probe);
                return Err(error.into());
            }
            fs::remove_file(probe)?;
        }
        let _ = self.rebuild_index();
        Ok(())
    }

    /// Rebuilds the derived FTS index from the canonical JSON scope files.
    /// Canonical reads complete before the index is replaced, and a SQLite
    /// transaction prevents a partial index from being visible to readers.
    pub fn rebuild_index(&self) -> Result<(), AxiomMemoryError> {
        let snapshots = self.canonical_snapshots()?;
        let path = self.index_path()?;
        if path.exists() {
            let _ = fs::remove_file(&path);
        }
        self.sync_index(&snapshots, true)
    }

    #[cfg(test)]
    fn fail_after_successful_writes(&self, count: usize) {
        self.fail_write_after.store(count, Ordering::SeqCst);
    }

    #[cfg(test)]
    fn clear_write_failure(&self) {
        self.fail_write_after.store(usize::MAX, Ordering::SeqCst);
    }

    pub fn remember(&self, scope: MemoryScope, content: &str) -> Result<(), AxiomMemoryError> {
        self.append(scope, LocalRecordKind::Remember, content)
    }

    pub fn delete_memory_entry(&self, id: &str) -> Result<bool, AxiomMemoryError> {
        let Some((scope_id, _)) = id.split_once(':') else {
            return Ok(false);
        };
        let mut matching_scopes = [
            MemoryScope::User,
            MemoryScope::Workspace,
            MemoryScope::Project,
        ]
        .into_iter()
        .filter(|scope| self.scope_ids.id(*scope) == scope_id);
        let Some(scope) = matching_scopes.next() else {
            return Ok(false);
        };
        if matching_scopes.next().is_some() {
            return Ok(false);
        }
        self.delete_memory_entry_in_scope(scope, id)
    }

    pub fn delete_memory_entry_in_scope(
        &self,
        scope: MemoryScope,
        id: &str,
    ) -> Result<bool, AxiomMemoryError> {
        self.retry_completed_sessions();
        let Some((scope_id, record_id)) = id.split_once(':') else {
            return Ok(false);
        };
        if scope_id != self.scope_ids.id(scope) {
            return Ok(false);
        }
        let Ok(record_id) = record_id.parse::<u64>() else {
            return Ok(false);
        };
        let Some(mut file) = self.read_file(scope)? else {
            return Ok(false);
        };
        let original_len = file.records.len();
        file.records.retain(|record| record.id != record_id);
        if file.records.len() == original_len {
            return Ok(false);
        }
        self.write_file(scope, &file)?;
        self.refresh_index(scope, &file)?;
        Ok(true)
    }

    pub fn clear_memory(&self, scope: MemoryScope) -> Result<usize, AxiomMemoryError> {
        self.retry_completed_sessions();
        let Some(mut file) = self.read_file(scope)? else {
            return Ok(0);
        };
        let removed = file.records.len();
        if removed == 0 {
            return Ok(0);
        }
        file.records.clear();
        self.write_file(scope, &file)?;
        self.refresh_index(scope, &file)?;
        self.vacuum_index()?;
        Ok(removed)
    }

    pub fn clear_all_workspace_memory(&self) -> Result<usize, AxiomMemoryError> {
        self.clear_memory(MemoryScope::Workspace)
    }

    pub fn decision(&self, scope: MemoryScope, content: &str) -> Result<(), AxiomMemoryError> {
        self.append(scope, LocalRecordKind::Decision, content)
    }

    pub fn gotcha(&self, scope: MemoryScope, content: &str) -> Result<(), AxiomMemoryError> {
        self.append(scope, LocalRecordKind::Gotcha, content)
    }

    pub fn persist_handoff(&self, handoff: &MemoryHandoff) -> Result<(), AxiomMemoryError> {
        let content = handoff
            .entries
            .iter()
            .map(|entry| entry.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        self.append(handoff.scope, LocalRecordKind::Handoff, &content)
    }

    pub fn query(&self, scope: MemoryScope, query: &str, limit: usize) -> Vec<MemoryResult> {
        self.query_checked(scope, query, limit).unwrap_or_default()
    }

    pub fn briefing(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        self.retry_completed_sessions();
        deduplicate_records(self.read_records(scope).unwrap_or_default())
            .into_iter()
            .filter(|record| record.kind != LocalRecordKind::SessionOutcome)
            .take(limit.min(MAX_LOCAL_MEMORY_RESULTS))
            .map(|record| self.result(scope, record))
            .collect()
    }

    pub fn recent(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        self.retry_completed_sessions();
        let mut records = deduplicate_records(self.read_records(scope).unwrap_or_default());
        records.reverse();
        records
            .into_iter()
            .take(limit.min(MAX_LOCAL_MEMORY_RESULTS))
            .map(|record| self.result(scope, record))
            .collect()
    }

    pub fn history(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        self.retry_completed_sessions();
        self.results(scope, limit)
    }

    fn results(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        deduplicate_records(self.read_records(scope).unwrap_or_default())
            .into_iter()
            .take(limit.min(MAX_LOCAL_MEMORY_RESULTS))
            .map(|record| self.result(scope, record))
            .collect()
    }

    fn append(
        &self,
        scope: MemoryScope,
        kind: LocalRecordKind,
        content: &str,
    ) -> Result<(), AxiomMemoryError> {
        self.retry_completed_sessions();
        let content = sanitize_content(content)?;
        let mut file = self.read_file(scope)?.unwrap_or_else(|| LocalMemoryFile {
            schema_version: LOCAL_MEMORY_SCHEMA_VERSION,
            scope: scope_name(scope).to_owned(),
            scope_id: self.scope_ids.id(scope).to_owned(),
            next_record_id: 1,
            records: Vec::new(),
        });
        if let Some(record) = file.records.iter_mut().find(|record| {
            record.session_id.is_none()
                && record.kind == kind
                && same_content(&record.content, &content)
        }) {
            record.occurrence_count = record.occurrence_count.saturating_add(1);
            return self.write_file(scope, &file);
        }
        if is_protected(kind)
            && file
                .records
                .iter()
                .filter(|record| is_protected(record.kind))
                .count()
                >= MAX_PROTECTED_RECORDS
        {
            return Err(AxiomMemoryError::ProtectedRecordLimit);
        }
        let id = allocate_record_id(&mut file);
        file.records.push(LocalRecord {
            id,
            kind,
            content,
            session_id: None,
            session_completed: false,
            occurrence_count: 1,
        });
        trim_durable_records(&mut file.records);
        self.write_file(scope, &file)
    }

    fn append_observation(
        &self,
        scope: MemoryScope,
        session: crate::MemorySessionId,
        kind: LocalRecordKind,
        content: &str,
    ) -> Result<(), AxiomMemoryError> {
        let content = sanitize_content(content)?;
        let mut file = self.read_file(scope)?.unwrap_or_else(|| LocalMemoryFile {
            schema_version: LOCAL_MEMORY_SCHEMA_VERSION,
            scope: scope_name(scope).to_owned(),
            scope_id: self.scope_ids.id(scope).to_owned(),
            next_record_id: 1,
            records: Vec::new(),
        });
        if let Some(record) = file.records.iter_mut().find(|record| {
            record.session_id == Some(session.value())
                && record.kind == kind
                && same_content(&record.content, &content)
        }) {
            record.occurrence_count = record.occurrence_count.saturating_add(1);
        } else {
            let id = allocate_record_id(&mut file);
            file.records.push(LocalRecord {
                id,
                kind,
                content,
                session_id: Some(session.value()),
                session_completed: false,
                occurrence_count: 1,
            });
        }
        trim_episodic_records(&mut file.records);
        self.write_file(scope, &file)
    }

    fn retry_completed_sessions(&self) {
        let pending = self
            .sessions
            .lock()
            .ok()
            .map(|sessions| {
                sessions
                    .iter()
                    .filter(|(_, state)| state.completed)
                    .map(|(id, state)| (*id, state.scope))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (id, scope) in pending {
            let _ = self.complete_and_consolidate(scope, id);
        }
        for scope in [
            MemoryScope::User,
            MemoryScope::Workspace,
            MemoryScope::Project,
        ] {
            let completed = self
                .read_records(scope)
                .unwrap_or_default()
                .into_iter()
                .filter(|record| record.session_completed)
                .filter_map(|record| record.session_id)
                .collect::<HashSet<_>>();
            for session_id in completed {
                let _ = self.consolidate_completed_session(scope, session_id);
            }
        }
    }

    fn complete_and_consolidate(
        &self,
        scope: MemoryScope,
        session: crate::MemorySessionId,
    ) -> Result<(), AxiomMemoryError> {
        let Some(mut file) = self.read_file(scope)? else {
            self.remove_session(session);
            return Ok(());
        };
        if !file
            .records
            .iter()
            .any(|record| record.session_id == Some(session.value()))
        {
            self.remove_session(session);
            return Ok(());
        }
        let mut changed = false;
        for record in &mut file.records {
            if record.session_id == Some(session.value()) && !record.session_completed {
                record.session_completed = true;
                changed = true;
            }
        }
        if changed {
            self.write_file(scope, &file)?;
        }
        self.consolidate_completed_session(scope, session.value())
    }

    fn consolidate_completed_session(
        &self,
        scope: MemoryScope,
        session_id: u64,
    ) -> Result<(), AxiomMemoryError> {
        let Some(mut file) = self.read_file(scope)? else {
            self.remove_session(crate::MemorySessionId::new(session_id));
            return Ok(());
        };
        if !file
            .records
            .iter()
            .any(|record| record.session_id == Some(session_id))
        {
            self.remove_session(crate::MemorySessionId::new(session_id));
            return Ok(());
        }
        if file
            .records
            .iter()
            .any(|record| record.session_id == Some(session_id) && !record.session_completed)
        {
            return Ok(());
        }
        let mut records = Vec::with_capacity(file.records.len());
        for mut record in file.records.drain(..) {
            if record.session_id != Some(session_id) {
                records.push(record);
                continue;
            }
            if record.kind == LocalRecordKind::SessionOutcome {
                records.push(record);
                continue;
            }
            let Some(kind) = durable_kind(record.kind) else {
                continue;
            };
            record.kind = kind;
            record.session_id = None;
            record.session_completed = false;
            if let Some(existing) = records.iter_mut().find(|candidate: &&mut LocalRecord| {
                candidate.session_id.is_none()
                    && candidate.kind == kind
                    && same_content(&candidate.content, &record.content)
            }) {
                existing.occurrence_count = existing
                    .occurrence_count
                    .saturating_add(record.occurrence_count);
            } else {
                if is_protected(kind)
                    && records
                        .iter()
                        .filter(|candidate| is_protected(candidate.kind))
                        .count()
                        >= MAX_PROTECTED_RECORDS
                {
                    return Err(AxiomMemoryError::ProtectedRecordLimit);
                }
                records.push(record);
            }
        }
        file.records = records;
        trim_episodic_records(&mut file.records);
        trim_durable_records(&mut file.records);
        self.write_file(scope, &file)?;
        self.remove_session(crate::MemorySessionId::new(session_id));
        Ok(())
    }

    fn remove_session(&self, session: crate::MemorySessionId) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(&session);
        }
    }

    fn result(&self, scope: MemoryScope, record: LocalRecord) -> MemoryResult {
        MemoryResult {
            id: Some(format!("{}:{}", self.scope_ids.id(scope), record.id)),
            scope,
            category: Some(match record.kind {
                LocalRecordKind::Remember => MemoryCategory::Remember,
                LocalRecordKind::Decision => MemoryCategory::Decision,
                LocalRecordKind::Gotcha => MemoryCategory::Gotcha,
                LocalRecordKind::Handoff => MemoryCategory::Handoff,
                LocalRecordKind::Mutation => MemoryCategory::Mutation,
                LocalRecordKind::ValidationOutcome => MemoryCategory::Validation,
                LocalRecordKind::Outcome => MemoryCategory::Outcome,
                LocalRecordKind::SessionOutcome => MemoryCategory::SessionOutcome,
                LocalRecordKind::FailedApproach => MemoryCategory::FailedApproach,
                LocalRecordKind::OpenQuestion => MemoryCategory::OpenQuestion,
            }),
            content: record.content,
        }
    }

    fn read_records(&self, scope: MemoryScope) -> Result<Vec<LocalRecord>, AxiomMemoryError> {
        Ok(self
            .read_file(scope)?
            .map_or_else(Vec::new, |file| file.records))
    }

    fn read_file(&self, scope: MemoryScope) -> Result<Option<LocalMemoryFile>, AxiomMemoryError> {
        let path = self.file_path(scope)?;
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if metadata.len() > MAX_MEMORY_FILE_BYTES {
            return Err(AxiomMemoryError::CorruptStore);
        }
        let text = fs::read_to_string(path)?;
        let file: LocalMemoryFile =
            serde_json::from_str(&text).map_err(|_| AxiomMemoryError::CorruptStore)?;
        if file.schema_version != LOCAL_MEMORY_SCHEMA_VERSION
            || file.scope != scope_name(scope)
            || file.scope_id != self.scope_ids.id(scope)
        {
            return Err(AxiomMemoryError::CorruptStore);
        }
        Ok(Some(file))
    }

    fn write_file(
        &self,
        scope: MemoryScope,
        file: &LocalMemoryFile,
    ) -> Result<(), AxiomMemoryError> {
        let path = self.file_path(scope)?;
        let parent = path.parent().ok_or(AxiomMemoryError::InvalidScopeId)?;
        fs::create_dir_all(parent)?;
        let json = serde_json::to_string_pretty(file)?;
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp = parent.join(format!(
            ".{}.{}.tmp",
            path.file_name().unwrap().to_string_lossy(),
            sequence
        ));
        if let Err(error) = fs::write(&temp, json) {
            let _ = fs::remove_file(&temp);
            return Err(error.into());
        }
        #[cfg(test)]
        {
            let remaining = self.fail_write_after.load(Ordering::SeqCst);
            if remaining == 0 {
                let _ = fs::remove_file(&temp);
                return Err(io::Error::other("injected memory commit failure").into());
            }
            if remaining != usize::MAX {
                self.fail_write_after.fetch_sub(1, Ordering::SeqCst);
            }
        }
        if let Err(error) = fs::rename(&temp, &path) {
            let _ = fs::remove_file(&temp);
            return Err(error.into());
        }
        // JSON is canonical. A stale or unavailable derived index is safe;
        // subsequent reads fall back to the canonical records.
        let _ = self.refresh_index(scope, file);
        Ok(())
    }

    fn refresh_index(
        &self,
        scope: MemoryScope,
        file: &LocalMemoryFile,
    ) -> Result<(), AxiomMemoryError> {
        let mut snapshots = self.canonical_snapshots()?;
        snapshots[scope_index(scope)] = Some(file.clone());
        self.sync_index_with_recovery(&snapshots)
    }

    fn sync_index_with_recovery(
        &self,
        snapshots: &[Option<LocalMemoryFile>; 3],
    ) -> Result<(), AxiomMemoryError> {
        match self.sync_index(snapshots, false) {
            Ok(()) => Ok(()),
            Err(first_error) if is_rebuildable_index_error(&first_error) => {
                let path = self.index_path()?;
                let _ = fs::remove_file(path);
                self.sync_index(snapshots, true).map_err(|_| first_error)
            }
            Err(error) => Err(error),
        }
    }

    fn canonical_snapshots(&self) -> Result<[Option<LocalMemoryFile>; 3], AxiomMemoryError> {
        Ok([
            self.read_file(MemoryScope::User)?,
            self.read_file(MemoryScope::Workspace)?,
            self.read_file(MemoryScope::Project)?,
        ])
    }

    fn sync_index(
        &self,
        snapshots: &[Option<LocalMemoryFile>; 3],
        force: bool,
    ) -> Result<(), AxiomMemoryError> {
        let path = self.index_path()?;
        let mut connection = match Connection::open(&path) {
            Ok(connection) => connection,
            Err(error) => {
                if !force {
                    let _ = fs::remove_file(&path);
                }
                return Err(AxiomMemoryError::Io(io::Error::other(error.to_string())));
            }
        };
        connection
            .busy_timeout(std::time::Duration::from_millis(250))
            .map_err(sqlite_error)?;
        let transaction = connection.transaction().map_err(sqlite_error)?;
        transaction
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS memory_index_meta (
                    key TEXT PRIMARY KEY NOT NULL,
                    value TEXT NOT NULL
                 );
                 CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
                    record_key UNINDEXED,
                    scope UNINDEXED,
                    scope_id UNINDEXED,
                    record_id UNINDEXED,
                    kind UNINDEXED,
                    content,
                    tokenize = 'unicode61'
                 );",
            )
            .map_err(sqlite_error)?;
        let stored_version = transaction
            .query_row(
                "SELECT value FROM memory_index_meta WHERE key = 'schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(sqlite_error)?;
        if stored_version.as_deref() != Some(MEMORY_INDEX_SCHEMA_VERSION) {
            transaction
                .execute("DELETE FROM memory_fts", [])
                .map_err(sqlite_error)?;
            transaction
                .execute("DELETE FROM memory_index_meta", [])
                .map_err(sqlite_error)?;
            transaction
                .execute(
                    "INSERT INTO memory_index_meta(key, value) VALUES ('schema_version', ?1)",
                    params![MEMORY_INDEX_SCHEMA_VERSION],
                )
                .map_err(sqlite_error)?;
        }
        for (index, scope) in [
            MemoryScope::User,
            MemoryScope::Workspace,
            MemoryScope::Project,
        ]
        .into_iter()
        .enumerate()
        {
            let scope_id = self.scope_ids.id(scope);
            let fingerprint = snapshots[index]
                .as_ref()
                .map(canonical_fingerprint)
                .transpose()?
                .unwrap_or_else(|| "missing".to_owned());
            let key = fingerprint_key(scope);
            let previous = transaction
                .query_row(
                    "SELECT value FROM memory_index_meta WHERE key = ?1",
                    params![key],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(sqlite_error)?;
            if !force && previous.as_deref() == Some(fingerprint.as_str()) {
                continue;
            }
            let scope_name = scope_name(scope);
            transaction
                .execute("DELETE FROM memory_fts WHERE scope = ?1 AND scope_id = ?2", params![scope_name, scope_id])
                .map_err(sqlite_error)?;
            let Some(file) = snapshots[index].as_ref() else {
                transaction
                    .execute(
                        "INSERT INTO memory_index_meta(key, value) VALUES (?1, ?2)
                         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                        params![key, fingerprint],
                    )
                    .map_err(sqlite_error)?;
                continue;
            };
            for record in deduplicate_records(file.records.clone()) {
                transaction
                    .execute(
                        "INSERT INTO memory_fts(record_key, scope, scope_id, record_id, kind, content)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![
                            format!("{}:{}", file.scope_id, record.id),
                            scope_name,
                            file.scope_id,
                            record.id,
                            local_record_kind_name(record.kind),
                            record.content,
                        ],
                    )
                    .map_err(sqlite_error)?;
            }
            transaction
                .execute(
                    "INSERT INTO memory_index_meta(key, value) VALUES (?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![key, fingerprint],
                )
                .map_err(sqlite_error)?;
        }
        transaction.commit().map_err(sqlite_error)?;
        Ok(())
    }

    fn vacuum_index(&self) -> Result<(), AxiomMemoryError> {
        let connection = Connection::open(self.index_path()?).map_err(sqlite_error)?;
        connection
            .busy_timeout(std::time::Duration::from_millis(250))
            .map_err(sqlite_error)?;
        connection
            .execute("INSERT INTO memory_fts(memory_fts) VALUES('optimize')", [])
            .map_err(sqlite_error)?;
        connection.execute_batch("VACUUM;").map_err(sqlite_error)
    }

    fn fts_records(
        &self,
        scope: MemoryScope,
        query: &str,
        limit: usize,
        records: &[LocalRecord],
    ) -> Result<Vec<LocalRecord>, AxiomMemoryError> {
        let snapshots = self.canonical_snapshots()?;
        if self.sync_index_with_recovery(&snapshots).is_err() {
            return Err(AxiomMemoryError::Io(io::Error::other("memory FTS unavailable")));
        }
        let connection = Connection::open(self.index_path()?).map_err(sqlite_error)?;
        let mut statement = connection
            .prepare(
                "SELECT record_key FROM memory_fts
                 WHERE memory_fts MATCH ?1 AND scope = ?2 AND scope_id = ?3
                   AND kind <> 'session_outcome'
                 ORDER BY bm25(memory_fts), record_key
                 LIMIT ?4",
            )
            .map_err(sqlite_error)?;
        let match_query = fts_match_query(query);
        let keys = statement
            .query_map(
                params![
                    match_query,
                    scope_name(scope),
                    self.scope_ids.id(scope),
                    limit.min(MAX_LOCAL_MEMORY_RESULTS) as i64,
                ],
                |row| row.get::<_, String>(0),
            )
            .map_err(sqlite_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sqlite_error)?;
        let by_key = records
            .iter()
            .cloned()
            .map(|record| (format!("{}:{}", self.scope_ids.id(scope), record.id), record))
            .collect::<HashMap<_, _>>();
        Ok(keys
            .into_iter()
            .filter_map(|key| by_key.get(&key).cloned())
            .collect())
    }

    fn file_path(&self, scope: MemoryScope) -> Result<PathBuf, AxiomMemoryError> {
        let id = self.scope_ids.id(scope);
        if id.is_empty() || id.contains(['/', '\\']) || id == "." || id == ".." {
            return Err(AxiomMemoryError::InvalidScopeId);
        }
        Ok(self
            .base_directory
            .join(scope_name(scope))
            .join(format!("{id}.json")))
    }

    fn index_path(&self) -> Result<PathBuf, AxiomMemoryError> {
        fs::create_dir_all(&self.base_directory)?;
        Ok(self.base_directory.join("memory_index.sqlite"))
    }
}

impl AxiomMemoryBackend {
    pub(crate) fn observe_checked(
        &mut self,
        session: crate::MemorySessionId,
        observation: MemoryObservation,
    ) -> Result<(), AxiomMemoryError> {
        let (kind, content) = match observation {
            MemoryObservation::Discovery(content) => (LocalRecordKind::Remember, content),
            MemoryObservation::Decision(content) => (LocalRecordKind::Decision, content),
            MemoryObservation::Gotcha(content) => (LocalRecordKind::Gotcha, content),
            MemoryObservation::MutationSummary(content) => (LocalRecordKind::Mutation, content),
            MemoryObservation::Validation(content) => (LocalRecordKind::ValidationOutcome, content),
            MemoryObservation::TaskOutcome(content) => (LocalRecordKind::Outcome, content),
            MemoryObservation::SessionOutcome(content) => {
                (LocalRecordKind::SessionOutcome, content)
            }
            MemoryObservation::FailedApproach(content) => {
                (LocalRecordKind::FailedApproach, content)
            }
            MemoryObservation::OpenQuestion(content) => (LocalRecordKind::OpenQuestion, content),
        };
        let Some(scope) = self.sessions.lock().ok().and_then(|mut sessions| {
            let state = sessions.get_mut(&session)?;
            if state.completed || state.observations >= MAX_OBSERVATIONS_PER_SESSION {
                return None;
            }
            state.observations += 1;
            Some(state.scope)
        }) else {
            return Ok(());
        };
        self.append_observation(scope, session, kind, &content)
    }

    pub(crate) fn session_end_checked(
        &mut self,
        session: crate::MemorySessionId,
    ) -> Result<(), AxiomMemoryError> {
        let scope = self.sessions.lock().ok().and_then(|mut sessions| {
            let state = sessions.get_mut(&session)?;
            state.completed = true;
            Some(state.scope)
        });
        if let Some(scope) = scope {
            self.complete_and_consolidate(scope, session)?;
        }
        Ok(())
    }
}

impl MemoryService for AxiomMemoryBackend {
    fn can_remember(&self) -> bool {
        true
    }

    fn session_start(&mut self, scope: MemoryScope) -> crate::MemorySessionId {
        self.retry_completed_sessions();
        let id = crate::MemorySessionId::new(
            self.next_session_id
                .fetch_add(1, Ordering::Relaxed)
                .saturating_add(1),
        );
        if let Ok(mut sessions) = self.sessions.lock() {
            if sessions.len() < MAX_ACTIVE_SESSIONS {
                sessions.insert(
                    id,
                    SessionState {
                        scope,
                        completed: false,
                        observations: 0,
                    },
                );
            }
        }
        id
    }

    fn observe(&mut self, session: crate::MemorySessionId, observation: MemoryObservation) {
        let _ = self.observe_checked(session, observation);
    }

    fn session_end(&mut self, session: crate::MemorySessionId) {
        let _ = self.session_end_checked(session);
    }

    fn query(&self, scope: MemoryScope, query: &str, limit: usize) -> Vec<MemoryResult> {
        Self::query(self, scope, query, limit)
    }

    fn query_checked(
        &self,
        scope: MemoryScope,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MemoryResult>, MemoryReadError> {
        self.retry_completed_sessions();
        let query = normalize(query);
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let records = deduplicate_records(self.read_records(scope).map_err(memory_read_error)?);
        let matches = self
            .fts_records(scope, &query, limit, &records)
            .unwrap_or_else(|_| {
                let query = query.to_lowercase();
                records
                    .iter()
                    .filter(|record| record.kind != LocalRecordKind::SessionOutcome)
                    .filter(|record| normalize(&record.content).to_lowercase().contains(&query))
                    .take(limit.min(MAX_LOCAL_MEMORY_RESULTS))
                    .cloned()
                    .collect()
            });
        Ok(matches
            .into_iter()
            .map(|record| self.result(scope, record))
            .collect())
    }

    fn briefing(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        Self::briefing(self, scope, limit)
    }

    fn briefing_checked(
        &self,
        scope: MemoryScope,
        limit: usize,
    ) -> Result<Vec<MemoryResult>, MemoryReadError> {
        self.retry_completed_sessions();
        Ok(
            deduplicate_records(self.read_records(scope).map_err(memory_read_error)?)
                .into_iter()
                .filter(|record| record.kind != LocalRecordKind::SessionOutcome)
                .take(limit.min(MAX_LOCAL_MEMORY_RESULTS))
                .map(|record| self.result(scope, record))
                .collect(),
        )
    }

    fn recent_checked(
        &self,
        scope: MemoryScope,
        limit: usize,
    ) -> Result<Vec<MemoryResult>, MemoryReadError> {
        self.retry_completed_sessions();
        let mut records = deduplicate_records(self.read_records(scope).map_err(memory_read_error)?);
        records.reverse();
        Ok(records
            .into_iter()
            .take(limit.min(MAX_LOCAL_MEMORY_RESULTS))
            .map(|record| self.result(scope, record))
            .collect())
    }

    fn history_checked(
        &self,
        scope: MemoryScope,
        limit: usize,
    ) -> Result<Vec<MemoryResult>, MemoryReadError> {
        self.retry_completed_sessions();
        Ok(
            deduplicate_records(self.read_records(scope).map_err(memory_read_error)?)
                .into_iter()
                .take(limit.min(MAX_LOCAL_MEMORY_RESULTS))
                .map(|record| self.result(scope, record))
                .collect(),
        )
    }

    fn recent(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        Self::recent(self, scope, limit)
    }

    fn history(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        Self::history(self, scope, limit)
    }

    fn snapshot(
        &self,
        scope: MemoryScope,
        limit: usize,
    ) -> Result<MemorySnapshot, MemorySnapshotError> {
        let records = self.read_records(scope).map_err(|_| MemorySnapshotError)?;
        let mut recent = deduplicate_records(records.clone());
        recent.reverse();
        Ok(MemorySnapshot {
            recent: recent
                .into_iter()
                .take(limit.min(MAX_LOCAL_MEMORY_RESULTS))
                .map(|record| self.result(scope, record))
                .collect(),
            history: deduplicate_records(records)
                .into_iter()
                .take(limit.min(MAX_LOCAL_MEMORY_RESULTS))
                .map(|record| self.result(scope, record))
                .collect(),
        })
    }

    fn get(&self, scope: MemoryScope, id: &str) -> Option<MemoryResult> {
        self.retry_completed_sessions();
        let (scope_id, record_id) = id.split_once(':')?;
        if scope_id != self.scope_ids.id(scope) {
            return None;
        }
        let record_id = record_id.parse::<u64>().ok()?;
        self.read_records(scope)
            .ok()?
            .into_iter()
            .find(|record| record.id == record_id)
            .map(|record| self.result(scope, record))
    }

    fn get_checked(
        &self,
        scope: MemoryScope,
        id: &str,
    ) -> Result<Option<MemoryResult>, MemoryReadError> {
        self.retry_completed_sessions();
        let Some((scope_id, record_id)) = id.split_once(':') else {
            return Ok(None);
        };
        if scope_id != self.scope_ids.id(scope) {
            return Ok(None);
        }
        let Ok(record_id) = record_id.parse::<u64>() else {
            return Ok(None);
        };
        Ok(self
            .read_records(scope)
            .map_err(memory_read_error)?
            .into_iter()
            .find(|record| record.id == record_id)
            .map(|record| self.result(scope, record)))
    }

    fn delete_memory_entry(
        &self,
        scope: MemoryScope,
        id: &str,
    ) -> Result<bool, MemoryMutationError> {
        AxiomMemoryBackend::delete_memory_entry_in_scope(self, scope, id)
            .map_err(|_| MemoryMutationError::StorageFailure)
    }

    fn clear_memory(&self, scope: MemoryScope) -> Result<usize, MemoryMutationError> {
        AxiomMemoryBackend::clear_memory(self, scope)
            .map_err(|_| MemoryMutationError::StorageFailure)
    }

    fn handoff(&self, _session: crate::MemorySessionId) -> Option<MemoryHandoff> {
        None
    }

    fn remember(
        &self,
        scope: MemoryScope,
        content: &str,
    ) -> Result<(), MemoryWriteError> {
        AxiomMemoryBackend::remember(self, scope, content).map_err(|error| match error {
            AxiomMemoryError::SensitiveContent | AxiomMemoryError::ProtectedRecordLimit => {
                MemoryWriteError::Rejected
            }
            AxiomMemoryError::Io(_)
            | AxiomMemoryError::Serialization(_)
            | AxiomMemoryError::CorruptStore
            | AxiomMemoryError::InvalidScopeId => MemoryWriteError::StorageFailure,
        })
    }

    fn decision(&mut self, scope: MemoryScope, content: &str) {
        let _ = AxiomMemoryBackend::decision(self, scope, content);
    }

    fn gotcha(&mut self, scope: MemoryScope, content: &str) {
        let _ = AxiomMemoryBackend::gotcha(self, scope, content);
    }

    fn persist_handoff(&mut self, handoff: &MemoryHandoff) {
        let _ = AxiomMemoryBackend::persist_handoff(self, handoff);
    }
}

fn allocate_record_id(file: &mut LocalMemoryFile) -> u64 {
    let next_existing = file
        .records
        .iter()
        .map(|record| record.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let id = file.next_record_id.max(next_existing).max(1);
    file.next_record_id = id.saturating_add(1);
    id
}

fn is_protected(kind: LocalRecordKind) -> bool {
    matches!(kind, LocalRecordKind::Decision | LocalRecordKind::Gotcha)
}

fn durable_kind(kind: LocalRecordKind) -> Option<LocalRecordKind> {
    match kind {
        LocalRecordKind::Remember
        | LocalRecordKind::Decision
        | LocalRecordKind::Gotcha
        | LocalRecordKind::ValidationOutcome
        | LocalRecordKind::Outcome
        | LocalRecordKind::FailedApproach
        | LocalRecordKind::OpenQuestion => Some(kind),
        LocalRecordKind::Mutation
        | LocalRecordKind::Handoff
        | LocalRecordKind::SessionOutcome => None,
    }
}

fn same_content(left: &str, right: &str) -> bool {
    normalize(left).to_lowercase() == normalize(right).to_lowercase()
}

fn trim_episodic_records(records: &mut Vec<LocalRecord>) {
    while records
        .iter()
        .filter(|record| record.session_id.is_some())
        .count()
        > MAX_EPISODIC_RECORDS
    {
        let Some(index) = records
            .iter()
            .position(|record| record.session_id.is_some())
        else {
            break;
        };
        records.remove(index);
    }
}

fn trim_durable_records(records: &mut Vec<LocalRecord>) {
    while records
        .iter()
        .filter(|record| record.session_id.is_none() && !is_protected(record.kind))
        .count()
        > MAX_DURABLE_RECORDS
    {
        let Some(index) = records
            .iter()
            .position(|record| record.session_id.is_none() && !is_protected(record.kind))
        else {
            break;
        };
        records.remove(index);
    }
}

fn deduplicate_records(records: Vec<LocalRecord>) -> Vec<LocalRecord> {
    let durable_keys = records
        .iter()
        .filter(|record| record.session_id.is_none())
        .map(|record| (record.kind, normalize(&record.content).to_lowercase()))
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    records
        .into_iter()
        .filter(|record| {
            let key = (record.kind, normalize(&record.content).to_lowercase());
            if record.session_id.is_some() && durable_keys.contains(&key) {
                return false;
            }
            seen.insert(key)
        })
        .collect()
}

fn scope_name(scope: MemoryScope) -> &'static str {
    match scope {
        MemoryScope::User => "user",
        MemoryScope::Workspace => "workspace",
        MemoryScope::Project => "project",
    }
}

fn scope_index(scope: MemoryScope) -> usize {
    match scope {
        MemoryScope::User => 0,
        MemoryScope::Workspace => 1,
        MemoryScope::Project => 2,
    }
}

fn fingerprint_key(scope: MemoryScope) -> String {
    format!("fingerprint:{}", scope_name(scope))
}

fn canonical_fingerprint(file: &LocalMemoryFile) -> Result<String, AxiomMemoryError> {
    let bytes = serde_json::to_vec(file)?;
    let hash = bytes.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    });
    Ok(format!("{hash:016x}"))
}

fn local_record_kind_name(kind: LocalRecordKind) -> &'static str {
    match kind {
        LocalRecordKind::Remember => "remember",
        LocalRecordKind::Decision => "decision",
        LocalRecordKind::Gotcha => "gotcha",
        LocalRecordKind::Handoff => "handoff",
        LocalRecordKind::Mutation => "mutation",
        LocalRecordKind::ValidationOutcome => "validation_outcome",
        LocalRecordKind::Outcome => "outcome",
        LocalRecordKind::SessionOutcome => "session_outcome",
        LocalRecordKind::FailedApproach => "failed_approach",
        LocalRecordKind::OpenQuestion => "open_question",
    }
}

fn fts_match_query(query: &str) -> String {
    normalize(query)
        .split_whitespace()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn sqlite_error(error: rusqlite::Error) -> AxiomMemoryError {
    AxiomMemoryError::Io(io::Error::other(error.to_string()))
}

fn is_rebuildable_index_error(error: &AxiomMemoryError) -> bool {
    match error {
        AxiomMemoryError::Io(error) => {
            let message = error.to_string().to_ascii_lowercase();
            message.contains("malformed")
                || message.contains("not a database")
                || message.contains("no such table")
                || message.contains("no such module")
        }
        _ => false,
    }
}

fn normalize(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn sanitize_content(content: &str) -> Result<String, AxiomMemoryError> {
    let content = normalize(content);
    if content.is_empty() || content.chars().count() > MAX_RECORD_CONTENT_CHARS {
        return Err(AxiomMemoryError::SensitiveContent);
    }
    let lower = content.to_ascii_lowercase();
    let sensitive = [
        "authorization:",
        "bearer ",
        "password=",
        "password:",
        "api_key=",
        "api-key:",
        "access_token=",
        "access-token:",
        "-----begin private key-----",
    ];
    if sensitive.iter().any(|pattern| lower.contains(pattern)) {
        return Err(AxiomMemoryError::SensitiveContent);
    }
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn backend() -> (AxiomMemoryBackend, PathBuf) {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("axiom-memory-{id}"));
        let backend = AxiomMemoryBackend::new(
            &path,
            AxiomMemoryScopeIds::new("user", "workspace", "project"),
        );
        (backend, path)
    }

    #[test]
    fn explicit_records_survive_backend_recreation() {
        let (backend, path) = backend();
        backend.remember(MemoryScope::Project, "uses Pest").unwrap();
        backend
            .decision(MemoryScope::Project, "run PHPStan")
            .unwrap();
        backend.gotcha(MemoryScope::Project, "stale cache").unwrap();
        let recreated = AxiomMemoryBackend::new(
            &path,
            AxiomMemoryScopeIds::new("user", "workspace", "project"),
        );
        assert_eq!(recreated.history(MemoryScope::Project, 10).len(), 3);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn memory_service_write_propagates_rejection_and_storage_failure() {
        use crate::MemoryWriteError;

        let (backend, path) = backend();
        assert_eq!(
            MemoryService::remember(
                &backend,
                MemoryScope::Workspace,
                "Authorization: Bearer secret"
            ),
            Err(MemoryWriteError::Rejected)
        );

        backend.fail_after_successful_writes(0);
        assert_eq!(
            MemoryService::remember(
                &backend,
                MemoryScope::Workspace,
                "valid content but failed commit"
            ),
            Err(MemoryWriteError::StorageFailure)
        );
        assert!(backend
            .query(
                MemoryScope::Workspace,
                "valid content but failed commit",
                10
            )
            .is_empty());
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn scopes_are_separate_and_results_are_bounded() {
        let (backend, path) = backend();
        backend.remember(MemoryScope::User, "user fact").unwrap();
        backend
            .remember(MemoryScope::Workspace, "workspace fact")
            .unwrap();
        backend
            .remember(MemoryScope::Project, "project fact")
            .unwrap();
        assert!(
            backend
                .query(MemoryScope::Project, "user fact", 10)
                .is_empty()
        );
        assert_eq!(backend.briefing(MemoryScope::Project, usize::MAX).len(), 1);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn schema_is_readable_and_corruption_degrades_safely() {
        let (backend, path) = backend();
        backend
            .remember(MemoryScope::Project, "visible fact")
            .unwrap();
        let file = path.join("project").join("project.json");
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains("\"schema_version\": 1"));
        fs::write(file, "not json").unwrap();
        assert!(backend.history(MemoryScope::Project, 10).is_empty());
        assert_eq!(
            backend.query_checked(MemoryScope::Project, "visible", 10),
            Err(MemoryReadError::CorruptStore)
        );
        assert_eq!(
            backend.briefing_checked(MemoryScope::Project, 10),
            Err(MemoryReadError::CorruptStore)
        );
        assert_eq!(
            backend.recent_checked(MemoryScope::Project, 10),
            Err(MemoryReadError::CorruptStore)
        );
        assert_eq!(
            backend.history_checked(MemoryScope::Project, 10),
            Err(MemoryReadError::CorruptStore)
        );
        assert_eq!(
            backend.get_checked(MemoryScope::Project, "project:1"),
            Err(MemoryReadError::CorruptStore)
        );
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn checked_reads_distinguish_empty_store_from_unknown_record() {
        let (backend, path) = backend();
        assert_eq!(
            backend.query_checked(MemoryScope::Project, "missing", 10),
            Ok(Vec::new())
        );
        assert_eq!(
            backend.get_checked(MemoryScope::Project, "project:999"),
            Ok(None)
        );
        backend
            .remember(MemoryScope::Project, "checked result exists")
            .unwrap();
        let found = backend
            .query_checked(MemoryScope::Project, "checked result", 10)
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            backend
                .get_checked(MemoryScope::Project, found[0].id.as_deref().unwrap())
                .unwrap(),
            Some(found[0].clone())
        );
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn deleting_and_clearing_memory_updates_canonical_records_and_fts() {
        let (backend, path) = backend();
        backend
            .remember(MemoryScope::Workspace, "workspace removable observation")
            .unwrap();
        backend
            .remember(MemoryScope::Workspace, "workspace retained until clear")
            .unwrap();
        backend
            .remember(MemoryScope::Project, "project memory survives workspace clear")
            .unwrap();

        let entry = backend.history(MemoryScope::Workspace, 10)[0].clone();
        let id = entry.id.as_deref().unwrap();
        assert_eq!(backend.query(MemoryScope::Workspace, "removable observation", 10).len(), 1);
        assert!(!backend
            .delete_memory_entry_in_scope(MemoryScope::Project, id)
            .unwrap());
        assert!(backend
            .delete_memory_entry(id)
            .unwrap());
        assert!(backend.get(MemoryScope::Workspace, id).is_none());
        assert!(backend
            .query(MemoryScope::Workspace, "removable observation", 10)
            .is_empty());

        assert_eq!(backend.clear_all_workspace_memory().unwrap(), 1);
        assert!(backend.history(MemoryScope::Workspace, 10).is_empty());
        assert_eq!(
            backend
                .query(MemoryScope::Project, "project memory survives", 10)
                .len(),
            1
        );
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn workspace_purge_vacuums_and_shrinks_the_fts_database() {
        let (backend, path) = backend();
        let payload = "memory payload ".repeat(240);
        for index in 0..32 {
            backend
                .remember(
                    MemoryScope::Workspace,
                    &format!("entry {index} {payload}"),
                )
                .unwrap();
        }
        let index_path = path.join("memory_index.sqlite");
        let size_before = fs::metadata(&index_path).unwrap().len();
        assert_eq!(backend.clear_all_workspace_memory().unwrap(), 32);
        let size_after = fs::metadata(&index_path).unwrap().len();
        assert!(
            size_after < size_before,
            "expected VACUUM to shrink index: {size_before} -> {size_after} bytes"
        );
        assert!(backend
            .query(MemoryScope::Workspace, "memory payload", 10)
            .is_empty());
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn fts_search_ranks_relevant_records_and_keeps_scope_isolation() {
        let (backend, path) = backend();
        backend
            .remember(
                MemoryScope::Project,
                "Axiom architecture uses focused Rust components",
            )
            .unwrap();
        backend
            .remember(
                MemoryScope::Project,
                "Axiom architecture and memory retrieval are documented",
            )
            .unwrap();
        backend
            .remember(MemoryScope::Project, "unrelated PHP project note")
            .unwrap();
        backend
            .remember(MemoryScope::Workspace, "Axiom architecture workspace note")
            .unwrap();

        let results = backend.query_checked(
            MemoryScope::Project,
            "Axiom architecture",
            10,
        )
        .unwrap();
        assert_eq!(results.len(), 2);
        let repeated = backend
            .query_checked(MemoryScope::Project, "Axiom architecture", 10)
            .unwrap();
        assert_eq!(results, repeated);
        assert!(results.iter().any(|result| result.content.contains("memory retrieval")));
        assert!(results.iter().all(|result| result.scope == MemoryScope::Project));
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn fts_search_quotes_terms_and_supports_unicode_and_punctuation() {
        let (backend, path) = backend();
        backend
            .remember(
                MemoryScope::Project,
                "Axiom: memória persistente — revisão \"M.10\"",
            )
            .unwrap();

        for query in ["memória persistente", "Axiom: \"M.10\""] {
            let results = backend
                .query_checked(MemoryScope::Project, query, 10)
                .unwrap();
            assert_eq!(results.len(), 1, "query should remain valid: {query}");
        }
        assert!(backend
            .query_checked(MemoryScope::Project, "memória OR NOT inválida", 10)
            .unwrap()
            .is_empty());
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn fts_rebuilds_when_index_is_missing_or_corrupt() {
        let (backend, path) = backend();
        backend
            .remember(MemoryScope::Project, "rebuildable canonical fact")
            .unwrap();
        let index = path.join("memory_index.sqlite");
        assert!(index.is_file());

        fs::remove_file(&index).unwrap();
        assert_eq!(
            backend
                .query_checked(MemoryScope::Project, "rebuildable", 10)
                .unwrap()
                .len(),
            1
        );
        assert!(index.is_file());

        fs::write(&index, b"corrupt sqlite").unwrap();
        assert_eq!(
            backend
                .query_checked(MemoryScope::Project, "canonical fact", 10)
                .unwrap()
                .len(),
            1
        );
        assert!(index.is_file());
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn canonical_updates_refresh_fts_and_index_failures_fall_back_to_json() {
        let (backend, path) = backend();
        backend
            .remember(MemoryScope::Project, "old canonical wording")
            .unwrap();
        let file_path = path.join("project").join("project.json");
        let mut file: LocalMemoryFile =
            serde_json::from_str(&fs::read_to_string(&file_path).unwrap()).unwrap();
        file.records[0].content = "new canonical wording".into();
        fs::write(&file_path, serde_json::to_string_pretty(&file).unwrap()).unwrap();
        assert_eq!(
            backend
                .query_checked(MemoryScope::Project, "new canonical", 10)
                .unwrap()
                .len(),
            1
        );

        fs::remove_file(path.join("memory_index.sqlite")).unwrap();
        fs::create_dir(path.join("memory_index.sqlite")).unwrap();
        backend
            .remember(MemoryScope::Project, "json fallback fact")
            .unwrap();
        assert_eq!(
            backend
                .query_checked(MemoryScope::Project, "json fallback", 10)
                .unwrap()
                .len(),
            1
        );
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn sensitive_content_is_rejected_and_not_persisted() {
        let (backend, path) = backend();
        assert!(matches!(
            backend.remember(MemoryScope::Project, "Authorization: Bearer secret"),
            Err(AxiomMemoryError::SensitiveContent)
        ));
        assert!(!path.exists());
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn recent_is_reverse_chronological_and_handoff_is_stored() {
        let (backend, path) = backend();
        backend.remember(MemoryScope::Project, "first").unwrap();
        backend.remember(MemoryScope::Project, "second").unwrap();
        backend
            .persist_handoff(&MemoryHandoff {
                session: crate::MemorySessionId::new(4),
                scope: MemoryScope::Project,
                entries: vec![MemoryResult {
                    id: None,
                    scope: MemoryScope::Project,
                    category: None,
                    content: "handoff".into(),
                }],
            })
            .unwrap();
        assert_eq!(
            backend.recent(MemoryScope::Project, 2)[0].content,
            "handoff"
        );
        assert_eq!(backend.history(MemoryScope::Project, 10).len(), 3);
        let _ = fs::remove_dir_all(path);
    }

    fn stored(backend: &AxiomMemoryBackend, scope: MemoryScope) -> Vec<LocalRecord> {
        backend.read_records(scope).unwrap()
    }

    fn duplicate_setup() -> (
        AxiomMemoryBackend,
        PathBuf,
        String,
        String,
        crate::MemorySessionId,
    ) {
        let (mut backend, path) = backend();
        backend
            .remember(MemoryScope::Project, "stable fact")
            .unwrap();
        let durable_id = backend.history(MemoryScope::Project, 10)[0]
            .id
            .clone()
            .unwrap();
        let session = backend.session_start(MemoryScope::Project);
        backend.observe(
            session,
            MemoryObservation::Discovery(" STABLE   FACT ".into()),
        );
        let raw_id = stored(&backend, MemoryScope::Project)
            .into_iter()
            .find(|record| record.session_id == Some(session.value()))
            .map(|record| format!("project:{}", record.id))
            .unwrap();
        (backend, path, durable_id, raw_id, session)
    }

    #[test]
    fn only_completed_sessions_are_consolidated() {
        let (mut backend, path) = backend();
        let session = backend.session_start(MemoryScope::Project);
        backend.observe(session, MemoryObservation::Discovery("in progress".into()));
        assert_eq!(backend.history(MemoryScope::Project, 10).len(), 1);
        let active = stored(&backend, MemoryScope::Project);
        assert_eq!(active[0].session_id, Some(session.value()));
        assert!(!active[0].session_completed);
        backend.session_end(session);
        let completed = stored(&backend, MemoryScope::Project);
        assert_eq!(completed[0].session_id, None);
        assert_eq!(backend.history(MemoryScope::Project, 10).len(), 1);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn repeated_identical_observations_merge_within_completed_session() {
        let (mut backend, path) = backend();
        let session = backend.session_start(MemoryScope::Project);
        backend.observe(
            session,
            MemoryObservation::Discovery("  Stable   fact ".into()),
        );
        backend.observe(session, MemoryObservation::Discovery("stable fact".into()));
        let before = stored(&backend, MemoryScope::Project);
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].occurrence_count, 2);
        backend.session_end(session);
        let after = stored(&backend, MemoryScope::Project);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].kind, LocalRecordKind::Remember);
        assert_eq!(after[0].occurrence_count, 2);
        assert_eq!(after[0].session_id, None);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn completed_session_consolidation_is_idempotent() {
        let (mut backend, path) = backend();
        let session = backend.session_start(MemoryScope::Project);
        backend.observe(session, MemoryObservation::Discovery("stable fact".into()));
        backend.session_end(session);
        let first = backend.history(MemoryScope::Project, 10);
        backend.session_end(session);
        let recreated = AxiomMemoryBackend::new(
            &path,
            AxiomMemoryScopeIds::new("user", "workspace", "project"),
        );
        let second = recreated.history(MemoryScope::Project, 10);
        assert_eq!(second, first);
        assert_eq!(stored(&recreated, MemoryScope::Project).len(), 1);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn same_content_in_distinct_scopes_is_not_merged() {
        let (mut backend, path) = backend();
        let workspace_session = backend.session_start(MemoryScope::Workspace);
        let project_session = backend.session_start(MemoryScope::Project);
        backend.observe(
            workspace_session,
            MemoryObservation::Discovery("shared text".into()),
        );
        backend.observe(
            project_session,
            MemoryObservation::Discovery("shared text".into()),
        );
        backend.session_end(workspace_session);
        backend.session_end(project_session);
        assert_eq!(backend.history(MemoryScope::Workspace, 10).len(), 1);
        assert_eq!(backend.history(MemoryScope::Project, 10).len(), 1);
        assert_ne!(
            backend.history(MemoryScope::Workspace, 10)[0].id,
            backend.history(MemoryScope::Project, 10)[0].id
        );
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn decisions_remain_protected_durable_records() {
        let (mut backend, path) = backend();
        let session = backend.session_start(MemoryScope::Project);
        backend.observe(
            session,
            MemoryObservation::Decision("use locked checks".into()),
        );
        backend.session_end(session);
        let record = stored(&backend, MemoryScope::Project).remove(0);
        assert_eq!(record.kind, LocalRecordKind::Decision);
        assert_eq!(record.session_id, None);
        assert!(is_protected(record.kind));
        assert_eq!(
            backend.history(MemoryScope::Project, 10)[0].category,
            Some(MemoryCategory::Decision)
        );
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn repeated_gotchas_consolidate_to_one_protected_record() {
        let (mut backend, path) = backend();
        let session = backend.session_start(MemoryScope::Project);
        backend.observe(session, MemoryObservation::Gotcha("stale cache".into()));
        backend.observe(session, MemoryObservation::Gotcha(" STALE   CACHE ".into()));
        backend.session_end(session);
        let records = stored(&backend, MemoryScope::Project);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, LocalRecordKind::Gotcha);
        assert!(is_protected(records[0].kind));
        assert_eq!(records[0].occurrence_count, 2);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn validations_and_outcomes_consolidate_without_mutation_log_records() {
        let (mut backend, path) = backend();
        let session = backend.session_start(MemoryScope::Project);
        backend.observe(
            session,
            MemoryObservation::Validation("cargo check passed".into()),
        );
        backend.observe(
            session,
            MemoryObservation::TaskOutcome("completed successfully".into()),
        );
        backend.observe(
            session,
            MemoryObservation::MutationSummary("raw tool output".into()),
        );
        backend.session_end(session);
        let records = stored(&backend, MemoryScope::Project);
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|record| record.session_id.is_none()));
        assert!(
            records
                .iter()
                .any(|record| record.kind == LocalRecordKind::ValidationOutcome)
        );
        assert!(
            records
                .iter()
                .any(|record| record.kind == LocalRecordKind::Outcome)
        );
        assert!(
            records
                .iter()
                .all(|record| !record.content.contains("raw tool output"))
        );
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn rejected_sensitive_observations_never_reappear_after_consolidation() {
        let (mut backend, path) = backend();
        let session = backend.session_start(MemoryScope::Project);
        backend.observe(
            session,
            MemoryObservation::Discovery("Authorization: Bearer secret".into()),
        );
        backend.session_end(session);
        assert!(backend.query(MemoryScope::Project, "secret", 10).is_empty());
        assert!(stored(&backend, MemoryScope::Project).is_empty());
        assert!(!path.join("project/project.json").exists());
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn failed_consolidation_keeps_completed_raw_records_and_retries() {
        let (mut backend, path) = backend();
        let session = backend.session_start(MemoryScope::Project);
        backend.observe(
            session,
            MemoryObservation::Discovery("retryable fact".into()),
        );
        backend.fail_after_successful_writes(1);
        backend.session_end(session);
        let after_failure = stored(&backend, MemoryScope::Project);
        assert_eq!(after_failure.len(), 1);
        assert_eq!(after_failure[0].session_id, Some(session.value()));
        assert!(after_failure[0].session_completed);
        drop(backend);
        let recreated = AxiomMemoryBackend::new(
            &path,
            AxiomMemoryScopeIds::new("user", "workspace", "project"),
        );
        let result = recreated.briefing(MemoryScope::Project, 10);
        assert_eq!(result.len(), 1);
        let recovered = stored(&recreated, MemoryScope::Project);
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].session_id, None);
        assert_eq!(recovered[0].content, "retryable fact");
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn failed_atomic_replace_preserves_previous_file_and_cleans_temp() {
        let (backend, path) = backend();
        backend.remember(MemoryScope::Project, "original").unwrap();
        let file = path.join("project/project.json");
        let before = fs::read(&file).unwrap();
        backend.fail_after_successful_writes(0);
        assert!(
            backend
                .remember(MemoryScope::Project, "replacement")
                .is_err()
        );
        backend.clear_write_failure();
        assert_eq!(fs::read(&file).unwrap(), before);
        assert_eq!(backend.history(MemoryScope::Project, 10).len(), 1);
        assert_eq!(fs::read_dir(file.parent().unwrap()).unwrap().count(), 1);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn repeated_sessions_are_bounded_by_durable_retention() {
        let (mut backend, path) = backend();
        for index in 0..(MAX_DURABLE_RECORDS + 12) {
            let session = backend.session_start(MemoryScope::Project);
            backend.observe(
                session,
                MemoryObservation::Discovery(format!("fact {index}")),
            );
            backend.session_end(session);
        }
        let records = stored(&backend, MemoryScope::Project);
        assert!(records.iter().all(|record| record.session_id.is_none()));
        assert_eq!(records.len(), MAX_DURABLE_RECORDS);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn protected_decisions_survive_unprotected_retention_and_are_admission_bounded() {
        let (backend, path) = backend();
        backend
            .decision(MemoryScope::Project, "preserve this")
            .unwrap();
        for index in 0..(MAX_DURABLE_RECORDS + 4) {
            backend
                .remember(MemoryScope::Project, &format!("fact {index}"))
                .unwrap();
        }
        let records = stored(&backend, MemoryScope::Project);
        assert!(
            records
                .iter()
                .any(|record| record.kind == LocalRecordKind::Decision
                    && record.content == "preserve this")
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| !is_protected(record.kind))
                .count(),
            MAX_DURABLE_RECORDS
        );
        for index in 0..MAX_PROTECTED_RECORDS - 1 {
            backend
                .gotcha(MemoryScope::Project, &format!("gotcha {index}"))
                .unwrap();
        }
        assert!(matches!(
            backend.gotcha(MemoryScope::Project, "one too many"),
            Err(AxiomMemoryError::ProtectedRecordLimit)
        ));
        assert!(
            stored(&backend, MemoryScope::Project)
                .iter()
                .any(|record| record.content == "preserve this")
        );
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn active_and_completed_episodic_retention_is_bounded() {
        let (mut backend, path) = backend();
        let mut sessions = Vec::new();
        for index in 0..MAX_ACTIVE_SESSIONS {
            let session = backend.session_start(MemoryScope::Project);
            for item in 0..(MAX_OBSERVATIONS_PER_SESSION + 4) {
                backend.observe(
                    session,
                    MemoryObservation::Discovery(format!("{index}-{item}")),
                );
            }
            sessions.push(session);
        }
        let overflow = backend.session_start(MemoryScope::Project);
        backend.observe(
            overflow,
            MemoryObservation::Discovery("not admitted".into()),
        );
        assert_eq!(
            stored(&backend, MemoryScope::Project).len(),
            MAX_EPISODIC_RECORDS
        );
        for session in sessions {
            backend.session_end(session);
        }
        let records = stored(&backend, MemoryScope::Project);
        assert!(
            records
                .iter()
                .filter(|record| record.session_id.is_some())
                .count()
                <= MAX_EPISODIC_RECORDS
        );
        assert!(records.len() <= MAX_DURABLE_RECORDS);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn briefing_prefers_durable_duplicate_over_raw_observation() {
        let (backend, path, durable_id, _raw_id, _session) = duplicate_setup();
        let results = backend.briefing(MemoryScope::Project, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id.as_deref(), Some(durable_id.as_str()));
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn memory_search_suppresses_durable_and_raw_duplicates() {
        let (backend, path, durable_id, _raw_id, _session) = duplicate_setup();
        let results = backend.query(MemoryScope::Project, "stable fact", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id.as_deref(), Some(durable_id.as_str()));
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn provider_failures_remain_bounded_session_evidence_not_durable_memory() {
        let (mut backend, path) = backend();
        for _ in 0..3 {
            let session = backend.session_start(MemoryScope::Project);
            backend.observe(
                session,
                MemoryObservation::SessionOutcome("failed due to provider error".into()),
            );
            backend.session_end(session);
        }

        let records = stored(&backend, MemoryScope::Project);
        assert_eq!(records.len(), 3);
        assert!(records.iter().all(|record| {
            record.kind == LocalRecordKind::SessionOutcome
                && record.session_id.is_some()
                && record.session_completed
                && !is_protected(record.kind)
        }));
        assert!(backend.briefing(MemoryScope::Project, 10).is_empty());
        assert!(backend
            .query(MemoryScope::Project, "provider error", 10)
            .is_empty());
        let history = backend.history(MemoryScope::Project, 10);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].category, Some(MemoryCategory::SessionOutcome));
        assert_eq!(history[0].content, "failed due to provider error");
        assert!(records.len() <= MAX_EPISODIC_RECORDS);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn memory_get_keeps_durable_ids_and_drops_compacted_duplicate_ids() {
        let (mut backend, path, durable_id, raw_id, session) = duplicate_setup();
        let durable = backend.get(MemoryScope::Project, &durable_id).unwrap();
        assert!(backend.get(MemoryScope::Project, &raw_id).is_some());
        backend.session_end(session);
        assert_eq!(
            backend.get(MemoryScope::Project, &durable_id),
            Some(durable)
        );
        assert!(backend.get(MemoryScope::Project, &raw_id).is_none());
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn promoted_observation_keeps_its_logical_id() {
        let (mut backend, path) = backend();
        let session = backend.session_start(MemoryScope::Project);
        backend.observe(
            session,
            MemoryObservation::Discovery("promoted fact".into()),
        );
        let raw_id = stored(&backend, MemoryScope::Project)[0].id;
        let logical_id = format!("project:{raw_id}");
        let before = backend.get(MemoryScope::Project, &logical_id).unwrap();

        backend.session_end(session);

        assert_eq!(backend.get(MemoryScope::Project, &logical_id), Some(before));
        let _ = fs::remove_dir_all(path);
    }
}
