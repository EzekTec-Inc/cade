# ADR-0030: Tool continuation, capability readiness and observed outcomes

## Status

Accepted. Refines ADR-0020, ADR-0026 and ADR-0027; preserves ADR-0028's Working Session grant lifetime.

## Decision

Concentrate model completion, capability lifecycle and Run observation in their existing modules. Callers and regression tests cross the same interface. Reuse the existing wire, stdio/HTTP and embedded/HTTP adapters rather than introduce another execution loop or dependency.

- **Tool Continuation:** the private OpenAI Responses module validates terminal output before releasing tool calls. It serializes ordered opaque reasoning items and function-item identities in a versioned `cade:openai-responses:v1:` envelope in the existing optional `LlmToolCall.thought_signature` field. Conversation persistence already carries this field, so no storage migration or parallel transcript is needed. The historical field name now means provider continuation metadata; Gemini must never receive an OpenAI envelope. Include that metadata in context accounting, retain a bounded user anchor when splitting history, and keep the active parallel call/result exchange intact when the database window cuts into it.
- **CapabilityIntent preparation:** resolve filesystem arguments lexically against Execution Scope before authorization. Preserve ambiguous identities, revisions, repository-relative content paths and URIs. Generic `source`, `target`, `project` and `files` keys are not evidence of filesystem meaning; known filesystem operations supply the needed context. Execute the exact prepared value rather than repeat filesystem resolution after approval.
- **Capability Readiness:** the MCP manager owns process generations, configuration freshness and cancellation-independent recovery. Permission evaluation binds to the implementation generation actually invoked; a replacement requires fresh authorization. Reconnect restores availability but cannot automatically repeat a mutating invocation whose effects are unknown. The live catalog drives model visibility; a settled projection reconciles SQLite transactionally, and catalog changes invalidate cached model context.
- **Run outcome publication:** terminal status and its journal event commit together. Only then publish terminal completion. If publication fails, attempt to record an error outcome; if even recovery storage fails, surface an explicit incomplete observation rather than manufacture success. A client stops on verified terminal evidence, not EOF, a provider finish reason or an empty message list. Durable status can recover missing terminal delivery; an explicit unrecoverable finalization diagnostic ends observation with an error.
- **Working Session commands:** one declaration drives recognition, aliases, completion and help. Session transitions publish live Agent/Conversation identity only after session persistence succeeds. Queue admission runs on every outer iteration. Terminal and Lua busy controls use the same control dispatcher. `/stream` controls text/reasoning presentation only: decisions and tool progress remain live, and deferred partial text flushes once when observation ends.

## Trade-offs and limits

The continuation envelope avoids schema churn but cannot reconstruct reasoning discarded by earlier versions. It currently preserves reasoning associated with tool calls; standalone reasoning-only responses have no tool-call carrier. Explicit configured wire endpoints remain authoritative rather than being silently replaced by model-name guesses.

Binding invocation to a generation may reject an otherwise retryable call after reconnect; reauthorization is preferable to invoking an unreviewed replacement. Unknown write outcomes require verification of effects before retrying. If storage rejects every status update, the durable Run row can remain `running`; the live client reports the persistence failure, and this change does not add a background repair process.

Filesystem preparation is not inference from arbitrary external schemas. Existing path-key conventions and identified filesystem operations are supported; unknown ambiguous values are preserved. Path grants remain authoritative at execution.

## Verification

Regression tests exercise persisted GPT-5/GPT-6 continuation with low budgets, actual loopback HTTP and Rust stdio fixtures, reload/approval races, uncertain writes, cancelled recovery, SQLite terminal-publication failures, HTTP/embedded observation, and command transitions with active terminal/Lua controls. Fixture success does not establish live-provider compatibility for every configured model, non-Linux runtime behavior, or relative performance against another CLI.
