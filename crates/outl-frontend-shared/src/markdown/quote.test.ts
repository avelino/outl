import { describe, expect, it } from "vitest";

import type { InlineToken } from "../api/types";
import {
  QUOTE_PREFIX,
  isQuote,
  splitQuote,
  stripQuoteFromTokens,
} from "./quote";

describe("splitQuote", () => {
  it("recognises the `> ` marker and strips it from the body", () => {
    expect(splitQuote("> a quote")).toEqual({ quoted: true, body: "a quote" });
  });

  it("returns the input untouched when the marker is absent", () => {
    expect(splitQuote("plain block")).toEqual({
      quoted: false,
      body: "plain block",
    });
  });

  it("requires the trailing space (CommonMark rule)", () => {
    expect(splitQuote(">no space")).toEqual({
      quoted: false,
      body: ">no space",
    });
  });

  it("accepts an empty body — `> ` alone is a legal quoted block", () => {
    expect(splitQuote("> ")).toEqual({ quoted: true, body: "" });
  });

  it("does not unwrap a nested quote — body keeps the inner `> `", () => {
    // Single split only: ">> foo" yields ("> foo") as the body. This
    // matches the Rust implementation and the issue's "no nested
    // quotes in first cut" policy.
    expect(splitQuote("> > foo")).toEqual({ quoted: true, body: "> foo" });
  });

  it("leaves inline tokens inside the body verbatim — wrapper is transparent", () => {
    const { quoted, body } = splitQuote("> **bold** [[ref]] #tag");
    expect(quoted).toBe(true);
    expect(body).toBe("**bold** [[ref]] #tag");
  });
});

describe("isQuote", () => {
  it("returns true only for the `> ` prefix", () => {
    expect(isQuote("> hi")).toBe(true);
    expect(isQuote("> ")).toBe(true);
    expect(isQuote(">hi")).toBe(false);
    expect(isQuote("hi")).toBe(false);
  });
});

describe("QUOTE_PREFIX", () => {
  it("matches the Rust mirror exactly", () => {
    // The Rust constant lives in `outl_actions::quote::QUOTE_PREFIX`
    // and is `"> "`. Drift here breaks cross-runtime parity.
    expect(QUOTE_PREFIX).toBe("> ");
  });
});

describe("stripQuoteFromTokens", () => {
  it("removes the `> ` prefix from the first Plain token", () => {
    const tokens: InlineToken[] = [{ kind: "plain", value: "> hello" }];
    expect(stripQuoteFromTokens(tokens)).toEqual([
      { kind: "plain", value: "hello" },
    ]);
  });

  it("leaves trailing tokens (refs, tags, bold) untouched", () => {
    const tokens: InlineToken[] = [
      { kind: "plain", value: "> see " },
      { kind: "ref", value: "Foo" },
      { kind: "plain", value: " for context" },
    ];
    const out = stripQuoteFromTokens(tokens);
    expect(out).toEqual([
      { kind: "plain", value: "see " },
      { kind: "ref", value: "Foo" },
      { kind: "plain", value: " for context" },
    ]);
  });

  it("is a no-op when the first token isn't a Plain starting with `> `", () => {
    const tokens: InlineToken[] = [
      { kind: "ref", value: "Foo" },
      { kind: "plain", value: " body" },
    ];
    expect(stripQuoteFromTokens(tokens)).toBe(tokens);
  });

  it("is a no-op on an empty token list", () => {
    const tokens: InlineToken[] = [];
    expect(stripQuoteFromTokens(tokens)).toBe(tokens);
  });
});
