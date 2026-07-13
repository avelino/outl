/**
 * Most-frequently-used ordering for the mobile edit toolbar.
 *
 * A direct port of `OutlKit/Toolbar/ToolbarMFU.swift`: persist per-device
 * tap counts and re-order the *middle* of the row by count desc on each
 * read, keeping the first/last slots pinned (`newLine` / `done`).
 *
 * The store is `localStorage` (the web equivalent of the iOS bar's
 * `UserDefaults`) under the same versioned key. The two stores are
 * independent — a device driving the native iOS bar and a device driving
 * the web bar keep their own counts — which is correct: MFU is per-device
 * UI state, never a synced value.
 *
 * The pure `orderedMiddleActions(counts)` / `record(action, counts)`
 * overloads take an explicit map so they stay deterministic and testable;
 * the `*FromStore` convenience wrappers read/write `localStorage`.
 */
import {
  DEFAULT_ORDER,
  PINNED_FIRST,
  PINNED_LAST,
  type ToolbarAction,
} from "./actions";

/** Versioned so a future schema change can't misread old shapes.
 *  Same string as the Swift `ToolbarMFU.storageKey`. */
export const MFU_STORAGE_KEY = "outl.toolbar.mfu.v1";

export type ToolbarCounts = Partial<Record<ToolbarAction, number>>;

function isToolbarAction(key: string): key is ToolbarAction {
  return (DEFAULT_ORDER as readonly string[]).includes(key);
}

/** Parse a persisted counts blob. Tolerant of a missing key, malformed
 *  JSON, non-object shapes, unknown action ids, and non-integer values —
 *  any of which yields an empty map rather than throwing. */
export function parseCounts(raw: string | null): ToolbarCounts {
  if (!raw) return {};
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return {};
  }
  if (typeof parsed !== "object" || parsed === null) return {};
  const counts: ToolbarCounts = {};
  for (const [key, value] of Object.entries(parsed as Record<string, unknown>)) {
    if (isToolbarAction(key) && typeof value === "number" && Number.isFinite(value)) {
      counts[key] = Math.trunc(value);
    }
  }
  return counts;
}

/**
 * Pure MFU ordering for the middle (scrollable) range — excludes the two
 * pinned slots. The client renders `[PINNED_FIRST, ...this, PINNED_LAST]`,
 * keeping the pinned buttons static outside the scroll. Mirrors
 * `ToolbarMFU.orderedMiddleActions(counts:)`.
 */
export function orderedMiddleActions(counts: ToolbarCounts): ToolbarAction[] {
  const middle = DEFAULT_ORDER.filter(
    (a) => a !== PINNED_FIRST && a !== PINNED_LAST,
  );
  return [...middle].sort((a, b) => {
    const ca = counts[a] ?? 0;
    const cb = counts[b] ?? 0;
    if (ca !== cb) return cb - ca;
    // Stable tiebreak: original `DEFAULT_ORDER` position.
    return DEFAULT_ORDER.indexOf(a) - DEFAULT_ORDER.indexOf(b);
  });
}

/**
 * Increment `action`'s count in an explicit map (pure). No-op for the
 * pinned actions — their slot is fixed by position, so counting them just
 * wastes storage. Mirrors `ToolbarMFU.record`.
 */
export function record(action: ToolbarAction, counts: ToolbarCounts): ToolbarCounts {
  if (action === PINNED_FIRST || action === PINNED_LAST) return counts;
  return { ...counts, [action]: (counts[action] ?? 0) + 1 };
}

// ── localStorage-backed convenience ──────────────────────────────────
// `localStorage` may be unavailable (SSR, private-mode quotas); every
// wrapper degrades to in-memory defaults rather than throwing, so a
// storage failure never breaks the toolbar.

/** Resolve the `Storage` from whichever global exposes it (the bare
 *  `localStorage` identifier resolves against `window` in the webview
 *  and against the happy-dom global in tests; `globalThis.localStorage`
 *  is undefined in some of those environments). Returns `null` when
 *  storage is unavailable (SSR, private-mode). */
function store(): Storage | null {
  try {
    return typeof localStorage !== "undefined" ? localStorage : null;
  } catch {
    return null;
  }
}

function safeGet(key: string): string | null {
  try {
    return store()?.getItem(key) ?? null;
  } catch {
    return null;
  }
}

function safeSet(key: string, value: string): void {
  try {
    store()?.setItem(key, value);
  } catch {
    // ignore — MFU is best-effort UI polish, not durable state
  }
}

/** Read counts from `localStorage`. */
export function readCountsFromStore(): ToolbarCounts {
  return parseCounts(safeGet(MFU_STORAGE_KEY));
}

/** Increment `action` in `localStorage` and return the new counts. */
export function recordToStore(action: ToolbarAction): ToolbarCounts {
  const next = record(action, readCountsFromStore());
  safeSet(MFU_STORAGE_KEY, JSON.stringify(next));
  return next;
}

/** MFU-ordered middle range read straight from `localStorage`. */
export function orderedMiddleFromStore(): ToolbarAction[] {
  return orderedMiddleActions(readCountsFromStore());
}
