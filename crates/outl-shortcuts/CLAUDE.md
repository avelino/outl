# CLAUDE.md — outl-shortcuts

Single source of truth for outl's keyboard bindings.
**Every client that resolves a key → action goes through this catalog** — TUI today, desktop today, any future GUI tomorrow.

## Why this crate exists

Before this crate, the TUI defined its bindings in `outl-tui/src/input/` and the desktop wired its own `KeyboardEvent` handlers in `lib/shortcuts.ts`.
Result: `Cmd+P` opened the picker on the desktop, `Ctrl+P` did nothing on the TUI for two weeks until somebody noticed.
With both crates pulling from a shared `(chord → action)` table, "the key the user knows" works the same on every surface, and the help overlay on each client can list every binding in one query.

## Hard rule

**No client hardcodes a `(chord, action)` pair.**
Every binding lives in `src/defaults.rs::default_bindings`; clients consume it via `default_bindings()`, `bindings_for_mode(mode)`, or `lookup(mode, chord)`.

If a client needs a new shortcut, it goes here first.
A binding that only the TUI cares about still lives here (with `Mode::Normal` / `Mode::Insert` / `Mode::Visual` / `Mode::Overlay`) — the desktop just won't subscribe to it unless `settings.vim_mode == true`.

## What this crate owns

- **[`Action`]** — every named operation outl performs in response to a key. Tagged-union serde (`{"kind": "OpenToday"}`) so the desktop frontend can `switch` on a string instead of an arbitrary integer.
- **[`Chord`] / [`ChordSequence`]** — modifier-prefixed key combos, expressed independently of any input library so `crossterm::KeyEvent` (TUI) and `KeyboardEvent` (browser DOM) can both map into them.
- **[`Mode`]** — which modal state a binding applies to. `Global` matches everywhere; `Normal` / `Insert` / `Visual` / `Overlay` are the vim modes (desktop subscribes only while `settings.vim_mode == true`).
- **[`Binding`]** — `(mode, chord, action, description)` row.
  The description is what the help overlay displays — keep it short and verb-led ("Open today's journal", not "This shortcut opens today's journal in the current window").
- **[`default_bindings`]** — the canonical table. Hand-curated, ordered for help-overlay readability.
- **[`bindings_for_mode`] / [`lookup`]** — query helpers. `lookup` is `O(n)` over the table; the table is small (under 100 entries today) so we don't bother with a hashmap.

## What this crate does NOT own

- ❌ Handlers. Each client maps `Action -> {do_something}` itself. The TUI's `App::dispatch` and the desktop's `lib/action-handlers.ts` are the dispatchers; this crate doesn't know how a "commit insert buffer" actually commits.
- ❌ Input adapters. `crossterm::KeyEvent → Chord` lives in `outl-tui`; `KeyboardEvent → Chord` lives in `outl-desktop/src/lib/shortcuts.ts`. Both produce a [`Chord`] this crate resolves.
- ❌ User-level overrides (rebinding `i` to `a`). When that ships, it'll go through the same `Vec<Binding>` shape — a user override is just a different source list fed into the same `lookup` algorithm.
- ❌ OS-specific chord rewriting (`Cmd` ↔ `Ctrl`). Each client decides which physical key its `Chord::ctrl(c)` corresponds to on the running OS.

## Adding a binding

1. **Pick the mode honestly.** `Global` only if the chord must fire in every mode — chrome chords (`Cmd+P`, `Cmd+T`) yes; `j` / `k` no (those are `Normal`).
2. Append a `Binding { mode, chord, action, description }` to the relevant section of `default_bindings()`.
3. Run `cargo test -p outl-shortcuts` — `no_duplicate_chord_in_same_mode` catches collisions; `every_binding_has_a_description` catches empty descriptions; `bindings_round_trip_via_serde` catches schema breakage.
4. If the action is **new**, also extend [`Action`] (`src/action.rs`) in the same commit. Group it under the right "intent" section (chrome / navigation / editing / visual / code).
5. Wire the handler on every client that needs it:
   - TUI: `crates/outl-tui/src/runtime/dispatch.rs` (or wherever the action switch lives).
   - Desktop: `crates/outl-desktop/src/lib/action-handlers.ts`.
   A client that doesn't need the action just doesn't add a handler — `lookup` returns `Some(Action::Foo)` and the dispatcher no-ops with a debug log.
6. Update the help table in `crates/outl-desktop/CLAUDE.md` (and `docs/tui.md` if it's a TUI-relevant chord).

## Mode semantics

| Mode | Meaning | Subscribed by |
|---|---|---|
| `Global` | Always active. Chrome chords (`Cmd+P`, `Cmd+T`, `Cmd+,`). | Every client. |
| `Normal` | Vim Normal mode — outline navigation (`j`, `k`, `i`, `o`, `dd`). | TUI always; desktop when `vim_mode == true`. |
| `Insert` | Inside a textarea / EditBuffer. Movement (`Up`, `Down`, `Left`, `Right`), commit (`Esc`), inline-markdown wrappers (`Cmd+B`, `Cmd+I`). | TUI + desktop. |
| `Visual` | Range selection (vim Visual). | TUI; desktop when `vim_mode == true`. |
| `Overlay` | Picker / settings modal / help is open. | TUI + desktop — used to give the overlay its own escape chord without it colliding with `Normal` mode bindings. |

**`Cmd+B` is the canonical "context-dependent chord" example:**

- `Global` mode → no binding (we want bold-in-insert to take priority).
- `Insert` mode → `WrapBold`.
- `Normal` mode → no binding either (the desktop frontend used to wire it to "toggle sidebar"; that's `Cmd+Shift+E` now to avoid hijacking markdown's universal bold chord — see `crates/outl-desktop/CLAUDE.md` for the rationale).

If you find yourself wanting two different actions on the same chord across modes, the catalog already supports it — just add two `Binding` rows with different `mode` fields and the `no_duplicate_chord_in_same_mode` test will let them through.

## Wire format (Tauri / JSON)

The desktop ships the whole binding table to the frontend on boot via the `list_shortcut_bindings` Tauri command (`crates/outl-desktop/src-tauri/src/commands/shortcuts.rs`).
Serde format is stable and load-bearing:

```json
{
  "mode": "Global",
  "chord": { "chords": [{ "modifiers": "META", "key": { "Char": "p" } }] },
  "action": { "kind": "OpenPicker" },
  "description": "Quick switcher"
}
```

- `Action` uses `#[serde(tag = "kind")]` — the desktop's `switch` over the string discriminant compiles to a fast dispatch in JS.
- `ChordSequence` is `{ "chords": [Chord, …] }` even for single-chord bindings (so the JS side has one shape to handle).
- `Modifiers` ships as a string of `|`-joined flag names (`"META"`, `"META|SHIFT"`).

**Don't change these names** — the desktop frontend lives in pure TypeScript without a `bindgen` step. A field rename here is a silent frontend break.

## Verify before "done"

```bash
cargo fmt --all
cargo clippy -p outl-shortcuts --all-targets -- -D warnings
cargo test -p outl-shortcuts
```

If you added a new `Action` or changed the chord for an existing one:

```bash
cargo test -p outl-tui      # input/* tests + dispatch coverage
bun --filter outl-desktop test  # action-handlers smoke
```

And smoke-test the TUI + desktop manually — the help overlay should list the new entry, the chord should fire, and the dispatcher's debug log should print the resolved action.
