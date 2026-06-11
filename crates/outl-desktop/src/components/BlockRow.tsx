import { For, Show, createEffect, createSignal } from "solid-js";

import type { BlockNode, PageMeta, TodoState } from "@outl/shared/api/types";
import {
  MarkdownInline,
  QuoteWrap,
  isBlockQuoted,
  splitQuote,
  stripQuoteFromTokens,
} from "@outl/shared/markdown";
import {
  applySuggestion,
  autoClosePair,
  autoDeletePair,
  detectRefContext,
} from "@outl/shared/autocomplete";
import { openRef, searchPages, searchPersons } from "@outl/shared/api/commands";
import { HighlightedCode } from "@outl/shared/highlight";
import { looksLikeOutline, utf16OffsetToCharOffset } from "@outl/shared/paste";

import { detectFence } from "@outl/shared/highlight";
import { appState, setAppState } from "../lib/store";

export interface BlockCallbacks {
  /** A textarea was double-clicked → enter edit mode on `id`. */
  onStartEdit: (id: string) => void;
  /** Commit the current edit. Called on Esc / blur / structural ops. */
  onCommit: (id: string, text: string) => Promise<void>;
  /** Enter pressed → commit + create a sibling below + focus it. */
  onEnter: (id: string, text: string) => Promise<void>;
  /** Tab pressed inside the textarea. */
  onIndent: (id: string) => Promise<void>;
  /** Shift-Tab pressed inside the textarea. */
  onOutdent: (id: string) => Promise<void>;
  /** Backspace on empty text → delete this block, jump cursor to prev. */
  onDeleteEmpty: (id: string) => Promise<void>;
  /** Checkbox click — flip TODO/DONE/none. */
  onToggleTodo: (id: string) => Promise<void>;
  /** Chevron click — fold / unfold. */
  onToggleCollapsed: (id: string, collapsed: boolean) => Promise<void>;
  /** External-clipboard paste with outline-like payload. */
  onPasteMarkdown: (id: string, caret: number, text: string) => Promise<void>;
  /** Run a fenced code block through `outl-exec`. */
  onRunCodeBlock: (id: string) => Promise<void>;
  /** Ref / tag click handlers (forwarded to MarkdownInline). */
  onRefClick: (target: string) => void;
  onTagClick: (tag: string) => void;
}

/** Wire format the block was stored as — TODO/DONE prefix included.
 *  We hand this verbatim to the textarea on edit so the user can
 *  *erase* the prefix to drop the TODO state and *type* `TODO ` /
 *  `DONE ` to add it. Mirrors how TUI and mobile show the raw text
 *  in their editors. */
function rawTextWithTodo(block: BlockNode): string {
  if (!block.todo) return block.text;
  return `${block.todo} ${block.text}`;
}


/**
 * One outline block. Renders read-only by default; flips to a
 * textarea editor when `editing === true`. Mouse and keyboard
 * interactions route through the shared `BlockCallbacks` so the
 * parent (`OutlineView`) owns the Tauri-side state mutations.
 */
