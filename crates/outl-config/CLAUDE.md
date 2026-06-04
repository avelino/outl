# CLAUDE.md — outl-config

Shared user-config crate for every outl client.
**One file in one place** — `~/.config/outl/config.toml` — read by the TUI, the CLI, and the desktop app via this same module.
Read this before adding a field.

## Why this crate exists

Before this crate, the desktop wrote settings to `~/Library/Application Support/app.outl.desktop/settings.json` (JSON, macOS-only path) and the TUI carried per-workspace state in `<workspace>/.outl/config.toml`.
Two readers, two writers, two schemas — flipping a theme in the desktop did nothing for the TUI on the next launch.
This crate ends that: TOML, XDG-style on every OS (including macOS), one schema, both clients import the same `Config` struct.

## Hard rule

**No client parses or writes `config.toml` by hand.**
Every read goes through [`load`] / [`load_from`]; every write goes through [`save`] / [`save_to`].
Bypassing this crate is how schema drift starts.

The desktop's `settings.rs` is the canonical adapter pattern: a flat wire-format struct for the frontend, converted via `From` impls in and out of `outl_config::Config`.
If a new client needs a different shape on the wire, do the same — adapt, don't fork the reader.

## Path layout

```
~/.config/outl/                         ← `config_dir()` (XDG-style on every OS)
├── config.toml                         ← `config_path()`
└── actor                               ← the desktop's per-machine ULID (NOT this crate's concern)
```

- macOS / Linux: respects `$XDG_CONFIG_HOME` first, else `~/.config/outl/`.
- Windows: `%APPDATA%\outl\config.toml`.
- **Not** `~/Library/Application Support/…` on macOS — deliberate (see lib doc).

The `actor` file next to `config.toml` is **not** part of this crate's schema; it's the desktop's device identity (a ULID) and is read directly by `outl-desktop/src-tauri/src/lib.rs`. Don't add `actor` to `Config` — actors belong with the workspace they write to, not with user preferences.

## Schema

```toml
[workspace]
last = "/Users/me/iCloud/outl"   # absolute path; optional

[theme]
preset = "outl"                   # name from outl_theme::PRESETS

[editor]
vim_mode = true                   # default true
font_size = 15                    # pixels, desktop-only
```

Three sections, each modelled as its own struct ([`WorkspaceCfg`], [`ThemeCfg`], [`EditorCfg`]).
`#[serde(default)]` everywhere — a missing field falls back to the type's `Default`, so an older binary reading a newer config doesn't choke and a newer binary reading an older config doesn't blow up.

## Behaviour contract (read this before changing anything)

| Situation | What this crate does |
|---|---|
| File missing | Returns `Config::default()` silently. First launch is normal. |
| File present, empty | Returns `Config::default()`. |
| File present, malformed TOML | Returns `Config::default()` **+ `tracing::warn!`**. Never panics. |
| Unknown field | Ignored. Older binary survives a newer config. |
| Partial section (e.g. only `[theme]` populated) | Other sections fall back to their per-section `Default`. |
| `save()` | Atomic write (`config.toml.tmp` → rename). Creates `~/.config/outl/` if missing. A crash mid-write never leaves a truncated config. |

The forgiving read path is **load-bearing for UX**: a user editing TOML by hand mid-typo doesn't lose every preference; they just see defaults until the next save fixes the file.
Do not make load fail-fast — fail-fast belongs in the workspace itself, not in user preferences.

## Adding a field

1. Add the field to the relevant struct in `src/schema.rs` with `#[serde(default)]` (or a per-type `Default` impl).
2. Update the example in `src/lib.rs`'s module doc.
3. Update `docs/config.md` — the user-facing schema table.
4. Update `crates/outl-cli/CLAUDE.md` and/or `crates/outl-desktop/CLAUDE.md` if a new client now reads the field.
5. Wire the reader in the consuming crate (`outl-tui/src/runtime.rs` for TUI, `outl-desktop/src-tauri/src/settings.rs` for desktop).
6. Add a `tests` case covering the partial-TOML path (only the new section populated) to confirm the default still applies.

If the field is **per-workspace** (not global), it doesn't belong here — it belongs in `<workspace>/.outl/config.toml`, written by `outl-cli`'s `init` command.
If the field **must converge between devices**, it doesn't belong in TOML at all — it goes through the op log (root `CLAUDE.md` invariant #7).

## Where each field is read

| Field | Reader | File |
|---|---|---|
| `workspace.last` | TUI/CLI fallback in `resolve_path`; desktop on boot | `crates/outl-cli/src/main.rs::resolve_path`, `crates/outl-desktop/src-tauri/src/lib.rs::run` |
| `theme.preset` | TUI palette resolver; desktop settings | `crates/outl-tui/src/runtime.rs::resolve_theme`, `crates/outl-desktop/src-tauri/src/commands/theme.rs` |
| `editor.vim_mode` | Desktop only (TUI ignores) | `crates/outl-desktop/src-tauri/src/settings.rs` |
| `editor.font_size` | Desktop only | `crates/outl-desktop/src-tauri/src/settings.rs` |

Update this table whenever a new reader appears.

## What this crate does NOT do

- ❌ Parse the **per-workspace** `<workspace>/.outl/config.toml`. That belongs to `outl-cli::cmd::init` and the workspace-open path; it's a different schema (per-device `actor_id`, workspace-only overrides).
- ❌ Hold the actor ULID. Lives next to `config.toml` as a separate file, owned by the consumer.
- ❌ Provide a settings UI / form schema. Each client renders its own.
- ❌ Validate semantic correctness (does the theme name exist? is the path readable?). Validation is the consumer's job — this crate just round-trips bytes.

## Verify before "done"

```bash
cargo fmt --all
cargo clippy -p outl-config --all-targets -- -D warnings
cargo test -p outl-config
```

If you touched the schema, also smoke the readers:

```bash
cargo test -p outl-tui      # runtime::resolve_theme tests
cargo test -p outl-desktop  # settings round-trip tests
```
