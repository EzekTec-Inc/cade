/**
 * Tests for CadeConnection.
 *
 * We drive a real in-process TCP server so the connection code exercises
 * actual socket I/O. The VS Code API is mocked via the __mocks__/vscode.ts
 * stub (no VS Code host required).
 */

import * as fs from "fs";
import * as net from "net";
import * as os from "os";
import * as path from "path";

import { CadeConnection } from "./connection";
import { AdapterMessage, CallbackOp, ServerMessage, decodeAdapterMessage } from "./protocol";

// ── Mock output channel ───────────────────────────────────────────────────────

function makeOutput() {
  const lines: string[] = [];
  return {
    appendLine: (s: string) => lines.push(s),
    show: jest.fn(),
    dispose: jest.fn(),
    lines,
  } as unknown as import("vscode").OutputChannel & { lines: string[] };
}

// ── TCP test server helper ────────────────────────────────────────────────────

interface TestServer {
  port: number;
  nextClient(): Promise<net.Socket>;
  close(): void;
  /** Read one newline-delimited frame from `sock`. */
  readLine(sock: net.Socket): Promise<string>;
  /** Write a `ServerMessage` line to `sock`. */
  writeLine(sock: net.Socket, msg: ServerMessage): void;
}

function startTestServer(): Promise<TestServer> {
  return new Promise((resolve) => {
    const pending: Array<(s: net.Socket) => void> = [];
    const ready: net.Socket[] = [];

    const server = net.createServer((sock) => {
      if (pending.length > 0) {
        pending.shift()!(sock);
      } else {
        ready.push(sock);
      }
    });

    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address() as net.AddressInfo;

      resolve({
        port,
        nextClient(): Promise<net.Socket> {
          return new Promise((res) => {
            if (ready.length > 0) {
              res(ready.shift()!);
            } else {
              pending.push(res);
            }
          });
        },
        close() {
          server.close();
        },
        readLine(sock: net.Socket): Promise<string> {
          return new Promise((res, rej) => {
            let buf = "";
            const onData = (chunk: Buffer) => {
              buf += chunk.toString();
              const idx = buf.indexOf("\n");
              if (idx !== -1) {
                sock.off("data", onData);
                sock.off("error", onError);
                res(buf.slice(0, idx));
              }
            };
            const onError = (e: Error) => rej(e);
            sock.on("data", onData);
            sock.once("error", onError);
          });
        },
        writeLine(sock: net.Socket, msg: ServerMessage): void {
          sock.write(JSON.stringify(msg) + "\n");
        },
      });
    });
  });
}

// ── Discovery file helpers ────────────────────────────────────────────────────

function writeDiscovery(port: number): string {
  const dir = path.join(os.homedir(), ".cade", "ide");
  fs.mkdirSync(dir, { recursive: true });
  const filePath = path.join(dir, `test-${process.pid}.json`);
  fs.writeFileSync(filePath, JSON.stringify({ pid: process.pid, addr: `127.0.0.1:${port}` }));
  return filePath;
}

