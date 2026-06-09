# Clients and shared logic

outl has multiple clients today (TUI, mobile, desktop) and more coming (plugins).
They all sit on top of the same workspace and the same op log.
To keep them honest, we route every workspace operation through one shared crate: **`outl-actions`**, and the TS+Solid frontends share `@outl/shared` (`crates/outl-frontend-shared`) for everything pure (DTO types, `<MarkdownInline />`, paste helpers, autocomplete).

## The stack

```text
┌──────────────────────────────────────────────────────────────┐
│ Clients                                                       │
│   outl-cli  outl-tui  outl-mobile  outl-desktop  …plugins    │
└──────────────────────────────────────────────────────────────┘
                          │
                          ▼
┌──────────────────────────────────────────────────────────────┐
│ outl-actions                                                  │
│   block · tree · todo · journal · outline · page · backlinks  │
│   sync (SyncEngine: reload workspace, reproject page,         │
│         snapshot peer jsonls, scan for orphan .md)            │
└──────────────────────────────────────────────────────────────┘
                          │
                          ▼
┌──────────────────────────────────────────────────────────────┐
│ outl-md          (.md parse/render, sidecar, matching,       │
│                   inline tokens, outline_ops)                │
└──────────────────────────────────────────────────────────────┘
                          │
                          ▼
┌──────────────────────────────────────────────────────────────┐
│ outl-core        (CRDT, op log, storage trait)               │
└──────────────────────────────────────────────────────────────┘
```

## What lives where

| Concern                              | Crate                           |
|--------------------------------------|---------------------------------|
| Op log, tree CRDT, storage trait     | `outl-core`                     |
| `.md` parse / render, sidecar        | `outl-md`                       |
| Workspace mutations (edit, indent, todo, delete, journal render) | `outl-actions` |
| Code-block execution (runtimes + orchestration) | `outl-exec`            |
| Cross-client "run a fence" glue (`run_code_block`) | `outl-actions::exec` |
| TUI: keymaps, modes, overlays, in-flight AST manipulation | `outl-tui`         |
| Mobile: iCloud storage, Tauri commands, Solid frontend | `outl-mobile`         |
| CLI subcommands                      | `outl-cli`                      |

## When to put logic in `outl-actions`

Yes if any of these are true:

- Two or more clients (today or in the next quarter) need the same op.
- The function takes only `Workspace + HlcGenerator` and returns `Result<_, ActionError>`.
- It produces ops by way of `Workspace::apply` — no direct storage writes, no filesystem touches outside `journal::write_md_atomic`.

No if:

- It manipulates client UI state (selection, modes, toasts, focus, keymaps).
- It manipulates an in-flight `Vec<OutlineNode>` that hasn't been parsed back into a workspace yet (those helpers live in `outl-md::outline_ops`, re-exported through a one-liner shim at `outl-tui/src/outline_ops.rs` because the mobile client needs them too — they're workspace-free pure AST manipulation, so they sit in `outl-md` rather than `outl-actions`).
- It's storage-backend-specific (iCloud watcher, future ChronDB) — those implement `outl_core::Storage` in the binary that needs them.

## Surfacing parser warnings on every client

A user can drop a `.md` into the workspace by hand, paste an exported Roam/Logseq tree, or edit a file in vim before outl ever saw it.
When that file doesn't match the outl dialect (e.g. starts with `# heading`, contains a free paragraph, or imports a markdown table), the parser **does not** drop content — it preserves the line as a regular block and records the recovery in `ParsedPage.warnings: Vec<outl_md::ParseWarning>`.

Every client surfaces these warnings to the user instead of pretending the file is clean:

| Client | Surface |
|--------|---------|
| TUI | Banner at the top of the outline + chip in the status line; `?` opens the help overlay with the full list (line number + first 60 chars of `raw`). |
| Mobile / Desktop | `<ParseWarningsBanner>` from `@outl/shared` renders above the outline. Tap a row to scroll to the offending line in the raw view. |
| CLI | `outl doctor` lists every page with warnings and writes a structured row per warning to `.outl/orphans.log`. |

