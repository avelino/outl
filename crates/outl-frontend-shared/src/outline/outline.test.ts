import { describe, expect, it } from "vitest";

import type { BlockNode } from "../api/types";

import {
  countDescendants,
  findBlock,
  flattenAll,
  flattenNodes,
  flattenParents,
  flattenVisible,
  isInVisualRange,
  nextVisibleId,
  previousVisibleId,
  rawTextWithTodo,
  visualRangeIds,
  visualRangeSet,
} from "./index";

function block(
  id: string,
  opts: {
    text?: string;
    todo?: "TODO" | "DONE" | null;
    collapsed?: boolean;
    children?: BlockNode[];
  } = {},
): BlockNode {
  return {
    id,
    text: opts.text ?? id,
    todo: opts.todo ?? null,
    tokens: [],
    collapsed: opts.collapsed ?? false,
    properties: [],
    children: opts.children ?? [],
  };
}

describe("rawTextWithTodo", () => {
  it("returns text verbatim when there is no TODO state", () => {
    expect(rawTextWithTodo(block("a", { text: "ship it" }))).toBe("ship it");
  });

  it("reattaches TODO prefix", () => {
    expect(rawTextWithTodo(block("x", { text: "ship it", todo: "TODO" }))).toBe(
      "TODO ship it",
    );
  });

  it("reattaches DONE prefix", () => {
    expect(rawTextWithTodo(block("x", { text: "ship it", todo: "DONE" }))).toBe(
      "DONE ship it",
    );
  });
});

describe("findBlock", () => {
  it("finds a top-level block", () => {
    const tree = [block("a"), block("b")];
    expect(findBlock(tree, "b")?.id).toBe("b");
  });

  it("descends into children recursively", () => {
    const tree = [
      block("a", { children: [block("a1", { children: [block("a1a")] })] }),
    ];
    expect(findBlock(tree, "a1a")?.id).toBe("a1a");
  });

  it("returns null when the id is not present", () => {
    expect(findBlock([block("a")], "missing")).toBeNull();
  });
});

describe("flattenNodes", () => {
  it("walks DFS preorder", () => {
    const tree = [
      block("a", { children: [block("a1"), block("a2")] }),
      block("b", { children: [block("b1")] }),
    ];
    expect(flattenNodes(tree).map((b) => b.id)).toEqual([
      "a",
      "a1",
      "a2",
      "b",
      "b1",
    ]);
  });

  it("returns an empty list for an empty tree", () => {
    expect(flattenNodes([])).toEqual([]);
  });

  it("includes children of collapsed nodes (fold state is ignored)", () => {
    const tree = [
      block("a", { collapsed: true, children: [block("a1")] }),
    ];
    expect(flattenNodes(tree).map((b) => b.id)).toEqual(["a", "a1"]);
  });
});

describe("countDescendants", () => {
  it("returns 0 for a leaf", () => {
    expect(countDescendants(block("a"))).toBe(0);
  });

  it("counts direct children", () => {
    const b = block("p", { children: [block("c1"), block("c2")] });
    expect(countDescendants(b)).toBe(2);
  });

  it("counts nested descendants", () => {
    const b = block("p", {
      children: [
        block("c1", { children: [block("c1a"), block("c1b")] }),
        block("c2"),
      ],
    });
    // c1 + c1a + c1b + c2 = 4
    expect(countDescendants(b)).toBe(4);
  });

  it("does not count the block itself", () => {
    expect(countDescendants(block("solo"))).toBe(0);
  });
});

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

  it("returns null at the top — never the current block (no wrap)", () => {
    // Must be null, not "a": returning the current (top) block left the
    // cursor on the very block a caller was about to delete, and the new
    // block then landed under the trash root (`o`-after-delete-all crash).
    expect(previousVisibleId("a", tree)).toBeNull();
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

describe("flattenAll", () => {
  it("includes children of collapsed nodes (unlike flattenVisible)", () => {
    // The whole reason flattenAll exists: zR / cursor-pruning must see
    // blocks hidden under a folded parent, which flattenVisible skips.
    const tree: BlockNode[] = [
      block("a", { collapsed: true, children: [block("a1"), block("a2")] }),
      block("b"),
    ];
    expect(flattenVisible(tree)).toEqual(["a", "b"]);
    expect(flattenAll(tree)).toEqual(["a", "a1", "a2", "b"]);
  });

  it("is empty for an empty outline", () => {
    expect(flattenAll([])).toEqual([]);
  });

  it("walks the same DFS order as flattenNodes, ids instead of nodes", () => {
    const tree: BlockNode[] = [
      block("a", { children: [block("a1")] }),
      block("b"),
    ];
    expect(flattenAll(tree)).toEqual(flattenNodes(tree).map((b) => b.id));
  });
});

describe("flattenParents", () => {
  it("includes only nodes with children, skipping leaves", () => {
    // zM (fold-all) targets parents only — folding a leaf writes a
    // SetCollapsed op that would make future children appear collapsed.
    const tree: BlockNode[] = [
      block("a", { children: [block("a1", { children: [block("a11")] })] }),
      block("b"),
    ];
    // a and a1 are parents; a11 and b are leaves.
    expect(flattenParents(tree)).toEqual(["a", "a1"]);
  });

  it("descends into collapsed parents too", () => {
    const tree: BlockNode[] = [
      block("a", {
        collapsed: true,
        children: [block("a1", { children: [block("a11")] })],
      }),
    ];
    expect(flattenParents(tree)).toEqual(["a", "a1"]);
  });
});

describe("visualRangeSet", () => {
  const tree: BlockNode[] = [block("a"), block("b"), block("c"), block("d")];

  it("builds the inclusive set of ids between anchor and cursor", () => {
    expect(visualRangeSet("b", "d", tree)).toEqual(new Set(["b", "c", "d"]));
  });

  it("orders anchor + cursor regardless of direction", () => {
    expect(visualRangeSet("d", "b", tree)).toEqual(new Set(["b", "c", "d"]));
  });

  it("is null when either endpoint is unset or off-outline", () => {
    expect(visualRangeSet(null, "b", tree)).toBeNull();
    expect(visualRangeSet("b", null, tree)).toBeNull();
    expect(visualRangeSet("b", "ghost", tree)).toBeNull();
  });
});
