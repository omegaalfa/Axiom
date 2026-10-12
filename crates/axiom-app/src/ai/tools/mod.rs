//! Minimal, provider-independent Agent tool layer.

mod delete_file;
mod execute_command;
mod fetch_url;
mod find_files;
mod find_references;
mod find_symbol;
mod list_directory;
mod read_file;
mod search_text;
mod update_file;
mod write_file;

use axiom_agent::{MemoryResult, MemoryScope, MemoryService, MemoryWriteError};
use axiom_project::project_delete::ProjectDeleteCapability;
use axiom_project::project_directory::ProjectDirectoryCapability;
use axiom_project::project_read::{ProjectReadCapability, ReadFileRange};
use axiom_project::project_search::ProjectSearchCapability;
use axiom_project::project_update::{ProjectUpdateCapability, TextFileFingerprint};
use axiom_project::project_write::ProjectWriteCapability;
use serde_json::json;

pub(crate) use fetch_url::FetchUrlTool;

/// Provider-facing budget for fetched text. This is separate from axiom-web's
/// network and capability output limits because model context is smaller than
/// a safe HTTP response buffer.
pub(crate) const MAX_TOOL_CONTENT_BYTES: usize = 24 * 1024;
pub(crate) use delete_file::DeleteFileTool;
pub(crate) use find_files::FindFilesTool;
pub(crate) use find_references::FindReferencesTool;
pub(crate) use find_symbol::FindSymbolTool;
pub(crate) use list_directory::ListDirectoryTool;
pub(crate) use read_file::ReadFileTool;
pub(crate) use search_text::SearchTextTool;
pub(crate) use update_file::UpdateFileTool;
pub(crate) use write_file::WriteFileTool;

