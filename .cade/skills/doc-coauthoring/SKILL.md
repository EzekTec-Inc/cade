---
id: doc-coauthoring
name: doc-coauthoring
description: Guide users through a structured workflow for co-authoring documentation.
---

# Doc Co-Authoring Workflow

Collaboratively create documents across three stages: Context Gathering, Refinement & Structure, and Reader Testing.

## Stage 1: Context Gathering
Offer the user this structured workflow or freeform. Ask meta-questions:
1. Document type & template?
2. Target audience & desired impact?
3. Constraints, dependencies, and timeline?

Encourage a raw info dump (messaging threads, slack connector, links). Ask 5-10 targeted clarifying questions about gaps/edge cases. Allow shorthand replies.

## Stage 2: Refinement & Structure
1. **Scaffold**: Create a file with markdown headers and placeholders (e.g. `[To be written]`).
2. **Drafting (Section-by-Section)**:
   * Ask 3-5 specific questions about what to include.
   * Brainstorm 5-15 bulleted ideas.
   * Ask user to keep/remove/combine ideas (e.g. "Keep 1, 3; Combine 5 & 6").
   * Draft the section text using `str_replace` or `edit_file`. Never rewrite/reprint the whole file.
   * Ask for feedback and apply surgical edits.
3. **Consistency check**: Review the complete document for flow, redundancy, filler text, or contradictions.

## Stage 3: Reader Testing
Test the document with a fresh sub-agent to find blind spots.
1. **Predict**: List 5-10 reader search questions.
2. **Execute**: Spawn a read-only sub-agent (`claude-3-5-haiku-latest`) containing only the document text. Query it with the predicted questions.
3. **Fix**: Identify errors, false assumptions, or ambiguities found by the reader sub-agent, edit the file, and re-test until perfect.
