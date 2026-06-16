"use strict";
var __create = Object.create;
var __defProp = Object.defineProperty;
var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
var __getOwnPropNames = Object.getOwnPropertyNames;
var __getProtoOf = Object.getPrototypeOf;
var __hasOwnProp = Object.prototype.hasOwnProperty;
var __export = (target, all) => {
  for (var name in all)
    __defProp(target, name, { get: all[name], enumerable: true });
};
var __copyProps = (to, from, except, desc) => {
  if (from && typeof from === "object" || typeof from === "function") {
    for (let key of __getOwnPropNames(from))
      if (!__hasOwnProp.call(to, key) && key !== except)
        __defProp(to, key, { get: () => from[key], enumerable: !(desc = __getOwnPropDesc(from, key)) || desc.enumerable });
  }
  return to;
};
var __toESM = (mod, isNodeMode, target) => (target = mod != null ? __create(__getProtoOf(mod)) : {}, __copyProps(
  // If the importer is in node compatibility mode or this is not an ESM
  // file that has been converted to a CommonJS file using a Babel-
  // compatible transform (i.e. "__esModule" has not been set), then set
  // "default" to the CommonJS "module.exports" for node compatibility.
  isNodeMode || !mod || !mod.__esModule ? __defProp(target, "default", { value: mod, enumerable: true }) : target,
  mod
));
var __toCommonJS = (mod) => __copyProps(__defProp({}, "__esModule", { value: true }), mod);

// src/extension.ts
var extension_exports = {};
__export(extension_exports, {
  activate: () => activate,
  deactivate: () => deactivate
});
module.exports = __toCommonJS(extension_exports);
var vscode4 = __toESM(require("vscode"));

// src/connection.ts
var fs = __toESM(require("fs"));
var net = __toESM(require("net"));
var os = __toESM(require("os"));
var path = __toESM(require("path"));
var readline = __toESM(require("readline"));
var vscode = __toESM(require("vscode"));

// src/protocol.ts
function encodeAdapterMessage(msg) {
  return JSON.stringify(msg) + "\n";
}
function decodeServerMessage(line) {
  const obj = JSON.parse(line.trim());
  if (typeof obj["type"] !== "string") {
    throw new Error(`ServerMessage missing 'type' field: ${line}`);
  }
  return obj;
}

// src/connection.ts
var PROTOCOL_VERSION = 1;
var RECONNECT_DELAY_MS = 3e3;
var CadeConnection = class {
  constructor(output) {
    this.output = output;
  }
  socket;
  rl;
  disposed = false;
  reconnectTimer;
  _onCallbackRequest;
  /** Register the handler called when a `CallbackRequest` arrives. */
  onCallbackRequest(handler) {
    this._onCallbackRequest = handler;
  }
  /** Find and connect to the running `cade-ide-mcp` process. */
  async connect() {
    if (this.disposed) return;
    this.clearSocket();
    const info = this.readDiscoveryFile();
    if (!info) {
      this.output.appendLine(
        `CADE: cade-ide-mcp not running (no discovery file found). Retrying in ${RECONNECT_DELAY_MS / 1e3}s\u2026`
      );
      this.scheduleReconnect();
      return;
    }
    const [host, portStr] = info.addr.split(":");
    const port = parseInt(portStr, 10);
    return new Promise((resolve) => {
      const sock = new net.Socket();
      this.socket = sock;
      sock.once("connect", () => {
        this.output.appendLine(`CADE: connected to cade-ide-mcp at ${info.addr}`);
        this.setupReader(sock);
        this.sendHello(sock);
        resolve();
      });
      sock.once("error", (err) => {
        this.output.appendLine(`CADE: connection error \u2014 ${err.message}`);
        this.scheduleReconnect();
        resolve();
      });
      sock.once("close", () => {
        if (!this.disposed) {
          this.output.appendLine("CADE: disconnected from cade-ide-mcp. Reconnecting\u2026");
          this.scheduleReconnect();
        }
      });
      sock.connect(port, host);
    });
  }
  /** Send a full state snapshot to the server. */
  sendStateUpdate(snapshot) {
    const msg = { type: "state_update", ...snapshot };
    this.write(encodeAdapterMessage(msg));
  }
  /** Send a `CallbackResponse` for the given request id. */
  sendResponse(id, result) {
    const msg = { type: "callback_response", id, result };
    this.write(encodeAdapterMessage(msg));
  }
  dispose() {
    this.disposed = true;
    this.clearReconnectTimer();
    this.clearSocket();
  }
  // ── private ──────────────────────────────────────────────────────────────
  sendHello(sock) {
    const msg = {
      type: "hello",
      label: `vscode-${vscode.env.appName}`,
      protocol_version: PROTOCOL_VERSION
    };
    sock.write(encodeAdapterMessage(msg));
  }
  setupReader(sock) {
    this.rl?.close();
    const rl = readline.createInterface({ input: sock, crlfDelay: Infinity });
    this.rl = rl;
    rl.on("line", (line) => {
      if (!line.trim()) return;
      let msg;
      try {
        msg = decodeServerMessage(line);
      } catch (e) {
        this.output.appendLine(`CADE: malformed frame \u2014 ${String(e)}`);
        return;
      }
      this.handleServerMessage(msg);
    });
  }
  handleServerMessage(msg) {
    if (msg.type === "hello_ack") {
      this.output.appendLine(
        `CADE: HelloAck received (protocol v${msg.protocol_version}). Adapter ready.`
      );
      return;
    }
    if (msg.type === "callback_request") {
      const { id, type: _t, ...rest } = msg;
      const op = rest;
      this._onCallbackRequest?.(id, op);
    }
  }
  write(data) {
    if (this.socket && !this.socket.destroyed) {
      this.socket.write(data);
    }
  }
  clearSocket() {
    this.rl?.close();
    this.rl = void 0;
    this.socket?.destroy();
    this.socket = void 0;
  }
  scheduleReconnect() {
    if (this.disposed) return;
    this.clearReconnectTimer();
    this.reconnectTimer = setTimeout(() => {
      void this.connect();
    }, RECONNECT_DELAY_MS);
  }
  clearReconnectTimer() {
    if (this.reconnectTimer !== void 0) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = void 0;
    }
  }
  /** Check if a process with `pid` is alive. */
  isProcessAlive(pid) {
    try {
      process.kill(pid, 0);
      return true;
    } catch {
      return false;
    }
  }
  readDiscoveryFile() {
    const dir = path.join(os.homedir(), ".cade", "ide");
    let entries;
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return void 0;
    }
    const jsonFiles = entries.filter((e) => e.isFile() && e.name.endsWith(".json")).map((e) => {
      const fullPath = path.join(dir, e.name);
      try {
        const stat = fs.statSync(fullPath);
        return { fullPath, name: e.name, mtimeMs: stat.mtimeMs };
      } catch {
        return null;
      }
    }).filter((x) => x !== null).sort((a, b) => b.mtimeMs - a.mtimeMs);
    for (const entry of jsonFiles) {
      const pid = parseInt(path.basename(entry.name, ".json"), 10);
      if (!Number.isNaN(pid) && !this.isProcessAlive(pid)) {
        try {
          fs.unlinkSync(entry.fullPath);
          this.output.appendLine(
            `CADE: removed stale discovery file for dead pid ${pid}`
          );
        } catch {
        }
        continue;
      }
      try {
        const raw = fs.readFileSync(entry.fullPath, "utf8");
        return JSON.parse(raw);
      } catch {
        continue;
      }
    }
    return void 0;
  }
};

