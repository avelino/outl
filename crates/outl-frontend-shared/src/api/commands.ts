/**
 * Typed wrappers around the Tauri commands every outl frontend
 * client invokes. The Rust side (`outl-mobile/src-tauri/src/lib.rs`,
 * `outl-desktop/src-tauri/src/lib.rs`) registers commands with the
 * exact names below; this file is the single TS surface so both
 * clients agree on shape, name and return type.
 *
 * Client-specific commands (mobile gestures, `pick_workspace_dir`
 * on desktop, etc) live in the client's own `lib/api.ts` and never
 * end up here.
 *
 * `run_code_block` used to be considered client-specific (desktop
 * only). Mobile picked up the same command as of v0.6.x (long-press
 * → "Run code"), and both clients now share the wrapper below.
 */

import { invoke } from "@tauri-apps/api/core";

import type {
  CreateBlockReply,
  PageMeta,
  PageView,
  RunCodeBlockReply,
  WorkspaceSummary,
} from "./types";

/**
 * One emoji match returned by {@link searchEmojis}. Mirrors
 * `outl_md::emoji::EmojiHit` (`outl-md/src/emoji.rs`). `score` is
 * stable enough for the autocomplete popup to sort on — not a public
 * ranking guarantee.
 */
export interface EmojiHit {
  shortcode: string;
  glyph: string;
  score: number;
}

// ---------------------------------------------------------------------------
// Page / journal navigation
// ---------------------------------------------------------------------------

export function listPages(): Promise<PageMeta[]> {
  return invoke<PageMeta[]>("list_all_pages");
}

/**
 * Fuzzy-search known pages by `query`. Empty query returns up to 25
 * pages in the workspace's natural order; non-empty filters by
 * case-insensitive substring on title and slug and ranks exact and
 * prefix matches first.
 *
 * Used by the floating ref suggester that appears while the user
 * types inside `[[…]]`.
 */
export function searchPages(query: string): Promise<PageMeta[]> {
  return invoke<PageMeta[]>("search_pages", { query });
}

/**
 * Search the workspace for pages with `type:: person`, ranked by the
 * query (same fuzzy shape as {@link searchPages} — exact, prefix,
 * contains). Powers the `@` mention autocomplete in every client.
 * Backed by `outl_actions::search_persons` so every surface ranks the
 * same way without re-implementing the filter.
 */
export function searchPersons(query: string): Promise<PageMeta[]> {
  return invoke<PageMeta[]>("search_persons", { query });
}

/**
 * Search the GitHub gemoji catalog for shortcodes matching `query`.
 * Ranks exact → prefix → substring; shorter shortcodes win ties.
 * `limit` caps the result set (defaults to 8 — the size of the
 * autocomplete popup in every client). Backed by
 * `outl_md::emoji::search` so TUI / mobile / desktop rank identically.
 */
export function searchEmojis(query: string, limit = 8): Promise<EmojiHit[]> {
  return invoke<EmojiHit[]>("outl_emoji_search", { query, limit });
}

export function openTodayJournal(): Promise<PageView> {
  return invoke<PageView>("open_today_journal");
}

export function openJournalFor(slug: string): Promise<PageView> {
  return invoke<PageView>("open_journal_for", { slug });
}

export function openPageBySlug(slug: string): Promise<PageView> {
  return invoke<PageView>("open_page_by_slug", { slug });
}

/**
 * Resolve and open whatever a user-typed ref / tag / picker entry
 * points at, in one round-trip. The backend runs the canonical
 * decision tree (date → journal, else literal/slugified/title match
 * → existing page, else create a fresh page with the typed string
 * as the title). This is the entry point every ref-click handler
 * on the frontend should use; never branch by shape in TS before
 * calling — the regex-vs-parser drift is exactly what this command
 * exists to remove (`[[2026-13-01]]` used to surface
 * `invalid date slug` because the TS shape regex disagreed with
 * the Rust semantic parser).
 */
export function openRef(target: string): Promise<PageView> {
  return invoke<PageView>("open_ref", { target });
}

export function previousDay(slug: string): Promise<string> {
  return invoke<string>("previous_day", { slug });
}

export function nextDay(slug: string): Promise<string> {
  return invoke<string>("next_day", { slug });
}

export function todaySlug(): Promise<string> {
  return invoke<string>("today_slug_cmd");
}

export function dateTitle(slug: string): Promise<string> {
  return invoke<string>("date_title", { slug });
}

export function resolveRef(target: string): Promise<PageMeta | null> {
  return invoke<PageMeta | null>("resolve_ref", { target });
}

export function workspaceStats(): Promise<WorkspaceSummary> {
  return invoke<WorkspaceSummary>("workspace_stats");
}

// ---------------------------------------------------------------------------
// Block mutations (all scoped to a page)
// ---------------------------------------------------------------------------

