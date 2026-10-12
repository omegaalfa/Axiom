use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

const HISTORY_VERSION: u32 = 1;
const MAX_HISTORY_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_MESSAGES_PER_CONVERSATION: usize = 500;
const MAX_MESSAGE_CHARS: usize = 32_000;
const MAX_TOOL_SUMMARIES: usize = 32;
const MAX_TOOL_NAME_CHARS: usize = 64;
const SNAPSHOTS_TO_KEEP: usize = 8;
const WRITER_SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
static NEXT_STORAGE_REVISION: AtomicU64 = AtomicU64::new(0);
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AgentHistoryMessage {
    pub id: u64,
    pub role: AgentHistoryRole,
    pub content: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum AgentHistoryRole {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AgentToolSummary {
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AgentHistoryConversation {
    pub id: String,
    pub title: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub messages: Vec<AgentHistoryMessage>,
    #[serde(default)]
    pub tool_summaries: Vec<AgentToolSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AgentHistorySnapshot {
    version: u32,
    workspace_id: String,
    revision: u64,
    next_conversation_id: u64,
    conversations: Vec<AgentHistoryConversation>,
}

#[derive(Clone, Debug)]
pub(crate) struct AgentHistoryStore {
    workspace_id: String,
    revision: u64,
    next_conversation_id: u64,
    conversations: Vec<AgentHistoryConversation>,
    warning: Option<String>,
}

impl AgentHistoryStore {
    pub(crate) fn load(directory: &Path, workspace_id: String) -> Result<Self, String> {
        let snapshots = snapshot_paths(directory).map_err(|error| error.to_string())?;
        if snapshots.is_empty() {
            return Ok(Self {
                workspace_id,
                revision: 0,
                next_conversation_id: 1,
                conversations: Vec::new(),
                warning: None,
            });
        }

        let highest_revision = snapshots.last().map(|(revision, _)| *revision).unwrap_or(0);
        let mut latest_corruption = None;
        for (revision, path) in snapshots.into_iter().rev() {
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    latest_corruption = Some(format!("{}: {error}", path.display()));
                    continue;
                }
            };
            if bytes.len() as u64 > MAX_HISTORY_FILE_BYTES {
                latest_corruption = Some(format!(
                    "{}: history snapshot exceeds size limit",
                    path.display()
                ));
                continue;
            }
            let snapshot = match serde_json::from_slice::<AgentHistorySnapshot>(&bytes) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    latest_corruption = Some(format!("{}: {error}", path.display()));
                    continue;
                }
            };
            if snapshot.version != HISTORY_VERSION || snapshot.workspace_id != workspace_id {
                latest_corruption = Some(format!(
                    "{}: unsupported or mismatched history snapshot",
                    path.display()
                ));
                continue;
            }
            return Ok(Self {
                workspace_id,
                revision: snapshot.revision.max(revision).max(highest_revision),
                next_conversation_id: snapshot.next_conversation_id.max(1),
                conversations: snapshot.conversations,
                warning: latest_corruption,
            });
        }
        Err(latest_corruption.unwrap_or_else(|| "Agent history could not be loaded".into()))
    }

    pub(crate) fn conversations(&self) -> &[AgentHistoryConversation] {
        &self.conversations
    }

    pub(crate) fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    pub(crate) fn conversation(&self, id: &str) -> Option<&AgentHistoryConversation> {
        self.conversations
            .iter()
            .find(|conversation| conversation.id == id)
    }

    pub(crate) fn delete_conversation(&mut self, id: &str) -> bool {
        let before = self.conversations.len();
        self.conversations.retain(|conversation| conversation.id != id);
        before != self.conversations.len()
    }

    pub(crate) fn create_conversation(&mut self, first_message: &str, now: u64) -> String {
        let sequence = self.next_conversation_id;
        self.next_conversation_id = self.next_conversation_id.saturating_add(1);
        let id = format!("{}-{sequence}", self.workspace_id);
        self.conversations.push(AgentHistoryConversation {
            id: id.clone(),
            title: title_from_message(first_message),
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
            tool_summaries: Vec::new(),
        });
        id
    }

    pub(crate) fn replace_messages(
        &mut self,
        id: &str,
        messages: Vec<AgentHistoryMessage>,
        now: u64,
    ) -> Result<(), String> {
        let conversation = self
            .conversations
            .iter_mut()
            .find(|conversation| conversation.id == id)
            .ok_or_else(|| "Agent conversation no longer exists".to_owned())?;
        if messages.len() > MAX_MESSAGES_PER_CONVERSATION
            || messages
                .iter()
                .any(|message| message.content.chars().count() > MAX_MESSAGE_CHARS)
        {
            return Err("Agent conversation exceeds the history size limits".into());
        }
        if messages
            .iter()
            .any(|message| contains_sensitive_marker(&message.content))
        {
            return Err("Conversation contains sensitive-looking content; it was not saved".into());
        }
        conversation.messages = messages;
        conversation.updated_at = now;
        self.sort_conversations();
        Ok(())
    }

    pub(crate) fn add_tool_summaries(&mut self, id: &str, names: &[String], now: u64) {
        let Some(conversation) = self
            .conversations
            .iter_mut()
            .find(|conversation| conversation.id == id)
        else {
            return;
        };
        for name in names {
            if conversation.tool_summaries.len() >= MAX_TOOL_SUMMARIES {
                break;
            }
            let safe_name: String = name
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                .take(MAX_TOOL_NAME_CHARS)
                .collect();
            if safe_name.is_empty() {
                continue;
            }
            if !conversation
                .tool_summaries
                .iter()
                .any(|summary| summary.name == safe_name)
            {
                conversation
                    .tool_summaries
                    .push(AgentToolSummary { name: safe_name });
            }
            if conversation.tool_summaries.len() >= MAX_TOOL_SUMMARIES {
                break;
            }
        }
        conversation.updated_at = now;
        self.sort_conversations();
    }

    fn sort_conversations(&mut self) {
        self.conversations.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| b.created_at.cmp(&a.created_at))
                .then_with(|| b.id.cmp(&a.id))
        });
    }

    fn snapshot_bytes(&self, revision: u64) -> Result<Vec<u8>, String> {
        if self.conversations.iter().any(|conversation| {
            conversation
                .messages
                .iter()
                .any(|message| contains_sensitive_marker(&message.content))
        }) {
            return Err("Conversation contains sensitive-looking content; it was not saved".into());
        }
        let snapshot = AgentHistorySnapshot {
            version: HISTORY_VERSION,
            workspace_id: self.workspace_id.clone(),
            revision,
            next_conversation_id: self.next_conversation_id,
            conversations: self.conversations.clone(),
        };
        let bytes = serde_json::to_vec(&snapshot).map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_HISTORY_FILE_BYTES {
            return Err("Agent history exceeds the 8 MiB snapshot limit".into());
        }
        Ok(bytes)
    }
}

