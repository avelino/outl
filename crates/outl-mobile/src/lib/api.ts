import { invoke } from "@tauri-apps/api/core";

export type TodoState = "TODO" | "DONE";
export type PageKind = "page" | "journal";

/**
 * Pre-tokenized inline markdown coming from the Rust backend
 * (`outl_md::tokenize_owned`). The mobile renderer maps each variant
 * to JSX in `lib/markdown.tsx::MarkdownInline`. There is no parallel
 * TS tokenizer — `outl_md::inline::tokenize` is the single source of
 * truth for inline syntax across every client. Adding a token in
 * Rust means extending this union and the renderer switch in the
 * same change.
 */
export type InlineToken =
  | { kind: "plain"; value: string }
  | { kind: "bold"; value: string }
  | { kind: "italic"; value: string }
  | { kind: "strike"; value: string }
  | { kind: "code"; value: string }
  | { kind: "link"; value: string; href: string }
  | { kind: "ref"; value: string }
  | { kind: "tag"; value: string }
  | { kind: "blockref"; value: string }
  | { kind: "embed"; value: string };

export interface BlockNode {
  id: string;
  text: string;
  todo: TodoState | null;
  /**
   * Inline markdown tokens for `text` (no TODO/DONE prefix). Backend
   * pre-tokenizes via `outl_md::tokenize_owned` so the renderer
   * doesn't run a second tokenizer in JS. See {@link InlineToken}.
   */
  tokens: InlineToken[];
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
  todo: TodoState | null;
  source_page: PageMeta | null;
  /**
   * Source block as a self-contained outline subtree (text, tokens,
   * children, properties). Mirrors what `read_page_view_with_workspace`
   * would return for the same block. The mobile renderer reads
   * `source_block.tokens` for the inline markdown; the raw text
   * lives at `source_block.text` if a caller ever needs it.
   *
   * Note: the Rust `Backlink` struct also carries `block_text` for
   * the CLI/MCP JSON envelope (`outl page rename` returns it under
   * `affected_refs`). It's intentionally omitted from this TS
   * interface because mobile never reads it — Tauri still ships the
   * field over the wire; JS just ignores it.
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
