# Agents and Conversations

A CADE **agent** is a persistent, named identity — its own memory, its
own model preference, its own conversation history. A **conversation**
is a single ordered thread within an agent. **Checkpoints** snapshot the
working tree alongside specific points in a conversation.

## Agents

```bash
/agents              # list agents on the server
/new-agent           # create one (interactive)
/agent               # show the active agent
/rename <name>       # rename
/delete [name]       # delete (current if no arg)
/pin                 # set as global default
/info                # detailed agent + workspace info
```

### Agent resolution on startup

CADE picks the active agent in this order:

1. **CLI flag** — `--new-agent`, `--agent <id>`, or `--name <query>`
2. **Local project agent** — `agent_id` in `.cade/session.json`
3. **Global last agent** — `last_agent_id` in `~/.cade/settings.json`
4. **Create new** — fallback if no saved agent matches

### Agent settings

Each agent stores:

- `model` — provider/name (e.g. `anthropic/claude-sonnet-4-5`)
- `compaction_model` — optional override; cheap model used for
  consolidation (see [memory-system.md](memory-system.md))
- `permission_mode` — `default | acceptEdits | plan | bypassPermissions`
- `toolset` — `default | codex | gemini`
- `reasoning_effort` — `none | low | medium | high | xhigh`

All editable inline:

```bash
/model anthropic/claude-sonnet-4-5
/compaction-model anthropic/claude-3-5-haiku-latest
/mode plan
/toolset codex
/reasoning high
```

## Conversations

A conversation is a chronologically ordered set of messages within an
agent. The CLI auto-titles new conversations from the first user message.

```bash
/new                 # start a new conversation on the current agent
/resume              # browse + switch to a past conversation
```

Older messages are paginated server-side; the CLI fetches additional
pages as you scroll up.

## Checkpoints

A checkpoint captures:

- The `git HEAD` commit hash
- An optional label and description
- A pointer to the conversation message after which it was created

```bash
/checkpoint pre-refactor      # save (label optional)
/cp                           # alias
/tree                         # browse + restore (fullscreen picker)
/checkpoints                  # alias
/undo                         # restore the most recent checkpoint
/fork [label]                 # branch a new conversation from a checkpoint
```

Restore checks out the recorded commit hash, returning the working tree to the exact committed state at that point. No-op restore (clean tree, same commit) is
free.

### When to checkpoint

- Before risky edits — large refactors, generated code, file moves
- Before invoking a subagent that will edit files
- Before any tool call that will run for more than a few seconds

The `software-engineer` skill recommends it; `strict-project-execution`
mandates it for "destructive operations".

## Forks

`/fork` creates a new conversation that starts from a chosen
checkpoint's working-tree state. The original conversation is unchanged;
the fork is a sibling. Useful for exploring two competing approaches.

## Artifacts

Long tool outputs (logs, diffs, fetched docs) get persisted as
**artifacts** — content-addressed, retrievable by id, never re-prompted
into the LLM unless explicitly requested.

```bash
/artifacts                    # list
```

Tools that produce them: `store_artifact`, `archival_memory_insert`, plus
auto-extraction during consolidation.

## Background runs

Long agent turns can be detached. The CLI supports
`cade --prompt "<task>" --output-format json` headless mode, which
persists the run server-side and emits structured JSON. From within a
session, the SSE stream auto-detaches if you `Ctrl+C` once — re-attach
later with the run id.

```
GET  /v1/runs/:id            # status snapshot
GET  /v1/runs/:id/stream     # SSE re-attach
```