pub(crate) fn workspace_location(
    config_directory: &Path,
    root: &Path,
) -> Result<(PathBuf, String), String> {
    let root = fs::canonicalize(root).map_err(|error| error.to_string())?;
    let normalized = root.to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    let normalized = normalized.to_lowercase();
    let workspace_id = format!("{:016x}", stable_hash(normalized.as_bytes()));
    Ok((
        config_directory.join("agent-history").join(&workspace_id),
        workspace_id,
    ))
}

pub(crate) fn load_for_workspace(
    config_directory: &Path,
    root: &Path,
) -> Result<(PathBuf, AgentHistoryStore), String> {
    let (directory, workspace_id) = workspace_location(config_directory, root)?;
    let store = AgentHistoryStore::load(&directory, workspace_id)?;
    Ok((directory, store))
}

fn stable_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

fn title_from_message(message: &str) -> String {
    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = normalized.chars().take(48).collect::<String>();
    if normalized.chars().count() > 48 {
        chars.push('…');
    }
    chars
}

fn contains_sensitive_marker(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    [
        "authorization:",
        "bearer ",
        "password=",
        "password:",
        "api_key=",
        "api-key:",
        "access_token=",
        "access-token:",
        "-----begin private key-----",
    ]
    .iter()
    .any(|pattern| lower.contains(pattern))
}

pub(crate) fn format_timestamp(timestamp: u64) -> String {
    let days = (timestamp / 86_400) as i64;
    let seconds = timestamp % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        seconds / 3_600,
        (seconds % 3_600) / 60
    )
}

