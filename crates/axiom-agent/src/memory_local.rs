//! Axiom-owned durable memory backend.
//!
//! The JSON files in this module are the canonical representation. The
//! backend has no provider, MCP, HTTP, SQLite, or GPUI dependency. Callers
//! must use it from a background worker when persistence is involved.

use crate::{
    MemoryCategory, MemoryHandoff, MemoryObservation, MemoryResult, MemoryScope, MemoryService,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

pub const LOCAL_MEMORY_SCHEMA_VERSION: u32 = 1;
pub const MAX_LOCAL_MEMORY_RESULTS: usize = 100;
pub const MAX_RECORD_CONTENT_CHARS: usize = 4096;
const MAX_MEMORY_FILE_BYTES: u64 = 64 * 1024 * 1024;
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
}

impl std::fmt::Display for AxiomMemoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(_) => f.write_str("memory storage I/O failed"),
            Self::Serialization(_) => f.write_str("memory storage serialization failed"),
            Self::CorruptStore => f.write_str("memory storage is corrupt"),
            Self::SensitiveContent => f.write_str("memory content rejected as sensitive"),
            Self::InvalidScopeId => f.write_str("memory scope identifier is invalid"),
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum LocalRecordKind {
    Remember,
    Decision,
    Gotcha,
    Handoff,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct LocalRecord {
    id: u64,
    kind: LocalRecordKind,
    content: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct LocalMemoryFile {
    schema_version: u32,
    scope: String,
    scope_id: String,
    records: Vec<LocalRecord>,
}

pub struct AxiomMemoryBackend {
    base_directory: PathBuf,
    scope_ids: AxiomMemoryScopeIds,
    next_session_id: AtomicU64,
    sessions: Mutex<HashMap<crate::MemorySessionId, MemoryScope>>,
}

impl AxiomMemoryBackend {
    pub fn new(base_directory: impl Into<PathBuf>, scope_ids: AxiomMemoryScopeIds) -> Self {
        Self {
            base_directory: base_directory.into(),
            scope_ids,
            next_session_id: AtomicU64::new(0),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub fn base_directory(&self) -> &Path {
        &self.base_directory
    }

    pub fn remember(&self, scope: MemoryScope, content: &str) -> Result<(), AxiomMemoryError> {
        self.append(scope, LocalRecordKind::Remember, content)
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
        let query = normalize(query);
        if query.is_empty() {
            return Vec::new();
        }
        self.read_records(scope)
            .unwrap_or_default()
            .into_iter()
            .filter(|record| {
                normalize(&record.content)
                    .to_ascii_lowercase()
                    .contains(&query.to_ascii_lowercase())
            })
            .take(limit.min(MAX_LOCAL_MEMORY_RESULTS))
            .map(|record| self.result(scope, record))
            .collect()
    }

    pub fn briefing(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        self.results(scope, limit)
    }

    pub fn recent(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        let mut records = self.read_records(scope).unwrap_or_default();
        records.reverse();
        records
            .into_iter()
            .take(limit.min(MAX_LOCAL_MEMORY_RESULTS))
            .map(|record| self.result(scope, record))
            .collect()
    }

    pub fn history(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        self.results(scope, limit)
    }

    fn results(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        self.read_records(scope)
            .unwrap_or_default()
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
        let content = sanitize_content(content)?;
        let mut file = self.read_file(scope)?.unwrap_or_else(|| LocalMemoryFile {
            schema_version: LOCAL_MEMORY_SCHEMA_VERSION,
            scope: scope_name(scope).to_owned(),
            scope_id: self.scope_ids.id(scope).to_owned(),
            records: Vec::new(),
        });
        let id = file
            .records
            .last()
            .map_or(1, |record| record.id.saturating_add(1));
        file.records.push(LocalRecord { id, kind, content });
        self.write_file(scope, &file)
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
        let Ok(metadata) = fs::metadata(&path) else {
            return Ok(None);
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
        fs::write(&temp, json)?;
        if let Err(error) = fs::rename(&temp, &path) {
            let _ = fs::remove_file(&temp);
            return Err(error.into());
        }
        Ok(())
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
}

impl MemoryService for AxiomMemoryBackend {
    fn session_start(&mut self, scope: MemoryScope) -> crate::MemorySessionId {
        let id = crate::MemorySessionId::new(
            self.next_session_id
                .fetch_add(1, Ordering::Relaxed)
                .saturating_add(1),
        );
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.insert(id, scope);
        }
        id
    }

    fn observe(&mut self, session: crate::MemorySessionId, observation: MemoryObservation) {
        let scope = self
            .sessions
            .lock()
            .ok()
            .and_then(|sessions| sessions.get(&session).copied());
        let Some(scope) = scope else {
            return;
        };
        let (kind, content) = match observation {
            MemoryObservation::Discovery(content) => (LocalRecordKind::Remember, content),
            MemoryObservation::Decision(content) => (LocalRecordKind::Decision, content),
            MemoryObservation::MutationSummary(content) => (LocalRecordKind::Remember, content),
            MemoryObservation::Validation(content) => (LocalRecordKind::Remember, content),
            MemoryObservation::TaskOutcome(content) => (LocalRecordKind::Remember, content),
        };
        let _ = self.append(scope, kind, &content);
    }

    fn session_end(&mut self, session: crate::MemorySessionId) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(&session);
        }
    }

    fn query(&self, scope: MemoryScope, query: &str, limit: usize) -> Vec<MemoryResult> {
        Self::query(self, scope, query, limit)
    }

    fn briefing(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        Self::briefing(self, scope, limit)
    }

    fn recent(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        Self::recent(self, scope, limit)
    }

    fn history(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        Self::history(self, scope, limit)
    }

    fn get(&self, scope: MemoryScope, id: &str) -> Option<MemoryResult> {
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

    fn handoff(&self, _session: crate::MemorySessionId) -> Option<MemoryHandoff> {
        None
    }
}

fn scope_name(scope: MemoryScope) -> &'static str {
    match scope {
        MemoryScope::User => "user",
        MemoryScope::Workspace => "workspace",
        MemoryScope::Project => "project",
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
}
