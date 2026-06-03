import { describe, expect, it } from "vitest";
import { BlockNode } from "./api";
import {
  countDescendants,
  findBlock,
  findInsertedAfter,
  flatten,
  rawTextWithTodo,
} from "./outline";

function block(id: string, text = "", children: BlockNode[] = []): BlockNode {
  return { id, text, todo: null, collapsed: false, properties: [], children };
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
