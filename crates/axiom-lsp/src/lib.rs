//! Generic stdio Language Server Protocol client.

use std::{
    collections::HashMap,
    env, fmt,
    io::{self, BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Duration,
};

use lsp_types::{
    ClientCapabilities, CompletionItem, CompletionResponse, Diagnostic,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, DocumentFormattingParams, FormattingOptions, GotoDefinitionResponse,
    Hover, InitializeParams, InitializedParams, Location, Position, PositionEncodingKind,
    PublishDiagnosticsParams, ReferenceContext, ReferenceParams, SignatureHelpParams,
    TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem,
    TextDocumentPositionParams, TextEdit, Uri, VersionedTextDocumentIdentifier, WorkspaceFolder,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

type ResponseSender = Sender<Result<Value, LspError>>;
type PendingRequests = Arc<Mutex<HashMap<i64, ResponseSender>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerStatus {
    Starting,
    Ready,
    Stopped,
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionEncoding {
    Utf8,
    Utf16,
    Utf32,
}

impl Default for PositionEncoding {
    fn default() -> Self {
        Self::Utf16
    }
}

impl PositionEncoding {
    fn from_lsp(value: Option<&PositionEncodingKind>) -> Self {
        match value.map(PositionEncodingKind::as_str) {
            Some("utf-8") => Self::Utf8,
            Some("utf-32") => Self::Utf32,
            _ => Self::Utf16,
        }
    }
}

pub struct PositionCodec;

impl PositionCodec {
    pub fn offset_to_position(text: &str, offset: usize, encoding: PositionEncoding) -> Position {
        let offset = floor_char_boundary(text, offset.min(text.len()));
        let prefix = &text[..offset];
        let line = prefix.bytes().filter(|byte| *byte == b'\n').count() as u32;
        let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);
        let character = units(&text[line_start..offset], encoding) as u32;
        Position::new(line, character)
    }

    pub fn position_to_offset(text: &str, position: Position, encoding: PositionEncoding) -> usize {
        let line_start = text
            .split_inclusive('\n')
            .take(position.line as usize)
            .map(str::len)
            .sum::<usize>()
            .min(text.len());
        let line_end = text[line_start..]
            .find('\n')
            .map_or(text.len(), |length| line_start + length);
        let line = &text[line_start..line_end];
        let target = position.character as usize;
        let mut consumed = 0;
        for (byte, ch) in line.char_indices() {
            let next = consumed + char_units(ch, encoding);
            if next > target {
                return line_start + byte;
            }
            consumed = next;
            if consumed == target {
                return line_start + byte + ch.len_utf8();
            }
        }
        line_end
    }
}

fn units(text: &str, encoding: PositionEncoding) -> usize {
    text.chars().map(|ch| char_units(ch, encoding)).sum()
}

fn char_units(ch: char, encoding: PositionEncoding) -> usize {
    match encoding {
        PositionEncoding::Utf8 => ch.len_utf8(),
        PositionEncoding::Utf16 => ch.len_utf16(),
        PositionEncoding::Utf32 => 1,
    }
}

fn floor_char_boundary(text: &str, mut offset: usize) -> usize {
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

#[derive(Debug)]
pub enum LspError {
    Io(io::Error),
    Json(serde_json::Error),
    Protocol(String),
    ServerNotFound,
    Timeout,
    Disconnected,
    Response { code: i64, message: String },
}

impl fmt::Display for LspError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Json(error) => error.fmt(f),
            Self::Protocol(message) => f.write_str(message),
            Self::ServerNotFound => f.write_str("language server executable not found"),
            Self::Timeout => f.write_str("language server request timed out"),
            Self::Disconnected => f.write_str("language server disconnected"),
            Self::Response { code, message } => {
                write!(f, "language server error {code}: {message}")
            }
        }
    }
}

impl std::error::Error for LspError {}

impl From<io::Error> for LspError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for LspError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

#[derive(Debug, Clone)]
pub enum ServerEvent {
    Diagnostics(PublishDiagnosticsParams),
    Log(String),
    Stopped,
}

pub struct PendingResponse {
    pub id: i64,
    receiver: Receiver<Result<Value, LspError>>,
}

impl PendingResponse {
    pub fn recv<T: DeserializeOwned>(self, timeout: Duration) -> Result<T, LspError> {
        let value = self
            .receiver
            .recv_timeout(timeout)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => LspError::Timeout,
                mpsc::RecvTimeoutError::Disconnected => LspError::Disconnected,
            })??;
        Ok(serde_json::from_value(value)?)
    }
}

