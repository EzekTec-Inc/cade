/**
 * CADE Inline Completions — VS Code Extension
 *
 * Implements `InlineCompletionItemProvider` to stream code completions from
 * the local CADE server's stateless `/v1/agents/:id/complete` endpoint.
 * Also integrates the bidirectional TCP socket editor adapter bridge to sync state.
 */

import * as vscode from "vscode";
import { BridgeConnection } from "./mcp/connection";
import { StatePublisher } from "./mcp/publisher";
import { CallbackHandler } from "./mcp/callbacks";

// ── Configuration helpers ────────────────────────────────────────────────────

interface CadeConfig {
  enabled: boolean;
  port: number;
  agentId: string;
  apiKey: string;
  linesBefore: number;
  linesAfter: number;
}

function getConfig(): CadeConfig {
  const cfg = vscode.workspace.getConfiguration("cade");
  return {
    enabled: cfg.get<boolean>("enabled", true),
    port: cfg.get<number>("serverPort", 8284),
    agentId:
      cfg.get<string>("agentId", "") || process.env.CADE_AGENT_ID || "",
    apiKey: cfg.get<string>("apiKey", "") || process.env.CADE_API_KEY || "",
    linesBefore: cfg.get<number>("linesBefore", 50),
    linesAfter: cfg.get<number>("linesAfter", 20),
  };
}

// ── SSE streaming fetch ──────────────────────────────────────────────────────

async function fetchCompletion(
  prefix: string,
  suffix: string,
  language: string,
  signal: AbortSignal
): Promise<string> {
  const cfg = getConfig();
  if (!cfg.agentId) {
    return "";
  }

  const url = `http://127.0.0.1:${cfg.port}/v1/agents/${encodeURIComponent(cfg.agentId)}/complete`;

  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    Accept: "text/event-stream",
  };
  if (cfg.apiKey) {
    headers["Authorization"] = `Bearer ${cfg.apiKey}`;
  }

  let response: Response;
  try {
    response = await fetch(url, {
      method: "POST",
      headers,
      body: JSON.stringify({ prefix, suffix, language }),
      signal,
    });
  } catch {
    return "";
  }

  if (!response.ok || !response.body) {
    return "";
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let accumulated = "";
  let buffer = "";

  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }

      buffer += decoder.decode(value, { stream: true });

      const lines = buffer.split("\n");
      buffer = lines.pop() || "";

      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed.startsWith("data: ")) {
          continue;
        }

        const payload = trimmed.slice(6);
        if (payload === "[DONE]") {
          return accumulated;
        }

        try {
          const obj = JSON.parse(payload);
          if (obj.message_type === "stream_delta" && obj.content) {
            accumulated += obj.content;
          } else if (obj.error) {
            return "";
          }
        } catch {
          // Malformed JSON line — skip
        }
      }
    }
  } catch {
    // Abort or read error
  }

  return accumulated;
}

// ── InlineCompletionItemProvider ──────────────────────────────────────────────

class CadeCompletionProvider
  implements vscode.InlineCompletionItemProvider
{
  async provideInlineCompletionItems(
    document: vscode.TextDocument,
    position: vscode.Position,
    _context: vscode.InlineCompletionContext,
    token: vscode.CancellationToken
  ): Promise<vscode.InlineCompletionItem[] | undefined> {
    const cfg = getConfig();
    if (!cfg.enabled || !cfg.agentId) {
      return undefined;
    }

    if (document.uri.scheme !== "file" && document.uri.scheme !== "untitled") {
      return undefined;
    }

    const prefixStartLine = Math.max(0, position.line - cfg.linesBefore);
    const prefixRange = new vscode.Range(
      new vscode.Position(prefixStartLine, 0),
      position
    );
    const prefix = document.getText(prefixRange);

    const suffixEndLine = Math.min(
      document.lineCount - 1,
      position.line + cfg.linesAfter
    );
    const lastLineLen = document.lineAt(suffixEndLine).text.length;
    const suffixRange = new vscode.Range(
      position,
      new vscode.Position(suffixEndLine, lastLineLen)
    );
    const suffix = document.getText(suffixRange);

    if (prefix.length < 3) {
      return undefined;
    }

    const abort = new AbortController();
    token.onCancellationRequested(() => abort.abort());

    const language = document.languageId || "text";
    const completion = await fetchCompletion(
      prefix,
      suffix,
      language,
      abort.signal
    );

    if (!completion || token.isCancellationRequested) {
      return undefined;
    }

    const item = new vscode.InlineCompletionItem(
      completion,
      new vscode.Range(position, position)
    );

    return [item];
  }
}

// ── Extension lifecycle ──────────────────────────────────────────────────────

let statusBarItem: vscode.StatusBarItem;
let connection: BridgeConnection | null = null;
let publisher: StatePublisher | null = null;

export function activate(context: vscode.ExtensionContext) {
  // Register the inline completion provider for all languages
  const provider = new CadeCompletionProvider();
  const disposable = vscode.languages.registerInlineCompletionItemProvider(
    { pattern: "**" },
    provider
  );
  context.subscriptions.push(disposable);

  // Toggle command
  const toggleCmd = vscode.commands.registerCommand(
    "cade.toggleCompletions",
    () => {
      const cfg = vscode.workspace.getConfiguration("cade");
      const current = cfg.get<boolean>("enabled", true);
      cfg.update("enabled", !current, vscode.ConfigurationTarget.Global);
      vscode.window.showInformationMessage(
        `CADE completions ${!current ? "enabled" : "disabled"}`
      );
      updateStatusBar(!current);
    }
  );
  context.subscriptions.push(toggleCmd);

  // Status bar indicator
  statusBarItem = vscode.window.createStatusBarItem(
    vscode.StatusBarAlignment.Right,
    100
  );
  statusBarItem.command = "cade.toggleCompletions";
  updateStatusBar(getConfig().enabled);
  statusBarItem.show();
  context.subscriptions.push(statusBarItem);

  // Initialize TCP bridge connection to CADE server
  connection = new BridgeConnection(
    async (msg) => {
      if (msg && msg.type === "callback_request") {
        try {
          const result = await CallbackHandler.handleCallback(msg.action);
          connection?.send({
            type: "callback_response",
            id: msg.id,
            result,
          });
        } catch (err: any) {
          connection?.send({
            type: "callback_response",
            id: msg.id,
            result: { err: err.message || "Unknown callback execution error" },
          });
        }
      }
    },
    () => {
      vscode.window.showInformationMessage("CADE Editor Bridge connected.");
    }
  );

  publisher = new StatePublisher(connection);

  // Trigger telemetry state publication on workspace/editor changes
  context.subscriptions.push(
    vscode.window.onDidChangeActiveTextEditor(() => publisher?.schedulePublish()),
    vscode.workspace.onDidChangeTextDocument(() => publisher?.schedulePublish()),
    vscode.window.onDidChangeTextEditorSelection(() => publisher?.schedulePublish()),
    vscode.languages.onDidChangeDiagnostics(() => publisher?.schedulePublish())
  );

  // Launch the TCP connection
  connection.connect();
}

function updateStatusBar(enabled: boolean) {
  if (enabled) {
    statusBarItem.text = "$(sparkle) CADE";
    statusBarItem.tooltip = "CADE completions enabled — click to toggle";
  } else {
    statusBarItem.text = "$(circle-slash) CADE";
    statusBarItem.tooltip = "CADE completions disabled — click to toggle";
  }
}

export function deactivate() {
  if (connection) {
    connection.dispose();
  }
}