fn snapshot_paths(directory: &Path) -> std::io::Result<Vec<(u64, PathBuf)>> {
    let mut snapshots = Vec::new();
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(snapshots),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(revision) = name
            .strip_prefix("snapshot-")
            .and_then(|name| name.strip_suffix(".json"))
            .and_then(|revision| revision.parse::<u64>().ok())
        else {
            continue;
        };
        snapshots.push((revision, entry.path()));
    }
    snapshots.sort_by_key(|(revision, _)| *revision);
    Ok(snapshots)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AgentHistoryWriteEvent {
    Saved(u64),
    Failed(u64, String),
}

struct PendingWrite {
    request_revision: u64,
    storage_revision: u64,
    store: AgentHistoryStore,
}

struct WriterQueue {
    pending: Option<PendingWrite>,
    closed: bool,
}

pub(crate) struct AgentHistoryWriter {
    store: AgentHistoryStore,
    queue: Arc<(Mutex<WriterQueue>, Condvar)>,
    events: mpsc::Receiver<AgentHistoryWriteEvent>,
    completion: mpsc::Receiver<()>,
    worker: Option<thread::JoinHandle<()>>,
    next_revision: AtomicU64,
}

impl AgentHistoryWriter {
    pub(crate) fn new(directory: PathBuf, store: AgentHistoryStore) -> Result<Self, String> {
        NEXT_STORAGE_REVISION.fetch_max(store.revision, Ordering::SeqCst);
        let queue = Arc::new((
            Mutex::new(WriterQueue {
                pending: None,
                closed: false,
            }),
            Condvar::new(),
        ));
        let worker_queue = queue.clone();
        let worker_directory = directory.clone();
        let (events_tx, events) = mpsc::channel();
        let (completion_tx, completion) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("axiom-agent-history-writer".into())
            .spawn(move || writer_loop(worker_directory, worker_queue, events_tx, completion_tx))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            next_revision: AtomicU64::new(store.revision),
            store,
            queue,
            events,
            completion,
            worker: Some(worker),
        })
    }

    pub(crate) fn store(&self) -> &AgentHistoryStore {
        &self.store
    }

    pub(crate) fn store_mut(&mut self) -> &mut AgentHistoryStore {
        &mut self.store
    }

    pub(crate) fn submit(&mut self) -> Result<u64, String> {
        if self.store.conversations.iter().any(|conversation| {
            conversation.messages.len() > MAX_MESSAGES_PER_CONVERSATION
                || conversation
                    .messages
                    .iter()
                    .any(|message| message.content.chars().count() > MAX_MESSAGE_CHARS)
        }) {
            return Err("Agent history exceeds its storage limits".into());
        }
        let revision = self.next_revision.fetch_add(1, Ordering::Relaxed) + 1;
        let storage_revision = next_storage_revision();
        let (lock, wake) = &*self.queue;
        let mut state = lock
            .lock()
            .map_err(|_| "history writer unavailable".to_owned())?;
        if state.closed {
            return Err("history writer is closed".into());
        }
        state.pending = Some(PendingWrite {
            request_revision: revision,
            storage_revision,
            store: self.store.clone(),
        });
        wake.notify_one();
        Ok(revision)
    }

    pub(crate) fn try_event(&self) -> Option<AgentHistoryWriteEvent> {
        self.events.try_recv().ok()
    }

    pub(crate) fn shutdown(&mut self) -> Result<(), String> {
        if self.worker.is_none() {
            return Ok(());
        }
        let (lock, wake) = &*self.queue;
        let mut state = lock
            .lock()
            .map_err(|_| "history writer unavailable".to_owned())?;
        state.closed = true;
        wake.notify_one();
        drop(state);

        self.completion
            .recv_timeout(WRITER_SHUTDOWN_TIMEOUT)
            .map_err(|_| "history writer shutdown timed out".to_owned())?;
        let worker = self
            .worker
            .take()
            .ok_or_else(|| "history writer unavailable".to_owned())?;
        worker
            .join()
            .map_err(|_| "history writer thread panicked".to_owned())
    }
}

