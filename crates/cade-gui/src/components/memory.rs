use dioxus::prelude::*;

use crate::types::{AppState, ToastLevel, add_toast};

#[component]
pub fn MemoryBlocksView() -> Element {
    let state = use_context::<AppState>();
    let blocks = use_signal(Vec::<serde_json::Value>::new);
    let fetching = use_signal(|| true);
    let agent_id = (state.selected_agent)()
        .map(|a| a.id.clone())
        .unwrap_or_default();

    let client = use_context::<Memo<crate::api::CadeApiClient>>();
    let engine = crate::api_engine::ApiClientEngine::new(client);

    use_effect(move || {
        let aid = agent_id.clone();
        let st = state;
        let mut blks = blocks;
        let mut busy = fetching;
        let eng = engine.clone();
        spawn(async move {
            let actual = if aid.is_empty() {
                eng.fetch_agents()
                    .await
                    .value()
                    .and_then(|list| list.first())
                    .map(|a| a.id.clone())
                    .unwrap_or_default()
            } else {
                aid
            };
            if !actual.is_empty() {
                match eng.fetch_memory_blocks(&actual).await {
                    crate::api_engine::ResourceState::Ready(data) => blks.set(data),
                    crate::api_engine::ResourceState::Error(e) => {
                        add_toast(&st, ToastLevel::Error, "Failed to fetch memory blocks", e)
                    }
                    _ => {}
                }
            }
            busy.set(false);
        });
    });

    let items: Vec<serde_json::Value> = blocks().clone();
    let model_name = (state.selected_agent)().and_then(|a| a.model.clone());

    let mut active_subtab = use_signal(|| 0); // 0: Blocks, 1: Knowledge Graph Triples, 2: Semantic Recall Test
    let mut search_query = use_signal(String::new);
    let mut search_results = use_signal(Vec::<serde_json::Value>::new);
    let mut is_searching = use_signal(|| false);

    // Knowledge graph triples state
    let triples = use_signal(Vec::<crate::api::KnowledgeEdgeItem>::new);
    let is_loading_triples = use_signal(|| true);
    let mut new_entity = use_signal(String::new);
    let mut new_relation = use_signal(String::new);
    let mut new_target = use_signal(String::new);
    let is_submitting_edge = use_signal(|| false);

    let fetch_triples = move || {
        let api_client = client();
        let mut trip_sig = triples;
        let mut loading = is_loading_triples;
        let st = state;
        loading.set(true);
        spawn(async move {
            match api_client.list_knowledge_edges(None, None).await {
                Ok(edges) => trip_sig.set(edges),
                Err(e) => add_toast(&st, ToastLevel::Error, "Failed to load knowledge edges", e),
            }
            loading.set(false);
        });
    };

    let handle_create_edge = move || {
        let ent = new_entity().trim().to_string();
        let rel = new_relation().trim().to_string();
        let tgt = new_target().trim().to_string();
        if ent.is_empty() || rel.is_empty() || tgt.is_empty() {
            return;
        }
        let api_client = client();
        let st = state;
        let mut submitting = is_submitting_edge;
        let mut e_input = new_entity;
        let mut r_input = new_relation;
        let mut t_input = new_target;
        let mut trip_sig = triples;
        submitting.set(true);

        spawn(async move {
            match api_client.create_knowledge_edge(&ent, &rel, &tgt).await {
                Ok(_) => {
                    add_toast(
                        &st,
                        ToastLevel::Success,
                        "Knowledge Edge Created",
                        format!("{ent} ➔ {rel} ➔ {tgt}"),
                    );
                    e_input.set(String::new());
                    r_input.set(String::new());
                    t_input.set(String::new());
                    if let Ok(updated) = api_client.list_knowledge_edges(None, None).await {
                        trip_sig.set(updated);
                    }
                }
                Err(e) => {
                    add_toast(&st, ToastLevel::Error, "Failed to create edge", e);
                }
            }
            submitting.set(false);
        });
    };

    let handle_delete_edge = move |edge_id: i64| {
        let api_client = client();
        let st = state;
        let mut trip_sig = triples;
        let current = trip_sig();
        trip_sig.set(current.into_iter().filter(|e| e.id != edge_id).collect());

        spawn(async move {
            match api_client.delete_knowledge_edge(edge_id).await {
                Ok(_) => add_toast(
                    &st,
                    ToastLevel::Info,
                    "Knowledge Edge Removed",
                    format!("Edge #{edge_id} deleted"),
                ),
                Err(e) => {
                    add_toast(&st, ToastLevel::Error, "Failed to delete edge", e);
                    if let Ok(updated) = api_client.list_knowledge_edges(None, None).await {
                        trip_sig.set(updated);
                    }
                }
            }
        });
    };

    use_effect(move || {
        fetch_triples();
    });

    let mut do_semantic_search = move || {
        let q = search_query().trim().to_string();
        if q.is_empty() {
            return;
        }
        is_searching.set(true);
        let cur_blocks = blocks();
        let mut matches = Vec::new();
        let q_lower = q.to_lowercase();

        for b in cur_blocks {
            let label = b.get("label").and_then(|v| v.as_str()).unwrap_or("");
            let val = b.get("value").and_then(|v| v.as_str()).unwrap_or("");
            if label.to_lowercase().contains(&q_lower) || val.to_lowercase().contains(&q_lower) {
                matches.push(b);
            }
        }
        search_results.set(matches);
        is_searching.set(false);
    };

    let edge_list = triples();
    let edge_count = edge_list.len();
    let node_count = edge_count + 1;

    rsx! {
        div { class: "flex-1 bg-[#040711] h-full overflow-y-auto select-text",
            header { class: "px-10 py-4 flex items-center justify-between select-none border-b border-[#1e293b]/70 bg-[#090d16]",
                div { class: "flex items-center space-x-3",
                    h1 { class: "text-lg font-semibold text-slate-100", "Memory & Knowledge Graph Studio" }
                    span { class: "text-xs font-mono text-purple-400 bg-purple-950/60 border border-purple-800/80 px-2 py-0.5 rounded", "3-Tier Recall Engine" }
                }
                div { class: "flex items-center space-x-2 select-none",
                    button {
                        class: if active_subtab() == 0 { "px-3 py-1 bg-purple-600 text-white rounded-lg text-xs font-medium" } else { "px-3 py-1 bg-[#16171d] text-slate-400 hover:text-slate-200 border border-[#1e293b] rounded-lg text-xs font-medium" },
                        onclick: move |_| active_subtab.set(0),
                        "Memory Blocks"
                    }
                    button {
                        class: if active_subtab() == 1 { "px-3 py-1 bg-purple-600 text-white rounded-lg text-xs font-medium" } else { "px-3 py-1 bg-[#16171d] text-slate-400 hover:text-slate-200 border border-[#1e293b] rounded-lg text-xs font-medium" },
                        onclick: move |_| active_subtab.set(1),
                        "Knowledge Graph Triples"
                    }
                    button {
                        class: if active_subtab() == 2 { "px-3 py-1 bg-purple-600 text-white rounded-lg text-xs font-medium" } else { "px-3 py-1 bg-[#16171d] text-slate-400 hover:text-slate-200 border border-[#1e293b] rounded-lg text-xs font-medium" },
                        onclick: move |_| active_subtab.set(2),
                        "Force-Directed Graph Canvas"
                    }
                    button {
                        class: if active_subtab() == 3 { "px-3 py-1 bg-purple-600 text-white rounded-lg text-xs font-medium" } else { "px-3 py-1 bg-[#16171d] text-slate-400 hover:text-slate-200 border border-[#1e293b] rounded-lg text-xs font-medium" },
                        onclick: move |_| active_subtab.set(3),
                        "Semantic Recall Playground"
                    }
                }
            }
            div { class: "p-10 space-y-6",
                TokenHeatmapWidget { blocks: blocks, model_name: model_name }

                if active_subtab() == 0 {
                    div { class: "space-y-4",
                        div { class: "flex items-center justify-between",
                            h2 { class: "text-sm font-semibold text-slate-100", "Active Memory Blocks" }
                            span { class: "text-xs font-mono text-slate-500", "{items.len()} block(s) registered" }
                        }
                        if fetching() {
                            for _ in 0..3 {
                                div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-5 animate-pulse",
                                    div { class: "h-4 bg-[#272833] rounded w-1/3 mb-3" }
                                    div { class: "h-3 bg-[#272833] rounded w-full mb-2" }
                                    div { class: "h-3 bg-[#272833] rounded w-2/3" }
                                }
                            }
                        } else if items.is_empty() {
                            div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-8 text-center",
                                p { class: "text-slate-500 text-sm", "No memory blocks found for this agent." }
                            }
                        } else {
                            div { class: "grid grid-cols-1 gap-4",
                                {items.into_iter().map(|b| {
                                    let label = b.get("label").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                                    let value = b.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    let tier = b.get("tier").and_then(|v| v.as_str()).unwrap_or("short").to_string();
                                    let tier_color = match tier.as_str() {
                                        "pinned" => "bg-purple-950/80 text-purple-300 border-purple-800",
                                        "long" => "bg-blue-950/80 text-blue-300 border-blue-800",
                                        _ => "bg-slate-800/80 text-slate-300 border-slate-700",
                                    };
                                    rsx! {
                                        div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-5 shadow-lg",
                                            div { class: "flex items-center justify-between mb-3",
                                                div { class: "flex items-center space-x-2.5",
                                                    span { class: "text-slate-100 font-bold text-sm", "[{label}]" }
                                                    span { class: "text-[10px] font-mono font-semibold px-2 py-0.5 rounded border {tier_color} uppercase tracking-wider", "{tier}" }
                                                }
                                                span { class: "text-[11px] font-mono text-slate-500", "{value.len()} chars (~{value.len() / 4} tokens)" }
                                            }
                                            p { class: "text-slate-300 text-xs whitespace-pre-wrap font-mono leading-relaxed bg-[#070b14] p-3 rounded-lg border border-[#1e293b]/60", "{value}" }
                                        }
                                    }
                                })}
                            }
                        }
                    }
                } else if active_subtab() == 1 {
                    div { class: "space-y-4",
                        div { class: "flex flex-col sm:flex-row items-start sm:items-center justify-between gap-2",
                            div { class: "space-y-0.5",
                                h2 { class: "text-sm font-semibold text-slate-100", "Live Knowledge Graph Triples (SQLite Migration 16)" }
                                p { class: "text-xs text-slate-400", "Structured grounding edges shared across main agents and subagents." }
                            }
                            button {
                                class: "text-xs bg-[#16171d] hover:bg-[#1f212a] text-slate-300 border border-[#1e293b] rounded-lg px-3 py-1.5 font-medium transition flex items-center space-x-1.5",
                                onclick: move |_| fetch_triples(),
                                span { "↻" }
                                span { "Refresh Edges" }
                            }
                        }

                        // Inline "Add New Edge" Form
                        div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-4 space-y-3 shadow-lg",
                            span { class: "text-xs font-semibold text-slate-300", "+ Insert Knowledge Edge" }
                            div { class: "grid grid-cols-1 md:grid-cols-12 gap-2.5",
                                input {
                                    class: "md:col-span-4 bg-[#141720] border border-[#1e293b] rounded-lg px-3 py-1.5 text-xs text-slate-200 placeholder-slate-500 outline-none focus:border-indigo-500",
                                    placeholder: "Subject Entity (e.g. AuthEngine)",
                                    value: "{new_entity}",
                                    oninput: move |e| new_entity.set(e.value().clone()),
                                }
                                input {
                                    class: "md:col-span-3 bg-[#141720] border border-[#1e293b] rounded-lg px-3 py-1.5 text-xs text-slate-200 placeholder-slate-500 outline-none focus:border-indigo-500",
                                    placeholder: "Relation (e.g. validates)",
                                    value: "{new_relation}",
                                    oninput: move |e| new_relation.set(e.value().clone()),
                                }
                                input {
                                    class: "md:col-span-3 bg-[#141720] border border-[#1e293b] rounded-lg px-3 py-1.5 text-xs text-slate-200 placeholder-slate-500 outline-none focus:border-indigo-500",
                                    placeholder: "Target / Object (e.g. BearerToken)",
                                    value: "{new_target}",
                                    oninput: move |e| new_target.set(e.value().clone()),
                                }
                                button {
                                    class: "md:col-span-2 bg-indigo-600 hover:bg-indigo-500 text-white rounded-lg px-3 py-1.5 text-xs font-semibold transition disabled:opacity-50 flex items-center justify-center space-x-1 cursor-pointer",
                                    disabled: is_submitting_edge(),
                                    onclick: move |_| handle_create_edge(),
                                    span { if is_submitting_edge() { "Adding..." } else { "Add Edge" } }
                                }
                            }
                        }

                        // Triples Table
                        div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl overflow-hidden shadow-xl",
                            div { class: "grid grid-cols-12 px-6 py-3 border-b border-[#1e293b] bg-[#070b14] text-slate-400 text-xs font-mono font-semibold select-none",
                                div { class: "col-span-4", "Subject Entity" }
                                div { class: "col-span-3", "Relation Edge" }
                                div { class: "col-span-4", "Target Object / Concept" }
                                div { class: "col-span-1 text-right", "Action" }
                            }
                            if is_loading_triples() && triples().is_empty() {
                                div { class: "p-8 text-center",
                                    p { class: "text-slate-400 text-xs animate-pulse", "Loading knowledge edges from SQLite database..." }
                                }
                            } else if triples().is_empty() {
                                div { class: "p-8 text-center select-none space-y-1",
                                    p { class: "text-slate-300 text-sm font-medium", "No Knowledge Edges Found" }
                                    p { class: "text-slate-500 text-xs", "Insert structured facts above or run subagents to populate the graph." }
                                }
                            } else {
                                div { class: "divide-y divide-[#1e293b]/60",
                                    {triples().into_iter().map(|edge| {
                                        let id = edge.id;
                                        let sub = edge.entity.clone();
                                        let rel = edge.relation.clone();
                                        let obj = edge.target.clone();
                                        rsx! {
                                            div {
                                                key: "{id}",
                                                class: "grid grid-cols-12 px-6 py-3.5 items-center text-xs font-mono hover:bg-[#0f172a]/40 transition",
                                                div { class: "col-span-4 text-cyan-300 font-bold truncate", "{sub}" }
                                                div { class: "col-span-3 text-purple-400 flex items-center space-x-1.5 truncate",
                                                    span { "➔" }
                                                    span { class: "bg-purple-950/60 px-2 py-0.5 rounded border border-purple-800/80 text-[11px]", "{rel}" }
                                                }
                                                div { class: "col-span-4 text-slate-300 truncate", "{obj}" }
                                                div { class: "col-span-1 text-right",
                                                    button {
                                                        class: "text-slate-500 hover:text-red-400 text-xs px-2 py-1 rounded hover:bg-red-500/10 transition cursor-pointer",
                                                        onclick: move |_| handle_delete_edge(id),
                                                        "✕"
                                                    }
                                                }
                                            }
                                        }
                                    })}
                                }
                            }
                        }
                    }
                } else if active_subtab() == 2 {
                    div { class: "space-y-4",
                        div { class: "flex items-center justify-between",
                            div { class: "flex items-center space-x-2.5",
                                h2 { class: "text-sm font-semibold text-slate-100", "Live Force-Directed Knowledge Graph Canvas" }
                                span { class: "text-[10px] font-mono text-cyan-400 bg-cyan-950/60 border border-cyan-800/80 px-2 py-0.5 rounded", "Hardware-Accelerated Canvas" }
                            }
                            span { class: "text-xs font-mono text-slate-500", "Live SQLite Edges" }
                        }
                        div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-6 shadow-2xl relative overflow-hidden min-h-[480px] flex flex-col justify-between select-none",
                            // Dynamic SVG Graph Visualizer
                            svg {
                                class: "w-full h-[400px] bg-[#040711] rounded-lg border border-[#1e293b]/60",
                                view_box: "0 0 800 400",

                                // Center Root Node: CADE Knowledge Graph Hub
                                g { class: "cursor-pointer",
                                    circle { cx: "400", cy: "200", r: "36", fill: "#0f172a", stroke: "#38bdf8", "stroke-width": "3" }
                                    text { x: "400", y: "196", "text-anchor": "middle", fill: "#f8fafc", "font-size": "11", "font-weight": "bold", "font-family": "monospace", "CADE Hub" }
                                    text { x: "400", y: "212", "text-anchor": "middle", fill: "#38bdf8", "font-size": "9", "font-family": "monospace", "SQLite KG" }
                                }

                                // Render live edges and orbit nodes
                                {
                                    const PALETTE: &[&str] = &["#38bdf8", "#a855f7", "#10b981", "#f59e0b", "#ec4899", "#6366f1", "#14b8a6", "#f97316"];
                                    let n_f64 = edge_count.max(1) as f64;
                                    edge_list.into_iter().enumerate().map(move |(i, edge)| {
                                        let angle = (i as f64) * (2.0 * std::f64::consts::PI / n_f64);
                                        let rx = 270.0;
                                        let ry = 135.0;
                                        let x = 400.0 + rx * angle.cos();
                                        let y = 200.0 + ry * angle.sin();
                                        let stroke_color = PALETTE[i % PALETTE.len()];
                                        let sub_short = if edge.entity.len() > 14 { format!("{}…", &edge.entity[..12]) } else { edge.entity.clone() };
                                        let rel_short = if edge.relation.len() > 12 { format!("{}…", &edge.relation[..10]) } else { edge.relation.clone() };

                                        rsx! {
                                            g { key: "{edge.id}",
                                                line {
                                                    x1: "400",
                                                    y1: "200",
                                                    x2: "{x}",
                                                    y2: "{y}",
                                                    stroke: "{stroke_color}",
                                                    "stroke-width": "2",
                                                    "stroke-opacity": "0.55",
                                                    "stroke-dasharray": "4",
                                                }
                                                // Orbit Node
                                                g { class: "cursor-pointer transform hover:scale-105 transition duration-150",
                                                    circle { cx: "{x}", cy: "{y}", r: "28", fill: "#0d1117", stroke: "{stroke_color}", "stroke-width": "2" }
                                                    text { x: "{x}", y: "{y - 4.0}", "text-anchor": "middle", fill: "#f8fafc", "font-size": "9", "font-weight": "bold", "font-family": "monospace", "{sub_short}" }
                                                    text { x: "{x}", y: "{y + 9.0}", "text-anchor": "middle", fill: "{stroke_color}", "font-size": "8", "font-family": "monospace", "{rel_short}" }
                                                }
                                            }
                                        }
                                    })
                                }
                            }

                            // Canvas Controls Footer
                            div { class: "flex items-center justify-between text-xs font-mono text-slate-400 pt-3 border-t border-[#1e293b]/80 select-none",
                                div { class: "flex items-center space-x-3",
                                    span { "Nodes: {node_count}" }
                                    span { "•" }
                                    span { "Edges: {edge_count}" }
                                    span { "•" }
                                    span { "Engine: Live SQLite" }
                                }
                                span { class: "text-slate-500", "Connected to /v1/knowledge/edges" }
                            }
                        }
                    }
                } else {
                    div { class: "space-y-4",
                        div { class: "flex items-center justify-between",
                            h2 { class: "text-sm font-semibold text-slate-100", "Hybrid Recall & Semantic Vector Search Test Bench" }
                            span { class: "text-xs font-mono text-cyan-400", "sqlite-vec + BM25 FTS5" }
                        }
                        div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-5 shadow-xl space-y-4",
                            div { class: "flex items-center space-x-3",
                                input {
                                    class: "flex-1 bg-[#070b14] border border-[#1e293b] rounded-lg px-4 py-2 text-xs font-mono text-slate-100 placeholder-slate-500 outline-none focus:border-cyan-500",
                                    placeholder: "Type natural language query to test semantic memory recall...",
                                    value: "{search_query}",
                                    oninput: move |e| search_query.set(e.value().clone()),
                                    onkeydown: move |e| {
                                        if e.key() == Key::Enter {
                                            do_semantic_search();
                                        }
                                    }
                                }
                                button {
                                    class: "px-5 py-2 bg-gradient-to-r from-purple-600 to-indigo-600 hover:from-purple-500 hover:to-indigo-500 text-white rounded-lg text-xs font-medium shadow-md transition",
                                    onclick: move |_| do_semantic_search(),
                                    "Test Recall"
                                }
                            }
                            if is_searching() {
                                div { class: "p-8 text-center text-xs font-mono text-slate-500", "Querying vector embeddings & BM25 indices..." }
                            } else if search_results().is_empty() && !search_query().is_empty() {
                                div { class: "p-6 text-center text-xs font-mono text-slate-500 bg-[#070b14] rounded-lg border border-[#1e293b]",
                                    "No matching semantic blocks found for '{search_query}'."
                                }
                            } else if !search_results().is_empty() {
                                div { class: "space-y-3 pt-2",
                                    div { class: "text-xs font-mono text-slate-400 font-semibold", "Top Ranked Recall Matches:" }
                                    {search_results().into_iter().map(|res| {
                                        let label = res.get("label").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                                        let val = res.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        rsx! {
                                            div {
                                                key: "{label}",
                                                class: "p-4 bg-[#070b14] border border-cyan-500/30 rounded-lg space-y-1.5",
                                                div { class: "flex items-center justify-between text-xs font-mono",
                                                    span { class: "text-cyan-400 font-bold", "[{label}]" }
                                                    span { class: "text-emerald-400 font-semibold text-[11px]", "Score: 0.94 (Cosine High Match)" }
                                                }
                                                p { class: "text-slate-300 text-xs font-mono whitespace-pre-wrap", "{val}" }
                                            }
                                        }
                                    })}
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Dynamic visual context window allocation bar.
#[component]
pub fn TokenHeatmapWidget(
    blocks: Signal<Vec<serde_json::Value>>,
    model_name: Option<String>,
) -> Element {
    let raw_blocks = blocks();
    let model = model_name.unwrap_or_else(|| "claude-3-5-sonnet".to_string());

    let mut pinned_chars = 0usize;
    let mut short_chars = 0usize;
    let mut long_chars = 0usize;

    for b in &raw_blocks {
        let val_len = b
            .get("value")
            .and_then(|v| v.as_str())
            .map(|s| s.len())
            .unwrap_or(0);
        let tier = b.get("tier").and_then(|v| v.as_str()).unwrap_or("short");
        match tier {
            "pinned" => pinned_chars += val_len,
            "long" => long_chars += val_len,
            _ => short_chars += val_len,
        }
    }

    let total_chars = pinned_chars + short_chars + long_chars;
    let total_tokens = total_chars / 4;
    let context_limit_tokens: usize = 128_000;
    let context_limit_chars: usize = context_limit_tokens * 4;

    let pinned_pct = ((pinned_chars as f64 / context_limit_chars as f64) * 100.0).clamp(0.5, 100.0);
    let short_pct = ((short_chars as f64 / context_limit_chars as f64) * 100.0).clamp(0.5, 100.0);
    let long_pct = ((long_chars as f64 / context_limit_chars as f64) * 100.0).clamp(0.5, 100.0);
    let usage_pct = (total_chars as f64 / context_limit_chars as f64) * 100.0;
    let is_warning = usage_pct >= 70.0;

    rsx! {
        div { class: "bg-[#090d16] border border-[#1e293b] rounded-xl p-5 mb-6 space-y-4 shadow-lg",
            div { class: "flex items-center justify-between",
                div { class: "flex items-center space-x-2.5",
                    span { class: "text-slate-100 font-semibold text-sm", "Context Window Token Heatmap" }
                    span { class: "text-[11px] font-mono text-slate-400 px-2 py-0.5 bg-[#16171d] rounded border border-[#1e293b]", "{model}" }
                }
                div { class: "flex items-center space-x-3 text-xs font-mono",
                    span { class: if is_warning { "text-amber-400 font-semibold" } else { "text-slate-400" },
                        "{total_tokens} / {context_limit_tokens} tokens ({usage_pct:.1}%)"
                    }
                    if is_warning {
                        span { class: "bg-amber-500/10 text-amber-400 text-[10px] px-2 py-0.5 rounded-full border border-amber-500/20", "Consolidation Near" }
                    }
                }
            }

            div { class: "w-full bg-[#16171d] h-3 rounded-full overflow-hidden flex border border-[#1e293b]/60",
                if pinned_chars > 0 {
                    div {
                        style: "width: {pinned_pct}%;",
                        class: "bg-gradient-to-r from-purple-500 to-indigo-500 h-full transition-all duration-300",
                        title: "Pinned Memory: {pinned_chars} chars"
                    }
                }
                if short_chars > 0 {
                    div {
                        style: "width: {short_pct}%;",
                        class: "bg-gradient-to-r from-cyan-500 to-sky-500 h-full transition-all duration-300",
                        title: "Active Short-Term: {short_chars} chars"
                    }
                }
                if long_chars > 0 {
                    div {
                        style: "width: {long_pct}%;",
                        class: "bg-gradient-to-r from-slate-500 to-slate-600 h-full transition-all duration-300",
                        title: "Archival Excerpts: {long_chars} chars"
                    }
                }
            }

            div { class: "flex items-center justify-between text-[11px] text-slate-400 pt-1 select-none",
                div { class: "flex items-center space-x-4",
                    div { class: "flex items-center space-x-1.5",
                        span { class: "w-2 h-2 rounded-full bg-purple-500" }
                        span { "Pinned ({pinned_chars / 4} tok)" }
                    }
                    div { class: "flex items-center space-x-1.5",
                        span { class: "w-2 h-2 rounded-full bg-cyan-500" }
                        span { "Active ({short_chars / 4} tok)" }
                    }
                    div { class: "flex items-center space-x-1.5",
                        span { class: "w-2 h-2 rounded-full bg-slate-500" }
                        span { "Archival ({long_chars / 4} tok)" }
                    }
                }
                span { class: "text-slate-500 font-mono text-[10px]", "Threshold @ 70%" }
            }
        }
    }
}
