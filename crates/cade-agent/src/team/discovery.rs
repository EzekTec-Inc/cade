use super::member::{MemberDef, MemberScope, MemberTools};
use super::mode::TeamMode;
use crate::Result;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct TeamDef {
    pub id: String,
    pub name: String,
    pub description: String,
    pub mode: TeamMode,
    pub max_iterations: usize,
    pub leader_model: Option<String>,
    pub members: Vec<MemberDef>,
    pub scope: MemberScope,
    pub path: Option<PathBuf>,
}
impl TeamDef {
    pub fn summary(&self) -> String {
        format!(
            "  [{:<8}] {:<22} — {} (mode: {}, {} members)",
            self.scope,
            self.id,
            self.description,
            self.mode,
            self.members.len()
        )
    }
    pub fn members_xml(&self) -> String {
        let mut o = String::from("<team_members>\n");
        for m in &self.members {
            o.push_str(&m.to_xml_description(2));
        }
        o.push_str("</team_members>");
        o
    }
}
pub fn builtin_default_team() -> TeamDef {
    TeamDef {
        id: "default".into(),
        name: "Default Team".into(),
        description: "General-purpose team".into(),
        mode: TeamMode::Coordinate,
        max_iterations: 10,
        leader_model: None,
        members: builtin_members(),
        scope: MemberScope::Builtin,
        path: None,
    }
}
pub fn builtin_members() -> Vec<MemberDef> {
    vec![
    MemberDef{id:"worker".into(),name:"Worker".into(),role:Some("General-purpose coding worker".into()),description:"Explore, plan, implement, review".into(),model:None,tools:MemberTools::All,system_prompt:"You are a highly capable worker agent. Complete the assigned task autonomously.\n\nCOMPLETION CONTRACT:\n- Use tools to accomplish the task.\n- When done, call `finish(summary, status)` with status='done'.\n- If stuck, call `finish` with status='blocked'.".into(),skills:vec![],scope:MemberScope::Builtin,path:None},
    MemberDef{id:"reflection".into(),name:"Reflection Agent".into(),role:Some("Memory maintenance".into()),description:"Reflects on conversation and updates memory".into(),model:None,tools:MemberTools::List(vec!["update_memory".into(),"read_file".into(),"glob".into()]),system_prompt:"You are a memory-maintenance agent. Update memory blocks with new facts and corrections.".into(),skills:vec![],scope:MemberScope::Builtin,path:None},
    MemberDef{id:"recall".into(),name:"Recall Agent".into(),role:Some("Context retrieval".into()),description:"Search past conversations for context".into(),model:None,tools:MemberTools::Readonly,system_prompt:"You are a search agent. Return precise answers with source references.".into(),skills:vec![],scope:MemberScope::Builtin,path:None},
]
}
fn discover_members_in_dir(dir: &Path, scope: MemberScope) -> Vec<MemberDef> {
    if !dir.exists() {
        return vec![];
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut d = vec![];
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(c) = std::fs::read_to_string(&p) else {
            continue;
        };
        let fid = p
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        match parse_member_md(&fid, &c, scope, p.clone()) {
            Ok(def) => d.push(def),
            Err(e) => tracing::warn!("Bad member at {}: {e}", p.display()),
        }
    }
    d
}
fn parse_member_md(
    fid: &str,
    content: &str,
    scope: MemberScope,
    path: PathBuf,
) -> Result<MemberDef> {
    let c = content.trim();
    let (fm, body) = if let Some(s) = c.strip_prefix("---") {
        match s.find("---") {
            Some(e) => (&c[3..e + 3], &c[e + 6..]),
            None => ("", c),
        }
    } else {
        ("", c)
    };
    let (mut id, mut name, mut role, mut desc, mut model, mut tools, mut skills) = (
        fid.into(),
        fid.to_string(),
        None::<String>,
        String::new(),
        None::<String>,
        MemberTools::All,
        vec![],
    );
    for l in fm.lines() {
        let l = l.trim();
        if let Some((k, v)) = l.split_once(':') {
            let (k, v) = (k.trim(), v.trim().trim_matches('"').trim_matches('\''));
            match k {
                "id" => id = v.into(),
                "name" => name = v.into(),
                "role" => role = Some(v.into()),
                "description" => desc = v.into(),
                "model" => model = Some(v.into()),
                "tools" => tools = MemberTools::from_str(v),
                "skills" => skills = v.split(',').map(|s| s.trim().to_string()).collect(),
                _ => {}
            }
        }
    }
    Ok(MemberDef {
        id,
        name,
        role,
        description: desc,
        model,
        tools,
        skills,
        scope,
        path: Some(path),
        system_prompt: body.trim().to_string(),
    })
}
fn discover_teams_in_dir(dir: &Path, scope: MemberScope) -> Vec<TeamDef> {
    if !dir.exists() {
        return vec![];
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut d = vec![];
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Ok(c) = std::fs::read_to_string(&p) else {
            continue;
        };
        let fid = p
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        match parse_team_toml(&fid, &c, scope, p.clone()) {
            Ok(def) => d.push(def),
            Err(e) => tracing::warn!("Bad team at {}: {e}", p.display()),
        }
    }
    d
}
fn parse_team_toml(fid: &str, content: &str, scope: MemberScope, path: PathBuf) -> Result<TeamDef> {
    let t = toml_to_json(content)?;
    Ok(TeamDef {
        id: t["id"]
            .as_str()
            .or_else(|| t["name"].as_str())
            .unwrap_or(fid)
            .into(),
        name: t["name"].as_str().unwrap_or(fid).into(),
        description: t["description"].as_str().unwrap_or("").into(),
        mode: t["mode"]
            .as_str()
            .and_then(TeamMode::from_str)
            .unwrap_or(TeamMode::Coordinate),
        max_iterations: t["max_iterations"]
            .as_u64()
            .map(|n| n as usize)
            .unwrap_or(10),
        leader_model: t["leader_model"]
            .as_str()
            .or_else(|| t["model"].as_str())
            .map(|s| s.into()),
        members: t["members"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| parse_toml_member(m, scope))
                    .collect()
            })
            .unwrap_or_default(),
        scope,
        path: Some(path),
    })
}
fn parse_toml_member(v: &serde_json::Value, scope: MemberScope) -> Option<MemberDef> {
    let id = v["id"].as_str()?.to_string();
    Some(MemberDef {
        id: id.clone(),
        name: v["name"].as_str().unwrap_or(&id).into(),
        role: v["role"].as_str().map(|s| s.into()),
        description: v["description"].as_str().unwrap_or("").into(),
        model: v["model"].as_str().map(|s| s.into()),
        tools: v["tools"]
            .as_str()
            .map(MemberTools::from_str)
            .or_else(|| {
                v["tools"].as_array().map(|a| {
                    MemberTools::List(
                        a.iter()
                            .filter_map(|t| t.as_str().map(String::from))
                            .collect(),
                    )
                })
            })
            .unwrap_or(MemberTools::All),
        system_prompt: v["system_prompt"].as_str().unwrap_or("").into(),
        skills: v["skills"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        scope,
        path: None,
    })
}
#[allow(clippy::collapsible_if)]
fn toml_to_json(content: &str) -> Result<serde_json::Value> {
    use serde_json::{Map, Value};
    let mut root = Map::new();
    let mut cak: Option<String> = None;
    let mut cm = Map::new();
    for line in content.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if t.starts_with("[[") && t.ends_with("]]") {
            if let Some(k) = &cak {
                if !cm.is_empty() {
                    let a = root.entry(k).or_insert_with(|| Value::Array(vec![]));
                    if let Value::Array(a) = a {
                        a.push(Value::Object(std::mem::take(&mut cm)));
                    }
                }
            }
            cak = Some(
                t.trim_start_matches('[')
                    .trim_end_matches(']')
                    .trim()
                    .into(),
            );
            cm = Map::new();
            continue;
        }
        if let Some((k, v)) = t.split_once('=') {
            let (k, val) = (k.trim().to_string(), ptv(v.trim()));
            if cak.is_some() {
                cm.insert(k, val);
            } else {
                root.insert(k, val);
            }
        }
    }
    if let Some(ref k) = cak {
        if !cm.is_empty() {
            let a = root.entry(k).or_insert_with(|| Value::Array(vec![]));
            if let Value::Array(a) = a {
                a.push(Value::Object(cm));
            }
        }
    }
    Ok(Value::Object(root))
}
fn ptv(v: &str) -> serde_json::Value {
    use serde_json::Value;
    let v = v.trim();
    if (v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\'')) {
        return Value::String(v[1..v.len() - 1].into());
    }
    if v == "true" {
        return Value::Bool(true);
    }
    if v == "false" {
        return Value::Bool(false);
    }
    if let Ok(n) = v.parse::<i64>() {
        return Value::Number(n.into());
    }
    if v.starts_with('[') && v.ends_with(']') {
        return Value::Array(
            v[1..v.len() - 1]
                .split(',')
                .map(|s| ptv(s.trim()))
                .filter(|v| !v.is_null())
                .collect(),
        );
    }
    Value::String(v.into())
}
#[allow(clippy::collapsible_if)]
pub fn discover_all_teams(cwd: &Path) -> Vec<TeamDef> {
    let mut all: Vec<TeamDef> = vec![builtin_default_team()];
    if let Some(h) = dirs::home_dir() {
        all.extend(discover_teams_in_dir(
            &h.join(".cade").join("teams"),
            MemberScope::Global,
        ));
    }
    all.extend(discover_teams_in_dir(
        &cwd.join(".cade").join("teams"),
        MemberScope::Project,
    ));
    let sm = discover_standalone_members(cwd);
    if !sm.is_empty() {
        if let Some(dt) = all.iter_mut().find(|t| t.id == "default") {
            for m in sm {
                if !dt.members.iter().any(|e| e.id == m.id) {
                    dt.members.push(m);
                }
            }
        }
    }
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut merged: Vec<TeamDef> = vec![];
    for d in all {
        if let Some(&i) = seen.get(&d.id) {
            if d.scope > merged[i].scope {
                merged[i] = d;
            }
        } else {
            seen.insert(d.id.clone(), merged.len());
            merged.push(d);
        }
    }
    merged.sort_by(|a, b| (a.scope as u8).cmp(&(b.scope as u8)).then(a.id.cmp(&b.id)));
    merged
}
fn discover_standalone_members(cwd: &Path) -> Vec<MemberDef> {
    let mut all = vec![];
    if let Some(h) = dirs::home_dir() {
        all.extend(discover_members_in_dir(
            &h.join(".cade").join("members"),
            MemberScope::Global,
        ));
        all.extend(discover_members_in_dir(
            &h.join(".cade").join("subagents"),
            MemberScope::Global,
        ));
    }
    all.extend(discover_members_in_dir(
        &cwd.join(".cade").join("members"),
        MemberScope::Project,
    ));
    all.extend(discover_members_in_dir(
        &cwd.join(".cade").join("subagents"),
        MemberScope::Project,
    ));
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut m: Vec<MemberDef> = vec![];
    for d in all {
        if let Some(&i) = seen.get(&d.id) {
            if d.scope > m[i].scope {
                m[i] = d;
            }
        } else {
            seen.insert(d.id.clone(), m.len());
            m.push(d);
        }
    }
    m
}
pub fn find_team<'a>(id: &str, all: &'a [TeamDef]) -> Option<&'a TeamDef> {
    all.iter().find(|t| t.id == id)
}
pub fn resolve_team_def<'a>(id: &str, all: &'a [TeamDef]) -> Option<&'a TeamDef> {
    find_team(id, all).or_else(|| find_team("default", all))
}
