/**
 * Tests for CallbackHandler.
 *
 * VS Code APIs are stubbed via __mocks__/vscode.ts. Each test configures
 * the relevant stub and asserts that `handle(op)` calls the right API
 * and returns the correct `CallbackResult`.
 */

import * as vscode from "vscode";
import { CallbackHandler } from "./callbackHandler";
import { CallbackOp } from "./protocol";

function makeOutput() {
  return {
    appendLine: jest.fn(),
    show: jest.fn(),
    dispose: jest.fn(),
  } as unknown as vscode.OutputChannel;
}

function makeHandler() {
  return new CallbackHandler(makeOutput());
}

// ── apply_edit ────────────────────────────────────────────────────────────────

describe("apply_edit", () => {
  it("calls workspace.applyEdit and returns ok", async () => {
    (vscode.workspace.applyEdit as jest.Mock).mockResolvedValue(true);

    const op: CallbackOp = {
      op: "apply_edit",
      path: "/tmp/a.ts",
      text_edits: [
        {
          range: { start: { line: 0, character: 0 }, end: { line: 0, character: 0 } },
          new_text: "// header\n",
        },
      ],
    };

    const result = await makeHandler().handle(op);
    expect(result).toEqual({ ok: null });
    expect(vscode.workspace.applyEdit).toHaveBeenCalledTimes(1);
  });

  it("returns err when applyEdit is rejected", async () => {
    (vscode.workspace.applyEdit as jest.Mock).mockResolvedValue(false);

    const op: CallbackOp = {
      op: "apply_edit",
      path: "/tmp/b.ts",
      text_edits: [],
    };

    const result = await makeHandler().handle(op);
    expect(result).toMatchObject({ err: expect.stringContaining("rejected") });
  });
});

// ── reveal_file ───────────────────────────────────────────────────────────────

describe("reveal_file", () => {
  it("opens document and shows it, returns ok", async () => {
    const fakeDoc = { uri: { fsPath: "/tmp/c.ts" } };
    (vscode.workspace.openTextDocument as jest.Mock).mockResolvedValue(fakeDoc);
    (vscode.window.showTextDocument as jest.Mock) = jest.fn().mockResolvedValue({});

    const op: CallbackOp = { op: "reveal_file", path: "/tmp/c.ts" };
    const result = await makeHandler().handle(op);

    expect(result).toEqual({ ok: null });
    expect(vscode.workspace.openTextDocument).toHaveBeenCalled();
    expect(vscode.window.showTextDocument).toHaveBeenCalledWith(fakeDoc);
  });

  it("returns err when openTextDocument throws", async () => {
    (vscode.workspace.openTextDocument as jest.Mock).mockRejectedValue(
      new Error("file not found"),
    );

    const op: CallbackOp = { op: "reveal_file", path: "/nonexistent.ts" };
    const result = await makeHandler().handle(op);
    expect(result).toMatchObject({ err: "file not found" });
  });
});

// ── set_selection ─────────────────────────────────────────────────────────────

describe("set_selection", () => {
  it("sets editor selection and returns ok", async () => {
    const fakeEditor = { selection: null as unknown };
    const fakeDoc = { uri: { fsPath: "/tmp/d.ts" } };
    (vscode.workspace.openTextDocument as jest.Mock).mockResolvedValue(fakeDoc);
    (vscode.window.showTextDocument as jest.Mock) = jest.fn().mockResolvedValue(fakeEditor);

    const op: CallbackOp = {
      op: "set_selection",
      path: "/tmp/d.ts",
      range: { start: { line: 2, character: 0 }, end: { line: 2, character: 5 } },
    };

    const result = await makeHandler().handle(op);
    expect(result).toEqual({ ok: null });
    expect(fakeEditor.selection).not.toBeNull();
  });
});

// ── save ──────────────────────────────────────────────────────────────────────

