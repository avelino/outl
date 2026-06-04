import { describe, expect, it } from "vitest";

import { looksLikeOutline, utf16OffsetToCharOffset } from "./index";

describe("looksLikeOutline", () => {
  it("returns false for empty input", () => {
    expect(looksLikeOutline("")).toBe(false);
  });

  it("returns false for plain text", () => {
    expect(looksLikeOutline("just one line of text")).toBe(false);
    expect(looksLikeOutline("multi\nline\nbut no bullets")).toBe(false);
  });

  it("returns true on a single bullet line", () => {
    expect(looksLikeOutline("- one bullet")).toBe(true);
  });

  it("returns true when the bullet is indented", () => {
    expect(looksLikeOutline("    - nested")).toBe(true);
    expect(looksLikeOutline("\t- tab-indented")).toBe(true);
  });

  it("returns true when bullets appear after non-bullet preface", () => {
    expect(looksLikeOutline("intro paragraph\n- bullet")).toBe(true);
  });

  it("ignores leading whitespace lines", () => {
    expect(looksLikeOutline("\n\n  \n- after blanks")).toBe(true);
  });

  it("treats an empty bullet marker as outline", () => {
    expect(looksLikeOutline("-")).toBe(true);
    expect(looksLikeOutline("  -")).toBe(true);
  });

  it("returns false for dash followed by non-space", () => {
    expect(looksLikeOutline("-foo")).toBe(false);
    expect(looksLikeOutline("hyphen-word")).toBe(false);
  });
});

describe("utf16OffsetToCharOffset", () => {
  it("returns 0 for offset 0", () => {
    expect(utf16OffsetToCharOffset("anything", 0)).toBe(0);
    expect(utf16OffsetToCharOffset("", 0)).toBe(0);
  });

  it("matches the UTF-16 offset for pure ASCII", () => {
    const s = "hello world";
    expect(utf16OffsetToCharOffset(s, 5)).toBe(5);
    expect(utf16OffsetToCharOffset(s, s.length)).toBe(s.length);
  });

  it("matches the UTF-16 offset for BMP text", () => {
    // pt-BR with accents — `á` is U+00E1, still BMP, one code unit.
    const s = "olá mundo";
    expect(utf16OffsetToCharOffset(s, 4)).toBe(4); // after "olá "
    expect(utf16OffsetToCharOffset(s, s.length)).toBe(s.length);
  });

  it("collapses surrogate pairs to a single char", () => {
    // 😀 = U+1F600 — supplementary plane, takes 2 UTF-16 code units.
    const s = "hi 😀 you";
    expect(utf16OffsetToCharOffset(s, 5)).toBe(4);
    expect(s.length).toBe(9);
    expect(utf16OffsetToCharOffset(s, s.length)).toBe(8);
  });

  it("clamps when the offset overshoots", () => {
    const s = "abc";
    expect(utf16OffsetToCharOffset(s, 999)).toBe(3);
  });
});