pub struct LanguageServer {
    writer: Arc<Mutex<ChildStdin>>,
    child: Arc<Mutex<Child>>,
    pending: PendingRequests,
    next_id: AtomicI64,
    events: Receiver<ServerEvent>,
    status: Arc<Mutex<ServerStatus>>,
    encoding: PositionEncoding,
}

impl LanguageServer {
    pub fn detect_php_server() -> Option<PathBuf> {
        if let Some(configured) =
            env::var_os("AXIOM_PHP_LSP").or_else(|| env::var_os("RUSTSTORM_PHP_LSP"))
        {
            let path = PathBuf::from(configured);
            if path.is_file() {
                return Some(path);
            }
        }
        find_on_path(if cfg!(windows) {
            "intelephense.cmd"
        } else {
            "intelephense"
        })
    }

    pub fn start(executable: impl AsRef<Path>, args: &[&str]) -> Result<Self, LspError> {
        let mut child = Command::new(executable.as_ref())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| LspError::Protocol("missing server stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| LspError::Protocol("missing server stdout".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| LspError::Protocol("missing server stderr".into()))?;
        let pending = Arc::new(Mutex::new(
            HashMap::<i64, Sender<Result<Value, LspError>>>::new(),
        ));
        let status = Arc::new(Mutex::new(ServerStatus::Starting));
        let (event_tx, event_rx) = mpsc::channel();
        spawn_stdout_reader(stdout, pending.clone(), status.clone(), event_tx.clone());
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                tracing::debug!(target: "axiom_lsp", "server stderr: {line}");
                let _ = event_tx.send(ServerEvent::Log(line));
            }
        });
        Ok(Self {
            writer: Arc::new(Mutex::new(stdin)),
            child: Arc::new(Mutex::new(child)),
            pending,
            next_id: AtomicI64::new(1),
            events: event_rx,
            status,
            encoding: PositionEncoding::Utf16,
        })
    }

    pub fn start_intelephense() -> Result<Self, LspError> {
        let executable = Self::detect_php_server().ok_or(LspError::ServerNotFound)?;
        Self::start(executable, &["--stdio"])
    }

    #[allow(deprecated)]
    pub fn initialize(&mut self, root: &Path) -> Result<(), LspError> {
        let root_uri = path_to_uri(root)?;
        let params = InitializeParams {
            capabilities: ClientCapabilities {
                general: Some(lsp_types::GeneralClientCapabilities {
                    position_encodings: Some(vec![
                        PositionEncodingKind::UTF16,
                        PositionEncodingKind::UTF8,
                        PositionEncodingKind::UTF32,
                    ]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            root_uri: None,
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: root_uri,
                name: root
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("project")
                    .into(),
            }]),
            ..Default::default()
        };
        let result: lsp_types::InitializeResult = self
            .request("initialize", params)?
            .recv(DEFAULT_REQUEST_TIMEOUT)?;
        self.encoding = PositionEncoding::from_lsp(result.capabilities.position_encoding.as_ref());
        self.notify("initialized", InitializedParams {})?;
        *self.status.lock().expect("status lock poisoned") = ServerStatus::Ready;
        Ok(())
    }

    pub fn status(&self) -> ServerStatus {
        *self.status.lock().expect("status lock poisoned")
    }

    pub fn position_encoding(&self) -> PositionEncoding {
        self.encoding
    }

    pub fn try_event(&self) -> Option<ServerEvent> {
        self.events.try_recv().ok()
    }

    pub fn request(
        &self,
        method: &str,
        params: impl serde::Serialize,
    ) -> Result<PendingResponse, LspError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel();
        self.pending
            .lock()
            .expect("pending lock poisoned")
            .insert(id, sender);
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        if let Err(error) = self.write(&message) {
            self.pending
                .lock()
                .expect("pending lock poisoned")
                .remove(&id);
            return Err(error);
        }
        Ok(PendingResponse { id, receiver })
    }

    pub fn notify(&self, method: &str, params: impl serde::Serialize) -> Result<(), LspError> {
        self.write(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    pub fn did_open(&self, uri: Uri, version: i32, text: String) -> Result<(), LspError> {
        self.notify(
            "textDocument/didOpen",
            DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri,
                    language_id: "php".into(),
                    version,
                    text,
                },
            },
        )
    }

    pub fn did_change(&self, uri: Uri, version: i32, text: String) -> Result<(), LspError> {
        self.did_change_event(
            uri,
            version,
            TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text,
            },
        )
    }

    pub fn did_change_event(
        &self,
        uri: Uri,
        version: i32,
        change: TextDocumentContentChangeEvent,
    ) -> Result<(), LspError> {
        self.notify(
            "textDocument/didChange",
            DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier { uri, version },
                content_changes: vec![change],
            },
        )
    }

    pub fn did_save(&self, uri: Uri, text: Option<String>) -> Result<(), LspError> {
        self.notify(
            "textDocument/didSave",
            DidSaveTextDocumentParams {
                text_document: TextDocumentIdentifier { uri },
                text,
            },
        )
    }

    pub fn did_close(&self, uri: Uri) -> Result<(), LspError> {
        self.notify(
            "textDocument/didClose",
            DidCloseTextDocumentParams {
                text_document: TextDocumentIdentifier { uri },
            },
        )
    }

    pub fn completion(&self, uri: Uri, position: Position) -> Result<PendingResponse, LspError> {
        self.request(
            "textDocument/completion",
            TextDocumentPositionParams::new(TextDocumentIdentifier { uri }, position),
        )
    }

    pub fn hover(&self, uri: Uri, position: Position) -> Result<PendingResponse, LspError> {
        self.request(
            "textDocument/hover",
            TextDocumentPositionParams::new(TextDocumentIdentifier { uri }, position),
        )
    }

    pub fn definition(&self, uri: Uri, position: Position) -> Result<PendingResponse, LspError> {
        self.request(
            "textDocument/definition",
            TextDocumentPositionParams::new(TextDocumentIdentifier { uri }, position),
        )
    }

    pub fn references(&self, uri: Uri, position: Position) -> Result<PendingResponse, LspError> {
        self.request(
            "textDocument/references",
            ReferenceParams {
                text_document_position: TextDocumentPositionParams::new(
                    TextDocumentIdentifier { uri },
                    position,
                ),
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                context: ReferenceContext {
                    include_declaration: true,
                },
            },
        )
    }

    pub fn formatting(
        &self,
        uri: Uri,
        tab_size: u32,
        insert_spaces: bool,
    ) -> Result<PendingResponse, LspError> {
        self.request(
            "textDocument/formatting",
            DocumentFormattingParams {
                text_document: TextDocumentIdentifier { uri },
                work_done_progress_params: Default::default(),
                options: FormattingOptions {
                    tab_size,
                    insert_spaces,
                    ..Default::default()
                },
            },
        )
    }

    pub fn signature_help(
        &self,
        uri: Uri,
        position: Position,
    ) -> Result<PendingResponse, LspError> {
        self.request(
            "textDocument/signatureHelp",
            SignatureHelpParams {
                text_document_position_params: TextDocumentPositionParams::new(
                    TextDocumentIdentifier { uri },
                    position,
                ),
                work_done_progress_params: Default::default(),
                context: None,
            },
        )
    }

    pub fn shutdown(&self) -> Result<(), LspError> {
        let _: Value = self
            .request("shutdown", Value::Null)?
            .recv(DEFAULT_REQUEST_TIMEOUT)?;
        self.notify("exit", Value::Null)
    }

    pub fn wait(&self) -> Result<(), LspError> {
        self.child.lock().expect("child lock poisoned").wait()?;
        Ok(())
    }

    fn write(&self, value: &Value) -> Result<(), LspError> {
        write_message(
            &mut *self.writer.lock().expect("writer lock poisoned"),
            value,
        )
    }
}