describe("save", () => {
  it("save single file calls doc.save() and returns ok", async () => {
    const mockDoc = {
      uri: { fsPath: "/tmp/e.ts" },
      save: jest.fn().mockResolvedValue(true),
    };
    (vscode.workspace as unknown as Record<string, unknown>).textDocuments = [mockDoc];

    const op: CallbackOp = { op: "save", path: "/tmp/e.ts" };
    const result = await makeHandler().handle(op);

    expect(result).toEqual({ ok: null });
    expect(mockDoc.save).toHaveBeenCalledTimes(1);
  });

  it("save all calls workspace.saveAll and returns ok", async () => {
    (vscode.workspace.saveAll as jest.Mock).mockResolvedValue(true);

    const op: CallbackOp = { op: "save", path: null };
    const result = await makeHandler().handle(op);

    expect(result).toEqual({ ok: null });
    expect(vscode.workspace.saveAll).toHaveBeenCalledWith(false);
  });

  it("returns err when file is not open", async () => {
    (vscode.workspace as unknown as Record<string, unknown>).textDocuments = [];

    const op: CallbackOp = { op: "save", path: "/not/open.ts" };
    const result = await makeHandler().handle(op);
    expect(result).toMatchObject({ err: expect.stringContaining("not open") });
  });
});

// ── run_task ──────────────────────────────────────────────────────────────────

describe("run_task", () => {
  it("finds task by name and executes it, returns ok", async () => {
    const task = { name: "cargo-build" } as vscode.Task;
    (vscode.tasks.fetchTasks as jest.Mock).mockResolvedValue([task]);
    (vscode.tasks.executeTask as jest.Mock).mockResolvedValue({ terminate: jest.fn() });

    const op: CallbackOp = { op: "run_task", name: "cargo-build" };
    const result = await makeHandler().handle(op);

    expect(result).toEqual({ ok: null });
    expect(vscode.tasks.executeTask).toHaveBeenCalledWith(task);
  });

  it("returns err when task is not found", async () => {
    (vscode.tasks.fetchTasks as jest.Mock).mockResolvedValue([]);

    const op: CallbackOp = { op: "run_task", name: "nonexistent" };
    const result = await makeHandler().handle(op);
    expect(result).toMatchObject({ err: expect.stringContaining("not found") });
  });
});

// ── run_terminal ──────────────────────────────────────────────────────────────

describe("run_terminal", () => {
  it("creates terminal and sends text, returns ok", async () => {
    const fakeTerm = { show: jest.fn(), sendText: jest.fn() };
    (vscode.window.createTerminal as jest.Mock) = jest.fn().mockReturnValue(fakeTerm);

    const op: CallbackOp = { op: "run_terminal", command: "cargo test" };
    const result = await makeHandler().handle(op);

    expect(result).toEqual({ ok: null });
    expect(fakeTerm.show).toHaveBeenCalled();
    expect(fakeTerm.sendText).toHaveBeenCalledWith("cargo test");
  });
});

// ── debug_control ─────────────────────────────────────────────────────────────

describe("debug_control", () => {
  it("start: calls debug.startDebugging and returns ok", async () => {
    (vscode.debug.startDebugging as jest.Mock).mockResolvedValue(true);

    const op: CallbackOp = { op: "debug_control", action: "start", config: "unit-tests" };
    const result = await makeHandler().handle(op);

    expect(result).toEqual({ ok: null });
    expect(vscode.debug.startDebugging).toHaveBeenCalledWith(
      undefined,
      "unit-tests",
    );
  });

  it("start: returns err when startDebugging returns false", async () => {
    (vscode.debug.startDebugging as jest.Mock).mockResolvedValue(false);

    const op: CallbackOp = { op: "debug_control", action: "start", config: "bad-config" };
    const result = await makeHandler().handle(op);
    expect(result).toMatchObject({ err: expect.stringContaining("rejected") });
  });

  it("stop: calls debug.stopDebugging and returns ok", async () => {
    (vscode.debug.stopDebugging as jest.Mock).mockResolvedValue(undefined);

    const op: CallbackOp = { op: "debug_control", action: "stop" };
    const result = await makeHandler().handle(op);

    expect(result).toEqual({ ok: null });
    expect(vscode.debug.stopDebugging).toHaveBeenCalledTimes(1);
  });
});