impl Drop for AgentHistoryWriter {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn writer_loop(
    directory: PathBuf,
    queue: Arc<(Mutex<WriterQueue>, Condvar)>,
    events: mpsc::Sender<AgentHistoryWriteEvent>,
    completion: mpsc::Sender<()>,
) {
    'writer: loop {
        let pending = {
            let (lock, wake) = &*queue;
            let Ok(mut state) = lock.lock() else { break 'writer };
            while state.pending.is_none() && !state.closed {
                let Ok(next) = wake.wait(state) else { break 'writer };
                state = next;
            }
            if state.pending.is_none() && state.closed {
                break 'writer;
            }
            state.pending.take()
        };
        let Some(pending) = pending else { continue };
        let result = write_snapshot(&directory, pending.storage_revision, &pending.store);
        let event = match result {
            Ok(()) => AgentHistoryWriteEvent::Saved(pending.request_revision),
            Err(error) => AgentHistoryWriteEvent::Failed(pending.request_revision, error),
        };
        let _ = events.send(event);
    }
    let _ = completion.send(());
}

fn write_snapshot(
    directory: &Path,
    revision: u64,
    store: &AgentHistoryStore,
) -> Result<(), String> {
    let bytes = store.snapshot_bytes(revision)?;
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let final_path = directory.join(format!("snapshot-{revision:020}.json"));
    let temp_path = directory.join(format!(
        "snapshot-{revision:020}-{}-{}.tmp",
        std::process::id(),
        TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = File::options()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .map_err(|error| error.to_string())?;
    let write_result = (|| {
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::hard_link(&temp_path, &final_path)?;
        fs::remove_file(&temp_path)?;
        Ok::<(), std::io::Error>(())
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(error.to_string());
    }
    prune_snapshots(directory, revision);
    Ok(())
}

fn next_storage_revision() -> u64 {
    let wall_clock = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default();
    NEXT_STORAGE_REVISION
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
            Some(current.max(wall_clock).saturating_add(1))
        })
        .unwrap_or_default()
        .max(wall_clock)
        .saturating_add(1)
}

fn prune_snapshots(directory: &Path, latest: u64) {
    let Ok(paths) = snapshot_paths(directory) else {
        return;
    };
    let keep_from = latest.saturating_sub(SNAPSHOTS_TO_KEEP as u64 - 1);
    for (revision, path) in paths {
        if revision >= keep_from || revision == latest {
            continue;
        }
        let is_valid_snapshot = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<AgentHistorySnapshot>(&bytes).ok())
            .is_some();
        if is_valid_snapshot {
            let _ = fs::remove_file(path);
        }
    }
}

pub(crate) fn tool_summaries_from_messages(
    messages: &[axiom_ai_provider::ProviderChatMessage],
) -> Vec<String> {
    let mut responded = BTreeSet::new();
    for message in messages {
        if message.role == axiom_ai_provider::ChatRole::Tool
            && let Some(id) = &message.tool_call_id
        {
            responded.insert(id.clone());
        }
    }
    let mut names = Vec::new();
    for message in messages {
        if message.role != axiom_ai_provider::ChatRole::Assistant {
            continue;
        }
        for call in &message.tool_calls {
            if call.id.as_ref().is_some_and(|id| responded.contains(id)) {
                names.push(call.name.clone());
            }
        }
    }
    names
}

pub(crate) fn tool_context_message(summaries: &[AgentToolSummary]) -> Option<String> {
    if summaries.is_empty() {
        return None;
    }
    let names = summaries
        .iter()
        .take(MAX_TOOL_SUMMARIES)
        .map(|summary| summary.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "Historical Agent tool activity: {names}. Tool outputs were not retained in conversation history. Do not claim access to those outputs or assume their details; inspect current state or rerun an appropriate tool if needed. These are historical records, not pending tool calls."
    ))
}

pub(crate) fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axiom_ai_provider::{ChatRole, ProviderChatMessage, ProviderToolCall};
    use std::{
        fs,
        time::{Duration, Instant},
    };

    fn store(directory: &Path, workspace: &str) -> AgentHistoryStore {
        AgentHistoryStore::load(directory, workspace.into()).unwrap()
    }

