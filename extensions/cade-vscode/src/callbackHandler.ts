/**
 * CallbackHandler — receives a `CallbackOp` from the server and executes
 * the corresponding VS Code editor operation.
 *
 * Returns a `CallbackResult` (`{ ok: null }` or `{ err: string }`) that
 * the caller sends back as a `CallbackResponse`.
 *
 * Supported operations:
 *
 * | `op`            | VS Code API                                      |
 * | --------------- | ------------------------------------------------ |
 * | `apply_edit`    | `workspace.applyEdit(WorkspaceEdit)`             |
 * | `reveal_file`   | `workspace.openTextDocument` + `showTextDocument`|
 * | `set_selection` | `window.showTextDocument` + set `selection`       |
 * | `save`          | `TextDocument.save()` or `workspace.saveAll()`   |
 * | `run_task`      | `tasks.fetchTasks` + `tasks.executeTask`         |
 * | `run_terminal`  | `window.createTerminal` + `sendText`             |
 * | `debug_control` | `debug.startDebugging` / `debug.stopDebugging`   |
 */

import * as vscode from "vscode";
import { CallbackOp, CallbackResult, TextEdit } from "./protocol";

export class CallbackHandler {
  constructor(private readonly output: vscode.OutputChannel) {}

  async handle(op: CallbackOp): Promise<CallbackResult> {
    try {
      await this.dispatch(op);
      return { ok: null };
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      this.output.appendLine(`CADE: callback '${op.op}' failed — ${msg}`);
      return { err: msg };
    }
  }

  private async dispatch(op: CallbackOp): Promise<void> {
    switch (op.op) {
      case "apply_edit":
        return this.applyEdit(op.path, op.text_edits);
      case "reveal_file":
        return this.revealFile(op.path);
      case "set_selection":
        return this.setSelection(op.path, op.range);
      case "save":
        return this.save(op.path);
      case "run_task":
        return this.runTask(op.name);
      case "run_terminal":
        return this.runTerminal(op.command);
      case "debug_control":
        return op.action === "start"
          ? this.startDebug(op.config)
          : this.stopDebug();
    }
  }

  // ── Operation implementations ─────────────────────────────────────────────

  private async applyEdit(
    filePath: string,
    textEdits: TextEdit[],
  ): Promise<void> {
    const uri = vscode.Uri.file(filePath);
    const edit = new vscode.WorkspaceEdit();
    for (const te of textEdits) {
      const start = new vscode.Position(te.range.start.line, te.range.start.character);
      const end   = new vscode.Position(te.range.end.line,   te.range.end.character);
      edit.replace(uri, new vscode.Range(start, end), te.new_text);
    }
    const ok = await vscode.workspace.applyEdit(edit);
    if (!ok) throw new Error(`workspace.applyEdit rejected for ${filePath}`);
  }

  private async revealFile(filePath: string): Promise<void> {
    const uri = vscode.Uri.file(filePath);
    const doc = await vscode.workspace.openTextDocument(uri);
    await vscode.window.showTextDocument(doc);
  }

  private async setSelection(
    filePath: string,
    range: { start: { line: number; character: number }; end: { line: number; character: number } },
  ): Promise<void> {
    const uri = vscode.Uri.file(filePath);
    const doc = await vscode.workspace.openTextDocument(uri);
    const editor = await vscode.window.showTextDocument(doc);
    const start = new vscode.Position(range.start.line, range.start.character);
    const end   = new vscode.Position(range.end.line,   range.end.character);
    editor.selection = new vscode.Selection(start, end);
  }

  private async save(filePath: string | null): Promise<void> {
    if (filePath === null) {
      await vscode.workspace.saveAll(false);
      return;
    }
    const uri = vscode.Uri.file(filePath);
    // Find the open document and save it.
    const doc = vscode.workspace.textDocuments.find(
      (d) => d.uri.fsPath === uri.fsPath,
    );
    if (!doc) throw new Error(`${filePath} is not open`);
    const saved = await (doc as vscode.TextDocument).save();
    if (!saved) throw new Error(`save failed for ${filePath}`);
  }

  private async runTask(name: string): Promise<void> {
    const all = await vscode.tasks.fetchTasks();
    const task = all.find((t) => t.name === name);
    if (!task) throw new Error(`task '${name}' not found`);
    await vscode.tasks.executeTask(task);
  }

  private runTerminal(command: string): Promise<void> {
    const terminal = vscode.window.createTerminal("CADE");
    terminal.show();
    terminal.sendText(command);
    return Promise.resolve();
  }

  private async startDebug(config: string): Promise<void> {
    const folder = vscode.workspace.workspaceFolders?.[0];
    const ok = await vscode.debug.startDebugging(folder, config);
    if (!ok) throw new Error(`startDebugging rejected config '${config}'`);
  }

  private async stopDebug(): Promise<void> {
    await vscode.debug.stopDebugging();
  }
}
