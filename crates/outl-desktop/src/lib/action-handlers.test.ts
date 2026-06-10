/**
 * Regression tests for the `OpenRefUnderCursor` handler (issue #70).
 *
 * The desktop's Normal mode has no character cursor — only a selected
 * block — so "open the ref under the cursor" cannot be resolved. An
 * earlier handler approximated it as "the first `[[ref]]` in the
 * block", which made every ref-carrying block impossible to edit via
 * `Enter`. These tests pin the fixed contract:
 *
 * 1. Selection on an outline block (refs or not) → enter Insert.
 *    Following a ref stays the click on the token (`onRefClick`).
 * 2. Selection on a backlink row (read-only) → open the source page
 *    and land the cursor on the referencing block.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Backlink, BlockNode, PageView } from "@outl/shared/api/types";

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => ({ close: vi.fn() })),
}));

vi.mock("@outl/shared/api/commands", () => ({
  createBlock: vi.fn(),
  deleteBlock: vi.fn(),
  editBlock: vi.fn(),
  indentBlock: vi.fn(),
  moveBlockDown: vi.fn(),
  moveBlockUp: vi.fn(),
  nextDay: vi.fn(),
  openJournalFor: vi.fn(),
  openRef: vi.fn(),
  openTodayJournal: vi.fn(),
  outdentBlock: vi.fn(),
  previousDay: vi.fn(),
  setBlockCollapsed: vi.fn(),
  todaySlug: vi.fn(),
  toggleTodo: vi.fn(),
}));

vi.mock("./api", () => ({
  runCodeBlock: vi.fn(),
}));

import { openRef } from "@outl/shared/api/commands";

import { buildHandlers } from "./action-handlers";
import { appState, setAppState } from "./store";

function block(id: string, text: string): BlockNode {
  return {
    id,
    text,
    todo: null,
    tokens: [],
    collapsed: false,
    properties: [],
    children: [],
  };
}

function pageView(): PageView {
  return {
    page: { id: "pg-source", slug: "source", title: "Source", kind: "page" },
    outline: [],
    backlinks: [],
  };
}

describe("OpenRefUnderCursor (Normal-mode Enter)", () => {
  const applyView = vi.fn();
  const setError = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    setAppState({
      page: { id: "pg-1", slug: "today", title: "Today", kind: "journal" },
      outline: [
        block("blk-plain", "no refs here"),
        block("blk-with-ref", "see [[some-page]] and [[another]]"),
      ],
      backlinks: [],
      selectedBlockId: null,
      selectedBacklinkBlockId: null,
      editingBlockId: null,
      backlinksOpen: true,
    });
  });

  it("enters Insert on the selected block even when it contains a [[ref]] (#70)", async () => {
    setAppState("selectedBlockId", "blk-with-ref");

    const handlers = buildHandlers({ applyView, setError });
    await handlers.OpenRefUnderCursor?.();

    // Enter means edit — never "follow the first ref in the block".
    expect(appState.editingBlockId).toBe("blk-with-ref");
    expect(openRef).not.toHaveBeenCalled();
    expect(applyView).not.toHaveBeenCalled();
  });

  it("enters Insert on a ref-free block", async () => {
    setAppState("selectedBlockId", "blk-plain");

    const handlers = buildHandlers({ applyView, setError });
    await handlers.OpenRefUnderCursor?.();

    expect(appState.editingBlockId).toBe("blk-plain");
    expect(openRef).not.toHaveBeenCalled();
  });

  it("does nothing when no block is selected", async () => {
    const handlers = buildHandlers({ applyView, setError });
    await handlers.OpenRefUnderCursor?.();

    expect(appState.editingBlockId).toBeNull();
    expect(openRef).not.toHaveBeenCalled();
    expect(applyView).not.toHaveBeenCalled();
  });

  it("opens the source page when the selection sits on a backlink row", async () => {
    const view = pageView();
    vi.mocked(openRef).mockResolvedValue(view);
    const backlink: Backlink = {
      block_id: "blk-source",
      todo: null,
      source_page: {
        id: "pg-source",
        slug: "source",
        title: "Source",
        kind: "page",
      },
      source_block: block("blk-source", "points at [[today]]"),
      source_block_path: [0],
    };
    setAppState({
      backlinks: [backlink],
      selectedBlockId: null,
      selectedBacklinkBlockId: "blk-source",
    });

    const handlers = buildHandlers({ applyView, setError });
    await handlers.OpenRefUnderCursor?.();

    // Backlink rows are read-only: Enter opens, never edits.
    expect(openRef).toHaveBeenCalledWith("source");
    expect(applyView).toHaveBeenCalledWith(view);
    expect(appState.editingBlockId).toBeNull();
    // Cursor lands on the referencing block of the opened page.
    expect(appState.selectedBacklinkBlockId).toBeNull();
    expect(appState.selectedBlockId).toBe("blk-source");
  });
});