export function BlockRow(props: {
  block: BlockNode;
  depth: number;
  editingId: string | null;
  cb: BlockCallbacks;
}) {
  const isEditing = () => props.editingId === props.block.id;
  // Draft mirrors the wire format (with TODO/DONE prefix). Same
  // shape the TUI buffer and the mobile editor use, so users can
  // type / erase the prefix to flip state.
  const [draft, setDraft] = createSignal<string>(rawTextWithTodo(props.block));

  // ── `[[page]]` ref autocomplete ──────────────────────────────────
  // While the caret sits inside an open `[[…]]`, we offer a popup of
  // matching pages. Detection + span replacement reuse the shared
  // `@outl/shared/autocomplete` helpers (same logic the TUI and mobile
  // run); page lookup reuses the `search_pages` command the Cmd+P
  // picker already calls. The popup is "open" iff `suggestions` is
  // non-empty.
  const [suggestions, setSuggestions] = createSignal<PageMeta[]>([]);
  const [suggestIndex, setSuggestIndex] = createSignal(0);
  // `query` last sent to the backend — skip redundant round-trips when
  // the caret moves without changing the in-ref text (mirrors mobile's
  // `lastQuery` guard). `null` means "not in a ref right now".
  let lastQuery: string | null = null;
  let searchToken = 0;

  let textareaRef: HTMLTextAreaElement | undefined;

  /** The page name we splice into `[[…]]`. Journals are anchored on
   *  their ISO slug (`2026-06-08`), regular pages on their title —
   *  same choice mobile's native chip strip makes. */
  function refReplacement(page: PageMeta): string {
    return page.kind === "journal" ? page.slug : page.title;
  }

  function closeSuggest() {
    lastQuery = null;
    if (suggestions().length > 0) setSuggestions([]);
    setSuggestIndex(0);
  }

  /**
   * Recompute the suggestion popup from the live textarea state.
   * Called after every keystroke / caret move while editing. When the
   * caret is inside an open `[[…]]` it (debounce-free, but de-duped on
   * `lastQuery`) fetches matching pages; otherwise it closes the popup.
   * Block refs (`((…))`) are intentionally ignored here — that's a
   * separate feature.
   */
  function refreshSuggest() {
    const ta = textareaRef;
    if (!ta) return closeSuggest();
    const ctx = detectRefContext(ta.value, ta.selectionStart ?? 0);
    // `page` → fuzzy over every page; `mention` → fuzzy over persons
    // only. Block-ref autocompletion is intentionally skipped here.
    if (!ctx || (ctx.kind !== "page" && ctx.kind !== "mention")) {
      return closeSuggest();
    }
    if (ctx.query === lastQuery) return;
    lastQuery = ctx.query;
    const token = ++searchToken;
    const fetcher = ctx.kind === "mention" ? searchPersons : searchPages;
    const wantedKind = ctx.kind;
    void fetcher(ctx.query)
      .then((list) => {
        // Drop stale responses: the user kept typing (newer token) or
        // moved the caret out of the ref while we were waiting.
        if (token !== searchToken) return;
        const cur = textareaRef
          ? detectRefContext(textareaRef.value, textareaRef.selectionStart ?? 0)
          : null;
        if (!cur || cur.kind !== wantedKind || cur.query !== ctx.query) return;
        // Create-new affordance for mentions — mirrors the TUI's
        // `candidates_for_mention`. When the query doesn't match any
        // existing person exactly (case-insensitive), append the
        // query itself as a synthetic candidate so the user can mint
        // a new person without leaving the popup. The page is
        // materialised lazily by `open_or_create_by_ref` when the
        // user opens the inserted `[[@<query>]]` ref.
        let finalList = list;
        if (
          wantedKind === "mention" &&
          ctx.query.trim().length > 0 &&
          !list.some(
            (p) => p.title.toLowerCase() === ctx.query.toLowerCase(),
          )
        ) {
          finalList = [
            ...list,
            {
              id: "",
              slug: ctx.query,
              title: ctx.query,
              kind: "page" as const,
              page_type: "person",
            },
          ];
        }
        setSuggestions(finalList);
        setSuggestIndex(0);
      })
      .catch(() => closeSuggest());
  }

  /** Accept `page`: replace the `[[…]]` (or `@…`) span with the
   *  chosen target, sync the draft signal + textarea, and park the
   *  caret after the closer. The shared `applySuggestion` decides
   *  whether to wrap the replacement in `[[…]]` (`page`) or `[[@…]]`
   *  (`mention`). */
  function acceptSuggestion(page: PageMeta) {
    const ta = textareaRef;
    if (!ta) return;
    const ctx = detectRefContext(ta.value, ta.selectionStart ?? 0);
    if (!ctx || (ctx.kind !== "page" && ctx.kind !== "mention")) {
      return closeSuggest();
    }
    // For mentions the page identity carries no `@` — pass the title
    // verbatim; `applySuggestion` prepends `@` on the link side.
    const replacement =
      ctx.kind === "mention" ? page.title : refReplacement(page);
    // Mention sugar: materialise the person page in the backend
    // (fire-and-forget) so the inserted `[[@title]]` link resolves
    // on subsequent loads — `open_or_create_by_ref` strips the `@`
    // and sets `type:: person` when the page doesn't exist yet, and
    // is idempotent when it does. Without this, accepting a
    // create-new candidate inserts the link but no page ever lands
    // on disk, and the next `@title` lookup misses it.
    if (ctx.kind === "mention") {
      void openRef(`@${page.title}`).catch((e) => {
        // Non-fatal — the link is already in the buffer; the user
        // can still navigate it later (which would create the page
        // then). Surface to the console so a backend regression
        // (e.g. permission denied on `pages/`) shows up in dev.
        console.warn("openRef for mention failed:", e);
      });
    }
    const completion = applySuggestion(ta.value, ctx, replacement);
    setDraft(completion.value);
    ta.value = completion.value;
    ta.setSelectionRange(completion.caret, completion.caret);
    closeSuggest();
    ta.focus();
  }

  function focusTextarea() {
    queueMicrotask(() => {
      textareaRef?.focus();
      autoSize();
    });
  }

  /**
   * Grow the textarea to fit its current content. Without this the
   * `rows={1}` textarea would clip multi-line blocks (code fences,
   * paragraphs typed with plain `Enter`). Cheap enough to call on
   * every draft change.
   */
  function autoSize() {
    const ta = textareaRef;
    if (!ta) return;
    ta.style.height = "auto";
    ta.style.height = `${ta.scrollHeight}px`;
  }

  // Re-run autoSize when the draft signal changes — Solid's
  // reactivity ties this to keystrokes the textarea fires.
  createEffect(() => {
    draft();
    if (isEditing()) autoSize();
  });

  /*
   * Focus the textarea whenever this block transitions into edit
   * mode — whether the user clicked into it or `<OutlineView />`
   * set `editingId` programmatically after `createBlock` (Cmd+Enter
   * fires the parent to create a sibling, and we want that sibling
   * editable without a second click). The microtask lets Solid
   * commit the `<Show>` swap to the textarea branch so
   * `textareaRef` is populated by the time we call focus().
   */
  createEffect(() => {
    if (isEditing()) {
      queueMicrotask(() => {
        textareaRef?.focus();
        autoSize();
      });
    }
  });

  async function commit() {
    // Draft is already wire format (prefix included if any). The
    // backend re-runs `split_todo` on it and re-projects the block
    // with the right `todo` state.
    const raw = draft();
    const wire = rawTextWithTodo(props.block);
    if (raw !== wire) {
      // `onCommit` flips `editingBlockId` to null after the Tauri
      // round-trip, so the row will re-render in read-only mode.
      await props.cb.onCommit(props.block.id, raw);
      return;
    }
    // Unchanged text — still need to leave Insert and flip the row
    // back to render mode. Without this an Esc on an unmodified
    // block leaves the textarea visible with raw markdown showing
    // (`**bold**` instead of the rendered **bold**), which is the
    // "Esc didn't exit edit mode" bug.
    setAppState("editingBlockId", null);
  }

  async function handleKeydown(e: KeyboardEvent) {
    // Ref-suggester navigation takes precedence while the popup is up:
    // arrows move the highlight, Enter/Tab accept, Esc closes the popup
    // (a *second* Esc then commits the block). stopPropagation keeps the
    // global shortcut dispatcher from also acting on these keys.
    const items = suggestions();
    if (items.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        e.stopPropagation();
        setSuggestIndex((i) => (i + 1) % items.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        e.stopPropagation();
        setSuggestIndex((i) => (i - 1 + items.length) % items.length);
        return;
      }
      if (
        e.key === "Enter" &&
        !e.metaKey &&
        !e.ctrlKey &&
        !e.shiftKey &&
        !e.altKey
      ) {
        e.preventDefault();
        e.stopPropagation();
        acceptSuggestion(items[suggestIndex()]);
        return;
      }
      if (e.key === "Tab") {
        e.preventDefault();
        e.stopPropagation();
        acceptSuggestion(items[suggestIndex()]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        closeSuggest();
        return;
      }
    }
    if (e.key === "Escape") {
      e.preventDefault();
      await commit();
      return;
    }
    // `Cmd+Enter` / `Cmd+Shift+Enter` / `Cmd+T` are all owned by the
    // global shortcut catalog (`outl-shortcuts::defaults`). We
    // deliberately do NOT intercept Enter+modifier here — otherwise
    // we'd race the catalog dispatcher and fire two actions at once
    // (commit-and-continue AND toggle-todo, the "Cmd+Shift+Enter
    // breaks the line" bug). Plain `Enter` still passes through to
    // the textarea so it inserts a real `\n` (multi-line blocks).
    if (e.key === "Tab") {
      e.preventDefault();
      await commit();
      if (e.shiftKey) await props.cb.onOutdent(props.block.id);
      else await props.cb.onIndent(props.block.id);
      return;
    }
    if (e.key === "Backspace" && draft().length === 0) {
      e.preventDefault();
      await props.cb.onDeleteEmpty(props.block.id);
      return;
    }
    // Bracket-pair completion ([[…]], ((…))).
    if (e.key === "[" || e.key === "(") {
      // Let the character land first; check on the next tick.
      queueMicrotask(() => {
        if (!textareaRef) return;
        const completion = autoClosePair(textareaRef.value, textareaRef.selectionStart ?? 0);
        if (completion) {
          setDraft(completion.value);
          textareaRef.value = completion.value;
          textareaRef.setSelectionRange(completion.caret, completion.caret);
        }
        // Whether or not the pair auto-closed, the caret may now be
        // inside a `[[…]]` — refresh the suggester (setting the value
        // programmatically above doesn't fire `onInput`).
        refreshSuggest();
      });
      return;
    }
    if (e.key === "Backspace") {
      const ta = textareaRef;
      if (ta) {
        const collapse = autoDeletePair(ta.value, ta.selectionStart ?? 0);
        if (collapse) {
          e.preventDefault();
          setDraft(collapse.value);
          ta.value = collapse.value;
          ta.setSelectionRange(collapse.caret, collapse.caret);
        }
      }
    }
    // Default: let the keystroke through; the bound input handler
    // updates the draft signal.
  }

  async function handlePaste(e: ClipboardEvent) {
    const ta = textareaRef;
    if (!ta) return;
    const text = e.clipboardData?.getData("text/plain") ?? "";
    if (!text || !looksLikeOutline(text)) return;
    e.preventDefault();
    const caretChars = utf16OffsetToCharOffset(ta.value, ta.selectionStart ?? 0);
    await props.cb.onPasteMarkdown(props.block.id, caretChars, text);
  }

  /** TODO/DONE state to render the bullet by. Edit mode is the
   *  **raw markdown view** — the `TODO `/`DONE ` prefix is literally
   *  visible in the textarea, so the bullet collapses to the neutral
   *  `•` to avoid showing the same state twice. Read mode is where
   *  the prefix is replaced by `▢` / `▣`. */
  function effectiveTodo(): TodoState | null {
    return isEditing() ? null : props.block.todo;
  }

  /** Bullet glyph + click action — folds TODO/DONE/none into a
   *  single visual primitive (TUI parity, `▢` / `▣` / `•`). */
  function bulletGlyph(): string {
    const t = effectiveTodo();
    if (t === "DONE") return "▣";
    if (t === "TODO") return "▢";
    return "•";
  }
  function bulletClass(): string {
    const t = effectiveTodo();
    if (t === "DONE") {
      return "text-(--color-outl-todo-done-fg)";
    }
    if (t === "TODO") {
      return "text-(--color-outl-todo-open-fg)";
    }
    return "text-(--color-outl-fg-dimmer)";
  }
  /** Body styling — DONE blocks render dim + struck-through (TUI
   *  uses theme.todo_done_body which is fg_dimmer + CROSSED_OUT). */
  function bodyClass(): string {
    return props.block.todo === "DONE" ? "line-through opacity-60" : "";
  }

  const isSelected = () => appState.selectedBlockId === props.block.id;
  const isInteractive = () => isEditing() || props.block.todo !== null;

  /** Outer row click — select without entering Insert. Lets the
   *  user mouse onto a block and then keyboard-nav from there
   *  (j/k), instead of clicking forcing an edit. The inner text
   *  `<div onClick>` still calls `onStartEdit` to enter edit mode,
   *  but the buttons (bullet, chevron) `stopPropagation()` so they
   *  don't both fire. */
  function selectRow(e: MouseEvent) {
    // The text-click handler stops propagation; this only fires
    // when the user clicked on row chrome (gutter, indent area).
    e.stopPropagation();
    setAppState("selectedBlockId", props.block.id);
  }

  return (
    <div>
      <div
        class={`outl-row group relative flex items-start rounded-sm py-[3px] pr-2 ${
          isSelected()
            ? "bg-(--color-outl-accent)/[0.06]"
            : "hover:bg-(--color-outl-bg-elev)/30"
        }`}
        data-selected={isSelected() ? "true" : "false"}
        data-editing={isEditing() ? "true" : "false"}
        onClick={selectRow}
      >
        {/* Vertical accent bar for the selected row — Bear-style
         * "this is where you are" indicator. Sits in the row's
         * gutter so it never reflows text. */}
        <Show when={isSelected()}>
          <span
            aria-hidden="true"
            class="absolute top-[4px] bottom-[4px] -left-[2px] w-[3px] rounded-full bg-(--color-outl-accent)"
          />
        </Show>

        {/*
         * Indent guides — one per ancestor level. Hairline only,
         * fades in when the row chrome is visible (hover / selected /
         * editing). Keeps the reading surface clean when at rest.
         */}
        <For each={Array.from({ length: props.depth })}>
          {() => (
            <span
              aria-hidden="true"
              class="outl-row-chrome ml-[10px] w-3 shrink-0 self-stretch border-l border-(--color-outl-border)/20"
            />
          )}
        </For>

        {/* Fold chevron — opacity 0 at rest (via `.outl-row-chrome`
         * styles), visible on hover / selection. */}
        <button
          type="button"
          class={`outl-row-chrome ml-[6px] mt-[6px] min-w-[16px] select-none whitespace-nowrap text-left text-[9px] font-mono ${
            props.block.children.length === 0 ? "" : "cursor-pointer"
          } opacity-60 hover:opacity-100 disabled:cursor-default`}
          disabled={props.block.children.length === 0}
          onClick={(e) => {
            e.stopPropagation();
            if (props.block.children.length === 0) return;
            void props.cb.onToggleCollapsed(props.block.id, !props.block.collapsed);
          }}
          aria-label={props.block.collapsed ? "Expand" : "Collapse"}
        >
          {props.block.children.length > 0 ? (
            props.block.collapsed ? (
              <>
                <span>▶</span>
                <span class="ml-1 text-(--color-outl-fg-dimmer)">
                  {props.block.children.length}
                </span>
              </>
            ) : (
              "▼"
            )
          ) : (
            ""
          )}
        </button>

        {/*
         * Quote chrome wraps **bullet + body** via the shared
         * `<QuoteWrap />` so the left border lands *before* the
         * checkbox — TUI parity, where `│ ☐ body` reads as "this is
         * a quoted task" instead of "a task whose body happens to be
         * a quote". When the block isn't quoted the wrapper degrades
         * to a plain flex container, so non-quoted rows render
         * byte-identical.
         */}
        {(() => {
          const bullet = (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                void props.cb.onToggleTodo(props.block.id);
              }}
              {...(isInteractive() ? { "data-todo": "true" } : {})}
              class={`outl-row-chrome mt-[5px] mr-2 w-3 shrink-0 cursor-pointer select-none text-center text-[13px] leading-none transition-opacity hover:opacity-70 ${bulletClass()}`}
              title={
                props.block.todo === "DONE"
                  ? "Click to uncheck"
                  : props.block.todo === "TODO"
                    ? "Click to mark done"
                    : "Click to mark as TODO"
              }
              aria-label={
                props.block.todo === "DONE"
                  ? "Mark not done"
                  : props.block.todo === "TODO"
                    ? "Mark done"
                    : "Mark as TODO"
              }
            >
              {bulletGlyph()}
            </button>
          );
          const body = (
            <div class={`min-w-0 flex-1 leading-snug ${bodyClass()}`}>
              {(() => {
                const fence = !isEditing() ? detectFence(props.block.text) : null;
                return (
                  <Show
                    when={isEditing()}
                    fallback={
                      fence ? (
                        <CodeFenceView
                          language={fence.language}
                          body={fence.body}
                          onEdit={() => {
                            setDraft(rawTextWithTodo(props.block));
                            props.cb.onStartEdit(props.block.id);
                            focusTextarea();
                          }}
                          onRun={() => props.cb.onRunCodeBlock(props.block.id)}
                        />
                      ) : (
                        (() => {
                          // The chrome lives on the wrapper a level
                          // up — here we just strip the `> ` from the
                          // tokens so the marker doesn't double-paint.
                          const split = splitQuote(props.block.text);
                          const renderedTokens = split.quoted
                            ? stripQuoteFromTokens(props.block.tokens)
                            : props.block.tokens;
                          const hasContent = split.quoted
                            ? split.body.length > 0
                            : Boolean(props.block.text);
                          return (
                            <div
                              class="cursor-text whitespace-pre-wrap break-words"
                              onClick={() => {
                                setDraft(rawTextWithTodo(props.block));
                                props.cb.onStartEdit(props.block.id);
                                focusTextarea();
                              }}
                            >
                              <Show
                                when={renderedTokens && renderedTokens.length > 0}
                                fallback={
                                  <span class={!hasContent ? "opacity-30" : ""}>
                                    {hasContent
                                      ? (split.quoted ? split.body : props.block.text)
                                      : "Click to add text…"}
                                  </span>
                                }
                              >
                                <MarkdownInline
                                  tokens={renderedTokens}
                                  variant="inline"
                                  onRefClick={props.cb.onRefClick}
                                  onTagClick={props.cb.onTagClick}
                                />
                              </Show>
                            </div>
                          );
                        })()
                      )
                    }
                  >
            <div class="relative">
              <textarea
                ref={textareaRef}
                value={draft()}
                autofocus
                rows={1}
                spellcheck={false}
                data-block-id={props.block.id}
                class="w-full resize-none overflow-hidden bg-transparent text-current outline-none"
                onInput={(e) => {
                  setDraft(e.currentTarget.value);
                  refreshSuggest();
                }}
                onSelect={() => refreshSuggest()}
                onBlur={() => void commit()}
                onKeyDown={handleKeydown}
                onPaste={handlePaste}
              />
              <RefSuggestPopup
                items={suggestions()}
                activeIndex={suggestIndex()}
                onHover={setSuggestIndex}
                onPick={acceptSuggestion}
              />
            </div>
          </Show>
            );
          })()}
        </div>
          );
          // Tailwind classes are passed as **string literals** so the
          // JIT discovers them at build time — the shared
          // `<QuoteWrap />` just composes the conditional `class=`.
          return (
            <QuoteWrap
              quoted={isBlockQuoted(props.block.text)}
              baseClass="flex min-w-0 flex-1 items-start"
              chromeClass="rounded-r-md border-l-2 border-(--color-outl-fg-dimmer)/50 bg-(--color-outl-fg-dimmer)/[0.06] pl-2"
            >
              {bullet}
              {body}
            </QuoteWrap>
          );
        })()}
      </div>

      <Show when={!props.block.collapsed && props.block.children.length > 0}>
        <For each={props.block.children}>
          {(child) => (
            <BlockRow
              block={child}
              depth={props.depth + 1}
              editingId={props.editingId}
              cb={props.cb}
            />
          )}
        </For>
      </Show>
    </div>
  );
}

