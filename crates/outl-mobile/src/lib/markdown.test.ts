import { describe, expect, it } from "vitest";
import { tokenize } from "./markdown";

describe("tokenize", () => {
  it("emits plain text exactly once when no patterns match", () => {
    const tokens = tokenize("plain text");
    expect(tokens).toEqual([{ kind: "plain", value: "plain text" }]);
  });

  it("handles a single bold span", () => {
    const tokens = tokenize("**bold**");
    expect(tokens).toEqual([{ kind: "bold", value: "bold" }]);
  });

  it("mixes plain and bold", () => {
    const tokens = tokenize("hello **world**");
    expect(tokens).toEqual([
      { kind: "plain", value: "hello " },
      { kind: "bold", value: "world" },
    ]);
  });

  it("recognises asterisk italic", () => {
    const tokens = tokenize("an *italic* word");
    expect(tokens).toEqual([
      { kind: "plain", value: "an " },
      { kind: "italic", value: "italic" },
      { kind: "plain", value: " word" },
    ]);
  });

  it("recognises underscore italic", () => {
    const tokens = tokenize("_italic_ start");
    expect(tokens).toEqual([
      { kind: "italic", value: "italic" },
      { kind: "plain", value: " start" },
    ]);
  });

  it("doesn't treat underscores inside identifiers as italic", () => {
    const tokens = tokenize("foo_bar_baz");
    expect(tokens).toEqual([{ kind: "plain", value: "foo_bar_baz" }]);
  });

  it("renders inline code", () => {
    const tokens = tokenize("call `fn()` here");
    expect(tokens).toEqual([
      { kind: "plain", value: "call " },
      { kind: "code", value: "fn()" },
      { kind: "plain", value: " here" },
    ]);
  });

  it("captures wiki refs", () => {
    const tokens = tokenize("see [[avelino]] and [[2026-05-28]] please");
    expect(tokens).toEqual([
      { kind: "plain", value: "see " },
      { kind: "ref", value: "avelino" },
      { kind: "plain", value: " and " },
      { kind: "ref", value: "2026-05-28" },
      { kind: "plain", value: " please" },
    ]);
  });

  it("captures hashtags", () => {
    const tokens = tokenize("a #project tag");
    expect(tokens).toEqual([
      { kind: "plain", value: "a " },
      { kind: "tag", value: "#project" },
      { kind: "plain", value: " tag" },
    ]);
  });

  it("ignores hash inside a word", () => {
    const tokens = tokenize("pre#post");
    expect(tokens).toEqual([{ kind: "plain", value: "pre#post" }]);
  });

  it("preserves trailing text after the last match", () => {
    const tokens = tokenize("**bold** after");
    expect(tokens).toEqual([
      { kind: "bold", value: "bold" },
      { kind: "plain", value: " after" },
    ]);
  });

  it("handles consecutive matches without empty plain runs", () => {
    const tokens = tokenize("**a***b*");
    expect(tokens).toEqual([
      { kind: "bold", value: "a" },
      { kind: "italic", value: "b" },
    ]);
  });

  it("returns an empty list for the empty string", () => {
    expect(tokenize("")).toEqual([]);
  });

  it("captures inline block references", () => {
    const tokens = tokenize("see ((blk-r6s4a1)) and ((blk-foo)) now");
    expect(tokens).toEqual([
      { kind: "plain", value: "see " },
      { kind: "blockref", value: "blk-r6s4a1" },
      { kind: "plain", value: " and " },
      { kind: "blockref", value: "blk-foo" },
      { kind: "plain", value: " now" },
    ]);
  });

  it("captures block embeds with the leading bang", () => {
    const tokens = tokenize("expand !((blk-abc)) inline");
    expect(tokens).toEqual([
      { kind: "plain", value: "expand " },
      { kind: "embed", value: "blk-abc" },
      { kind: "plain", value: " inline" },
    ]);
  });

  it("distinguishes embed from blockref when both present", () => {
    const tokens = tokenize("!((blk-aaa)) vs ((blk-bbb))");
    expect(tokens).toEqual([
      { kind: "embed", value: "blk-aaa" },
      { kind: "plain", value: " vs " },
      { kind: "blockref", value: "blk-bbb" },
    ]);
  });

  it("never duplicates a plain run (regression for tokenizer bug)", () => {
    // The previous tokenizer accidentally pushed the tail plain text
    // twice when no patterns matched anywhere — the bullet would
    // render its text doubled in read mode.
    const text = "hello world";
    const tokens = tokenize(text);
    const reconstructed = tokens
      .map((t) => (t.kind === "plain" ? t.value : ""))
      .join("");
    expect(reconstructed).toBe(text);
  });
});
