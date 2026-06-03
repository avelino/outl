import { For, JSX, Show, onCleanup, onMount } from "solid-js";
import { BlockNode } from "../lib/api";
import { MarkdownInline } from "../lib/markdown";
import { autoClosePair, autoDeletePair } from "../lib/autocomplete";
import { haptic } from "../lib/haptics";
import { rawTextWithTodo } from "../lib/outline";
import { looksLikeOutline, utf16OffsetToCharOffset } from "../lib/paste";
import { parkCaret } from "../lib/textarea";
import { isDesktop } from "../lib/platform";
import { SwipeRow } from "./SwipeRow";

interface BlockRowProps {
  block: BlockNode;
  depth: number;
  editingId: string | null;
  /**
   * Lazy accessor for the draft signal. Receiving a getter instead
   * of `string` means only the block that's *actually* in edit
   * subscribes to `draft()` changes — the other 199 rows in a
   * 200-block outline ignore each keystroke. Without this, typing
   * one character re-runs a reactive effect in every BlockRow.
   */
  draftText: () => string;
  onStartEdit: (id: string, initialText: string) => void;
  onDraftChange: (text: string) => void;
  onCommitEdit: () => void;
  onToggleTodo: (id: string) => void;
  onDelete: (id: string) => void;
  /**
   * Desktop-only: delete an empty block via Backspace and move focus
   * to the end of the previous block. Distinct from `onDelete` (the
   * swipe gesture), which deletes without re-focusing a neighbour.
   */
  onDeleteEmpty: (id: string) => void;
  onIndent: (id: string) => void;
  onOutdent: (id: string) => void;
  onCreateAfter: (id: string) => void;
  /**
   * Flip the block's collapsed flag. Implemented by the parent so
   * the persistence path (Tauri → sidecar) is shared with every
   * other block-mutating action and the parent can re-render with
   * the fresh `PageView`.
   */
  onToggleCollapse: (id: string, next: boolean) => void;
  onRefClick?: (target: string) => void;
  onTagClick?: (tag: string) => void;
  onTextareaMount?: (el: HTMLTextAreaElement) => void;
  /**
   * Called when the user pastes outline-shaped markdown into this
   * block's textarea. The frontend has already detected via
   * `looksLikeOutline` that the clipboard payload deserves a
   * full-on tree conversion; the parent wires this up to the Tauri
   * `paste_markdown_at` command and refreshes the page on resolve.
   * `caret` is a `char` offset into the host block's text.
   */
  onPasteMarkdown?: (blockId: string, caret: number, text: string) => void;
}

/**
 * One row of the outline. Handles read-mode (rendered markdown) and
 * edit-mode (textarea), TODO checkbox, swipe-to-delete, long-press
 * to toggle TODO, and renders children recursively.
 */
const INDENT_PX = 22;

