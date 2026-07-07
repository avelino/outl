# Configuration

outl reads two TOML files at launch.
They are read in this order; the second can override fields from the first.

| Layer | Path | Scope | Written by |
|---|---|---|---|
| **Global** | `~/.config/outl/config.toml` | The user's machine — every workspace, every client | The desktop app's Settings modal; you can also edit it by hand |
| **Per-workspace** | `<workspace>/.outl/config.toml` | One workspace only | `outl init` (for the actor id); hand-edit for the rest |

The path layout is **XDG-style on every OS — including macOS**.
outl is keyboard-first and CLI-friendly; the macOS-native `~/Library/Application Support/…` location would split the TUI and desktop into two config files for no real benefit.

The reader for both files is the **`outl-config`** crate (`crates/outl-config/`).
TUI and desktop import the same module so a field can't drift between clients — extending the schema in one place lights up in both.

---

## Global config (`~/.config/outl/config.toml`)

Every field is optional; missing values fall back to the documented default.
A malformed file is logged and replaced with defaults rather than refused to boot — preferences aren't worth blocking the app on.

```toml
# ~/.config/outl/config.toml — full example with every supported field

[workspace]
# Absolute path to the workspace the user last opened. The desktop
# writes this on every `set_workspace` call; the TUI / CLI read it
# when no `--workspace` flag and no positional path is given.
last = "/Users/me/iCloud/outl"

[theme]
# Palette preset name from `outl_theme::PRESETS`.
# Choices: "outl" (default), "default-dark", "light", "dracula",
#          "solarized-dark", "nord", "monokai".
preset = "outl"

[editor]
# Vim-style modal bindings (Normal / Insert / Visual). Defaults to
# `true` — outl is keyboard-first. The desktop honours this; the TUI
# is vim-style by definition and ignores the flag.
vim_mode = true

# Outline font size in pixels (desktop only — the TUI uses your
# terminal font).
font_size = 15

[calendar]
# Optional IANA timezone name for the journal date + status-line clock.
# Omit (the default) to use the operating system's local timezone.
# Set it when the OS clock runs in the wrong zone — containers and
# Chrome OS Crostini report UTC regardless of where you are (issue #107).
timezone = "Europe/London"

[sync]
# Which transport moves the per-actor op log between devices.
#   "iroh" (default) — direct P2P over QUIC (hole punching + relay).
#   "file"           — iCloud Drive / shared filesystem. Zero infra opt-out.
# Missing [sync] falls back to "iroh" — P2P is outl's primary sync.
transport = "iroh"

# Optional relay URL for the "iroh" transport. Empty (or omitted)
# means use iroh's n0 default relays. Ignored by the "file" transport.
relay_url = ""
```

### Field reference

#### `[workspace]`

| Field | Type | Default | Read by | Effect |
|---|---|---|---|---|
| `last` | absolute path | _none_ | desktop, TUI, CLI | Where the next `outl` (with no args) opens. The desktop persists this on every workspace switch. If the path no longer exists, every reader silently falls through to its next fallback (CLI: cwd; desktop: workspace picker). |

#### `[theme]`

| Field | Type | Default | Read by | Effect |
|---|---|---|---|---|
| `preset` | string | `"outl"` | TUI, desktop | Active palette. Unknown names fall through to `outl`. |

Available presets: `outl`, `default-dark`, `light`, `logseq-light`, `dracula`, `solarized-dark`, `nord`, `monokai`.
See [theming.md](theming.md) for the look of each.

#### `[editor]`

| Field | Type | Default | Read by | Effect |
|---|---|---|---|---|
| `vim_mode` | bool | `true` | desktop | When `false`, the desktop drops the modal `Normal / Insert / Visual` model and only listens to OS-standard chrome chords (`⌘P`, `⌘B`, …). The TUI is vim-style by definition and ignores this. |
| `font_size` | integer (pixels) | `15` | desktop | Outline body font size. The TUI uses the user's terminal font; setting this has no effect there. |

#### `[sync]`

| Field | Type | Default | Read by | Effect |
|---|---|---|---|---|
| `transport` | `"iroh"` \| `"file"` | `"iroh"` | every client (TUI / desktop / mobile / MCP) | Which transport ships each device's `ops-<actor>.jsonl` to the others. `"iroh"` opens direct P2P QUIC connections to paired peers; `"file"` is the opt-out that relies on iCloud Drive / a shared filesystem. Missing `[sync]` defaults to iroh (P2P is the primary sync). |
| `relay_url` | string (URL) | _empty_ | TUI peer-sync wiring | iroh relay used for NAT traversal + fallback. Empty means iroh's n0 public relays. Ignored when `transport = "file"`. See [relay.md](relay.md). |

#### `[snapshot]`

| Field | Type | Default | Read by | Effect |
|---|---|---|---|---|
| `enabled` | bool | `true` | TUI / desktop / mobile | Master switch for materialised-state snapshots on disk. The CLI ignores this (always off — its work is ephemeral). When `true`, `Workspace::apply` writes a snapshot every `op_threshold` ops so the next boot skips the full op-log replay. |
| `op_threshold` | integer (ops) | `10_000` | TUI / desktop / mobile | How many ops between snapshot writes. Lower = faster boot, more disk churn; higher = less churn, slower boot. |

#### `[storage]`