The shared entry point that bundles outline + warnings in one trip is `outl_actions::outline::read_page_outline` (and the workspace-aware variant `read_page_outline_with_workspace`) returning `PageOutline { nodes, warnings }`.
Tauri commands on mobile + desktop expose this directly; the TUI calls it via `lifecycle::load_current`.

The contract is intentionally non-blocking: a file with warnings is still editable, still saves cleanly (render normalises it to `- <raw>` on the next write), and never refuses to load.
Users decide when to clean up; outl never deletes content on their behalf.

## Running code blocks

Every client that lets the user execute a `` ```lang ``` `` block (TUI `g x`, desktop `Cmd+X` / Run button, mobile long-press → "Run code") goes through **one** shared entry point: `outl_actions::exec::run_code_block(ws, hlc, root, registry, page, block)`.

```text
client gesture (TUI chord / Cmd+X / long-press)
        │
        ▼
outl_actions::exec::run_code_block
   ├── outl_actions::flat_index_for_block   (DFS-locate the block)
   ├── outl_actions::journal::page_md_path  (resolve .md path)
   └── outl_exec::run_block_at_index        (execute + persist > **result:** sibling)
        │
        ▼
RunCodeBlockOutcome { language, result_ok | error }
        │
        ▼
client wraps with refreshed PageView and ships it down its Tauri/TUI surface
```

The DTO returned is intentionally *narrow* — `language`, `result_ok` (stdout/stderr/duration/exit), `error`.
Clients add the refreshed page projection themselves because each client owns its own `PageView` shape (mobile's iCloud-backed variant differs from desktop's path-picker variant).
The duplication that used to live in `outl-desktop/src-tauri/src/commands/exec.rs` and `outl-mobile/src-tauri/src/exec.rs` was collapsed into this single function — `flat_index_for_block` and the path lookup were the canonical "two parallel implementations" case the workspace-level Reuse-first policy exists to prevent.

The runtime catalog is selected per-binary via `outl-exec` features:

- `outl-cli`, `outl-tui`, `outl-desktop` — default features (Lisp + JS + Python + Lua + Rust via wasmtime).
- `outl-mobile` — opts out of `lang-rust` (wasmtime is heavy and trips iOS code-signing restrictions on dynamic code generation).
- `outl-actions` — `default-features = false` so it never drags `wasmtime` into the mobile IPA via the back door.

## TODO/DONE convention

A block's TODO state is **a prefix on its text**, not a property:

```
"foo"             plain block
"TODO foo"        open task
"DONE foo"        completed task
```

This is the wire format the TUI already uses and what `.md` files contain when synced to other tools.
`outl-actions::cycle_todo` walks `None → TODO → DONE → None`.
UI surfaces parse the prefix out via `split_todo` so they can render a checkbox.

## Blockquote convention

Blockquotes follow the same shape as TODO/DONE — a per-block text prefix, no AST field, every client renders its own visual.
The prefix is the CommonMark `"> "` (greater-than + single space), so an `.md` round-trips cleanly when an external tool opens it:

```
"foo"             plain block
"> foo"           quoted block
```

`outl-actions::quote::toggle_quote` flips the prefix on/off; `split_quote` separates the marker from the body for UI rendering.
Multi-line quote bodies keep the `> ` on every continuation line so the `.md` stays a valid CommonMark blockquote.
Children of a quoted block are **not** implicitly quoted — the marker lives on the block, not on its subtree.
Inline tokens (`**bold**`, `[[ref]]`, `#tag`, `((blk-…))`) continue to tokenize **inside** the body — the wrapper is transparent.

## iCloud sync (mobile + TUI, today)

The iOS app is on a public TestFlight beta — <https://testflight.apple.com/join/P2GdWAMd>.
Install it on the iPhone, then point the TUI at the same iCloud Drive container to share the workspace.

The mobile client persists the op log to the iCloud Ubiquity Container.
The TUI reaches the same workspace by pointing `--workspace` at the container's `Documents/` directory:

