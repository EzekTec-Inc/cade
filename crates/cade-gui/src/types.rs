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
#[derive(Clone, Copy)]
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
