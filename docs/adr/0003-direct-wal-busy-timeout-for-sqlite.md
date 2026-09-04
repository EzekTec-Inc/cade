# ADR 3: Direct WAL with Busy Timeout for SQLite Concurrency

* **Status**: Accepted
* **Decided on**: 2026-06-25

## Context

CADE executes multiple concurrent subagent threads (up to `CADE_MAX_SUBAGENTS`, default `4`) that read and write concurrently to a single central SQLite database file for conversation history, logs, and memory updates. 

SQLite only allows a single active writer at any time. We need a design pattern to handle write contentions without introducing complex, heavy asynchronous message queues or single-writer channel workers that add overhead to CADE's desktop binary.

## Decision

We decided to retain a direct write model using SQLite's **Write-Ahead Logging (WAL)** mode combined with a generous `busy_timeout` of `5000ms`:

```sql
PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;
PRAGMA busy_timeout=5000;
```

Rather than implementing a complex multi-threaded write queue or dedicated tokio channels, we rely on SQLite's internal, lock-free concurrent readers and thread-blocking write serialization. 

## Consequences

### Positive (Pros)
* **Simplicity**: No extra asynchronous channels, message queues, or background writer threads are required. The Rust database code remains clean and standard.
* **Low Memory & CPU Overhead**: We leverage SQLite's highly optimized, native C-level locking mechanisms.
* **Sufficient for Desktop Scale**: Since parallel subagent concurrency is strictly capped (typically $\le 4$), write contentions are extremely brief, and a 5-second wait window guarantees that threads will write successfully without failing.

### Negative (Cons)
* **Scaling Ceiling**: If CADE scales to support dozens of highly concurrent, rapid-fire parallel write threads, the blocking 5-second wait model could degrade latency compared to an asynchronous non-blocking write channel.