```
<container>/Documents/                ← TUI: outl --workspace "<container>/Documents"
├── journals/
│   └── YYYY-MM-DD.md                 ← daily journal projection
├── pages/
│   ├── <slug>.md                     ← regular page projection
│   └── <slug>.outl                   ← sidecar (block IDs + hashes)
└── ops/
    ├── ops-<this_device>.jsonl       ← only this device writes here
    ├── ops-<other_device>.jsonl
    └── ...
```

> The folder is **`ops/`**, not `.ops/`. iCloud Documents / Ubiquity Containers do not sync paths starting with `.` across devices, so a dotted name silently breaks multi-device sync.
> The same rule is why the sidecar moved from `.foo.outl` to `foo.outl` in v0.

Each device only writes to its own `ops-<actor>.jsonl`, so iCloud never has to merge file contents — the CRDT does that work after reading every actor's ops.
The `.md` projection is rewritten after every mutation; do **not** parse it back to reconstruct workspace state, the op log is authoritative.

### Shared sync engine

Both clients use `outl_actions::SyncEngine` for the reload-workspace + reproject-page flow.
Detection is client-specific (TUI runs a worker thread polling `snapshot_peers()` every ~2s; mobile registers `NSMetadataQuery` on the ubiquity container).
Once detection fires, the call site is identical:

```rust
let engine = SyncEngine::new(workspace_root, actor);
let fresh = engine.reload_workspace()?;
engine.reproject_page(&fresh, focused_page_id)?;
```

The TUI defers the reload while the user is in Insert mode (the in-flight `ParsedPage` would be clobbered) via a `pending_reload` flag drained on commit.
Mobile applies immediately because every mutation is one atomic Tauri command.
The policy diverges; the engine does not.

`engine.scan_for_orphans()` is the other shared piece: it walks `journals/` and `pages/` for `.md` files whose sidecar is missing or stale (fresh import from Roam/Logseq, peer-shipped projection without sidecar, external vim edit).
The TUI runs the scan every 10s on a worker thread; mobile runs it once at boot.
Both feed the same `outl_md::reconcile::reconcile_md`.

See `crates/outl-mobile/CLAUDE.md` for the full bundle ID, signing team, container ID set required to build it, and the `NSFileCoordinator`-based peer-file materialisation step that has to run before any read of a peer `ops-*.jsonl`.

## Opening a page from a user-typed ref

When a click on `[[avelino/outl]]`, `#code-review`, `[[2026-06-04]]`
or a picker field hands a client a string, the client must not split
the "journal vs page" decision between a frontend regex and a
backend parser. The two will drift. They already did:
`[[2026-13-01]]` matched the mobile frontend's `^\d{4}-\d{2}-\d{2}$`
shape regex, the command then fed `2026-13-01` into the strict date
parser, and the user got an `invalid date slug` toast for what
should have been a regular page.

The canonical entry point is
`outl_actions::page::open_or_create_by_ref(target)`. It runs the
whole decision tree in one place:

1. Date-shaped target → journal (semantic validator, not the regex
   shape — `2026-13-01` falls through).
2. Literal slug match → existing page (clean slug from the picker).
3. Slugified slug match → existing page (`[[avelino/outl]]` finds
   `pages/avelino-outl.md` even if the ref was typed before the
   page existed).
4. Case-insensitive title match → existing page.
5. Fallback: create a fresh page via `open_or_create_by_name`
   (slugifies disk path, keeps the typed string as title).

Every client that turns a tap on a ref / tag / picker entry into a
page view should wrap this single helper. There is no client-side
discrimination to maintain. The
`open_or_create_by_name(name, kind)` variant stays for callers that
already know they want a regular page (no date branch).

## Adding a new client

The pattern is small:

1. Take a dependency on `outl-core`, `outl-md`, `outl-actions`.
2. Open a `JsonlStorage` rooted at `<workspace>/ops/`, or bring your own `Storage` impl.
3. Open a `Workspace` with that storage; hold one `HlcGenerator` per device.
4. Call into `outl-actions` for every user-visible mutation.
5. Call `outl_actions::apply_journal_md` (or the per-page equivalent when we add it) if you want the `.md` projection on disk.

What you write in your client crate: command surface (Tauri, keyboard, HTTP, …), UI state, navigation, animations.
Nothing else.