/**
 * Floating page-suggestion list shown while the caret is inside an
 * open `[[…]]`. Anchored just below the block's textarea.
 *
 * Selection uses `onMouseDown` + `preventDefault` (not `onClick`): a
 * plain click would blur the textarea first, firing its `onBlur`
 * commit and tearing down edit mode before the pick registered.
 * Preventing the default on mousedown keeps focus in the textarea so
 * `acceptSuggestion` can splice the value and re-park the caret.
 */
function RefSuggestPopup(props: {
  items: PageMeta[];
  activeIndex: number;
  onHover: (i: number) => void;
  onPick: (page: PageMeta) => void;
}) {
  return (
    <Show when={props.items.length > 0}>
      <ul
        class="absolute top-full left-0 z-30 mt-1 max-h-56 w-72 overflow-y-auto rounded-md border border-(--color-outl-border) bg-(--color-outl-bg-elev) py-1 text-[13px] shadow-lg"
        role="listbox"
      >
        <For each={props.items}>
          {(page, i) => (
            <li role="option" aria-selected={i() === props.activeIndex}>
              <button
                type="button"
                onMouseDown={(e) => {
                  e.preventDefault();
                  props.onPick(page);
                }}
                onMouseEnter={() => props.onHover(i())}
                class={`flex w-full items-center gap-1.5 px-2 py-1 text-left ${
                  i() === props.activeIndex
                    ? "bg-(--color-outl-accent) text-(--color-outl-bg)"
                    : "hover:bg-(--color-outl-bg)/50"
                }`}
              >
                <span aria-hidden="true" class="shrink-0 opacity-70">
                  {page.icon || (page.kind === "journal" ? "📅" : "📄")}
                </span>
                <span class="truncate">
                  {page.kind === "journal" ? page.slug : page.title}
                </span>
              </button>
            </li>
          )}
        </For>
      </ul>
    </Show>
  );
}

