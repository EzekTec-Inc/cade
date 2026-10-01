//! Command state transitions shared by interactive selection and queued input.
//! No terminal is required to use or verify these production transitions.
use cade_agent::agent::{client::AgentState, session::SessionStore};
use parking_lot::Mutex;
use std::collections::VecDeque;

pub(super) struct CommandSession<'a> {
    pub agent_id: &'a Mutex<String>,
    pub agent_name: &'a Mutex<String>,
    pub conversation_id: &'a Mutex<Option<String>>,
    pub store: &'a Mutex<SessionStore>,
}

impl CommandSession<'_> {
    pub fn select_conversation(&self, selected: Option<String>) -> crate::Result<()> {
        if selected.as_deref().is_some_and(|id| id.trim().is_empty()) {
            return Err(crate::Error::custom(
                "Server did not return a conversation ID",
            ));
        }
        let mut conversation = self.conversation_id.lock();
        let mut store = self.store.lock();
        let previous = store.session.clone();
        store.session.conversation_id = selected.clone();
        store.session.run_id = None;
        store.session.last_seq_id = None;
        if let Err(error) = store.save() {
            store.session = previous;
            return Err(error.into());
        }
        *conversation = selected;
        Ok(())
    }

    pub fn select_agent(&self, agent: &AgentState) -> crate::Result<()> {
        let mut id = self.agent_id.lock();
        let mut name = self.agent_name.lock();
        let mut conversation = self.conversation_id.lock();
        let mut store = self.store.lock();
        let previous = store.session.clone();
        let changed = *id != agent.id;
        store.session.agent_id = Some(agent.id.clone());
        store.session.agent_name = Some(agent.name.clone());
        if changed {
            store.session.conversation_id = None;
            store.session.run_id = None;
            store.session.last_seq_id = None;
        }
        if let Err(error) = store.save() {
            store.session = previous;
            return Err(error.into());
        }
        *id = agent.id.clone();
        *name = agent.name.clone();
        if changed {
            *conversation = None;
        }
        Ok(())
    }
}

/// Admit the next command before waiting for new terminal input. This is called
/// on every REPL iteration, including after slash/Lua/template early continues.
pub(super) fn next_input(
    pending: &mut Option<String>,
    followups: &Mutex<VecDeque<String>>,
    steering: &Mutex<Option<String>>,
) -> Option<String> {
    pending
        .take()
        .or_else(|| followups.lock().pop_front())
        .or_else(|| steering.lock().take())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str) -> AgentState {
        AgentState {
            id: id.into(),
            name: format!("Agent {id}"),
            model: None,
            description: None,
            system_prompt: None,
        }
    }

    #[test]
    fn selecting_an_agent_clears_foreign_conversation_and_recovery_cursor_live_and_on_disk() {
        let workspace = tempfile::tempdir().unwrap();
        let mut store = SessionStore::load(workspace.path());
        store.set_agent("a".into(), Some("Agent a".into())).unwrap();
        store
            .set_conversation(Some("conversation-a".into()))
            .unwrap();
        store.set_run(Some("run-a".into()), Some(42)).unwrap();
        let store = Mutex::new(store);
        let id = Mutex::new("a".into());
        let name = Mutex::new("Agent a".into());
        let conversation = Mutex::new(Some("conversation-a".into()));
        let session = CommandSession {
            agent_id: &id,
            agent_name: &name,
            conversation_id: &conversation,
            store: &store,
        };
        session.select_agent(&agent("b")).unwrap();
        assert_eq!(*id.lock(), "b");
        assert_eq!(*name.lock(), "Agent b");
        assert_eq!(
            *conversation.lock(),
            None,
            "next Run must not submit a's conversation with b"
        );
        let reloaded = SessionStore::load(workspace.path());
        assert_eq!(reloaded.session.agent_id.as_deref(), Some("b"));
        assert_eq!(reloaded.session.conversation_id, None);
        assert_eq!(reloaded.session.run_id, None);
        assert_eq!(reloaded.session.last_seq_id, None);
    }

    #[test]
    fn failed_selection_does_not_publish_a_partial_live_identity() {
        let workspace = tempfile::tempdir().unwrap();
        let mut store = SessionStore::load(workspace.path());
        store.session.agent_id = Some("a".into());
        store.session.conversation_id = Some("conversation-a".into());
        std::fs::write(workspace.path().join(".cade"), "not a directory").unwrap();
        let store = Mutex::new(store);
        let id = Mutex::new("a".into());
        let name = Mutex::new("Agent a".into());
        let conversation = Mutex::new(Some("conversation-a".into()));
        let session = CommandSession {
            agent_id: &id,
            agent_name: &name,
            conversation_id: &conversation,
            store: &store,
        };
        assert!(session.select_agent(&agent("b")).is_err());
        assert_eq!(*id.lock(), "a");
        assert_eq!(*conversation.lock(), Some("conversation-a".into()));
        assert_eq!(store.lock().session.agent_id.as_deref(), Some("a"));
    }

    #[test]
    fn queued_slash_command_cannot_strand_the_next_prompt_or_steering() {
        let followups = Mutex::new(VecDeque::from(["/info".into(), "continue the task".into()]));
        let steering = Mutex::new(Some("use the other approach".into()));
        let mut pending = Some("/help".into());
        assert_eq!(
            next_input(&mut pending, &followups, &steering).as_deref(),
            Some("/help")
        );
        assert_eq!(
            next_input(&mut pending, &followups, &steering).as_deref(),
            Some("/info")
        );
        assert_eq!(
            next_input(&mut pending, &followups, &steering).as_deref(),
            Some("continue the task")
        );
        assert_eq!(
            next_input(&mut pending, &followups, &steering).as_deref(),
            Some("use the other approach")
        );
        assert_eq!(next_input(&mut pending, &followups, &steering), None);
    }

    #[test]
    fn conversation_selection_resets_recovery_but_reselecting_agent_preserves_conversation() {
        let workspace = tempfile::tempdir().unwrap();
        let store = Mutex::new(SessionStore::load(workspace.path()));
        let id = Mutex::new("a".into());
        let name = Mutex::new("Agent a".into());
        let conversation = Mutex::new(None);
        let session = CommandSession {
            agent_id: &id,
            agent_name: &name,
            conversation_id: &conversation,
            store: &store,
        };
        session.select_agent(&agent("a")).unwrap();
        session
            .select_conversation(Some("conversation-a".into()))
            .unwrap();
        store.lock().set_run(Some("run-a".into()), Some(1)).unwrap();
        session.select_agent(&agent("a")).unwrap();
        assert_eq!(conversation.lock().as_deref(), Some("conversation-a"));
        assert_eq!(store.lock().session.run_id.as_deref(), Some("run-a"));
        assert!(session.select_conversation(Some("".into())).is_err());
        assert_eq!(conversation.lock().as_deref(), Some("conversation-a"));
        session
            .select_conversation(Some("conversation-b".into()))
            .unwrap();
        assert_eq!(conversation.lock().as_deref(), Some("conversation-b"));
        let reloaded = SessionStore::load(workspace.path());
        assert_eq!(
            reloaded.session.conversation_id.as_deref(),
            Some("conversation-b")
        );
        assert_eq!(reloaded.session.run_id, None);
    }
}