fn spawn_stdout_reader(
    stdout: impl Read + Send + 'static,
    pending: PendingRequests,
    status: Arc<Mutex<ServerStatus>>,
    events: Sender<ServerEvent>,
) {
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            match read_message(&mut reader) {
                Ok(Some(message)) => dispatch_message(message, &pending, &events),
                Ok(None) => break,
                Err(error) => {
                    tracing::warn!(target: "axiom_lsp", "protocol error: {error}");
                    break;
                }
            }
        }
        *status.lock().expect("status lock poisoned") = ServerStatus::Stopped;
        for (_, sender) in pending.lock().expect("pending lock poisoned").drain() {
            let _ = sender.send(Err(LspError::Disconnected));
        }
        let _ = events.send(ServerEvent::Stopped);
    });
}

fn dispatch_message(
    message: Value,
    pending: &Mutex<HashMap<i64, Sender<Result<Value, LspError>>>>,
    events: &Sender<ServerEvent>,
) {
    if let Some(id) = message.get("id").and_then(Value::as_i64) {
        if let Some(sender) = pending.lock().expect("pending lock poisoned").remove(&id) {
            let response = if let Some(error) = message.get("error") {
                Err(LspError::Response {
                    code: error.get("code").and_then(Value::as_i64).unwrap_or(-32603),
                    message: error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error")
                        .into(),
                })
            } else {
                Ok(message.get("result").cloned().unwrap_or(Value::Null))
            };
            let _ = sender.send(response);
        }
    } else if message.get("method").and_then(Value::as_str)
        == Some("textDocument/publishDiagnostics")
        && let Some(params) = message.get("params")
        && let Ok(diagnostics) = serde_json::from_value::<PublishDiagnosticsParams>(params.clone())
    {
        let _ = events.send(ServerEvent::Diagnostics(diagnostics));
    }
}

