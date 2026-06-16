/**
 * CADE Inline Completions — VS Code Extension
 *
 * Implements `InlineCompletionItemProvider` to stream code completions from
 * the local CADE server's stateless `/v1/agents/:id/complete` endpoint.
 *
 * VS Code handles all ghost-text rendering, Tab acceptance, and debouncing
 * natively — this extension only needs to:
 *   1. Gather prefix/suffix context from the active document
 *   2. POST to CADE's SSE endpoint
 *   3. Accumulate streamed tokens into a single completion string
 *   4. Return an InlineCompletionItem
 */

import * as vscode from "vscode";

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

/**
 * Calls the CADE `/v1/complete` endpoint and accumulates streamed text.
 * Returns the full completion string, or empty string on error/cancellation.
 */
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
    // Network error or abort — silent
    return "";
  }

  if (!response.ok || !response.body) {
    return "";
  }

  // Read the SSE stream
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

      // Process complete SSE lines
      const lines = buffer.split("\n");
      // Keep the last (potentially incomplete) line in the buffer
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
    // Abort or read error — return what we have
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

    // Skip non-file schemes (output panels, git diffs, etc.)
    if (document.uri.scheme !== "file" && document.uri.scheme !== "untitled") {
      return undefined;
    }

    // ── Build prefix (lines before cursor + current line up to cursor) ───
    const prefixStartLine = Math.max(0, position.line - cfg.linesBefore);
    const prefixRange = new vscode.Range(
      new vscode.Position(prefixStartLine, 0),
      position
    );
    const prefix = document.getText(prefixRange);

    // ── Build suffix (rest of current line + lines after cursor) ─────────
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

    // Skip if there's barely any context
    if (prefix.length < 3) {
      return undefined;
    }

    // Wire CancellationToken → AbortController
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

    // Return a single inline completion item inserted at the cursor
    const item = new vscode.InlineCompletionItem(
      completion,
      new vscode.Range(position, position)
    );

    return [item];
  }
}

// ── Extension lifecycle ──────────────────────────────────────────────────────

let statusBarItem: vscode.StatusBarItem;

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

export function deactivate() {}