/**
 * Rendered view of a `\`\`\`lang\n…\n\`\`\`` block. Shows the source
 * monospaced, a tiny language chip, and a Run button. Clicking the
 * source body kicks the editor in (so the user can edit the fence
 * just like any other block).
 */
function CodeFenceView(props: {
  language: string;
  body: string;
  onEdit: () => void;
  onRun: () => Promise<void>;
}) {
  const [busy, setBusy] = createSignal(false);

  async function run() {
    setBusy(true);
    try {
      await props.onRun();
    } finally {
      setBusy(false);
    }
  }

  return (
    <div class="rounded-md border border-(--color-outl-fg)/10 bg-(--color-outl-bg-elev)/60">
      <div class="flex items-center justify-between border-b border-(--color-outl-fg)/10 px-2 py-1">
        <span class="font-mono text-[10px] uppercase opacity-60">{props.language}</span>
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            void run();
          }}
          disabled={busy()}
          class="rounded bg-(--color-outl-fg)/10 px-2 py-0.5 text-[11px] hover:bg-(--color-outl-fg)/20 disabled:opacity-50"
        >
          {busy() ? "Running…" : "▶ Run"}
        </button>
      </div>
      <div onClick={props.onEdit} class="cursor-text">
        <HighlightedCode language={props.language} code={props.body || " "} />
      </div>
    </div>
  );
}
