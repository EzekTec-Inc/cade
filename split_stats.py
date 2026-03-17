import re

with open("src/cli/repl.rs", "r") as f:
    content = f.read()

# Extract ModelStats and SessionStats
pattern = re.compile(r"(/// Per-model token breakdown.*?impl SessionStats \{.*?\n\})", re.DOTALL)
match = pattern.search(content)

if match:
    stats_code = match.group(1)
    
    # We need to make structs and fields public
    stats_code = stats_code.replace("struct ModelStats", "pub struct ModelStats")
    stats_code = stats_code.replace("struct SessionStats", "pub struct SessionStats")
    stats_code = stats_code.replace("    reqs:", "    pub reqs:")
    stats_code = stats_code.replace("    input_tokens:", "    pub input_tokens:")
    stats_code = stats_code.replace("    cache_read_tokens:", "    pub cache_read_tokens:")
    stats_code = stats_code.replace("    cache_write_tokens:", "    pub cache_write_tokens:")
    stats_code = stats_code.replace("    output_tokens:", "    pub output_tokens:")
    stats_code = stats_code.replace("    started_at:", "    pub started_at:")
    stats_code = stats_code.replace("    agent_active_ms:", "    pub agent_active_ms:")
    stats_code = stats_code.replace("    api_time_ms:", "    pub api_time_ms:")
    stats_code = stats_code.replace("    tool_time_ms:", "    pub tool_time_ms:")
    stats_code = stats_code.replace("    tool_calls_total:", "    pub tool_calls_total:")
    stats_code = stats_code.replace("    tool_calls_ok:", "    pub tool_calls_ok:")
    stats_code = stats_code.replace("    tool_calls_err:", "    pub tool_calls_err:")
    stats_code = stats_code.replace("    approved:", "    pub approved:")
    stats_code = stats_code.replace("    reviewed:", "    pub reviewed:")
    stats_code = stats_code.replace("    lines_added:", "    pub lines_added:")
    stats_code = stats_code.replace("    lines_removed:", "    pub lines_removed:")
    stats_code = stats_code.replace("    per_model:", "    pub per_model:")
    
    stats_code = stats_code.replace("fn new", "pub fn new")
    stats_code = stats_code.replace("fn record_usage", "pub fn record_usage")
    stats_code = stats_code.replace("fn compute_cost", "pub fn compute_cost")
    stats_code = stats_code.replace("fn render_card", "pub fn render_card")
    stats_code = stats_code.replace("fn render_model_detail", "pub fn render_model_detail")
    
    with open("src/ui/stats.rs", "w") as out:
        out.write(stats_code)
    
    # Remove from repl.rs
    new_content = content.replace(match.group(1), "")
    # Wait, need to add import to repl.rs: use crate::ui::stats::SessionStats;
    new_content = new_content.replace("use crate::ui::{TuiApp, RenderLine, cycle_mode, cycle_mode_back};", "use crate::ui::{TuiApp, RenderLine, cycle_mode, cycle_mode_back};\nuse crate::ui::stats::SessionStats;")
    
    with open("src/cli/repl.rs", "w") as out:
        out.write(new_content)
    
    print("Success")
else:
    print("Pattern not found")

