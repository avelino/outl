import { describe, expect, it } from "vitest";
import { looksLikeOutline } from "./paste";

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
    // `-foo` is not a bullet — must be `- foo`.
    expect(looksLikeOutline("-foo")).toBe(false);
    expect(looksLikeOutline("hyphen-word")).toBe(false);
  });
});
