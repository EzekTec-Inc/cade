# Implementation Plan: Memory Alignment

## Phase 1: Database Schema & Migrations
- [x] Task: Refactor Memory Blocks Schema for Shared Memory a8b3c9
    - [x] Read `src/server/storage/sqlite.rs`.
    - [x] Add migration to create `shared_memory_blocks` and `agent_memory_blocks` tables.
    - [x] Migrate existing data from `memory_blocks` to the new tables.
- [x] Task: Implement FTS5 for Archival Memory b7d2e1c
    - [x] Add migration to create an FTS5 virtual table for `messages` (`messages_fts`).
    - [x] Create triggers to keep `messages_fts` updated on insert/update/delete of `messages`.
- [x] Task: Conductor - User Manual Verification 'Phase 1: Database Schema & Migrations' (Protocol in workflow.md)

## Phase 2: Update Storage Logic
- [x] Task: Update memory block storage methods c9d2e1f
    - [x] Update `upsert_memory_block`, `delete_memory_block`, `get_memory_blocks`, and `get_memory_blocks_with_ts` to use the new schema.
- [x] Task: Update message search methods d8e2f1a
    - [x] Update `search_messages` to use the FTS5 virtual table instead of `LIKE`.
- [x] Task: Conductor - User Manual Verification 'Phase 2: Update Storage Logic' (Protocol in workflow.md)

## Phase 3: Final Polish
- [x] Task: Run tests and verify e1a2b3c
    - [x] Ensure `cargo test` passes.
- [x] Task: Conductor - User Manual Verification 'Phase 3: Final Polish' (Protocol in workflow.md)