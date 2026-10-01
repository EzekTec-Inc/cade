# Provider runtime and discovery

`LlmRouter` is the canonical routing implementation. `ConcurrentRouter` wraps
`Arc<tokio::sync::RwLock<LlmRouter>>` and takes a short-lived routing snapshot
before network I/O. Both delegate completion, streaming, structured output, model
validation, and discovery. `make_provider` also uses this runtime.

## Editable configuration

- Provider definitions: `~/.cade/providers.json`, or `CADE_PROVIDERS_CONFIG`.
  This is an array of definitions. Older entries remain OpenAI-compatible by
  default. Bundled entries are merged by name; unspecified bundled fields are
  inherited. Overriding `chat_url` derives discovery from that gateway unless
  `models_url` is explicitly configured. Missing/null `models_url` permits
  protocol-based URL derivation; `discovery: false` disables listing explicitly.
- Execution/model metadata: `~/.cade/models.json`, or `CADE_MODELS_CONFIG`.
  Rules and registrations override bundled compatibility data. Missing fallback
  fields inherit conservative bundled budgets. The bundled template is
  `src/default_models.json`; its `seeded_models` holds editable exact offline
  compatibility records. `CATALOGUE` is a lazy owned compatibility snapshot of
  those records, exact registrations, and runtime discovery; no Rust model table
  or separate model-database limits are used.
- Existing pricing configuration remains independent of execution capabilities.

Partial provider overrides can inherit native endpoint/kind/default fields.
Provider names win over aliases; ambiguous aliases, invalid environment names,
non-HTTP endpoints, and malformed configured headers reject the custom registry
atomically. Missing/unreadable/invalid files recover to bundled JSON with a
diagnostic. Seeding uses create-new semantics and never truncates an existing file.
`display_name` and `headers` configure gateway attribution and transport headers;
there are no host-name-specific label/header branches.

Provider/model files are configuration, not allowlists. Arbitrary provider names
can select one of the supported Rust adapters: `anthropic`, `openai`, `gemini`,
`ollama`, or `openai-compatible`. Arbitrary registered and unlisted upstream
model IDs are accepted. Only the **first routing prefix** is removed; nested
upstream IDs and registered upstream IDs beginning with a provider name survive.

Example provider definition:

```json
[
  {
    "name": "office",
    "kind": "openai-compatible",
    "env_vars": ["OFFICE_API_KEY"],
    "chat_url": "https://gateway.example/v1",
    "models_url": "https://gateway.example/catalog/models",
    "default_model": "tenant/deployment",
    "fast_model": "tenant/small-deployment",
    "priority": 50
  }
]
```

`default_model` and `fast_model` are **upstream IDs**. Server-selected defaults
are qualified with the definition's provider name; `CADE_DEFAULT_MODEL` is an
explicit routing ID override. A provider without a default requires that override.

`background_model` optionally selects compaction/background work independently
of the fast subagent choice. When absent, configured fast/default choices are
used. All choices preserve nested upstream namespaces. Compatibility rules may
set `prefer_primary_for_background: true` and match `suffixes`, so local or
shared-quota tiers can reuse the primary without provider-ID logic in Rust.
Seeded background choices preserve previous compaction behavior. Explicit agent
`compaction_model` settings bypass this implicit selector.

Example execution configuration (partial overrides inherit bundled defaults):

```json
{
  "fallback": {"max_tokens": 2048, "context_window": 16000},
  "models": [
    {
      "id": "office/tenant/deployment",
      "aliases": ["office-main"],
      "protocol": "responses",
      "token_parameter": "max_output_tokens",
      "reasoning": "nested_reasoning_object",
      "developer_role": true,
      "tools": true,
      "native_structured": true,
      "max_tools": 64,
      "tokenizer": "o200k_base",
      "chars_per_token": 3,
      "max_tokens": 12000,
      "context_window": 128000
    }
  ]
}
```

Use a root OpenAI-compatible URL to allow registered metadata to choose between
`/chat/completions` and `/responses`. A complete endpoint URL explicitly chooses
its protocol, and its request body and decoder follow that choice. The preview
gateway override follows the same pairing. Responses uses `input`,
`max_output_tokens`, and `text.format` for native structured output; Chat
Completions uses `messages`, its registered token parameter, and `response_format`.

Responses tool turns request encrypted reasoning content and preserve reasoning
items and function-item identities through the existing serialized
`LlmToolCall.thought_signature` field. OpenAI uses a versioned
`cade:openai-responses:v1:` envelope; Gemini retains its native signature format
and never sends the OpenAI envelope as a Gemini signature. This metadata is
separate from visible text and arguments, survives Conversation persistence and
context rebuilding, and is included in context-budget estimates. Historical
turns written before this support cannot recover previously discarded reasoning.