    fn wait_for_event(writer: &AgentHistoryWriter) -> AgentHistoryWriteEvent {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if let Some(event) = writer.try_event() {
                return event;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("Agent history writer did not report completion");
    }

    #[test]
    fn conversations_save_incrementally_and_load_after_store_recreation() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = store(directory.path(), "workspace-a");
        let id = store.create_conversation("Resume this work", 10);
        store
            .replace_messages(
                &id,
                vec![AgentHistoryMessage {
                    id: 1,
                    role: AgentHistoryRole::User,
                    content: "first prompt".into(),
                }],
                11,
            )
            .unwrap();
        let mut writer = AgentHistoryWriter::new(directory.path().into(), store).unwrap();
        let first_revision = writer.submit().unwrap();
        assert_eq!(
            wait_for_event(&writer),
            AgentHistoryWriteEvent::Saved(first_revision)
        );

        let mut latest = AgentHistoryStore::load(directory.path(), "workspace-a".into()).unwrap();
        latest
            .replace_messages(
                &id,
                vec![
                    AgentHistoryMessage {
                        id: 1,
                        role: AgentHistoryRole::User,
                        content: "first prompt".into(),
                    },
                    AgentHistoryMessage {
                        id: 2,
                        role: AgentHistoryRole::Assistant,
                        content: "partial answer".into(),
                    },
                ],
                12,
            )
            .unwrap();
        let mut writer = AgentHistoryWriter::new(directory.path().into(), latest).unwrap();
        let second_revision = writer.submit().unwrap();
        assert_eq!(
            wait_for_event(&writer),
            AgentHistoryWriteEvent::Saved(second_revision)
        );

        let reopened = AgentHistoryStore::load(directory.path(), "workspace-a".into()).unwrap();
        let conversation = reopened.conversation(&id).unwrap();
        assert_eq!(conversation.title, "Resume this work");
        assert_eq!(conversation.messages.len(), 2);
        assert_eq!(conversation.messages[1].content, "partial answer");
    }

    #[test]
    fn shutdown_flushes_pending_snapshot_before_writer_terminates() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = store(directory.path(), "workspace-shutdown");
        let id = store.create_conversation("Flush on shutdown", 1);
        store
            .replace_messages(
                &id,
                vec![AgentHistoryMessage {
                    id: 1,
                    role: AgentHistoryRole::Assistant,
                    content: "last snapshot must be durable".into(),
                }],
                2,
            )
            .unwrap();

        let mut writer = AgentHistoryWriter::new(directory.path().into(), store).unwrap();
        writer.submit().unwrap();
        writer.shutdown().unwrap();

