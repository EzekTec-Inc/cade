/// Code intelligence tools — agent-side REST wrappers.
///
/// These tools call the cade-server `/v1/symbols`, `/v1/repo-map`, and
/// `/v1/agents/:id/index` endpoints which in turn use `cade-codeintel`.
use serde_json::{Value, json};

use crate::agent::client::CadeClient;
use crate::Result;

// region:    --- Tool implementations

pub struct SymbolSearchTool;
impl SymbolSearchTool {
    pub async fn run(client: &CadeClient, cwd: &std::path::Path, args: &Value) -> Result<String> {
        let query = args["query"].as_str().unwrap_or("").trim();
        let limit = args["limit"].as_u64().unwrap_or(20) as usize;
        if query.is_empty() {
            return Ok("Error: 'query' is required".to_string());
        }
        let repo_root = args["repo_root"].as_str()
            .map(String::from)
            .unwrap_or_else(|| cwd.to_string_lossy().to_string());

        let resp = client.raw_get(
            &format!("/symbols?q={}&limit={}&repo_root={}", 
                     urlencoding::encode(query), limit, urlencoding::encode(&repo_root))
        ).await?;
        format_symbol_list(&resp)
    }

    pub fn schema() -> Value {
        json!({
            "name": "symbol_search",
            "description": "Search the codebase for symbols (functions, structs, classes, methods) by name or documentation. Returns file path, line number, and signature.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Symbol name or partial name to search for" },
                    "limit": { "type": "integer", "description": "Max results (default 20)" },
                    "repo_root": { "type": "string", "description": "Repository root path (default: current directory)" }
                },
                "required": ["query"]
            }
        })
    }
}

pub struct FindReferencesTool;
impl FindReferencesTool {
    pub async fn run(client: &CadeClient, args: &Value) -> Result<String> {
        let name      = args["name"].as_str().unwrap_or("").trim();
        let repo_root = args["repo_root"].as_str().unwrap_or("");
        if name.is_empty() {
            return Ok("Error: 'name' is required".to_string());
        }
        let resp = client.raw_get(
            &format!("/symbols/{}/refs?repo_root={}", 
                     urlencoding::encode(name), urlencoding::encode(repo_root))
        ).await?;
        format_ref_list(&resp)
    }

    pub fn schema() -> Value {
        json!({
            "name": "find_references",
            "description": "Find all references to a symbol name in the codebase. Returns file paths and line numbers where the symbol is used.",
            "parameters": {
                "type": "object",
                "properties": {
                    "name":      { "type": "string", "description": "Exact symbol name to find references for" },
                    "repo_root": { "type": "string", "description": "Repository root path (default: current directory)" }
                },
                "required": ["name"]
            }
        })
    }
}

pub struct GotoDefinitionTool;
impl GotoDefinitionTool {
    pub async fn run(client: &CadeClient, args: &Value) -> Result<String> {
        let name      = args["name"].as_str().unwrap_or("").trim();
        let from_file = args["from_file"].as_str().unwrap_or("");
        if name.is_empty() {
            return Ok("Error: 'name' is required".to_string());
        }
        let query = if from_file.is_empty() {
            format!("/symbols/{}/definition", urlencoding::encode(name))
        } else {
            format!("/symbols/{}/definition?from_file={}", 
                    urlencoding::encode(name), urlencoding::encode(from_file))
        };
        let resp = client.raw_get(&query).await?;
        format_symbol_detail(&resp)
    }

    pub fn schema() -> Value {
        json!({
            "name": "goto_definition",
            "description": "Find the definition of a symbol — returns the file path, line number, and signature.",
            "parameters": {
                "type": "object",
                "properties": {
                    "name":      { "type": "string", "description": "Exact symbol name" },
                    "from_file": { "type": "string", "description": "Optional: caller's file path for same-file preference" }
                },
                "required": ["name"]
            }
        })
    }
}

pub struct GetRepoMapTool;
impl GetRepoMapTool {
    pub async fn run(client: &CadeClient, cwd: &std::path::Path, args: &Value) -> Result<String> {
        let max_symbols = args["max_symbols_per_file"].as_u64().unwrap_or(8) as usize;
        let repo_root = args["repo_root"].as_str()
            .map(String::from)
            .unwrap_or_else(|| cwd.to_string_lossy().to_string());
        let resp = client.raw_get(
            &format!("/repo-map?max_symbols={}&repo_root={}", 
                     max_symbols, urlencoding::encode(&repo_root))
        ).await?;
        // Server returns a text field
        Ok(resp["map"].as_str().unwrap_or("(no repo map available — run index_repository first)").to_string())
    }

