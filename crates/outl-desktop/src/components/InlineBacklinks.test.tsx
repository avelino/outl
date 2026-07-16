import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { Backlink, PageView, TodoState } from "@outl/shared/api/types";
import { openRef, toggleTodo } from "@outl/shared/api/commands";

import { InlineBacklinks } from "./InlineBacklinks";
import { setAppState } from "../lib/store";

vi.mock("@outl/shared/api/commands", () => ({
  openRef: vi.fn(),
  setBacklinksOrder: vi.fn(),
  toggleTodo: vi.fn(),
}));

let dispose: (() => void) | undefined;

function backlink(id: string, todo: TodoState | null): Backlink {
  return {
    block_id: id,
    todo,
    source_page: { id: "page-1", slug: "journal", title: "Journal", kind: "page" },
    source_block: {
      id,
      text: "task #project",
      todo,
      tokens: [
        { kind: "plain", value: "task " },
        { kind: "tag", value: "#project" },
      ],
      collapsed: false,
      properties: [],
      children: [],
    },
    source_block_path: [],
  };
}

function mount(backlinks: Backlink[]): HTMLElement {
  setAppState({
    page: { id: "tag-page", slug: "project", title: "project", kind: "page" },
    backlinksOpen: true,
    backlinks,
    backlinksOrder: "newest",
  });
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <InlineBacklinks />, host);
  return host;
}

function pageView(backlinks: Backlink[] = []): PageView {
  return {
    page: { id: "page-1", slug: "journal", title: "Journal", kind: "page" },
    outline: [],
    backlinks,
    backlinks_order: "newest",
  };
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  vi.clearAllMocks();
  setAppState({
    page: null,
    backlinks: [],
    backlinksOpen: true,
    backlinksOrder: "newest",
  });
  document.body.innerHTML = "";
});

describe("InlineBacklinks task state — #144", () => {
  it("renders task indicators in tag backlink listings", () => {
    const host = mount([
      backlink("todo", "TODO"),
      backlink("done", "DONE"),
      backlink("plain", null),
    ]);

    expect(host.querySelector('[data-todo="TODO"]')?.textContent).toBe("▢");
    expect(host.querySelector('[data-todo="DONE"]')?.textContent).toBe("▣");
    expect(host.querySelector('[data-todo="none"]')?.textContent).toBe("•");
    expect(host.querySelectorAll("[data-todo]")).toHaveLength(3);
    expect(host.querySelector('[data-todo="DONE"]')?.nextElementSibling?.className).toContain(
      "line-through",
    );
  });

  it("toggles the source task and refreshes the current tag view", async () => {
    vi.mocked(toggleTodo).mockResolvedValue(pageView());
    vi.mocked(openRef).mockResolvedValue(pageView([backlink("todo", "DONE")]));
    const host = mount([backlink("todo", "TODO")]);

    (host.querySelector('[data-todo="TODO"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(toggleTodo).toHaveBeenCalledWith("page-1", "todo");
      expect(openRef).toHaveBeenCalledWith("project");
    });
    expect(host.querySelector('[data-todo="DONE"]')?.textContent).toBe("▣");
  });

  it("starts a plain backlink as TODO", async () => {
    vi.mocked(toggleTodo).mockResolvedValue(pageView());
    vi.mocked(openRef).mockResolvedValue(pageView([backlink("plain", "TODO")]));
    const host = mount([backlink("plain", null)]);

    (host.querySelector('[data-todo="none"]') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(toggleTodo).toHaveBeenCalledWith("page-1", "plain");
      expect(host.querySelector('[data-todo="TODO"]')?.textContent).toBe("▢");
    });
  });

  it("keeps text clicks navigating to the source block", async () => {
    vi.mocked(openRef).mockResolvedValue(pageView());
    const host = mount([backlink("todo", "TODO")]);
    const textButton = Array.from(host.querySelectorAll("button")).find((button) =>
      button.textContent?.includes("task"),
    );

    if (!textButton) throw new Error("backlink text button did not render");
    textButton.click();

    await vi.waitFor(() => {
      expect(openRef).toHaveBeenCalledWith("journal");
    });
    expect(toggleTodo).not.toHaveBeenCalled();
  });
});
