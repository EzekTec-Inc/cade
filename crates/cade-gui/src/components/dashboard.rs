use dioxus::prelude::*;

use crate::api_engine::{ApiClientEngine, ConsoleStatus, ResourceState};
use crate::primitives::{FeatureTile, TabPill, TileAccent, TileIcon};
use crate::types::{AppState, CodeLanguage, SelectedPage};

/// Dashboard home page with modern developer tool aesthetic.
#[component]
pub fn DashboardView() -> Element {
    let state = use_context::<AppState>();
    let engine = use_context::<Memo<ApiClientEngine>>();
    let mut console_status = use_signal(|| ResourceState::<ConsoleStatus>::Loading);

    let mut active_tab = use_signal(|| 0);
    let selected_lang = use_signal(|| CodeLanguage::Rust);
    let copied_key = use_signal(|| false);
    let copied_code = use_signal(|| false);

    let agent_id_opt = (state.selected_agent)().map(|a| a.id);
    use_effect(move || {
        let eng = engine();
        let agent_id = agent_id_opt.clone();
        spawn(async move {
            let res = eng.fetch_console_status(agent_id.as_deref()).await;
            console_status.set(res);
        });
    });

    let status_state = console_status.read().clone();
    let status_error = status_state.error().map(str::to_owned);
    let status_val = status_state.value().cloned().unwrap_or_else(ConsoleStatus::unknown);
    let (pill_cls, dot_cls, pill_label) = match status_state {
        ResourceState::Loading => (
            "bg-slate-900 text-slate-300 border-slate-700",
            "bg-slate-400 animate-pulse",
            "Checking engine".to_string(),
        ),
        ResourceState::Error(_) => (
            "bg-rose-950/80 text-rose-400 border-rose-800/80",
            "bg-rose-400",
            "Status unavailable".to_string(),
        ),
        ResourceState::Ready(status) if status.is_healthy => (
            "bg-emerald-950/80 text-emerald-400 border-emerald-800/80",
            "bg-emerald-400",
            status.engine_status,
        ),
        ResourceState::Ready(status) => (
            "bg-rose-950/80 text-rose-400 border-rose-800/80",
            "bg-rose-400",
            status.engine_status,
        ),
    };
    let mcp_label = if status_error.is_some() {
        "Unavailable".to_string()
    } else if status_val.configured_mcp_count == 0 {
        "None configured".to_string()
    } else {
        format!(
            "{} / {} active",
            status_val.active_mcp_count, status_val.configured_mcp_count
        )
    };

    let (tab_title, tab_desc, tab_link, tab_href) = match active_tab() {
        0 => (
            "Send a message to an agent",
            "Make an API request to send your stateful agent a message.",
            "Get started with the API",
            "https://github.com/EzekTec-Inc/cade/blob/main/docs/getting-started.md",
        ),
        1 => (
            "Create an agent",
            "Deploy a new autonomous agent with custom system prompts, tools, and persona.",
            "Explore agent creation",
            "https://github.com/EzekTec-Inc/cade/blob/main/docs/agents-and-conversations.md",
        ),
        _ => (
            "Inspect agent memory",
            "Retrieve and inspect the stateful core memory blocks of an active agent.",
            "Read about memory state",
            "https://github.com/EzekTec-Inc/cade/blob/main/docs/memory-system.md",
        ),
    };

    let code_content = code_for_tab(active_tab(), selected_lang());
    let active_page = state.active_page;

    rsx! {
        // Header bar with Glassmorphism & Status Beacon
        header { class: "px-10 py-4 flex items-center justify-between select-none border-b border-[#1e293b]/70 bg-[#090d16]/90 backdrop-blur-md sticky top-0 z-10",
            div { class: "flex items-center space-x-3",
                div { class: "w-2 h-2 rounded-full bg-cyan-400 animate-ping" }
                span { class: "text-xs font-mono font-bold text-cyan-400 uppercase tracking-widest", "CADE Autonomous Intelligence Platform" }
            }
            div { class: "flex items-center space-x-6 text-[13px] text-slate-400 font-medium",
                a { href: "https://github.com/EzekTec-Inc/cade/blob/main/docs/index.md", target: "_blank", class: "hover:text-slate-100 cursor-pointer transition-colors duration-150", "Docs" }
                a { href: "https://github.com/EzekTec-Inc/cade/blob/main/docs/getting-started.md", target: "_blank", class: "hover:text-slate-100 cursor-pointer transition-colors duration-150", "API Spec" }
                span { class: "{pill_cls} border px-3 py-1 rounded-full text-xs font-semibold shadow-sm flex items-center space-x-2",
                    span { class: "w-2 h-2 rounded-full {dot_cls}" }
                    span { "{pill_label}" }
                }
            }
        }

        // Dashboard Content
        div { class: "px-10 pt-8 pb-12 flex-1 overflow-y-auto bg-[#040711]",
            // Greeting & Live Telemetry Ticker
            div { class: "mb-8 flex flex-col lg:flex-row lg:items-center justify-between gap-6 border-b border-[#1e293b]/60 pb-6",
                div {
                    h1 { class: "text-2xl font-bold text-slate-100 tracking-tight text-balance flex items-center space-x-3",
                        span { "Executive AI Intelligence Console" }
                    }
                    p { class: "text-xs text-slate-400 mt-1 text-pretty max-w-xl", "Real-time stateful autonomous agent mesh, multi-model arena, and AST refactoring harness." }
                }
                div { class: "flex flex-wrap items-center gap-2.5 select-none",
                    div { class: "bg-[#090d16] border border-slate-800/90 rounded-xl px-3.5 py-2 flex flex-col items-center shadow-[0px_2px_3px_-1px_rgba(0,0,0,0.1),0px_1px_0px_0px_rgba(25,28,33,0.02),0px_0px_0px_1px_rgba(25,28,33,0.08)]",
                        span { class: "text-[10px] uppercase text-slate-500 font-mono", "Provider / Model" }
                        span { class: "text-xs font-semibold text-sky-400 font-mono", "{status_val.provider} / {status_val.default_model}" }
                    }
                    div { class: "bg-[#090d16] border border-slate-800/90 rounded-xl px-3.5 py-2 flex flex-col items-center shadow-[0px_2px_3px_-1px_rgba(0,0,0,0.1),0px_1px_0px_0px_rgba(25,28,33,0.02),0px_0px_0px_1px_rgba(25,28,33,0.08)]",
                        span { class: "text-[10px] uppercase text-slate-500 font-mono", "Model Context" }
                        span { class: "text-xs font-semibold text-sky-400 font-mono tabular-nums", "{status_val.context_window}" }
                    }
                    div { class: "bg-[#090d16] border border-slate-800/90 rounded-xl px-3.5 py-2 flex flex-col items-center shadow-[0px_2px_3px_-1px_rgba(0,0,0,0.1),0px_1px_0px_0px_rgba(25,28,33,0.02),0px_0px_0px_1px_rgba(25,28,33,0.08)]",
                        span { class: "text-[10px] uppercase text-slate-500 font-mono", "Recall Seam" }
                        span { class: "text-xs font-semibold text-slate-300 font-mono", "{status_val.recall_backend}" }
                    }
                    div { class: "bg-[#090d16] border border-slate-800/90 rounded-xl px-3.5 py-2 flex flex-col items-center shadow-[0px_2px_3px_-1px_rgba(0,0,0,0.1),0px_1px_0px_0px_rgba(25,28,33,0.02),0px_0px_0px_1px_rgba(25,28,33,0.08)]",
                        span { class: "text-[10px] uppercase text-slate-500 font-mono", "MCP Status" }
                        span { class: "text-xs font-semibold text-purple-400 font-mono tabular-nums", "{mcp_label}" }
                    }
                }
                if let Some(error) = status_error {
                    p { class: "mt-3 text-xs text-rose-300", "Live status could not be verified: {error}" }
                }
            }

            // Quick Action Launchpad Cards (Surpassing Abacus.ai)
            div { class: "mb-10 select-none",
                div { class: "flex items-center justify-between mb-3",
                    h2 { class: "text-xs font-bold uppercase tracking-wider text-slate-400 font-mono", "Autonomous Action Launchpad" }
                    span { class: "text-[11px] text-slate-500 font-mono", "Single-click operations" }
                }
                div { class: "grid grid-cols-1 md:grid-cols-4 gap-4",
                    ActionCard {
                        icon: "⚡",
                        tag: "Arena Battle",
                        title: "Multi-Model Arena",
                        description: "Stream 2-4 models side-by-side with latency & diff analysis.",
                        accent: TileAccent::Sky,
                        destination: SelectedPage::Arena,
                        active_page,
                    }
                    ActionCard {
                        icon: "🔄",
                        tag: "DAG Visualizer",
                        title: "Workflows & Pipelines",
                        description: "Visual DAG canvas with animated step execution pulses.",
                        accent: TileAccent::Purple,
                        destination: SelectedPage::Workflows,
                        active_page,
                    }
                    ActionCard {
                        icon: "🌐",
                        tag: "Swarm Tree",
                        title: "Swarm Topology",
                        description: "Inspect supervisory trees, subagents, and token metrics.",
                        accent: TileAccent::Emerald,
                        destination: SelectedPage::Swarm,
                        active_page,
                    }
                    ActionCard {
                        icon: "📦",
                        tag: "Artifacts",
                        title: "Artifact Studio",
                        description: "Inspect generated code diffs, datasets, and markdown.",
                        accent: TileAccent::Amber,
                        destination: SelectedPage::Artifacts,
                        active_page,
                    }
                }
            }

            // Feature cards grid (using deep FeatureTile primitives)
            div { class: "grid grid-cols-1 md:grid-cols-3 gap-6 mb-10 select-none",
                FeatureTile {
                    title: "Desktop Extensions",
                    description: "Cross-platform desktop automation, screen capture, window control, and notification hooks.",
                    badge: "Native",
                    action_label: "Explore Tools & Approvals",
                    icon: TileIcon::Desktop,
                    accent: TileAccent::Sky,
                    destination: SelectedPage::Tools,
                    active_page,
                }
                FeatureTile {
                    title: "Smart Shell & CLI",
                    description: "Interactive Ratatui TUI console with autonomous workflows, plan checklists, and diff inspectors.",
                    badge: "Terminal",
                    action_label: "Inspect Live Activity",
                    icon: TileIcon::Terminal,
                    accent: TileAccent::Emerald,
                    destination: SelectedPage::Live,
                    active_page,
                }
                FeatureTile {
                    title: "Embedded SDK",
                    description: "Zero-daemon in-process runtime (Rust, Python, TypeScript) for standalone and serverless deployments.",
                    badge: "SDK",
                    action_label: "Open API Playground",
                    icon: TileIcon::Sdk,
                    accent: TileAccent::Amber,
                    destination: SelectedPage::Code,
                    active_page,
                }
            }

            // API Section
            div { class: "flex items-center justify-between mb-4",
                h2 { class: "text-base font-bold text-slate-100 tracking-tight text-balance", "Developer API Workbench" }
                span { class: "text-xs text-slate-500 font-mono", "REST / SSE / In-Process" }
            }

            div { class: "border border-slate-800/90 bg-[#090d16] rounded-xl overflow-hidden shadow-[0px_0px_0px_1px_rgba(0,0,0,0.06),0px_1px_1px_-0.5px_rgba(0,0,0,0.06),0px_3px_3px_-1.5px_rgba(0,0,0,0.06),_0px_6px_6px_-3px_rgba(0,0,0,0.06),0px_12px_12px_-6px_rgba(0,0,0,0.06),0px_24px_24px_-12px_rgba(0,0,0,0.06)] flex flex-col",
                // Tab navigation (using deep TabPill primitives)
                div { class: "px-6 py-3 border-b border-slate-800/80 flex items-center justify-between select-none text-[13px] font-medium text-slate-400 bg-[#0f172a]/40",
                    div { class: "flex items-center space-x-2",
                        TabPill {
                            active: active_tab() == 0,
                            label: "Send Message".to_string(),
                            onclick: move |_| active_tab.set(0),
                        }
                        TabPill {
                            active: active_tab() == 1,
                            label: "Deploy Agent".to_string(),
                            onclick: move |_| active_tab.set(1),
                        }
                        TabPill {
                            active: active_tab() == 2,
                            label: "Inspect Memory".to_string(),
                            onclick: move |_| active_tab.set(2),
                        }
                    }
                    // API Key widget
                    api_key_widget { copied_key: copied_key, api_key: state.api_key }
                }

                // Main block split section
                div { class: "grid grid-cols-1 md:grid-cols-12 min-h-[300px]",
                    // Left column - description
                    div { class: "md:col-span-4 p-8 border-r border-slate-800/80 flex flex-col justify-between bg-[#070b14]",
                        div {
                            h3 { class: "text-slate-100 text-base font-bold mb-3 tracking-tight text-balance", "{tab_title}" }
                            p { class: "text-slate-400 text-xs leading-relaxed text-pretty", "{tab_desc}" }
                        }
                        a {
                            href: "{tab_href}",
                            target: "_blank",
                            class: "inline-flex items-center space-x-2 text-xs font-medium text-slate-200 border border-slate-700 bg-slate-800/80 hover:bg-slate-700 hover:text-white py-2 px-3.5 rounded-lg w-fit shadow-sm transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-sky-400 outline-none",
                            span { "{tab_link}" }
                            span { class: "text-[10px] text-slate-400", "↗" }
                        }
                    }

                    // Right column - code display
                    code_panel {
                        selected_lang: selected_lang,
                        copied_code: copied_code,
                        code_content: code_content
                    }
                }
            }
        }
    }
}

