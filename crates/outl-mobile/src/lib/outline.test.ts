import { describe, expect, it } from "vitest";
import { BlockNode } from "./api";
import {
  countDescendants,
  findBlock,
  findInsertedAfter,
  flatten,
  flattenVisible,
  neighborId,
  rawTextWithTodo,
} from "./outline";

function block(id: string, text = "", children: BlockNode[] = []): BlockNode {
  return { id, text, todo: null, collapsed: false, properties: [], children };
}

/** Same as `block` but pre-collapsed, so its children are hidden in
 *  the visible flatten / navigation order. */
function collapsed(id: string, children: BlockNode[] = []): BlockNode {
  return { id, text: "", todo: null, collapsed: true, properties: [], children };
}

describe("findBlock", () => {
  it("finds a top-level block", () => {
    const tree = [block("a"), block("b")];
    expect(findBlock(tree, "b")?.id).toBe("b");
  });

  it("descends into children recursively", () => {
    const tree = [block("a", "", [block("a1", "", [block("a1a")])])];
    expect(findBlock(tree, "a1a")?.id).toBe("a1a");
  });

  it("returns null when the id is not present", () => {
    expect(findBlock([block("a")], "missing")).toBeNull();
  });
});

describe("flatten", () => {
  it("walks DFS preorder", () => {
    const tree = [
      block("a", "", [block("a1"), block("a2")]),
      block("b", "", [block("b1")]),
    ];
    expect(flatten(tree).map((b) => b.id)).toEqual([
      "a",
      "a1",
      "a2",
      "b",
      "b1",
    ]);
  });

  it("returns an empty list for an empty tree", () => {
    expect(flatten([])).toEqual([]);
  });
});

describe("flattenVisible", () => {
  it("walks DFS preorder like flatten when nothing is collapsed", () => {
    const tree = [
      block("a", "", [block("a1"), block("a2")]),
      block("b", "", [block("b1")]),
    ];
    expect(flattenVisible(tree).map((b) => b.id)).toEqual([
      "a",
      "a1",
      "a2",
      "b",
      "b1",
    ]);
  });

  it("skips the children of a collapsed block", () => {
    const tree = [
      collapsed("a", [block("a1"), block("a2")]),
      block("b"),
    ];
    expect(flattenVisible(tree).map((b) => b.id)).toEqual(["a", "b"]);
  });

  it("keeps a collapsed block itself, only hides its subtree", () => {
    const tree = [block("a", "", [collapsed("a1", [block("a1a")])])];
    expect(flattenVisible(tree).map((b) => b.id)).toEqual(["a", "a1"]);
  });
});

describe("neighborId", () => {
  const tree = [
    block("a", "", [block("a1"), block("a2")]),
    block("b"),
  ];

  it("returns the next visible block going down", () => {
    expect(neighborId(tree, "a", "down")).toBe("a1");
    expect(neighborId(tree, "a2", "down")).toBe("b");
  });

  it("returns the previous visible block going up", () => {
    expect(neighborId(tree, "a1", "up")).toBe("a");
    expect(neighborId(tree, "b", "up")).toBe("a2");
  });

  it("returns null at the top and bottom edges", () => {
    expect(neighborId(tree, "a", "up")).toBeNull();
    expect(neighborId(tree, "b", "down")).toBeNull();
  });

  it("returns null for an unknown id", () => {
    expect(neighborId(tree, "ghost", "down")).toBeNull();
  });

  it("steps over a collapsed subtree instead of entering it", () => {
    const folded = [collapsed("a", [block("a1")]), block("b")];
    expect(neighborId(folded, "a", "down")).toBe("b");
    expect(neighborId(folded, "b", "up")).toBe("a");
  });
});

describe("findInsertedAfter", () => {
  it("returns the next block in DFS order", () => {
    const tree = [block("a"), block("b"), block("c")];
    expect(findInsertedAfter(tree, "a")?.id).toBe("b");
    expect(findInsertedAfter(tree, "b")?.id).toBe("c");
  });

  it("returns null when there is no follower", () => {
    const tree = [block("a")];
    expect(findInsertedAfter(tree, "a")).toBeNull();
  });

  it("returns null when the seed id is unknown", () => {
    expect(findInsertedAfter([block("a")], "ghost")).toBeNull();
  });

  it("crosses subtree boundaries", () => {
    const tree = [block("a", "", [block("a1")]), block("b")];
    expect(findInsertedAfter(tree, "a1")?.id).toBe("b");
  });
});

describe("rawTextWithTodo", () => {
  it("returns text verbatim when there is no TODO state", () => {
    expect(rawTextWithTodo(block("a", "ship it"))).toBe("ship it");
  });

  it("reattaches TODO prefix", () => {
    const b: BlockNode = {
      id: "x",
      text: "ship it",
      todo: "TODO",
      collapsed: false,
      properties: [],
      children: [],
    };
    expect(rawTextWithTodo(b)).toBe("TODO ship it");
  });

  it("reattaches DONE prefix", () => {
    const b: BlockNode = {
      id: "x",
      text: "ship it",
      todo: "DONE",
      collapsed: false,
      properties: [],
      children: [],
    };
    expect(rawTextWithTodo(b)).toBe("DONE ship it");
  });
});

describe("countDescendants", () => {
  it("returns 0 for a leaf", () => {
    expect(countDescendants(block("a"))).toBe(0);
  });

  it("counts direct children", () => {
    const b = block("p", "", [block("c1"), block("c2")]);
    expect(countDescendants(b)).toBe(2);
  });

  it("counts nested descendants", () => {
    const b = block("p", "", [
      block("c1", "", [block("c1a"), block("c1b")]),
      block("c2"),
    ]);
    // c1 + c1a + c1b + c2 = 4
    expect(countDescendants(b)).toBe(4);
  });

  it("does not count the block itself", () => {
    const b = block("solo");
    expect(countDescendants(b)).toBe(0);
  });
});
