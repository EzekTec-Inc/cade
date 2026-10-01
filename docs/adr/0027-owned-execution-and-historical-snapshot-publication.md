# ADR-0027: Owned execution scopes and historical snapshot publication

## Status

Accepted. Amends the timestamp-only filtering detail in ADR-0007 while preserving its captured historical-anchor intent.

## Decision

Resolve each Run's workspace, permissions, hooks, backend, and limits once at acceptance, and carry that owned Execution Scope through parent and child execution. HTTP and embedded adapters use the same runtime; process-wide directory or environment mutation cannot bind concurrent Runs to a workspace.

Consolidation captures history and managed Memory Block identities under an agent-and-conversation claim, performs model work outside a database transaction, and publishes summary rotations and exact summarized-message coverage in one fenced transaction. Historical anchors include a stable insertion sequence and captured insertion bound. Exact coverage also preserves retained backdated messages after clock regression: neither a timestamp-only filter nor an advancing snapshot bound alone identifies the summarized prefix. The subsequently inserted marker's identity must never substitute for captured source identities, which would repeat the race described in ADR-0007.

## Consequences

- Preserve `RunRequest` constructors and legacy transport behavior; explicit execution options extend acceptance rather than introducing another execution loop.
- Parent and child tools authorize and execute the same normalized arguments. Child grants narrow inherited grants, including when an isolated workspace rebases their paths.
- Migrations 23 and 24 preserve existing message identifiers and timestamps. Ambiguous legacy timestamp markers conservatively retain their anchor second; ambiguous version-23 backdated intervals replay rather than discard possibly unsummarized messages.
- Raw archival is an independent durable outcome when summarization fails; it cannot advance the published horizon.
- Workspace reconciliation compares against its captured baseline before applying changes. It rejects conflicting deltas; per-file replacement is atomic and multi-file rollback is best-effort rather than a filesystem transaction.
