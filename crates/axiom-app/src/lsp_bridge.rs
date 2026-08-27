use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};

use gpui::background_executor;

use axiom_lsp::{
    DEFAULT_REQUEST_TIMEOUT, LanguageServer, ServerEvent, ServerStatus, definition_locations,
    hover_text, normalize_completions,
};
use lsp_types::{CompletionResponse, GotoDefinitionResponse, Hover, Location, Position, Uri};

#[derive(Debug)]
pub enum IdeLspEvent {
    Diagnostics {
        params: lsp_types::PublishDiagnosticsParams,
        session: u64,
    },
    Completion {
        uri: Uri,
        items: Vec<lsp_types::CompletionItem>,
        generation: u64,
    },
    Formatting {
        uri: Uri,
        edits: Vec<lsp_types::TextEdit>,
        generation: u64,
    },
    SignatureHelp {
        uri: Uri,
        text: Option<String>,
        generation: u64,
    },
    Hover {
        uri: Uri,
        text: Option<String>,
        generation: u64,
    },
    Definition {
        uri: Uri,
        locations: Vec<Location>,
        generation: u64,
    },
    References {
        uri: Uri,
        locations: Vec<Location>,
        generation: u64,
    },
    Error(String),
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LspRequestKind {
    Completion,
    Hover,
    Definition,
    References,
    Formatting,
    SignatureHelp,
}

pub struct LspBridge {
    server: Option<Arc<Mutex<LanguageServer>>>,
    status: Mutex<ServerStatus>,
    pending_events: Arc<Mutex<Vec<IdeLspEvent>>>,
    generations: Mutex<HashMap<(Uri, LspRequestKind), u64>>,
    document_sessions: Mutex<HashMap<Uri, u64>>,
}

impl LspBridge {
    pub fn start(root: &Path) -> Arc<Self> {
        match LanguageServer::start_intelephense() {
            Ok(mut server) => match server.initialize(root) {
                Ok(()) => Arc::new(Self {
                    server: Some(Arc::new(Mutex::new(server))),
                    status: Mutex::new(ServerStatus::Ready),
                    pending_events: Arc::new(Mutex::new(Vec::new())),
                    generations: Mutex::new(HashMap::new()),
                    document_sessions: Mutex::new(HashMap::new()),
                }),
                Err(error) => Arc::new(Self {
                    server: None,
                    status: Mutex::new(ServerStatus::Stopped),
                    pending_events: Arc::new(Mutex::new(vec![IdeLspEvent::Error(
                        error.to_string(),
                    )])),
                    generations: Mutex::new(HashMap::new()),
                    document_sessions: Mutex::new(HashMap::new()),
                }),
            },
            Err(axiom_lsp::LspError::ServerNotFound) => Arc::new(Self {
                server: None,
                status: Mutex::new(ServerStatus::NotFound),
                pending_events: Arc::new(Mutex::new(Vec::new())),
                generations: Mutex::new(HashMap::new()),
                document_sessions: Mutex::new(HashMap::new()),
            }),
            Err(error) => Arc::new(Self {
                server: None,
                status: Mutex::new(ServerStatus::Stopped),
                pending_events: Arc::new(Mutex::new(vec![IdeLspEvent::Error(error.to_string())])),
                generations: Mutex::new(HashMap::new()),
                document_sessions: Mutex::new(HashMap::new()),
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

    fn next_generation(&self, uri: &Uri, kind: LspRequestKind) -> u64 {
        let mut generations = self.generations.lock().expect("generation lock poisoned");
        let generation = generations.entry((uri.clone(), kind)).or_insert(0);
        *generation += 1;
        *generation
    }

    pub fn request_completion(&self, uri: Uri, position: Position) {
        let generation = self.next_generation(&uri, LspRequestKind::Completion);
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
        background_executor()
            .spawn(async move {
                let event =
                    match pending.recv::<Option<CompletionResponse>>(DEFAULT_REQUEST_TIMEOUT) {
                        Ok(response) => IdeLspEvent::Completion {
                            uri,
                            items: normalize_completions(response),
                            generation,
                        },
                        Err(error) => IdeLspEvent::Error(error.to_string()),
                    };
                events.lock().expect("event lock poisoned").push(event);
            })
            .detach();
    }

    pub fn request_hover(&self, uri: Uri, position: Position) {
        let generation = self.next_generation(&uri, LspRequestKind::Hover);
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
        background_executor()
            .spawn(async move {
                let event = match pending.recv::<Option<Hover>>(DEFAULT_REQUEST_TIMEOUT) {
                    Ok(response) => IdeLspEvent::Hover {
                        uri,
                        text: hover_text(response),
                        generation,
                    },
                    Err(error) => IdeLspEvent::Error(error.to_string()),
                };
                events.lock().expect("event lock poisoned").push(event);
            })
            .detach();
    }

    pub fn request_definition(&self, uri: Uri, position: Position) {
        let generation = self.next_generation(&uri, LspRequestKind::Definition);
        let Some(server) = &self.server else { return };
        let pending = match server
            .lock()
            .expect("LSP lock poisoned")
            .definition(uri.clone(), position)
        {
            Ok(pending) => pending,
            Err(error) => {
                self.push(IdeLspEvent::Error(error.to_string()));
                return;
            }
        };
        let events = self.pending_events.clone();
        background_executor()
            .spawn(async move {
                let event =
                    match pending.recv::<Option<GotoDefinitionResponse>>(DEFAULT_REQUEST_TIMEOUT) {
                        Ok(response) => IdeLspEvent::Definition {
                            uri,
                            locations: definition_locations(response),
                            generation,
                        },
                        Err(error) => IdeLspEvent::Error(error.to_string()),
                    };
                events.lock().expect("event lock poisoned").push(event);
            })
            .detach();
    }

    pub fn request_references(&self, uri: Uri, position: Position) {
        let generation = self.next_generation(&uri, LspRequestKind::References);
        let Some(server) = &self.server else { return };
        let pending = match server
            .lock()
            .expect("LSP lock poisoned")
            .references(uri.clone(), position)
        {
            Ok(pending) => pending,
            Err(error) => {
                self.push(IdeLspEvent::Error(error.to_string()));
                return;
            }
        };
        let events = self.pending_events.clone();
        background_executor()
            .spawn(async move {
                let event = match pending.recv::<Option<Vec<Location>>>(DEFAULT_REQUEST_TIMEOUT) {
                    Ok(response) => IdeLspEvent::References {
                        uri,
                        locations: response.unwrap_or_default(),
                        generation,
                    },
                    Err(error) => IdeLspEvent::Error(error.to_string()),
                };
                events.lock().expect("event lock poisoned").push(event);
            })
            .detach();
    }

    pub fn request_formatting(&self, uri: Uri, tab_size: u32, insert_spaces: bool) {
        let generation = self.next_generation(&uri, LspRequestKind::Formatting);
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
        background_executor()
            .spawn(async move {
                let event = match pending
                    .recv::<Option<Vec<lsp_types::TextEdit>>>(DEFAULT_REQUEST_TIMEOUT)
                {
                    Ok(edits) => IdeLspEvent::Formatting {
                        uri,
                        edits: edits.unwrap_or_default(),
                        generation,
                    },
                    Err(error) => IdeLspEvent::Error(error.to_string()),
                };
                events.lock().expect("event lock poisoned").push(event);
            })
            .detach();
    }

    pub fn request_signature_help(&self, uri: Uri, position: Position) {
        let generation = self.next_generation(&uri, LspRequestKind::SignatureHelp);
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
        background_executor()
            .spawn(async move {
                let event = match pending
                    .recv::<Option<lsp_types::SignatureHelp>>(DEFAULT_REQUEST_TIMEOUT)
                {
                    Ok(help) => IdeLspEvent::SignatureHelp {
                        uri,
                        text: help.map(signature_help_text),
                        generation,
                    },
                    Err(error) => IdeLspEvent::Error(error.to_string()),
                };
                events.lock().expect("event lock poisoned").push(event);
            })
            .detach();
    }

    pub fn register_document_session(&self, uri: Uri, session: u64) {
        if let Ok(mut sessions) = self.document_sessions.lock() {
            sessions.insert(uri, session);
        }
    }

    pub fn invalidate_document_session(&self, uri: &Uri, session: u64) {
        if let Ok(mut sessions) = self.document_sessions.lock() {
            if sessions.get(uri).copied() == Some(session) {
                sessions.remove(uri);
            }
        }
        if let Ok(mut events) = self.pending_events.lock() {
            events.retain(|event| !matches!(event, IdeLspEvent::Diagnostics { params, session: old } if &params.uri == uri && *old == session));
        }
    }

    pub fn drain_events(&self) -> Vec<IdeLspEvent> {
        if let Some(server) = &self.server {
            let server = server.lock().expect("LSP lock poisoned");
            while let Some(event) = server.try_event() {
                match event {
                    ServerEvent::Diagnostics(params) => {
                        let session = self
                            .document_sessions
                            .lock()
                            .ok()
                            .and_then(|sessions| sessions.get(&params.uri).copied())
                            .unwrap_or(0);
                        self.push(IdeLspEvent::Diagnostics { params, session });
                    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    fn accept_generation(last: &mut u64, generation: u64) -> bool {
        if generation < *last {
            return false;
        }
        *last = generation;
        true
    }

    #[test]
    fn document_session_invalidation_rejects_old_diagnostics() {
        let uri: Uri = "file:///A.php".parse().unwrap();
        let bridge = LspBridge {
            server: None,
            status: Mutex::new(ServerStatus::Stopped),
            pending_events: Arc::new(Mutex::new(Vec::new())),
            generations: Mutex::new(HashMap::new()),
            document_sessions: Mutex::new(HashMap::new()),
        };
        bridge.register_document_session(uri.clone(), 10);
        bridge
            .pending_events
            .lock()
            .unwrap()
            .push(IdeLspEvent::Diagnostics {
                params: lsp_types::PublishDiagnosticsParams {
                    uri: uri.clone(),
                    diagnostics: Vec::new(),
                    version: None,
                },
                session: 10,
            });
        bridge.invalidate_document_session(&uri, 10);
        bridge.register_document_session(uri.clone(), 11);
        assert!(bridge.drain_events().is_empty());
        bridge
            .pending_events
            .lock()
            .unwrap()
            .push(IdeLspEvent::Diagnostics {
                params: lsp_types::PublishDiagnosticsParams {
                    uri,
                    diagnostics: Vec::new(),
                    version: None,
                },
                session: 11,
            });
        assert_eq!(bridge.drain_events().len(), 1);
    }
    #[test]
    fn newer_lsp_response_wins_over_out_of_order_older_response() {
        let mut last = 0;
        assert!(accept_generation(&mut last, 2));
        assert!(!accept_generation(&mut last, 1));
        assert_eq!(last, 2);
        assert!(accept_generation(&mut last, 3));
        assert_eq!(last, 3);
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