#[derive(Clone, Debug)]
pub(crate) enum TerminalCommandEvent {
    Started {
        command: String,
        cwd: Option<String>,
        profile: Option<String>,
    },
    Output {
        stderr: bool,
        text: String,
    },
    Finished,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ToolName {
    ReadFile,
    ListDirectory,
    SearchText,
    FindFiles,
    FindSymbol,
    FindReferences,
    FetchUrl,
    WriteFile,
    UpdateFile,
    DeleteFile,
    ExecuteCommand,
    MemorySearch,
    MemoryGet,
    MemoryRemember,
    Unknown(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolKind {
    ReadOnly,
    Mutating,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ToolArguments {
    ReadFile {
        path: String,
        range: Option<ReadFileRange>,
    },
    ListDirectory {
        path: String,
    },
    SearchText {
        query: String,
        path: Option<String>,
        file_pattern: Option<String>,
    },
    FindFiles {
        pattern: String,
    },
    FindSymbol {
        query: String,
        kind: Option<String>,
        limit: Option<usize>,
    },
    FindReferences {
        query: String,
        kind: Option<String>,
        limit: Option<usize>,
    },
    FetchUrl {
        url: String,
    },
    WriteFile {
        path: String,
        content: String,
    },
    UpdateFile {
        path: String,
        expected_fingerprint: String,
        content: String,
    },
    DeleteFile {
        path: String,
        expected_fingerprint: String,
    },
    ExecuteCommand {
        command: String,
        cwd: Option<String>,
        profile: Option<String>,
    },
    MemorySearch {
        query: String,
        limit: Option<usize>,
    },
    MemoryGet {
        id: String,
    },
    MemoryRemember {
        content: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToolRequest {
    pub(crate) name: ToolName,
    pub(crate) arguments: ToolArguments,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToolMetadata {
    pub(crate) path: String,
    pub(crate) bytes: usize,
    pub(crate) range: Option<ReadFileRange>,
    pub(crate) source_bytes: Option<usize>,
    pub(crate) truncated: bool,
    pub(crate) fingerprint: Option<TextFileFingerprint>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToolOutput {
    pub(crate) content: String,
    pub(crate) metadata: ToolMetadata,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ToolError {
    UnknownTool(String),
    InvalidArguments(String),
    InvalidPath(String),
    OutsideWorkspace(String),
    NotFound(String),
    Directory(String),
    NotDirectory(String),
    TooManyEntries {
        path: String,
        limit: usize,
    },
    TooLarge {
        path: String,
        bytes: u64,
        limit: u64,
    },
    InvalidRange {
        start_line: usize,
        end_line: usize,
    },
    Io {
        path: String,
        message: String,
    },
    UnsupportedEncoding(String),
    AlreadyExists(String),
    SymlinkNotAllowed(String),
    InvalidUrl,
    UnsupportedScheme(String),
    BlockedAddress(String),
    Timeout,
    UnsupportedContentType(String),
    HttpStatus(u16),
    Network(String),
    Cancelled,
    RedirectLimit,
    InvalidFingerprint,
    FingerprintMismatch,
    NotRegularFile(String),
    CurrentFileTooLarge {
        path: String,
        limit: usize,
        actual: usize,
    },
    MemoryWriteUnavailable,
    MemoryWriteRejected,
    MemoryWriteFailed,
    MemoryWriteTimeout,
    MemoryReadFailed,
    CommandInvalid(String),
    CommandCwd(String),
    CommandSpawn(String),
    CommandWait(String),
    CommandRead(String),
    CommandTimeout,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToolResult {
    pub(crate) tool: ToolName,
    pub(crate) result: Result<ToolOutput, ToolError>,
}

pub(crate) struct ToolRegistry {
    read_file: ReadFileTool,
    list_directory: ListDirectoryTool,
    search_text: SearchTextTool,
    find_files: FindFilesTool,
    find_symbol: FindSymbolTool,
    find_references: FindReferencesTool,
    fetch_url: FetchUrlTool,
    write_file: Option<WriteFileTool>,
    update_file: Option<UpdateFileTool>,
    delete_file: Option<DeleteFileTool>,
    workspace_root: std::path::PathBuf,
    terminal_events: Option<std::sync::mpsc::SyncSender<TerminalCommandEvent>>,
    memory: Option<Box<dyn MemoryService>>,
    memory_scope: MemoryScope,
}

pub(crate) const MEMORY_SEARCH_DEFAULT_LIMIT: usize = 5;
pub(crate) const MEMORY_SEARCH_MAX_LIMIT: usize = 20;
pub(crate) const MEMORY_QUERY_MAX_CHARS: usize = 256;
pub(crate) const MEMORY_SEARCH_RESULT_MAX_CHARS: usize = 1024;
pub(crate) const MEMORY_GET_RESULT_MAX_CHARS: usize = 3500;
pub(crate) const MEMORY_OUTPUT_MAX_CHARS: usize = 4096;

impl ToolRegistry {
    #[cfg(test)]
    pub(crate) fn new(
        read_capability: ProjectReadCapability,
        directory_capability: ProjectDirectoryCapability,
    ) -> Self {
        Self::new_with_fetch_url(
            read_capability,
            directory_capability,
            axiom_web::FetchUrlCapability::new(),
        )
    }

    pub(crate) fn new_with_fetch_url(
        read_capability: ProjectReadCapability,
        directory_capability: ProjectDirectoryCapability,
        fetch_url_capability: axiom_web::FetchUrlCapability,
    ) -> Self {
        let workspace_root = read_capability.workspace_root().to_path_buf();
        let search = ProjectSearchCapability::new(read_capability.workspace_root())
            .expect("read capability workspace root must remain valid");
        let find_files = axiom_project::project_find_files::ProjectFindFilesCapability::new(
            read_capability.workspace_root(),
        )
        .expect("read capability workspace root must remain valid");
        Self {
            read_file: ReadFileTool::new(read_capability),
            list_directory: ListDirectoryTool::new(directory_capability),
            search_text: SearchTextTool::new(search),
            find_files: FindFilesTool::new(find_files),
            find_symbol: FindSymbolTool::new(workspace_root.clone(), None),
            find_references: FindReferencesTool::new(workspace_root.clone(), None, None),
            fetch_url: FetchUrlTool::new(fetch_url_capability),
            write_file: None,
            update_file: None,
            delete_file: None,
            workspace_root,
            terminal_events: None,
            memory: None,
            memory_scope: MemoryScope::Workspace,
        }
    }

    pub(crate) fn with_symbol_index(
        mut self,
        workspace_root: std::path::PathBuf,
        index: Option<std::sync::Arc<std::sync::RwLock<axiom_index::ProjectSymbolIndex>>>,
    ) -> Self {
        self.find_symbol = FindSymbolTool::new(workspace_root.clone(), index.clone());
        self.find_references = FindReferencesTool::new(workspace_root, index, None);
        self
    }

    pub(crate) fn with_semantic_engine(
        mut self,
        engine: Option<std::sync::Arc<axiom_index::SemanticEngine>>,
    ) -> Self {
        self.find_references = self.find_references.with_semantic_engine(engine);
        self
    }

    #[cfg(test)]
    pub(crate) fn new_with_write_file(
        read_capability: ProjectReadCapability,
        directory_capability: ProjectDirectoryCapability,
        fetch_url_capability: axiom_web::FetchUrlCapability,
        write_capability: ProjectWriteCapability,
    ) -> Self {
        let workspace_root = read_capability.workspace_root().to_path_buf();
        let search = ProjectSearchCapability::new(read_capability.workspace_root())
            .expect("read capability workspace root must remain valid");
        let find_files = axiom_project::project_find_files::ProjectFindFilesCapability::new(
            read_capability.workspace_root(),
        )
        .expect("read capability workspace root must remain valid");
        Self {
            read_file: ReadFileTool::new(read_capability),
            list_directory: ListDirectoryTool::new(directory_capability),
            search_text: SearchTextTool::new(search),
            find_files: FindFilesTool::new(find_files),
            find_symbol: FindSymbolTool::new(workspace_root.clone(), None),
            find_references: FindReferencesTool::new(workspace_root.clone(), None, None),
            fetch_url: FetchUrlTool::new(fetch_url_capability),
            write_file: Some(WriteFileTool::new(write_capability)),
            update_file: None,
            delete_file: None,
            workspace_root,
            terminal_events: None,
            memory: None,
            memory_scope: MemoryScope::Workspace,
        }
    }

    pub(crate) fn new_with_mutations(
        read_capability: ProjectReadCapability,
        directory_capability: ProjectDirectoryCapability,
        fetch_url_capability: axiom_web::FetchUrlCapability,
        write_capability: ProjectWriteCapability,
        update_capability: ProjectUpdateCapability,
    ) -> Self {
        let workspace_root = read_capability.workspace_root().to_path_buf();
        let search = ProjectSearchCapability::new(read_capability.workspace_root())
            .expect("read capability workspace root must remain valid");
        let find_files = axiom_project::project_find_files::ProjectFindFilesCapability::new(
            read_capability.workspace_root(),
        )
        .expect("read capability workspace root must remain valid");
        Self {
            read_file: ReadFileTool::new(read_capability),
            list_directory: ListDirectoryTool::new(directory_capability),
            search_text: SearchTextTool::new(search),
            find_files: FindFilesTool::new(find_files),
            find_symbol: FindSymbolTool::new(workspace_root.clone(), None),
            find_references: FindReferencesTool::new(workspace_root.clone(), None, None),
            fetch_url: FetchUrlTool::new(fetch_url_capability),
            write_file: Some(WriteFileTool::new(write_capability)),
            update_file: Some(UpdateFileTool::new(update_capability)),
            delete_file: None,
            workspace_root,
            terminal_events: None,
            memory: None,
            memory_scope: MemoryScope::Workspace,
        }
    }

    pub(crate) fn new_with_all_mutations(
        read_capability: ProjectReadCapability,
        directory_capability: ProjectDirectoryCapability,
        fetch_url_capability: axiom_web::FetchUrlCapability,
        write_capability: ProjectWriteCapability,
        update_capability: ProjectUpdateCapability,
        delete_capability: ProjectDeleteCapability,
    ) -> Self {
        let mut registry = Self::new_with_mutations(
            read_capability,
            directory_capability,
            fetch_url_capability,
            write_capability,
            update_capability,
        );
        registry.delete_file = Some(DeleteFileTool::new(delete_capability));
        registry
    }

    #[allow(dead_code)]
    pub(crate) fn with_memory_service(self, memory: Box<dyn MemoryService>) -> Self {
        self.with_memory_service_in_scope(memory, MemoryScope::Workspace)
    }

    pub(crate) fn with_terminal_events(
        mut self,
        events: std::sync::mpsc::SyncSender<TerminalCommandEvent>,
    ) -> Self {
        self.terminal_events = Some(events);
        self
    }

    #[allow(dead_code)]
    pub(crate) fn with_memory_service_in_scope(
        mut self,
        memory: Box<dyn MemoryService>,
        scope: MemoryScope,
    ) -> Self {
        if memory.is_available() {
            self.memory = Some(memory);
            self.memory_scope = scope;
        }
        self
    }

    pub(crate) fn has_memory_tools(&self) -> bool {
        self.memory.is_some()
    }

    pub(crate) fn has_memory_write_tool(&self) -> bool {
        self.memory
            .as_ref()
            .is_some_and(|memory| memory.can_remember())
    }

    pub(crate) fn kind(&self, name: &ToolName) -> Option<ToolKind> {
        match name {
            ToolName::ReadFile
            | ToolName::ListDirectory
            | ToolName::SearchText
            | ToolName::FetchUrl => Some(ToolKind::ReadOnly),
            ToolName::FindFiles => Some(ToolKind::ReadOnly),
            ToolName::FindSymbol => Some(ToolKind::ReadOnly),
            ToolName::FindReferences => Some(ToolKind::ReadOnly),
            ToolName::WriteFile => self.write_file.is_some().then_some(ToolKind::Mutating),
            ToolName::UpdateFile => self.update_file.is_some().then_some(ToolKind::Mutating),
            ToolName::DeleteFile => self.delete_file.is_some().then_some(ToolKind::Mutating),
            ToolName::ExecuteCommand => Some(ToolKind::Mutating),
            ToolName::MemorySearch | ToolName::MemoryGet => {
                self.memory.is_some().then_some(ToolKind::ReadOnly)
            }
            ToolName::MemoryRemember => self
                .memory
                .as_ref()
                .is_some_and(|memory| memory.can_remember())
                .then_some(ToolKind::Mutating),
            ToolName::Unknown(_) => None,
        }
    }

    pub(crate) fn execute(&self, request: ToolRequest) -> ToolResult {
        self.execute_with_cancel(request, || false)
    }

    pub(crate) fn execute_with_cancel<F>(&self, request: ToolRequest, cancelled: F) -> ToolResult
    where
        F: Fn() -> bool,
    {
        match request {
            ToolRequest {
                name: ToolName::ReadFile,
                arguments: ToolArguments::ReadFile { path, range },
            } => self.read_file.execute(path, range),
            ToolRequest {
                name: ToolName::ListDirectory,
                arguments: ToolArguments::ListDirectory { path },
            } => self.list_directory.execute(path),
            ToolRequest {
                name: ToolName::SearchText,
                arguments:
                    ToolArguments::SearchText {
                        query,
                        path,
                        file_pattern,
                    },
            } => self
                .search_text
                .execute(query, path, file_pattern, cancelled),
            ToolRequest {
                name: ToolName::FindFiles,
                arguments: ToolArguments::FindFiles { pattern },
            } => self.find_files.execute(pattern, cancelled),
            ToolRequest {
                name: ToolName::FindSymbol,
                arguments: ToolArguments::FindSymbol { query, kind, limit },
            } => self.find_symbol.execute(query, kind, limit),
            ToolRequest {
                name: ToolName::FindReferences,
                arguments: ToolArguments::FindReferences { query, kind, limit },
            } => self.find_references.execute(query, kind, limit),
            ToolRequest {
                name: ToolName::FetchUrl,
                arguments: ToolArguments::FetchUrl { url },
            } => self.fetch_url.execute(url, cancelled),
            ToolRequest {
                name: ToolName::WriteFile,
                arguments: ToolArguments::WriteFile { path, content },
            } => {
                if cancelled() {
                    return ToolResult {
                        tool: ToolName::WriteFile,
                        result: Err(ToolError::Cancelled),
                    };
                }
                match self.write_file.as_ref() {
                    Some(write_file) => write_file.execute(path, content),
                    None => ToolResult {
                        tool: ToolName::WriteFile,
                        result: Err(ToolError::UnknownTool("write_file".into())),
                    },
                }
            }
            ToolRequest {
                name: ToolName::UpdateFile,
                arguments:
                    ToolArguments::UpdateFile {
                        path,
                        expected_fingerprint,
                        content,
                    },
            } => {
                if cancelled() {
                    return ToolResult {
                        tool: ToolName::UpdateFile,
                        result: Err(ToolError::Cancelled),
                    };
                }
                match self.update_file.as_ref() {
                    Some(update_file) => update_file.execute(path, expected_fingerprint, content),
                    None => ToolResult {
                        tool: ToolName::UpdateFile,
                        result: Err(ToolError::UnknownTool("update_file".into())),
                    },
                }
            }
            ToolRequest {
                name: ToolName::DeleteFile,
                arguments:
                    ToolArguments::DeleteFile {
                        path,
                        expected_fingerprint,
                    },
            } => {
                if cancelled() {
                    return ToolResult {
                        tool: ToolName::DeleteFile,
                        result: Err(ToolError::Cancelled),
                    };
                }
                match self.delete_file.as_ref() {
                    Some(delete_file) => delete_file.execute(path, expected_fingerprint),
                    None => ToolResult {
                        tool: ToolName::DeleteFile,
                        result: Err(ToolError::UnknownTool("delete_file".into())),
                    },
                }
            }
            ToolRequest {
                name: ToolName::ExecuteCommand,
                arguments: ToolArguments::ExecuteCommand {
                    command,
                    cwd,
                    profile,
                },
            } => {
                if let Some(events) = &self.terminal_events {
                    let _ = events.try_send(TerminalCommandEvent::Started {
                        command: command.clone(),
                        cwd: cwd.clone(),
                        profile: profile.clone(),
                    });
                }
                let sink = self.terminal_events.as_ref().map(|events| {
                    let events = events.clone();
                    std::sync::Arc::new(move |stderr, text| {
                        let _ = events.try_send(TerminalCommandEvent::Output { stderr, text });
                    }) as std::sync::Arc<dyn Fn(bool, String) + Send + Sync>
                });
                let result = match sink {
                    Some(sink) => execute_command::execute_with_sink(
                        &self.workspace_root,
                        command,
                        cwd,
                        profile,
                        sink,
                    ),
                    None => execute_command::execute(&self.workspace_root, command, cwd, profile),
                };
                if let Some(events) = &self.terminal_events {
                    let _ = events.try_send(TerminalCommandEvent::Finished);
                }
                match result {
                Ok(content) => memory_success(ToolName::ExecuteCommand, content),
                Err(error) => ToolResult {
                    tool: ToolName::ExecuteCommand,
                    result: Err(match error {
                        execute_command::ExecuteCommandError::Invalid(message) => {
                            ToolError::CommandInvalid(message)
                        }
                        execute_command::ExecuteCommandError::Cwd(message) => {
                            ToolError::CommandCwd(message)
                        }
                        execute_command::ExecuteCommandError::Spawn(message) => {
                            ToolError::CommandSpawn(message)
                        }
                        execute_command::ExecuteCommandError::Wait(message) => {
                            ToolError::CommandWait(message)
                        }
                        execute_command::ExecuteCommandError::Read(message) => {
                            ToolError::CommandRead(message)
                        }
                        execute_command::ExecuteCommandError::Timeout => ToolError::CommandTimeout,
                    }),
                },
                }
            }
            ToolRequest {
                name: ToolName::MemorySearch,
                arguments: ToolArguments::MemorySearch { query, limit },
            } => self.memory_search(query, limit),
            ToolRequest {
                name: ToolName::MemoryGet,
                arguments: ToolArguments::MemoryGet { id },
            } => self.memory_get(id),
            ToolRequest {
                name: ToolName::MemoryRemember,
                arguments: ToolArguments::MemoryRemember { content },
            } => {
                if cancelled() {
                    return ToolResult {
                        tool: ToolName::MemoryRemember,
                        result: Err(ToolError::Cancelled),
                    };
                }
                self.memory_remember(content)
            }
            ToolRequest {
                name: ToolName::Unknown(name),
                ..
            } => ToolResult {
                tool: ToolName::Unknown(name.clone()),
                result: Err(ToolError::UnknownTool(name)),
            },
            ToolRequest { name, .. } => ToolResult {
                tool: name,
                result: Err(ToolError::InvalidArguments(
                    "tool arguments do not match the tool name".into(),
                )),
            },
        }
    }

    fn memory_search(&self, query: String, limit: Option<usize>) -> ToolResult {
        let query = normalize_memory_query(&query);
        if query.is_empty() {
            return memory_error(ToolName::MemorySearch, "query must not be empty");
        }
        let Some(memory) = self.memory.as_ref() else {
            return memory_error(ToolName::MemorySearch, "memory unavailable");
        };
        let limit = limit
            .unwrap_or(MEMORY_SEARCH_DEFAULT_LIMIT)
            .min(MEMORY_SEARCH_MAX_LIMIT);
        let results = match memory.query_checked(self.memory_scope, &query, limit) {
            Ok(results) => results,
            Err(_) => {
                return ToolResult {
                    tool: ToolName::MemorySearch,
                    result: Err(ToolError::MemoryReadFailed),
                };
            }
        }
        .into_iter()
        .filter(|result| result.scope == self.memory_scope)
        .collect();
        memory_success(
            ToolName::MemorySearch,
            format_memory_results(results, Some(&query), MEMORY_SEARCH_RESULT_MAX_CHARS),
        )
    }

    fn memory_get(&self, id: String) -> ToolResult {
        let id = id.trim();
        if id.is_empty() || id.chars().count() > MEMORY_QUERY_MAX_CHARS {
            return memory_error(ToolName::MemoryGet, "id is invalid or too long");
        }
        let Some(memory) = self.memory.as_ref() else {
            return memory_error(ToolName::MemoryGet, "memory unavailable");
        };
        let result = match memory.get_checked(self.memory_scope, id) {
            Ok(result) => result,
            Err(_) => {
                return ToolResult {
                    tool: ToolName::MemoryGet,
                    result: Err(ToolError::MemoryReadFailed),
                };
            }
        };
        memory_success(
            ToolName::MemoryGet,
            format_memory_results(
                result.into_iter().collect(),
                None,
                MEMORY_GET_RESULT_MAX_CHARS,
            ),
        )
    }

    fn memory_remember(&self, content: String) -> ToolResult {
        let Some(memory) = self.memory.as_ref() else {
            return ToolResult {
                tool: ToolName::MemoryRemember,
                result: Err(ToolError::MemoryWriteUnavailable),
            };
        };
        if content.chars().count() > axiom_agent::MAX_RECORD_CONTENT_CHARS {
            return ToolResult {
                tool: ToolName::MemoryRemember,
                result: Err(ToolError::MemoryWriteRejected),
            };
        }
        match memory.remember(self.memory_scope, &content) {
            Ok(()) => memory_success(
                ToolName::MemoryRemember,
                json!({
                    "persisted": true,
                    "scope": match self.memory_scope {
                        MemoryScope::User => "user",
                        MemoryScope::Workspace => "workspace",
                        MemoryScope::Project => "project",
                    },
                    "message": "Observation persisted to workspace memory."
                })
                .to_string(),
            ),
            Err(MemoryWriteError::Unavailable) => ToolResult {
                tool: ToolName::MemoryRemember,
                result: Err(ToolError::MemoryWriteUnavailable),
            },
            Err(MemoryWriteError::Rejected) => ToolResult {
                tool: ToolName::MemoryRemember,
                result: Err(ToolError::MemoryWriteRejected),
            },
            Err(MemoryWriteError::StorageFailure) => ToolResult {
                tool: ToolName::MemoryRemember,
                result: Err(ToolError::MemoryWriteFailed),
            },
            Err(MemoryWriteError::Timeout) => ToolResult {
                tool: ToolName::MemoryRemember,
                result: Err(ToolError::MemoryWriteTimeout),
            },
        }
    }
}

fn normalize_memory_query(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MEMORY_QUERY_MAX_CHARS)
        .collect()
}

fn relevant_memory_excerpt(content: &str, query: &str, max_chars: usize) -> String {
    let chars = content.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return content.to_owned();
    }

    let query_chars = query.chars().collect::<Vec<_>>();
    let match_start = if query_chars.is_empty() || query_chars.len() > chars.len() {
        0
    } else {
        chars
            .windows(query_chars.len())
            .position(|window| {
                window
                    .iter()
                    .zip(&query_chars)
                    .all(|(left, right)| left.eq_ignore_ascii_case(right))
            })
            .unwrap_or(0)
    };
    let context_before = max_chars.saturating_sub(query_chars.len()) / 2;
    let start = match_start.saturating_sub(context_before);
    let end = (start + max_chars).min(chars.len());
    let mut excerpt = String::new();
    if start > 0 {
        excerpt.push('…');
    }
    excerpt.extend(chars[start..end].iter());
    if end < chars.len() {
        excerpt.push('…');
    }
    excerpt
}

fn compact_memory_result(
    result: MemoryResult,
    query: Option<&str>,
    max_chars: usize,
) -> serde_json::Value {
    let content = match query {
        Some(query) => relevant_memory_excerpt(&result.content, query, max_chars),
        None => result.content.chars().take(max_chars).collect::<String>(),
    };
    json!({
        "id": result.id,
        "scope": match result.scope {
            MemoryScope::User => "user",
            MemoryScope::Workspace => "workspace",
            MemoryScope::Project => "project",
        },
        "content": content,
    })
}

fn format_memory_results(
    results: Vec<MemoryResult>,
    query: Option<&str>,
    max_result_chars: usize,
) -> String {
    let warning =
        "Memory is historical and may be stale or incomplete; verify current project state.";
    let mut items = Vec::new();
    for result in results {
        items.push(compact_memory_result(result, query, max_result_chars));
        let candidate = json!({
            "historical": true,
            "warning": warning,
            "results": items,
        })
        .to_string();
        if candidate.chars().count() > MEMORY_OUTPUT_MAX_CHARS {
            items.pop();
            break;
        }
    }
    json!({
        "historical": true,
        "warning": warning,
        "results": items,
    })
    .to_string()
}

fn memory_success(tool: ToolName, content: String) -> ToolResult {
    ToolResult {
        tool,
        result: Ok(ToolOutput {
            content,
            metadata: empty_tool_metadata(),
        }),
    }
}

fn memory_error(tool: ToolName, message: &str) -> ToolResult {
    ToolResult {
        tool,
        result: Ok(ToolOutput {
            content: json!({
                "historical": true,
                "available": false,
                "results": [],
                "message": message,
            })
            .to_string(),
            metadata: empty_tool_metadata(),
        }),
    }
}

fn empty_tool_metadata() -> ToolMetadata {
    ToolMetadata {
        path: String::new(),
        bytes: 0,
        range: None,
        source_bytes: None,
        truncated: false,
        fingerprint: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axiom_agent::{
        AxiomMemoryBackend, AxiomMemoryScopeIds, InMemoryMemoryService, MemoryObservation,
        MemoryService,
    };
    use axiom_project::project_read::ProjectReadCapability;
    use axiom_project::project_update::ProjectUpdateCapability;
    use axiom_project::project_write::ProjectWriteCapability;
    use std::fs;
    use tempfile::tempdir;

    fn registry() -> (tempfile::TempDir, ToolRegistry) {
        let dir = tempdir().unwrap();
        let read = ProjectReadCapability::new(dir.path()).unwrap();
        let directory = ProjectDirectoryCapability::new(dir.path()).unwrap();
        (dir, ToolRegistry::new(read, directory))
    }

    fn mutation_registry() -> (tempfile::TempDir, ToolRegistry) {
        let dir = tempdir().unwrap();
        let read = ProjectReadCapability::new(dir.path()).unwrap();
        let directory = ProjectDirectoryCapability::new(dir.path()).unwrap();
        let write = ProjectWriteCapability::new(dir.path()).unwrap();
        (
            dir,
            ToolRegistry::new_with_write_file(
                read,
                directory,
                axiom_web::FetchUrlCapability::new(),
                write,
            ),
        )
    }

    fn update_registry() -> (tempfile::TempDir, ToolRegistry) {
        let dir = tempdir().unwrap();
        let read = ProjectReadCapability::new(dir.path()).unwrap();
        let directory = ProjectDirectoryCapability::new(dir.path()).unwrap();
        let write = ProjectWriteCapability::new(dir.path()).unwrap();
        let update = ProjectUpdateCapability::new(dir.path()).unwrap();
        (
            dir,
            ToolRegistry::new_with_mutations(
                read,
                directory,
                axiom_web::FetchUrlCapability::new(),
                write,
                update,
            ),
        )
    }

    fn request(path: &str) -> ToolRequest {
        ToolRequest {
            name: ToolName::ReadFile,
            arguments: ToolArguments::ReadFile {
                path: path.into(),
                range: None,
            },
        }
    }

    #[test]
    fn registry_knows_read_file_as_read_only() {
        let (_dir, registry) = registry();
        assert_eq!(registry.kind(&ToolName::ReadFile), Some(ToolKind::ReadOnly));
        assert_eq!(
            registry.kind(&ToolName::ListDirectory),
            Some(ToolKind::ReadOnly)
        );
        assert_eq!(registry.kind(&ToolName::FetchUrl), Some(ToolKind::ReadOnly));
        assert_eq!(registry.kind(&ToolName::WriteFile), None);
    }

    #[test]
    fn memory_tools_are_optional_and_writes_require_a_writable_backend() {
        let (_dir, empty_registry) = registry();
        assert!(!empty_registry.has_memory_tools());
        assert_eq!(empty_registry.kind(&ToolName::MemorySearch), None);
        assert_eq!(empty_registry.kind(&ToolName::MemoryGet), None);

        let unavailable = registry()
            .1
            .with_memory_service(Box::new(axiom_agent::NoMemory));
        assert!(!unavailable.has_memory_tools());
        assert_eq!(unavailable.kind(&ToolName::MemorySearch), None);
        assert_eq!(unavailable.kind(&ToolName::MemoryGet), None);

        let mut memory = InMemoryMemoryService::default();
        let session = memory.session_start(MemoryScope::Project);
        memory.observe(
            session,
            MemoryObservation::Discovery("project-only fact".into()),
        );
        memory.session_end(session);
        let registry = registry()
            .1
            .with_memory_service_in_scope(Box::new(memory), MemoryScope::Project);
        assert_eq!(
            registry.kind(&ToolName::MemorySearch),
            Some(ToolKind::ReadOnly)
        );
        assert_eq!(
            registry.kind(&ToolName::MemoryGet),
            Some(ToolKind::ReadOnly)
        );
        assert_eq!(registry.kind(&ToolName::MemoryRemember), None);
    }

    #[test]
    fn memory_search_is_scoped_normalized_and_bounded() {
        let mut memory = InMemoryMemoryService::default();
        let project = memory.session_start(MemoryScope::Project);
        for index in 0..40 {
            memory.observe(
                project,
                MemoryObservation::Discovery(format!("project fact {index} {}", "x".repeat(900))),
            );
        }
        memory.session_end(project);
        let workspace = memory.session_start(MemoryScope::Workspace);
        memory.observe(
            workspace,
            MemoryObservation::Discovery("workspace-only fact".into()),
        );
        memory.session_end(workspace);

        let registry = registry()
            .1
            .with_memory_service_in_scope(Box::new(memory), MemoryScope::Project);
        let result = registry.execute(ToolRequest {
            name: ToolName::MemorySearch,
            arguments: ToolArguments::MemorySearch {
                query: "  project   fact  ".into(),
                limit: Some(usize::MAX),
            },
        });
        let ToolResult {
            result: Ok(output), ..
        } = result
        else {
            panic!("memory search must return a normalized result");
        };
        assert!(output.content.chars().count() <= MEMORY_OUTPUT_MAX_CHARS);
        assert!(!output.content.contains("workspace-only fact"));
        let payload: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(payload["historical"], true);
        assert!(payload["results"].as_array().unwrap().len() <= MEMORY_SEARCH_MAX_LIMIT);
    }

    #[test]
    fn memory_search_rejects_empty_query_and_memory_get_is_bounded() {
        let mut memory = InMemoryMemoryService::default();
        let session = memory.session_start(MemoryScope::Workspace);
        memory.observe(
            session,
            MemoryObservation::Decision("known decision".into()),
        );
        memory.session_end(session);
        let id = memory.briefing(MemoryScope::Workspace, 1)[0]
            .id
            .clone()
            .unwrap();
        let registry = registry().1.with_memory_service(Box::new(memory));

        let empty = registry.execute(ToolRequest {
            name: ToolName::MemorySearch,
            arguments: ToolArguments::MemorySearch {
                query: " \n\t ".into(),
                limit: None,
            },
        });
        assert!(
            matches!(empty.result, Ok(output) if output.content.contains("query must not be empty"))
        );

        let found = registry.execute(ToolRequest {
            name: ToolName::MemoryGet,
            arguments: ToolArguments::MemoryGet { id },
        });
        assert!(matches!(found.result, Ok(output) if output.content.contains("known decision")));

        let too_long = registry.execute(ToolRequest {
            name: ToolName::MemoryGet,
            arguments: ToolArguments::MemoryGet {
                id: "x".repeat(MEMORY_QUERY_MAX_CHARS + 1),
            },
        });
        assert!(matches!(too_long.result, Ok(output) if output.content.contains("too long")));
    }

    #[test]
    fn memory_tools_report_read_failure_instead_of_empty_results() {
        let dir = tempdir().unwrap();
        let ids = AxiomMemoryScopeIds::new("user", "workspace-read-error", "project");
        let backend = AxiomMemoryBackend::new(dir.path(), ids.clone());
        backend.initialize_storage().unwrap();
        std::fs::write(
            dir.path()
                .join("workspace")
                .join(format!("{}.json", ids.workspace)),
            b"corrupt",
        )
        .unwrap();
        let (_workspace, registry) = registry();
        let registry =
            registry.with_memory_service_in_scope(Box::new(backend), MemoryScope::Workspace);

        let search = registry.execute(ToolRequest {
            name: ToolName::MemorySearch,
            arguments: ToolArguments::MemorySearch {
                query: "some fact".into(),
                limit: None,
            },
        });
        assert_eq!(search.result, Err(ToolError::MemoryReadFailed));
        let get = registry.execute(ToolRequest {
            name: ToolName::MemoryGet,
            arguments: ToolArguments::MemoryGet {
                id: "workspace-read-error:1".into(),
            },
        });
        assert_eq!(get.result, Err(ToolError::MemoryReadFailed));
    }

    #[test]
    fn memory_search_and_get_recover_architecture_details() {
        let architecture = format!(
            "{}\naxiom-app: GPUI application shell and user interface.\n\
             axiom-syntax: Tree-sitter parsing, tokens, syntax highlighting, and diagnostics.\n\
             axiom-project: project lifecycle, filesystem model, Composer metadata, and PSR-4 resolution.",
            "Axiom architecture context. ".repeat(18)
        );
        let mut memory = InMemoryMemoryService::default();
        let session = memory.session_start(MemoryScope::Workspace);
        memory.observe(session, MemoryObservation::Discovery(architecture.clone()));
        memory.session_end(session);
        let id = memory.briefing(MemoryScope::Workspace, 1)[0]
            .id
            .clone()
            .unwrap();
        let registry = registry()
            .1
            .with_memory_service_in_scope(Box::new(memory), MemoryScope::Workspace);

        let search = registry.execute(ToolRequest {
            name: ToolName::MemorySearch,
            arguments: ToolArguments::MemorySearch {
                query: "axiom-syntax".into(),
                limit: Some(1),
            },
        });
        let ToolResult {
            result: Ok(search_output),
            ..
        } = search
        else {
            panic!("memory search must return the matching architecture record");
        };
        assert!(search_output.content.chars().count() <= MEMORY_OUTPUT_MAX_CHARS);
        for detail in [
            "axiom-app: GPUI application shell and user interface",
            "axiom-syntax: Tree-sitter parsing, tokens, syntax highlighting, and diagnostics",
            "axiom-project: project lifecycle, filesystem model, Composer metadata, and PSR-4 resolution",
        ] {
            assert!(search_output.content.contains(detail), "missing {detail}");
        }

        let get = registry.execute(ToolRequest {
            name: ToolName::MemoryGet,
            arguments: ToolArguments::MemoryGet { id },
        });
        let ToolResult {
            result: Ok(get_output),
            ..
        } = get
        else {
            panic!("memory_get must return the full architecture record");
        };
        assert!(get_output.content.chars().count() <= MEMORY_OUTPUT_MAX_CHARS);
        let payload: serde_json::Value = serde_json::from_str(&get_output.content).unwrap();
        let expected = format!(
            "discovery: {}",
            architecture
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        );
        assert_eq!(payload["results"][0]["content"], expected);
        for detail in [
            "axiom-app: GPUI application shell and user interface",
            "axiom-syntax: Tree-sitter parsing, tokens, syntax highlighting, and diagnostics",
            "axiom-project: project lifecycle, filesystem model, Composer metadata, and PSR-4 resolution",
        ] {
            assert!(get_output.content.contains(detail), "missing {detail}");
        }
    }

    #[test]
    fn write_file_is_mutating_and_creates_utf8_content() {
        let (dir, registry) = mutation_registry();
        fs::create_dir(dir.path().join("src")).unwrap();
        assert_eq!(
            registry.kind(&ToolName::WriteFile),
            Some(ToolKind::Mutating)
        );
        let result = registry.execute(ToolRequest {
            name: ToolName::WriteFile,
            arguments: ToolArguments::WriteFile {
                path: "src/new.txt".into(),
                content: "Olá, Axiom! 🚀".into(),
            },
        });
        assert!(matches!(result.result, Ok(ToolOutput { .. })));
        assert_eq!(
            fs::read_to_string(dir.path().join("src/new.txt")).unwrap(),
            "Olá, Axiom! 🚀"
        );
    }

    #[test]
    fn write_file_rejects_existing_destination_without_overwriting() {
        let (dir, registry) = mutation_registry();
        fs::write(dir.path().join("existing.txt"), "original").unwrap();
        let result = registry.execute(ToolRequest {
            name: ToolName::WriteFile,
            arguments: ToolArguments::WriteFile {
                path: "existing.txt".into(),
                content: "replacement".into(),
            },
        });
        assert!(matches!(result.result, Err(ToolError::AlreadyExists(_))));
        assert_eq!(
            fs::read_to_string(dir.path().join("existing.txt")).unwrap(),
            "original"
        );
    }

    #[test]
    fn update_file_requires_matching_fingerprint_and_preserves_utf8() {
        let (dir, registry) = update_registry();
        fs::write(dir.path().join("file.txt"), "old").unwrap();
        let expected = ProjectUpdateCapability::new(dir.path())
            .unwrap()
            .fingerprint_text_file("file.txt")
            .unwrap()
            .to_wire_string();
        assert_eq!(
            registry.kind(&ToolName::UpdateFile),
            Some(ToolKind::Mutating)
        );
        let result = registry.execute(ToolRequest {
            name: ToolName::UpdateFile,
            arguments: ToolArguments::UpdateFile {
                path: "file.txt".into(),
                expected_fingerprint: expected,
                content: "Olá, Axiom! 🚀".into(),
            },
        });
        assert!(matches!(result.result, Ok(ToolOutput { .. })));
        assert_eq!(
            fs::read_to_string(dir.path().join("file.txt")).unwrap(),
            "Olá, Axiom! 🚀"
        );
    }

    #[test]
    fn update_file_rejects_stale_and_malformed_fingerprints_without_mutation() {
        let (dir, registry) = update_registry();
        fs::write(dir.path().join("file.txt"), "current").unwrap();
        for expected_fingerprint in [
            "sha256:0000000000000000000000000000000000000000000000000000000000000000".into(),
            "malformed".into(),
        ] {
            let result = registry.execute(ToolRequest {
                name: ToolName::UpdateFile,
                arguments: ToolArguments::UpdateFile {
                    path: "file.txt".into(),
                    expected_fingerprint,
                    content: "replacement".into(),
                },
            });
            assert!(matches!(
                result.result,
                Err(ToolError::FingerprintMismatch | ToolError::InvalidFingerprint)
            ));
        }
        assert_eq!(
            fs::read_to_string(dir.path().join("file.txt")).unwrap(),
            "current"
        );
    }

    #[test]
    fn update_file_conflict_preserves_newer_content_and_missing_file_is_not_created() {
        let (dir, registry) = update_registry();
        fs::write(dir.path().join("file.txt"), "old").unwrap();
        let expected = ProjectUpdateCapability::new(dir.path())
            .unwrap()
            .fingerprint_text_file("file.txt")
            .unwrap()
            .to_wire_string();
        fs::write(dir.path().join("file.txt"), "newer").unwrap();
        let result = registry.execute(ToolRequest {
            name: ToolName::UpdateFile,
            arguments: ToolArguments::UpdateFile {
                path: "file.txt".into(),
                expected_fingerprint: expected,
                content: "stale replacement".into(),
            },
        });
        assert!(matches!(result.result, Err(ToolError::FingerprintMismatch)));
        assert_eq!(
            fs::read_to_string(dir.path().join("file.txt")).unwrap(),
            "newer"
        );

        let missing =
            registry.execute(ToolRequest {
                name: ToolName::UpdateFile,
                arguments: ToolArguments::UpdateFile {
                    path: "missing.txt".into(),
                    expected_fingerprint:
                        "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                            .into(),
                    content: "must not be created".into(),
                },
            });
        assert!(matches!(missing.result, Err(ToolError::NotFound(_))));
        assert!(!dir.path().join("missing.txt").exists());
    }

    #[test]
    fn read_file_adapter_delegates_content_range_and_metadata() {
        let (dir, registry) = registry();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/Test.php"), "one\ntwo\nthree\n").unwrap();
        let result = registry.execute(ToolRequest {
            name: ToolName::ReadFile,
            arguments: ToolArguments::ReadFile {
                path: "src/Test.php".into(),
                range: Some(ReadFileRange {
                    start_line: 2,
                    end_line: 3,
                }),
            },
        });
        let output = result.result.unwrap();
        assert_eq!(output.content, "two\nthree\n");
        assert_eq!(output.metadata.path, "src/Test.php");
        assert_eq!(
            output.metadata.range,
            Some(ReadFileRange {
                start_line: 2,
                end_line: 3
            })
        );
    }

    #[test]
    fn capability_errors_are_mapped_without_collapsing_categories() {
        let (dir, registry) = registry();
        let result = registry.execute(request("missing.txt"));
        assert!(matches!(result.result, Err(ToolError::NotFound(_))));
        fs::create_dir(dir.path().join("App")).unwrap();
        let result = registry.execute(request("App"));
        assert!(matches!(result.result, Err(ToolError::Directory(path)) if path == "App"));
    }

    #[test]
    fn list_directory_adapter_returns_deterministic_structured_content() {
        let (dir, registry) = registry();
        fs::create_dir(dir.path().join("App")).unwrap();
        fs::write(dir.path().join("App/Z.php"), "z").unwrap();
        fs::write(dir.path().join("App/A.php"), "a").unwrap();
        let result = registry.execute(ToolRequest {
            name: ToolName::ListDirectory,
            arguments: ToolArguments::ListDirectory { path: "App".into() },
        });
        let output = result.result.unwrap();
        assert!(output.content.contains("A.php"));
        assert!(output.content.contains("Z.php"));
        assert_eq!(output.metadata.path, "App");
    }

    #[test]
    fn list_directory_adapter_preserves_nested_path_and_root_distinction() {
        let (dir, registry) = registry();
        fs::create_dir(dir.path().join("App")).unwrap();
        fs::create_dir_all(dir.path().join("src/App")).unwrap();
        fs::write(dir.path().join("App/Root.php"), "root").unwrap();
        fs::write(dir.path().join("src/App/FileStone.php"), "nested").unwrap();

        let src = registry.execute(ToolRequest {
            name: ToolName::ListDirectory,
            arguments: ToolArguments::ListDirectory { path: "src".into() },
        });
        let src_output = src.result.unwrap();
        assert_eq!(src_output.metadata.path, "src");
        assert!(src_output.content.contains("\"name\":\"App\""));
        assert!(!src_output.content.contains("FileStone.php"));

        let nested = registry.execute(ToolRequest {
            name: ToolName::ListDirectory,
            arguments: ToolArguments::ListDirectory {
                path: "src/App".into(),
            },
        });
        let nested_output = nested.result.unwrap();
        assert_eq!(nested_output.metadata.path, "src/App");
        assert!(nested_output.content.contains("FileStone.php"));
        assert!(!nested_output.content.contains("Root.php"));
    }

    #[test]
    fn unknown_tool_is_a_typed_error() {
        let (_dir, registry) = registry();
        let result = registry.execute(ToolRequest {
            name: ToolName::Unknown("list_files".into()),
            arguments: ToolArguments::ReadFile {
                path: "x".into(),
                range: None,
            },
        });
        assert_eq!(result.tool, ToolName::Unknown("list_files".into()));
        assert!(matches!(result.result, Err(ToolError::UnknownTool(name)) if name == "list_files"));
    }

    #[test]
    fn fetch_url_blocked_address_is_a_controlled_tool_error() {
        let (_dir, registry) = registry();
        let result = registry.execute(ToolRequest {
            name: ToolName::FetchUrl,
            arguments: ToolArguments::FetchUrl {
                url: "http://127.0.0.1/".into(),
            },
        });
        assert!(matches!(result.result, Err(ToolError::BlockedAddress(_))));
    }
}
