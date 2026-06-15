/**
 * Universal keyboard dispatcher for the desktop client.
 *
 * Pulls the binding catalog from the `outl-shortcuts` Rust crate
 * via the `list_shortcut_bindings` Tauri command, translates each
 * `KeyboardEvent` into a portable [`Chord`], buffers prefix
 * sequences (vim-style `g j`, `d d`), and dispatches the resolved
 * [`Action`] to a single map of handler functions.
 *
 * Same chord vocabulary the TUI uses — so a user who learned
 * `[` / `]` in the terminal sees identical navigation here.
 *
 * Mode detection is DOM-driven:
 *
 * - Picker / settings open → `overlay`
 * - Focus in a `<textarea>` (block editor) → `insert`
 * - Otherwise → `normal`
 *
 * Visual mode is engaged explicitly by `EnterVisual` (TUI parity).
 */

import {
  type Action,
  type Binding,
  type Chord,
  type Key,
  type ShortcutMode,
  MOD_ALT,
  MOD_CTRL,
  MOD_META,
  MOD_SHIFT,
  listShortcutBindings,
} from "./api";

import { appState } from "./store";

// ---------------------------------------------------------------------------
// KeyboardEvent → Chord
// ---------------------------------------------------------------------------

function modsOf(e: KeyboardEvent): number {
  let m = 0;
  if (e.ctrlKey) m |= MOD_CTRL;
  if (e.altKey) m |= MOD_ALT;
  if (e.metaKey) m |= MOD_META;
  if (e.shiftKey) {
    // Convention: Shift modifier on a printable single-char `e.key`
    // is only meaningful when the produced character is a letter
    // (`Shift+I` → `I`, lookup as Char('i') + SHIFT). For symbol
    // characters (`?`, `!`, `@`, …) Shift is "how that symbol is
    // typed on a US layout" and `e.key` already reflects the
    // shifted form — adding the SHIFT bit would prevent matches
    // against bindings like `ch('?')` that the TUI / catalog
    // expresses without the modifier.
    const k = e.key;
    const isSymbolChar = k.length === 1 && !/[a-zA-Z]/.test(k);
    if (!isSymbolChar) m |= MOD_SHIFT;
  }
  return m;
}

/**
 * Translate `e.key` (the DOM's localized key name) to a portable
 * [`Key`]. Returns `null` for keys we don't care about (pure
 * modifier presses, dead keys, etc.) so the caller can skip them.
 */
function eventToKey(e: KeyboardEvent): Key | null {
  switch (e.key) {
    case "Enter":
      return { kind: "Enter" };
    case "Escape":
      return { kind: "Esc" };
    case "Tab":
      return { kind: "Tab" };
    case "Backspace":
      return { kind: "Backspace" };
    case "Delete":
      return { kind: "Delete" };
    case "ArrowUp":
      return { kind: "Up" };
    case "ArrowDown":
      return { kind: "Down" };
    case "ArrowLeft":
      return { kind: "Left" };
    case "ArrowRight":
      return { kind: "Right" };
    case "Home":
      return { kind: "Home" };
    case "End":
      return { kind: "End" };
    case "PageUp":
      return { kind: "PageUp" };
    case "PageDown":
      return { kind: "PageDown" };
    case " ":
      return { kind: "Space" };
  }
  if (/^F\d{1,2}$/.test(e.key)) {
    return { kind: "Function", value: Number(e.key.slice(1)) };
  }
  // Plain printable character — lowercase to match Rust's
  // `Key::char` normalisation. Shift is preserved as a modifier
  // bit, so `Shift+I` lands as `Char('i')` + SHIFT.
  if (e.key.length === 1) {
    return { kind: "Char", value: e.key.toLowerCase() };
  }
  return null;
}

function eventToChord(e: KeyboardEvent): Chord | null {
  const key = eventToKey(e);
  if (!key) return null;
  return { mods: modsOf(e), key };
}

// ---------------------------------------------------------------------------
// Chord / ChordSequence equality
// ---------------------------------------------------------------------------

function keyEq(a: Key, b: Key): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "Char" && b.kind === "Char") return a.value === b.value;
  if (a.kind === "Function" && b.kind === "Function")
    return a.value === b.value;
  return true;
}

function chordEq(a: Chord, b: Chord): boolean {
  return a.mods === b.mods && keyEq(a.key, b.key);
}

function seqEq(a: Chord[], b: Chord[]): boolean {
  return a.length === b.length && a.every((c, i) => chordEq(c, b[i]));
}

function isPrefix(prefix: Chord[], full: Chord[]): boolean {
  return (
    prefix.length < full.length && prefix.every((c, i) => chordEq(c, full[i]))
  );
}

// ---------------------------------------------------------------------------
// Mode detection
// ---------------------------------------------------------------------------

function detectMode(): ShortcutMode {
  if (appState.pickerOpen || appState.settingsOpen || appState.helpOpen)
    return "overlay";
  // `appState.mode` already tracks Normal/Insert/Visual when set
  // explicitly by handlers (e.g. EnterVisual / ExitInsert).
  if (appState.mode === "vim-visual") return "visual";
  if (appState.mode === "vim-normal") return "normal";
  if (appState.mode === "vim-insert") return "insert";
  // DOM fallback for the no-vim-mode user: focus in textarea =
  // insert; otherwise normal.
  const el = document.activeElement;
  if (el instanceof HTMLTextAreaElement || el instanceof HTMLInputElement) {
    return "insert";
  }
  return "normal";
}

