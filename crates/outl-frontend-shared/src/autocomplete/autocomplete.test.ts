import { describe, expect, it } from "vitest";

import {
  applySuggestion,
  autoClosePair,
  autoDeletePair,
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

describe("detectRefContext: mention", () => {
  it("detects @ at the start of the buffer", () => {
    expect(detectRefContext("@av", 3)).toEqual({
      kind: "mention",
      query: "av",
      openIndex: 0,
      replaceEnd: 3,
    });
  });

  it("detects @ after whitespace (word-initial)", () => {
    expect(detectRefContext("hi @av", 6)).toEqual({
      kind: "mention",
      query: "av",
      openIndex: 3,
      replaceEnd: 6,
    });
  });

  it("keeps spaces in the query for composite names", () => {
    expect(detectRefContext("@Thiago Av", 10)).toEqual({
      kind: "mention",
      query: "Thiago Av",
      openIndex: 0,
      replaceEnd: 10,
    });
  });

  it("does not trigger on a mid-word @ (email-shaped input)", () => {
    // `a@b.com` is an email, not a mention. The caret sits after
    // `b`, walked back through `b` and `@`. Since `@` is preceded by
    // `a` (no whitespace), the mention pre-pass must NOT fire.
    const ctx = detectRefContext("a@b", 3);
    expect(ctx?.kind).not.toBe("mention");
  });

  it("captures an empty query immediately after the @", () => {
    expect(detectRefContext("hi @", 4)).toEqual({
      kind: "mention",
      query: "",
      openIndex: 3,
      replaceEnd: 4,
    });
  });

  it("stops at a `[[` opener so page refs win over a stray @", () => {
    // The caret sits inside an open `[[av…` — the page-ref branch
    // must win, regardless of the `@x ` that sits earlier in the
    // line.
    const ctx = detectRefContext("@x [[av", 7);
    expect(ctx?.kind).toBe("page");
    expect(ctx?.query).toBe("av");
  });
});

describe("applySuggestion: mention", () => {
  it("wraps the chosen title in `[[@…]]`", () => {
    const ctx = detectRefContext("hi @av", 6)!;
    const result = applySuggestion("hi @av", ctx, "avelino");
    expect(result).toEqual({ value: "hi [[@avelino]]", caret: 15 });
  });

  it("preserves spaces inside the chosen composite name", () => {
    const ctx = detectRefContext("@Thiago Av", 10)!;
    const result = applySuggestion("@Thiago Av", ctx, "Thiago Avelino");
    expect(result).toEqual({
      value: "[[@Thiago Avelino]]",
      caret: 19,
    });
  });
});
