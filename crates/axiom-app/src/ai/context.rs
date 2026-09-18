//! Minimal, provider-independent context values for the active AI conversation.

pub(crate) const MAX_CONTEXT_MESSAGES: usize = 64;

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
        Self { messages }
    }
}

#[cfg(test)]
mod tests {
    use super::{ContextMessage, ContextRole, ContextSnapshot, MAX_CONTEXT_MESSAGES};

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
}
