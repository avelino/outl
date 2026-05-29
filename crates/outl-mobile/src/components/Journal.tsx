import {
  For,
  Show,
  batch,
  createEffect,
  createResource,
  createSignal,
  onCleanup,
  onMount,
} from "solid-js";
import {
  BlockNode,
  PageView,
  createBlock,
  dateTitle,
  deleteBlock,
  editBlock,
  indentBlock,
  moveBlockDown,
  moveBlockUp,
  nextDay,
  openJournalFor,
  openPageBySlug,
  openTodayJournal,
  outdentBlock,
  previousDay,
  reloadWorkspace,
  resolveRef,
  searchPages,
  todaySlug,
  toggleTodo,
  workspaceStats,
} from "../lib/api";
import {
  countDescendants,
  findBlock,
  findInsertedAfter,
  flatten,
} from "../lib/outline";
import {
  applySuggestion,
  detectRefContext,
  insertPair,
  insertText,
} from "../lib/autocomplete";
import {
  HIDE_MESSAGE,
  buildShowMessage,
  registerPickedCallback,
  setNativeSuggesterState,
} from "../lib/native-suggester";
import { PageSwitcher } from "./PageSwitcher";
import { PullToRefresh } from "./PullToRefresh";
import { SwipeNavigator } from "./SwipeNavigator";
import { SyncDot } from "./SyncDot";
import { BlockRow } from "./BlockRow";
import { EditToolbar } from "./EditToolbar";
import { haptic } from "../lib/haptics";
import { useKeyboardInset } from "../lib/viewport";
import { BacklinksSection } from "./BacklinksSection";
import { ConfirmDialog } from "./ConfirmDialog";

