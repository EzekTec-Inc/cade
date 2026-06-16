/**
 * CADE IDE Bridge — VS Code extension entry point.
 *
 * Lifecycle:
 *   activate()   — called by VS Code when the extension starts.
 *                  Starts the CadeConnection (reads discovery file,
 *                  connects to cade-ide-mcp over TCP) and registers
 *                  VS Code event listeners.
 *   deactivate() — called on extension teardown; disposes all resources.
 */

import * as vscode from "vscode";
import { CadeConnection } from "./connection";
import { StatePublisher } from "./statePublisher";
import { CallbackHandler } from "./callbackHandler";

let connection: CadeConnection | undefined;
let statePublisher: StatePublisher | undefined;
let callbackHandler: CallbackHandler | undefined;

export function activate(context: vscode.ExtensionContext): void {
  const output = vscode.window.createOutputChannel("CADE IDE Bridge");
  output.appendLine("CADE IDE Bridge activating…");

  connection = new CadeConnection(output);
  callbackHandler = new CallbackHandler(output);
  statePublisher = new StatePublisher(connection, output);

  // Wire: incoming CallbackRequests → CallbackHandler → response back.
  connection.onCallbackRequest(async (id, op) => {
    const result = await callbackHandler!.handle(op);
    connection!.sendResponse(id, result);
  });

  // Start connecting (non-blocking; retries internally).
  void connection.connect();

  // Register the manual reconnect command.
  const reconnectCmd = vscode.commands.registerCommand("cade.reconnect", () => {
    output.appendLine("Manual reconnect requested.");
    void connection!.connect();
  });

  context.subscriptions.push(
    { dispose: () => connection?.dispose() },
    { dispose: () => statePublisher?.dispose() },
    { dispose: () => output.dispose() },
    reconnectCmd,
  );
}

export function deactivate(): void {
  connection?.dispose();
  statePublisher?.dispose();
}
