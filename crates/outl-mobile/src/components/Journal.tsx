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
import type { PageView } from "@outl/shared/api/types";
import {
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
  openExternalUrl,
  openRef,
  openTodayJournal,
  outdentBlock,
  pasteMarkdown,
  previousDay,
  reloadWorkspace,
  runCodeBlock,
  searchEmojis,
  searchPages,
  searchPersons,
  setBlockCollapsed,
  todaySlug,
  toggleTodo,
  workspaceStats,
} from "@outl/shared/api/commands";
import { detectFence } from "@outl/shared/highlight";
import {
  countDescendants,
  findBlock,
  rawTextWithTodo,
} from "../lib/outline";
import {
  applyEmojiSuggestion,
  applySuggestion,
  detectEmojiContext,
  detectRefContext,
  withCreateNewPersonCandidate,
} from "@outl/shared/autocomplete";
import { ParseWarningsBanner } from "@outl/shared/warnings";
import { parkCaret, spliceText } from "../lib/textarea";
import { withTimeout } from "../lib/async";

/** Maximum time we wait for a single Tauri command to settle before
 *  surfacing a timeout error. Keeps the UI from getting stuck in
 *  "syncing…" forever when iCloud coordination stalls. */
const EDIT_TIMEOUT_MS = 8000;
import {
  HIDE_MESSAGE,
  buildEmojiShowMessage,
  buildShowMessage,
  registerPickedCallback,
  setNativeSuggesterState,
} from "../lib/native-suggester";
import { Calendar } from "./Calendar";
import { PageSwitcher } from "./PageSwitcher";
import { PullToRefresh } from "./PullToRefresh";
import { SyncDot } from "./SyncDot";
import { BlockRow } from "./BlockRow";
import { SkeletonOutline } from "./Skeleton";
import { haptic } from "../lib/haptics";
import { BacklinksSection } from "./BacklinksSection";
import { BlockContextMenu, type BlockContextAction } from "./BlockContextMenu";
import { ConfirmDialog } from "./ConfirmDialog";
import { Toast } from "./Toast";

