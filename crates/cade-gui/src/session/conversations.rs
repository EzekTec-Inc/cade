//! Conversation-list state for [`super::SessionState`].

use super::*;

impl SessionState {
    /// Store conversations fetched from the server.
    pub fn on_conversations(&mut self, convs: Vec<crate::api::ConversationInfo>) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { conversations, .. } = &mut **session;
            *conversations = convs;
        }
    }

    /// The current list of conversations.
    pub fn conversations(&self) -> &[crate::api::ConversationInfo] {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { conversations, .. } = &**session;
            conversations
        } else {
            &[]
        }
    }

    /// Currently selected conversation index.
    pub fn selected_conversation(&self) -> Option<usize> {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession {
                selected_conversation,
                ..
            } = &**session;
            *selected_conversation
        } else {
            None
        }
    }

    /// Select a conversation by index.  Returns `true` if the selection
    /// changed.  When changed, clears messages and sets conversation_id
    /// so the caller can re-fetch messages for that conversation.
    pub fn on_select_conversation(&mut self, idx: usize) -> bool {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession {
                conversations,
                selected_conversation,
                messages,
                conversation_id,
                ..
            } = &mut **session;
            if idx >= conversations.len() {
                return false;
            }
            if *selected_conversation == Some(idx) {
                return false;
            }
            *selected_conversation = Some(idx);
            *conversation_id = Some(conversations[idx].id.clone());
            messages.clear();
            true
        } else {
            false
        }
    }

    /// Start a fresh conversation — clears conversation_id, messages,
    /// and selected_conversation so the next send creates a new one on
    /// the server.
    pub fn on_new_conversation(&mut self) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession {
                conversation_id,
                messages,
                selected_conversation,
                ..
            } = &mut **session;
            *conversation_id = None;
            messages.clear();
            *selected_conversation = None;
        }
    }

    /// Remove a conversation at `idx` from the local list.
    ///
    /// If the deleted conversation was selected, the selection is cleared and
    /// `messages` / `conversation_id` are reset so the user starts fresh.
    pub fn on_conversation_deleted(&mut self, idx: usize) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession {
                conversations,
                selected_conversation,
                messages,
                conversation_id,
                ..
            } = &mut **session;
            if idx >= conversations.len() {
                return;
            }
            conversations.remove(idx);
            match *selected_conversation {
                Some(sel) if sel == idx => {
                    // Deleted the currently-active conversation — reset.
                    *selected_conversation = None;
                    *conversation_id = None;
                    messages.clear();
                }
                Some(sel) if sel > idx => {
                    // Shift selection down by one to keep it pointing at the
                    // same conversation (which moved up in the list).
                    *selected_conversation = Some(sel - 1);
                }
                _ => {}
            }
        }
    }
}
