import { describe, expect, it } from "vitest";
import {
  applySuggestion,
  autoClosePair,
  detectRefContext,
  insertPair,
  insertText,
} from "./autocomplete";

describe("autoClosePair", () => {
  it("closes a freshly-typed [[ with ]]", () => {
    const result = autoClosePair("foo [[", 6);
    expect(result).toEqual({ value: "foo [[]]", caret: 6 });
  });

  it("closes a freshly-typed (( with ))", () => {
    const result = autoClosePair("see ((", 6);
    expect(result).toEqual({ value: "see (())", caret: 6 });
  });

  it("does nothing when the closer is already to the right", () => {
    expect(autoClosePair("[[]]", 2)).toBeNull();
    expect(autoClosePair("(())", 2)).toBeNull();
  });

  it("ignores single brackets", () => {
    expect(autoClosePair("a [b", 4)).toBeNull();
    expect(autoClosePair("a (b", 4)).toBeNull();
  });

  it("ignores positions too close to the start", () => {
    expect(autoClosePair("[", 1)).toBeNull();
  });
});

describe("insertPair", () => {
  it("inserts opener+closer with the caret in the middle", () => {
    const result = insertPair("hello world", 5, "[[", "]]");
    expect(result).toEqual({ value: "hello[[]] world", caret: 7 });
  });
});

describe("insertText", () => {
  it("inserts plain text and advances the caret", () => {
    const result = insertText("foo bar", 4, "#");
    expect(result).toEqual({ value: "foo #bar", caret: 5 });
  });
});

describe("detectRefContext", () => {
  it("recognises a page ref before the closer is typed", () => {
    const ctx = detectRefContext("see [[ave", 9);
    expect(ctx).toEqual({
      kind: "page",
      query: "ave",
      openIndex: 4,
      replaceEnd: 9,
    });
  });

  it("recognises a block ref before the closer is typed", () => {
    const ctx = detectRefContext("link ((blk-r", 12);
    expect(ctx).toEqual({
      kind: "block",
      query: "blk-r",
      openIndex: 5,
      replaceEnd: 12,
    });
  });

  it("includes the existing closer in the replace span", () => {
    const ctx = detectRefContext("see [[ave]] later", 9);
    expect(ctx).toEqual({
      kind: "page",
      query: "ave",
      openIndex: 4,
      replaceEnd: 11,
    });
  });

  it("returns null when the caret is past the closer", () => {
    expect(detectRefContext("see [[ave]] later", 12)).toBeNull();
  });

  it("does not capture across newlines", () => {
    expect(detectRefContext("first [[stale\nsecond", 20)).toBeNull();
  });

  it("does not capture when there is no opener", () => {
    expect(detectRefContext("plain text", 5)).toBeNull();
  });

  it("captures an empty query right after the opener", () => {
    const ctx = detectRefContext("see [[", 6);
    expect(ctx).toEqual({
      kind: "page",
      query: "",
      openIndex: 4,
      replaceEnd: 6,
    });
  });
});

describe("applySuggestion", () => {
  it("replaces an open page ref with the picked title", () => {
    const ctx = detectRefContext("see [[ave", 9)!;
    const result = applySuggestion("see [[ave", ctx, "Avelino");
    expect(result).toEqual({ value: "see [[Avelino]]", caret: 15 });
  });

  it("replaces an open block ref with the picked handle", () => {
    const ctx = detectRefContext("see ((blk-r", 11)!;
    const result = applySuggestion("see ((blk-r", ctx, "blk-r6s4a1");
    expect(result).toEqual({ value: "see ((blk-r6s4a1))", caret: 18 });
  });

  it("replaces a fully-closed ref in place", () => {
    const ctx = detectRefContext("see [[old]] tail", 7)!;
    const result = applySuggestion("see [[old]] tail", ctx, "new");
    expect(result).toEqual({ value: "see [[new]] tail", caret: 11 });
  });
});
