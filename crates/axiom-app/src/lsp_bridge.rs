use std::{
    path::Path,
    sync::{Arc, Mutex},
    thread,
};

use axiom_lsp::{
    DEFAULT_REQUEST_TIMEOUT, LanguageServer, ServerEvent, ServerStatus, definition_locations,
    hover_text, normalize_completions,
};
use lsp_types::{CompletionResponse, GotoDefinitionResponse, Hover, Location, Position, Uri};

#[derive(Debug)]
pub enum IdeLspEvent {
    Diagnostics(lsp_types::PublishDiagnosticsParams),
    Completion {
        uri: Uri,
        items: Vec<lsp_types::CompletionItem>,
    },
    Formatting {
        uri: Uri,
        edits: Vec<lsp_types::TextEdit>,
    },
    SignatureHelp {
        uri: Uri,
        text: Option<String>,
    },
    Hover {
        uri: Uri,
        text: Option<String>,
    },
    Definition {
        locations: Vec<Location>,
    },
    References {
        count: usize,
    },
    Error(String),
    Stopped,
}

pub struct LspBridge {
    server: Option<Arc<Mutex<LanguageServer>>>,
    status: Mutex<ServerStatus>,
    pending_events: Arc<Mutex<Vec<IdeLspEvent>>>,
}

impl LspBridge {
    pub fn start(root: &Path) -> Arc<Self> {
        match LanguageServer::start_intelephense() {
            Ok(mut server) => match server.initialize(root) {
                Ok(()) => Arc::new(Self {
                    server: Some(Arc::new(Mutex::new(server))),
                    status: Mutex::new(ServerStatus::Ready),
                    pending_events: Arc::new(Mutex::new(Vec::new())),
                }),
                Err(error) => Arc::new(Self {
                    server: None,
                    status: Mutex::new(ServerStatus::Stopped),
                    pending_events: Arc::new(Mutex::new(vec![IdeLspEvent::Error(
                        error.to_string(),
                    )])),
                }),
            },
            Err(axiom_lsp::LspError::ServerNotFound) => Arc::new(Self {
                server: None,
                status: Mutex::new(ServerStatus::NotFound),
                pending_events: Arc::new(Mutex::new(Vec::new())),
            }),
            Err(error) => Arc::new(Self {
                server: None,
                status: Mutex::new(ServerStatus::Stopped),
                pending_events: Arc::new(Mutex::new(vec![IdeLspEvent::Error(error.to_string())])),
            }),
        }
    }

    pub fn status(&self) -> ServerStatus {
        self.server
            .as_ref()
            .map(|server| server.lock().expect("LSP lock poisoned").status())
            .unwrap_or_else(|| *self.status.lock().expect("status lock poisoned"))
    }

    pub fn encoding(&self) -> axiom_lsp::PositionEncoding {
        self.server
            .as_ref()
            .map(|server| {
                server
                    .lock()
                    .expect("LSP lock poisoned")
                    .position_encoding()
            })
            .unwrap_or_default()
    }

    pub fn with_server(&self, operation: impl FnOnce(&LanguageServer)) {
        if let Some(server) = &self.server {
            operation(&server.lock().expect("LSP lock poisoned"));
        }
    }

    pub fn request_completion(&self, uri: Uri, position: Position) {
        let Some(server) = &self.server else { return };
        let pending = match server
            .lock()
            .expect("LSP lock poisoned")
            .completion(uri.clone(), position)
        {
            Ok(pending) => pending,
            Err(error) => {
                self.push(IdeLspEvent::Error(error.to_string()));
                return;
            }
        };
        let events = self.pending_events.clone();
        thread::spawn(move || {
            let event = match pending.recv::<Option<CompletionResponse>>(DEFAULT_REQUEST_TIMEOUT) {
                Ok(response) => IdeLspEvent::Completion {
                    uri,
                    items: normalize_completions(response),
                },
                Err(error) => IdeLspEvent::Error(error.to_string()),
            };
            events.lock().expect("event lock poisoned").push(event);
        });
    }

    pub fn request_hover(&self, uri: Uri, position: Position) {
        let Some(server) = &self.server else { return };
        let pending = match server
            .lock()
            .expect("LSP lock poisoned")
            .hover(uri.clone(), position)
        {
            Ok(pending) => pending,
            Err(error) => {
                self.push(IdeLspEvent::Error(error.to_string()));
                return;
            }
        };
        let events = self.pending_events.clone();
        thread::spawn(move || {
            let event = match pending.recv::<Option<Hover>>(DEFAULT_REQUEST_TIMEOUT) {
                Ok(response) => IdeLspEvent::Hover {
                    uri,
                    text: hover_text(response),
                },
                Err(error) => IdeLspEvent::Error(error.to_string()),
            };
            events.lock().expect("event lock poisoned").push(event);
        });
    }