function removeDiscovery(filePath: string): void {
  try {
    fs.unlinkSync(filePath);
  } catch {
    /* ignore */
  }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe("CadeConnection", () => {
  let server: TestServer;
  let discPath: string;

  beforeEach(async () => {
    server = await startTestServer();
    discPath = writeDiscovery(server.port);
  });

  afterEach(() => {
    server.close();
    removeDiscovery(discPath);
  });

  // ── Hello / HelloAck handshake ────────────────────────────────────────────

  it("sends Hello frame after connecting", async () => {
    const output = makeOutput();
    const conn = new CadeConnection(output as unknown as import("vscode").OutputChannel);

    const connectPromise = conn.connect();
    const sock = await server.nextClient();
    const line = await server.readLine(sock);

    const msg = decodeAdapterMessage(line);
    expect(msg.type).toBe("hello");
    if (msg.type === "hello") {
      expect(msg.protocol_version).toBe(1);
      expect(typeof msg.label).toBe("string");
    }

    // Send HelloAck so the reader doesn't block.
    server.writeLine(sock, { type: "hello_ack", protocol_version: 1 });

    await connectPromise;
    conn.dispose();
    sock.destroy();
  });

  it("logs HelloAck receipt to output channel", async () => {
    const output = makeOutput();
    const conn = new CadeConnection(output as unknown as import("vscode").OutputChannel);

    void conn.connect();
    const sock = await server.nextClient();
    await server.readLine(sock); // consume Hello

    server.writeLine(sock, { type: "hello_ack", protocol_version: 1 });

    // Give the readline handler time to fire.
    await new Promise((r) => setTimeout(r, 30));

    expect(output.lines.some((l) => l.includes("HelloAck"))).toBe(true);

    conn.dispose();
    sock.destroy();
  });

  // ── sendStateUpdate ───────────────────────────────────────────────────────

  it("sendStateUpdate writes a state_update frame", async () => {
    const output = makeOutput();
    const conn = new CadeConnection(output as unknown as import("vscode").OutputChannel);

    void conn.connect();
    const sock = await server.nextClient();
    await server.readLine(sock); // consume Hello
    server.writeLine(sock, { type: "hello_ack", protocol_version: 1 });
    await new Promise((r) => setTimeout(r, 20));

    conn.sendStateUpdate({
      open_files: [],
      active_file: "/tmp/a.ts",
      selection: null,
      diagnostics: [],
      workspace_folders: [],
      visible_range: null,
    });

    const line = await server.readLine(sock);
    const msg = decodeAdapterMessage(line);
    expect(msg.type).toBe("state_update");
    if (msg.type === "state_update") {
      expect(msg.active_file).toBe("/tmp/a.ts");
    }

    conn.dispose();
    sock.destroy();
  });

  // ── sendResponse ──────────────────────────────────────────────────────────

  it("sendResponse writes a callback_response frame", async () => {
    const output = makeOutput();
    const conn = new CadeConnection(output as unknown as import("vscode").OutputChannel);

    void conn.connect();
    const sock = await server.nextClient();
    await server.readLine(sock);
    server.writeLine(sock, { type: "hello_ack", protocol_version: 1 });
    await new Promise((r) => setTimeout(r, 20));

    conn.sendResponse(99, { ok: null });

    const line = await server.readLine(sock);
    const msg = decodeAdapterMessage(line);
    expect(msg.type).toBe("callback_response");
    if (msg.type === "callback_response") {
      expect(msg.id).toBe(99);
      expect(msg.result).toEqual({ ok: null });
    }

    conn.dispose();
    sock.destroy();
  });

  // ── CallbackRequest dispatch ──────────────────────────────────────────────

  it("dispatches CallbackRequest to onCallbackRequest handler", async () => {
    const output = makeOutput();
    const conn = new CadeConnection(output as unknown as import("vscode").OutputChannel);

    const received: Array<{ id: number; op: CallbackOp }> = [];
    conn.onCallbackRequest((id, op) => received.push({ id, op }));

    void conn.connect();
    const sock = await server.nextClient();
    await server.readLine(sock);
    server.writeLine(sock, { type: "hello_ack", protocol_version: 1 });
    await new Promise((r) => setTimeout(r, 20));

    // Send a CallbackRequest from the server side.
    const req: ServerMessage = {
      type: "callback_request",
      id: 55,
      op: "run_task",
      name: "cargo-build",
    };
    server.writeLine(sock, req);

    await new Promise((r) => setTimeout(r, 30));

    expect(received).toHaveLength(1);
    expect(received[0].id).toBe(55);
    expect(received[0].op).toMatchObject({ op: "run_task", name: "cargo-build" });

    conn.dispose();
    sock.destroy();
  });

  // ── No discovery file ─────────────────────────────────────────────────────

  it("does not throw when no discovery file exists", async () => {
    removeDiscovery(discPath); // remove it before connecting
    const output = makeOutput();
    const conn = new CadeConnection(output as unknown as import("vscode").OutputChannel);

    // Should complete without throwing (schedules a reconnect internally).
    await expect(conn.connect()).resolves.toBeUndefined();
    // No discovery file → logs a message about cade-ide-mcp not being found.
    expect(output.lines.length).toBeGreaterThan(0);

    conn.dispose();
  });

  // ── dispose ───────────────────────────────────────────────────────────────

  it("dispose prevents further connections", async () => {
    const output = makeOutput();
    const conn = new CadeConnection(output as unknown as import("vscode").OutputChannel);
    conn.dispose();

    // connect() should return immediately without touching the server.
    await expect(conn.connect()).resolves.toBeUndefined();
    expect(output.lines.length).toBe(0);
  });
});