export function Journal() {
  const [view, setView] = createSignal<PageView | null>(null);
  const [loaded, setLoaded] = createSignal(false);
  const [refreshing, setRefreshing] = createSignal(false);
  const [editingId, setEditingId] = createSignal<string | null>(null);
  const [draft, setDraft] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [stats] = createResource(workspaceStats);
  const [switcherOpen, setSwitcherOpen] = createSignal(false);
  // When set, the delete-confirmation dialog is open. Holds the
  // block id we're about to delete + a descendant count for the
  // copy. Cleared on confirm or cancel.
  const [pendingDelete, setPendingDelete] = createSignal<
    { id: string; descendants: number } | null
  >(null);
  const [syncing, setSyncing] = createSignal(false);
  const keyboardInset = useKeyboardInset();
  const [activeTextareaSignal, setActiveTextareaSignal] = createSignal<
    HTMLTextAreaElement | null
  >(null);
  let activeTextarea: HTMLTextAreaElement | undefined;
  // Hidden textarea used to capture focus synchronously inside a user
  // gesture handler. iOS WKWebView refuses to open the keyboard if
  // `focus()` is called outside a tap event, so we focus this first
  // and let the real block's textarea steal focus once it mounts.
  let ghostInput: HTMLTextAreaElement | undefined;
  // Navigation back-stack so a swipe from a `[[ref]]`-opened page
  // returns to where we came from.
  const [history, setHistory] = createSignal<PageView[]>([]);

  function focusGhost() {
    // Must run synchronously inside the tap to keep iOS in
    // "keyboard mode".
    ghostInput?.focus({ preventScroll: true });
  }

  function pushHistory(v: PageView) {
    setHistory((s) => [...s, v]);
  }

  function popHistory(): PageView | null {
    const stack = history();
    if (stack.length === 0) return null;
    const head = stack[stack.length - 1];
    setHistory(stack.slice(0, -1));
    return head;
  }

  function applyView(v: PageView) {
    setView(v);
  }

  onMount(async () => {
    await loadTodayWithRetry();
    registerNativeToolbarBridge();
    registerOpsChangeBridge();
    listenForWorkspaceReady();
    registerNativeSuggesterBridge();
  });

  /**
   * Drive the native ref suggester (UIKit chip strip above the
   * toolbar — see `main.mm` → `OutlSuggestView` /
   * `OutlAccessoryContainer`). UIKit polls
   * `window.__outlSuggesterState` every 150ms while the keyboard is
   * up; tap → `window.__outlSuggesterPicked(slug, kind)` calls back
   * into here.
   */
  function registerNativeSuggesterBridge() {
    const cleanup = registerPickedCallback((slug, _kind) => {
      const el = activeTextareaSignal();
      if (!el) return;
      const ctx = detectRefContext(el.value, el.selectionStart ?? 0);
      if (!ctx) return;
      const result = applySuggestion(el.value, ctx, slug);
      el.value = result.value;
      try {
        el.setSelectionRange(result.caret, result.caret);
      } catch {
        // ignore — textarea may be momentarily blurred
      }
      setDraft(result.value);
      el.focus({ preventScroll: true });
      setNativeSuggesterState(null);
    });
    onCleanup(cleanup);

    let queryToken = 0;
    let lastQuery: string | null = null;
    createEffect(() => {
      const el = activeTextareaSignal();
      const text = draft();
      if (!el || !editingId()) {
        if (lastQuery !== null) {
          setNativeSuggesterState(null);
          lastQuery = null;
        }
        return;
      }
      const ctx = detectRefContext(el.value, el.selectionStart ?? text.length);
      if (!ctx || ctx.kind !== "page") {
        if (lastQuery !== null) {
          setNativeSuggesterState(null);
          lastQuery = null;
        }
        return;
      }
      if (ctx.query === lastQuery) return;
      lastQuery = ctx.query;
      const token = ++queryToken;
      searchPages(ctx.query).then((items) => {
        if (token !== queryToken) return;
        if (items.length === 0) {
          setNativeSuggesterState(HIDE_MESSAGE);
          return;
        }
        setNativeSuggesterState(buildShowMessage(items));
      });
    });
  }

  /**
   * Bridge for the iCloud watcher in `main.mm`. iOS calls this when
   * `ops-*.jsonl` files inside the ubiquitous container change —
   * meaning a sibling device pushed new ops. We reload the workspace
   * + refresh the current view so the user sees peer changes without
   * having to pull-to-refresh.
   */
  function registerOpsChangeBridge() {
    let pending = false;
    (window as unknown as {
      __outlOpsChanged?: () => void;
    }).__outlOpsChanged = async () => {
      if (pending) return;
      pending = true;
      setSyncing(true);
      try {
        await reloadWorkspace();
        const cur = view();
        if (cur) {
          const next =
            cur.page.kind === "journal"
              ? await openJournalFor(cur.page.slug)
              : await openPageBySlug(cur.page.slug);
          applyView(next);
        }
      } catch {
        // best effort; next interaction will refresh
      } finally {
        pending = false;
        setSyncing(false);
      }
    };
  }

  async function loadTodayWithRetry() {
    for (let i = 0; i < 50; i += 1) {
      try {
        const v = await openTodayJournal();
        applyView(v);
        setError(null);
        setLoaded(true);
        return;
      } catch (e) {
        const msg = String(e);
        if (msg.includes("workspace_loading")) {
          // Workspace opener still in flight; back off briefly and
          // try again. Capped at ~10s of retries.
          await new Promise((r) => setTimeout(r, 200));
          continue;
        }
        setError(msg);
        setLoaded(true);
        return;
      }
    }
    setError("Workspace took too long to open.");
    setLoaded(true);
  }

  function listenForWorkspaceReady() {
    // Best-effort: refresh the current view once the background
    // opener finishes, so anything the user did during the brief
    // "loading" window converges on the freshly opened workspace.
    import("@tauri-apps/api/event").then(({ listen }) => {
      listen("workspace-ready", async () => {
        const v = view();
        if (!v) {
          await loadTodayWithRetry();
          return;
        }
        try {
          const next =
            v.page.kind === "journal"
              ? await openJournalFor(v.page.slug)
              : await openPageBySlug(v.page.slug);
          applyView(next);
        } catch {
          // ignore — next user interaction will refresh
        }
      });
    });
  }

  /**
   * Bridge between the native UIKit keyboard accessory view (defined
   * in `gen/apple/Sources/outl-mobile/main.mm`) and the Solid handlers
   * below. The native buttons call `evaluateJavaScript` with
   * `window.__outlToolbar(action)` and we map each action onto the
   * existing handler.
   */
  function registerNativeToolbarBridge() {
    (window as unknown as {
      __outlToolbar?: (action: string) => void;
    }).__outlToolbar = (action: string) => {
      const id = editingId();
      switch (action) {
        case "indent":
          if (id) handleIndent(id);
          return;
        case "outdent":
          if (id) handleOutdent(id);
          return;
        case "moveUp":
          if (id) handleMoveUp(id);
          return;
        case "moveDown":
          if (id) handleMoveDown(id);
          return;
        case "todo":
          if (id) handleToggleTodo(id);
          return;
        case "delete":
          if (id) handleDelete(id);
          return;
        case "newLine":
          if (id) {
            handleCreateAfter(id);
          } else {
            handleAppendBlock();
          }
          return;
        case "bold":
          wrapSelection("bold");
          return;
        case "italic":
          wrapSelection("italic");
          return;
        case "code":
          wrapSelection("code");
          return;
        case "insertRef":
          insertAtCursor("pair", "[[", "]]");
          return;
        case "insertBlock":
          insertAtCursor("pair", "((", "))");
          return;
        case "insertHash":
          insertAtCursor("text", "#");
          return;
        case "done":
          if (editingId()) commitEdit();
          return;
      }
    };
  }

  async function withError<T>(fn: () => Promise<T>): Promise<T | undefined> {
    try {
      setError(null);
      return await fn();
    } catch (e) {
      setError(String(e));
      haptic("warning");
      return undefined;
    }
  }

  function pageId(): string | null {
    return view()?.page.id ?? null;
  }

  function startEdit(id: string, initial: string) {
    batch(() => {
      setEditingId(id);
      setDraft(initial);
    });
    haptic("light");
  }

  async function commitEdit() {
    const id = editingId();
    const pid = pageId();
    if (!id || !pid) return;
    const text = draft();
    setEditingId(null);
    const next = await withError(() => editBlock(pid, id, text));
    if (next) applyView(next);
  }

  async function handleToggleTodo(id: string) {
    const pid = pageId();
    if (!pid) return;
    const wasEditing = editingId() === id;
    if (wasEditing) {
      // Commit current draft text into the workspace so the cycle
      // operates on what the user typed, without dropping out of
      // edit mode (we want the keyboard to stay up).
      const text = draft();
      const committed = await withError(() => editBlock(pid, id, text));
      if (committed) setView(committed);
    }
    const next = await withError(() => toggleTodo(pid, id));
    if (!next) return;
    applyView(next);
    if (wasEditing) {
      // Keep edit mode on the same block; refresh draft to the
      // backend's view (without the TODO/DONE prefix).
      const block = findBlock(next.outline, id);
      if (block) setDraft(block.text);
    }
  }

  /**
   * Delete a block. When the block has descendants we *always*
   * prompt — deleting a parent destroys the whole subtree and the
   * user can't undo that from the mobile UI yet. Leaf blocks
   * delete immediately (no prompt) to keep the swipe gesture fast.
   */
  function handleDelete(id: string) {
    const cur = view();
    if (!cur) return;
    const block = findBlock(cur.outline, id);
    const descendants = block ? countDescendants(block) : 0;
    if (descendants > 0) {
      setPendingDelete({ id, descendants });
      return;
    }
    void performDelete(id);
  }

  async function performDelete(id: string) {
    const pid = pageId();
    if (!pid) return;
    if (editingId() === id) setEditingId(null);
    const next = await withError(() => deleteBlock(pid, id));
    if (next) applyView(next);
  }

  async function handleIndent(id: string) {
    const pid = pageId();
    if (!pid) return;
    haptic("light");
    const next = await withError(() => indentBlock(pid, id));
    if (next) applyView(next);
  }

  async function handleOutdent(id: string) {
    const pid = pageId();
    if (!pid) return;
    haptic("light");
    const next = await withError(() => outdentBlock(pid, id));
    if (next) applyView(next);
  }

  async function handleMoveUp(id: string) {
    const pid = pageId();
    if (!pid) return;
    haptic("light");
    const next = await withError(() => moveBlockUp(pid, id));
    if (next) applyView(next);
  }

  async function handleMoveDown(id: string) {
    const pid = pageId();
    if (!pid) return;
    haptic("light");
    const next = await withError(() => moveBlockDown(pid, id));
    if (next) applyView(next);
  }

  async function handleCreateAfter(id: string) {
    const pid = pageId();
    if (!pid) return;
    // Keep keyboard up across the async create by parking focus on
    // the ghost textarea first.
    focusGhost();
    if (editingId()) await commitEdit();
    const next = await withError(() =>
      createBlock(pid, { afterId: id, text: null }),
    );
    if (next) {
      applyView(next);
      const last = findInsertedAfter(next.outline, id);
      if (last) startEdit(last.id, "");
    }
  }

  async function handleAppendBlock() {
    const pid = pageId();
    if (!pid) return;
    focusGhost();
    if (editingId()) await commitEdit();
    haptic("medium");
    const next = await withError(() =>
      createBlock(pid, { afterId: null, parentId: null, text: null }),
    );
    if (next) {
      applyView(next);
      const last = flatten(next.outline).at(-1);
      if (last) startEdit(last.id, "");
    }
  }

  async function handleRefresh() {
    const pid = pageId();
    if (!pid) return;
    setRefreshing(true);
    setSyncing(true);
    haptic("light");
    await withError(reloadWorkspace);
    // Reopen current page after refresh.
    const cur = view();
    if (cur) {
      const next =
        cur.page.kind === "journal"
          ? await withError(() => openJournalFor(cur.page.slug))
          : await withError(() => openPageBySlug(cur.page.slug));
      if (next) applyView(next);
    }
    setRefreshing(false);
    setSyncing(false);
  }

  async function handlePrevDay() {
    const cur = view();
    if (!cur || cur.page.kind !== "journal") return;
    haptic("light");
    const slug = await withError(() => previousDay(cur.page.slug));
    if (slug) {
      const next = await withError(() => openJournalFor(slug));
      if (next) applyView(next);
    }
  }

  async function handleNextDay() {
    const cur = view();
    if (!cur || cur.page.kind !== "journal") return;
    haptic("light");
    const slug = await withError(() => nextDay(cur.page.slug));
    if (slug) {
      const next = await withError(() => openJournalFor(slug));
      if (next) applyView(next);
    }
  }

  async function handleJumpToday() {
    haptic("light");
    const next = await withError(openTodayJournal);
    if (next) applyView(next);
  }

  async function handleRefClick(target: string) {
    haptic("light");
    const currentView = view();
    // Try as date slug first.
    const asDate = /^\d{4}-\d{2}-\d{2}$/.test(target);
    if (asDate) {
      const next = await withError(() => openJournalFor(target));
      if (next) {
        if (currentView) pushHistory(currentView);
        applyView(next);
        return;
      }
    }
    // Resolve as page slug or title.
    const meta = await withError(() => resolveRef(target));
    if (meta) {
      const next =
        meta.kind === "journal"
          ? await withError(() => openJournalFor(meta.slug))
          : await withError(() => openPageBySlug(meta.slug));
      if (next) {
        if (currentView) pushHistory(currentView);
        applyView(next);
      }
      return;
    }
    // Fallback: open/create by slug.
    const next = await withError(() => openPageBySlug(target));
    if (next) {
      if (currentView) pushHistory(currentView);
      applyView(next);
    }
  }

  async function handleTagClick(tag: string) {
    // `#foo` arrives as "#foo"; strip the leading hash and open the
    // page with that slug (same semantics as a `[[foo]]` ref).
    const target = tag.startsWith("#") ? tag.slice(1) : tag;
    if (!target) return;
    haptic("light");
    const currentView = view();
    const next = await withError(() => openPageBySlug(target));
    if (next) {
      if (currentView) pushHistory(currentView);
      applyView(next);
    }
  }

  async function handlePickPage(slug: string, kind: "page" | "journal") {
    setSwitcherOpen(false);
    haptic("light");
    const currentView = view();
    const next =
      kind === "journal"
        ? await withError(() => openJournalFor(slug))
        : await withError(() => openPageBySlug(slug));
    if (next) {
      if (currentView) pushHistory(currentView);
      applyView(next);
    }
  }

  function handleBack() {
    const prev = popHistory();
    if (prev) {
      haptic("light");
      applyView(prev);
    }
  }

  /** Insert a snippet (or open/close pair) into the active textarea
   *  synchronously so iOS keeps the keyboard up across the change. */
  function insertAtCursor(
    mode: "text" | "pair",
    open: string,
    close: string = "",
  ) {
    const el = activeTextarea;
    if (!el) return;
    const value = el.value;
    const caret = el.selectionStart ?? value.length;
    const result =
      mode === "pair"
        ? insertPair(value, caret, open, close)
        : insertText(value, caret, open);
    el.value = result.value;
    setDraft(result.value);
    try {
      el.setSelectionRange(result.caret, result.caret);
    } catch {
      // ignore — happens if the textarea is momentarily blurred
    }
    el.focus({ preventScroll: true });
  }

  function wrapSelection(style: "bold" | "italic" | "code") {
    const el = activeTextarea;
    if (!el) return;
    const before = el.value;
    const start = el.selectionStart ?? before.length;
    const end = el.selectionEnd ?? before.length;
    const wrap = style === "bold" ? "**" : style === "italic" ? "*" : "`";
    const next =
      before.slice(0, start) +
      wrap +
      before.slice(start, end) +
      wrap +
      before.slice(end);
    // Mutate DOM + state synchronously so iOS doesn't see us drop
    // focus across an async boundary.
    el.value = next;
    setDraft(next);
    const caret = start + wrap.length + (end - start) + wrap.length;
    try {
      el.setSelectionRange(caret, caret);
    } catch {
      // ignore — happens on textareas that are momentarily blurred
    }
    el.focus({ preventScroll: true });
  }

  return (
    <div class="flex h-full flex-col">
      <header
        class="z-30 shrink-0 border-b border-(--color-ios-divider)/30 bg-(--color-ios-bg)/95 px-4 pt-2 pb-3 backdrop-blur-xl dark:border-(--color-iosd-divider)/30 dark:bg-(--color-iosd-bg)/95"
        style="padding-top: max(env(safe-area-inset-top), 12px);"
      >
          <div class="flex items-center justify-between gap-3">
            <Show
              when={view()?.page.kind === "journal"}
              fallback={
                <PageHeader title={view()?.page.title ?? ""} kind={view()?.page.kind ?? null} />
              }
            >
              <JournalHeader
                slug={view()?.page.slug ?? ""}
                onPrev={handlePrevDay}
                onNext={handleNextDay}
                onToday={handleJumpToday}
              />
            </Show>
            <button
              type="button"
              aria-label="Pages"
              onClick={() => {
                haptic("light");
                setSwitcherOpen(true);
              }}
              class="rounded-full p-2 active:opacity-50"
            >
              <svg
                width="22"
                height="22"
                viewBox="0 0 24 24"
                fill="none"
                stroke="var(--color-ios-accent)"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <path d="M21 21l-4.3-4.3M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16z" />
              </svg>
            </button>
            <div class="flex items-center gap-1">
              <SyncDot status={syncing() ? "syncing" : "synced"} />
              <button
                type="button"
                aria-label="Sync from iCloud"
                onClick={handleRefresh}
                class="rounded-full p-2 active:opacity-50"
              >
                <svg
                  width="20"
                  height="20"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="var(--color-ios-accent)"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  style={{
                    transform: refreshing()
                      ? "rotate(360deg)"
                      : "rotate(0deg)",
                    transition: "transform 800ms ease-in-out",
                  }}
                  aria-hidden="true"
                >
                  <path d="M21 12a9 9 0 1 1-3-6.7L21 8" />
                  <path d="M21 3v5h-5" />
                </svg>
              </button>
            </div>
          </div>
      </header>

      <main class="ios-scroll flex-1 pb-32">
        <PullToRefresh onRefresh={handleRefresh}>
        <SwipeNavigator
          disabled={editingId() !== null}
          onSwipeRight={() => {
            if (view()?.page.kind === "journal") {
              handlePrevDay();
            } else if (history().length > 0) {
              handleBack();
            }
          }}
          onSwipeLeft={() => {
            if (view()?.page.kind === "journal") {
              handleNextDay();
            } else if (history().length > 0) {
              // No "forward" yet on page navigation; mirror back so
              // users can swipe either direction to return.
              handleBack();
            }
          }}
        >
        <div class="min-h-[60vh]">
        <section class="mt-1 pb-1">
          <Show
            when={loaded() && view() && view()!.outline.length > 0}
            fallback={
              <Show
                when={loaded()}
                fallback={
                  <div class="px-5 py-12 text-center text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
                    Loading…
                  </div>
                }
              >
                <button
                  type="button"
                  onClick={handleAppendBlock}
                  class="block w-full px-5 py-12 text-center text-(--color-ios-text-secondary) active:opacity-50 dark:text-(--color-iosd-text-secondary)"
                >
                  <p class="text-[15px]">Nothing here yet.</p>
                  <p class="mt-1 text-[13px] text-(--color-ios-accent) dark:text-(--color-iosd-accent)">
                    Tap to start writing
                  </p>
                </button>
              </Show>
            }
          >
            <For each={view()!.outline}>
              {(block) => (
                <BlockRow
                  block={block}
                  depth={0}
                  editingId={editingId()}
                  draftText={draft()}
                  onStartEdit={startEdit}
                  onDraftChange={setDraft}
                  onCommitEdit={commitEdit}
                  onToggleTodo={handleToggleTodo}
                  onDelete={handleDelete}
                  onIndent={handleIndent}
                  onOutdent={handleOutdent}
                  onCreateAfter={handleCreateAfter}
                  onRefClick={handleRefClick}
                  onTagClick={handleTagClick}
                  onTextareaMount={(el) => {
                    activeTextarea = el;
                    setActiveTextareaSignal(el);
                  }}
                />
              )}
            </For>
          </Show>
        </section>

        <Show when={error()}>
          <p class="mx-5 mt-3 text-center text-[13px] text-(--color-ios-destructive) dark:text-(--color-iosd-destructive)">
            {error()}
          </p>
        </Show>

        <Show when={view() && view()!.backlinks.length > 0}>
          <BacklinksSection
            backlinks={view()!.backlinks}
            onJump={async (link) => {
              if (!link.source_page) return;
              haptic("light");
              const sp = link.source_page;
              const currentView = view();
              const next =
                sp.kind === "journal"
                  ? await withError(() => openJournalFor(sp.slug))
                  : await withError(() => openPageBySlug(sp.slug));
              if (next) {
                if (currentView) pushHistory(currentView);
                applyView(next);
              }
            }}
          />
        </Show>
        </div>
        </SwipeNavigator>
        </PullToRefresh>

        <Show when={stats()}>
          <footer class="px-5 pt-3 pb-32 text-center text-[12px] text-(--color-ios-text-tertiary) dark:text-(--color-iosd-text-tertiary)">
            {stats()!.blocks} blocks · {stats()!.ops} ops · actor{" "}
            {stats()!.actor.slice(0, 6)}
          </footer>
        </Show>
      </main>

      <Show when={!editingId() && view()}>
        <button
          type="button"
          aria-label="Add block"
          onClick={handleAppendBlock}
          class="outl-press fixed right-5 z-30 flex h-14 w-14 items-center justify-center rounded-full bg-(--color-ios-accent) shadow-lg dark:bg-(--color-iosd-accent)"
          style="bottom: max(env(safe-area-inset-bottom), 20px);"
        >
          <svg
            width="26"
            height="26"
            viewBox="0 0 24 24"
            fill="none"
            stroke="white"
            stroke-width="2.5"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <path d="M12 5v14M5 12h14" />
          </svg>
        </button>
      </Show>

      {/* Ghost textarea kept off-screen, focused inside tap handlers
          to preserve iOS keyboard state across async work. */}
      <textarea
        ref={ghostInput}
        aria-hidden="true"
        tabindex="-1"
        readonly
        class="pointer-events-none absolute h-0 w-0 -translate-y-full opacity-0"
        style="left: -9999px; top: -9999px;"
      />

      <PageSwitcher
        open={switcherOpen()}
        currentSlug={view()?.page.slug ?? null}
        onClose={() => setSwitcherOpen(false)}
        onPick={handlePickPage}
      />

      <ConfirmDialog
        open={pendingDelete() !== null}
        title="Delete block?"
        message={
          pendingDelete()
            ? `This block has ${pendingDelete()!.descendants} ${
                pendingDelete()!.descendants === 1 ? "child" : "children"
              } that will also be deleted. This can't be undone.`
            : ""
        }
        onCancel={() => setPendingDelete(null)}
        onConfirm={() => {
          const p = pendingDelete();
          setPendingDelete(null);
          if (p) void performDelete(p.id);
        }}
      />

      {/* HTML toolbar kept only as a desktop / non-iOS fallback. On
          iOS the native UIKit toolbar (see main.mm) takes over via
          `window.__outlToolbar`. */}
      <EditToolbar
        visible={false}
        keyboardInset={keyboardInset()}
        onIndent={() => {
          const id = editingId();
          if (id) handleIndent(id);
        }}
        onOutdent={() => {
          const id = editingId();
          if (id) handleOutdent(id);
        }}
        onToggleTodo={() => {
          const id = editingId();
          if (id) handleToggleTodo(id);
        }}
        onDelete={() => {
          const id = editingId();
          if (id) handleDelete(id);
        }}
        onNewLine={() => {
          const id = editingId();
          if (id) handleCreateAfter(id);
        }}
        onDone={commitEdit}
        onWrap={wrapSelection}
        onMoveUp={() => {
          const id = editingId();
          if (id) handleMoveUp(id);
        }}
        onMoveDown={() => {
          const id = editingId();
          if (id) handleMoveDown(id);
        }}
      />
    </div>
  );
}

