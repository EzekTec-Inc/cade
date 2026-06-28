use dioxus::prelude::*;

/// Top-level navigation pages.
#[derive(Clone, Copy, PartialEq)]
pub enum SelectedPage {
    Dashboard,
    Code,
    Chat,
    Agents,
    Logs,
    MemoryBlocks,
    Tools,
    Models,
    Providers,
    ApiKeys,
    Usage,
    Settings,
}

/// Code language selector for API examples.
#[derive(Clone, Copy, PartialEq)]
pub enum CodeLanguage {
    Javascript,
    Python,
    Curl,
}

/// A transient toast notification.
#[derive(Clone, PartialEq)]
pub struct ToastMessage {
    pub id: u64,
    pub level: ToastLevel,
    pub title: String,
    pub detail: String,
}

#[derive(Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
}

/// Shared application state, provided via Dioxus context.
///
/// Fields are `Signal<T>` — a `Clone`-and-`Copy` smart pointer so the
/// whole struct derives `Clone` and can be cheaply shared across
/// components via `use_context`.


#[derive(Clone, Default)]
pub struct SafeAbortHandle(pub std::sync::Arc<std::sync::atomic::AtomicBool>);

impl PartialEq for SafeAbortHandle {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Clone, Copy, PartialEq)]
pub struct AppState {
    pub api_key: Signal<String>,
    pub active_page: Signal<SelectedPage>,
    pub selected_agent: Signal<Option<cade_api_types::AgentInfo>>,
    pub messages: Signal<Vec<cade_api_types::ChatMessage>>,
    pub input_text: Signal<String>,
    pub is_loading: Signal<bool>,
    pub conversations: Signal<Vec<cade_api_types::ConversationInfo>>,
    pub active_conversation: Signal<Option<String>>,
    pub toasts: Signal<Vec<ToastMessage>>,
    pub global_error: Signal<Option<String>>,
    pub active_stream_id: Signal<Option<String>>,
    pub active_stream: Signal<SafeAbortHandle>,
}

/// Helper: push a toast notification into global state.
pub fn add_toast(state: &AppState, level: ToastLevel, title: impl Into<String>, detail: impl Into<String>) {
    let mut toasts = state.toasts;
    let mut list = toasts();
    list.push(ToastMessage {
        id: js_sys::Date::now() as u64,
        level,
        title: title.into(),
        detail: detail.into(),
    });
    toasts.set(list);
}

#[derive(Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub struct AppSessionStore {
    pub state: AppState,
}

#[allow(dead_code)]
impl AppSessionStore {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    pub fn add_toast(&self, level: ToastLevel, title: impl Into<String>, detail: impl Into<String>) {
        let mut toasts = self.state.toasts;
        let mut list = toasts();
        list.push(ToastMessage {
            id: js_sys::Date::now() as u64,
            level,
            title: title.into(),
            detail: detail.into(),
        });
        toasts.set(list);
    }

    pub fn set_api_key(&self, key: String) {
        let mut sig = self.state.api_key;
        sig.set(key);
    }

    pub fn set_active_page(&self, page: SelectedPage) {
        let mut sig = self.state.active_page;
        sig.set(page);
    }

    pub fn set_selected_agent(&self, agent: Option<cade_api_types::AgentInfo>) {
        let mut sig = self.state.selected_agent;
        sig.set(agent);
    }

    pub fn set_active_conversation(&self, conv_id: Option<String>) {
        let mut sig = self.state.active_conversation;
        sig.set(conv_id);
    }

    pub fn set_input_text(&self, text: String) {
        let mut sig = self.state.input_text;
        sig.set(text);
    }
}
