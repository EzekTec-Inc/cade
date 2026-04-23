import {
  AdapterMessage,
  CallbackResult,
  ServerMessage,
  StateSnapshot,
  decodeAdapterMessage,
  decodeServerMessage,
  encodeAdapterMessage,
} from "./protocol";

// ── Helpers ───────────────────────────────────────────────────────────────────

function noEmbeddedNewlines(s: string): void {
  // The frame must be a single JSON line — no embedded \n before the
  // trailing one appended by encodeAdapterMessage.
  const withoutTrailing = s.endsWith("\n") ? s.slice(0, -1) : s;
  expect(withoutTrailing).not.toContain("\n");
}

const emptySnapshot: StateSnapshot = {
  open_files: [],
  active_file: null,
  selection: null,
  diagnostics: [],
  workspace_folders: [],
  visible_range: null,
};

// ── encodeAdapterMessage / decodeAdapterMessage ───────────────────────────────

describe("encodeAdapterMessage", () => {
  it("hello: serialises type tag, label, protocol_version", () => {
    const msg: AdapterMessage = {
      type: "hello",
      label: "vscode-1.90.0",
      protocol_version: 1,
    };
    const line = encodeAdapterMessage(msg);
    noEmbeddedNewlines(line);
    expect(line).toContain('"type":"hello"');
    expect(line).toContain('"label":"vscode-1.90.0"');
    expect(line).toContain('"protocol_version":1');
  });

  it("hello: round-trips through decodeAdapterMessage", () => {
    const msg: AdapterMessage = {
      type: "hello",
      label: "test",
      protocol_version: 1,
    };
    expect(decodeAdapterMessage(encodeAdapterMessage(msg))).toEqual(msg);
  });

  it("state_update (empty snapshot): round-trips", () => {
    const msg: AdapterMessage = { type: "state_update", ...emptySnapshot };
    expect(decodeAdapterMessage(encodeAdapterMessage(msg))).toEqual(msg);
  });

  it("state_update (full snapshot): round-trips", () => {
    const msg: AdapterMessage = {
      type: "state_update",
      open_files: [
        {
          path: "/tmp/a.ts",
          text: "const x = 1;\n",
          language_id: "typescript",
          version: 3,
          is_dirty: true,
        },
      ],
      active_file: "/tmp/a.ts",
      selection: {
        path: "/tmp/a.ts",
        range: {
          start: { line: 0, character: 0 },
          end: { line: 0, character: 5 },
        },
        text: "const",
      },
      diagnostics: [
        {
          path: "/tmp/a.ts",
          range: {
            start: { line: 0, character: 0 },
            end: { line: 0, character: 5 },
          },
          severity: "warning",
          message: "unused variable",
          source: "ts",
          code: "6133",
        },
      ],
      workspace_folders: [{ path: "/tmp", name: "tmp" }],
      visible_range: [0, 40],
    };
    expect(decodeAdapterMessage(encodeAdapterMessage(msg))).toEqual(msg);
  });

  it("callback_response ok: round-trips", () => {
    const msg: AdapterMessage = {
      type: "callback_response",
      id: 42,
      result: { ok: null } satisfies CallbackResult,
    };
    expect(decodeAdapterMessage(encodeAdapterMessage(msg))).toEqual(msg);
  });

  it("callback_response err: round-trips", () => {
    const msg: AdapterMessage = {
      type: "callback_response",
      id: 7,
      result: { err: "file not open" } satisfies CallbackResult,
    };
    expect(decodeAdapterMessage(encodeAdapterMessage(msg))).toEqual(msg);
  });
});

// ── decodeServerMessage ───────────────────────────────────────────────────────

describe("decodeServerMessage", () => {
  it("hello_ack: deserialises correctly", () => {
    const line = JSON.stringify({ type: "hello_ack", protocol_version: 1 });
    const msg = decodeServerMessage(line);
    expect(msg).toEqual({ type: "hello_ack", protocol_version: 1 });
  });

  it("callback_request apply_edit: deserialises correctly", () => {
    const payload: ServerMessage = {
      type: "callback_request",
      id: 1,
      op: "apply_edit",
      path: "/tmp/a.ts",
      text_edits: [
        {
          range: {
            start: { line: 0, character: 0 },
            end: { line: 0, character: 0 },
          },
          new_text: "// header\n",
        },
      ],
    };
    const line = JSON.stringify(payload);
    expect(decodeServerMessage(line)).toEqual(payload);
  });

  it("callback_request reveal_file: deserialises correctly", () => {
    const payload: ServerMessage = {
      type: "callback_request",
      id: 2,
      op: "reveal_file",
      path: "/tmp/b.ts",
    };
    expect(decodeServerMessage(JSON.stringify(payload))).toEqual(payload);
  });

  it("callback_request set_selection: deserialises correctly", () => {
    const payload: ServerMessage = {
      type: "callback_request",
      id: 3,
      op: "set_selection",
      path: "/tmp/c.ts",
      range: {
        start: { line: 5, character: 2 },
        end: { line: 5, character: 10 },
      },
    };
    expect(decodeServerMessage(JSON.stringify(payload))).toEqual(payload);
  });

  it("callback_request save (single): deserialises correctly", () => {
    const payload: ServerMessage = {
      type: "callback_request",
      id: 4,
      op: "save",
      path: "/tmp/d.ts",
    };
    expect(decodeServerMessage(JSON.stringify(payload))).toEqual(payload);
  });

  it("callback_request save (all): deserialises correctly", () => {
    const payload: ServerMessage = {
      type: "callback_request",
      id: 5,
      op: "save",
      path: null,
    };
    expect(decodeServerMessage(JSON.stringify(payload))).toEqual(payload);
  });

  it("callback_request run_task: deserialises correctly", () => {
    const payload: ServerMessage = {
      type: "callback_request",
      id: 6,
      op: "run_task",
      name: "cargo-build",
    };
    expect(decodeServerMessage(JSON.stringify(payload))).toEqual(payload);
  });

  it("callback_request run_terminal: deserialises correctly", () => {
    const payload: ServerMessage = {
      type: "callback_request",
      id: 7,
      op: "run_terminal",
      command: "cargo test",
    };
    expect(decodeServerMessage(JSON.stringify(payload))).toEqual(payload);
  });

  it("callback_request debug_control start: deserialises correctly", () => {
    const payload: ServerMessage = {
      type: "callback_request",
      id: 8,
      op: "debug_control",
      action: "start",
      config: "unit-tests",
    };
    expect(decodeServerMessage(JSON.stringify(payload))).toEqual(payload);
  });

  it("callback_request debug_control stop: deserialises correctly", () => {
    const payload: ServerMessage = {
      type: "callback_request",
      id: 9,
      op: "debug_control",
      action: "stop",
    };
    expect(decodeServerMessage(JSON.stringify(payload))).toEqual(payload);
  });

  it("throws on missing type field", () => {
    expect(() => decodeServerMessage('{"id":1}')).toThrow("missing 'type'");
  });

  it("throws on invalid JSON", () => {
    expect(() => decodeServerMessage("not-json")).toThrow();
  });
});