| Field | Type | Default | Read by | Effect |
|---|---|---|---|---|
| `lru_cap` | integer (ops) | `20_000` | TUI / desktop / mobile | Maximum number of ops held in `JsonlStorage`'s in-memory cache. `0` keeps the legacy unbounded behaviour (every op resident forever). Any positive value caps the cache so RSS stays roughly constant regardless of workspace history; cold ops stay addressable through the per-actor offset index (`ops-<actor>.idx`). Mobile pins this to `min(lru_cap, 5_000)` to stay well under iOS jetsam. See [RFC #137](https://github.com/avelino/outl/issues/137). |

#### `[tui]`

| Field | Type | Default | Read by | Effect |
|---|---|---|---|---|
| `mouse_capture` | bool | `false` | TUI only | When `true`, the TUI captures mouse events: the scroll wheel moves the outline selection, a click selects the block under the pointer, and dragging selects a range that is copied as clean outl markdown to the OS clipboard on release. Default is `false` because capturing the mouse disables the terminal's own text-selection (Shift-drag). The keyboard yank (`yy` / `Y` / Visual `y`) always writes to the clipboard regardless of this flag. |

> The iroh transport also reads `~/.outl/identity.key` (this device's ed25519 keypair, per-machine) and `<workspace>/.outl/peers.json` (the paired-device list, per-graph).
> Those are managed by `outl peer …`, not by this config file — see [sync.md → iroh transport](sync.md#transport-2-iroh-p2p).

---

## Per-workspace config (`<workspace>/.outl/config.toml`)

Written by `outl init`; carries the device's per-workspace identity and (optionally) workspace-scoped overrides.

```toml
# Per-workspace config — auto-generated by `outl init`, can be
# hand-edited.

[workspace]
# The per-device-per-workspace actor id (a ULID). DO NOT copy this
# file between machines: every device needs its own id so the op
# log can attribute writes correctly.
actor_id = "01HKZX9YBPDC5XJZ3R8K2QGM7E"

# Persistent storage backend. JSONL (one append-only `ops-<actor>.jsonl`
# per device) is the ONLY backend, so this is almost always omitted.
# Omitting it means "jsonl" — leave it out unless you have a reason.
storage = "jsonl"

[theme]
# Workspace-only override. When set, takes precedence over the
# global `[theme] preset` while you're inside this workspace.
preset = "monokai"
```

> The `[workspace] actor_id` field **cannot** move to the global config — it's per-device-per-workspace by design.
> A peer's op log identifies writes by this id; sharing it across machines silently breaks convergence.

### `[workspace] storage` and peer sync

| Field | Type | Default | Read by | Effect |
|---|---|---|---|---|
| `storage` | `"jsonl"` | `"jsonl"` (when absent) | TUI | Selects the persistent backend. JSONL is the only one, so the key is normally absent. The TUI treats **absent OR `"jsonl"`** as a shareable workspace and starts its peer-sync threads (the iroh transport + the filesystem poller); only an explicit non-`jsonl` value turns them off. |

This matters because a workspace **created by a GUI client or P2P sync** (not by `outl init`) seeds its `config.toml` without a `storage` line.
The TUI must read that absence as the jsonl default, or it would open such a workspace with **no peer sync at all**.
The symptom was the TUI never receiving a paired phone's edits — the desktop, which the phone had already reached over iroh, wrote those ops to the shared `ops/`, but the TUI never started a poller to notice.
Storage is a trait with one persistent impl (`JsonlStorage`); the `storage` key is a legacy selector from when a second backend was on the table, kept only so an explicit opt-out is still expressible.

---

## Precedence chains

### Workspace path

When you type `outl` (no args):

1. Subcommand-positional path (`outl page get … <PATH>`).
2. Global flag `--workspace <DIR>`.
3. `[workspace] last` from `~/.config/outl/config.toml`.
4. Current working directory (the `cd ~/notes && outl` fallback).

A path from `config.toml` that no longer exists on disk is skipped silently and the chain falls through to the next step.

### Theme

When the TUI / desktop decides which palette to render:

1. `--theme <preset>` CLI flag (TUI only).
2. Per-workspace `[theme] preset` from `<workspace>/.outl/config.toml`.
3. Global `[theme] preset` from `~/.config/outl/config.toml`.
4. Built-in default — `outl`.

Unknown preset names fall through to the next step rather than erroring.

---

## Editing safely

The TOML reader (`outl-config::load`) is **forgiving by design**:

- Missing file → defaults, no warning (first launch is normal).
- Malformed TOML → defaults + a `tracing::warn` log line, the app boots normally.
- Unknown fields → ignored.
  Older binaries reading a newer config don't choke; you can add fields ahead of time.
- Partial schema (e.g. only `[theme]` populated) → other sections fall back to their per-section `Default`.

Saving (`outl-config::save`) writes atomically — the new content lands in `config.toml.tmp` and the file is renamed on top.
A crash mid-write never leaves a truncated config.

---

## Migrating from earlier versions

The desktop briefly stored its settings as JSON at `~/Library/Application Support/app.outl.desktop/settings.json` (and the actor at the same directory).
That path is no longer read.
If you upgrade from one of those builds:

- The desktop picks up `~/.config/outl/config.toml` (creates it on first save).
- The actor ULID at `~/.config/outl/actor` is generated fresh on first run; your local op log keeps writing under the new id.
- Your workspace's `ops/` directory is unchanged — only the **client's** identity rotates, not the workspace's history.

If you want to preserve the old actor id, copy it from the old path into `~/.config/outl/actor` before launching the desktop.