        let reloaded = AgentHistoryStore::load(directory.path(), "workspace-shutdown".into())
            .unwrap();
        assert_eq!(
            reloaded.conversations()[0].messages[0].content,
            "last snapshot must be durable"
        );
    }

    #[test]
    fn conversations_are_listed_and_selected_by_id() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = store(directory.path(), "workspace-a");
        let first = store.create_conversation("first", 1);
        let second = store.create_conversation("second", 2);
        let mut writer = AgentHistoryWriter::new(directory.path().into(), store.clone()).unwrap();
        let revision = writer.submit().unwrap();
        assert_eq!(
            wait_for_event(&writer),
            AgentHistoryWriteEvent::Saved(revision)
        );
        let reopened = AgentHistoryStore::load(directory.path(), "workspace-a".into()).unwrap();
        assert_eq!(reopened.conversations().len(), 2);
        assert_eq!(reopened.conversation(&first).unwrap().title, "first");
        assert_eq!(reopened.conversation(&second).unwrap().title, "second");
    }

    #[test]
    fn deleting_conversation_persists_and_missing_delete_is_a_noop() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = store(directory.path(), "workspace-a");
        let keep = store.create_conversation("keep", 1);
        let remove = store.create_conversation("remove", 2);
        assert!(store.delete_conversation(&remove));
        assert!(!store.delete_conversation("workspace-a-unknown"));
        let mut writer = AgentHistoryWriter::new(directory.path().into(), store).unwrap();
        let revision = writer.submit().unwrap();
        assert_eq!(
            wait_for_event(&writer),
            AgentHistoryWriteEvent::Saved(revision)
        );

        let reopened = AgentHistoryStore::load(directory.path(), "workspace-a".into()).unwrap();
        assert!(reopened.conversation(&keep).is_some());
        assert!(reopened.conversation(&remove).is_none());
    }

    #[test]
    fn coalesced_snapshots_cannot_leave_an_older_conversation_state_on_disk() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = store(directory.path(), "workspace-a");
        let id = store.create_conversation("first", 1);
        store
            .replace_messages(
                &id,
                vec![AgentHistoryMessage {
                    id: 1,
                    role: AgentHistoryRole::User,
                    content: "first".into(),
                }],
                1,
            )
            .unwrap();
        let mut writer = AgentHistoryWriter::new(directory.path().into(), store).unwrap();
        let older = writer.submit().unwrap();
        writer
            .store_mut()
            .replace_messages(
                &id,
                vec![
                    AgentHistoryMessage {
                        id: 1,
                        role: AgentHistoryRole::User,
                        content: "first".into(),
                    },
                    AgentHistoryMessage {
                        id: 2,
                        role: AgentHistoryRole::Assistant,
                        content: "latest".into(),
                    },
                ],
                2,
            )
            .unwrap();
        let latest = writer.submit().unwrap();
        assert!(latest > older);
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut latest_saved = false;
        while Instant::now() < deadline {
            if let Some(AgentHistoryWriteEvent::Saved(revision)) = writer.try_event()
                && revision == latest
            {
                latest_saved = true;
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(latest_saved);
        let reopened = AgentHistoryStore::load(directory.path(), "workspace-a".into()).unwrap();
        assert_eq!(
            reopened
                .conversation(&id)
                .unwrap()
                .messages
                .last()
                .unwrap()
                .content,
            "latest"
        );
    }

    #[test]
    fn dropping_writer_still_flushes_latest_queued_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = store(directory.path(), "workspace-a");
        let id = store.create_conversation("first", 1);
        store
            .replace_messages(
                &id,
                vec![AgentHistoryMessage {
                    id: 1,
                    role: AgentHistoryRole::User,
                    content: "older".repeat(2_500),
                }],
                1,
            )
            .unwrap();
        let mut writer = AgentHistoryWriter::new(directory.path().into(), store).unwrap();
        writer.submit().unwrap();
        writer
            .store_mut()
            .replace_messages(
                &id,
                vec![AgentHistoryMessage {
                    id: 1,
                    role: AgentHistoryRole::User,
                    content: "latest snapshot".into(),
                }],
                2,
            )
            .unwrap();
        writer.submit().unwrap();
        drop(writer);

        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Ok(reopened) = AgentHistoryStore::load(directory.path(), "workspace-a".into())
                && reopened
                    .conversation(&id)
                    .and_then(|conversation| conversation.messages.first())
                    .is_some_and(|message| message.content == "latest snapshot")
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "latest queued snapshot was not flushed"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn failed_write_reports_error_and_keeps_prior_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let mut initial = store(directory.path(), "workspace-a");
        let id = initial.create_conversation("saved", 1);
        let mut writer = AgentHistoryWriter::new(directory.path().into(), initial).unwrap();
        let revision = writer.submit().unwrap();
        assert_eq!(
            wait_for_event(&writer),
            AgentHistoryWriteEvent::Saved(revision)
        );
        let mut next = AgentHistoryStore::load(directory.path(), "workspace-a".into()).unwrap();
        let second_id = next.create_conversation("not saved", 2);
        let obstruction = directory.path().join("file-blocking-history-dir");
        fs::write(&obstruction, b"not a directory").unwrap();
        let mut writer = AgentHistoryWriter::new(obstruction.join("history"), next).unwrap();
        let revision = writer.submit().unwrap();
        assert!(matches!(
            wait_for_event(&writer),
            AgentHistoryWriteEvent::Failed(_, _)
        ));
        assert!(writer.store().conversation(&second_id).is_some());
        let recovered = AgentHistoryStore::load(directory.path(), "workspace-a".into()).unwrap();
        assert!(recovered.conversation(&id).is_some());
        assert!(recovered.conversation(&second_id).is_none());
        let _ = revision;
    }

    #[test]
    fn workspaces_are_isolated_and_corrupt_snapshots_are_preserved() {
        let directory = tempfile::tempdir().unwrap();
        let mut first = store(directory.path(), "workspace-a");
        let id = first.create_conversation("private to workspace A", 1);
        let mut writer = AgentHistoryWriter::new(directory.path().into(), first).unwrap();
        let revision = writer.submit().unwrap();
        assert_eq!(
            wait_for_event(&writer),
            AgentHistoryWriteEvent::Saved(revision)
        );
        assert!(AgentHistoryStore::load(directory.path(), "workspace-b".into()).is_err());

        let latest_revision = snapshot_paths(directory.path())
            .unwrap()
            .last()
            .map(|(revision, _)| *revision)
            .unwrap();
        let corrupt = directory
            .path()
            .join(format!("snapshot-{:020}.json", latest_revision + 1));
        fs::write(&corrupt, b"corrupt").unwrap();
        let recovered = AgentHistoryStore::load(directory.path(), "workspace-a".into()).unwrap();
        assert!(recovered.conversation(&id).is_some());
        assert!(recovered.warning().is_some());
        assert_eq!(fs::read(corrupt).unwrap(), b"corrupt");
    }

    #[test]
    fn workspace_roots_get_separate_history_directories() {
        let config = tempfile::tempdir().unwrap();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let (first_dir, first_id) = workspace_location(config.path(), first.path()).unwrap();
        let (second_dir, second_id) = workspace_location(config.path(), second.path()).unwrap();
        assert_ne!(first_id, second_id);
        assert_ne!(first_dir, second_dir);
    }

    #[test]
    fn conversation_timestamps_are_readable_and_utc() {
        assert_eq!(format_timestamp(0), "1970-01-01 00:00 UTC");
        assert_eq!(format_timestamp(1_700_000_000), "2023-11-14 22:13 UTC");
    }

    #[test]
    fn tool_context_is_bounded_to_names_and_cannot_replay_calls() {
        let messages = vec![
            ProviderChatMessage {
                role: ChatRole::Assistant,
                content: String::new(),
                reasoning: Some("private reasoning must not persist".into()),
                tool_call_id: None,
                tool_calls: vec![ProviderToolCall {
                    id: Some("call-1".into()),
                    name: "read_file".into(),
                    arguments: serde_json::json!({"path": "secret/path"}),
                }],
            },
            ProviderChatMessage {
                role: ChatRole::Tool,
                content: "secret output omitted".into(),
                reasoning: None,
                tool_call_id: Some("call-1".into()),
                tool_calls: Vec::new(),
            },
        ];
        let names = tool_summaries_from_messages(&messages);
        assert_eq!(names, vec!["read_file"]);
        assert!(!names.join(" ").contains("secret"));
        let note = tool_context_message(&[AgentToolSummary {
            name: names[0].clone(),
        }])
        .unwrap();
        assert!(note.contains("outputs were not retained"));
        let provider = axiom_ai_provider::ProviderChatMessage {
            role: ChatRole::System,
            content: note,
            reasoning: None,
            tool_call_id: None,
            tool_calls: Vec::new(),
        };
        assert!(provider.tool_calls.is_empty());
        assert!(provider.tool_call_id.is_none());
    }

    #[test]
    fn sensitive_transcript_content_is_refused_without_writing_a_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = store(directory.path(), "workspace-a");
        let id = store.create_conversation("safe title", 1);
        assert!(
            store
                .replace_messages(
                    &id,
                    vec![AgentHistoryMessage {
                        id: 1,
                        role: AgentHistoryRole::User,
                        content: "password=should-not-persist".into(),
                    }],
                    2,
                )
                .is_err()
        );
        let mut writer = AgentHistoryWriter::new(directory.path().into(), store).unwrap();
        assert!(writer.submit().is_ok());
        let event = wait_for_event(&writer);
        assert!(matches!(event, AgentHistoryWriteEvent::Saved(_)));
        let reopened = AgentHistoryStore::load(directory.path(), "workspace-a".into()).unwrap();
        assert!(reopened.conversation(&id).unwrap().messages.is_empty());
        let persisted = fs::read(snapshot_paths(directory.path()).unwrap()[0].1.as_path()).unwrap();
        assert!(!String::from_utf8_lossy(&persisted).contains("should-not-persist"));
    }
}