// ── Sub-components ─────────────────────────────────────────────────────────

/// Deep ActionCard component encapsulating Launchpad quick-action navigation,
/// accessibility (role="button", tabindex, keyboard activation), and themed hover states.
#[component]
fn ActionCard(
    icon: &'static str,
    tag: &'static str,
    title: &'static str,
    description: &'static str,
    accent: TileAccent,
    destination: SelectedPage,
    mut active_page: Signal<SelectedPage>,
) -> Element {
    let border_hover = match accent {
        TileAccent::Sky => "hover:border-cyan-500/60 focus-visible:border-cyan-400 hover:shadow-cyan-500/10",
        TileAccent::Purple => "hover:border-purple-500/60 focus-visible:border-purple-400 hover:shadow-purple-500/10",
        TileAccent::Emerald => "hover:border-emerald-500/60 focus-visible:border-emerald-400 hover:shadow-emerald-500/10",
        TileAccent::Amber => "hover:border-amber-500/60 focus-visible:border-amber-400 hover:shadow-amber-500/10",
    };
    let tag_cls = match accent {
        TileAccent::Sky => "text-cyan-400 bg-cyan-950/80 border-cyan-800",
        TileAccent::Purple => "text-purple-400 bg-purple-950/80 border-purple-800",
        TileAccent::Emerald => "text-emerald-400 bg-emerald-950/80 border-emerald-800",
        TileAccent::Amber => "text-amber-400 bg-amber-950/80 border-amber-800",
    };
    let text_hover = match accent {
        TileAccent::Sky => "group-hover:text-cyan-300",
        TileAccent::Purple => "group-hover:text-purple-300",
        TileAccent::Emerald => "group-hover:text-emerald-300",
        TileAccent::Amber => "group-hover:text-amber-300",
    };

    rsx! {
        div {
            class: "bg-gradient-to-br from-[#090d16] to-[#0f172a] border border-[#1e293b] {border_hover} rounded-xl p-5 cursor-pointer group transition-all duration-200 shadow-lg select-none outline-none focus-visible:ring-2 focus-visible:ring-cyan-400",
            role: "button",
            tabindex: 0,
            "aria-label": "{title}: {description}",
            onclick: move |_| active_page.set(destination),
            onkeydown: move |evt: KeyboardEvent| {
                if evt.key() == Key::Enter || evt.key() == Key::Character(" ".to_string()) {
                    active_page.set(destination);
                }
            },
            div { class: "flex items-center justify-between mb-2",
                span { class: "text-lg", "{icon}" }
                span { class: "text-[10px] font-mono font-bold px-2 py-0.5 rounded border {tag_cls}", "{tag}" }
            }
            h3 { class: "text-slate-100 font-bold text-sm {text_hover} transition-colors", "{title}" }
            p { class: "text-slate-400 text-xs mt-1", "{description}" }
        }
    }
}

