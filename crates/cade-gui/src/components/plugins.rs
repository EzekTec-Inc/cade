use dioxus::prelude::*;
use serde_json::Value;

use crate::api_engine::{ApiClientEngine, ResourceMutation, ResourceState};
use crate::types::{AppState, ToastLevel, add_toast};

/// Detailed metadata model representing an inspected plugin (from inventory or marketplace).
#[derive(Clone, PartialEq, Debug)]
pub struct SelectedPluginDetail {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub scope: Option<String>,
    pub status: Option<String>,
    pub diagnostic: Option<String>,
    pub tags: Vec<String>,
    pub tools_count: usize,
    pub skills_count: usize,
    pub mcp_servers_count: usize,
    pub exported_tools: Vec<String>,
    pub url: Option<String>,
    pub sha256: Option<String>,
    pub is_installed: bool,
}

#[component]
pub fn PluginSettings() -> Element {
    let state = use_context::<AppState>();
    let engine = use_context::<ApiClientEngine>();

    let mut active_tab = use_signal(|| "installed".to_string());
    let installed_plugins = use_signal(Vec::<Value>::new);
    let marketplace_plugins = use_signal(Vec::<Value>::new);
    let is_loading = use_signal(|| false);

    // Filter and search signals
    let mut installed_search = use_signal(String::new);
    let mut market_search_query = use_signal(String::new);
    let mut category_filter = use_signal(|| "All".to_string());
    let mut selected_tag = use_signal(|| Option::<String>::None);

    // Detail inspection modal / drawer
    let mut inspected_plugin = use_signal(|| Option::<SelectedPluginDetail>::None);

    // Custom package direct install form state
    let mut custom_url = use_signal(String::new);
    let mut custom_id = use_signal(String::new);
    let is_installing_custom = use_signal(|| false);

    // Refresh installed plugins from backend
    let refresh_installed = {
        let eng = engine.clone();
        move || {
            let eng = eng.clone();
            let mut installed = installed_plugins;
            let mut loading = is_loading;
            spawn(async move {
                loading.set(true);
                match eng.fetch_plugins().await {
                    ResourceState::Ready(items) => installed.set(items),
                    ResourceState::Error(_err) => {}
                    _ => {}
                }
                loading.set(false);
            });
        }
    };

    // Search marketplace catalog
    let search_catalog = {
        let eng = engine.clone();
        move |query: String| {
            let eng = eng.clone();
            let mut market = marketplace_plugins;
            let mut loading = is_loading;
            spawn(async move {
                loading.set(true);
                match eng.search_marketplace(&query).await {
                    ResourceState::Ready(items) => market.set(items),
                    ResourceState::Error(_err) => {}
                    _ => {}
                }
                loading.set(false);
            });
        }
    };

    // Initial load
    use_effect({
        let refresh = refresh_installed.clone();
        let search = search_catalog.clone();
        move || {
            refresh();
            search(String::new());
        }
    });

    // Action: Install from catalog
    let on_market_install = {
        let eng = engine.clone();
        let refresh = refresh_installed.clone();
        move |id: String, download_url: Option<String>| {
            let eng = eng.clone();
            let refresh = refresh.clone();
            let st = state;
            let mut inspected = inspected_plugin;
            let url = download_url.unwrap_or_else(|| id.clone());
            spawn(async move {
                add_toast(
                    &st,
                    ToastLevel::Info,
                    "Installing Plugin",
                    &format!("Installing plugin package '{id}'..."),
                );
                match eng
                    .mutate(ResourceMutation::InstallPlugin {
                        url,
                        plugin_id: id.clone(),
                    })
                    .await
                {
                    Ok(installed_id) => {
                        add_toast(
                            &st,
                            ToastLevel::Success,
                            "Plugin Installed",
                            &format!("Plugin '{installed_id}' installed successfully."),
                        );
                        refresh();
                        // Update inspection modal if currently open
                        if let Some(mut current) = inspected() {
                            if current.id == id {
                                current.is_installed = true;
                                inspected.set(Some(current));
                            }
                        }
                    }
                    Err(error) => {
                        add_toast(
                            &st,
                            ToastLevel::Error,
                            "Installation Failed",
                            &format!("Failed to install '{id}': {error}"),
                        );
                    }
                }
            });
        }
    };

    // Action: Uninstall plugin
    let on_uninstall = {
        let eng = engine.clone();
        let refresh = refresh_installed.clone();
        move |id: String| {
            let eng = eng.clone();
            let refresh = refresh.clone();
            let st = state;
            let mut inspected = inspected_plugin;
            spawn(async move {
                add_toast(
                    &st,
                    ToastLevel::Info,
                    "Uninstalling Plugin",
                    &format!("Removing plugin '{id}'..."),
                );
                match eng
                    .mutate(ResourceMutation::UninstallPlugin {
                        plugin_id: id.clone(),
                    })
                    .await
                {
                    Ok(_) => {
                        add_toast(
                            &st,
                            ToastLevel::Success,
                            "Plugin Uninstalled",
                            &format!("Plugin '{id}' removed successfully."),
                        );
                        refresh();
                        // Close or update inspection drawer
                        if let Some(mut current) = inspected() {
                            if current.id == id {
                                current.is_installed = false;
                                inspected.set(Some(current));
                            }
                        }
                    }
                    Err(error) => {
                        add_toast(
                            &st,
                            ToastLevel::Error,
                            "Uninstall Failed",
                            &format!("Failed to remove '{id}': {error}"),
                        );
                    }
                }
            });
        }
    };

    // Action: Install custom package from URL
    let on_install_custom = {
        let eng = engine.clone();
        let refresh = refresh_installed.clone();
        move || {
            let url_val = custom_url().trim().to_string();
            let id_val = custom_id().trim().to_string();
            if url_val.is_empty() || id_val.is_empty() {
                add_toast(
                    &state,
                    ToastLevel::Warning,
                    "Validation Error",
                    "Both Package URL and Plugin ID are required.",
                );
                return;
            }

            let eng = eng.clone();
            let refresh = refresh.clone();
            let st = state;
            let mut installing = is_installing_custom;
            let mut tab = active_tab;
            let mut c_url = custom_url;
            let mut c_id = custom_id;

            installing.set(true);
            spawn(async move {
                add_toast(
                    &st,
                    ToastLevel::Info,
                    "Installing Package",
                    &format!("Downloading and activating '{id_val}' from '{url_val}'..."),
                );
                match eng
                    .mutate(ResourceMutation::InstallPlugin {
                        url: url_val.clone(),
                        plugin_id: id_val.clone(),
                    })
                    .await
                {
                    Ok(installed_id) => {
                        add_toast(
                            &st,
                            ToastLevel::Success,
                            "Package Installed",
                            &format!("Custom plugin '{installed_id}' is now active."),
                        );
                        c_url.set(String::new());
                        c_id.set(String::new());
                        refresh();
                        tab.set("installed".to_string());
                    }
                    Err(error) => {
                        add_toast(
                            &st,
                            ToastLevel::Error,
                            "Installation Failed",
                            &format!("Failed to install custom plugin '{id_val}': {error}"),
                        );
                    }
                }
                installing.set(false);
            });
        }
    };

    // Filter installed plugins
    let search_inst = installed_search().to_lowercase();
    let filtered_installed: Vec<Value> = installed_plugins()
        .into_iter()
        .filter(|p| {
            if search_inst.is_empty() {
                return true;
            }
            let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
            let scope = p.get("scope").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
            let status = p.get("status").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
            id.contains(&search_inst)
                || name.contains(&search_inst)
                || scope.contains(&search_inst)
                || status.contains(&search_inst)
        })
        .collect();

    // Filter marketplace plugins
    let current_cat = category_filter();
    let active_tag_opt = selected_tag();
    let filtered_market: Vec<Value> = marketplace_plugins()
        .into_iter()
        .filter(|p| {
            let tags: Vec<String> = p
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|t| t.as_str().map(|s| s.to_lowercase()))
                        .collect()
                })
                .unwrap_or_default();
            let desc = p
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();
            let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();

            // Tag filter
            if let Some(ref tag) = active_tag_opt {
                let tag_lower = tag.to_lowercase();
                if !tags.iter().any(|t| t.contains(&tag_lower)) {
                    return false;
                }
            }

            // Category filter
            match current_cat.as_str() {
                "MCP Servers" => {
                    tags.iter().any(|t| t.contains("mcp")) || desc.contains("mcp") || id.contains("mcp")
                }
                "Skills" => {
                    tags.iter().any(|t| t.contains("skill")) || desc.contains("skill") || id.contains("skill")
                }
                "Developer Tools" => {
                    tags.iter().any(|t| {
                        t.contains("tool") || t.contains("lsp") || t.contains("dev") || t.contains("cli")
                    }) || desc.contains("tool") || desc.contains("developer")
                }
                "AI & Agents" => {
                    tags.iter().any(|t| {
                        t.contains("ai") || t.contains("agent") || t.contains("llm") || t.contains("model")
                    }) || desc.contains("agent") || desc.contains("ai")
                }
                _ => true,
            }
        })
        .collect();

    let installed_ids: Vec<String> = installed_plugins()
        .iter()
        .filter_map(|p| p.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();

    rsx! {
        div {
            class: "flex flex-col h-full overflow-hidden bg-[#11111b] text-[#cdd6f4]",

            // Header banner
            div {
                class: "flex items-center justify-between px-6 py-4 border-b border-[#313244] bg-[#181825]",
                div {
                    class: "flex items-center gap-3",
                    div {
                        class: "w-9 h-9 rounded-lg bg-[#89b4fa]/15 flex items-center justify-center text-[#89b4fa] font-bold text-lg",
                        "⚡"
                    }
                    div {
                        h1 { class: "text-lg font-bold text-[#cdd6f4]", "Plugin & Extensions Hub" }
                        p { class: "text-xs text-[#a6adc8]", "Discover, install, and manage extensible MCP servers, tool bundles, and agent skills" }
                    }
                }

                div {
                    class: "flex items-center gap-2",
                    button {
                        class: "px-3 py-1.5 rounded bg-[#313244] hover:bg-[#45475a] text-xs text-[#cdd6f4] transition flex items-center gap-1.5",
                        onclick: {
                            let refresh = refresh_installed.clone();
                            let search = search_catalog.clone();
                            let q = market_search_query();
                            move |_| {
                                refresh();
                                search(q.clone());
                            }
                        },
                        "↻ Refresh"
                    }
                }
            }

            // Tab Navigation Bar
            div {
                class: "flex items-center justify-between px-6 border-b border-[#313244] bg-[#181825]/60",
                div {
                    class: "flex items-center gap-4",
                    button {
                        class: if active_tab() == "installed" {
                            "py-3 px-1 border-b-2 border-[#89b4fa] text-[#89b4fa] font-medium text-sm flex items-center gap-2"
                        } else {
                            "py-3 px-1 border-b-2 border-transparent text-[#a6adc8] hover:text-[#cdd6f4] text-sm transition flex items-center gap-2"
                        },
                        onclick: move |_| active_tab.set("installed".to_string()),
                        span { "Installed Plugins" }
                        span {
                            class: "px-1.5 py-0.5 text-xs rounded-full bg-[#313244] text-[#cdd6f4]",
                            "{installed_plugins().len()}"
                        }
                    }
                    button {
                        class: if active_tab() == "marketplace" {
                            "py-3 px-1 border-b-2 border-[#89b4fa] text-[#89b4fa] font-medium text-sm flex items-center gap-2"
                        } else {
                            "py-3 px-1 border-b-2 border-transparent text-[#a6adc8] hover:text-[#cdd6f4] text-sm transition flex items-center gap-2"
                        },
                        onclick: move |_| active_tab.set("marketplace".to_string()),
                        span { "Marketplace Catalog" }
                        span {
                            class: "px-1.5 py-0.5 text-xs rounded-full bg-[#89b4fa]/20 text-[#89b4fa]",
                            "{marketplace_plugins().len()}"
                        }
                    }
                    button {
                        class: if active_tab() == "custom" {
                            "py-3 px-1 border-b-2 border-[#89b4fa] text-[#89b4fa] font-medium text-sm flex items-center gap-2"
                        } else {
                            "py-3 px-1 border-b-2 border-transparent text-[#a6adc8] hover:text-[#cdd6f4] text-sm transition flex items-center gap-2"
                        },
                        onclick: move |_| active_tab.set("custom".to_string()),
                        span { "＋ Install Custom Package" }
                    }
                }

                if is_loading() {
                    div {
                        class: "text-xs text-[#89b4fa] animate-pulse flex items-center gap-1.5",
                        span { "●" }
                        "Syncing..."
                    }
                }
            }

            // Main Content Area
            div {
                class: "flex-1 overflow-y-auto p-6 space-y-6",

                // TAB 1: INSTALLED PLUGINS
                if active_tab() == "installed" {
                    div {
                        class: "space-y-4",

                        // Filter bar
                        div {
                            class: "flex items-center justify-between gap-4",
                            div {
                                class: "relative flex-1 max-w-md",
                                input {
                                    class: "w-full bg-[#181825] border border-[#313244] rounded-lg px-3 py-2 text-sm text-[#cdd6f4] placeholder-[#6c7086] focus:outline-none focus:border-[#89b4fa] transition",
                                    placeholder: "Filter installed plugins by name, ID, or status...",
                                    value: "{installed_search()}",
                                    oninput: move |e| installed_search.set(e.value()),
                                }
                            }
                            div {
                                class: "text-xs text-[#a6adc8]",
                                "Showing {filtered_installed.len()} of {installed_plugins().len()} active extensions"
                            }
                        }

                        // Grid of installed plugins
                        if filtered_installed.is_empty() {
                            div {
                                class: "p-8 text-center border border-dashed border-[#313244] rounded-xl bg-[#181825]/40",
                                p { class: "text-sm text-[#a6adc8]", "No installed plugins match your filter criteria." }
                                button {
                                    class: "mt-3 px-4 py-1.5 text-xs rounded bg-[#89b4fa] text-[#11111b] font-medium hover:bg-[#b4befe] transition",
                                    onclick: move |_| active_tab.set("marketplace".to_string()),
                                    "Browse Marketplace Catalog"
                                }
                            }
                        } else {
                            div {
                                class: "grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4",
                                for p in filtered_installed {
                                    {
                                        let p_id = p.get("id").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                                        let p_name = p.get("name").and_then(|v| v.as_str()).unwrap_or(&p_id).to_string();
                                        let p_ver = p.get("version").and_then(|v| v.as_str()).unwrap_or("0.1.0").to_string();
                                        let p_scope = p.get("scope").and_then(|v| v.as_str()).unwrap_or("project").to_string();
                                        let p_status = p.get("status").and_then(|v| v.as_str()).unwrap_or("active").to_string();
                                        let p_diag = p.get("diagnostic").and_then(|v| v.as_str()).map(|s| s.to_string());
                                        let tools_count = p.get("tools_count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                                        let skills_count = p.get("skills_count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                                        let mcp_count = p.get("mcp_servers_count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                                        let exported_tools: Vec<String> = p.get("exported_tools")
                                            .and_then(|v| v.as_array())
                                            .map(|arr| arr.iter().filter_map(|t| t.as_str().map(|s| s.to_string())).collect())
                                            .unwrap_or_default();

                                        let p_id_detail = p_id.clone();
                                        let p_name_detail = p_name.clone();
                                        let p_ver_detail = p_ver.clone();
                                        let p_scope_detail = p_scope.clone();
                                        let p_status_detail = p_status.clone();
                                        let p_diag_detail = p_diag.clone();
                                        let exported_detail = exported_tools.clone();

                                        rsx! {
                                            div {
                                                key: "{p_id}",
                                                class: "p-4 rounded-xl border border-[#313244] bg-[#181825] hover:border-[#45475a] transition flex flex-col justify-between space-y-4",

                                                div {
                                                    class: "space-y-2",
                                                    div {
                                                        class: "flex items-start justify-between gap-2",
                                                        div {
                                                            class: "flex items-center gap-2",
                                                            h3 { class: "font-semibold text-sm text-[#cdd6f4] truncate", "{p_name}" }
                                                            span { class: "text-xs px-1.5 py-0.5 rounded bg-[#313244] text-[#a6adc8]", "v{p_ver}" }
                                                        }
                                                        span {
                                                            class: if p_status == "active" {
                                                                "text-[10px] px-2 py-0.5 rounded-full bg-[#a6e3a1]/20 text-[#a6e3a1] font-medium"
                                                            } else {
                                                                "text-[10px] px-2 py-0.5 rounded-full bg-[#f9e2af]/20 text-[#f9e2af] font-medium"
                                                            },
                                                            "{p_status}"
                                                        }
                                                    }

                                                    p { class: "text-xs text-[#a6adc8] font-mono truncate", "{p_id}" }

                                                    if let Some(ref diag) = p_diag {
                                                        div {
                                                            class: "text-[11px] text-[#f38ba8] bg-[#f38ba8]/10 p-2 rounded border border-[#f38ba8]/20",
                                                            "{diag}"
                                                        }
                                                    }

                                                    // Metrics counters
                                                    div {
                                                        class: "flex items-center gap-3 pt-2 text-xs text-[#6c7086]",
                                                        span { class: "flex items-center gap-1", span { class: "text-[#89b4fa]", "🛠" }, "{tools_count} tools" }
                                                        span { class: "flex items-center gap-1", span { class: "text-[#a6e3a1]", "🧠" }, "{skills_count} skills" }
                                                        span { class: "flex items-center gap-1", span { class: "text-[#fab387]", "🔌" }, "{mcp_count} mcp" }
                                                    }
                                                }

                                                // Actions
                                                div {
                                                    class: "flex items-center justify-between pt-3 border-t border-[#313244]/60",
                                                    span {
                                                        class: "text-[11px] text-[#6c7086] uppercase tracking-wide",
                                                        "Scope: {p_scope}"
                                                    }
                                                    div {
                                                        class: "flex items-center gap-2",
                                                        button {
                                                            class: "px-2.5 py-1 text-xs rounded bg-[#313244] hover:bg-[#45475a] text-[#cdd6f4] transition",
                                                            onclick: move |_| {
                                                                inspected_plugin.set(Some(SelectedPluginDetail {
                                                                    id: p_id_detail.clone(),
                                                                    name: p_name_detail.clone(),
                                                                    version: p_ver_detail.clone(),
                                                                    author: "Local / Installed".to_string(),
                                                                    description: "Installed extension bundle in active PluginEngine scope.".to_string(),
                                                                    scope: Some(p_scope_detail.clone()),
                                                                    status: Some(p_status_detail.clone()),
                                                                    diagnostic: p_diag_detail.clone(),
                                                                    tags: vec!["installed".to_string(), p_scope_detail.clone()],
                                                                    tools_count,
                                                                    skills_count,
                                                                    mcp_servers_count: mcp_count,
                                                                    exported_tools: exported_detail.clone(),
                                                                    url: None,
                                                                    sha256: None,
                                                                    is_installed: true,
                                                                }));
                                                            },
                                                            "Inspect"
                                                        }
                                                        button {
                                                            class: "px-2.5 py-1 text-xs rounded bg-[#f38ba8]/20 hover:bg-[#f38ba8]/30 text-[#f38ba8] transition",
                                                            onclick: {
                                                                let on_u = on_uninstall.clone();
                                                                let id_to_remove = p_id.clone();
                                                                move |_| on_u(id_to_remove.clone())
                                                            },
                                                            "Uninstall"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // TAB 2: MARKETPLACE CATALOG
                if active_tab() == "marketplace" {
                    div {
                        class: "space-y-4",

                        // Search and Category Pills Bar
                        div {
                            class: "flex flex-col md:flex-row items-stretch md:items-center justify-between gap-4 bg-[#181825] p-4 rounded-xl border border-[#313244]",

                            // Search input
                            div {
                                class: "flex items-center gap-2 flex-1 max-w-lg",
                                input {
                                    class: "flex-1 bg-[#11111b] border border-[#313244] rounded-lg px-3 py-2 text-sm text-[#cdd6f4] placeholder-[#6c7086] focus:outline-none focus:border-[#89b4fa] transition",
                                    placeholder: "Search catalog by keyword, author, or tags...",
                                    value: "{market_search_query()}",
                                    oninput: move |e| market_search_query.set(e.value()),
                                }
                                button {
                                    class: "px-4 py-2 rounded-lg bg-[#89b4fa] text-[#11111b] font-medium text-xs hover:bg-[#b4befe] transition whitespace-nowrap",
                                    onclick: {
                                        let search = search_catalog.clone();
                                        let q = market_search_query();
                                        move |_| search(q.clone())
                                    },
                                    "Search Catalog"
                                }
                            }

                            // Category filter pills
                            div {
                                class: "flex items-center gap-1.5 flex-wrap",
                                for cat in &["All", "MCP Servers", "Skills", "Developer Tools", "AI & Agents"] {
                                    {
                                        let cat_str = cat.to_string();
                                        let is_active = category_filter() == cat_str;
                                        rsx! {
                                            button {
                                                key: "{cat}",
                                                class: if is_active {
                                                    "px-3 py-1 rounded-full text-xs font-semibold bg-[#89b4fa] text-[#11111b] transition shadow-sm"
                                                } else {
                                                    "px-3 py-1 rounded-full text-xs bg-[#313244] hover:bg-[#45475a] text-[#cdd6f4] transition"
                                                },
                                                onclick: move |_| category_filter.set(cat_str.clone()),
                                                "{cat}"
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Active Tag Banner (if user filtered by clicking a tag chip)
                        if let Some(ref tag) = active_tag_opt {
                            div {
                                class: "flex items-center justify-between px-4 py-2 bg-[#89b4fa]/10 border border-[#89b4fa]/30 rounded-lg text-xs text-[#89b4fa]",
                                span {
                                    "Filtering by tag: "
                                    strong { "#{tag}" }
                                }
                                button {
                                    class: "hover:underline text-[#f38ba8] text-xs font-medium",
                                    onclick: move |_| selected_tag.set(None),
                                    "✕ Clear Tag Filter"
                                }
                            }
                        }

                        // Marketplace Results Grid
                        if filtered_market.is_empty() {
                            div {
                                class: "p-12 text-center border border-dashed border-[#313244] rounded-xl bg-[#181825]/40 space-y-2",
                                p { class: "text-sm text-[#a6adc8]", "No marketplace plugins found matching the criteria." }
                                p { class: "text-xs text-[#6c7086]", "Try resetting your search query, selecting 'All' category, or clearing active tag filters." }
                            }
                        } else {
                            div {
                                class: "grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4",
                                for p in filtered_market {
                                    {
                                        let p_id = p.get("id").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                                        let p_ver = p.get("version").and_then(|v| v.as_str()).unwrap_or("0.1.0").to_string();
                                        let p_desc = p.get("description").and_then(|v| v.as_str()).unwrap_or("No description provided.").to_string();
                                        let p_author = p.get("author").and_then(|v| v.as_str()).unwrap_or("community").to_string();
                                        let p_url = p.get("url").and_then(|v| v.as_str()).map(|s| s.to_string());
                                        let p_sha256 = p.get("sha256").and_then(|v| v.as_str()).map(|s| s.to_string());
                                        let tags: Vec<String> = p.get("tags")
                                            .and_then(|v| v.as_array())
                                            .map(|arr| arr.iter().filter_map(|t| t.as_str().map(|s| s.to_string())).collect())
                                            .unwrap_or_default();

                                        let is_installed = installed_ids.contains(&p_id);

                                        let p_id_detail = p_id.clone();
                                        let p_ver_detail = p_ver.clone();
                                        let p_desc_detail = p_desc.clone();
                                        let p_author_detail = p_author.clone();
                                        let p_url_detail = p_url.clone();
                                        let p_sha_detail = p_sha256.clone();
                                        let tags_detail = tags.clone();

                                        rsx! {
                                            div {
                                                key: "{p_id}",
                                                class: "p-4 rounded-xl border border-[#313244] bg-[#181825] hover:border-[#45475a] transition flex flex-col justify-between space-y-4",

                                                div {
                                                    class: "space-y-2",
                                                    div {
                                                        class: "flex items-start justify-between gap-2",
                                                        div {
                                                            h3 { class: "font-semibold text-sm text-[#cdd6f4] truncate", "{p_id}" }
                                                            span { class: "text-xs text-[#6c7086]", "by {p_author} • v{p_ver}" }
                                                        }
                                                        if is_installed {
                                                            span {
                                                                class: "text-[10px] px-2 py-0.5 rounded-full bg-[#a6e3a1]/20 text-[#a6e3a1] font-medium whitespace-nowrap",
                                                                "✓ Installed"
                                                            }
                                                        }
                                                    }

                                                    p { class: "text-xs text-[#a6adc8] line-clamp-2", "{p_desc}" }

                                                    // Tags
                                                    if !tags.is_empty() {
                                                        div {
                                                            class: "flex items-center gap-1.5 flex-wrap pt-1",
                                                            for tag in tags.iter().take(4) {
                                                                {
                                                                    let tag_str = tag.clone();
                                                                    rsx! {
                                                                        button {
                                                                            key: "{tag}",
                                                                            class: "text-[10px] px-2 py-0.5 rounded bg-[#313244] hover:bg-[#45475a] text-[#89b4fa] transition",
                                                                            onclick: move |_| selected_tag.set(Some(tag_str.clone())),
                                                                            "#{tag}"
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }

                                                // Card Footer Actions
                                                div {
                                                    class: "flex items-center justify-between pt-3 border-t border-[#313244]/60",
                                                    button {
                                                        class: "text-xs text-[#89b4fa] hover:underline font-medium",
                                                        onclick: move |_| {
                                                            inspected_plugin.set(Some(SelectedPluginDetail {
                                                                id: p_id_detail.clone(),
                                                                name: p_id_detail.clone(),
                                                                version: p_ver_detail.clone(),
                                                                author: p_author_detail.clone(),
                                                                description: p_desc_detail.clone(),
                                                                scope: None,
                                                                status: None,
                                                                diagnostic: None,
                                                                tags: tags_detail.clone(),
                                                                tools_count: 0,
                                                                skills_count: 0,
                                                                mcp_servers_count: 0,
                                                                exported_tools: vec![],
                                                                url: p_url_detail.clone(),
                                                                sha256: p_sha_detail.clone(),
                                                                is_installed,
                                                            }));
                                                        },
                                                        "View Details"
                                                    }

                                                    if is_installed {
                                                        button {
                                                            class: "px-3 py-1.5 text-xs rounded bg-[#f38ba8]/20 hover:bg-[#f38ba8]/30 text-[#f38ba8] transition",
                                                            onclick: {
                                                                let on_u = on_uninstall.clone();
                                                                let id_to_remove = p_id.clone();
                                                                move |_| on_u(id_to_remove.clone())
                                                            },
                                                            "Uninstall"
                                                        }
                                                    } else {
                                                        button {
                                                            class: "px-3 py-1.5 text-xs rounded bg-[#89b4fa] hover:bg-[#b4befe] text-[#11111b] font-medium transition",
                                                            onclick: {
                                                                let on_i = on_market_install.clone();
                                                                let id_to_install = p_id.clone();
                                                                let url_to_install = p_url.clone();
                                                                move |_| on_i(id_to_install.clone(), url_to_install.clone())
                                                            },
                                                            "Install"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // TAB 3: INSTALL CUSTOM PACKAGE FORM
                if active_tab() == "custom" {
                    div {
                        class: "max-w-2xl mx-auto p-6 rounded-xl border border-[#313244] bg-[#181825] space-y-6 shadow-md",

                        div {
                            class: "space-y-1 border-b border-[#313244] pb-4",
                            h2 { class: "text-base font-bold text-[#cdd6f4]", "Install Custom Plugin Package" }
                            p { class: "text-xs text-[#a6adc8]", "Install external plugins from a direct tarball archive, git repository, or remote custom package endpoint." }
                        }

                        div {
                            class: "space-y-4",

                            div {
                                class: "space-y-1.5",
                                label { class: "text-xs font-semibold text-[#cdd6f4]", "Plugin Package URL or Archive Path" }
                                input {
                                    class: "w-full bg-[#11111b] border border-[#313244] rounded-lg px-3 py-2 text-sm text-[#cdd6f4] placeholder-[#6c7086] focus:outline-none focus:border-[#89b4fa] transition font-mono",
                                    placeholder: "https://example.com/plugins/my-plugin.tar.gz",
                                    value: "{custom_url()}",
                                    oninput: move |e| custom_url.set(e.value()),
                                }
                                p { class: "text-[11px] text-[#6c7086]", "Direct HTTPS URL to a validated .tar.gz bundle or git repository endpoint." }
                            }

                            div {
                                class: "space-y-1.5",
                                label { class: "text-xs font-semibold text-[#cdd6f4]", "Plugin Identifier (Package ID)" }
                                input {
                                    class: "w-full bg-[#11111b] border border-[#313244] rounded-lg px-3 py-2 text-sm text-[#cdd6f4] placeholder-[#6c7086] focus:outline-none focus:border-[#89b4fa] transition font-mono",
                                    placeholder: "@organization/my-plugin",
                                    value: "{custom_id()}",
                                    oninput: move |e| custom_id.set(e.value()),
                                }
                                p { class: "text-[11px] text-[#6c7086]", "Canonical identifier matching the manifest ID declared inside the package." }
                            }

                            div {
                                class: "p-4 rounded-lg bg-[#11111b] border border-[#313244] space-y-2",
                                h4 { class: "text-xs font-semibold text-[#89b4fa]", "Verification & Security Note" }
                                p { class: "text-xs text-[#a6adc8]", "Installed plugins run in isolated sandboxes or native subprocesses. The package manifest will be checked for security compliance, exported tool signatures, and required capabilities before activation." }
                            }

                            div {
                                class: "flex items-center justify-end gap-3 pt-4 border-t border-[#313244]",
                                button {
                                    class: "px-4 py-2 text-xs rounded bg-[#313244] hover:bg-[#45475a] text-[#cdd6f4] transition",
                                    onclick: move |_| {
                                        custom_url.set(String::new());
                                        custom_id.set(String::new());
                                        active_tab.set("marketplace".to_string());
                                    },
                                    "Cancel"
                                }
                                button {
                                    class: if is_installing_custom() || custom_url().trim().is_empty() || custom_id().trim().is_empty() {
                                        "px-5 py-2 text-xs rounded bg-[#313244] text-[#6c7086] cursor-not-allowed font-medium"
                                    } else {
                                        "px-5 py-2 text-xs rounded bg-[#89b4fa] hover:bg-[#b4befe] text-[#11111b] font-semibold transition shadow-md"
                                    },
                                    disabled: is_installing_custom() || custom_url().trim().is_empty() || custom_id().trim().is_empty(),
                                    onclick: {
                                        let on_c = on_install_custom.clone();
                                        move |_| on_c()
                                    },
                                    if is_installing_custom() {
                                        "Installing Package..."
                                    } else {
                                        "Download & Activate Plugin"
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // MODAL / INSPECTION DRAWER
            if let Some(detail) = inspected_plugin() {
                div {
                    class: "fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4",
                    div {
                        class: "w-full max-w-xl bg-[#181825] border border-[#313244] rounded-2xl shadow-2xl overflow-hidden flex flex-col max-h-[85vh]",

                        // Modal Header
                        div {
                            class: "px-6 py-4 border-b border-[#313244] flex items-center justify-between bg-[#11111b]/50",
                            div {
                                class: "flex items-center gap-2",
                                h2 { class: "text-base font-bold text-[#cdd6f4]", "{detail.name}" }
                                span { class: "text-xs px-2 py-0.5 rounded bg-[#313244] text-[#89b4fa]", "v{detail.version}" }
                            }
                            button {
                                class: "text-[#a6adc8] hover:text-[#cdd6f4] text-lg font-bold px-2 py-0.5 rounded hover:bg-[#313244] transition",
                                onclick: move |_| inspected_plugin.set(None),
                                "✕"
                            }
                        }

                        // Modal Body
                        div {
                            class: "p-6 overflow-y-auto space-y-5 flex-1",

                            div {
                                class: "space-y-1",
                                span { class: "text-[11px] uppercase tracking-wider text-[#6c7086] font-semibold", "Description" }
                                p { class: "text-sm text-[#cdd6f4]", "{detail.description}" }
                            }

                            div {
                                class: "grid grid-cols-2 gap-3 p-3 rounded-xl bg-[#11111b] border border-[#313244] text-xs",
                                div {
                                    span { class: "text-[#6c7086]", "Package ID: " }
                                    span { class: "font-mono text-[#cdd6f4] break-all", "{detail.id}" }
                                }
                                div {
                                    span { class: "text-[#6c7086]", "Author: " }
                                    span { class: "text-[#cdd6f4]", "{detail.author}" }
                                }
                                if let Some(ref scope) = detail.scope {
                                    div {
                                        span { class: "text-[#6c7086]", "Scope: " }
                                        span { class: "text-[#a6e3a1]", "{scope}" }
                                    }
                                }
                                if let Some(ref status) = detail.status {
                                    div {
                                        span { class: "text-[#6c7086]", "Status: " }
                                        span { class: "text-[#89b4fa]", "{status}" }
                                    }
                                }
                            }

                            // Diagnostics if any
                            if let Some(ref diag) = detail.diagnostic {
                                div {
                                    class: "space-y-1.5",
                                    span { class: "text-[11px] uppercase tracking-wider text-[#f38ba8] font-semibold", "Diagnostic Warning" }
                                    div {
                                        class: "p-3 rounded-lg bg-[#f38ba8]/10 border border-[#f38ba8]/20 text-xs text-[#f38ba8]",
                                        "{diag}"
                                    }
                                }
                            }

                            // Tags
                            if !detail.tags.is_empty() {
                                div {
                                    class: "space-y-1.5",
                                    span { class: "text-[11px] uppercase tracking-wider text-[#6c7086] font-semibold", "Tags & Categories" }
                                    div {
                                        class: "flex flex-wrap gap-1.5",
                                        for tag in &detail.tags {
                                            span {
                                                key: "{tag}",
                                                class: "text-xs px-2 py-0.5 rounded bg-[#313244] text-[#89b4fa]",
                                                "#{tag}"
                                            }
                                        }
                                    }
                                }
                            }

                            // Exported Tools list
                            if !detail.exported_tools.is_empty() {
                                div {
                                    class: "space-y-1.5",
                                    span { class: "text-[11px] uppercase tracking-wider text-[#6c7086] font-semibold", "Exported Tools ({detail.exported_tools.len()})" }
                                    div {
                                        class: "flex flex-wrap gap-1.5 max-h-32 overflow-y-auto p-2 rounded-lg bg-[#11111b] border border-[#313244]",
                                        for tool in &detail.exported_tools {
                                            span {
                                                key: "{tool}",
                                                class: "text-xs px-2 py-0.5 rounded bg-[#313244] text-[#a6e3a1] font-mono",
                                                "⚙ {tool}"
                                            }
                                        }
                                    }
                                }
                            }

                            // Checksum & Download URL
                            if detail.sha256.is_some() || detail.url.is_some() {
                                div {
                                    class: "space-y-2 pt-2 border-t border-[#313244] text-xs",
                                    if let Some(ref sha) = detail.sha256 {
                                        div {
                                            span { class: "text-[#6c7086] block text-[10px] uppercase font-semibold", "Integrity Checksum (SHA-256)" }
                                            code { class: "font-mono text-[#f9e2af] text-[11px] break-all", "{sha}" }
                                        }
                                    }
                                    if let Some(ref d_url) = detail.url {
                                        div {
                                            span { class: "text-[#6c7086] block text-[10px] uppercase font-semibold", "Archive URL" }
                                            span { class: "font-mono text-[#a6adc8] text-[11px] break-all", "{d_url}" }
                                        }
                                    }
                                }
                            }
                        }

                        // Modal Footer
                        div {
                            class: "px-6 py-4 border-t border-[#313244] bg-[#11111b]/50 flex items-center justify-between",
                            button {
                                class: "px-4 py-2 text-xs rounded bg-[#313244] hover:bg-[#45475a] text-[#cdd6f4] transition",
                                onclick: move |_| inspected_plugin.set(None),
                                "Close"
                            }

                            if detail.is_installed {
                                button {
                                    class: "px-4 py-2 text-xs rounded bg-[#f38ba8]/20 hover:bg-[#f38ba8]/30 text-[#f38ba8] font-medium transition",
                                    onclick: {
                                        let on_u = on_uninstall.clone();
                                        let p_id = detail.id.clone();
                                        move |_| on_u(p_id.clone())
                                    },
                                    "Uninstall Plugin"
                                }
                            } else {
                                button {
                                    class: "px-4 py-2 text-xs rounded bg-[#89b4fa] hover:bg-[#b4befe] text-[#11111b] font-semibold transition",
                                    onclick: {
                                        let on_i = on_market_install.clone();
                                        let p_id = detail.id.clone();
                                        let d_url = detail.url.clone();
                                        move |_| on_i(p_id.clone(), d_url.clone())
                                    },
                                    "Install Plugin"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