// ---------------------------------------------------------------------------
// Action dispatcher
// ---------------------------------------------------------------------------

/**
 * Handler map. Each property is a function the caller supplies that
 * fires when its corresponding [`Action`] resolves. Missing entries
 * fall back to a `console.warn` so the user sees which actions
 * still need wiring during development.
 */
export type ActionHandlers = Partial<
  Record<Action["kind"], () => void | Promise<void>>
>;

async function dispatch(action: Action, handlers: ActionHandlers) {
  const h = handlers[action.kind];
  if (!h) {
    console.warn(`[shortcuts] no handler for action ${action.kind}`);
    return;
  }
  try {
    await h();
  } catch (e) {
    console.error(`[shortcuts] handler ${action.kind} threw`, e);
  }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/**
 * Install the global keydown dispatcher. Returns a cleanup
 * function — call it in `onCleanup` so the listener is detached
 * on remount.
 *
 * `handlers` is a map from `Action["kind"]` to a function. Only
 * the actions the desktop actively supports today need to be
 * filled in; the rest log a warning so we can see which TUI-only
 * actions remain unimplemented.
 */
export async function installShortcuts(
  handlers: ActionHandlers,
): Promise<() => void> {
  const bindings = await loadBindings();

  /** Buffered chord prefix (e.g. user typed `g`, waiting for `j`). */
  let pending: Chord[] = [];
  let pendingTimer: number | undefined;

  function resetPending() {
    pending = [];
    if (pendingTimer !== undefined) {
      window.clearTimeout(pendingTimer);
      pendingTimer = undefined;
    }
  }

  function armPendingTimeout() {
    if (pendingTimer !== undefined) window.clearTimeout(pendingTimer);
    // 1s window to complete a chord prefix; fall back to plain
    // typing afterwards (matches the TUI's chord buffer).
    pendingTimer = window.setTimeout(resetPending, 1000);
  }

  const onKey = async (e: KeyboardEvent) => {
    const chord = eventToChord(e);
    if (!chord) return;

    const mode = detectMode();

    // Per-keystroke trace at `info` level so it shows up in the
    // default DevTools output (no need to toggle Verbose). One line
    // per key — easy to scan when chasing "why didn't my chord fire?".
    console.info(
      `[shortcuts:trace] mode=${mode} mods=${chord.mods} key=${
        chord.key.kind === "Char" ? chord.key.value : chord.key.kind
      }`,
    );

    // In Insert mode without modifiers, only intercept
    // explicit-key actions (Enter / Esc / Tab / Backspace / arrows
    // / Delete). Plain typing of letters / numbers must reach the
    // textarea — otherwise the user can't write anything.
    const isPlainChar = chord.mods === 0 && chord.key.kind === "Char";
    if (mode === "insert" && isPlainChar && pending.length === 0) {
      return;
    }

    const sequence = [...pending, chord];

    // Mode-specific match wins over Global. Without this the catalog
    // declaration order would silently shadow per-mode overrides —
    // e.g. `Cmd+T` is `OpenToday` (Global) but also `ToggleTodo`
    // (Insert), and inside a textarea the Insert binding has to win
    // even though Global is declared first in `defaults.rs`.
    const hit =
      bindings.find((b) => b.mode === mode && seqEq(b.chord, sequence)) ??
      bindings.find((b) => b.mode === "global" && seqEq(b.chord, sequence));
    if (hit) {
      // Only preventDefault when we actually have a handler ready —
      // otherwise we'd swallow legitimate keystrokes (e.g. `Enter`
      // in Insert mode, which the textarea's own onKeyDown handler
      // is the canonical processor for) just because the chord
      // catalog *names* an action with no JS implementation yet.
      const hasHandler = !!handlers[hit.action.kind];
      if (hasHandler) {
        e.preventDefault();
        resetPending();
        console.info(`[shortcuts] ${mode} → ${hit.action.kind}`);
        await dispatch(hit.action, handlers);
      } else {
        console.warn(
          `[shortcuts] ${mode} chord matches ${hit.action.kind} but no JS handler`,
          { chord: sequence },
        );
        resetPending();
      }
      return;
    }

    // Prefix? Buffer and wait for the next keypress.
    const isPrefixOfSomething = bindings.some(
      (b) =>
        (b.mode === "global" || b.mode === mode) && isPrefix(sequence, b.chord),
    );
    if (isPrefixOfSomething) {
      e.preventDefault();
      pending = sequence;
      armPendingTimeout();
      return;
    }

    // Not a match, not a prefix — reset and let the event through.
    resetPending();
  };

  window.addEventListener("keydown", onKey);
  return () => {
    window.removeEventListener("keydown", onKey);
    resetPending();
  };
}

// ---------------------------------------------------------------------------
// Catalog caching
// ---------------------------------------------------------------------------

let cachedBindings: Binding[] | null = null;

async function loadBindings(): Promise<Binding[]> {
  if (cachedBindings) return cachedBindings;
  cachedBindings = await listShortcutBindings();
  return cachedBindings;
}
