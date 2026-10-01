# ADR-0029: Ignore-aware incremental RAG indexing and responsive retrieval

## Status

Accepted

## Context

CADE launches the external `cade-rag-mcp` binary configured in `.cade/settings.json`. Its implementation lives in the sibling `mcp-servers/cade-rag-mcp` repository.

A stdio reproduction showed that `.gitignore` was bypassed in non-Git workspaces. The watcher separately cached only the root ignore file, opened a database per changed file, and embedded chunks individually in unbounded tasks. Indexing also eagerly loaded an unused cross-encoder, ran blocking work on the async executor, and recalculated line numbers by repeatedly scanning source prefixes. Search requested unsupported client sampling and reread whole files instead of returning the indexed snapshot.

## Decision

- Keep the MCP tools as the external seam. Put discovery, ignore policy, metadata comparison, bounded embedding batches, transactional replacement, and pruning behind the `WorkspaceIndex::refresh` interface. Explicit indexing and watcher updates use this same deep module.
- Use `ignore::WalkBuilder` with `require_git(false)` for parent and nested ignore semantics. A watcher event burst triggers incremental reconciliation through that walk, including ignore-rule changes and deletions. Exclude index/model-cache internals and bound the pending refresh queue.
- Serialize refreshes per canonical workspace. Keep embedding and reranking models independently lazy; load them and run inference on blocking threads. FIFO model access is released between 32-chunk embedding batches so a query can interleave with large-file indexing. Unchanged refreshes load neither model.
- Use nanosecond file versions and linear prefix accounting for chunk line numbers. Existing second-precision entries are rebuilt on their first refresh.
- Batch SQLite writes and pruning, enable WAL and a busy timeout, and index chunk paths for efficient cascading replacement/deletion.
- Rank candidate IDs before retrieving document text, with FTS relevance ordering and one read snapshot. Return stored chunks and line numbers. Sampling requires advertised support and has a bounded fallback.

## Consequences

Callers retain the existing tool names and parameters; retrieval explicitly reflects the indexed snapshot. Watchers trade one metadata scan per relevant debounced burst for consistent ignore semantics, bounded work, and locality. Cross-encoder reranking still has an intrinsic model-loading/inference cost; `rerank: false` avoids that cost.

Verification crosses the same stdio seam as CADE. Rust tests additionally exercise the internal seams for model-free refreshes, parent/nested rules, transactional rollback, and concurrent WAL readers. The configured release binary must be rebuilt and the MCP connection restarted to load these changes.