export function BlockRow(props: BlockRowProps): JSX.Element {
  const isEditing = () => props.editingId === props.block.id;
  const hasChildren = () => props.block.children.length > 0;

  return (
    <div class="relative">
      <SwipeRow
        leftActionLabel="Delete"
        onSwipeLeft={() => {
          haptic("warning");
          props.onDelete(props.block.id);
        }}
      >
        <BlockBody
          block={props.block}
          editing={isEditing()}
          draftText={props.draftText}
          depth={props.depth}
          hasChildren={hasChildren()}
          onToggleCollapse={() => {
            haptic("light");
            props.onToggleCollapse(props.block.id, !props.block.collapsed);
          }}
          onStartEdit={() =>
            props.onStartEdit(props.block.id, rawTextWithTodo(props.block))
          }
          onDraftChange={props.onDraftChange}
          onCommitEdit={props.onCommitEdit}
          onToggleTodo={() => {
            haptic("light");
            props.onToggleTodo(props.block.id);
          }}
          onLongPress={() => {
            // Distinct haptic so the user can tell something happened
            // even when they're not looking at the checkbox: "success"
            // (3-pulse) when we're about to *create* a TODO on a plain
            // block, plain "medium" when we're cycling an existing
            // TODO ↔ DONE ↔ none.
            haptic(props.block.todo === null ? "success" : "medium");
            props.onToggleTodo(props.block.id);
          }}
          onCreateAfter={() => props.onCreateAfter(props.block.id)}
          onIndent={() => props.onIndent(props.block.id)}
          onOutdent={() => props.onOutdent(props.block.id)}
          onDeleteEmpty={() => props.onDeleteEmpty(props.block.id)}
          onRefClick={props.onRefClick}
          onTagClick={props.onTagClick}
          onTextareaMount={props.onTextareaMount}
          onPasteMarkdown={
            props.onPasteMarkdown
              ? (caret, text) =>
                  props.onPasteMarkdown!(props.block.id, caret, text)
              : undefined
          }
        />
      </SwipeRow>

      <Show when={hasChildren() && !props.block.collapsed}>
        <div class="relative">
          {/* Guide line connecting parent bullet to children */}
          <span
            aria-hidden="true"
            class="absolute top-0 bottom-0 w-px bg-(--color-ios-divider)/35 dark:bg-(--color-iosd-divider)/30"
            style={{ left: `${16 + props.depth * INDENT_PX + 5}px` }}
          />
          <For each={props.block.children}>
            {(child) => (
              <BlockRow
                block={child}
                depth={props.depth + 1}
                editingId={props.editingId}
                draftText={props.draftText}
                onStartEdit={props.onStartEdit}
                onDraftChange={props.onDraftChange}
                onCommitEdit={props.onCommitEdit}
                onToggleTodo={props.onToggleTodo}
                onDelete={props.onDelete}
                onDeleteEmpty={props.onDeleteEmpty}
                onIndent={props.onIndent}
                onOutdent={props.onOutdent}
                onCreateAfter={props.onCreateAfter}
                onToggleCollapse={props.onToggleCollapse}
                onRefClick={props.onRefClick}
                onTagClick={props.onTagClick}
                onTextareaMount={props.onTextareaMount}
              />
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}

function BlockBody(props: {
  block: BlockNode;
  editing: boolean;
  /** Lazy accessor — only read inside the edit-mode branch so non-
   *  editing rows don't subscribe to `draft()`. */
  draftText: () => string;
  depth: number;
  /** `true` when the block has at least one child. Drives the
   *  triangle marker (▶/▼). */
  hasChildren: boolean;
  /** Flip `block.collapsed`. No-op visually when `hasChildren` is
   *  `false`; the tap target hides itself in that case. */
  onToggleCollapse: () => void;
  onStartEdit: () => void;
  onDraftChange: (text: string) => void;
  onCommitEdit: () => void;
  onToggleTodo: () => void;
  onLongPress: () => void;
  onCreateAfter: () => void;
  onIndent: () => void;
  onOutdent: () => void;
  onDeleteEmpty: () => void;
  onRefClick?: (target: string) => void;
  onTagClick?: (tag: string) => void;
  onTextareaMount?: (el: HTMLTextAreaElement) => void;
  /** See `BlockRowProps.onPasteMarkdown`. The parent has already
   *  injected `blockId`; this variant gets the caret + text. */
  onPasteMarkdown?: (caret: number, text: string) => void;
}) {
  let longPressTimer: number | undefined;
  let downX = 0;
  let downY = 0;
  let didLongPress = false;

  /**
   * True when the gesture started inside an interactive child — a
   * page ref (`[[…]]`), tag (`#…`), inline code, link, or any
   * `button`/`[role=button]`. Those need to handle their own taps;
   * we bail before arming the long-press timer or starting an edit
   * so the user actually navigates to the ref instead of opening
   * the textarea on top of it.
   */
  function pressedInteractive(e: PointerEvent): boolean {
    const target = e.target as HTMLElement | null;
    return !!target?.closest(
      "a,button,[role='button'],code,textarea,input",
    );
  }

  function onPointerDown(e: PointerEvent) {
    if (props.editing) return;
    if (pressedInteractive(e)) return;
    downX = e.clientX;
    downY = e.clientY;
    didLongPress = false;
    longPressTimer = window.setTimeout(() => {
      didLongPress = true;
      props.onLongPress();
    }, 450);
  }
  function onPointerMove(e: PointerEvent) {
    if (longPressTimer === undefined) return;
    if (
      Math.abs(e.clientX - downX) > 8 ||
      Math.abs(e.clientY - downY) > 8
    ) {
      window.clearTimeout(longPressTimer);
      longPressTimer = undefined;
    }
  }
  function onPointerUp() {
    if (longPressTimer !== undefined) {
      window.clearTimeout(longPressTimer);
      longPressTimer = undefined;
    }
  }
  function onClick(e: MouseEvent) {
    if (didLongPress) {
      didLongPress = false;
      return;
    }
    // A tap that landed inside an interactive child has already been
    // handled by that child (`stopPropagation` on the ref/tag span,
    // the checkbox button, etc). Don't fall through into "start
    // edit" — that's how tap-on-ref kept opening the editor.
    if ((e.target as HTMLElement | null)?.closest(
      "a,button,[role='button'],code,textarea,input",
    )) {
      return;
    }
    if (!props.editing) props.onStartEdit();
  }

  onCleanup(() => {
    if (longPressTimer !== undefined) window.clearTimeout(longPressTimer);
  });

  const padLeft = () => 16 + props.depth * INDENT_PX;

  return (
    <div
      class="group flex items-start gap-2.5 py-[5px] pr-4"
      style={{ "padding-left": `${padLeft()}px` }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onClick={onClick}
    >
      <CollapseTriangle
        visible={props.hasChildren}
        collapsed={props.block.collapsed}
        onToggle={() => {
          props.onToggleCollapse();
        }}
      />

      <BulletOrCheckbox
        todo={props.editing ? null : props.block.todo}
        onToggle={() => {
          props.onToggleTodo();
        }}
      />

      <div class="min-w-0 flex-1">
        <Show
          when={props.editing}
          fallback={
            <p
              class="break-words text-[17px] leading-[1.42]"
              classList={{
                "text-(--color-ios-text-tertiary) line-through dark:text-(--color-iosd-text-tertiary)":
                  props.block.todo === "DONE",
              }}
            >
              <Show
                when={props.block.text.length > 0}
                fallback={
                  <span class="italic text-(--color-ios-text-tertiary) dark:text-(--color-iosd-text-tertiary)">
                    Empty block
                  </span>
                }
              >
                <MarkdownInline
                  text={props.block.text}
                  onRefClick={props.onRefClick}
                  onTagClick={props.onTagClick}
                />
              </Show>
            </p>
          }
        >
          <EditableTextarea
            value={props.draftText()}
            onInput={props.onDraftChange}
            onBlur={props.onCommitEdit}
            onMount={props.onTextareaMount}
            onShiftEnter={props.onCreateAfter}
            onIndent={props.onIndent}
            onOutdent={props.onOutdent}
            onBackspaceEmpty={props.onDeleteEmpty}
            onPaste={props.onPasteMarkdown}
          />
        </Show>
      </div>
    </div>
  );
}

function CollapseTriangle(props: {
  visible: boolean;
  collapsed: boolean;
  onToggle: () => void;
}) {
  // Always reserve the slot — even on leaves — so the bullet column
  // stays put regardless of whether a sibling has children. Width
  // matches the bullet (`w-[26px]`).
  return (
    <Show
      when={props.visible}
      fallback={<span aria-hidden="true" class="w-[18px] shrink-0" />}
    >
      <button
        type="button"
        aria-label={props.collapsed ? "Expand block" : "Collapse block"}
        aria-expanded={!props.collapsed}
        onClick={(e) => {
          e.stopPropagation();
          props.onToggle();
        }}
        class="relative z-10 -my-1.5 -ml-1 flex h-[30px] w-[18px] shrink-0 items-center justify-center text-(--color-ios-text-tertiary) dark:text-(--color-iosd-text-tertiary)"
      >
        <span aria-hidden="true" class="text-[10px] leading-none">
          {props.collapsed ? "▶" : "▼"}
        </span>
      </button>
    </Show>
  );
}

function BulletOrCheckbox(props: {
  todo: BlockNode["todo"];
  onToggle: () => void;
}) {
  // Apple HIG: minimum tap target is 44×44. We hit ~36×30 here so we
  // stay visually compact in dense outlines but no longer demand
  // pixel-perfect taps. The visual dot/checkbox keeps its old size
  // — the surrounding `<button>` is what grows.
  return (
    <Show
      when={props.todo !== null}
      fallback={
        <button
          type="button"
          aria-label="Mark as TODO"
          onClick={(e) => {
            e.stopPropagation();
            props.onToggle();
          }}
          class="group/bullet relative z-10 -my-1.5 -ml-2 flex h-[30px] w-[26px] shrink-0 items-center justify-center"
        >
          <span
            aria-hidden="true"
            class="h-1.5 w-1.5 rounded-full bg-(--color-ios-text-tertiary) transition-transform group-active/bullet:scale-150 dark:bg-(--color-iosd-text-tertiary)"
          />
        </button>
      }
    >
      <button
        type="button"
        aria-label={props.todo === "DONE" ? "Mark as TODO" : "Mark as done"}
        onClick={(e) => {
          e.stopPropagation();
          props.onToggle();
        }}
        class="relative z-10 -my-1.5 -ml-1 flex h-[30px] w-[30px] shrink-0 items-center justify-center"
      >
        <span
          class="flex h-[20px] w-[20px] items-center justify-center rounded-full border-[1.5px] transition-colors"
          classList={{
            "border-(--color-ios-accent) bg-(--color-ios-accent) dark:border-(--color-iosd-accent) dark:bg-(--color-iosd-accent)":
              props.todo === "DONE",
            "border-(--color-ios-text-secondary) bg-transparent dark:border-(--color-iosd-text-secondary)":
              props.todo !== "DONE",
          }}
        >
          <Show when={props.todo === "DONE"}>
            <svg
              width="12"
              height="12"
              viewBox="0 0 24 24"
              fill="none"
              stroke="white"
              stroke-width="3.5"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M5 12l4 4 10-10" />
            </svg>
          </Show>
        </span>
      </button>
    </Show>
  );
}

function EditableTextarea(props: {
  value: string;
  onInput: (v: string) => void;
  onBlur: () => void;
  onMount?: (el: HTMLTextAreaElement) => void;
  /** Desktop-only keyboard shortcuts. No-ops on touch devices, where
   *  these chords don't exist (block creation/indent live in the
   *  native toolbar). See `lib/platform.ts`. */
  onShiftEnter: () => void;
  onIndent: () => void;
  onOutdent: () => void;
  onBackspaceEmpty: () => void;
  /**
   * Called when the user pastes outline-shaped markdown. Receives
   * the caret position (in chars) and the verbatim clipboard text.
   * The parent is responsible for `preventDefault` semantics on the
   * paste event — we already do that here when this is set.
   */
  onPaste?: (caret: number, text: string) => void;
}) {
  let ref!: HTMLTextAreaElement;
  let resizeRaf = 0;

  // Reading `ref.scrollHeight` after writing `ref.style.height` forces
  // a synchronous layout. Doing that on every keystroke makes typing
  // feel sluggish on long pages — coalescing into a single
  // requestAnimationFrame keeps the work to once per frame.
  function autoResize() {
    if (!ref) return;
    if (resizeRaf) return;
    resizeRaf = window.requestAnimationFrame(() => {
      resizeRaf = 0;
      if (!ref) return;
      ref.style.height = "auto";
      ref.style.height = `${ref.scrollHeight}px`;
    });
  }

  onCleanup(() => {
    if (resizeRaf) window.cancelAnimationFrame(resizeRaf);
  });

  onMount(() => {
    autoResize();
    ref.focus();
    // Place cursor at end.
    const len = ref.value.length;
    ref.setSelectionRange(len, len);
    props.onMount?.(ref);
  });

  return (
    <textarea
      ref={ref}
      class="block w-full resize-none border-0 bg-transparent p-0 text-[17px] leading-snug outline-none"
      rows="1"
      value={props.value}
      // iOS Smart Punctuation silently rewrites `--` → `–`,
      // `...` → `…`, `"foo"` → `“foo”`. Disastrous for a markdown
      // outliner where code snippets, CLI commands and any
      // syntax-sensitive text gets corrupted *after* the user
      // typed. We turn the lot off here. `autocomplete="off"` is
      // belt-and-braces — WKWebView mostly ignores it on textareas
      // but it does suppress the proactive suggestion bar in some
      // iOS versions.
      autocorrect="off"
      autocapitalize="off"
      autocomplete="off"
      spellcheck={false}
      onKeyDown={(e) => {
        // Desktop-only physical-keyboard shortcuts. On touch devices
        // these chords either don't exist or are handled by the native
        // toolbar, so we leave the textarea's default behaviour intact.
        if (isDesktop()) {
          const ta = e.currentTarget;
          // Tab / Shift+Tab → indent / outdent. Skip when a non-Shift
          // modifier is held: Ctrl/Cmd/Alt+Tab are window- and
          // tab-switching chords and trapping the bare focus-traversal
          // key without an escape hatch is an accessibility regression.
          // Shift is part of our own outdent chord, so it's allowed.
          if (e.key === "Tab" && !e.ctrlKey && !e.metaKey && !e.altKey) {
            e.preventDefault();
            if (e.shiftKey) props.onOutdent();
            else props.onIndent();
            return;
          }
          // Shift+Enter → commit current draft and create a sibling
          // block below. Plain Enter falls through to the textarea's
          // native newline (soft line break inside the block).
          if (e.key === "Enter" && e.shiftKey) {
            e.preventDefault();
            props.onShiftEnter();
            return;
          }
          // Backspace on an empty block → delete it and move the caret
          // to the end of the previous block. Only when the block is
          // truly empty and there's no selection; a non-empty block
          // falls through to the pair-delete logic / native delete.
          if (
            e.key === "Backspace" &&
            ta.value.length === 0 &&
            ta.selectionStart === ta.selectionEnd
          ) {
            e.preventDefault();
            props.onBackspaceEmpty();
            return;
          }
        }

        // Backspace inside an empty `[[]]` or `(())` deletes the
        // whole pair so the user doesn't have to mash four times.
        // We do this in keydown (not input) so we can `preventDefault`
        // before the browser eats the lone `[` to the left of caret.
        if (e.key !== "Backspace") return;
        const ta = e.currentTarget;
        if (ta.selectionStart !== ta.selectionEnd) return; // user is deleting a selection
        const caret = ta.selectionStart ?? 0;
        const completion = autoDeletePair(ta.value, caret);
        if (!completion) return;
        e.preventDefault();
        // `ta.value = …` resets the caret to the end of the text in
        // iOS WKWebView. `parkCaret` (called twice — once before and
        // once after `props.onInput` triggers Solid's `value=`
        // re-binding) keeps the caret where we asked.
        ta.value = completion.value;
        parkCaret(ta, completion.caret);
        props.onInput(completion.value);
        parkCaret(ta, completion.caret);
        autoResize();
      }}
      onInput={(e) => {
        const ta = e.currentTarget;
        const caret = ta.selectionStart ?? ta.value.length;
        const completion = autoClosePair(ta.value, caret);
        if (completion) {
          // Same caret-reset trap as Backspace above. The user just
          // typed the second `[` (or `(`) and we appended the
          // matching closer; without parkCaret the cursor lands at
          // the end (`[[]]_`) instead of the middle (`[[_]]`).
          ta.value = completion.value;
          parkCaret(ta, completion.caret);
          props.onInput(completion.value);
          parkCaret(ta, completion.caret);
        } else {
          props.onInput(ta.value);
        }
        autoResize();
      }}
      onPaste={(e) => {
        // External-clipboard markdown → tree of blocks. We only
        // intercept when the payload looks like an outline; plain
        // text falls through to the browser's default splice so
        // pasting a single URL or code snippet still works the way
        // the user expects.
        if (!props.onPaste) return;
        const text = e.clipboardData?.getData("text/plain") ?? "";
        if (!looksLikeOutline(text)) return;
        e.preventDefault();
        // `selectionStart` is a UTF-16 code unit offset; the Rust
        // backend wants a codepoint count. Conversion is a no-op
        // for BMP text but matters when the host block contains
        // emoji or other supplementary-plane characters.
        const ta = e.currentTarget;
        const caret = utf16OffsetToCharOffset(ta.value, ta.selectionStart ?? 0);
        props.onPaste(caret, text);
      }}
      onBlur={props.onBlur}
    />
  );
}
