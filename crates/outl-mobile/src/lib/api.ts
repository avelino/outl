import { invoke } from "@tauri-apps/api/core";

export type TodoState = "TODO" | "DONE";
export type PageKind = "page" | "journal";

export interface BlockNode {
  id: string;
  text: string;
  todo: TodoState | null;
  /**
   * UI fold state echoed from the sidecar. `true` means the children
   * are hidden in the outline. Mutated via `setBlockCollapsed` —
   * persists in `.outl` so the state survives across sessions and
   * (since the sidecar syncs through iCloud) across devices.
   */
  collapsed: boolean;
  children: BlockNode[];
}

export interface PageMeta {
  id: string;
  slug: string;
  title: string;
  kind: PageKind;
}

export interface Backlink {
  block_id: string;
  block_text: string;
  source_page: PageMeta | null;
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
