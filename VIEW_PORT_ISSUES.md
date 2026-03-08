# Refactor Plan: "Forced Continuation Pattern"

## Problem Statement
The LLM agent successfully decides to use a tool (e.g., `git status`), receives the tool's execution result from the environment, but then stops generating tokens, leaving the user without a natural language explanation of the outcome. This usually stems from the LLM misinterpreting the "tool response" role block as the end of the conversational turn.

## Phase 1: Agent Loop Architecture Updates (The Application Layer)
1. **Implement a Strict Return Condition:** 
   Update the application code that orchestrates the LLM loop. Currently, the loop likely breaks simply because the LLM stops generating. Refactor the loop condition:
   *   **Old:** `while (llm_wants_to_call_tool) { execute_tool(); } return to_user;`
   *   **New:** `while (true) { response = call_llm(); if (response.has_tool_call) { execute_tool_and_append_result(); } else if (response.has_text) { return response.text; } }`
   The loop must *only* terminate and yield control back to the user when the LLM explicitly generates a text payload that does not contain a tool call.

2. **Auto-Re-prompting on Empty Yields:**
   If the LLM receives a tool response and returns an empty message (stops generating without text or a new tool call), the application layer should automatically inject a system prompt:
   `"System: Tool execution complete. You must now provide a final text response to the user summarizing the outcome or answering their question based on the tool data."`

## Phase 2: System Prompt Updates (The LLM Layer)
1. **Explicit Termination Rules:**
   Add a strict directive to the system prompt regarding turn completion.
   *   *Directive:* "Every tool call you make will return a result block. You MUST NOT consider your turn finished when you receive a tool result. After receiving a tool result, you MUST generate a follow-up text response explaining the findings to the user, unless you need to call another tool."

2. **Require a "Response Flag" (Structured Output):**
   If the LLM supports structured outputs (JSON mode), refactor the output schema to force the LLM to categorize its generation.
   ```json
   {
     "thought": "I need to check git status.",
     "tool_calls": [...],
     "final_answer_ready": false,
     "final_answer": null
   }
   ```
   When `final_answer_ready` is false, the application loop knows it must feed the tool output back and prompt the LLM again until `final_answer_ready` is true and `final_answer` is populated.

## Phase 3: Fine-tuning / Few-Shot Examples (The Context Layer)
1. **Inject Correct Interaction Examples:**
   Update the few-shot examples in the context window to explicitly demonstrate the desired behavior. Show an example where a tool is called, the system returns the result, and the Assistant immediately follows up with a text explanation.
   *   *User:* "What's the status?"
   *   *Assistant:* `[Call tool: git status]`
   *   *Tool:* `[Result: 3 files modified]`
   *   *Assistant:* "You have 3 modified files waiting to be staged."

## Execution Strategy for the LLM
If an LLM model is asked to implement this plan on a codebase, it should execute the following steps:
1. Locate the main agent loop (e.g., in `main.rs`, `agent.rs`, or `loop.py`).
2. Modify the `while` loop to enforce that a text-only response is the only exit condition.
3. Locate the file containing the `system_prompt`.
4. Append the "Explicit Termination Rules" to the prompt.
5. Add handling for edge cases where the model returns an empty string after a tool call.