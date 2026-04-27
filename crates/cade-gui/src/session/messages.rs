//! Message-list and send-input state for [`super::SessionState`].

use super::*;

impl SessionState {
    /// Feed the message list fetched for the currently selected agent.
    ///
    /// Only applies when `Connected` and an agent is selected.  No-op
    /// otherwise.
    pub fn on_messages(&mut self, msgs: Vec<ChatMessage>) {
        if let Self::Connected { messages, .. } = self {
            *messages = msgs;
        }
    }

    /// Set messages and pagination flag from a paged fetch.
    pub fn on_messages_paged(&mut self, msgs: Vec<ChatMessage>, has_more: bool) {
        if let Self::Connected {
            messages,
            has_more_messages,
            ..
        } = self
        {
            *messages = msgs;
            *has_more_messages = has_more;
        }
    }

    /// Prepend older messages (from "Load more") to the beginning.
    pub fn on_prepend_messages(&mut self, older: Vec<ChatMessage>, has_more: bool) {
        if let Self::Connected {
            messages,
            has_more_messages,
            ..
        } = self
        {
            let mut combined = older;
            combined.append(messages);
            *messages = combined;
            *has_more_messages = has_more;
        }
    }

    /// Whether there are older messages to load.
    pub fn has_more_messages(&self) -> bool {
        if let Self::Connected {
            has_more_messages, ..
        } = self
        {
            *has_more_messages
        } else {
            false
        }
    }

    /// Current message count (used as offset for pagination).
    pub fn message_count(&self) -> usize {
        if let Self::Connected { messages, .. } = self {
            messages.len()
        } else {
            0
        }
    }

    /// Submit the current input buffer as a user message.
    ///
    /// Returns the trimmed input text if the send is valid (agent selected,
    /// non-empty buffer, not already streaming).  Returns `None` if it's a
    /// no-op.  On success:
    ///   1. Appends a `ChatMessage { role: "user", content: input }` to messages.
    ///   2. Clears the input buffer.
    ///   3. Sets `streaming = true`.
    pub fn on_send(&mut self) -> Option<String> {
        if let Self::Connected {
            selected_agent: Some(_),
            input_buffer,
            messages,
            streaming,
            last_usage,
            last_finish_reason,
            ..
        } = self
        {
            if *streaming {
                return None;
            }
            let trimmed = input_buffer.trim().to_string();
            if trimmed.is_empty() {
                return None;
            }
            messages.push(ChatMessage {
                id: String::new(), // server assigns a real ID
                role: "user".to_string(),
                content: serde_json::Value::String(trimmed.clone()),
                conversation_id: None,
            });
            input_buffer.clear();
            *streaming = true;
            *last_usage = None;
            *last_finish_reason = None;
            Some(trimmed)
        } else {
            None
        }
    }

    /// Clear the local timeline display only.  Does NOT touch the
    /// server — reselecting the agent or sending a message will refetch.
    pub fn clear_timeline_local(&mut self) {
        if let Self::Connected { messages, .. } = self {
            messages.clear();
        }
    }

    /// Return the content of the most recent assistant message, if any.
    /// Used by the `/copy` palette command.
    pub fn last_assistant_content(&self) -> Option<String> {
        if let Self::Connected { messages, .. } = self {
            messages
                .iter()
                .rev()
                .find(|m| m.role == "assistant")
                .map(|m| match &m.content {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
        } else {
            None
        }
    }

}
