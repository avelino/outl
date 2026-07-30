import { describe, expect, it } from "vitest";

import {
  appendMarkdownToBlock,
  blockIdFromElement,
  joinAssetMarkdowns,
  physicalToCss,
} from "./index";

describe("physicalToCss", () => {
  it("divides physical pixels by the device pixel ratio", () => {
    expect(physicalToCss({ x: 200, y: 100 }, 2)).toEqual({ x: 100, y: 50 });
  });

  it("is a no-op at dpr 1", () => {
    expect(physicalToCss({ x: 42, y: 7 }, 1)).toEqual({ x: 42, y: 7 });
  });

  it("falls back to ratio 1 for a bogus (<=0) dpr", () => {
    expect(physicalToCss({ x: 10, y: 20 }, 0)).toEqual({ x: 10, y: 20 });
  });
});

describe("blockIdFromElement", () => {
  it("returns null for a null element", () => {
    expect(blockIdFromElement(null)).toBeNull();
  });

  it("reads data-block-id off the hit element itself", () => {
    const el = document.createElement("div");
    el.dataset.blockId = "blk-abc";
    expect(blockIdFromElement(el)).toBe("blk-abc");
  });

  it("walks up to the nearest ancestor carrying the id", () => {
    const row = document.createElement("div");
    row.dataset.blockId = "blk-parent";
    const inner = document.createElement("span");
    row.appendChild(inner);
    expect(blockIdFromElement(inner)).toBe("blk-parent");
  });

  it("returns null when no ancestor carries the id", () => {
    const el = document.createElement("div");
    const inner = document.createElement("span");
    el.appendChild(inner);
    expect(blockIdFromElement(inner)).toBeNull();
  });
});

describe("joinAssetMarkdowns", () => {
  it("joins multiple links with a single space", () => {
    expect(
      joinAssetMarkdowns(["[a](assets/a.pdf)", "![b](assets/b.png)"]),
    ).toBe("[a](assets/a.pdf) ![b](assets/b.png)");
  });

  it("drops empty entries", () => {
    expect(joinAssetMarkdowns(["", "[a](assets/a.pdf)", ""])).toBe(
      "[a](assets/a.pdf)",
    );
  });

  it("is empty for an all-empty list", () => {
    expect(joinAssetMarkdowns(["", ""])).toBe("");
  });
});

describe("appendMarkdownToBlock", () => {
  it("separates existing text from the link with one space", () => {
    expect(appendMarkdownToBlock("see this", "[a](assets/a.pdf)")).toBe(
      "see this [a](assets/a.pdf)",
    );
  });

  it("adds no leading space to an empty block", () => {
    expect(appendMarkdownToBlock("", "[a](assets/a.pdf)")).toBe(
      "[a](assets/a.pdf)",
    );
  });
});