pub fn read_message(reader: &mut impl BufRead) -> Result<Option<Value>, LspError> {
    let mut content_length = None;
    let mut saw_header = false;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            return if saw_header {
                Err(LspError::Protocol("unexpected EOF in LSP headers".into()))
            } else {
                Ok(None)
            };
        }
        saw_header = true;
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| LspError::Protocol("invalid Content-Length".into()))?,
            );
        }
    }
    let length =
        content_length.ok_or_else(|| LspError::Protocol("missing Content-Length".into()))?;
    let mut payload = vec![0; length];
    reader.read_exact(&mut payload)?;
    Ok(Some(serde_json::from_slice(&payload)?))
}

pub fn write_message(writer: &mut impl Write, value: &Value) -> Result<(), LspError> {
    let payload = serde_json::to_vec(value)?;
    write!(writer, "Content-Length: {}\r\n\r\n", payload.len())?;
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
}

fn find_on_path(executable: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths)
            .map(|path| path.join(executable))
            .find(|path| path.is_file())
    })
}

pub fn normalize_completions(response: Option<CompletionResponse>) -> Vec<CompletionItem> {
    match response {
        Some(CompletionResponse::Array(items)) => items,
        Some(CompletionResponse::List(list)) => list.items,
        None => Vec::new(),
    }
}

pub fn definition_locations(response: Option<GotoDefinitionResponse>) -> Vec<Location> {
    match response {
        Some(GotoDefinitionResponse::Scalar(location)) => vec![location],
        Some(GotoDefinitionResponse::Array(locations)) => locations,
        Some(GotoDefinitionResponse::Link(links)) => links
            .into_iter()
            .map(|link| Location {
                uri: link.target_uri,
                range: link.target_selection_range,
            })
            .collect(),
        None => Vec::new(),
    }
}

