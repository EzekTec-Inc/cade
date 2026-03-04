# Specification: Enhance interactive REPL UI with richer terminal output

## Overview
This track aims to improve the command-line user experience of CADE by introducing richer terminal outputs in the REPL (Read-Eval-Print Loop). The goal is to make the CLI feel more responsive, visually appealing, and informative, adhering to the "Minimal & Terse Output", "Rich Terminal UI", and "Speed & Responsiveness" principles defined in the Product Guidelines.

## Objectives
1.  **Improve Markdown Rendering:** Enhance the parsing and rendering of Markdown in the terminal (handling bold, italics, code fences, lists, etc.) using `ratatui`.
2.  **Add Visual Feedback:** Implement or improve spinners, progress bars, or loading indicators for background tasks and long-running operations.
3.  **Structured Layouts:** Introduce structured layouts for displaying complex information (e.g., tool schemas, memory blocks, skills listing) instead of plain text dumps.
4.  **Streaming Polish:** Ensure Server-Sent Events (SSE) streaming renders smoothly token-by-token without visual artifacts or layout breaking.

## Requirements
-   All UI components must be implemented using `ratatui` and `crossterm`.
-   The changes must not degrade the performance of the CLI.
-   The UI must gracefully handle terminal resizing.
-   Outputs should remain minimal and terse, focusing on high-signal information.
-   Existing features (slash commands, headless mode) must not be broken.