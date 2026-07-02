import { createEffect, createSignal, onCleanup, onMount } from "solid-js";

import { createBlock } from "@outl/shared/api/commands";
import type { PageView } from "@outl/shared/api/types";

import { setAppState } from "../lib/store";

/**
 * Ghost first block — rendered by `<OutlineView />` when the open
 * page has no blocks yet (a fresh journal day, an empty page). It
 * looks and edits like a real `<BlockRow />` in Insert mode, with the
 * caret already parked, but the block exists only in the frontend:
 * nothing reaches the op log until the user commits non-empty text.
 *
 * Closing the app (or navigating away) without typing therefore
 * leaves the day's op log and `.md` untouched. The TUI solves the
 * same "cursor needs a home" problem by seeding a real empty bullet
 * on disk; doing that here would append an `Op::Create` for every
 * day the user merely looked at, and the log is append-only.
 *
 * Commit surfaces mirror `<BlockRow />`:
 *   - `Esc` / blur   → materialise the draft as the first block
 *     (empty draft: just blur, back to Normal mode).
 *   - `Cmd/Ctrl+Shift+Enter` → materialise + create a sibling below
 *     and keep typing (the `CommitAndContinue` gesture — intercepted
 *     here because the catalog handler needs a `data-block-id` the
 *     ghost doesn't have).
 *   - plain `Enter`  → newline in the draft (multi-line blocks),
 *     same as a real block's textarea.
 *   - unmount with a pending draft (page nav, peer inserted the
 *     first block) → fire-and-forget materialise so typed text is
 *     never dropped.
 *
 * The `[[ref]]` / `:emoji:` / `/command` popups are deliberately not
 * wired here — they need a materialised block context; they light up
 * on the very next block once this one commits.
 */
export function GhostFirstBlock(props: {
  pageId: string;
  applyView: (view: PageView) => void;
  onError: (e: unknown) => void;
}) {
  const [draft, setDraft] = createSignal("");
  let textareaRef: HTMLTextAreaElement | undefined;
  // Set synchronously the moment a materialise round-trip starts so
  // the blur → cleanup cascade can't create the block twice.
  let committed = false;

  onMount(() => {
    queueMicrotask(() => textareaRef?.focus());
  });

  function autoSize() {
    const ta = textareaRef;
    if (!ta) return;
    ta.style.height = "auto";
    ta.style.height = `${ta.scrollHeight}px`;
  }

  createEffect(() => {
    draft();
    autoSize();
  });

  /** Turn the draft into the page's real first block. Returns the
   *  new block's id, or `null` when there was nothing to commit or
   *  the round-trip failed. */
  async function materialize(): Promise<string | null> {
    if (committed || draft().trim().length === 0) return null;
    committed = true;
    try {
      const reply = await createBlock(props.pageId, {
        afterId: null,
        parentId: null,
        text: draft(),
      });
      props.applyView(reply.view);
      setAppState("selectedBlockId", reply.new_id);
      return reply.new_id;
    } catch (e) {
      committed = false;
      props.onError(e);
      return null;
    }
  }

  async function commitAndContinue() {
    const firstId = await materialize();
    if (!firstId) return;
    try {
      const reply = await createBlock(props.pageId, {
        afterId: firstId,
        parentId: null,
        text: "",
      });
      props.applyView(reply.view);
      setAppState("selectedBlockId", reply.new_id);
      setAppState("editingBlockId", reply.new_id);
    } catch (e) {
      props.onError(e);
    }
  }

  async function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      // Stop the catalog's `ExitInsert` from also firing on this
      // keystroke — it would fall through to its pending-cut-cancel
      // branch once the textarea is blurred.
      e.stopImmediatePropagation();
      if (draft().trim().length === 0) {
        textareaRef?.blur();
        return;
      }
      await materialize();
      return;
    }
    if (e.key === "Enter" && e.shiftKey && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      // Keep the window dispatcher's `CommitAndContinue` off this
      // keystroke (same intercept `<BlockRow />` uses).
      e.stopImmediatePropagation();
      await commitAndContinue();
      return;
    }
    if (e.key === "Tab") {
      // Nothing to indent under yet; just keep focus in the draft.
      e.preventDefault();
    }
  }

  onCleanup(() => {
    // Unmounted with an uncommitted draft — the user navigated away,
    // or a peer materialised the page's first block from another
    // device. Persist the draft to the page it was typed on so the
    // text is never dropped. Fire-and-forget: the returned view may
    // belong to a page that is no longer on screen, so it is not
    // applied here.
    if (!committed && draft().trim().length > 0) {
      committed = true;
      void createBlock(props.pageId, {
        afterId: null,
        parentId: null,
        text: draft(),
      }).catch(props.onError);
    }
  });

  // Chrome mirrors `<BlockRow />`'s depth-0 row (chevron spacer +
  // bullet + body) so the draft doesn't jump when it becomes real.
  return (
    <div class="outl-row group relative flex items-start rounded-sm py-[3px] pr-2">
      <span
        aria-hidden="true"
        class="ml-[6px] mt-[6px] min-w-[16px] text-[9px]"
      />
      <span
        aria-hidden="true"
        class="mt-[5px] mr-2 w-3 shrink-0 select-none text-center text-[13px] leading-none text-(--color-outl-fg-dimmer)"
      >
        •
      </span>
      <div class="min-w-0 flex-1 leading-snug">
        <textarea
          ref={textareaRef}
          value={draft()}
          autofocus
          rows={1}
          spellcheck={false}
          placeholder="Start writing…"
          class="w-full resize-none overflow-hidden bg-transparent text-current outline-none placeholder:text-current placeholder:opacity-30"
          onInput={(e) => setDraft(e.currentTarget.value)}
          onBlur={() => void materialize()}
          onKeyDown={handleKeydown}
        />
      </div>
    </div>
  );
}