/// API key display widget with copy button and visual feedback.
#[component]
fn api_key_widget(mut copied_key: Signal<bool>, api_key: Signal<String>) -> Element {
    let key_val = api_key();
    let has_key = !key_val.is_empty();

    rsx! {
        div { class: "flex items-center space-x-2 bg-[#090d16] border border-[#1e293b] py-1 px-2.5 rounded-lg text-xs select-none",
            span { class: "text-slate-500 font-medium", "API Key:" }
            span { class: "text-slate-300 font-mono text-[11px] tracking-wider",
                if key_val.len() > 8 {
                    "{&key_val[..8]}...{&key_val[key_val.len()-4..]}"
                } else if has_key {
                    "{key_val}"
                } else {
                    "No Key Required"
                }
            }
            if has_key {
                button {
                    class: "text-[11px] text-slate-400 hover:text-white px-1.5 py-0.5 rounded bg-slate-800 border border-slate-700 hover:bg-slate-700 transition-colors ml-1 cursor-pointer",
                    title: "Copy API Key",
                    onclick: move |_| {
                        let to_copy = key_val.clone();
                        crate::api::copyText(&to_copy);
                        copied_key.set(true);
                        spawn(async move {
                            gloo_timers::future::TimeoutFuture::new(2000).await;
                            copied_key.set(false);
                        });
                    },
                    span { if copied_key() { "✓" } else { "Copy" } }
                }
            }
        }
    }
}