/**
 * Insert a new block under `pageId`. Placement precedence:
 * `beforeId` (sibling immediately before — vim `O`) wins over
 * `afterId` (sibling immediately after — vim `o`), falling back to
 * the last child of `parentId` (defaults to the page itself when all
 * are null). Returns the refreshed {@link PageView} paired with
 * `new_id` — the id of the freshly-inserted block — so the caller
 * can put it into edit mode without diffing the outline. See
 * {@link CreateBlockReply} for why the id is on the wire.
 */
export function createBlock(
  pageId: string,
  opts: {
    afterId?: string | null;
    beforeId?: string | null;
    parentId?: string | null;
    text?: string | null;
  },
): Promise<CreateBlockReply> {
  return invoke<CreateBlockReply>("create_block", {
    pageId,
    afterId: opts.afterId ?? null,
    beforeId: opts.beforeId ?? null,
    parentId: opts.parentId ?? null,
    text: opts.text ?? null,
  });
}

export function editBlock(pageId: string, id: string, text: string): Promise<PageView> {
  return invoke<PageView>("edit_block", { pageId, id, text });
}

export function toggleTodo(pageId: string, id: string): Promise<PageView> {
  return invoke<PageView>("toggle_todo", { pageId, id });
}

/**
 * Flip the block's blockquote marker on or off. Mirrors
 * `outl_actions::block::toggle_quote`. Both mobile and desktop expose
 * the same `toggle_quote` Tauri command so this wrapper works on
 * every client.
 */
export function toggleQuote(pageId: string, id: string): Promise<PageView> {
  return invoke<PageView>("toggle_quote", { pageId, id });
}

export function deleteBlock(pageId: string, id: string): Promise<PageView> {
  return invoke<PageView>("delete_block", { pageId, id });
}

export function indentBlock(pageId: string, id: string): Promise<PageView> {
  return invoke<PageView>("indent_block", { pageId, id });
}

export function outdentBlock(pageId: string, id: string): Promise<PageView> {
  return invoke<PageView>("outdent_block", { pageId, id });
}

export function moveBlockUp(pageId: string, id: string): Promise<PageView> {
  return invoke<PageView>("move_block_up", { pageId, id });
}

export function moveBlockDown(pageId: string, id: string): Promise<PageView> {
  return invoke<PageView>("move_block_down", { pageId, id });
}

export function reloadWorkspace(): Promise<void> {
  return invoke<void>("reload_workspace");
}

/**
 * Hand off an external-clipboard paste to the backend for conversion
 * into a tree of blocks. The Rust side normalises external syntax
 * (Roam `{{[[TODO]]}}`, GitHub checkboxes, etc.), parses the bullet
 * structure, and grafts it under `blockId` at the caret position.
 *
 * Returns the refreshed `PageView` so the caller can re-render.
 * `caret` is a `char` offset into the host block's text — for ASCII
 * content the textarea's `selectionStart` (UTF-16 code units) is
 * equivalent. The frontend should `preventDefault` on the original
 * paste event before calling this so the default browser splice
 * doesn't run alongside the backend conversion.
 */
export function pasteMarkdown(
  pageId: string,
  blockId: string,
  caret: number,
  text: string,
): Promise<PageView> {
  return invoke<PageView>("paste_markdown_at", {
    pageId,
    blockId,
    caret,
    text,
  });
}

/**
 * Persist the collapsed flag on a single block. The backend generates
 * `Op::SetCollapsed` and appends it to the device's per-actor
 * `ops-<actor>.jsonl` so the flag converges across peers through the
 * tree CRDT — never written to the sidecar (last-write-wins per file
 * would lose concurrent flips). Returns the refreshed page view so
 * the caller can re-render in one round trip.
 */
export function setBlockCollapsed(
  pageId: string,
  id: string,
  collapsed: boolean,
): Promise<PageView> {
  return invoke<PageView>("set_block_collapsed", { pageId, id, collapsed });
}

/**
 * Run the fenced code block identified by `blockId` inside the page
 * identified by `pageId`. The Rust side resolves the flat-DFS index,
 * runs `outl_exec::run_block_at_index` on a worker thread, and
 * persists the result as a `> **result:**` sibling subblock.
 *
 * The reply bundles the refreshed `PageView` so the caller swaps the
 * outline straight in — no follow-up navigation round-trip needed.
 * `result_ok` is the runtime payload (stdout/stderr/exit/duration);
 * `error` is an infrastructure failure message (unknown language,
 * timeout, sandbox crash). They are mutually exclusive.
 */
export function runCodeBlock(
  pageId: string,
  blockId: string,
): Promise<RunCodeBlockReply> {
  return invoke<RunCodeBlockReply>("run_code_block", { pageId, blockId });
}
