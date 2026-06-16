/**
 * Tests for StatePublisher / buildSnapshot.
 *
 * VS Code APIs are stubbed via src/__mocks__/vscode.ts. We configure
 * the stubs before each test to control what `buildSnapshot()` sees.
 */

import * as vscode from "vscode";
import { buildSnapshot, StatePublisher } from "./statePublisher";
import { CadeConnection } from "./connection";

// ── Helpers ───────────────────────────────────────────────────────────────────

function makeConn(): jest.Mocked<Pick<CadeConnection, "sendStateUpdate">> {
  return { sendStateUpdate: jest.fn() };
}

function makeOutput() {
  return { appendLine: jest.fn(), show: jest.fn(), dispose: jest.fn() } as unknown as vscode.OutputChannel;
}

// Helper to build a minimal mock TextDocument.
function mockDoc(
  fsPath: string,
  text: string,
  lang = "typescript",
  version = 1,
  dirty = false,
) {
  return {
    uri: { scheme: "file", fsPath, toString: () => `file://${fsPath}` },
    getText: (range?: unknown) => (range ? "" : text),
    languageId: lang,
    version,
    isDirty: dirty,
  };
}

// ── buildSnapshot ─────────────────────────────────────────────────────────────

describe("buildSnapshot", () => {
  beforeEach(() => {
    // Reset mocks to clean state.
    (vscode.workspace as unknown as Record<string, unknown>).textDocuments = [];
    (vscode.workspace as unknown as Record<string, unknown>).workspaceFolders = undefined;
    (vscode.window as unknown as Record<string, unknown>).activeTextEditor = undefined;
    (vscode.languages.getDiagnostics as jest.Mock).mockReturnValue([]);
  });

  it("returns empty snapshot when no editor is open", () => {
    const snap = buildSnapshot();
    expect(snap.open_files).toEqual([]);
    expect(snap.active_file).toBeNull();
    expect(snap.selection).toBeNull();
    expect(snap.diagnostics).toEqual([]);
    expect(snap.workspace_folders).toEqual([]);
    expect(snap.visible_range).toBeNull();
  });

  it("lists open text documents with file scheme", () => {
    (vscode.workspace as unknown as Record<string, unknown>).textDocuments = [
      mockDoc("/a.ts", "const x = 1;", "typescript", 2, true),
      mockDoc("/b.rs", "fn main() {}", "rust", 1, false),
      // non-file scheme — should be excluded
      { uri: { scheme: "output", fsPath: "", toString: () => "output:" }, getText: () => "", languageId: "log", version: 1, isDirty: false },
    ];

    const snap = buildSnapshot();
    expect(snap.open_files).toHaveLength(2);
    expect(snap.open_files[0]).toMatchObject({
      path: "/a.ts",
      text: "const x = 1;",
      language_id: "typescript",
      version: 2,
      is_dirty: true,
    });
    expect(snap.open_files[1].path).toBe("/b.rs");
  });

  it("captures active_file from activeTextEditor", () => {
    (vscode.window as unknown as Record<string, unknown>).activeTextEditor = {
      document: mockDoc("/active.ts", ""),
      selection: { start: { line: 0, character: 0 }, end: { line: 0, character: 0 } },
      visibleRanges: [],
    };

    const snap = buildSnapshot();
    expect(snap.active_file).toBe("/active.ts");
  });

  it("captures selection text from activeTextEditor", () => {
    const doc = {
      uri: { scheme: "file", fsPath: "/sel.ts", toString: () => "file:///sel.ts" },
      getText: (range?: unknown) => (range ? "hello" : ""),
      languageId: "typescript",
      version: 1,
      isDirty: false,
    };
    (vscode.window as unknown as Record<string, unknown>).activeTextEditor = {
      document: doc,
      selection: { start: { line: 1, character: 0 }, end: { line: 1, character: 5 } },
      visibleRanges: [{ start: { line: 0 }, end: { line: 30 } }],
    };

    const snap = buildSnapshot();
    expect(snap.selection).toMatchObject({
      path: "/sel.ts",
      range: { start: { line: 1, character: 0 }, end: { line: 1, character: 5 } },
      text: "hello",
    });
    expect(snap.visible_range).toEqual([0, 30]);
  });

  it("maps diagnostics with correct severity strings", () => {
    (vscode.languages.getDiagnostics as jest.Mock).mockReturnValue([
      [
        { scheme: "file", fsPath: "/d.ts", toString: () => "file:///d.ts" },
        [
          {
            range: { start: { line: 0, character: 0 }, end: { line: 0, character: 4 } },
            severity: vscode.DiagnosticSeverity.Error,
            message: "type error",
            source: "ts",
            code: 2304,
          },
          {
            range: { start: { line: 1, character: 0 }, end: { line: 1, character: 4 } },
            severity: vscode.DiagnosticSeverity.Warning,
            message: "unused var",
            source: "ts",
            code: undefined,
          },
        ],
      ],
    ]);

    const snap = buildSnapshot();
    expect(snap.diagnostics).toHaveLength(2);
    expect(snap.diagnostics[0].severity).toBe("error");
    expect(snap.diagnostics[0].code).toBe("2304");
    expect(snap.diagnostics[1].severity).toBe("warning");
    expect(snap.diagnostics[1].code).toBeNull();
  });

  it("includes workspace folders", () => {
    (vscode.workspace as unknown as Record<string, unknown>).workspaceFolders = [
      { uri: { fsPath: "/repo" }, name: "repo" },
    ];
    const snap = buildSnapshot();
    expect(snap.workspace_folders).toEqual([{ path: "/repo", name: "repo" }]);
  });

  it("excludes non-file scheme diagnostics", () => {
    (vscode.languages.getDiagnostics as jest.Mock).mockReturnValue([
      [
        { scheme: "untitled", fsPath: "", toString: () => "untitled:" },
        [{ range: { start: { line: 0, character: 0 }, end: { line: 0, character: 0 } }, severity: 0, message: "x", source: null, code: null }],
      ],
    ]);
    const snap = buildSnapshot();
    expect(snap.diagnostics).toHaveLength(0);
  });
});