pub fn hover_text(hover: Option<Hover>) -> Option<String> {
    use lsp_types::{HoverContents, MarkedString, MarkupContent};
    fn markup(content: MarkupContent) -> String {
        content.value
    }
    hover.map(|hover| match hover.contents {
        HoverContents::Scalar(MarkedString::String(text)) => text,
        HoverContents::Scalar(MarkedString::LanguageString(text)) => text.value,
        HoverContents::Array(items) => items
            .into_iter()
            .map(|item| match item {
                MarkedString::String(text) => text,
                MarkedString::LanguageString(text) => text.value,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        HoverContents::Markup(content) => markup(content),
    })
}

/// Applies LSP edits as one deterministic transaction to a UTF-8 document.
/// LSP positions are converted using the negotiated encoding and edits are
/// applied from the end so earlier ranges remain valid.
pub fn apply_text_edits(text: &str, edits: &[TextEdit], encoding: PositionEncoding) -> String {
    let mut edits = edits
        .iter()
        .map(|edit| {
            let start = PositionCodec::position_to_offset(text, edit.range.start, encoding);
            let end = PositionCodec::position_to_offset(text, edit.range.end, encoding);
            (start.min(end), start.max(end), edit.new_text.as_str())
        })
        .collect::<Vec<_>>();
    edits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    let mut result = text.to_owned();
    for (start, end, replacement) in edits {
        if start <= end
            && end <= result.len()
            && result.is_char_boundary(start)
            && result.is_char_boundary(end)
        {
            result.replace_range(start..end, replacement);
        }
    }
    result
}

#[derive(Debug, Clone)]
pub struct DocumentDiagnostics {
    pub uri: Uri,
    pub version: Option<i32>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn path_to_uri(path: &Path) -> Result<Uri, LspError> {
    let url = url::Url::from_file_path(path)
        .map_err(|_| LspError::Protocol(format!("invalid filesystem path: {}", path.display())))?;
    url.as_str()
        .parse()
        .map_err(|error| LspError::Protocol(format!("invalid file URI: {error}")))
}

pub fn uri_to_path(uri: &Uri) -> Result<PathBuf, LspError> {
    let url = url::Url::parse(uri.as_str())
        .map_err(|error| LspError::Protocol(format!("invalid URI: {error}")))?;
    url.to_file_path()
        .map_err(|_| LspError::Protocol(format!("URI is not a local file: {}", uri.as_str())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn framing_handles_multiple_messages() {
        let first = json!({"jsonrpc":"2.0","id":1,"result":{}});
        let second = json!({"jsonrpc":"2.0","method":"event"});
        let mut bytes = Vec::new();
        write_message(&mut bytes, &first).unwrap();
        write_message(&mut bytes, &second).unwrap();
        let mut reader = BufReader::new(Cursor::new(bytes));
        assert_eq!(read_message(&mut reader).unwrap(), Some(first));
        assert_eq!(read_message(&mut reader).unwrap(), Some(second));
        assert_eq!(read_message(&mut reader).unwrap(), None);
    }

    #[test]
    fn framing_handles_one_byte_reads() {
        struct Slow<R>(R);
        impl<R: Read> Read for Slow<R> {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                let length = buffer.len().min(1);
                self.0.read(&mut buffer[..length])
            }
        }
        let value = json!({"large": "x".repeat(4096)});
        let mut bytes = Vec::new();
        write_message(&mut bytes, &value).unwrap();
        let mut reader = BufReader::new(Slow(Cursor::new(bytes)));
        assert_eq!(read_message(&mut reader).unwrap(), Some(value));
    }

    #[test]
    fn position_codec_round_trips_required_unicode() {
        for text in [
            "hello",
            "João informação ação",
            "Olá 👋",
            "你好",
            "こんにちは",
        ] {
            for encoding in [
                PositionEncoding::Utf8,
                PositionEncoding::Utf16,
                PositionEncoding::Utf32,
            ] {
                for offset in text
                    .char_indices()
                    .map(|(offset, _)| offset)
                    .chain(std::iter::once(text.len()))
                {
                    let position = PositionCodec::offset_to_position(text, offset, encoding);
                    assert_eq!(
                        PositionCodec::position_to_offset(text, position, encoding),
                        offset,
                        "{text:?} {encoding:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn positions_account_for_lines_and_clamp() {
        let text = "Olá 👋\n你好";
        assert_eq!(
            PositionCodec::offset_to_position(
                text,
                text.find('你').unwrap(),
                PositionEncoding::Utf16
            ),
            Position::new(1, 0)
        );
        assert_eq!(
            PositionCodec::position_to_offset(text, Position::new(99, 99), PositionEncoding::Utf16),
            text.len()
        );
    }

    #[test]
    fn completion_and_definition_variants_normalize() {
        let items = vec![CompletionItem {
            label: "findByEmail".into(),
            ..Default::default()
        }];
        assert_eq!(
            normalize_completions(Some(CompletionResponse::Array(items))).len(),
            1
        );
        assert!(definition_locations(None).is_empty());
    }

    #[test]
    fn applies_multiple_unicode_text_edits_in_reverse_order() {
        let text = "Olá mundo\n🙂 fim";
        let edits = vec![
            TextEdit {
                range: lsp_types::Range::new(Position::new(0, 4), Position::new(0, 9)),
                new_text: "Axiom".into(),
            },
            TextEdit {
                range: lsp_types::Range::new(Position::new(1, 0), Position::new(1, 2)),
                new_text: "🚀".into(),
            },
        ];
        assert_eq!(
            apply_text_edits(text, &edits, PositionEncoding::Utf16),
            "Olá Axiom\n🚀 fim"
        );
    }

    #[test]
    fn responses_are_correlated_by_id_not_arrival_order() {
        let pending: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let (first_tx, first_rx) = mpsc::channel();
        let (second_tx, second_rx) = mpsc::channel();
        pending.lock().unwrap().insert(1, first_tx);
        pending.lock().unwrap().insert(2, second_tx);
        let (events, _) = mpsc::channel();
        dispatch_message(
            json!({"jsonrpc":"2.0","id":2,"result":"second"}),
            &pending,
            &events,
        );
        dispatch_message(
            json!({"jsonrpc":"2.0","id":1,"result":"first"}),
            &pending,
            &events,
        );
        assert_eq!(first_rx.recv().unwrap().unwrap(), json!("first"));
        assert_eq!(second_rx.recv().unwrap().unwrap(), json!("second"));
    }
}
