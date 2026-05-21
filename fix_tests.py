import re

def fix_file(path):
    with open(path, 'r') as f:
        content = f.read()

    # Fix DashMap init in tests
    content = content.replace("agent_metrics: std::sync::Arc::new(tokio::sync::RwLock::new(\n            std::collections::HashMap::new(),\n        ))", "agent_metrics: std::sync::Arc::new(dashmap::DashMap::new())")
    content = content.replace("agent_metrics: std::sync::Arc::new(tokio::sync::RwLock::new(\n                std::collections::HashMap::new(),\n            ))", "agent_metrics: std::sync::Arc::new(dashmap::DashMap::new())")
    content = content.replace("agent_metrics: Arc::new(RwLock::new(std::collections::HashMap::new()))", "agent_metrics: Arc::new(dashmap::DashMap::new())")
    content = content.replace("agent_metrics: Arc::new(AsyncRwLock::new(std::collections::HashMap::new()))", "agent_metrics: Arc::new(dashmap::DashMap::new())")

    with open(path, 'w') as f:
        f.write(content)

for p in [
    "crates/cade-server/src/server/api/auth_test.rs",
    "crates/cade-server/src/server/api/compact.rs",
    "crates/cade-server/src/server/api/complete.rs",
    "crates/cade-server/src/server/api/context_stats.rs",
    "crates/cade-server/src/server/api/dashboard_test.rs",
    "crates/cade-server/src/server/api/edit.rs",
    "crates/cade-server/src/server/api/evals_test.rs",
    "crates/cade-server/src/server/api/messages/tests.rs",
    "crates/cade-server/src/server/api/run/tests.rs",
    "crates/cade-server/src/server/api/skills.rs",
    "crates/cade-server/src/server/api/router_test.rs",
    "crates/cade-server/src/server/consolidation.rs"
]:
    fix_file(p)

def fix_state():
    with open("crates/cade-server/src/server/state.rs", "r") as f:
        content = f.read()
    
    # Fix asserts
    content = re.sub(r'assert_eq!\(m\.([a-zA-Z0-9_]+), (.*?)\);', r'assert_eq!(m.\1.load(std::sync::atomic::Ordering::Relaxed), \2);', content)
    
    # Fix struct init in tests
    # input_tokens_total: 1_000_000, -> input_tokens_total: 1_000_000.into(),
    # We can just replace the specific lines
    content = content.replace("input_tokens_total: u64::MAX - 5", "input_tokens_total: (u64::MAX - 5).into()")
    content = content.replace("input_tokens_total: 1_000_000", "input_tokens_total: 1_000_000.into()")
    content = content.replace("output_tokens_total: 200_000", "output_tokens_total: 200_000.into()")
    content = content.replace("cache_read_tokens_total: 5_000_000", "cache_read_tokens_total: 5_000_000.into()")
    content = content.replace("cache_write_tokens_total: 100_000", "cache_write_tokens_total: 100_000.into()")
    content = content.replace("input_tokens_total: 999_999_999", "input_tokens_total: 999_999_999.into()")
    content = content.replace("output_tokens_total: 999_999_999", "output_tokens_total: 999_999_999.into()")
    content = content.replace("cache_read_tokens_total: 999_999_999", "cache_read_tokens_total: 999_999_999.into()")
    content = content.replace("cache_write_tokens_total: 999_999_999", "cache_write_tokens_total: 999_999_999.into()")
    
    with open("crates/cade-server/src/server/state.rs", "w") as f:
        f.write(content)

fix_state()
