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
  copyBlockMarkdown: vi.fn(),
  createBlock: vi.fn(),
  deleteBlock: vi.fn(),
  deletePage: vi.fn(),
  editBlock: vi.fn(),
  indentBlock: vi.fn(),
  moveBlockAfter: vi.fn(),
  moveBlockDown: vi.fn(),
  moveBlockUp: vi.fn(),
  nextDay: vi.fn(),
  openJournalFor: vi.fn(),
  openRef: vi.fn(),
  openTodayJournal: vi.fn(),
  outdentBlock: vi.fn(),
  pasteBlockAfter: vi.fn(),
  previousDay: vi.fn(),
  setBlockCollapsed: vi.fn(),
  todaySlug: vi.fn(),
  toggleTodo: vi.fn(),
}));

vi.mock("./api", () => ({
  redoPage: vi.fn(),
  runCodeBlock: vi.fn(),
  undoPage: vi.fn(),
}));

import {
  copyBlockMarkdown,
  moveBlockAfter,
  openRef,
  pasteBlockAfter,
} from "@outl/shared/api/commands";

import { buildHandlers } from "./action-handlers";
import { redoPage, undoPage } from "./api";
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

describe("Undo / Redo (Cmd+Z / Cmd+Shift+Z)", () => {
  const applyView = vi.fn();
  const setError = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    setAppState({
      page: { id: "pg-1", slug: "today", title: "Today", kind: "journal" },
      outline: [block("blk-1", "hello")],
      backlinks: [],
      selectedBlockId: "blk-1",
      selectedBacklinkBlockId: null,
      editingBlockId: null,
      backlinksOpen: false,
    });
  });

  it("undoes on the current page and applies the restored view", async () => {
    const view = pageView();
    vi.mocked(undoPage).mockResolvedValue(view);

    const handlers = buildHandlers({ applyView, setError });
    await handlers.Undo?.();

    expect(undoPage).toHaveBeenCalledWith("pg-1");
    expect(applyView).toHaveBeenCalledWith(view);
    expect(setError).not.toHaveBeenCalled();
  });

  it("redoes on the current page and applies the restored view", async () => {
    const view = pageView();
    vi.mocked(redoPage).mockResolvedValue(view);

    const handlers = buildHandlers({ applyView, setError });
    await handlers.Redo?.();

    expect(redoPage).toHaveBeenCalledWith("pg-1");
    expect(applyView).toHaveBeenCalledWith(view);
  });

  it("surfaces an empty history as a status message, not a crash", async () => {
    vi.mocked(undoPage).mockRejectedValue("nothing to undo");

    const handlers = buildHandlers({ applyView, setError });
    await handlers.Undo?.();

    expect(setError).toHaveBeenCalledWith("nothing to undo");
    expect(applyView).not.toHaveBeenCalled();
  });

  it("does nothing when no page is open", async () => {
    setAppState("page", null);

    const handlers = buildHandlers({ applyView, setError });
    await handlers.Undo?.();
    await handlers.Redo?.();

    expect(undoPage).not.toHaveBeenCalled();
    expect(redoPage).not.toHaveBeenCalled();
  });
});

/**
 * Smoke tests for the view-mode block clipboard (cut / copy / paste).
 *
 * Cut is deferred: it only arms `appState.blockClipboard`; the actual
 * move happens on paste (a single identity-preserving `Op::Move`, so
 * `((blk-…))` refs survive). Copy snapshots the subtree as markdown and
 * its paste duplicates with fresh ids. These pin the cut-vs-copy branch
 * of `PasteBlock` so the two clipboards can't quietly swap behaviour.
 */
describe("block clipboard (cut / copy / paste)", () => {
  const applyView = vi.fn();
  const setError = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    setAppState({
      page: { id: "pg-1", slug: "today", title: "Today", kind: "journal" },
      outline: [block("blk-a", "a"), block("blk-b", "b")],
      backlinks: [],
      selectedBlockId: null,
      blockClipboard: null,
      editingBlockId: null,
    });
  });

  it("cut only arms the clipboard, no backend call yet", async () => {
    setAppState("selectedBlockId", "blk-a");

    const handlers = buildHandlers({ applyView, setError });
    await handlers.CutBlock?.();

    expect(appState.blockClipboard).toEqual({ kind: "cut", nodeId: "blk-a" });
    expect(moveBlockAfter).not.toHaveBeenCalled();
  });

  it("copy snapshots the block as markdown", async () => {
    vi.mocked(copyBlockMarkdown).mockResolvedValue("- a");
    setAppState("selectedBlockId", "blk-a");

    const handlers = buildHandlers({ applyView, setError });
    await handlers.CopyBlock?.();

    expect(copyBlockMarkdown).toHaveBeenCalledWith("blk-a");
    expect(appState.blockClipboard).toEqual({ kind: "copy", markdown: "- a" });
  });

  it("paste after a cut moves by id, consumes the clipboard, follows the block", async () => {
    const view = pageView();
    vi.mocked(moveBlockAfter).mockResolvedValue(view);
    setAppState("blockClipboard", { kind: "cut", nodeId: "blk-a" });
    setAppState("selectedBlockId", "blk-b");

    const handlers = buildHandlers({ applyView, setError });
    await handlers.PasteBlock?.();

    expect(moveBlockAfter).toHaveBeenCalledWith("pg-1", "blk-a", "blk-b");
    expect(pasteBlockAfter).not.toHaveBeenCalled();
    expect(applyView).toHaveBeenCalledWith(view);
    // A cut is consumed by its paste; selection follows the moved block.
    expect(appState.blockClipboard).toBeNull();
    expect(appState.selectedBlockId).toBe("blk-a");
  });

  it("paste after a copy duplicates and keeps the clipboard armed", async () => {
    const view = pageView();
    vi.mocked(pasteBlockAfter).mockResolvedValue(view);
    setAppState("blockClipboard", { kind: "copy", markdown: "- a" });
    setAppState("selectedBlockId", "blk-b");

    const handlers = buildHandlers({ applyView, setError });
    await handlers.PasteBlock?.();

    expect(pasteBlockAfter).toHaveBeenCalledWith("pg-1", "blk-b", "- a");
    expect(moveBlockAfter).not.toHaveBeenCalled();
    expect(applyView).toHaveBeenCalledWith(view);
    // A copy persists so it can be pasted again.
    expect(appState.blockClipboard).toEqual({ kind: "copy", markdown: "- a" });
  });

  it("pasting a cut onto itself is a no-op", async () => {
    setAppState("blockClipboard", { kind: "cut", nodeId: "blk-a" });
    setAppState("selectedBlockId", "blk-a");

    const handlers = buildHandlers({ applyView, setError });
    await handlers.PasteBlock?.();

    expect(moveBlockAfter).not.toHaveBeenCalled();
    expect(applyView).not.toHaveBeenCalled();
  });
});