// ── StatePublisher ────────────────────────────────────────────────────────────

describe("StatePublisher", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    (vscode.workspace as unknown as Record<string, unknown>).textDocuments = [];
    (vscode.languages.getDiagnostics as jest.Mock).mockReturnValue([]);
  });

  it("calls sendStateUpdate on construction (initial snapshot)", async () => {
    const conn = makeConn();
    const pub = new StatePublisher(
      conn as unknown as CadeConnection,
      makeOutput(),
    );
    // Wait for debounce.
    await new Promise((r) => setTimeout(r, 100));
    expect(conn.sendStateUpdate).toHaveBeenCalledTimes(1);
    pub.dispose();
  });

  it("calls sendStateUpdate when activeTextEditor changes", async () => {
    const conn = makeConn();
    const pub = new StatePublisher(
      conn as unknown as CadeConnection,
      makeOutput(),
    );
    await new Promise((r) => setTimeout(r, 100)); // consume initial

    // Simulate VS Code firing the event.
    const handler = (vscode.window.onDidChangeActiveTextEditor as jest.Mock).mock.calls[0][0] as () => void;
    handler();
    await new Promise((r) => setTimeout(r, 100));

    expect(conn.sendStateUpdate).toHaveBeenCalledTimes(2);
    pub.dispose();
  });

  it("debounces rapid events into one publish", async () => {
    const conn = makeConn();
    const pub = new StatePublisher(
      conn as unknown as CadeConnection,
      makeOutput(),
    );
    await new Promise((r) => setTimeout(r, 100)); // initial

    const docHandler = (vscode.workspace.onDidChangeTextDocument as jest.Mock).mock.calls[0][0] as () => void;
    // Fire 5 rapid events.
    for (let i = 0; i < 5; i++) docHandler();
    await new Promise((r) => setTimeout(r, 100));

    // Only 2 total publishes: the initial + one debounced batch.
    expect(conn.sendStateUpdate).toHaveBeenCalledTimes(2);
    pub.dispose();
  });

  it("dispose stops further publishes", async () => {
    const conn = makeConn();
    const pub = new StatePublisher(
      conn as unknown as CadeConnection,
      makeOutput(),
    );
    pub.dispose();
    await new Promise((r) => setTimeout(r, 100));
    // Initial debounce was cancelled by dispose.
    expect(conn.sendStateUpdate).toHaveBeenCalledTimes(0);
  });
});
