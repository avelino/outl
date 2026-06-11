import { describe, expect, it } from "vitest";

import {
  applySuggestion,
  autoClosePair,
  autoDeletePair,
  autoPairBracket,
  detectRefContext,
  insertPair,
  insertText,
} from "./index";

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

describe("autoPairBracket", () => {
  it.each([
    ["(", ")"],
    ["[", "]"],
    ["{", "}"],
  ])("typing %s inserts the pair with the caret between", (open, close) => {
    const result = autoPairBracket("ab", 1, open);
    expect(result).toEqual({ value: `a${open}${close}b`, caret: 2 });
  });

  it("pairs at the very start and end of the value", () => {
    expect(autoPairBracket("", 0, "(")).toEqual({ value: "()", caret: 1 });
    expect(autoPairBracket("xy", 2, "{")).toEqual({ value: "xy{}", caret: 3 });
  });

  it("builds [[|]] when the second [ is typed inside an auto-paired []", () => {
    // First `[` produced `[|]`; the second one must land as `[[|]]`
    // so the existing `[[` ref flow keeps working (autoClosePair is
    // a no-op on that shape — the closer is never doubled).
    const result = autoPairBracket("a[]", 2, "[");
    expect(result).toEqual({ value: "a[[]]", caret: 3 });
    expect(autoClosePair(result!.value, result!.caret)).toBeNull();
  });

  it.each([")", "]", "}"])(
    "typing %s over an identical closer steps past it",
    (close) => {
      const open = { ")": "(", "]": "[", "}": "{" }[close]!;
      const value = `${open}x${close}`;
      expect(autoPairBracket(value, 2, close)).toEqual({ value, caret: 3 });
    },
  );

  it("steps over both closers of a doubled ref pair", () => {
    expect(autoPairBracket("[[a]]", 3, "]")).toEqual({ value: "[[a]]", caret: 4 });
    expect(autoPairBracket("[[a]]", 4, "]")).toEqual({ value: "[[a]]", caret: 5 });
  });

  it("lets a closer through when the next char differs", () => {
    expect(autoPairBracket("(x", 2, ")")).toBeNull();
    expect(autoPairBracket("(]", 1, ")")).toBeNull();
  });

  it("ignores non-bracket characters", () => {
    expect(autoPairBracket("ab", 1, "x")).toBeNull();
    expect(autoPairBracket("ab", 1, '"')).toBeNull();
    expect(autoPairBracket("ab", 1, "")).toBeNull();
  });
});

describe("autoDeletePair", () => {
  it("collapses an empty [[]] when caret is in the middle", () => {
    const result = autoDeletePair("foo [[]]", 6);
    expect(result).toEqual({ value: "foo ", caret: 4 });
  });

  it("collapses an empty (()) when caret is in the middle", () => {
    const result = autoDeletePair("see (())", 6);
    expect(result).toEqual({ value: "see ", caret: 4 });
  });

  it("does nothing when the pair has content between", () => {
    expect(autoDeletePair("[[ave]]", 5)).toBeNull();
    expect(autoDeletePair("((blk))", 5)).toBeNull();
  });

  it("does nothing when caret is outside the pair", () => {
    expect(autoDeletePair("[[]]", 0)).toBeNull();
    expect(autoDeletePair("[[]]", 4)).toBeNull();
  });

  it("does nothing without a matching closer to the right", () => {
    expect(autoDeletePair("[[", 2)).toBeNull();
    expect(autoDeletePair("((foo", 2)).toBeNull();
  });

  it("does not cross-mix [[ with ))", () => {
    expect(autoDeletePair("[[))", 2)).toBeNull();
  });

  it.each([
    ["()", "("],
    ["[]", "["],
    ["{}", "{"],
  ])("collapses an empty %s in one keystroke", (pair) => {
    const result = autoDeletePair(`a${pair}b`, 2);
    expect(result).toEqual({ value: "ab", caret: 1 });
  });

  it("prefers the doubled pair over the inner single pair", () => {
    // Caret in `[[|]]` — the whole ref scaffold goes, not just `[]`.
    expect(autoDeletePair("[[]]", 2)).toEqual({ value: "", caret: 0 });
  });

  it("collapses nested singles one level at a time", () => {
    expect(autoDeletePair("([])", 2)).toEqual({ value: "()", caret: 1 });
  });

  it("does not collapse a single pair with content between", () => {
    expect(autoDeletePair("(x)", 2)).toBeNull();
  });

  it("does not cross-mix single openers and closers", () => {
    expect(autoDeletePair("(]", 1)).toBeNull();
    expect(autoDeletePair("{)", 1)).toBeNull();
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
