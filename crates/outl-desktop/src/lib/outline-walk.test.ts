import { describe, expect, it } from "vitest";

import type { BlockNode } from "@outl/shared/api/types";

import {
  flattenVisible,
  isInVisualRange,
  nextVisibleId,
  previousVisibleId,
  visualRangeIds,
} from "./outline-walk";

function block(
  id: string,
  opts: { collapsed?: boolean; children?: BlockNode[] } = {},
): BlockNode {
  return {
    id,
    text: id,
    todo: null,
    tokens: [],
    collapsed: opts.collapsed ?? false,
    properties: [],
    children: opts.children ?? [],
  };
}

describe("flattenVisible", () => {
  it("walks parents before children, siblings in order", () => {
    const tree: BlockNode[] = [
      block("a", { children: [block("a1"), block("a2")] }),
      block("b"),
    ];
    expect(flattenVisible(tree)).toEqual(["a", "a1", "a2", "b"]);
  });

  it("skips children of collapsed nodes", () => {
    const tree: BlockNode[] = [
      block("a", {
        collapsed: true,
        children: [block("a1"), block("a2")],
      }),
      block("b"),
    ];
    expect(flattenVisible(tree)).toEqual(["a", "b"]);
  });

  it("returns [] for an empty outline", () => {
    expect(flattenVisible([])).toEqual([]);
  });

  it("recurses through deeply nested visible subtrees", () => {
    const tree: BlockNode[] = [
      block("a", {
        children: [
          block("a1", {
            children: [block("a1a"), block("a1b")],
          }),
        ],
      }),
    ];
    expect(flattenVisible(tree)).toEqual(["a", "a1", "a1a", "a1b"]);
  });
});

describe("nextVisibleId", () => {
  const tree: BlockNode[] = [
    block("a"),
    block("b", { collapsed: true, children: [block("b1")] }),
    block("c"),
  ];

  it("returns the first id when current is null", () => {
    expect(nextVisibleId(null, tree)).toBe("a");
  });

  it("returns the first id when current is unknown to the outline", () => {
    expect(nextVisibleId("nonexistent", tree)).toBe("a");
  });

  it("steps over collapsed subtrees", () => {
    expect(nextVisibleId("b", tree)).toBe("c");
  });

  it("clamps at the bottom (no wrap)", () => {
    expect(nextVisibleId("c", tree)).toBe("c");
  });

  it("returns null on an empty outline", () => {
    expect(nextVisibleId("anything", [])).toBeNull();
    expect(nextVisibleId(null, [])).toBeNull();
  });
});

describe("previousVisibleId", () => {
  const tree: BlockNode[] = [
    block("a"),
    block("b", { collapsed: true, children: [block("b1")] }),
    block("c"),
  ];

  it("clamps at the top (no wrap)", () => {
    expect(previousVisibleId("a", tree)).toBe("a");
  });

  it("skips children of the collapsed parent on the way up", () => {
    expect(previousVisibleId("c", tree)).toBe("b");
  });

  it("returns first visible when current is unknown", () => {
    expect(previousVisibleId("ghost", tree)).toBe("a");
  });

  it("returns null on empty outline", () => {
    expect(previousVisibleId(null, [])).toBeNull();
  });
});

describe("visualRangeIds / isInVisualRange", () => {
  const tree: BlockNode[] = [block("a"), block("b"), block("c"), block("d")];

  it("orders anchor + cursor regardless of direction", () => {
    expect(visualRangeIds("b", "d", tree)).toEqual({ lo: "b", hi: "d" });
    expect(visualRangeIds("d", "b", tree)).toEqual({ lo: "b", hi: "d" });
  });

  it("returns null when either endpoint is missing or invisible", () => {
    expect(visualRangeIds(null, "a", tree)).toBeNull();
    expect(visualRangeIds("a", null, tree)).toBeNull();
    expect(visualRangeIds("ghost", "a", tree)).toBeNull();
  });

  it("highlights every block in [lo, hi]", () => {
    expect(isInVisualRange("a", "b", "d", tree)).toBe(false);
    expect(isInVisualRange("b", "b", "d", tree)).toBe(true);
    expect(isInVisualRange("c", "b", "d", tree)).toBe(true);
    expect(isInVisualRange("d", "b", "d", tree)).toBe(true);
  });

  it("single-block range still includes the anchor", () => {
    expect(isInVisualRange("b", "b", "b", tree)).toBe(true);
    expect(isInVisualRange("a", "b", "b", tree)).toBe(false);
  });

  it("returns false when range is invalid", () => {
    expect(isInVisualRange("a", null, "b", tree)).toBe(false);
    expect(isInVisualRange("a", "ghost", "b", tree)).toBe(false);
  });
});
