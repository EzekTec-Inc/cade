# CADE execution and capability refactor

This implementation follows the seven selected deepening candidates. Each module owns caller-visible behavior, and behavior tests cross the same interface as production callers.

## Canonical Run execution

`ServerAgentRuntime` accepts both HTTP and embedded work. `RunRequest` retains its existing fields. `RunExecutionOptions` supplies optional workspace (`cwd`, with `workspace` as a wire alias), path grants, permissions, execution profile, reasoning settings, and a hard turn limit. In-process callers can inject their actual backend/runtime.

Acceptance validates Agent/Conversation ownership and workspace configuration, resolves one Execution Scope, and persists durable work before spawning execution. There is no invented run identifier on storage failure. Tools persist their full results before corresponding result events and the next context build. Provider stream failures become failed Runs.

Local native file, search, patch, shell, planning, and audit operations use the selected workspace without changing process cwd. Path grants are checked before local or alternate-backend dispatch. An alternate backend must implement an operation or reject it; unsupported search/patch operations cannot silently run on the host.

Subagents own their captured scope even after a parent disconnects or cancels. Scope narrowing and isolated-path rebasing preserve inherited restrictions. Session completion owns outcome publication, permits, ephemeral state, and workspace discard/reconciliation.

## Providers and models

Provider protocols remain explicit Rust adapters. Provider names, endpoints, defaults, model identities, capability recipes, token limits, tokenizers, and routing preferences are configuration/discovery data.

See [the provider runtime guide](../crates/cade-ai/README.md) for `CADE_PROVIDERS_CONFIG`, `CADE_MODELS_CONFIG`, custom gateways, aliases, metadata precedence, fallback estimates, and configuration examples. Restart after editing process-loaded model metadata, or update the registry through the SDK.

Complete, stream, and structured invocations pair the selected endpoint with the correct request encoder and response decoder. Concurrent routing takes a short snapshot and releases its lock before network I/O. Configured fallback routes preserve request meaning and do not replay already-delivered streams.

Unknown models are not rejected by a static allowlist. Unknown capabilities remain explicitly unknown, and conservative budget estimates are not represented as verified upstream limits. Deterministic loopback transport tests verify protocol contracts; live provider credentials and every upstream deployment remain separate verification requirements.

## Consolidation

Migration 23 adds stable historical ordering and fenced consolidation claims; migration 24 records exact summarized-message coverage, including recovery of ambiguous version-23 backdated intervals. A claim captures its history and managed Memory Blocks; model work holds no database transaction. Summary rotation, revisions, links, tiers, index, and horizon publication commit together or roll back together. Competing or expired workers cannot publish stale state.

Same-second retained messages and messages arriving during summarization remain visible. Independent same-label blocks are updated by their linked identity; intentionally shared blocks retain shared semantics. See [ADR-0027](adr/0027-owned-execution-and-historical-snapshot-publication.md).

## Conversation presentation and decisions

The browser chat-session module owns selection epochs, optimistic/persisted identity reconciliation, run ownership, and replay deduplication. Shared wire interpretation accepts production envelopes and legacy shapes. Mutable live-message content does not become an immutable cache entry.

Terminal questions and permissions share a Decision Dialog: adaptive floating presentation when space permits and compact presentation on constrained terminals, active theme tokens, formatted details, aligned/wrapped choices, freeform answers, scrolling, and mouse/keyboard routing. Explicit approval choices cannot be impersonated by custom text. Modal input preserves the composer draft.

Idle and active terminal scheduling share the production event pump. Lua tools and callbacks receive work wakeups, request queues are bounded, and concurrent tool admission is limited. Lua slash commands wait for normal REPL command dispatch during active work or decisions. PreparedCache owns width/theme/font/content validity and retires obsolete prepared artifacts without independently pruning Conversation history.

## Capabilities and workflows

Plugin catalogue and guarded dispatch use the same ready definitions and collision precedence. Native scripts execute through permissions and hooks in the accepted workspace; remote/read-only backends reject incompatible native execution. Removal invalidates readiness and cached context. Archive checksums are supported, installation is staged, and activation failure rolls back installation.

Plugin scripts are native processes, not WASM sandboxes. Other declared execution kinds must not be reported as implemented by that path.

Workflows validate dependencies, use canonical Run execution for their steps, propagate actual failure/cancellation, and skip dependent work after failure. Legacy webhook `execution_id` names the accepted canonical Run; `run_id` names the workflow aggregate. Workflow event streams currently cover active executions, not durable replay or restart recovery.

## Evaluation and comparison

`cade eval` uses isolated task copies, resolves its model from configured server state unless overridden, rejects empty verifier sets, counts failed attempts in the denominator, preserves runtime usage and phase timings, and records completion. Timeout/error cleanup cancels unfinished Runs and deletes ephemeral Agents.

For an OpenCode comparison, pin the exact executable revision and use two suites:

1. Matched model, reasoning, tools, instructions, inputs, machine, and budgets.
2. Each product's best supported configuration under common acceptance criteria and budgets.

Report first-attempt independently verified success, failures/timeouts, time to verified completion, total cost per accepted task, long-session retention, recovery, and event-to-visible responsiveness. No comparative superiority is established by compilation, implementation language, or mock-provider checks.

## Verification

```sh
cargo check --workspace --all-targets
cargo test --workspace --no-fail-fast
cargo check -p cade-gui --target wasm32-unknown-unknown
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Native workspace/lifecycle, transactional history, provider protocol, wire reconciliation, TestBackend dialogs, real plugin scripts, actual workflow outcomes, and evaluation accounting have dedicated regression tests. Native Windows/macOS execution, live provider access, browser rendering, full PTY interaction, and comparative work outcomes require their respective environments rather than being inferred from the Linux test suite.

## Existing Docker volumes

Compose now mounts persistent state at the existing non-root image user's home, `/home/cade/.cade`, rather than `/root/.cade`. Preserve the named volume and back it up before upgrading. If its existing files are root-owned, stop the server and adjust that volume's ownership once before restarting:

```sh
docker compose stop cade-server
docker compose run --rm --no-deps --user root cade-server \
  sh -c 'chown -R 10001:10001 /home/cade/.cade'
docker compose up -d --build cade-server
```

This operates on the existing `cade_data` mount; it does not delete or replace it. Host-mounted workspaces separately need suitable permissions for the container user. The image sets `CADE_SERVER_HOST=0.0.0.0` for published-port access; native launches retain the loopback default. `CADE_SERVER_HOST` also accepts an IPv6 address. Bearer authentication continues to protect execution.
