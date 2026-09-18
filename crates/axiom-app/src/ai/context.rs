//! Minimal, provider-independent context values for the active AI conversation.

pub(crate) const MAX_CONTEXT_MESSAGES: usize = 64;
pub(crate) const MAX_CONTEXT_SOURCES: usize = 8;
pub(crate) const MAX_CONTEXT_SOURCE_BYTES: usize = 32 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ContextRole {
    User,
    Assistant,
    System,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContextMessage {
    pub(crate) role: ContextRole,
    pub(crate) content: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContextSourceKind {
    ActiveFile,
    Selection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContextSource {
    pub(crate) kind: ContextSourceKind,
    pub(crate) label: String,
    pub(crate) content: String,
    pub(crate) truncated: bool,
}

impl ContextSource {
    pub(crate) fn new(
        kind: ContextSourceKind,
        label: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        let content = content.into();
        let (content, truncated) = truncate_utf8(&content, MAX_CONTEXT_SOURCE_BYTES);
        Self {
            kind,
            label: label.into(),
            content,
            truncated,
        }
    }
}

impl ContextMessage {
    pub(crate) fn new(role: ContextRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ContextSnapshot {
    pub(crate) messages: Vec<ContextMessage>,
    pub(crate) sources: Vec<ContextSource>,
}

impl ContextSnapshot {
    /// Keeps complete messages and retains the newest messages when bounded.
    /// An individual message is never cut; this intentionally is not token counting.
    pub(crate) fn from_messages<I>(messages: I) -> Self
    where
        I: IntoIterator<Item = ContextMessage>,
    {
        let mut messages: Vec<_> = messages.into_iter().collect();
        if messages.len() > MAX_CONTEXT_MESSAGES {
            let first = messages.len() - MAX_CONTEXT_MESSAGES;
            messages.drain(..first);
        }
        Self {
            messages,
            sources: Vec::new(),
        }
    }

    pub(crate) fn with_sources(mut self, sources: impl IntoIterator<Item = ContextSource>) -> Self {
        self.sources = bounded_sources(sources);
        self
    }

    pub(crate) fn user_request_with_sources(&self, user_request: &str) -> Option<String> {
        if self.sources.is_empty() {
            return None;
        }
        let mut text = String::from(
            "<axiom_explicit_context>\nThe user explicitly selected the following context for this request.\n\n",
        );
        for source in &self.sources {
            let kind = match source.kind {
                ContextSourceKind::ActiveFile => "ActiveFile",
                ContextSourceKind::Selection => "Selection",
            };
            text.push_str(&format!("Source: {}\nKind: {kind}\n", source.label));
            if source.truncated {
                text.push_str("Truncated: true\n");
            }
            text.push('\n');
            text.push_str(&source.content);
            text.push_str("\n\n");
        }
        text.push_str("</axiom_explicit_context>\n\n<axiom_user_request>\n");
        text.push_str(user_request);
        text.push_str("\n</axiom_user_request>");
        Some(text)
    }
}

fn bounded_sources<I>(sources: I) -> Vec<ContextSource>
where
    I: IntoIterator<Item = ContextSource>,
{
    let mut sources: Vec<_> = sources.into_iter().collect();
    if sources.len() > MAX_CONTEXT_SOURCES {
        let mut prioritized = sources
            .iter()
            .filter(|source| source.kind == ContextSourceKind::Selection)
            .cloned()
            .collect::<Vec<_>>();
        prioritized.extend(
            sources
                .into_iter()
                .filter(|source| source.kind != ContextSourceKind::Selection),
        );
        prioritized.truncate(MAX_CONTEXT_SOURCES);
        sources = prioritized;
    }
    sources
}

fn truncate_utf8(text: &str, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text.to_owned(), false);
    }
    let end = text
        .char_indices()
        .take_while(|(index, _)| *index < limit)
        .map(|(index, ch)| index + ch.len_utf8())
        .last()
        .unwrap_or(0)
        .min(limit);
    (text[..end].to_owned(), true)
}

#[cfg(test)]
mod tests {
    use super::{
        ContextMessage, ContextRole, ContextSnapshot, ContextSource, ContextSourceKind,
        MAX_CONTEXT_MESSAGES, MAX_CONTEXT_SOURCE_BYTES, MAX_CONTEXT_SOURCES,
    };

    fn user(content: &str) -> ContextMessage {
        ContextMessage::new(ContextRole::User, content)
    }

    #[test]
    fn empty_snapshot_is_empty() {
        assert_eq!(
            ContextSnapshot::from_messages(Vec::new()),
            ContextSnapshot::default()
        );
    }

    #[test]
    fn roles_order_unicode_and_multiline_are_preserved() {
        let snapshot = ContextSnapshot::from_messages([
            user("Olá, João 👋"),
            ContextMessage::new(ContextRole::Assistant, "linha 1\nlinha 2"),
        ]);
        assert_eq!(snapshot.messages[0].role, ContextRole::User);
        assert_eq!(snapshot.messages[0].content, "Olá, João 👋");
        assert_eq!(snapshot.messages[1].role, ContextRole::Assistant);
        assert_eq!(snapshot.messages[1].content, "linha 1\nlinha 2");
    }

    #[test]
    fn each_snapshot_isolated_and_limit_keeps_newest_complete_messages() {
        let first = ContextSnapshot::from_messages([user("chat A")]);
        let second = ContextSnapshot::from_messages([user("chat B")]);
        assert_eq!(first.messages[0].content, "chat A");
        assert_eq!(second.messages[0].content, "chat B");

        let messages = (0..MAX_CONTEXT_MESSAGES + 2)
            .map(|index| user(&format!("message {index}")))
            .collect::<Vec<_>>();
        let bounded = ContextSnapshot::from_messages(messages);
        assert_eq!(bounded.messages.len(), MAX_CONTEXT_MESSAGES);
        assert_eq!(bounded.messages[0].content, "message 2");
        assert_eq!(bounded.messages.last().unwrap().content, "message 65");
    }

    #[test]
    fn a_large_single_message_is_preserved_without_utf8_truncation() {
        let content = "😀".repeat(1024);
        let snapshot = ContextSnapshot::from_messages([user(&content)]);
        assert_eq!(snapshot.messages[0].content, content);
    }

    #[test]
    fn sources_are_bounded_and_serialized_deterministically() {
        let snapshot = ContextSnapshot::from_messages([user("question")]).with_sources([
            ContextSource::new(ContextSourceKind::ActiveFile, "src/main.rs", "fn main() {}"),
            ContextSource::new(
                ContextSourceKind::Selection,
                "src/main.rs:1-1",
                "fn main() {}",
            ),
        ]);
        assert_eq!(snapshot.sources.len(), 2);
        let text = snapshot.user_request_with_sources("question").unwrap();
        assert!(text.contains("Source: src/main.rs"));
        assert!(text.contains("Source: src/main.rs:1-1"));
        assert!(text.contains("fn main() {}"));
    }

    #[test]
    fn large_source_truncates_on_utf8_boundary_and_marks_metadata() {
        let source = ContextSource::new(
            ContextSourceKind::ActiveFile,
            "large.php",
            "😀".repeat(MAX_CONTEXT_SOURCE_BYTES),
        );
        assert!(source.truncated);
        assert!(source.content.len() <= MAX_CONTEXT_SOURCE_BYTES);
        assert!(source.content.is_char_boundary(source.content.len()));
        assert!(user_request_contains_truncated(&source));
    }

    fn user_request_contains_truncated(source: &ContextSource) -> bool {
        ContextSnapshot::default()
            .with_sources([source.clone()])
            .user_request_with_sources("question")
            .unwrap()
            .contains("Truncated: true")
    }

    #[test]
    fn selection_sources_have_priority_when_source_count_exceeds_limit() {
        let mut sources = (0..MAX_CONTEXT_SOURCES)
            .map(|index| {
                ContextSource::new(ContextSourceKind::ActiveFile, format!("{index}.php"), "x")
            })
            .collect::<Vec<_>>();
        sources.push(ContextSource::new(
            ContextSourceKind::Selection,
            "selected.php",
            "y",
        ));
        let snapshot = ContextSnapshot::default().with_sources(sources);
        assert_eq!(snapshot.sources.len(), MAX_CONTEXT_SOURCES);
        assert!(
            snapshot
                .sources
                .iter()
                .any(|source| source.kind == ContextSourceKind::Selection)
        );
    }
}
