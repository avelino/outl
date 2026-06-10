import { For, Show } from "solid-js";

import { openRef } from "@outl/shared/api/commands";
import { MarkdownInline } from "@outl/shared/markdown";
import type { Backlink } from "@outl/shared/api/types";

import { appState, setAppState } from "../lib/store";

/**
 * Inline backlinks section — rendered **below** the outline, not
 * as a side panel.
 *
 * Mirrors the TUI's `view::backlinks::render_backlinks_inline`:
 * a soft horizontal rule + a `Backlinks · N ref(s)` header, then
 * each referencing source page contributes a header (icon + title)
 * and one row per referencing block. Multiple backlinks from the
 * same source page collapse under one header — same UX as
 * `outl-tui` so a user moving between clients sees the same
 * structure.
 *
 * Hidden when:
 *
 * - `appState.backlinksOpen === false` (toggled with
 *   `Cmd/Ctrl+Shift+B`), or
 * - there are no backlinks for the current page (empty section
 *   doesn't earn its space).
 *
 * Each backlink row is **navigable**: vim `j/k` extends past the
 * outline's last block into this section (cursor lives at
 * `appState.selectedBacklinkBlockId`), and `Enter` opens the
 * source page positioned on the referencing block. Mouse click
 * does the same — both flows funnel through `openBacklink` so the
 * cursor lands at the same place no matter how the user
 * triggered the open.
 */
export function InlineBacklinks() {
  async function openBacklink(link: Backlink) {
    const target = link.source_page?.slug;
    if (!target) return;
    try {
      const view = await openRef(target);
      setAppState({
        page: view.page,
        outline: view.outline,
        backlinks: view.backlinks,
      });
      // Position cursor on the source block (the one we just came
      // from). Reset backlink cursor so j/k keep working in the
      // freshly-opened outline.
      setAppState("selectedBacklinkBlockId", null);
      setAppState("selectedBlockId", link.block_id);
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    }
  }

  /** Open the source page from a group header (jumps to the page,
   *  no per-block positioning). */
  async function jumpTo(sourceSlug: string | undefined) {
    if (!sourceSlug) return;
    try {
      const view = await openRef(sourceSlug);
      setAppState({
        page: view.page,
        outline: view.outline,
        backlinks: view.backlinks,
      });
      setAppState("selectedBacklinkBlockId", null);
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    }
  }

  /**
   * Group backlinks by their source page. `null` source (orphan
   * blocks with no enclosing page) collapse under a synthetic
   * `(orphan)` header.
   */
  function groupedBySource(): Array<{ key: string; title: string; icon: string; entries: Backlink[] }> {
    const groups = new Map<
      string,
      { key: string; title: string; icon: string; entries: Backlink[] }
    >();
    for (const b of appState.backlinks) {
      const key = b.source_page?.slug ?? "__orphan__";
      const title = b.source_page?.title ?? "(orphan)";
      const fallback = b.source_page?.kind === "journal" ? "📅" : "📄";
      const icon = b.source_page?.icon || fallback;
      const existing = groups.get(key);
      if (existing) existing.entries.push(b);
      else groups.set(key, { key, title, icon, entries: [b] });
    }
    return [...groups.values()];
  }

  return (
    <Show when={appState.backlinksOpen && appState.backlinks.length > 0}>
      <section class="mt-6">
        {/* Soft full-width rule — mirrors the TUI's `─` separator. */}
        <div class="border-t border-(--color-outl-border) opacity-60" />

        <header class="mt-3 mb-2 text-xs font-semibold uppercase tracking-wide opacity-60">
          Backlinks · {appState.backlinks.length} ref{appState.backlinks.length === 1 ? "" : "s"}
        </header>

        <div class="space-y-4">
          <For each={groupedBySource()}>
            {(group) => (
              <div>
                <button
                  type="button"
                  onClick={() => void jumpTo(group.key === "__orphan__" ? undefined : group.key)}
                  class="flex w-full items-baseline gap-2 rounded px-1 py-0.5 text-left text-sm font-semibold hover:bg-(--color-outl-fg)/5"
                >
                  <span aria-hidden="true">{group.icon}</span>
                  <span>{group.title}</span>
                  <span class="text-xs opacity-50">{group.entries.length}</span>
                </button>

                <ul class="mt-1 space-y-1 pl-6">
                  <For each={group.entries}>
                    {(link) => {
                      const selected = () =>
                        appState.selectedBacklinkBlockId === link.block_id;
                      return (
                        <li
                          // The selected state mirrors the outline's
                          // BlockRow highlight: 3px accent bar on the
                          // left + 6% background. Same visual
                          // language so j/k feels continuous from
                          // outline into backlinks.
                          class={
                            selected()
                              ? "relative -ml-3 rounded bg-(--color-outl-accent)/6 pl-3 before:absolute before:left-0 before:top-1 before:bottom-1 before:w-[3px] before:rounded-r before:bg-(--color-outl-accent)"
                              : ""
                          }
                        >
                          <button
                            type="button"
                            onClick={() => void openBacklink(link)}
                            onMouseEnter={() =>
                              setAppState(
                                "selectedBacklinkBlockId",
                                link.block_id,
                              )
                            }
                            class="block w-full rounded px-1 py-0.5 text-left text-sm leading-snug opacity-90 hover:bg-(--color-outl-fg)/5 hover:opacity-100"
                          >
                            <MarkdownInline
                              tokens={link.source_block.tokens}
                              variant="inline"
                            />
                          </button>
                        </li>
                      );
                    }}
                  </For>
                </ul>
              </div>
            )}
          </For>
        </div>
      </section>
    </Show>
  );
}
