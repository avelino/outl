# Clients and shared logic

outl has multiple clients today (TUI, mobile) and more coming
(Tauri desktop, plugins). They all sit on top of the same workspace
and the same op log. To keep them honest, we route every workspace
operation through one shared crate: **`outl-actions`**.

## The stack

```text
┌──────────────────────────────────────────────────────────────┐
│ Clients                                                       │
│   outl-cli  outl-tui  outl-mobile  …future desktop / plugins │
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
| Code-block execution                 | `outl-exec`                     |
| TUI: keymaps, modes, overlays, in-flight AST manipulation | `outl-tui`         |
| Mobile: iCloud storage, Tauri commands, Solid frontend | `outl-mobile`         |
| CLI subcommands                      | `outl-cli`                      |

## When to put logic in `outl-actions`

Yes if any of these are true:

- Two or more clients (today or in the next quarter) need the same op.
- The function takes only `Workspace + HlcGenerator` and returns
  `Result<_, ActionError>`.
- It produces ops by way of `Workspace::apply` — no direct storage
  writes, no filesystem touches outside `journal::write_md_atomic`.

No if:

- It manipulates client UI state (selection, modes, toasts, focus,
  keymaps).
- It manipulates an in-flight `Vec<OutlineNode>` that hasn't been
  parsed back into a workspace yet (those helpers live in
  `outl-md::outline_ops`, re-exported through a one-liner shim at
  `outl-tui/src/outline_ops.rs` because the mobile client needs them
  too — they're workspace-free pure AST manipulation, so they sit in
  `outl-md` rather than `outl-actions`).
- It's storage-backend-specific (iCloud, sqlite, ChronDB) — those
  implement `outl_core::Storage` in the binary that needs them.

## TODO/DONE convention

A block's TODO state is **a prefix on its text**, not a property:

```
"foo"             plain block
"TODO foo"        open task
"DONE foo"        completed task
```

This is the wire format the TUI already uses and what `.md` files
contain when synced to other tools. `outl-actions::cycle_todo` walks
`None → TODO → DONE → None`. UI surfaces parse the prefix out via
`split_todo` so they can render a checkbox.

## iCloud sync (mobile + TUI, today)

The mobile client persists the op log to the iCloud Ubiquity
Container. The TUI reaches the same workspace by pointing
`--path` at the container's `Documents/` directory:

```
<container>/Documents/                ← TUI: outl --path "<container>/Documents"
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

> The folder is **`ops/`**, not `.ops/`. iCloud Documents / Ubiquity
> Containers do not sync paths starting with `.` across devices, so a
> dotted name silently breaks multi-device sync. The same rule is why
> the sidecar moved from `.foo.outl` to `foo.outl` in v0.

Each device only writes to its own `ops-<actor>.jsonl`, so iCloud
never has to merge file contents — the CRDT does that work after
reading every actor's ops. The `.md` projection is rewritten after
every mutation; do **not** parse it back to reconstruct workspace
state, the op log is authoritative.

### Shared sync engine

Both clients use `outl_actions::SyncEngine` for the reload-workspace +
reproject-page flow. Detection is client-specific (TUI runs a worker
thread polling `snapshot_peers()` every ~2s; mobile registers
`NSMetadataQuery` on the ubiquity container). Once detection fires,
the call site is identical:

```rust
let engine = SyncEngine::new(workspace_root, actor);
let fresh = engine.reload_workspace()?;
engine.reproject_page(&fresh, focused_page_id)?;
```

The TUI defers the reload while the user is in Insert mode (the
in-flight `ParsedPage` would be clobbered) via a `pending_reload`
flag drained on commit. Mobile applies immediately because every
mutation is one atomic Tauri command. The policy diverges; the
engine does not.

`engine.scan_for_orphans()` is the other shared piece: it walks
`journals/` and `pages/` for `.md` files whose sidecar is missing or
stale (fresh import from Roam/Logseq, peer-shipped projection
without sidecar, external vim edit). The TUI runs the scan every 10s
on a worker thread; mobile runs it once at boot. Both feed the same
`outl_md::reconcile::reconcile_md`.

See `crates/outl-mobile/CLAUDE.md` for the full bundle ID, signing
team, container ID set required to build it, and the
`NSFileCoordinator`-based peer-file materialisation step that has to
run before any read of a peer `ops-*.jsonl`.

## Adding a new client

The pattern is small:

1. Take a dependency on `outl-core`, `outl-md`, `outl-actions`.
2. Bring your own `Storage` impl, or reuse `SqliteStorage`.
3. Open a `Workspace` with that storage; hold one `HlcGenerator` per
   device.
4. Call into `outl-actions` for every user-visible mutation.
5. Call `outl_actions::apply_journal_md` (or the per-page equivalent
   when we add it) if you want the `.md` projection on disk.

What you write in your client crate: command surface (Tauri,
keyboard, HTTP, …), UI state, navigation, animations. Nothing else.
