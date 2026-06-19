import * as vscode from "vscode";
import { BridgeConnection } from "./connection";

export class StatePublisher {
  private timer: NodeJS.Timeout | null = null;

  constructor(private readonly connection: BridgeConnection) {}

  public schedulePublish(): void {
    if (this.timer) {
      clearTimeout(this.timer);
    }
    this.timer = setTimeout(() => this.publish(), 300); // 300ms Debounce
  }

  private publish(): void {
    const activeEditor = vscode.window.activeTextEditor;
    if (!activeEditor) {
      return;
    }

    const snap = {
      type: "state_update",
      active_file: activeEditor.document.uri.fsPath,
      open_files: vscode.workspace.textDocuments
        .filter((doc) => doc.uri.scheme === "file")
        .map((doc) => doc.uri.fsPath),
      selection: {
        start: {
          line: activeEditor.selection.start.line,
          character: activeEditor.selection.start.character,
        },
        end: {
          line: activeEditor.selection.end.line,
          character: activeEditor.selection.end.character,
        },
      },
      diagnostics: this.gatherDiagnostics(activeEditor.document.uri),
    };

    this.connection.send(snap);
  }

  private gatherDiagnostics(uri: vscode.Uri): any[] {
    const diagnostics = vscode.languages.getDiagnostics(uri);
    return diagnostics.map((d) => ({
      range: {
        start: { line: d.range.start.line, character: d.range.start.character },
        end: { line: d.range.end.line, character: d.range.end.character },
      },
      message: d.message,
      severity: vscode.DiagnosticSeverity[d.severity],
    }));
  }
}