/// Code panel component displaying snippet in selected language with copy feedback.
#[component]
fn code_panel(
    selected_lang: Signal<CodeLanguage>,
    mut copied_code: Signal<bool>,
    code_content: String,
) -> Element {
    let text_to_copy = code_content.clone();

    rsx! {
        div { class: "md:col-span-8 p-6 flex flex-col justify-between bg-[#040711]",
            div { class: "flex items-center justify-between mb-3 border-b border-[#1e293b]/50 pb-2.5",
                div { class: "flex items-center space-x-2",
                    lang_button { selected_lang: selected_lang, lang: CodeLanguage::Rust, label: "Rust" }
                    lang_button { selected_lang: selected_lang, lang: CodeLanguage::Javascript, label: "Node.js" }
                    lang_button { selected_lang: selected_lang, lang: CodeLanguage::Python, label: "Python" }
                    lang_button { selected_lang: selected_lang, lang: CodeLanguage::Curl, label: "cURL" }
                }
                button {
                    class: "text-xs text-slate-400 hover:text-white flex items-center space-x-1.5 bg-slate-800/80 px-2.5 py-1 rounded-md border border-slate-700/60 transition-colors cursor-pointer",
                    onclick: move |_| {
                        let to_copy = text_to_copy.clone();
                        crate::api::copyText(&to_copy);
                        copied_code.set(true);
                        spawn(async move {
                            gloo_timers::future::TimeoutFuture::new(2000).await;
                            copied_code.set(false);
                        });
                    },
                    span { if copied_code() { "Copied ✓" } else { "Copy code" } }
                }
            }
            pre { class: "text-xs font-mono text-slate-300 overflow-x-auto p-4 bg-[#090d16] rounded-lg border border-[#1e293b]/60 leading-relaxed",
                code { "{code_content}" }
            }
        }
    }
}

