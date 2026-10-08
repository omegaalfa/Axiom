//! Provider-neutral memory contracts for the Agent runtime.
//!
//! This module intentionally contains no persistence or backend protocol.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MemoryScope {
    User,
    Workspace,
    Project,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemoryCategory {
    Remember,
    Decision,
    Gotcha,
    Handoff,
    Discovery,
    Mutation,
    Validation,
    Outcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryResult {
    pub id: Option<String>,
    pub scope: MemoryScope,
    pub category: Option<MemoryCategory>,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryHandoff {
    pub session: MemorySessionId,
    pub scope: MemoryScope,
    pub entries: Vec<MemoryResult>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemoryObservation {
    Discovery(String),
    Decision(String),
    MutationSummary(String),
    Validation(String),
    TaskOutcome(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MemorySessionId(u64);

impl MemorySessionId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

pub trait MemoryService: Send {
    fn is_available(&self) -> bool {
        true
    }

    fn session_start(&mut self, scope: MemoryScope) -> MemorySessionId;
    fn observe(&mut self, session: MemorySessionId, observation: MemoryObservation);
    fn session_end(&mut self, session: MemorySessionId);
    fn query(&self, scope: MemoryScope, query: &str, limit: usize) -> Vec<MemoryResult>;
    fn briefing(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult>;
    fn recent(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult>;
    fn history(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult>;
    fn get(&self, _scope: MemoryScope, _id: &str) -> Option<MemoryResult> {
        None
    }
    fn handoff(&self, session: MemorySessionId) -> Option<MemoryHandoff>;

    fn remember(&mut self, _scope: MemoryScope, _content: &str) {}
    fn decision(&mut self, _scope: MemoryScope, _content: &str) {}
    fn gotcha(&mut self, _scope: MemoryScope, _content: &str) {}
    fn persist_handoff(&mut self, _handoff: &MemoryHandoff) {}
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoMemory;

impl MemoryService for NoMemory {
    fn is_available(&self) -> bool {
        false
    }

    fn session_start(&mut self, _scope: MemoryScope) -> MemorySessionId {
        MemorySessionId::new(0)
    }

    fn observe(&mut self, _session: MemorySessionId, _observation: MemoryObservation) {}

    fn session_end(&mut self, _session: MemorySessionId) {}

    fn query(&self, _scope: MemoryScope, _query: &str, _limit: usize) -> Vec<MemoryResult> {
        Vec::new()
    }

    fn briefing(&self, _scope: MemoryScope, _limit: usize) -> Vec<MemoryResult> {
        Vec::new()
    }

    fn recent(&self, _scope: MemoryScope, _limit: usize) -> Vec<MemoryResult> {
        Vec::new()
    }

    fn history(&self, _scope: MemoryScope, _limit: usize) -> Vec<MemoryResult> {
        Vec::new()
    }

    fn get(&self, _scope: MemoryScope, _id: &str) -> Option<MemoryResult> {
        None
    }

    fn handoff(&self, _session: MemorySessionId) -> Option<MemoryHandoff> {
        None
    }
}

const MAX_OBSERVATIONS_PER_SESSION: usize = 64;
const MAX_MEMORY_RESULTS: usize = 32;
const MAX_CONTENT_CHARS: usize = 1024;

#[derive(Clone, Debug)]
struct InMemorySession {
    id: MemorySessionId,
    scope: MemoryScope,
    observations: Vec<String>,
    ended: bool,
}

#[derive(Clone, Debug, Default)]
pub struct InMemoryMemoryService {
    next_session_id: u64,
    sessions: Vec<InMemorySession>,
}

impl InMemoryMemoryService {
    fn normalized(value: &str) -> String {
        value
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(MAX_CONTENT_CHARS)
            .collect()
    }

    fn result(session: &InMemorySession, index: usize, content: &str) -> MemoryResult {
        MemoryResult {
            id: Some(format!("{}:{index}", session.id.value())),
            scope: session.scope,
            category: category_from_content(content),
            content: content.to_owned(),
        }
    }

    fn bounded_limit(limit: usize) -> usize {
        limit.min(MAX_MEMORY_RESULTS)
    }

    fn completed_results(
        &self,
        scope: MemoryScope,
        limit: usize,
        predicate: impl Fn(&str) -> bool,
    ) -> Vec<MemoryResult> {
        let limit = Self::bounded_limit(limit);
        if limit == 0 {
            return Vec::new();
        }
        self.sessions
            .iter()
            .filter(|session| session.scope == scope && session.ended)
            .flat_map(|session| {
                session
                    .observations
                    .iter()
                    .enumerate()
                    .filter(|(_, content)| predicate(content))
                    .map(|(index, content)| Self::result(session, index, content))
            })
            .take(limit)
            .collect()
    }
}

impl MemoryService for InMemoryMemoryService {
    fn session_start(&mut self, scope: MemoryScope) -> MemorySessionId {
        self.next_session_id = self.next_session_id.saturating_add(1);
        let id = MemorySessionId::new(self.next_session_id);
        self.sessions.push(InMemorySession {
            id,
            scope,
            observations: Vec::new(),
            ended: false,
        });
        id
    }

    fn observe(&mut self, session: MemorySessionId, observation: MemoryObservation) {
        let Some(session) = self
            .sessions
            .iter_mut()
            .find(|candidate| candidate.id == session && !candidate.ended)
        else {
            return;
        };
        if session.observations.len() >= MAX_OBSERVATIONS_PER_SESSION {
            return;
        }
        let content = match observation {
            MemoryObservation::Discovery(value) => format!("discovery: {value}"),
            MemoryObservation::Decision(value) => format!("decision: {value}"),
            MemoryObservation::MutationSummary(value) => format!("mutation: {value}"),
            MemoryObservation::Validation(value) => format!("validation: {value}"),
            MemoryObservation::TaskOutcome(value) => format!("outcome: {value}"),
        };
        let content = Self::normalized(&content);
        if !content.is_empty() {
            session.observations.push(content);
        }
    }

    fn session_end(&mut self, session: MemorySessionId) {
        if let Some(session) = self
            .sessions
            .iter_mut()
            .find(|candidate| candidate.id == session)
        {
            session.ended = true;
        }
    }

    fn query(&self, scope: MemoryScope, query: &str, limit: usize) -> Vec<MemoryResult> {
        let query = Self::normalized(query).to_ascii_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        self.completed_results(scope, limit, |content| {
            content.to_ascii_lowercase().contains(&query)
        })
    }

    fn briefing(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        self.completed_results(scope, limit, |_| true)
    }

    fn recent(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        self.completed_results(scope, limit, |_| true)
    }

    fn history(&self, scope: MemoryScope, limit: usize) -> Vec<MemoryResult> {
        self.completed_results(scope, limit, |_| true)
    }

    fn get(&self, scope: MemoryScope, id: &str) -> Option<MemoryResult> {
        self.sessions
            .iter()
            .filter(|session| session.scope == scope && session.ended)
            .flat_map(|session| {
                session
                    .observations
                    .iter()
                    .enumerate()
                    .map(|(index, content)| Self::result(session, index, content))
            })
            .find(|result| result.id.as_deref() == Some(id))
    }

    fn handoff(&self, session: MemorySessionId) -> Option<MemoryHandoff> {
        let session = self
            .sessions
            .iter()
            .find(|candidate| candidate.id == session && candidate.ended)?;
        let entries = session
            .observations
            .iter()
            .enumerate()
            .take(MAX_MEMORY_RESULTS)
            .map(|(index, content)| Self::result(session, index, content))
            .collect();
        Some(MemoryHandoff {
            session: session.id,
            scope: session.scope,
            entries,
        })
    }
}

fn category_from_content(content: &str) -> Option<MemoryCategory> {
    let category = content.split_once(':')?.0;
    Some(match category {
        "remember" => MemoryCategory::Remember,
        "decision" => MemoryCategory::Decision,
        "gotcha" => MemoryCategory::Gotcha,
        "handoff" => MemoryCategory::Handoff,
        "discovery" => MemoryCategory::Discovery,
        "mutation" => MemoryCategory::Mutation,
        "validation" => MemoryCategory::Validation,
        "outcome" => MemoryCategory::Outcome,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_cover_user_workspace_and_project() {
        assert_ne!(MemoryScope::User, MemoryScope::Workspace);
        assert_ne!(MemoryScope::Workspace, MemoryScope::Project);
    }

    #[test]
    fn no_memory_is_deterministic_and_has_no_state() {
        let mut memory = NoMemory;
        let first = memory.session_start(MemoryScope::Project);
        let second = memory.session_start(MemoryScope::User);
        assert_eq!(first, MemorySessionId::new(0));
        assert_eq!(second, first);
        memory.observe(first, MemoryObservation::Discovery("ignored".into()));
        memory.session_end(first);
        assert_eq!(memory.session_start(MemoryScope::Workspace), first);
        assert!(memory.query(MemoryScope::Project, "ignored", 10).is_empty());
        assert!(memory.briefing(MemoryScope::Project, 10).is_empty());
        assert!(memory.handoff(first).is_none());
    }

    #[test]
    fn memory_result_and_observation_are_backend_neutral() {
        let result = MemoryResult {
            id: Some("memory-1".into()),
            scope: MemoryScope::Workspace,
            category: None,
            content: "content".into(),
        };
        assert_eq!(result.scope, MemoryScope::Workspace);
        assert_eq!(
            MemoryObservation::TaskOutcome("completed".into()),
            MemoryObservation::TaskOutcome("completed".into())
        );
    }

    #[test]
    fn in_memory_sessions_are_unique_and_keep_scope() {
        let mut memory = InMemoryMemoryService::default();
        let project = memory.session_start(MemoryScope::Project);
        let workspace = memory.session_start(MemoryScope::Workspace);
        assert_ne!(project, workspace);
        memory.observe(project, MemoryObservation::Discovery("project fact".into()));
        memory.session_end(project);
        assert_eq!(
            memory.briefing(MemoryScope::Project, 10)[0].scope,
            MemoryScope::Project
        );
        assert!(memory.briefing(MemoryScope::Workspace, 10).is_empty());
    }

    #[test]
    fn observations_preserve_order_and_end_session() {
        let mut memory = InMemoryMemoryService::default();
        let session = memory.session_start(MemoryScope::User);
        memory.observe(session, MemoryObservation::Discovery(" first fact ".into()));
        memory.observe(session, MemoryObservation::Decision("second fact".into()));
        assert!(memory.briefing(MemoryScope::User, 10).is_empty());
        memory.session_end(session);
        let results = memory.briefing(MemoryScope::User, 10);
        assert_eq!(results[0].content, "discovery: first fact");
        assert_eq!(results[1].content, "decision: second fact");
    }

    #[test]
    fn query_normalizes_text_and_is_bounded() {
        let mut memory = InMemoryMemoryService::default();
        let session = memory.session_start(MemoryScope::Project);
        for index in 0..40 {
            memory.observe(
                session,
                MemoryObservation::Discovery(format!("Important item {index}")),
            );
        }
        memory.session_end(session);
        let results = memory.query(MemoryScope::Project, "  important   item ", 1000);
        assert_eq!(results.len(), MAX_MEMORY_RESULTS);
        assert_eq!(results[0].content, "discovery: Important item 0");
    }

    #[test]
    fn briefing_and_handoff_are_scoped_and_bounded() {
        let mut memory = InMemoryMemoryService::default();
        let project = memory.session_start(MemoryScope::Project);
        let user = memory.session_start(MemoryScope::User);
        for index in 0..40 {
            memory.observe(
                project,
                MemoryObservation::Validation(format!("project {index}")),
            );
        }
        memory.observe(user, MemoryObservation::Validation("user only".into()));
        memory.session_end(project);
        memory.session_end(user);
        assert_eq!(
            memory.briefing(MemoryScope::Project, 1000).len(),
            MAX_MEMORY_RESULTS
        );
        assert!(
            memory
                .query(MemoryScope::Project, "user only", 10)
                .is_empty()
        );
        let handoff = memory.handoff(project).unwrap();
        assert_eq!(handoff.scope, MemoryScope::Project);
        assert_eq!(handoff.entries.len(), MAX_MEMORY_RESULTS);
        assert!(
            handoff
                .entries
                .iter()
                .all(|entry| entry.content.contains("validation:"))
        );
    }
}
