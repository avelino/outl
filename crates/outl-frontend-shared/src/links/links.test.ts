import { describe, expect, it } from "vitest";

import { isAssetLink } from "./index";

describe("isAssetLink", () => {
  it("detects workspace asset links and their path variants", () => {
    expect(isAssetLink("assets/abc.pdf")).toBe(true);
    expect(isAssetLink("./assets/abc.pdf")).toBe(true);
    expect(isAssetLink("/assets/abc.pdf")).toBe(true);
  });

  it("rejects external and unrelated links", () => {
    expect(isAssetLink("https://example.com/x.pdf")).toBe(false);
    expect(isAssetLink("http://assets/x.pdf")).toBe(false);
    expect(isAssetLink("mailto:a@b.com")).toBe(false);
    expect(isAssetLink("pages/other.md")).toBe(false);
    expect(isAssetLink("my-assets/x.pdf")).toBe(false);
  });
});