#[component]
fn lang_button(selected_lang: Signal<CodeLanguage>, lang: CodeLanguage, label: String) -> Element {
    let is_active = selected_lang() == lang;
    let cls = if is_active {
        "text-xs font-semibold text-sky-400 border-b-2 border-sky-400 pb-1"
    } else {
        "text-xs text-slate-500 hover:text-slate-300 pb-1 transition-colors"
    };

    rsx! {
        button {
            class: "{cls}",
            onclick: move |_| selected_lang.set(lang),
            "{label}"
        }
    }
}

fn code_for_tab(tab_idx: i32, lang: CodeLanguage) -> String {
    match (tab_idx, lang) {
        (0, CodeLanguage::Rust) => r#"use cade_sdk::EmbeddedSession;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Direct zero-daemon in-process execution with cade-sdk
    let mut session = EmbeddedSession::builder()
        .model("anthropic/claude-sonnet-4-5")
        .build()
        .await?;

    let answer = session.prompt("Inspect workspace and describe structure.").await?;
    println!("{answer}");
    Ok(())
}"#
        .to_string(),
        (0, CodeLanguage::Javascript) => r#"import { AgentSession } from "@ezektec/cade";

const session = new AgentSession({ serverUrl: "http://localhost:8284" });
const answer = await session.prompt("Inspect workspace and describe structure.");
console.log(answer);"#
            .to_string(),
        (0, CodeLanguage::Python) => r#"from cade_sdk import EmbeddedSession

with EmbeddedSession(model="anthropic/claude-sonnet-4-5") as session:
    answer = session.prompt("Inspect workspace and describe structure.")
    print(answer)"#
            .to_string(),
        (0, CodeLanguage::Curl) => r#"curl -X POST http://localhost:8284/v1/agents/default/run \
  -H "Content-Type: application/json" \
  -d '{"input": "Inspect workspace and describe structure."}'"#
            .to_string(),
        (1, CodeLanguage::Rust) => r#"use cade_sdk::{AgentOptions, EmbeddedSession};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let session = EmbeddedSession::builder()
        .agent_options(AgentOptions {
            name: "Security Reviewer".into(),
            model: Some("anthropic/claude-sonnet-4-5".into()),
            system_prompt: Some("You are a specialized security reviewer.".into()),
            ..Default::default()
        })
        .build()
        .await?;

    println!("Agent deployed: {}", session.agent_id());
    Ok(())
}"#
        .to_string(),
        (1, CodeLanguage::Javascript) => r#"import { AgentSession } from "@ezektec/cade";

