# CADE Python SDK (`cade-sdk`)

Official Python bindings for CADE. Embed autonomous coding agents, multi-agent squads, and persistent memory in Python applications with zero external server daemons.

## Installation

```bash
pip install cade-sdk
```

## Quickstart

### 1. In-Process Agent Session

```python
from cade_sdk import EmbeddedSession

# Initialize zero-daemon in-memory agent
with EmbeddedSession(model="anthropic/claude-sonnet-4-5") as session:
    # Memory management
    session.set_memory("project_rule", "Strict TDD")
    print("Memory:", session.get_memory("project_rule"))

    # Execute prompt
    response = session.prompt("Inspect workspace and describe structure.")
    print("Agent Response:\n", response)
```

### 2. Multi-Agent Squad Orchestration

```python
from cade_sdk import TeamSession

with TeamSession(team_id="security-squad", mode="coordinate") as squad:
    results = squad.run("Audit codebase for security vulnerabilities.")
    for res in results:
        print(f"Task {res['task_index']}: {res['output']}")
```
