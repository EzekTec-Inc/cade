import * as vscode from "vscode";

export class CallbackHandler {
  static async handleCallback(request: any): Promise<any> {
    const { op, params } = request;

    if (op === "apply_edit") {
      const uri = vscode.Uri.file(params.path);
      const edit = new vscode.WorkspaceEdit();

      for (const editItem of params.edits) {
        const range = new vscode.Range(
          new vscode.Position(editItem.range.start.line, editItem.range.start.character),
          new vscode.Position(editItem.range.end.line, editItem.range.end.character)
        );
        edit.replace(uri, range, editItem.text);
      }

      const success = await vscode.workspace.applyEdit(edit);
      return { success };
    }

    if (op === "reveal_file") {
      const doc = await vscode.workspace.openTextDocument(params.path);
      await vscode.window.showTextDocument(doc);
      return { success: true };
    }

    if (op === "save_single") {
      const doc = await vscode.workspace.openTextDocument(params.path);
      const success = await doc.save();
      return { success };
    }

    if (op === "run_terminal") {
      const terminal =
        vscode.window.terminals.find((t) => t.name === "CADE") ||
        vscode.window.createTerminal("CADE");
      terminal.show();
      terminal.sendText(params.command);
      return { success: true };
    }

    throw new Error(`Unsupported callback operation: ${op}`);
  }
}