Responses streaming publishes tool calls only after terminal output has been
validated. Failed streams, premature EOF/`[DONE]`, unfinished calls, missing call
identities, and empty/malformed/non-object arguments cannot become executable
calls. An incomplete response may retain explicitly completed calls, but an
unfinished call rejects the complete call set. Chat Completions keeps its own
terminal/sentinel compatibility behavior.

Gemini accepts an API root or a `/models` root. Generation, streaming, structured
generation, and cache creation stay on the configured gateway. Authentication
query values are URL encoded. Every generation path includes `maxOutputTokens`.
Thinking options require explicit metadata: `thinking: "budget"` with
`thinking_budgets`, or `thinking: "level"` with `reasoning_values`.

Rules match provider scopes and prefixes in JSON; they are explicitly
**compatibility assumptions**, not live capability verification. Exact editable
registrations override rules and discovery. Discovery refreshes observations
separately so provider limits can change without overwriting configured overrides.
Process metadata is shared with legacy catalogue/budget helpers; isolated SDKs
can pass their own `SharedModelRegistry`.

Earlier matching rules override individual fields, retaining unspecified recipe
fields. Explicit `false` capabilities override seeded/discovered `true` values.
Token/context limits and character ratios must be positive. Registered aliases
and exact IDs resolve before optional dated compatibility aliases, preserving
custom deployment suffixes. Explicit `failover_providers: []` disables failover.

Tokenizers (`cl100k_base`, `o200k_base`, `characters`), character ratios, prompt
cache adapters/boundaries, and tool-selector adapters are selected by this same
metadata. Provider/model substrings in Rust do not choose them. Character counting
rounds upward so a nonempty short message cannot be counted as zero tokens.
Configured BPE vocabularies for non-native models are estimates, not verified
provider tokenizers.

Provider definitions are read at construction/registration. Execution metadata
is loaded on process startup; restart after file edits, or replace the shared
registry/register a model through the SDK. Reconnecting a provider rebuilds its
transport from current configuration.

## Unknown capabilities and discovery

Unknown limits use configurable conservative budgeting values. They are **not
claims about a model's actual limits**. Unknown tool support remains `None`;
caller-supplied tools are passed through for compatibility, while explicit
`tools: false` rejects tool-requiring requests locally. Unknown reasoning modes
are omitted. Unknown native structured capability uses a schema-instructed JSON
parsing fallback, which does not locally validate the full JSON Schema. Register
native structured support where the gateway/model guarantees it.

Discovery uses the registered endpoint and adapter protocol, not provider-name
switches. Gemini generation methods, token limits, and pagination; Anthropic
pagination/display names; OpenAI-compatible IDs and available limit/parameter
metadata; and Ollama installed IDs are supported. Unclassified IDs are not
removed based on model-name prefixes. Failures retain configured/offline or
previously discovered entries, marked `dynamic: false`. `/v1/models` includes
additive `metadata` with source and `limits_are_fallback` information. Legacy
numeric fields and response sections remain available.

Failover providers are configurable (`failover_providers`, seeded with the
existing OpenRouter behavior). Native routes must already be configured. Only
connection/upstream status failures before stream delivery trigger fallback;
streams are never replayed after a successful connection. Native structured
requests retain their schema, and usage is tagged with the actual selected route.
Synchronous validation reports a retryable busy condition during a router write.

## Verification (loopback transport)

Compatibility API changes:

- `CATALOGUE` is `LazyLock<Vec<CatalogueRow>>`; rows retain tuple positions but own
  their strings. `.iter()`, indexing and slice methods work; direct loops should
  use `.iter()`. `ModelEntry::from_catalogue` accepts owned and borrowed tuples.
  `catalogue_snapshot()` returns a fresh owned view after discovery/registration.
- `FALLBACK_CHARS_PER_TOKEN` is a lazy configuration snapshot (dereference it for
  its value). Conversion functions use current metadata, and
  `chars_for_tokens_for_model` honors per-model ratios.
- `RuntimeRegistry::try_register` returns validation errors; compatibility
  `register` logs invalid input without changing the registry.

```sh
cargo test -p cade-ai --test runtime_transport
cargo test -p cade-ai --test runtime_metadata
cargo test -p cade-ai --lib openai::tests::
cargo test -p cade-ai --lib catalogue::tests::
cargo test -p cade-ai --lib anthropic::tests::
cargo test -p cade-ai --lib provider_registry::tests::
cargo test -p cade-ai --lib tokenizer::tests::
cargo test -p cade-ai --lib prompt_cache::tests::
cargo test -p cade-ai --lib registry::tests::
```

The transport suite covers registered/nested IDs, Responses complete/structured
decoding, fragmented SSE, terminal/error events, Gemini generation parameters,
custom discovery/pagination, failover delegation, and lock release during HTTP.
These mocks verify protocol contracts; they do not verify access to live providers.
