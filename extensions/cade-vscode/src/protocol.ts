/**
 * TypeScript mirror of `crates/cade-ide-mcp/src/protocol.rs`.
 *
 * All messages are serialised as newline-delimited JSON (one JSON object
 * per line, no embedded newlines). Discriminated unions use the same
 * `type` tag and field names as the Rust implementation so adapter code
 * can be shared verbatim across language boundaries.
 */

// ── Shared sub-types ─────────────────────────────────────────────────────────

export interface Position {
  line: number;
  character: number;
}

export interface Range {
  start: Position;
  end: Position;
}

export interface OpenFile {
  path: string | null;
  text: string;
  language_id: string;
  version: number;
  is_dirty: boolean;
}

export interface Selection {
  path: string;
  range: Range;
  text: string;
}

export interface Diagnostic {
  path: string;
  range: Range;
  severity: "error" | "warning" | "info" | "hint";
  message: string;
  source: string | null;
  code: string | null;
}

export interface WorkspaceFolder {
  path: string;
  name: string;
}

export interface TextEdit {
  range: Range;
  new_text: string;
}

export interface ApplyEditRequest {
  path: string;
  text_edits: TextEdit[];
}

/** Full snapshot of editor state sent from adapter → server. */
export interface StateSnapshot {
  open_files: OpenFile[];
  active_file: string | null;
  selection: Selection | null;
  diagnostics: Diagnostic[];
  workspace_folders: WorkspaceFolder[];
  visible_range: [number, number] | null;
}

// ── Adapter → server ─────────────────────────────────────────────────────────

export type AdapterMessage =
  | { type: "hello"; label: string; protocol_version: number }
  | ({ type: "state_update" } & StateSnapshot)
  | { type: "callback_response"; id: number; result: CallbackResult };

export type CallbackResult = { ok: null } | { err: string };

// ── Server → adapter ─────────────────────────────────────────────────────────

export type CallbackOp =
  | { op: "apply_edit"; path: string; text_edits: TextEdit[] }
  | { op: "reveal_file"; path: string }
  | { op: "set_selection"; path: string; range: Range }
  | { op: "save"; path: string | null }
  | { op: "run_task"; name: string }
  | { op: "run_terminal"; command: string }
  | { op: "debug_control"; action: "start"; config: string }
  | { op: "debug_control"; action: "stop" };

export type ServerMessage =
  | { type: "hello_ack"; protocol_version: number }
  | ({ type: "callback_request"; id: number } & CallbackOp);

// ── Serialisation helpers ─────────────────────────────────────────────────────

/**
 * Serialise an `AdapterMessage` to a single JSON line (no embedded newlines).
 * Appends `\n` so it can be written directly to the TCP stream.
 */
export function encodeAdapterMessage(msg: AdapterMessage): string {
  return JSON.stringify(msg) + "\n";
}

/**
 * Parse one JSON line from the server into a `ServerMessage`.
 * Throws if the line is not valid JSON or does not have a `type` field.
 */
export function decodeServerMessage(line: string): ServerMessage {
  const obj = JSON.parse(line.trim()) as Record<string, unknown>;
  if (typeof obj["type"] !== "string") {
    throw new Error(`ServerMessage missing 'type' field: ${line}`);
  }
  return obj as unknown as ServerMessage;
}

/**
 * Parse one JSON line from the adapter into an `AdapterMessage`.
 * Used in tests and by the Rust server's adapter-facing read loop.
 */
export function decodeAdapterMessage(line: string): AdapterMessage {
  const obj = JSON.parse(line.trim()) as Record<string, unknown>;
  if (typeof obj["type"] !== "string") {
    throw new Error(`AdapterMessage missing 'type' field: ${line}`);
  }
  return obj as unknown as AdapterMessage;
}
