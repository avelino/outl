import { For, Show, createEffect, createSignal } from "solid-js";

import type { BlockNode, TodoState } from "@outl/shared/api/types";
import { MarkdownInline } from "@outl/shared/markdown";
import { autoClosePair, autoDeletePair } from "@outl/shared/autocomplete";
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

  let textareaRef: HTMLTextAreaElement | undefined;

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
         * Unified bullet — `•` / `▢` / `▣` depending on TODO state.
         * Click toggles cycle (None → TODO → DONE → None) via the
         * `toggle_todo` Tauri command. TUI parity: theme colors
         * todo_open / todo_done drive the glyph color, body styling
         * picks up strikethrough for DONE.
         */}
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
                    <div
                      class="cursor-text whitespace-pre-wrap break-words"
                      onClick={() => {
                        setDraft(rawTextWithTodo(props.block));
                        props.cb.onStartEdit(props.block.id);
                        focusTextarea();
                      }}
                    >
                      <Show
                        when={props.block.tokens && props.block.tokens.length > 0}
                        fallback={
                          <span class={!props.block.text ? "opacity-30" : ""}>
                            {props.block.text || "Click to add text…"}
                          </span>
                        }
                      >
                        <MarkdownInline
                          tokens={props.block.tokens}
                          variant="inline"
                          onRefClick={props.cb.onRefClick}
                          onTagClick={props.cb.onTagClick}
                        />
                      </Show>
                    </div>
                  )
                }
              >
            <textarea
              ref={textareaRef}
              value={draft()}
              autofocus
              rows={1}
              spellcheck={false}
              data-block-id={props.block.id}
              class="w-full resize-none overflow-hidden bg-transparent text-current outline-none"
              onInput={(e) => setDraft(e.currentTarget.value)}
              onBlur={() => void commit()}
              onKeyDown={handleKeydown}
              onPaste={handlePaste}
            />
          </Show>
            );
          })()}
        </div>
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
    <div class="rounded-md border border-white/10 bg-black/30">
      <div class="flex items-center justify-between border-b border-white/10 px-2 py-1">
        <span class="font-mono text-[10px] uppercase opacity-60">{props.language}</span>
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            void run();
          }}
          disabled={busy()}
          class="rounded bg-white/10 px-2 py-0.5 text-[11px] hover:bg-white/20 disabled:opacity-50"
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