const session = new AgentSession({
  serverUrl: "http://localhost:8284",
  model: "anthropic/claude-sonnet-4-5",
  systemPrompt: "You are a specialized security reviewer."
});"#
            .to_string(),
        (1, CodeLanguage::Python) => r#"from cade_sdk import EmbeddedSession

session = EmbeddedSession(
    model="anthropic/claude-sonnet-4-5",
    system_prompt="You are a specialized security reviewer."
)"#
        .to_string(),
        (1, CodeLanguage::Curl) => r#"curl -X POST http://localhost:8284/v1/agents \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Security Reviewer",
    "model": "anthropic/claude-sonnet-4-5",
    "system_prompt": "You are a specialized security reviewer."
  }'"#
        .to_string(),
        (_, CodeLanguage::Rust) => r#"use cade_sdk::EmbeddedSession;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let session = EmbeddedSession::builder().build().await?;
    let rules = session.get_memory_block("project").await?;
    println!("Project Rules:\n{rules:?}");
    Ok(())
}"#
        .to_string(),
        (_, CodeLanguage::Javascript) => r#"import { AgentSession } from "@ezektec/cade";

const session = new AgentSession({ serverUrl: "http://localhost:8284" });
const projectRules = await session.getMemory("project");
console.log(projectRules);"#
            .to_string(),
        (_, CodeLanguage::Python) => r#"from cade_sdk import EmbeddedSession

with EmbeddedSession() as session:
    rules = session.get_memory("project")
    print("Project Rules:", rules)"#
            .to_string(),
        (_, CodeLanguage::Curl) => {
            r#"curl http://localhost:8284/v1/agents/default/memory"#.to_string()
        }
    }
}
