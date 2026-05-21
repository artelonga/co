// Minimal vscode API mock for Jest unit tests.
// Only implements the subset used by the extension.

export enum FileType {
  Unknown = 0,
  File = 1,
  Directory = 2,
  SymbolicLink = 64,
}

export enum FileChangeType {
  Changed = 1,
  Created = 2,
  Deleted = 3,
}

export class FileSystemError extends Error {
  code?: string;
  constructor(message: string) {
    super(message);
    this.name = "FileSystemError";
  }
  static FileNotFound(uri?: unknown): FileSystemError {
    const e = new FileSystemError(`FileNotFound: ${uri}`);
    e.code = "FileNotFound";
    return e;
  }
  static FileExists(uri?: unknown): FileSystemError {
    const e = new FileSystemError(`FileExists: ${uri}`);
    e.code = "FileExists";
    return e;
  }
  static NoPermissions(uri?: unknown): FileSystemError {
    const e = new FileSystemError(`NoPermissions: ${uri}`);
    e.code = "NoPermissions";
    return e;
  }
}

type Listener<T> = (e: T) => void;

export class EventEmitter<T> {
  private listeners: Array<Listener<T>> = [];

  get event() {
    return (listener: Listener<T>) => {
      this.listeners.push(listener);
      return new Disposable(() => {
        const i = this.listeners.indexOf(listener);
        if (i >= 0) this.listeners.splice(i, 1);
      });
    };
  }

  fire(data: T): void {
    for (const l of this.listeners) l(data);
  }

  dispose(): void {
    this.listeners = [];
  }
}

export class Disposable {
  constructor(private readonly fn: () => void) {}
  dispose(): void {
    this.fn();
  }
  static from(...disposables: { dispose(): void }[]): Disposable {
    return new Disposable(() => {
      for (const d of disposables) d.dispose();
    });
  }
}

export class Uri {
  readonly scheme: string;
  readonly authority: string;
  readonly path: string;
  readonly query: string;
  readonly fragment: string;

  private constructor(
    scheme: string,
    authority: string,
    path: string,
    query = "",
    fragment = "",
  ) {
    this.scheme = scheme;
    this.authority = authority;
    this.path = path;
    this.query = query;
    this.fragment = fragment;
  }

  static parse(value: string): Uri {
    const m = value.match(/^(\w[\w+\-.]*):\/\/([^/?#]*)([^?#]*)(\?[^#]*)?(#.*)?$/);
    if (!m) return new Uri("file", "", value);
    return new Uri(
      m[1],
      m[2] ?? "",
      m[3] ?? "",
      (m[4] ?? "").replace(/^\?/, ""),
      (m[5] ?? "").replace(/^#/, ""),
    );
  }

  toString(): string {
    return `${this.scheme}://${this.authority}${this.path}`;
  }
}

export const CompletionItemKind = {
  Text: 0,
  Method: 1,
  Function: 2,
  File: 17,
  Folder: 19,
} as const;

export class CompletionItem {
  insertText?: string;
  filterText?: string;
  detail?: string;
  constructor(
    public label: string,
    public kind?: number,
  ) {}
}

export const workspace = {
  getConfiguration: (_section?: string) => ({
    get: (_key: string) => undefined,
  }),
  workspaceFolders: [] as unknown[],
  updateWorkspaceFolders: jest.fn(),
  registerFileSystemProvider: jest.fn(() => new Disposable(() => {})),
};

export const window = {
  showQuickPick: jest.fn(),
  showErrorMessage: jest.fn(),
  showWarningMessage: jest.fn(),
  showInformationMessage: jest.fn(),
};

export const languages = {
  registerCompletionItemProvider: jest.fn(() => new Disposable(() => {})),
};

export const commands = {
  registerCommand: jest.fn(() => new Disposable(() => {})),
  executeCommand: jest.fn(),
};