// src/statePublisher.ts
var vscode2 = __toESM(require("vscode"));
var DEBOUNCE_MS = 50;
function mapSeverity(s) {
  switch (s) {
    case vscode2.DiagnosticSeverity.Error:
      return "error";
    case vscode2.DiagnosticSeverity.Warning:
      return "warning";
    case vscode2.DiagnosticSeverity.Information:
      return "info";
    default:
      return "hint";
  }
}
function buildSnapshot() {
  const openFiles = vscode2.workspace.textDocuments.filter((d) => d.uri.scheme === "file").map((d) => ({
    path: d.uri.fsPath,
    text: d.getText(),
    language_id: d.languageId,
    version: d.version,
    is_dirty: d.isDirty
  }));
  const active = vscode2.window.activeTextEditor;
  const activeFile = active?.document.uri.scheme === "file" ? active.document.uri.fsPath : null;
  let selection = null;
  if (active && active.document.uri.scheme === "file") {
    const sel = active.selection;
    selection = {
      path: active.document.uri.fsPath,
      range: {
        start: { line: sel.start.line, character: sel.start.character },
        end: { line: sel.end.line, character: sel.end.character }
      },
      text: active.document.getText(sel)
    };
  }
  const diagnostics = vscode2.languages.getDiagnostics().flatMap(
    ([uri, diags]) => uri.scheme === "file" ? diags.map((d) => ({
      path: uri.fsPath,
      range: {
        start: { line: d.range.start.line, character: d.range.start.character },
        end: { line: d.range.end.line, character: d.range.end.character }
      },
      severity: mapSeverity(d.severity),
      message: d.message,
      source: d.source ?? null,
      code: d.code != null ? String(d.code) : null
    })) : []
  );
  const workspaceFolders = (vscode2.workspace.workspaceFolders ?? []).map((f) => ({
    path: f.uri.fsPath,
    name: f.name
  }));
  let visibleRange = null;
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
    visible_range: visibleRange
  };
}
var StatePublisher = class {
  constructor(conn, output) {
    this.conn = conn;
    this.output = output;
    this.disposables.push(
      vscode2.window.onDidChangeActiveTextEditor(() => this.schedulePublish()),
      vscode2.window.onDidChangeTextEditorSelection(() => this.schedulePublish()),
      vscode2.workspace.onDidChangeTextDocument(() => this.schedulePublish()),
      vscode2.languages.onDidChangeDiagnostics(() => this.schedulePublish())
    );
    this.schedulePublish();
  }
  disposables = [];
  debounceTimer;
  dispose() {
    if (this.debounceTimer !== void 0) {
      clearTimeout(this.debounceTimer);
    }
    for (const d of this.disposables) d.dispose();
  }
  schedulePublish() {
    if (this.debounceTimer !== void 0) {
      clearTimeout(this.debounceTimer);
    }
    this.debounceTimer = setTimeout(() => {
      this.debounceTimer = void 0;
      try {
        const snapshot = buildSnapshot();
        this.conn.sendStateUpdate(snapshot);
      } catch (e) {
        this.output.appendLine(`CADE: StatePublisher error \u2014 ${String(e)}`);
      }
    }, DEBOUNCE_MS);
  }
};

