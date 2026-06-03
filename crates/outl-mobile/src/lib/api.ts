import { invoke } from "@tauri-apps/api/core";

export type TodoState = "TODO" | "DONE";
export type PageKind = "page" | "journal";

export interface BlockNode {
  id: string;
  text: string;
  todo: TodoState | null;
  /**
   * UI fold state overlaid from the backend's op log. `true` means
   * the children are hidden in the outline. Mutated via
   * `setBlockCollapsed`, which generates `Op::SetCollapsed` and
   * appends it to the device's `ops-<actor>.jsonl`; iCloud /
   * Syncthing propagate the per-actor file and every peer's CRDT
   * replays the op in HLC order. The sidecar is never written for
   * this flag.
   */
  collapsed: boolean;
  /**
   * `(key, value)` block properties — the `key:: value` lines a user
   * authored under the block in markdown. Empty when the block has
   * none. Backend builds this from `Op::SetProp` entries so it
   * survives the same way collapsed does (op log, not sidecar).
   */
  properties: Array<[string, string]>;
  children: BlockNode[];
}

export interface PageMeta {
  id: string;
  slug: string;
  title: string;
  kind: PageKind;
  /**
   * Optional emoji / icon string the user set on the page via the
   * `icon::` page property. Backend omits the field when unset; the
   * frontend can fall back to `📄`/`📅` based on `kind`.
   */
  icon?: string;
}

export interface Backlink {
  block_id: string;
  /** Source block body **without** the TODO/DONE prefix. The prefix
   * lives in {@link Backlink.todo} so the checkbox can render
   * separately from the markdown body. */
  block_text: string;
  todo: TodoState | null;
  source_page: PageMeta | null;
  /**
   * Source block as a self-contained outline subtree (children +
   * properties). Mirrors what `read_page_view_with_workspace` would
   * return for the same block. Today the mobile frontend renders
   * only `block_text` + `todo`; this field is exposed so a future UI
   * pass can surface children + properties without extending the
   * backend contract.
   */
  source_block: BlockNode;
  /**
   * DFS path of the source block inside `source_page`. Empty array
   * means the block is a direct child of the page root. Mirrors
   * `outl_actions::Backlink::source_block_path` — used by clients
   * that navigate inside a backlink's subtree (the TUI today).
   */
  source_block_path: number[];
  /**
   * On-disk path of `source_page`'s `.md` (inside the iCloud
   * container). Backend omits the field when the source block has
   * no enclosing page (legacy data).
   */
  source_path?: string;
}

export interface PageView {
  page: PageMeta;
  outline: BlockNode[];
  backlinks: Backlink[];
}

export interface WorkspaceSummary {
  blocks: number;
  ops: number;
  actor: string;
  storage_root: string;
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

export function openTodayJournal(): Promise<PageView> {
  return invoke<PageView>("open_today_journal");
}

export function openJournalFor(slug: string): Promise<PageView> {
  return invoke<PageView>("open_journal_for", { slug });
}

export function openPageBySlug(slug: string): Promise<PageView> {
  return invoke<PageView>("open_page_by_slug", { slug });
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

export function createBlock(
  pageId: string,
  opts: { afterId?: string | null; parentId?: string | null; text?: string | null },
): Promise<PageView> {
  return invoke<PageView>("create_block", {
    pageId,
    afterId: opts.afterId ?? null,
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
 * Persist the collapsed flag on a single block. The backend writes
 * straight to the sidecar (no `Op` is generated — collapsed is UI
 * state). Returns the refreshed page view so the caller can re-render
 * in one round trip.
 */
export function setBlockCollapsed(
  pageId: string,
  id: string,
  collapsed: boolean,
): Promise<PageView> {
  return invoke<PageView>("set_block_collapsed", { pageId, id, collapsed });
}