export function Journal() {
  const [view, setView] = createSignal<PageView | null>(null);
  const [loaded, setLoaded] = createSignal(false);
  const [refreshing, setRefreshing] = createSignal(false);
  // Loading message + failure flag drive the initial-load placeholder.
  // The `SkeletonOutline` placeholder is the user-facing signal that
  // we're still loading; `loadFailed` flips only when we give up so
  // the retry button has a clean condition to render against.
  const [loadFailed, setLoadFailed] = createSignal(false);
  const [editingId, setEditingId] = createSignal<string | null>(null);
  const [draft, setDraft] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  // Optional retry handler tied to the most recent error. When set,
  // the toast pins (no auto-dismiss) and shows a "Retry" button. We
  // store it alongside `error` so callers can offer the affordance
  // without plumbing it through every async helper.
  const [errorRetry, setErrorRetry] = createSignal<(() => void) | null>(null);
  const [stats] = createResource(workspaceStats);
  const [switcherOpen, setSwitcherOpen] = createSignal(false);
  const [calendarOpen, setCalendarOpen] = createSignal(false);
  // When set, the delete-confirmation dialog is open. Holds the
  // block id we're about to delete + a descendant count for the
  // copy. Cleared on confirm or cancel.
  const [pendingDelete, setPendingDelete] = createSignal<
    { id: string; descendants: number } | null
  >(null);
  // Block id whose contextual menu is currently open (long-press
  // gesture target). `null` when no menu is showing.
  const [contextMenuBlockId, setContextMenuBlockId] = createSignal<
    string | null
  >(null);
  const [syncing, setSyncing] = createSignal(false);
  // Network state — drives the `<SyncDot>` "offline" pill so the
  // user knows iCloud peer pushes are stalled. `navigator.onLine`
  // is not perfectly accurate (it lies when a captive portal eats
  // requests) but it's the best signal a WebView has.
  const [online, setOnline] = createSignal(
    typeof navigator !== "undefined" ? navigator.onLine : true,
  );
  // Single in-flight `editBlock` lock. Two concurrent edits to the
  // same block can land in arbitrary order at the backend (e.g.
  // toggle-todo's optimistic commit racing with a delayed onBlur
  // commit), and the loser overwrites the winner. We serialize so
  // the user's last keystroke always wins.
  let commitInFlight: Promise<unknown> | null = null;
  const [activeTextareaSignal, setActiveTextareaSignal] = createSignal<
    HTMLTextAreaElement | null
  >(null);
  let activeTextarea: HTMLTextAreaElement | undefined;
  // Hidden textarea used to capture focus synchronously inside a user
  // gesture handler. iOS WKWebView refuses to open the keyboard if
  // `focus()` is called outside a tap event, so we focus this first
  // and let the real block's textarea steal focus once it mounts.
  let ghostInput: HTMLTextAreaElement | undefined;
  // Today's journal slug. Re-resolved on mount and whenever the app
  // returns to the foreground, so the affordance stays correct across a
  // midnight rollover (the app can sit open past midnight: "today"
  // changes but a value cached once on mount wouldn't). Single source of
  // truth for every "is this today?" decision — `canJumpToday` here and
  // `JournalHeader`'s label both read it, instead of resolving "today"
  // independently and risking disagreement.
  const [todaySlugValue, setTodaySlugValue] = createSignal<string | null>(null);

  function focusGhost() {
    // Must run synchronously inside the tap to keep iOS in
    // "keyboard mode".
    ghostInput?.focus({ preventScroll: true });
  }

  function applyView(v: PageView) {
    setView(v);
  }

  // Native bridges + reactive effects MUST register synchronously,
  // before any `await`. Solid loses the owner context across an
  // `await` boundary, so `createEffect` / `onCleanup` called after
  // an awaited call become orphans — the effect never tracks
  // signals, the cleanup never fires. Specifically: putting
  // `registerNativeSuggesterBridge()` after `await loadTodayWithRetry()`
  // is what made the ref autocomplete look broken on iOS: state was
  // published once and then never updated as the user typed inside
  // `[[…]]`.
  registerNativeToolbarBridge();
  registerOpsChangeBridge();
  registerNativeSuggesterBridge();

  // Track connectivity so the SyncDot can show "offline" when iCloud
  // can't reach peers. Both listeners are pure DOM side-effects but
  // they must be registered + torn down within the component's
  // owner; `onCleanup` here, not deep inside `onMount`'s async body.
  if (typeof window !== "undefined") {
    const upOnline = () => setOnline(true);
    const upOffline = () => setOnline(false);
    window.addEventListener("online", upOnline);
    window.addEventListener("offline", upOffline);
    onCleanup(() => {
      window.removeEventListener("online", upOnline);
      window.removeEventListener("offline", upOffline);
    });
  }

  // Resolve "today" up front and again every time the app comes back to
  // the foreground (covers the midnight rollover). `disposed` guards the
  // async setter so a resolution that lands after the component unmounts
  // doesn't poke a torn-down signal.
  let disposed = false;
  function refreshTodaySlug() {
    todaySlug()
      .then((t) => {
        if (!disposed) setTodaySlugValue(t);
      })
      .catch((e) => {
        // Best effort; the affordance just stays hidden until we know
        // today's slug. Log so a backend regression is still visible.
        console.warn("failed to resolve today's slug", e);
      });
  }
  refreshTodaySlug();
  if (typeof document !== "undefined") {
    const onVisible = () => {
      if (document.visibilityState === "visible") refreshTodaySlug();
    };
    document.addEventListener("visibilitychange", onVisible);
    onCleanup(() => {
      disposed = true;
      document.removeEventListener("visibilitychange", onVisible);
    });
  } else {
    onCleanup(() => {
      disposed = true;
    });
  }

  onMount(async () => {
    listenForWorkspaceReady();
    await loadTodayWithRetry();
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
    const cleanup = registerPickedCallback((slug, kind) => {
      const el = activeTextareaSignal();
      if (!el) return;
      // Emoji branch: the chip strip published `:shortcode:` candidates,
      // tap returns the shortcode. Use `detectEmojiContext` (the same
      // trigger detector the effect below ran) + `applyEmojiSuggestion`
      // so the disk form stays the canonical `:shortcode:` literal.
      if (kind === "emoji") {
        const ctx = detectEmojiContext(el.value, el.selectionStart ?? 0);
        if (!ctx) return;
        const result = applyEmojiSuggestion(el.value, ctx, slug);
        const insert = result.value.slice(ctx.openIndex, result.caret);
        spliceText(el, ctx.openIndex, ctx.replaceEnd, insert);
        parkCaret(el, result.caret);
        setDraft(el.value);
        parkCaret(el, result.caret);
        setNativeSuggesterState(null);
        return;
      }
      const ctx = detectRefContext(el.value, el.selectionStart ?? 0);
      if (!ctx) return;
      // Mention sugar: materialise the person page in the backend
      // (fire-and-forget) so the inserted `[[@title]]` link resolves
      // on subsequent loads. Idempotent — `open_or_create_by_ref`
      // strips the `@`, sets `type:: person` on a fresh page, and
      // returns the existing node otherwise. Same policy desktop +
      // TUI apply on the same gesture.
      if (ctx.kind === "mention") {
        void openRef(`@${slug}`).catch((e) => {
          console.warn("openRef for mention failed:", e);
        });
      }
      // Build the result through the pure helper so its semantics
      // (e.g. choosing `[[` vs `((` delimiters) stay one place, but
      // apply it via `spliceText` + `parkCaret` to dodge the
      // Solid-binding caret-reset trap that bit `el.value = …`.
      const result = applySuggestion(el.value, ctx, slug);
      const insert = result.value.slice(ctx.openIndex, result.caret);
      spliceText(el, ctx.openIndex, ctx.replaceEnd, insert);
      parkCaret(el, result.caret);
      setDraft(el.value);
      parkCaret(el, result.caret);
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
      const cursor = el.selectionStart ?? text.length;
      // Emoji takes precedence over ref detection because both can be
      // active at the same caret position (a `:` typed inside a stray
      // `[[…` would otherwise stay invisible). Bail to the ref branch
      // only when no `:shortcode` trigger is open.
      const emojiCtx = detectEmojiContext(el.value, cursor);
      if (emojiCtx) {
        const key = `emoji:${emojiCtx.query}`;
        if (key === lastQuery) return;
        lastQuery = key;
        const token = ++queryToken;
        // `limit: 8` mirrors every other client's autocomplete cap so
        // the chip strip doesn't overflow on long substring queries.
        void searchEmojis(emojiCtx.query, 8).then((hits) => {
          if (token !== queryToken) return;
          if (hits.length === 0) {
            setNativeSuggesterState(HIDE_MESSAGE);
            return;
          }
          setNativeSuggesterState(buildEmojiShowMessage(hits));
        });
        return;
      }
      const ctx = detectRefContext(el.value, cursor);
      // `page` → fuzzy over every page; `mention` → fuzzy over
      // persons only. Block-ref autocompletion stays out of this path.
      if (!ctx || (ctx.kind !== "page" && ctx.kind !== "mention")) {
        if (lastQuery !== null) {
          setNativeSuggesterState(null);
          lastQuery = null;
        }
        return;
      }
      const key = `${ctx.kind}:${ctx.query}`;
      if (key === lastQuery) return;
      lastQuery = key;
      const token = ++queryToken;
      const fetcher = ctx.kind === "mention" ? searchPersons : searchPages;
      const mention = ctx.kind === "mention";
      fetcher(ctx.query).then((items) => {
        if (token !== queryToken) return;
        // Create-new affordance for mentions — shared with desktop
        // via `@outl/shared/autocomplete::withCreateNewPersonCandidate`.
        const finalItems = mention
          ? withCreateNewPersonCandidate(items, ctx.query)
          : items;
        if (finalItems.length === 0) {
          setNativeSuggesterState(HIDE_MESSAGE);
          return;
        }
        setNativeSuggesterState(buildShowMessage(finalItems, { mention }));
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
    // Show a generic "Loading…" first, then upgrade the message to
    // The skeleton placeholder takes the place of the old progress
    // message; we keep retrying the workspace open silently and only
    // flip `loadFailed` if we exhaust the budget.
    setLoadFailed(false);
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
        setLoadFailed(true);
        setLoaded(true);
        return;
      }
    }
    setError("Workspace took too long to open.");
    setLoadFailed(true);
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
    // Serialize: if an earlier edit is still in flight, wait for it
    // to land before we send this one. Without this, a quick
    // sequence like (type → toggle TODO → blur) can hit the
    // backend out of order and the older edit overwrites the newer.
    if (commitInFlight) {
      try {
        await commitInFlight;
      } catch {
        // ignore — we still want our own commit to try
      }
    }
    setSyncing(true);
    const op: Promise<PageView> = withTimeout(
      editBlock(pid, id, text),
      EDIT_TIMEOUT_MS,
      "Save is taking too long",
    );
    commitInFlight = op;
    const next = await withError(() => op);
    if (commitInFlight === op) commitInFlight = null;
    setSyncing(false);
    if (next) {
      // Only drop out of edit mode once the backend confirmed the
      // save. If it failed, `withError` already surfaced the
      // message and we leave the editor open with the draft intact
      // so the user can retry instead of silently losing the text.
      setEditingId(null);
      applyView(next);
    } else if (error()) {
      // Save failed (timeout, backend error, etc). Offer a retry
      // affordance — the draft is still in the editor, so the
      // user's text is not lost.
      setErrorRetry(() => () => {
        void commitEdit();
      });
    }
  }

  /**
   * Apply an external-clipboard markdown paste to the workspace.
   *
   * `BlockRow`'s textarea has already detected via `looksLikeOutline`
   * that the payload deserves the outline → blocks conversion and
   * called `preventDefault` on the original paste event. We commit
   * any in-flight draft first (the host block's text would otherwise
   * race with the paste's `AtCaret` splice), hand the raw text to
   * the backend, then re-apply the resulting `PageView`.
   */
  async function handlePasteMarkdown(blockId: string, caret: number, text: string) {
    const pid = pageId();
    if (!pid) return;
    if (editingId() === blockId) {
      // Flush whatever the user was typing so the splice operates on
      // the workspace state the textarea is showing, not on stale
      // backend text.
      const draftText = draft();
      const committed = await withError(() => editBlock(pid, blockId, draftText));
      if (committed) setView(committed);
    }
    const next = await withError(() => pasteMarkdown(pid, blockId, caret, text));
    if (next) applyView(next);
  }

  async function handleToggleTodo(id: string) {
    const pid = pageId();
    if (!pid) return;
    haptic("medium");
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
      // backend's view, **with** the TODO/DONE prefix reattached so
      // the editor stays consistent with what the user just toggled.
      const block = findBlock(next.outline, id);
      if (block) setDraft(rawTextWithTodo(block));
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
      haptic("warning");
      setPendingDelete({ id, descendants });
      return;
    }
    haptic("heavy");
    void performDelete(id);
  }

  async function performDelete(id: string) {
    const pid = pageId();
    if (!pid) return;
    if (editingId() === id) setEditingId(null);
    const next = await withError(() => deleteBlock(pid, id));
    if (next) applyView(next);
  }

  /**
   * Flip the collapsed flag on a block. The backend generates
   * `Op::SetCollapsed`, applies it through the op log (same path as
   * every other mutation), and returns a fresh page view so the
   * renderer picks up the new flag in the same frame the user tapped
   * the triangle. The sidecar is not touched — fold state syncs
   * device-to-device via the per-actor jsonl, not the `.outl` file.
   */
  async function handleToggleCollapse(id: string, next: boolean) {
    const pid = pageId();
    if (!pid) return;
    haptic("light");
    const updated = await withError(() => setBlockCollapsed(pid, id, next));
    if (updated) applyView(updated);
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

  /**
   * Run a `\`\`\`lang …\`\`\`` block through `outl-exec`. Triggered
   * from the long-press context menu (the only "Run code" surface on
   * mobile — desktop has Cmd+X too). The backend persists the
   * `> **result:**` subblock and returns the refreshed `PageView`,
   * so a single round-trip swaps the outline in. Runtime errors
   * (`unknown language`, `timeout`) surface via the toast so the
   * user knows why nothing visibly happened.
   */
  async function handleRunCodeBlock(id: string) {
    const pid = pageId();
    if (!pid) return;
    haptic("medium");
    const reply = await withError(() => runCodeBlock(pid, id));
    if (!reply) return;
    applyView(reply.view);
    if (reply.error) {
      setError(`${reply.language}: ${reply.error}`);
    }
  }

  async function handleCreateAfter(id: string) {
    const pid = pageId();
    if (!pid) return;
    haptic("light");
    // Keep keyboard up across the async create by parking focus on
    // the ghost textarea first.
    focusGhost();
    if (editingId()) await commitEdit();
    const reply = await withError(() =>
      createBlock(pid, { afterId: id, text: null }),
    );
    if (reply) {
      applyView(reply.view);
      startEdit(reply.new_id, "");
    }
  }

  async function handleAppendBlock() {
    const pid = pageId();
    if (!pid) return;
    focusGhost();
    if (editingId()) await commitEdit();
    haptic("medium");
    const reply = await withError(() =>
      createBlock(pid, { afterId: null, parentId: null, text: null }),
    );
    if (reply) {
      applyView(reply.view);
      startEdit(reply.new_id, "");
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

  /**
   * Calendar picked a day. The backend's `open_journal_for` opens-or-
   * creates the journal page, so picking a day that has never been
   * visited still lands on a fresh page ready for the user to type
   * into — no "page doesn't exist" error.
   */
  async function handlePickDate(slug: string) {
    setCalendarOpen(false);
    haptic("light");
    const next = await withError(() => openJournalFor(slug));
    if (next) applyView(next);
  }

  async function handleRefClick(target: string) {
    // One Tauri call — `openRef` runs the journal-vs-page decision
    // tree on the Rust side and creates the page if nothing exists,
    // so this handler has no branching to keep in sync with the
    // backend. Used to be three commands gated by a `^\d{4}-\d{2}-\d{2}$`
    // regex, which surfaced `invalid date slug` toasts on inputs
    // like `[[2026-13-01]]` (regex shape OK, semantic parse fails).
    haptic("light");
    const next = await withError(() => openRef(target));
    if (next) applyView(next);
  }

  async function handleTagClick(tag: string) {
    // `#foo` arrives as `#foo`; strip the leading hash and route
    // through the same `openRef` decision tree as `[[foo]]`.
    const target = tag.startsWith("#") ? tag.slice(1) : tag;
    if (!target) return;
    haptic("light");
    const next = await withError(() => openRef(target));
    if (next) applyView(next);
  }

  function handleLinkClick(href: string) {
    // External `[label](url)` → open in the system browser via the
    // shared opener wrapper (scheme-guarded to http(s)/mailto). Mirrors
    // desktop; errors surface on the same status line as everything
    // else instead of throwing into the tap handler.
    haptic("light");
    void openExternalUrl(href).catch((e) => {
      setError(e instanceof Error ? e.message : String(e));
    });
  }

  async function handlePickPage(slug: string, kind: "page" | "journal") {
    setSwitcherOpen(false);
    haptic("light");
    const next =
      kind === "journal"
        ? await withError(() => openJournalFor(slug))
        : await withError(() => openPageBySlug(slug));
    if (next) applyView(next);
  }

  /**
   * Insert a snippet (or open/close pair) into the active textarea
   * synchronously so iOS keeps the keyboard up across the change.
   *
   * Uses the `spliceText` + double `parkCaret` pattern (see
   * `lib/textarea.ts`) so the caret lands at the intended spot
   * even when Solid's `value={draft()}` binding effect fires later
   * and would otherwise jump the caret to the end.
   */
  function insertAtCursor(
    mode: "text" | "pair",
    open: string,
    close: string = "",
  ) {
    const el = activeTextarea;
    if (!el) return;
    const start = el.selectionStart ?? el.value.length;
    const end = el.selectionEnd ?? el.value.length;
    const insert = mode === "pair" ? open + close : open;
    const targetCaret =
      mode === "pair" ? start + open.length : start + insert.length;

    spliceText(el, start, end, insert);
    parkCaret(el, targetCaret);
    setDraft(el.value);
    parkCaret(el, targetCaret);
  }

  function wrapSelection(style: "bold" | "italic" | "code") {
    const el = activeTextarea;
    if (!el) return;
    const start = el.selectionStart ?? el.value.length;
    const end = el.selectionEnd ?? el.value.length;
    const wrap = style === "bold" ? "**" : style === "italic" ? "*" : "`";
    const selected = el.value.slice(start, end);
    const insert = `${wrap}${selected}${wrap}`;
    spliceText(el, start, end, insert);
    const targetCaret = start + insert.length;
    parkCaret(el, targetCaret);
    setDraft(el.value);
    parkCaret(el, targetCaret);
  }

  return (
    <div class="flex h-full flex-col">
      {/* Bear-style chrome: header background stays as a soft blur over
          the canvas, with no divider underneath. Actions sit inside
          two floating capsules (left = back, right = grouped icons)
          so the title can breathe in the middle. */}
      <header
        class="z-30 shrink-0 bg-(--color-ios-bg)/80 px-3 pt-2 pb-3 backdrop-blur-xl dark:bg-(--color-iosd-bg)/80"
        style="padding-top: max(env(safe-area-inset-top), 12px);"
      >
        <div class="grid grid-cols-[auto_1fr_auto] items-center gap-2">
          {/* Left capsule — visible only when the user has navigated
              away from today's journal. We always reserve a placeholder
              of the same width so the title doesn't jump horizontally
              when the back button appears / disappears. */}
          <Show
            when={view() && view()!.page.kind !== "journal"}
            fallback={<span aria-hidden="true" class="block h-9 w-10" />}
          >
            <div class="inline-flex rounded-full bg-(--color-ios-card)/85 shadow-[var(--shadow-capsule)] backdrop-blur-xl dark:bg-(--color-iosd-card)/85 dark:shadow-[var(--shadow-capsule-dark)]">
              <button
                type="button"
                aria-label="Back to today's journal"
                onClick={handleJumpToday}
                class="flex h-9 w-10 items-center justify-center rounded-full text-(--color-ios-accent) active:bg-(--color-ios-divider)/40 dark:text-(--color-iosd-accent) dark:active:bg-(--color-iosd-divider)/40"
              >
                <svg
                  width="20"
                  height="20"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  aria-hidden="true"
                >
                  <path d="M9 14L4 9l5-5" />
                  <path d="M4 9h11a5 5 0 0 1 5 5v6" />
                </svg>
              </button>
            </div>
          </Show>

          {/* Center — title region. `min-w-0` is what lets the inner
              truncate work in PageHeader. */}
          <div class="min-w-0">
            <Show
              when={view()?.page.kind === "journal"}
              fallback={
                <PageHeader
                  title={view()?.page.title ?? ""}
                  kind={view()?.page.kind ?? null}
                />
              }
            >
              <JournalHeader
                slug={view()?.page.slug ?? ""}
                todaySlug={todaySlugValue()}
                onPrev={handlePrevDay}
                onNext={handleNextDay}
                onToday={handleJumpToday}
              />
            </Show>
          </div>

          {/* Right capsule — grouped page actions. SyncDot lives inline
              between pages-search and refresh so the user reads it as
              "status of the data this capsule controls". */}
          <div class="inline-flex items-center rounded-full bg-(--color-ios-card)/85 shadow-[var(--shadow-capsule)] backdrop-blur-xl dark:bg-(--color-iosd-card)/85 dark:shadow-[var(--shadow-capsule-dark)]">
            <button
              type="button"
              aria-label="Calendar"
              onClick={() => {
                haptic("light");
                setCalendarOpen(true);
              }}
              class="flex h-9 w-10 items-center justify-center rounded-full active:bg-(--color-ios-divider)/40 dark:active:bg-(--color-iosd-divider)/40"
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
                aria-hidden="true"
              >
                <rect x="3" y="4" width="18" height="18" rx="3" />
                <path d="M3 10h18M8 2v4m8-4v4" />
              </svg>
            </button>
            <button
              type="button"
              aria-label="Pages"
              onClick={() => {
                haptic("light");
                setSwitcherOpen(true);
              }}
              class="flex h-9 w-10 items-center justify-center rounded-full active:bg-(--color-ios-divider)/40 dark:active:bg-(--color-iosd-divider)/40"
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
                aria-hidden="true"
              >
                <path d="M21 21l-4.3-4.3M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16z" />
              </svg>
            </button>
            <span class="px-1.5">
              <SyncDot
                status={
                  !online() ? "offline" : syncing() ? "syncing" : "synced"
                }
              />
            </span>
            <button
              type="button"
              aria-label="Sync from iCloud"
              onClick={handleRefresh}
              class="flex h-9 w-10 items-center justify-center rounded-full active:bg-(--color-ios-divider)/40 dark:active:bg-(--color-iosd-divider)/40"
            >
              <svg
                width="18"
                height="18"
                viewBox="0 0 24 24"
                fill="none"
                stroke="var(--color-ios-accent)"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                style={{
                  transform: refreshing() ? "rotate(360deg)" : "rotate(0deg)",
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
        <div class="min-h-[60vh]">
        <section class="mt-1 pb-1">
          <Show
            when={loaded() && view() && view()!.outline.length > 0}
            fallback={
              <Show when={loaded()} fallback={<SkeletonOutline />}>
                <Show
                  when={loadFailed()}
                  fallback={
                    <button
                      type="button"
                      onClick={handleAppendBlock}
                      class="flex w-full flex-col items-center px-5 py-16 text-center active:opacity-50"
                    >
                      <svg
                        width="44"
                        height="44"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.5"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        class="mb-3 text-(--color-ios-text-tertiary) dark:text-(--color-iosd-text-tertiary)"
                        aria-hidden="true"
                      >
                        <path d="M12 20h9" />
                        <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4 12.5-12.5z" />
                      </svg>
                      <p class="text-[15px] text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
                        Nothing here yet.
                      </p>
                      <p class="mt-1 text-[13px] text-(--color-ios-accent) dark:text-(--color-iosd-accent)">
                        Tap to start writing
                      </p>
                    </button>
                  }
                >
                  <div class="flex flex-col items-center px-5 py-12 text-center">
                    <p class="text-[15px] text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
                      Couldn't open the workspace.
                    </p>
                    <button
                      type="button"
                      onClick={() => {
                        setLoaded(false);
                        void loadTodayWithRetry();
                      }}
                      class="mt-3 rounded-full bg-(--color-ios-accent) px-5 py-2 text-[14px] font-medium text-white active:opacity-70 dark:bg-(--color-iosd-accent)"
                    >
                      Retry
                    </button>
                  </div>
                </Show>
              </Show>
            }
          >
            <ParseWarningsBanner warnings={view()!.warnings ?? []} />
            <For each={view()!.outline}>
              {(block) => (
                <BlockRow
                  block={block}
                  depth={0}
                  editingId={editingId()}
                  draftText={draft}
                  onStartEdit={startEdit}
                  onDraftChange={setDraft}
                  onCommitEdit={commitEdit}
                  onToggleTodo={handleToggleTodo}
                  onDelete={handleDelete}
                  onIndent={handleIndent}
                  onOutdent={handleOutdent}
                  onCreateAfter={handleCreateAfter}
                  onToggleCollapse={handleToggleCollapse}
                  onContextMenu={(id) => setContextMenuBlockId(id)}
                  onRefClick={handleRefClick}
                  onTagClick={handleTagClick}
                  onLinkClick={handleLinkClick}
                  onPasteMarkdown={handlePasteMarkdown}
                  onTextareaMount={(el) => {
                    activeTextarea = el;
                    setActiveTextareaSignal(el);
                  }}
                />
              )}
            </For>
          </Show>
        </section>

        {/* Always render the section for non-journal pages so the
            bidirectional-linking concept is discoverable; journals
            stay hidden when empty (the daily flow is already busy
            enough without an empty box every day). */}
        <Show
          when={
            view() &&
            (view()!.backlinks.length > 0 || view()!.page.kind === "page")
          }
        >
          <BacklinksSection
            backlinks={view()!.backlinks}
            onJump={async (link) => {
              if (!link.source_page) return;
              haptic("light");
              const sp = link.source_page;
              const next =
                sp.kind === "journal"
                  ? await withError(() => openJournalFor(sp.slug))
                  : await withError(() => openPageBySlug(sp.slug));
              if (next) applyView(next);
            }}
          />
        </Show>
        </div>
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

      <Toast
        message={error()}
        onRetry={errorRetry() ?? undefined}
        onDismiss={() => {
          setError(null);
          setErrorRetry(null);
        }}
      />

      <PageSwitcher
        open={switcherOpen()}
        currentSlug={view()?.page.slug ?? null}
        onClose={() => setSwitcherOpen(false)}
        onPick={handlePickPage}
      />

      <Calendar
        open={calendarOpen()}
        selectedSlug={
          view()?.page.kind === "journal" ? (view()?.page.slug ?? null) : null
        }
        todaySlug={todaySlugValue()}
        onClose={() => setCalendarOpen(false)}
        onPick={handlePickDate}
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

      <BlockContextMenu
        open={contextMenuBlockId() !== null}
        onClose={() => setContextMenuBlockId(null)}
        actions={buildContextActions(
          contextMenuBlockId(),
          view(),
          {
            indent: handleIndent,
            outdent: handleOutdent,
            moveUp: handleMoveUp,
            moveDown: handleMoveDown,
            toggleTodo: handleToggleTodo,
            delete: handleDelete,
            runCode: handleRunCodeBlock,
            copy: async (id) => {
              const block = view() ? findBlock(view()!.outline, id) : null;
              if (!block) return;
              try {
                await navigator.clipboard?.writeText(rawTextWithTodo(block));
              } catch {
                // Some webviews refuse navigator.clipboard outside a
                // user gesture chain; failing silently is acceptable.
              }
            },
          },
        )}
      />

    </div>
  );
}

function JournalHeader(props: {
  slug: string;
  /** Today's slug, resolved once by the parent `Journal` so the header
   *  and the "back to today" button share a single source of truth.
   *  `null` while the parent is still resolving it. */
  todaySlug: string | null;
  onPrev: () => void;
  onNext: () => void;
  onToday: () => void;
}) {
  const isToday = () =>
    props.todaySlug !== null && props.todaySlug === props.slug;
  return (
    <div class="flex-1">
      <div class="flex items-center justify-center gap-2">
        <button
          type="button"
          aria-label="Previous day"
          onClick={props.onPrev}
          class="rounded-full p-1 text-(--color-ios-accent) active:opacity-50 dark:text-(--color-iosd-accent)"
        >
          <ChevronLeft />
        </button>
        <h1
          class="cursor-pointer whitespace-nowrap text-[17px] font-semibold leading-tight tracking-tight tabular-nums active:opacity-60"
          onClick={props.onToday}
        >
          {props.slug}
        </h1>
        <button
          type="button"
          aria-label="Next day"
          onClick={props.onNext}
          class="rounded-full p-1 text-(--color-ios-accent) active:opacity-50 dark:text-(--color-iosd-accent)"
        >
          <ChevronRight />
        </button>
      </div>
      {/* Always rendered (just hidden when not today) so the header
          keeps the same height across day navigation — otherwise the
          whole outline below jumps by ~14px every time the user pages
          past today, which reads as the header "dancing". */}
      <p
        class="mt-0.5 text-center text-[11px] font-medium uppercase tracking-[0.08em] text-(--color-ios-accent) dark:text-(--color-iosd-accent)"
        classList={{ invisible: !isToday() }}
        aria-hidden={!isToday()}
      >
        Today
      </p>
    </div>
  );
}

function PageHeader(props: { title: string; kind: "page" | "journal" | null }) {
  return (
    <div class="min-w-0 text-center">
      <p class="text-[11px] font-medium uppercase tracking-wider text-(--color-ios-text-tertiary) dark:text-(--color-iosd-text-tertiary)">
        {props.kind === "journal" ? "Journal" : "Page"}
      </p>
      <h1 class="truncate text-[17px] font-semibold leading-tight tracking-tight">
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

/**
 * Wire the long-press block id into a typed action list for
 * `<BlockContextMenu>`. Each action carries an SVG path, label, and
 * a guard (`enabled`) so we hide "Move up" on the first sibling and
 * "Move down" on the last — gestures iOS users expect to disappear
 * when they don't apply.
 *
 * The handlers are passed in from `Journal()`'s scope so the menu
 * doesn't have to import every Tauri command directly.
 */
function buildContextActions(
  blockId: string | null,
  pageView: import("@outl/shared/api/types").PageView | null,
  handlers: {
    indent: (id: string) => void;
    outdent: (id: string) => void;
    moveUp: (id: string) => void;
    moveDown: (id: string) => void;
    toggleTodo: (id: string) => void;
    delete: (id: string) => void;
    runCode: (id: string) => void;
    copy: (id: string) => void;
  },
): BlockContextAction[] {
  if (!blockId || !pageView) return [];
  // Resolve sibling position so we can hide move-up/down at the
  // ends. Walking the outline is cheap (the user just long-pressed,
  // there's no per-frame budget here).
  const siblings = locateSiblings(pageView.outline, blockId);
  const index = siblings
    ? siblings.findIndex((b) => b.id === blockId)
    : -1;
  const canMoveUp = index > 0;
  const canMoveDown = siblings ? index < siblings.length - 1 : false;
  // `Run code` only shows up when the long-pressed block is a fenced
  // `` ```lang …``` ``. The same detector the read-mode renderer
  // uses (`@outl/shared/highlight::detectFence`); the backend
  // re-validates on `run_code_block`, so a false-positive here would
  // just surface a runtime error in the toast instead of doing
  // damage.
  const block = findBlock(pageView.outline, blockId);
  const fence = block ? detectFence(block.text) : null;
  return [
    ...(fence
      ? [
          {
            id: "runCode",
            label: `Run ${fence.language}`,
            // SF-Symbols-equivalent "play.fill" — filled right
            // triangle, matches the desktop's `▶ Run` chip.
            iconPath: "M8 5v14l11-7z",
            onSelect: () => handlers.runCode(blockId),
          } satisfies BlockContextAction,
        ]
      : []),
    {
      id: "toggleTodo",
      label: "Toggle TODO",
      iconPath: "M5 12l4 4 10-10",
      onSelect: () => handlers.toggleTodo(blockId),
    },
    {
      id: "copy",
      label: "Copy text",
      iconPath:
        "M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2 M9 2h6a1 1 0 0 1 1 1v2a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1V3a1 1 0 0 1 1-1z",
      onSelect: () => handlers.copy(blockId),
    },
    {
      id: "indent",
      label: "Indent",
      iconPath: "M3 5h12M3 12h8M3 19h12M15 9l3 3-3 3",
      onSelect: () => handlers.indent(blockId),
    },
    {
      id: "outdent",
      label: "Outdent",
      iconPath: "M3 5h12M3 12h8M3 19h12M21 9l-3 3 3 3",
      onSelect: () => handlers.outdent(blockId),
    },
    {
      id: "moveUp",
      label: "Move up",
      iconPath: "M12 19V5M5 12l7-7 7 7",
      enabled: () => canMoveUp,
      onSelect: () => handlers.moveUp(blockId),
    },
    {
      id: "moveDown",
      label: "Move down",
      iconPath: "M12 5v14M19 12l-7 7-7-7",
      enabled: () => canMoveDown,
      onSelect: () => handlers.moveDown(blockId),
    },
    {
      id: "delete",
      label: "Delete",
      iconPath:
        "M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m-9 0v14a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2V6",
      destructive: true,
      onSelect: () => handlers.delete(blockId),
    },
  ];
}

/** DFS for the sibling list containing `targetId`. Returns the
 *  block array (not the parent) so the caller can use `findIndex`
 *  without an extra walk. */
function locateSiblings(
  forest: import("@outl/shared/api/types").BlockNode[],
  targetId: string,
): import("@outl/shared/api/types").BlockNode[] | null {
  for (const node of forest) {
    if (node.id === targetId) return forest;
    const inner = locateSiblings(node.children ?? [], targetId);
    if (inner) return inner;
  }
  return null;
}
