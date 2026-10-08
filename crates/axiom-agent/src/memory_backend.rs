//! Read-only adapter boundary for an external persistent-memory backend.
//!
//! No ai-memory protocol or transport implementation belongs here until a
//! supported public read surface is available. Callers must execute a real
//! transport on a background worker; this adapter itself performs no I/O.

use crate::{MemoryHandoff, MemoryResult, MemoryScope, MemoryService, MemorySessionId};

pub const MAX_BACKEND_RESULTS: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryReadOperation {
    Query,
    Briefing,
    Recent,
    History,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryReadRequest {
    pub operation: MemoryReadOperation,
    pub scope: MemoryScope,
    pub query: Option<String>,
    pub limit: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryReadError {
    Unavailable,
    Timeout,
    Cancelled,
    InvalidResponse,
}

/// Transport seam for a supported public read API.
///
/// Implementations translate their protocol response into Axiom-owned
/// `MemoryResult` values. They must enforce their own timeout/cancellation
/// boundary and should be called off the UI thread.
pub trait AiMemoryReadTransport: Send + Sync {
    fn read(&self, request: &MemoryReadRequest) -> Result<Vec<MemoryResult>, MemoryReadError>;
}

pub struct AiMemoryBackend<T> {
    transport: T,
}

impl<T> AiMemoryBackend<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    fn read(
        &self,
        operation: MemoryReadOperation,
        scope: MemoryScope,
        query: Option<&str>,
        limit: usize,
    ) -> Vec<MemoryResult>
    where
        T: AiMemoryReadTransport,
    {
        let request = MemoryReadRequest {
            operation,
            scope,
            query: query.map(str::to_owned),
            limit: limit.min(MAX_BACKEND_RESULTS),
        };
        match self.transport.read(&request) {
            Ok(mut results) => {
                results.truncate(request.limit);
                results
            }
            Err(_) => Vec::new(),
        }
    }
}

impl<T> MemoryService for AiMemoryBackend<T>
where
    T: AiMemoryReadTransport,
{
    fn session_start(&mut self, _scope: MemoryScope) -> MemorySessionId {
        MemorySessionId::new(0)
    }

    fn observe(&mut self, _session: MemorySessionId, _observation: crate::MemoryObservation) {}

    fn session_end(&mut self, _session: MemorySessionId) {}

    fn query(&self, scope: MemoryScope, query: &str, limit: usize) -> Vec<MemoryResult> {
        self.read(MemoryReadOperation::Query, scope, Some(query), limit)
    }

    fn briefing(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        self.read(MemoryReadOperation::Briefing, scope, None, limit)
    }

    fn recent(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        self.read(MemoryReadOperation::Recent, scope, None, limit)
    }

    fn history(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        self.read(MemoryReadOperation::History, scope, None, limit)
    }

    fn handoff(&self, _session: MemorySessionId) -> Option<MemoryHandoff> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeTransport {
        requests: std::sync::Arc<Mutex<Vec<MemoryReadRequest>>>,
        results: Vec<MemoryResult>,
        error: Option<MemoryReadError>,
    }

    impl AiMemoryReadTransport for FakeTransport {
        fn read(&self, request: &MemoryReadRequest) -> Result<Vec<MemoryResult>, MemoryReadError> {
            self.requests.lock().unwrap().push(request.clone());
            self.error.map_or_else(|| Ok(self.results.clone()), Err)
        }
    }

    fn result(index: usize, scope: MemoryScope) -> MemoryResult {
        MemoryResult {
            id: Some(format!("memory-{index}")),
            scope,
            category: None,
            content: format!("memory {index}"),
        }
    }

    #[test]
    fn query_maps_scope_and_normalizes_transport_results() {
        let transport = FakeTransport {
            results: vec![result(1, MemoryScope::Project)],
            ..Default::default()
        };
        let requests = transport.requests.clone();
        let backend = AiMemoryBackend::new(transport);
        let results = backend.query(MemoryScope::Project, "constraint", 4);
        assert_eq!(results, vec![result(1, MemoryScope::Project)]);
        let request = requests.lock().unwrap()[0].clone();
        assert_eq!(request.operation, MemoryReadOperation::Query);
        assert_eq!(request.scope, MemoryScope::Project);
        assert_eq!(request.query.as_deref(), Some("constraint"));
        assert_eq!(request.limit, 4);
    }

    #[test]
    fn every_read_operation_is_bounded_and_scoped() {
        let transport = FakeTransport {
            results: (0..128)
                .map(|index| result(index, MemoryScope::Workspace))
                .collect(),
            ..Default::default()
        };
        let requests = transport.requests.clone();
        let backend = AiMemoryBackend::new(transport);
        assert_eq!(
            backend.briefing(MemoryScope::Workspace, usize::MAX).len(),
            MAX_BACKEND_RESULTS
        );
        assert_eq!(backend.recent(MemoryScope::Workspace, 3).len(), 3);
        assert_eq!(backend.history(MemoryScope::Workspace, 2).len(), 2);
        let requests = requests.lock().unwrap();
        assert_eq!(requests[0].operation, MemoryReadOperation::Briefing);
        assert_eq!(requests[0].limit, MAX_BACKEND_RESULTS);
        assert_eq!(requests[1].operation, MemoryReadOperation::Recent);
        assert_eq!(requests[2].operation, MemoryReadOperation::History);
        assert!(
            requests
                .iter()
                .all(|request| request.scope == MemoryScope::Workspace)
        );
    }

    #[test]
    fn timeout_and_unavailable_transport_degrade_to_empty_results() {
        for error in [MemoryReadError::Timeout, MemoryReadError::Unavailable] {
            let backend = AiMemoryBackend::new(FakeTransport {
                error: Some(error),
                ..Default::default()
            });
            assert!(backend.query(MemoryScope::User, "anything", 10).is_empty());
            assert!(backend.briefing(MemoryScope::User, 10).is_empty());
        }
    }
}
