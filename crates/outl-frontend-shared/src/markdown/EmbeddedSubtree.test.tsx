import { render } from "solid-js/web";
import { describe, expect, it } from "vitest";

import type { BlockNode, InlineToken } from "../api/types";
import { EmbeddedSubtree } from "./EmbeddedSubtree";

function node(
  id: string,
  text: string,
  opts: { todo?: "TODO" | "DONE" | null; children?: BlockNode[] } = {},
): BlockNode {
  const tokens: InlineToken[] = [{ kind: "plain", value: text }];
  return {
    id,
    text,
    todo: opts.todo ?? null,
    tokens,
    collapsed: false,
    properties: [],
    children: opts.children ?? [],
  };
}

function mount(nodes: BlockNode[], depth?: number) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(
    () => <EmbeddedSubtree nodes={nodes} depth={depth} />,
    host,
  );
  return {
    host,
    dispose: () => {
      dispose();
      host.remove();
    },
  };
}

describe("EmbeddedSubtree", () => {
  it("renders each child row with a `↳` prefix", () => {
    const m = mount([node("a", "first"), node("b", "second")]);
    expect(m.host.textContent).toContain("↳ first");
    expect(m.host.textContent).toContain("↳ second");
    m.dispose();
  });

  it("renders nested children recursively", () => {
    const m = mount([
      node("a", "parent", { children: [node("a1", "child")] }),
    ]);
    expect(m.host.textContent).toContain("↳ parent");
    expect(m.host.textContent).toContain("↳ child");
    m.dispose();
  });

  it("prefixes the TODO/DONE marker", () => {
    const m = mount([
      node("a", "todo item", { todo: "TODO" }),
      node("b", "done item", { todo: "DONE" }),
    ]);
    expect(m.host.textContent).toContain("↳ ☐ todo item");
    expect(m.host.textContent).toContain("↳ ✓ done item");
    m.dispose();
  });

  it("stops recursing at depth 4 (embed cycle cap)", () => {
    // depth 4 root; its child would be depth 5 and must be dropped.
    const m = mount(
      [node("deep", "deep-root", { children: [node("deeper", "deeper-child")] })],
      4,
    );
    expect(m.host.textContent).toContain("↳ deep-root");
    expect(m.host.textContent).not.toContain("deeper-child");
    m.dispose();
  });

  it("renders four nesting levels before cutting off", () => {
    const tree = [
      node("l1", "level-1", {
        children: [
          node("l2", "level-2", {
            children: [
              node("l3", "level-3", {
                children: [
                  node("l4", "level-4", {
                    children: [node("l5", "level-5")],
                  }),
                ],
              }),
            ],
          }),
        ],
      }),
    ];
    const m = mount(tree);
    expect(m.host.textContent).toContain("↳ level-1");
    expect(m.host.textContent).toContain("↳ level-4");
    expect(m.host.textContent).not.toContain("level-5");
    m.dispose();
  });
});
