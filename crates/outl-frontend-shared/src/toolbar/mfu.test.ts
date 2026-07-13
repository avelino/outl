import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_ORDER, PINNED_FIRST, PINNED_LAST } from "./actions";
import {
  MFU_STORAGE_KEY,
  orderedMiddleActions,
  orderedMiddleFromStore,
  parseCounts,
  readCountsFromStore,
  recordToStore,
} from "./mfu";

/** Reconstruct the full ordered row the way the client renders it —
 *  mirrors the Swift `orderedActions` convenience. */
function orderedActions(counts: Parameters<typeof orderedMiddleActions>[0]) {
  return [PINNED_FIRST, ...orderedMiddleActions(counts), PINNED_LAST];
}

describe("toolbar MFU — pure ordering", () => {
  it("returns default order with empty counts", () => {
    expect(orderedActions({})).toEqual(DEFAULT_ORDER);
  });

  it("keeps pinnedFirst at index 0 even against huge counts", () => {
    const order = orderedActions({ italic: 9999, code: 9999 });
    expect(order[0]).toBe(PINNED_FIRST);
  });

  it("keeps pinnedLast at the final index", () => {
    const order = orderedActions({ italic: 9999, code: 9999 });
    expect(order[order.length - 1]).toBe(PINNED_LAST);
  });

  it("hoists the most-used action right after pinnedFirst", () => {
    const order = orderedActions({ code: 10, italic: 1 });
    expect(order[0]).toBe("newLine");
    expect(order[1]).toBe("code");
  });

  it("breaks count ties by default-order index (bold before italic)", () => {
    const order = orderedActions({ italic: 5, bold: 5 });
    expect(order.indexOf("bold")).toBeLessThan(order.indexOf("italic"));
  });

  it("ignores counts recorded against pinned actions", () => {
    const order = orderedActions({ newLine: 9999, done: 9999, code: 1 });
    expect(order[0]).toBe("newLine");
    expect(order[order.length - 1]).toBe("done");
  });

  it("preserves cardinality (no dupes, nothing dropped)", () => {
    const order = orderedActions({ code: 10 });
    expect(order).toHaveLength(DEFAULT_ORDER.length);
    expect(new Set(order).size).toBe(DEFAULT_ORDER.length);
  });
});

describe("toolbar MFU — middle range excludes pinned", () => {
  it("never contains the pinned slots and keeps every other action", () => {
    const middle = orderedMiddleActions({});
    expect(middle).not.toContain(PINNED_FIRST);
    expect(middle).not.toContain(PINNED_LAST);
    expect(middle).toHaveLength(DEFAULT_ORDER.length - 2);
  });

  it("honours counts (most-used first in the middle)", () => {
    expect(orderedMiddleActions({ code: 10 })[0]).toBe("code");
  });

  it("middle + pinned reconstructs the full ordered list", () => {
    const counts = { italic: 3, bold: 2 };
    expect(orderedActions(counts)).toEqual([
      PINNED_FIRST,
      ...orderedMiddleActions(counts),
      PINNED_LAST,
    ]);
  });
});

describe("toolbar MFU — parse tolerance", () => {
  it("returns empty map for null / malformed / non-object input", () => {
    expect(parseCounts(null)).toEqual({});
    expect(parseCounts("not json")).toEqual({});
    expect(parseCounts("[1,2,3]")).toEqual({});
    expect(parseCounts("42")).toEqual({});
  });

  it("drops unknown action ids and non-finite values", () => {
    const raw = JSON.stringify({ code: 3, bogus: 5, italic: "x", bold: 2.9 });
    expect(parseCounts(raw)).toEqual({ code: 3, bold: 2 });
  });
});

/** happy-dom (this repo's Vitest env) ships no `localStorage`; the real
 *  Tauri webview always has one. Stub a minimal in-memory Storage so the
 *  persistence wrappers exercise their real read/write path. */
class MemStorage {
  private m = new Map<string, string>();
  get length() {
    return this.m.size;
  }
  clear() {
    this.m.clear();
  }
  getItem(k: string) {
    return this.m.has(k) ? (this.m.get(k) as string) : null;
  }
  setItem(k: string, v: string) {
    this.m.set(k, String(v));
  }
  removeItem(k: string) {
    this.m.delete(k);
  }
  key(i: number) {
    return [...this.m.keys()][i] ?? null;
  }
}

describe("toolbar MFU — localStorage persistence", () => {
  beforeAll(() => {
    vi.stubGlobal("localStorage", new MemStorage());
  });
  afterAll(() => {
    vi.unstubAllGlobals();
  });
  beforeEach(() => {
    localStorage.clear();
  });

  it("record increments and persists the count", () => {
    recordToStore("code");
    recordToStore("code");
    expect(readCountsFromStore().code).toBe(2);
  });

  it("record is a no-op for pinnedFirst and pinnedLast", () => {
    recordToStore("newLine");
    recordToStore("done");
    const counts = readCountsFromStore();
    expect(counts.newLine).toBeUndefined();
    expect(counts.done).toBeUndefined();
  });

  it("orderedMiddleFromStore reflects persisted taps", () => {
    recordToStore("bold");
    const middle = orderedMiddleFromStore();
    expect(middle[0]).toBe("bold");
    expect(middle).not.toContain("newLine");
    expect(middle).not.toContain("done");
  });

  it("writes under the versioned key", () => {
    recordToStore("italic");
    expect(localStorage.getItem(MFU_STORAGE_KEY)).toContain("italic");
  });
});