function JournalHeader(props: {
  slug: string;
  onPrev: () => void;
  onNext: () => void;
  onToday: () => void;
}) {
  const [isToday, setIsToday] = createSignal(true);
  onMount(async () => {
    const t = await todaySlug();
    setIsToday(t === props.slug);
  });
  return (
    <div class="flex-1">
      <div class="flex items-center gap-1.5 text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
        <button
          type="button"
          aria-label="Previous day"
          onClick={props.onPrev}
          class="rounded-full p-1 active:opacity-50"
        >
          <ChevronLeft />
        </button>
        <p
          class="cursor-pointer text-[11px] font-medium uppercase tracking-[0.08em]"
          onClick={props.onToday}
        >
          <Show
            when={isToday()}
            fallback={
              <>
                Journal ·{" "}
                <span class="text-(--color-ios-accent) dark:text-(--color-iosd-accent)">
                  today
                </span>
              </>
            }
          >
            Today
          </Show>
        </p>
        <button
          type="button"
          aria-label="Next day"
          onClick={props.onNext}
          class="rounded-full p-1 active:opacity-50"
        >
          <ChevronRight />
        </button>
      </div>
      <h1 class="mt-0.5 text-[26px] font-bold leading-tight tracking-tight tabular-nums">
        {props.slug}
      </h1>
    </div>
  );
}

function PageHeader(props: { title: string; kind: "page" | "journal" | null }) {
  return (
    <div class="flex-1">
      <p class="text-[12px] font-medium uppercase tracking-wider text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
        {props.kind === "journal" ? "Journal" : "Page"}
      </p>
      <h1 class="mt-0.5 text-[26px] font-bold leading-tight tracking-tight">
        {props.title}
      </h1>
    </div>
  );
}

function ChevronLeft() {
  return (
    <svg
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2.5"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M15 18l-6-6 6-6" />
    </svg>
  );
}

function ChevronRight() {
  return (
    <svg
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2.5"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M9 18l6-6-6-6" />
    </svg>
  );
}

// Use referenced helper to silence unused-import false-positive.
const _holdTitle = dateTitle;
void _holdTitle;
