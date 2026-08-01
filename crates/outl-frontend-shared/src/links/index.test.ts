import { describe, expect, it } from "vitest";

import { assetFileName, isImagePath, isSafeHttpUrl } from "./index";

describe("isImagePath", () => {
  it("matches known image extensions", () => {
    for (const ext of [
      "png",
      "jpg",
      "jpeg",
      "gif",
      "webp",
      "svg",
      "bmp",
      "avif",
      "ico",
      "tiff",
      "tif",
    ]) {
      expect(isImagePath(`assets/ab12.${ext}`)).toBe(true);
    }
  });

  it("is case-insensitive on the extension", () => {
    expect(isImagePath("assets/AB12.PNG")).toBe(true);
    expect(isImagePath("photo.JpG")).toBe(true);
  });

  it("strips a query string and fragment before matching", () => {
    expect(isImagePath("https://cdn/x.png?v=2")).toBe(true);
    expect(isImagePath("https://cdn/x.png#hash")).toBe(true);
    expect(isImagePath("https://cdn/x.png?v=2#hash")).toBe(true);
  });

  it("rejects non-image and extensionless targets", () => {
    expect(isImagePath("assets/report.pdf")).toBe(false);
    expect(isImagePath("assets/notes.txt")).toBe(false);
    expect(isImagePath("https://example.com/gallery")).toBe(false);
    expect(isImagePath("")).toBe(false);
    expect(isImagePath("README")).toBe(false);
  });
});

describe("assetFileName", () => {
  it("returns the last path segment", () => {
    expect(assetFileName("assets/ab12.png")).toBe("ab12.png");
    expect(assetFileName("a/b/c/report.pdf")).toBe("report.pdf");
  });

  it("strips a query string and fragment", () => {
    expect(assetFileName("https://cdn/photo.png?w=100")).toBe("photo.png");
    expect(assetFileName("https://cdn/photo.png#top")).toBe("photo.png");
  });

  it("ignores a trailing slash instead of returning an empty label", () => {
    expect(assetFileName("assets/folder/")).toBe("folder");
  });

  it("falls back to the raw href when there is no segment", () => {
    expect(assetFileName("")).toBe("");
  });
});

describe("isSafeHttpUrl", () => {
  it("accepts http and https", () => {
    expect(isSafeHttpUrl("http://example.com/x.png")).toBe(true);
    expect(isSafeHttpUrl("https://example.com/x.png")).toBe(true);
    expect(isSafeHttpUrl("HTTPS://EXAMPLE.COM/x.png")).toBe(true);
  });

  it("trims surrounding whitespace before checking", () => {
    expect(isSafeHttpUrl("  https://example.com/x.png  ")).toBe(true);
  });

  it("rejects other schemes so they never reach an <img src>", () => {
    expect(isSafeHttpUrl("file:///etc/passwd")).toBe(false);
    expect(isSafeHttpUrl("data:image/png;base64,AAAA")).toBe(false);
    expect(isSafeHttpUrl("javascript:alert(1)")).toBe(false);
    expect(isSafeHttpUrl("assets/ab12.png")).toBe(false);
    expect(isSafeHttpUrl("//evil.example/x.png")).toBe(false);
    expect(isSafeHttpUrl("")).toBe(false);
  });
});
