import "@testing-library/jest-dom/vitest";
import { afterEach, vi } from "vitest";
import { cleanup } from "@testing-library/react";

// Node 25 ships an experimental global `localStorage` that isn't a real Storage and shadows
// jsdom's — install a plain in-memory one so components and tests get consistent behaviour.
const store = new Map<string, string>();
const memoryStorage: Storage = {
  get length() {
    return store.size;
  },
  clear: () => store.clear(),
  getItem: (k) => (store.has(k) ? store.get(k)! : null),
  key: (i) => [...store.keys()][i] ?? null,
  removeItem: (k) => {
    store.delete(k);
  },
  setItem: (k, v) => {
    store.set(k, String(v));
  },
};
Object.defineProperty(globalThis, "localStorage", { value: memoryStorage, configurable: true, writable: true });

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
});
