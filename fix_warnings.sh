# 1. hooks/mod.rs
sed -i 's////             UserPromptSubmit////     UserPromptSubmit/g' crates/cade-core/src/hooks/mod.rs

# 2. skills/mod.rs manual strip
sed -i 's/let (fm_str, body) = if content.starts_with("---") {/let (fm_str, body) = if let Some(stripped) = content.strip_prefix("---") {/' crates/cade-core/src/skills/mod.rs
sed -i 's/match content\[3..\]\.find("---") {/match stripped.find("---") {/' crates/cade-core/src/skills/mod.rs

# 3. toolsets/mod.rs from_str
sed -i 's/pub fn from_str(s: \&str)/pub fn from_name(s: \&str)/g' crates/cade-core/src/toolsets/mod.rs
sed -i 's/Toolset::from_str/Toolset::from_name/g' crates/cade-core/src/toolsets/mod.rs
sed -i 's/Toolset::from_str/Toolset::from_name/g' crates/cade-cli/src/cli/repl.rs
sed -i 's/Toolset::from_str/Toolset::from_name/g' crates/cade-cli/src/cli/args.rs

# 4. app.rs empty lines
sed -i '/^\/\/\/$/d' crates/cade-tui/src/app.rs
sed -i '/^\/\/ \/\/\/ Try to complete/d' crates/cade-tui/src/app.rs
sed -i '/^\/\/ \/\/\/ Returns/d' crates/cade-tui/src/app.rs
sed -i '/^\/\/ \/\/\/ Only triggers/d' crates/cade-tui/src/app.rs
sed -i '/^\/\/ \/\/\/ contains/d' crates/cade-tui/src/app.rs

# 5. app.rs &mut Vec -> &mut [_]
sed -i 's/history: \&mut Vec<String>,/history: \&mut \[String\],/g' crates/cade-tui/src/app.rs
sed -i 's/agents: \&mut Vec<AgentState>,/agents: \&mut \[AgentState\],/g' crates/cade-cli/src/cli/repl.rs

# 6. app.rs clamp
sed -i 's/\(plan.steps.len() as u16 + 2\).min(10).max(4)/\1.clamp(4, 10)/g' crates/cade-tui/src/app.rs
sed -i 's/total.max(1).min(MAX_INPUT_ROWS)/total.clamp(1, MAX_INPUT_ROWS)/g' crates/cade-tui/src/app.rs

# 7. autocomplete.rs manual strip
sed -i 's/let expanded: PathBuf = if partial.starts_with("\~\/") {/let expanded: PathBuf = if let Some(stripped) = partial.strip_prefix("\~\/") {/' crates/cade-tui/src/autocomplete.rs
sed -i 's/h.join(&partial\[2..\])/h.join(stripped)/' crates/cade-tui/src/autocomplete.rs

# 8. component.rs doc list item
sed -i 's/^\/\/!    if the event/\/\/!   if the event/' crates/cade-tui/src/component.rs

# 9. client.rs empty line doc comment
sed -i '/\/\/ -- Provider management/d' crates/cade-agent/src/agent/client.rs

# 10. subagents/mod.rs manual strip
sed -i 's/let (fm_str, body) = if content.starts_with("---") {/let (fm_str, body) = if let Some(stripped) = content.strip_prefix("---") {/' crates/cade-agent/src/subagents/mod.rs
sed -i 's/match content\[3..\]\.find("---") {/match stripped.find("---") {/' crates/cade-agent/src/subagents/mod.rs

# 11. desktop.rs manual strip
sed -i 's/if p.starts_with("\~\/") {/if let Some(stripped) = p.strip_prefix("\~\/") {/' crates/cade-agent/src/tools/desktop.rs
sed -i 's/home.join(&p\[2..\])/home.join(stripped)/' crates/cade-agent/src/tools/desktop.rs

# 12. search.rs manual strip
sed -i "s/if pat.starts_with('\*') {/if let Some(stripped) = pat.strip_prefix('\*') {/" crates/cade-agent/src/tools/search.rs
sed -i 's/fname.ends_with(&pat\[1..\])/fname.ends_with(stripped)/' crates/cade-agent/src/tools/search.rs

# 13. headless.rs redundant locals
sed -i '/let mcp = mcp;/d' crates/cade-cli/src/cli/headless.rs
sed -i '/let hooks = hooks;/d' crates/cade-cli/src/cli/headless.rs

# 14. repl.rs redundant locals
sed -i '/let seed_blocks = seed_blocks;/d' crates/cade-cli/src/cli/repl.rs
sed -i '/let hooks = hooks;/d' crates/cade-cli/src/cli/repl.rs

# 15. repl.rs doc lazy continuation
sed -i 's/^\s*\/\/\/ Generate a diff/\/\/\/      Generate a diff/g' crates/cade-cli/src/cli/repl.rs

