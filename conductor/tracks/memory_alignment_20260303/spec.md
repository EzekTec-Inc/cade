# Specification: Implement Letta-style memory alignment

## Overview
This track implements key findings from the Letta memory architecture investigation, specifically focusing on Shared Memory and Archival Memory enhancements.

## Objectives
1.  **Shared Memory:** Modify the database schema to support many-to-many relationships between agents and memory blocks, enabling real-time coordination via shared state.
2.  **Archival Memory (FTS5):** Enhance the historical message search by implementing SQLite's FTS5 (Full-Text Search) extension, replacing the inefficient `LIKE` queries for better semantic retrieval capabilities.

## Requirements
-   Update `src/server/storage/sqlite.rs` schema safely (with migrations).
-   `agent_memory_blocks` junction table should link `agent_id` and `block_id`.
-   Migrate existing `memory_blocks` to the new many-to-many structure.
-   Create an FTS5 virtual table for messages to enable advanced archival search.
-   Update existing API and search methods to use the new structures.
-   Maintain backwards compatibility where possible or provide seamless migrations.