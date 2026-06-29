# CLAUDE.md — outl-plugins

The plugin system shared by **every** client (TUI, desktop, mobile, CLI).
Read this before editing the crate.

## What this crate is

A plugin is bundled JavaScript described by `plugin.json`.
It declares the **capabilities** it registers and the **permissions** it needs.
The user approves permissions on install; the loader intersects capabilities with what the current client implements.
A plugin written once runs on every client because it talks to *this* crate, never to anything client-specific.

The runtime engine is **Boa** (already embedded in `outl-exec`, runs on iOS — pure-Rust, no JIT), behind the `PluginEngine` trait so it can move to QuickJS later **only if** gas/perf/async becomes a *measured* blocker.

## Non-negotiables (inherit the root invariants)

1. **Plugins never touch `outl-core` directly.**
   Every mutation goes through the host API → `outl-actions` → `Workspace::apply` → op log.
   No shortcut, no editing `.md`, no bypassing the CRDT.

2. **Every plugin op is stamped `actor = "plugin:<id>@<device>"`.**
   The op log is the audit trail — keep it that way.

3. **`storage:local` does not converge in d0.**
   It is per-device local KV.
   If a plugin needs cross-device state, model it as an `Op` (root invariant 7), never as a shared file.

4. **Permission is checked on every host call.**
   Deny by default.
   A capability the client lacks loads *partially* with a warning — never a silent crash, never a panic.

5. **The bundle hash is revalidated on every load.**
   A hash mismatch blocks the load.
   Plugins do not change silently.

## Layout

```
src/
├── lib.rs         # crate doc, re-exports, HOST_API_VERSION
├── error.rs       # PluginError
├── manifest.rs    # plugin.json parse + validation
├── capability.rs  # Capability enum + intersect()
├── permission.rs  # Permission enum + network domain rules + PermissionSet gating
├── lockfile.rs    # installed.json (InstalledPlugins / InstalledEntry) + bundle_hash
├── model.rs       # JS↔host data: ReadModel, BlockView, HostIntent, MoveTarget, TurnOutput
├── runtime.rs     # PluginEngine trait (engine seam: load / run_command / dispatch_op)
├── engine.rs      # BoaEngine (feature `js`): native bridge + JS prelude (describe→apply)
├── host.rs        # PluginHost: load, commands(), run_command(), sync_hooks() + intent apply
├── loader.rs      # disk loader + install_from_dir (.outl/plugins/<id>/ + installed.json + _dev)
└── registry.rs    # registry index (fetch/search) + marketplace API (feature `registry`)
```

## Marketplace API (one owner, both GUI clients wrap)

`registry.rs` owns the shared marketplace surface so the desktop and mobile Tauri layers stay thin shims (the bug this prevents: the same `registry ∩ lockfile` mapping written once per client and drifting).
All four take a `&Path` storage root; the desktop resolves its `Arc<Mutex<Option<PathBuf>>>` to a `&Path` at the call site, mobile passes its owned `PathBuf` directly.

- `MarketplaceItem` — a registry entry + local `installed`/`enabled` state (the serialized row both clients render).
- `marketplace_list(&Path)` — fetch the index, cross-reference the lockfile.
- `marketplace_install(&Path, &ActorId, id)` — download + install an official plugin, returns its name.
- `set_enabled(&Path, id, enabled)` — flip the lockfile flag (no network).
- Uninstall reuses the existing `loader::uninstall`.

## Execution model: describe → apply

The JS engine never holds `&mut Workspace`.
Each turn the host hands the engine a read-only `ReadModel` (blocks + pages) and the plugin config.
The plugin **reads** from it and **emits** `HostIntent`s into a buffer.
The host then drains the buffer and applies each intent through `outl-actions` (permission-gated).
Plugin handlers live in JS-land (`globalThis.__OUTL`), so no `JsFunction` is ever stored in Rust.

**Anti-loop:** `PluginHost` tracks `last_seen` log length.
Ops a plugin produces advance `last_seen` too, so they never re-trigger `onOp` — no plugin→op→plugin cycle.
`sync_hooks` is the single post-mutation entry point a client calls after any action.

`PluginHost` is **not `Send`** (Boa `Context` is single-threaded).
Fine for TUI/CLI; GUI clients run it on a dedicated plugin thread.

## Status

- **Done:** manifest, capability, permission, lockfile, model, `BoaEngine`,
  `PluginHost` (commands + run_command + onOp via sync_hooks), disk loader + install,
  registry index, SDK + example plugin — all with tests, including an end-to-end run
  of the **real shipped bundle** (`real_example_bundle_archives_done_blocks`).
- **Wiring in progress:** CLI `outl plugin`, TUI slash/hooks.
- **Remaining:** desktop/mobile wiring (needs a dedicated plugin thread for `!Send`),
  `.outlpkg` packaging, `github:` install source, `network`/`storage` host calls.

Full plan: the approved design doc the user signed off on (issue #25).

## Security tests are mandatory (root Rust rule)

Auth/ACL code needs deny ≥ allow.
Already covered: `network:*` rejected, leading-wildcard domain matching (apex and suffix-collision denied), permission growth via `covers()`, bundle-hash mismatch, keybinding-to-unknown-command.
Add to this set when you add a host call — happy path is **not** coverage.

## Reuse-first

The JS engine setup (Boa context + console shim) already exists in `outl-exec`.
When `BoaEngine` lands, extract the shared bits — do not write a second copy.
Host-API methods wrap `outl-actions` functions; never re-implement block/page ops here.
