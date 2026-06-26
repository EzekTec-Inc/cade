# ADR 7: Historical High-Water Mark Pinning for Consolidation Boundary Markers

* **Status**: Accepted
* **Decided on**: 2026-06-25

## Context

CADE employs a background summarization agent (the **Sleeptime Agent** in `crates/cade-server/src/server/consolidation.rs`) to compress older conversation history once active token windows near saturation. This compaction process requires calling an external LLM, which introduces 5 to 10 seconds of network and completion latency.

Upon completion, CADE inserts a boundary message (`role = 'compaction'`) into the database. 

Previously, this boundary was evaluated strictly using the relative SQLite `rowid > marker_rowid` filter. However, this introduced a severe state-invalidation race condition:
1. At start, CADE reads messages up to time $T_0$ to summarize.
2. During the 10-second LLM run, a user or peer subagent writes new messages (e.g. `rowid = 100`, `101`) to the DB.
3. The consolidator completes and inserts the compaction boundary marker which gets `rowid = 102`.
4. In the next turn, `list_messages_since_last_compaction` queries `rowid > 102`.
5. Messages `100` and `101` (which were never summarized, because they were written *during* the LLM latency!) have `rowid < 102`. They are completely skipped, leading to permanent, silent context loss.

## Decision

We decided to replace the relative `rowid > marker_rowid` boundary checks with a **Historical High-Water Mark Pinning** check using absolute created-at timestamps:

1. **Newest Dropped Anchor**: The consolidator identifies the `created_at` timestamp of the newest dropped message that is actually being sent to the LLM.
2. **Boundary Pinned Timestamp**: The compaction marker is created with its `created_at` timestamp set exactly equal to this high-water mark timestamp (`marker_ts`).
3. **Absolute Filtering**: Both `list_messages_since_last_compaction` and `get_context_window` are refactored to find the latest compaction marker's `created_at` timestamp, and filter strictly by `created_at > (SELECT marker_ts FROM boundary)`.

Since any message created *during* the LLM's completion latency gets a current system timestamp (which is $> \text{marker\_ts}$), those messages are guaranteed to be positioned *after* the boundary and will be correctly summarized in the subsequent consolidation run.

## Consequences

### Positive (Pros)
* **Strict Concurrency Protection**: Eliminates context loss caused by concurrent writes during long LLM completion cycles.
* **Backwards-Compatibility**: Integrates seamlessly with existing message-retrieval and windowing APIs.
* **Monotonic Integrity**: Uses robust database-backed timestamp filters.

### Negative (Cons)
* **Clock Dependencies**: Relies on system clock monotonic progression. (Since LLM completions take seconds, minor millisecond clock-drift poses zero practical risk).
