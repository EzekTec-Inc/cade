/**
 * StatePublisher — translates VS Code editor events into `StateUpdate`
 * messages sent to `cade-ide-mcp` via `CadeConnection`.
 *
 * Subscribes to:
 * - `onDidChangeActiveTextEditor`   — active file changed
 * - `onDidChangeTextEditorSelection`— selection changed
 * - `onDidChangeTextDocument`       — buffer text / version changed
 * - `onDidChangeDiagnostics`        — diagnostics updated
 *
 * On each event, a full snapshot of the current editor state is built
 * and sent as a `StateUpdate`. Snapshots are debounced by 50 ms so that
 * rapid keystrokes do not flood the connection.
 */

import * as vscode from "vscode";
import { CadeConnection } from "./connection";
import {
  Diagnostic,
  OpenFile,
  StateSnapshot,
  WorkspaceFolder,
} from "./protocol";

const DEBOUNCE_MS = 50;

// Map VS Code DiagnosticSeverity → protocol severity string.
function mapSeverity(
  s: vscode.DiagnosticSeverity,
): Diagnostic["severity"] {
  switch (s) {
    case vscode.DiagnosticSeverity.Error:       return "error";
    case vscode.DiagnosticSeverity.Warning:     return "warning";
    case vscode.DiagnosticSeverity.Information: return "info";
    default:                                    return "hint";
  }
}

function buildSnapshot(): StateSnapshot {
  // Open files.
  const openFiles: OpenFile[] = vscode.workspace.textDocuments
    .filter((d) => d.uri.scheme === "file")
    .map((d) => ({
      path: d.uri.fsPath,
      text: d.getText(),
      language_id: d.languageId,
      version: d.version,
      is_dirty: d.isDirty,
    }));

  // Active file.
  const active = vscode.window.activeTextEditor;
  const activeFile = active?.document.uri.scheme === "file"
    ? active.document.uri.fsPath
    : null;

  // Selection.
  let selection: StateSnapshot["selection"] = null;
  if (active && active.document.uri.scheme === "file") {
    const sel = active.selection;
    selection = {
      path: active.document.uri.fsPath,
      range: {
        start: { line: sel.start.line, character: sel.start.character },
        end:   { line: sel.end.line,   character: sel.end.character   },
      },
      text: active.document.getText(sel),
    };
  }

  // Diagnostics.
  const diagnostics: Diagnostic[] = vscode.languages
    .getDiagnostics()
    .flatMap(([uri, diags]) =>
      uri.scheme === "file"
        ? diags.map((d) => ({
            path: uri.fsPath,
            range: {
              start: { line: d.range.start.line, character: d.range.start.character },
              end:   { line: d.range.end.line,   character: d.range.end.character   },
            },
            severity: mapSeverity(d.severity),
            message: d.message,
            source: d.source ?? null,
            code: d.code != null ? String(d.code) : null,
          }))
        : [],
    );

  // Workspace folders.
  const workspaceFolders: WorkspaceFolder[] = (
    vscode.workspace.workspaceFolders ?? []
  ).map((f) => ({
    path: f.uri.fsPath,
    name: f.name,
  }));

  // Visible range.
  let visibleRange: [number, number] | null = null;
  if (active && active.visibleRanges.length > 0) {
    const r = active.visibleRanges[0];
    visibleRange = [r.start.line, r.end.line];
  }

  return {
    open_files: openFiles,
    active_file: activeFile,
    selection,
    diagnostics,
    workspace_folders: workspaceFolders,
    visible_range: visibleRange,
  };
}

export class StatePublisher {
  private readonly disposables: vscode.Disposable[] = [];
  private debounceTimer: ReturnType<typeof setTimeout> | undefined;

  constructor(
    private readonly conn: CadeConnection,
    private readonly output: vscode.OutputChannel,
  ) {
    this.disposables.push(
      vscode.window.onDidChangeActiveTextEditor(() => this.schedulePublish()),
      vscode.window.onDidChangeTextEditorSelection(() => this.schedulePublish()),
      vscode.workspace.onDidChangeTextDocument(() => this.schedulePublish()),
      vscode.languages.onDidChangeDiagnostics(() => this.schedulePublish()),
    );
    // Publish initial snapshot.
    this.schedulePublish();
  }

  dispose(): void {
    if (this.debounceTimer !== undefined) {
      clearTimeout(this.debounceTimer);
    }
    for (const d of this.disposables) d.dispose();
  }

  private schedulePublish(): void {
    if (this.debounceTimer !== undefined) {
      clearTimeout(this.debounceTimer);
    }
    this.debounceTimer = setTimeout(() => {
      this.debounceTimer = undefined;
      try {
        const snapshot = buildSnapshot();
        this.conn.sendStateUpdate(snapshot);
      } catch (e) {
        this.output.appendLine(`CADE: StatePublisher error — ${String(e)}`);
      }
    }, DEBOUNCE_MS);
  }
}

// Export for tests.
export { buildSnapshot };
