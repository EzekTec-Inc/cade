# CADE Inline Completions — VS Code Extension

AI code completions powered by your local [CADE](https://github.com/EzekTec-Inc/CADE) server. Copilot-style ghost text that streams from the same LLM backend you use for chat.

## Features

- **Inline ghost text** — completions appear as you type, accept with `Tab`
- **Streaming** — tokens render progressively for fast perceived latency
- **Stateless** — zero conversation pollution, no DB writes
- **Any model** — uses whatever provider your CADE agent is configured with (Anthropic, OpenAI, Gemini, Ollama)
- **Status bar toggle** — click to enable/disable, or run `CADE: Toggle Inline Completions`

## Setup

1. Make sure your CADE server is running (default port `8284`)
2. Configure the extension:

   | Setting | Env Var Fallback | Default | Description |
   |---------|-----------------|---------|-------------|
   | `cade.agentId` | `CADE_AGENT_ID` | — | **Required.** Your CADE agent ID |
   | `cade.apiKey` | `CADE_API_KEY` | — | Bearer token for server auth |
   | `cade.serverPort` | `CADE_SERVER_PORT` | `8284` | Local server port |
   | `cade.enabled` | — | `true` | Enable/disable completions |
   | `cade.linesBefore` | — | `50` | Context lines before cursor |
   | `cade.linesAfter` | — | `20` | Context lines after cursor |

3. Start typing — completions appear automatically.

## Build from Source

```bash
cd editors/vscode
npm install
npm run compile
# Package as .vsix:
npm run package
```

## Architecture

```
VS Code types → InlineCompletionItemProvider.provideInlineCompletionItems()
                    ↓
    Gathers prefix (50 lines) + suffix (20 lines) + languageId
                    ↓
    fetch() POST to http://127.0.0.1:8284/v1/agents/:id/complete
    SSE stream with AbortController wired to CancellationToken
                    ↓
    Accumulates stream_delta tokens → returns InlineCompletionItem
                    ↓
    VS Code renders ghost text natively, handles Tab accept
```

No DB writes. No conversation history. No consolidation triggers. The `/v1/complete` endpoint is fully stateless.
