import { describe, expect, it } from "vitest";
import {
  caretOnFirstLine,
  caretOnLastLine,
  parkCaret,
  spliceText,
} from "./textarea";

function newTextarea(value = "", caret = 0): HTMLTextAreaElement {
  const el = document.createElement("textarea");
  document.body.appendChild(el);
  el.value = value;
  el.setSelectionRange(caret, caret);
  el.focus();
  return el;
}

describe("spliceText", () => {
  it("inserts at start without disturbing surrounding text", () => {
    const el = newTextarea("world", 0);
    spliceText(el, 0, 0, "hello ");
    expect(el.value).toBe("hello world");
  });

  it("inserts mid-text at [start, end) range", () => {
    const el = newTextarea("foobar", 3);
    spliceText(el, 3, 3, "XYZ");
    expect(el.value).toBe("fooXYZbar");
  });

  it("replaces a selected range", () => {
    const el = newTextarea("foobar", 0);
    spliceText(el, 0, 3, "BAZ");
    expect(el.value).toBe("BAZbar");
  });

  it("falls back to el.value = … when setRangeText is missing", () => {
    const el = newTextarea("ab", 0);
    // Simulate an ancient browser by deleting setRangeText.
    // @ts-expect-error testing the fallback path
    el.setRangeText = undefined;
    spliceText(el, 1, 1, "Z");
    expect(el.value).toBe("aZb");
  });
});

describe("caretOnFirstLine", () => {
  it("is true for a single-line value at any caret", () => {
    expect(caretOnFirstLine("hello", 0)).toBe(true);
    expect(caretOnFirstLine("hello", 5)).toBe(true);
  });

  it("is true while the caret stays before the first newline", () => {
    expect(caretOnFirstLine("ab\ncd", 0)).toBe(true);
    expect(caretOnFirstLine("ab\ncd", 2)).toBe(true);
  });

  it("is false once the caret is past a newline", () => {
    // caret at index 3 sits on the second line ("cd").
    expect(caretOnFirstLine("ab\ncd", 3)).toBe(false);
    expect(caretOnFirstLine("ab\ncd", 5)).toBe(false);
  });
});

describe("caretOnLastLine", () => {
  it("is true for a single-line value at any caret", () => {
    expect(caretOnLastLine("hello", 0)).toBe(true);
    expect(caretOnLastLine("hello", 5)).toBe(true);
  });

  it("is true once the caret is on the final line", () => {
    expect(caretOnLastLine("ab\ncd", 3)).toBe(true);
    expect(caretOnLastLine("ab\ncd", 5)).toBe(true);
  });

  it("is false while a newline still follows the caret", () => {
    expect(caretOnLastLine("ab\ncd", 0)).toBe(false);
    expect(caretOnLastLine("ab\ncd", 2)).toBe(false);
  });
});

describe("parkCaret", () => {
  it("places the caret at the requested position", () => {
    const el = newTextarea("hello world", 0);
    parkCaret(el, 5);
    expect(el.selectionStart).toBe(5);
    expect(el.selectionEnd).toBe(5);
  });

  it("re-asserts the caret in a microtask (wins re-bindings)", async () => {
    const el = newTextarea("hello world", 0);
    parkCaret(el, 7);
    // Simulate Solid's `value={…}` binding effect running between
    // the sync and microtask phases.
    el.value = "hello world";
    el.setSelectionRange(el.value.length, el.value.length);
    // Yield to microtasks so the queued setSelectionRange runs.
    await Promise.resolve();
    expect(el.selectionStart).toBe(7);
  });

  it("does not throw when the element is detached", async () => {
    const el = newTextarea("hi", 0);
    el.remove();
    expect(() => parkCaret(el, 1)).not.toThrow();
    await Promise.resolve();
  });
});