    pub fn request_definition(&self, uri: Uri, position: Position) {
        let Some(server) = &self.server else { return };
        let pending = match server
            .lock()
            .expect("LSP lock poisoned")
            .definition(uri, position)
        {
            Ok(pending) => pending,
            Err(error) => {
                self.push(IdeLspEvent::Error(error.to_string()));
                return;
            }
        };
        let events = self.pending_events.clone();
        thread::spawn(move || {
            let event =
                match pending.recv::<Option<GotoDefinitionResponse>>(DEFAULT_REQUEST_TIMEOUT) {
                    Ok(response) => IdeLspEvent::Definition {
                        locations: definition_locations(response),
                    },
                    Err(error) => IdeLspEvent::Error(error.to_string()),
                };
            events.lock().expect("event lock poisoned").push(event);
        });
    }

    pub fn request_references(&self, uri: Uri, position: Position) {
        let Some(server) = &self.server else { return };
        let pending = match server
            .lock()
            .expect("LSP lock poisoned")
            .references(uri, position)
        {
            Ok(pending) => pending,
            Err(error) => {
                self.push(IdeLspEvent::Error(error.to_string()));
                return;
            }
        };
        let events = self.pending_events.clone();
        thread::spawn(move || {
            let event = match pending.recv::<Option<Vec<Location>>>(DEFAULT_REQUEST_TIMEOUT) {
                Ok(response) => IdeLspEvent::References {
                    count: response.unwrap_or_default().len(),
                },
                Err(error) => IdeLspEvent::Error(error.to_string()),
            };
            events.lock().expect("event lock poisoned").push(event);
        });
    }

    pub fn request_formatting(&self, uri: Uri, tab_size: u32, insert_spaces: bool) {
        let Some(server) = &self.server else { return };
        let pending = match server.lock().expect("LSP lock poisoned").formatting(
            uri.clone(),
            tab_size,
            insert_spaces,
        ) {
            Ok(pending) => pending,
            Err(error) => {
                self.push(IdeLspEvent::Error(error.to_string()));
                return;
            }
        };
        let events = self.pending_events.clone();
        thread::spawn(move || {
            let event =
                match pending.recv::<Option<Vec<lsp_types::TextEdit>>>(DEFAULT_REQUEST_TIMEOUT) {
                    Ok(edits) => IdeLspEvent::Formatting {
                        uri,
                        edits: edits.unwrap_or_default(),
                    },
                    Err(error) => IdeLspEvent::Error(error.to_string()),
                };
            events.lock().expect("event lock poisoned").push(event);
        });
    }

    pub fn request_signature_help(&self, uri: Uri, position: Position) {
        let Some(server) = &self.server else { return };
        let pending = match server
            .lock()
            .expect("LSP lock poisoned")
            .signature_help(uri.clone(), position)
        {
            Ok(pending) => pending,
            Err(error) => {
                self.push(IdeLspEvent::Error(error.to_string()));
                return;
            }
        };
        let events = self.pending_events.clone();
        thread::spawn(move || {
            let event =
                match pending.recv::<Option<lsp_types::SignatureHelp>>(DEFAULT_REQUEST_TIMEOUT) {
                    Ok(help) => IdeLspEvent::SignatureHelp {
                        uri,
                        text: help.map(signature_help_text),
                    },
                    Err(error) => IdeLspEvent::Error(error.to_string()),
                };
            events.lock().expect("event lock poisoned").push(event);
        });
    }

    pub fn drain_events(&self) -> Vec<IdeLspEvent> {
        if let Some(server) = &self.server {
            let server = server.lock().expect("LSP lock poisoned");
            while let Some(event) = server.try_event() {
                match event {
                    ServerEvent::Diagnostics(params) => self.push(IdeLspEvent::Diagnostics(params)),
                    ServerEvent::Stopped => self.push(IdeLspEvent::Stopped),
                    ServerEvent::Log(_) => {}
                }
            }
        }
        std::mem::take(&mut *self.pending_events.lock().expect("event lock poisoned"))
    }

    fn push(&self, event: IdeLspEvent) {
        self.pending_events
            .lock()
            .expect("event lock poisoned")
            .push(event);
    }
}

fn signature_help_text(help: lsp_types::SignatureHelp) -> String {
    let Some(signature) = help
        .signatures
        .get(help.active_signature.unwrap_or(0) as usize)
    else {
        return String::new();
    };
    match &signature.label {
        label if label.is_empty() => signature
            .documentation
            .as_ref()
            .map_or_else(String::new, |doc| format!("{doc:?}")),
        label => label.clone(),
    }
}

impl Drop for LspBridge {
    fn drop(&mut self) {
        if let Some(server) = &self.server {
            let server = server.lock().expect("LSP lock poisoned");
            if let Err(error) = server.shutdown() {
                tracing::warn!("language server shutdown failed: {error}");
            }
            if let Err(error) = server.wait() {
                tracing::warn!("language server wait failed: {error}");
            }
        }
    }
}
