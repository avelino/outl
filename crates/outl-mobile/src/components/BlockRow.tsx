import { For, JSX, Show, onCleanup, onMount } from "solid-js";
import { BlockNode } from "../lib/api";
import { MarkdownInline } from "../lib/markdown";
import { autoClosePair } from "../lib/autocomplete";
import { haptic } from "../lib/haptics";
import { SwipeRow } from "./SwipeRow";

interface BlockRowProps {
  block: BlockNode;
  depth: number;
  editingId: string | null;
  draftText: string;
  onStartEdit: (id: string, initialText: string) => void;
  onDraftChange: (text: string) => void;
  onCommitEdit: () => void;
  onToggleTodo: (id: string) => void;
  onDelete: (id: string) => void;
  onIndent: (id: string) => void;
  onOutdent: (id: string) => void;
  onCreateAfter: (id: string) => void;
  onRefClick?: (target: string) => void;
  onTagClick?: (tag: string) => void;
  onTextareaMount?: (el: HTMLTextAreaElement) => void;
}

/**
 * One row of the outline. Handles read-mode (rendered markdown) and
 * edit-mode (textarea), TODO checkbox, swipe-to-delete, long-press
 * to toggle TODO, and renders children recursively.
 */
const INDENT_PX = 22;

export function BlockRow(props: BlockRowProps): JSX.Element {
  const isEditing = () => props.editingId === props.block.id;

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
          onStartEdit={() =>
            props.onStartEdit(props.block.id, props.block.text)
          }
          onDraftChange={props.onDraftChange}
          onCommitEdit={props.onCommitEdit}
          onToggleTodo={() => {
            haptic("light");
            props.onToggleTodo(props.block.id);
          }}
          onLongPress={() => {
            haptic("medium");
            props.onToggleTodo(props.block.id);
          }}
          onRefClick={props.onRefClick}
          onTagClick={props.onTagClick}
          onTextareaMount={props.onTextareaMount}
        />
      </SwipeRow>

      <Show when={props.block.children.length > 0}>
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
                onIndent={props.onIndent}
                onOutdent={props.onOutdent}
                onCreateAfter={props.onCreateAfter}
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
  draftText: string;
  depth: number;
  onStartEdit: () => void;
  onDraftChange: (text: string) => void;
  onCommitEdit: () => void;
  onToggleTodo: () => void;
  onLongPress: () => void;
  onRefClick?: (target: string) => void;
  onTagClick?: (tag: string) => void;
  onTextareaMount?: (el: HTMLTextAreaElement) => void;
}) {
  let longPressTimer: number | undefined;
  let downX = 0;
  let downY = 0;
  let didLongPress = false;

  function onPointerDown(e: PointerEvent) {
    if (props.editing) return;
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
  function onClick() {
    if (didLongPress) {
      didLongPress = false;
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
      <BulletOrCheckbox
        todo={props.block.todo}
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
            value={props.draftText}
            onInput={props.onDraftChange}
            onBlur={props.onCommitEdit}
            onMount={props.onTextareaMount}
          />
        </Show>
      </div>
    </div>
  );
}

function BulletOrCheckbox(props: {
  todo: BlockNode["todo"];
  onToggle: () => void;
}) {
  return (
    <Show
      when={props.todo !== null}
      fallback={
        <span
          aria-hidden="true"
          class="relative z-10 mt-[12px] h-1 w-1 shrink-0 rounded-full bg-(--color-ios-text-tertiary) dark:bg-(--color-iosd-text-tertiary)"
        />
      }
    >
      <button
        type="button"
        aria-label={props.todo === "DONE" ? "Mark as TODO" : "Mark as done"}
        onClick={(e) => {
          e.stopPropagation();
          props.onToggle();
        }}
        class="relative z-10 mt-[3px] flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-full border-[1.5px]"
        classList={{
          "border-(--color-ios-accent) bg-(--color-ios-accent) dark:border-(--color-iosd-accent) dark:bg-(--color-iosd-accent)":
            props.todo === "DONE",
          "border-(--color-ios-text-secondary) bg-transparent dark:border-(--color-iosd-text-secondary)":
            props.todo !== "DONE",
        }}
      >
        <Show when={props.todo === "DONE"}>
          <svg
            width="11"
            height="11"
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
      </button>
    </Show>
  );
}

function EditableTextarea(props: {
  value: string;
  onInput: (v: string) => void;
  onBlur: () => void;
  onMount?: (el: HTMLTextAreaElement) => void;
}) {
  let ref!: HTMLTextAreaElement;

  function autoResize() {
    if (!ref) return;
    ref.style.height = "auto";
    ref.style.height = `${ref.scrollHeight}px`;
  }

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
      onInput={(e) => {
        const ta = e.currentTarget;
        const caret = ta.selectionStart ?? ta.value.length;
        const completion = autoClosePair(ta.value, caret);
        if (completion) {
          // Insert closer + re-park caret synchronously so iOS keeps
          // the keyboard up. We must mutate the DOM *and* the Solid
          // signal in the same tick.
          ta.value = completion.value;
          try {
            ta.setSelectionRange(completion.caret, completion.caret);
          } catch {
            // ignore — happens if the element is momentarily blurred
          }
          props.onInput(completion.value);
        } else {
          props.onInput(ta.value);
        }
        autoResize();
      }}
      onBlur={props.onBlur}
    />
  );
}