// src/callbackHandler.ts
var vscode3 = __toESM(require("vscode"));
var CallbackHandler = class {
  constructor(output) {
    this.output = output;
  }
  async handle(op) {
    try {
      await this.dispatch(op);
      return { ok: null };
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      this.output.appendLine(`CADE: callback '${op.op}' failed \u2014 ${msg}`);
      return { err: msg };
    }
  }
  async dispatch(op) {
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
        return op.action === "start" ? this.startDebug(op.config) : this.stopDebug();
    }
  }
  // ── Operation implementations ─────────────────────────────────────────────
  async applyEdit(filePath, textEdits) {
    const uri = vscode3.Uri.file(filePath);
    const edit = new vscode3.WorkspaceEdit();
    for (const te of textEdits) {
      const start = new vscode3.Position(te.range.start.line, te.range.start.character);
      const end = new vscode3.Position(te.range.end.line, te.range.end.character);
      edit.replace(uri, new vscode3.Range(start, end), te.new_text);
    }
    const ok = await vscode3.workspace.applyEdit(edit);
    if (!ok) throw new Error(`workspace.applyEdit rejected for ${filePath}`);
  }
  async revealFile(filePath) {
    const uri = vscode3.Uri.file(filePath);
    const doc = await vscode3.workspace.openTextDocument(uri);
    await vscode3.window.showTextDocument(doc);
  }
  async setSelection(filePath, range) {
    const uri = vscode3.Uri.file(filePath);
    const doc = await vscode3.workspace.openTextDocument(uri);
    const editor = await vscode3.window.showTextDocument(doc);
    const start = new vscode3.Position(range.start.line, range.start.character);
    const end = new vscode3.Position(range.end.line, range.end.character);
    editor.selection = new vscode3.Selection(start, end);
  }
  async save(filePath) {
    if (filePath === null) {
      await vscode3.workspace.saveAll(false);
      return;
    }
    const uri = vscode3.Uri.file(filePath);
    const doc = vscode3.workspace.textDocuments.find(
      (d) => d.uri.fsPath === uri.fsPath
    );
    if (!doc) throw new Error(`${filePath} is not open`);
    const saved = await doc.save();
    if (!saved) throw new Error(`save failed for ${filePath}`);
  }
  async runTask(name) {
    const all = await vscode3.tasks.fetchTasks();
    const task = all.find((t) => t.name === name);
    if (!task) throw new Error(`task '${name}' not found`);
    await vscode3.tasks.executeTask(task);
  }
  runTerminal(command) {
    const terminal = vscode3.window.createTerminal("CADE");
    terminal.show();
    terminal.sendText(command);
    return Promise.resolve();
  }
  async startDebug(config) {
    const folder = vscode3.workspace.workspaceFolders?.[0];
    const ok = await vscode3.debug.startDebugging(folder, config);
    if (!ok) throw new Error(`startDebugging rejected config '${config}'`);
  }
  async stopDebug() {
    await vscode3.debug.stopDebugging();
  }
};

// src/extension.ts
var connection;
var statePublisher;
var callbackHandler;
function activate(context) {
  const output = vscode4.window.createOutputChannel("CADE IDE Bridge");
  output.appendLine("CADE IDE Bridge activating\u2026");
  connection = new CadeConnection(output);
  callbackHandler = new CallbackHandler(output);
  statePublisher = new StatePublisher(connection, output);
  connection.onCallbackRequest(async (id, op) => {
    const result = await callbackHandler.handle(op);
    connection.sendResponse(id, result);
  });
  void connection.connect();
  const reconnectCmd = vscode4.commands.registerCommand("cade.reconnect", () => {
    output.appendLine("Manual reconnect requested.");
    void connection.connect();
  });
  context.subscriptions.push(
    { dispose: () => connection?.dispose() },
    { dispose: () => statePublisher?.dispose() },
    { dispose: () => output.dispose() },
    reconnectCmd
  );
}
function deactivate() {
  connection?.dispose();
  statePublisher?.dispose();
}
// Annotate the CommonJS export names for ESM import in node:
0 && (module.exports = {
  activate,
  deactivate
});
//# sourceMappingURL=extension.js.map
