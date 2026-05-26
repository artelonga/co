import { beforeEach } from "vitest";

// Provide a spec-compliant localStorage mock for happy-dom environments
// where the built-in localStorage does not implement all Storage API methods.
class LocalStorageMock implements Storage {
  private store: Map<string, string> = new Map();

  get length() { return this.store.size; }
  key(index: number): string | null {
    return Array.from(this.store.keys())[index] ?? null;
  }
  getItem(key: string): string | null { return this.store.get(key) ?? null; }
  setItem(key: string, value: string): void { this.store.set(key, String(value)); }
  removeItem(key: string): void { this.store.delete(key); }
  clear(): void { this.store.clear(); }
  [Symbol.iterator]() { return this.store.entries(); }
}

const localStorageMock = new LocalStorageMock();

Object.defineProperty(globalThis, "localStorage", {
  value: localStorageMock,
  writable: true,
});

beforeEach(() => {
  localStorageMock.clear();
});
