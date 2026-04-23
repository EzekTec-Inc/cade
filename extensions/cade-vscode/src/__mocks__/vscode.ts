/**
 * Minimal VS Code API mock for Jest tests.
 * Only stubs the surface used by this extension.
 */

export const window = {
  showErrorMessage: jest.fn(),
  showInformationMessage: jest.fn(),
  showWarningMessage: jest.fn(),
  activeTextEditor: undefined as unknown,
  onDidChangeActiveTextEditor: jest.fn(() => ({ dispose: jest.fn() })),
  onDidChangeTextEditorSelection: jest.fn(() => ({ dispose: jest.fn() })),
  createOutputChannel: jest.fn(() => ({
    appendLine: jest.fn(),
    show: jest.fn(),
    dispose: jest.fn(),
  })),
};

export const workspace = {
  textDocuments: [] as unknown[],
  workspaceFolders: undefined as unknown,
  onDidChangeTextDocument: jest.fn(() => ({ dispose: jest.fn() })),
  onDidChangeDiagnostics: jest.fn(() => ({ dispose: jest.fn() })),
  applyEdit: jest.fn(async () => true),
  openTextDocument: jest.fn(async (uri: unknown) => ({ uri })),
  saveAll: jest.fn(async () => true),
  getConfiguration: jest.fn(() => ({ get: jest.fn() })),
};

export const languages = {
  getDiagnostics: jest.fn(() => []),
};

export const commands = {
  registerCommand: jest.fn(() => ({ dispose: jest.fn() })),
  executeCommand: jest.fn(async () => undefined),
};

export const debug = {
  startDebugging: jest.fn(async () => true),
  stopDebugging: jest.fn(async () => undefined),
  activeDebugSession: undefined as unknown,
};

export const tasks = {
  fetchTasks: jest.fn(async () => []),
  executeTask: jest.fn(async () => ({ terminate: jest.fn() })),
};

export const Uri = {
  file: jest.fn((p: string) => ({ fsPath: p, toString: () => `file://${p}` })),
  parse: jest.fn((s: string) => ({ toString: () => s })),
};

export class WorkspaceEdit {
  private _edits: Array<{ uri: unknown; range: unknown; newText: string }> = [];
  replace(uri: unknown, range: unknown, newText: string): void {
    this._edits.push({ uri, range, newText });
  }
  get size(): number {
    return this._edits.length;
  }
}

export class Range {
  constructor(
    public readonly start: Position,
    public readonly end: Position,
  ) {}
}

export class Position {
  constructor(
    public readonly line: number,
    public readonly character: number,
  ) {}
}

export class Selection extends Range {
  constructor(start: Position, end: Position) {
    super(start, end);
  }
}

export const DiagnosticSeverity = {
  Error: 0,
  Warning: 1,
  Information: 2,
  Hint: 3,
} as const;

export const extensions = {
  getExtension: jest.fn(() => undefined),
};

export const env = {
  appName: "Visual Studio Code",
};
