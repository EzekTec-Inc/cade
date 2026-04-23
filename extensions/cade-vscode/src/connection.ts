/**
 * CadeConnection — manages the TCP connection to `cade-ide-mcp`.
 *
 * ## Lifecycle
 *
 * 1. `connect()` — reads `~/.cade/ide/<pid-of-cade-ide-mcp>.json`,
 *    opens a TCP socket to the advertised address, exchanges
 *    `Hello` / `HelloAck`, then stays connected.
 * 2. Incoming `CallbackRequest` frames are dispatched to the handler
 *    registered via `onCallbackRequest()`.
 * 3. `sendStateUpdate()` encodes a `StateSnapshot` and writes it to
 *    the socket.
 * 4. `sendResponse()` encodes a `CallbackResponse` and writes it.
 * 5. `dispose()` destroys the socket and cancels reconnection.
 *
 * The connection performs a single reconnect attempt after a 3-second
 * delay when the socket closes unexpectedly.
 */

import * as fs from "fs";
import * as net from "net";
import * as os from "os";
import * as path from "path";
import * as readline from "readline";
import * as vscode from "vscode";

import {
  AdapterMessage,
  CallbackOp,
  CallbackResult,
  ServerMessage,
  StateSnapshot,
  decodeServerMessage,
  encodeAdapterMessage,
} from "./protocol";

const PROTOCOL_VERSION = 1;
const RECONNECT_DELAY_MS = 3_000;

/** Shape of the discovery JSON written by `cade-ide-mcp`. */
interface DiscoveryInfo {
  pid: number;
  addr: string; // "127.0.0.1:<port>"
}

export type CallbackRequestHandler = (
  id: number,
  op: CallbackOp,
) => void;

export class CadeConnection {
  private socket: net.Socket | undefined;
  private rl: readline.Interface | undefined;
  private disposed = false;
  private reconnectTimer: ReturnType<typeof setTimeout> | undefined;
  private _onCallbackRequest: CallbackRequestHandler | undefined;

  constructor(private readonly output: vscode.OutputChannel) {}

  /** Register the handler called when a `CallbackRequest` arrives. */
  onCallbackRequest(handler: CallbackRequestHandler): void {
    this._onCallbackRequest = handler;
  }

  /** Find and connect to the running `cade-ide-mcp` process. */
  async connect(): Promise<void> {
    if (this.disposed) return;
    this.clearSocket();

    const info = this.readDiscoveryFile();
    if (!info) {
      this.output.appendLine(
        "CADE: cade-ide-mcp not running (no discovery file found). " +
          `Retrying in ${RECONNECT_DELAY_MS / 1000}s…`,
      );
      this.scheduleReconnect();
      return;
    }

    const [host, portStr] = info.addr.split(":");
    const port = parseInt(portStr, 10);

    return new Promise<void>((resolve) => {
      const sock = new net.Socket();
      this.socket = sock;

      sock.once("connect", () => {
        this.output.appendLine(`CADE: connected to cade-ide-mcp at ${info.addr}`);
        this.setupReader(sock);
        this.sendHello(sock);
        resolve();
      });

      sock.once("error", (err) => {
        this.output.appendLine(`CADE: connection error — ${err.message}`);
        this.scheduleReconnect();
        resolve();
      });

      sock.once("close", () => {
        if (!this.disposed) {
          this.output.appendLine("CADE: disconnected from cade-ide-mcp. Reconnecting…");
          this.scheduleReconnect();
        }
      });

      sock.connect(port, host);
    });
  }

  /** Send a full state snapshot to the server. */
  sendStateUpdate(snapshot: StateSnapshot): void {
    const msg: AdapterMessage = { type: "state_update", ...snapshot };
    this.write(encodeAdapterMessage(msg));
  }

  /** Send a `CallbackResponse` for the given request id. */
  sendResponse(id: number, result: CallbackResult): void {
    const msg: AdapterMessage = { type: "callback_response", id, result };
    this.write(encodeAdapterMessage(msg));
  }

  dispose(): void {
    this.disposed = true;
    this.clearReconnectTimer();
    this.clearSocket();
  }

  // ── private ──────────────────────────────────────────────────────────────

  private sendHello(sock: net.Socket): void {
    const msg: AdapterMessage = {
      type: "hello",
      label: `vscode-${vscode.env.appName}`,
      protocol_version: PROTOCOL_VERSION,
    };
    sock.write(encodeAdapterMessage(msg));
  }

  private setupReader(sock: net.Socket): void {
    this.rl?.close();
    const rl = readline.createInterface({ input: sock, crlfDelay: Infinity });
    this.rl = rl;

    rl.on("line", (line) => {
      if (!line.trim()) return;
      let msg: ServerMessage;
      try {
        msg = decodeServerMessage(line);
      } catch (e) {
        this.output.appendLine(`CADE: malformed frame — ${String(e)}`);
        return;
      }
      this.handleServerMessage(msg);
    });
  }

  private handleServerMessage(msg: ServerMessage): void {
    if (msg.type === "hello_ack") {
      this.output.appendLine(
        `CADE: HelloAck received (protocol v${msg.protocol_version}). Adapter ready.`,
      );
      return;
    }
    if (msg.type === "callback_request") {
      const { id, type: _t, ...rest } = msg as ServerMessage & {
        type: "callback_request";
        id: number;
      };
      const op = rest as unknown as CallbackOp;
      this._onCallbackRequest?.(id, op);
    }
  }

  private write(data: string): void {
    if (this.socket && !this.socket.destroyed) {
      this.socket.write(data);
    }
  }

  private clearSocket(): void {
    this.rl?.close();
    this.rl = undefined;
    this.socket?.destroy();
    this.socket = undefined;
  }

  private scheduleReconnect(): void {
    if (this.disposed) return;
    this.clearReconnectTimer();
    this.reconnectTimer = setTimeout(() => {
      void this.connect();
    }, RECONNECT_DELAY_MS);
  }

  private clearReconnectTimer(): void {
    if (this.reconnectTimer !== undefined) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = undefined;
    }
  }

  private readDiscoveryFile(): DiscoveryInfo | undefined {
    const dir = path.join(os.homedir(), ".cade", "ide");
    let entries: fs.Dirent[];
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return undefined;
    }

    // Pick the most-recently-modified discovery file — the running instance.
    const jsonFiles = entries
      .filter((e) => e.isFile() && e.name.endsWith(".json"))
      .map((e) => {
        const fullPath = path.join(dir, e.name);
        try {
          const stat = fs.statSync(fullPath);
          return { fullPath, mtimeMs: stat.mtimeMs };
        } catch {
          return null;
        }
      })
      .filter((x): x is { fullPath: string; mtimeMs: number } => x !== null)
      .sort((a, b) => b.mtimeMs - a.mtimeMs);

    if (jsonFiles.length === 0) return undefined;

    try {
      const raw = fs.readFileSync(jsonFiles[0].fullPath, "utf8");
      return JSON.parse(raw) as DiscoveryInfo;
    } catch {
      return undefined;
    }
  }
}
