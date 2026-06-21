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
}