    pub fn schema() -> Value {
        json!({
            "name": "get_repo_map",
            "description": "Get a compact map of the codebase: files with their top symbols. Use to understand project structure before diving in.",
            "parameters": {
                "type": "object",
                "properties": {
                    "max_symbols_per_file": { "type": "integer", "description": "Max symbols per file to show (default 8)" },
                    "repo_root": { "type": "string", "description": "Repository root path (default: current directory)" }
                },
                "required": []
            }
        })
    }
}

pub struct IndexRepositoryTool;
impl IndexRepositoryTool {
    pub async fn run(client: &CadeClient, agent_id: &str, args: &Value) -> Result<String> {
        let repo_root = args["repo_root"].as_str().unwrap_or(".");
        let resp = client.raw_post(
            &format!("/agents/{agent_id}/index"),
            &json!({ "repo_root": repo_root }),
        ).await?;
        let files   = resp["files_indexed"].as_u64().unwrap_or(0);
        let symbols = resp["symbols_added"].as_u64().unwrap_or(0);
        let ms      = resp["duration_ms"].as_u64().unwrap_or(0);
        Ok(format!("Indexed {files} files, {symbols} symbols in {ms}ms"))
    }

    pub fn schema() -> Value {
        json!({
            "name": "index_repository",
            "description": "Index the repository to enable symbol_search, find_references, goto_definition, and get_repo_map. Run once before using other code intelligence tools.",
            "parameters": {
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string", "description": "Repository root to index (default '.')" }
                },
                "required": []
            }
        })
    }
}

// endregion: --- Tool implementations

// region:    --- Formatting helpers

fn format_symbol_list(v: &Value) -> Result<String> {
    let symbols = match v.as_array() {
        Some(a) => a,
        None    => return Ok("No symbols found.".to_string()),
    };
    if symbols.is_empty() { return Ok("No matching symbols found.".to_string()); }
    let mut out = format!("{} symbol(s):\n", symbols.len());
    for sym in symbols.iter().take(20) {
        let name  = sym["name"].as_str().unwrap_or("?");
        let kind  = sym["kind"].as_str().unwrap_or("?");
        let file  = sym["file_path"].as_str().unwrap_or("?");
        let line  = sym["line_start"].as_u64().unwrap_or(0);
        let sig   = sym["signature"].as_str().unwrap_or(name);
        out.push_str(&format!("  [{kind}] {name}  @ {file}:{line}\n    {sig}\n"));
    }
    Ok(out.trim_end().to_string())
}

fn format_symbol_detail(v: &Value) -> Result<String> {
    if v.is_null() || v.get("name").is_none() {
        return Ok("Symbol not found in index. Try index_repository first.".to_string());
    }
    let name   = v["name"].as_str().unwrap_or("?");
    let kind   = v["kind"].as_str().unwrap_or("?");
    let file   = v["file_path"].as_str().unwrap_or("?");
    let line   = v["line_start"].as_u64().unwrap_or(0);
    let sig    = v["signature"].as_str().unwrap_or(name);
    let doc    = v["doc_comment"].as_str().filter(|s| !s.is_empty());
    let mut out = format!("[{kind}] {name}\nFile: {file}:{line}\nSignature: {sig}");
    if let Some(d) = doc { out.push_str(&format!("\nDoc: {d}")); }
    Ok(out)
}

fn format_ref_list(v: &Value) -> Result<String> {
    let refs = match v.as_array() {
        Some(a) => a,
        None    => return Ok("No references found.".to_string()),
    };
    if refs.is_empty() { return Ok("No references found in index.".to_string()); }
    let mut out = format!("{} reference(s):\n", refs.len());
    for r in refs.iter().take(30) {
        let file    = r["file_path"].as_str().unwrap_or("?");
        let line    = r["line"].as_u64().unwrap_or(0);
        let context = r["context"].as_str().unwrap_or("");
        out.push_str(&format!("  {file}:{line}  {context}\n"));
    }
    Ok(out.trim_end().to_string())
}

// endregion: --- Formatting helpers
