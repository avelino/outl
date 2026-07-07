# CLAUDE.md — outl-mobile

Tauri 2 mobile client (iOS first, Android later).
Solid.js + Tailwind frontend, Rust backend that **must stay thin** — every workspace operation is delegated to `outl-actions`.

## Layering

```text
outl-core                    (CRDT, op log, storage trait)
outl-md                      (.md parse/render, sidecar)
outl-actions                 (workspace operations + SyncEngine, shared with TUI)
   ↑
outl-mobile (this crate)
   ├── src-tauri/src/
   │   ├── lib.rs                  (mod decls + run() + invoke_handler)
   │   ├── state.rs                (AppState + AppHost impl; wire DTOs re-exported from outl-tauri-shared)
   │   ├── workspace_open.rs       (boot orchestration over outl_tauri_shared::workspace_open primitives)
   │   ├── workspace_picker.rs     (set_workspace — folder choice + persistence; native picker deferred)
   │   ├── iroh_sync.rs            (wire_iroh_transport — boot the P2P transport, register the bg-sync handle)
   │   ├── bg_sync.rs              (outl_ios_background_sync FFI — drives a forced sync from the iOS BGProcessingTask)
   │   ├── plugin_service.rs       (mobile shim: CLIENT id + capability set over outl_tauri_shared::PluginService)
   │   └── commands/               (thin #[tauri::command] wrappers over outl_tauri_shared::commands)
   ├── gen/apple/.../main.mm       (NSMetadataQuery + NSFileCoordinator iCloud watcher)
   └── (frontend in ../src)        (Solid components, Tailwind, Tauri bridge)
```

The command bodies, wire DTOs, helpers, and the plugin thread live in **`crates/outl-tauri-shared/`** (see [its CLAUDE.md](../outl-tauri-shared/CLAUDE.md)).
This crate keeps only thin `#[tauri::command]` wrappers plus what is genuinely mobile-specific (`bg_sync.rs`, `workspace_picker.rs`, the iOS glue).
The one structural divergence — mobile's `storage_root: PathBuf` instead of desktop's `Arc<Mutex<Option<PathBuf>>>`, because a folder swap is a relaunch here — is absorbed by the shared `AppHost` / `StorageRootProvider` traits.
The wrapper files therefore read identically to desktop's.
A command both clients need gets its body in `outl-tauri-shared` and a wrapper + `invoke_handler!` entry in **both** clients — never in just one.

The op log backend is the shared `outl_core::storage::JsonlStorage`;
there is no `icloud_storage.rs` because the only iCloud-specific work is resolving the ubiquity container path (via `icloud_path.rs`) and forcing peer-file materialisation before reads (via `OutlOpsWatcher.swift`).
The storage trait stays generic; the transport gets handled outside it.

## Storage is a chosen folder, not forced iCloud (Fase 2)

**The workspace root is a folder the user picks.**
It may live anywhere — the app's local data dir (the default), the Files app, or inside an iCloud container — and **iroh P2P is the primary sync**.
iCloud is now just *a place the folder might be*, never a hard dependency.
A fresh install works with **zero iCloud**.

Boot resolution (`workspace_open::resolve_storage_root`):

1. The persisted `WorkspaceCfg.last` path (from `outl-config`), when present and usable — survives restarts.
2. Else the app-local default `<app-data-dir>/outl/` — synced by iroh, no iCloud.

The old behaviour (force `<ubiquity-container>/Documents/`, fall back to local only if iCloud was unavailable) is gone.
iCloud is reachable on demand via `workspace_open::icloud_workspace_root()` (used by `workspace_picker::pick_in_icloud`).

**Folder selection.**
`workspace_picker.rs` owns the choice.

- `set_workspace(path)` validates, creates the dir, persists `WorkspaceCfg.last`, and emits `workspace-reopen-required`.
  The reopen is **boot-read** today (next launch picks up `last`); a runtime swap would need `AppState.storage_root` to become an `Arc<Mutex<Option<PathBuf>>>` plus an iroh rebind, deliberately deferred.
  Frontend wrapper: `setWorkspace(path) → Promise<void>` in `src/lib/api.ts`.
  No caller wires it yet — the arbitrary-folder native picker (`UIDocumentPickerViewController` + security-scoped bookmark) is deferred, so the local default is the only root a fresh install opens.

> **Registration note:** both commands are registered in `lib.rs`'s `invoke_handler!` list (`workspace_picker::set_workspace`, `workspace_picker::pick_in_icloud`).

## First-run onboarding

`components/Onboarding.tsx` is the first-run flow.
`App.tsx` gates it on a **per-install `localStorage` flag** (`outl.onboarded`) — pure UI state, **never** an Op (it must not converge across devices; each device onboards once).
Mobile has no "is a workspace chosen?" backend gate (a fresh install always resolves *a* root — the local default), so this flag is the only signal that distinguishes a brand-new install from a returning one.

Two honest steps, no filler:

1. **Storage** — "Keep on this device" (the local default, recommended; just advances, no `setWorkspace` call) vs "Store in iCloud" (`setWorkspace(pickInICloud())`).
   The iCloud option is **hidden** when `pickInICloud()` returns `null` (device not signed in) — never a dead button.
   Because `set_workspace` is boot-read, choosing iCloud shows a one-line "active after you restart" note instead of pretending the swap is instant.
   Arbitrary-folder picking stays deferred (native picker), so these two are the only choices today.
2. **Sync (optional)** — the shared `SYNC_STEP` copy (`@outl/shared/onboarding`) + a button that opens the existing `<DevicesSheet />` (set its `open` prop — internals untouched).
   Fully skippable.

The onboarding **copy** lives in `@outl/shared/onboarding` (identical to desktop); the bottom-sheet chrome + haptics stay here.
Pairing is **not** reimplemented — `Onboarding` opens the real `<DevicesSheet />`.

**DEFERRED — native folder picker + security-scoped bookmark.**
The native `UIDocumentPickerViewController` (folder mode) bridge is **not** implemented (and is not faked).
Two real blockers: Tauri 2's iOS folder picker is incomplete (tauri-apps/plugins-workspace#3030), and a folder *outside* the app sandbox needs an `NSURL` security-scoped **bookmark** to be reopenable across launches.
Storing just the string path in `WorkspaceCfg.last` only works for the sandbox and the local default.
The follow-up adds an `objc2` bridge that presents the picker, serialises a bookmark, persists it next to `actor`, and resolves it on boot before `resolve_storage_root`.
Until then, `set_workspace` works for any path the frontend can already reach without a scoped bookmark, and the local default is the only root a fresh install opens.

## Change detection: the iroh signal

Storage is a **local folder synced by iroh** (no iCloud — the Rust side was ripped out: `icloud_path.rs` deleted, `storage_is_icloud`/`pick_in_icloud`/`icloud_workspace_root` removed).
The transport fires a reload signal whenever it writes peer ops; `iroh_sync.rs` bridges it to the `workspace-ready` Tauri event.
There is no filesystem watcher in the Rust path.

**DEFERRED — native iCloud cleanup.**
The iOS-native `OutlOpsWatcher.swift` (`NSMetadataQuery` + `NSFileCoordinator`), the iCloud container entitlements, and the `Info.plist`/`pbxproj` references are still present from before the Rust teardown.
Because the chosen folder is now always local, the watcher's `NSMetadataQueryUbiquitousDocumentsScope` query matches nothing and stays **dormant** — it does nothing and breaks nothing.
Removing it (watcher → no-op, strip the entitlements + plist keys) is a follow-up that touches code-signing, so it must be validated with a device build, not done blind.

## Background sync (iOS)

iOS suspends the app's sockets the moment it backgrounds, so there is **no continuous background P2P**.
The sanctioned paths are the two opportunistic `BGTaskScheduler` windows — **both** sync, wired across three pieces:

1. **Info.plist** declares `UIBackgroundModes` (`fetch` + `processing`) and `BGTaskSchedulerPermittedIdentifiers` (`app.outl.mobile-app.refresh`, `app.outl.mobile-app.sync`).
   Without these the toggle never shows in Settings and `BGTaskScheduler.register`/`submit` fail silently.
2. **`OutlBackgroundRefresh.swift`** registers both tasks (`+load` → `install`) through one shared `handleTask` helper (reschedule first, FFI on a background queue, complete exactly once — the work and the OS expiration handler race).
   The `refresh` (`BGAppRefreshTask`, ~30s windows) drives the short FFI; the `sync` (`BGProcessingTask`, `requiresNetworkConnectivity = true`) drives the long one.
   **Scheduling is gated on having paired peers** (`outl_ios_peer_count() > 0`) so an unpaired device never boots the stack for nothing.
   A `didEnterBackgroundNotification` observer re-submits on every backgrounding, which also arms the gate right after the first pairing.
3. **`bg_sync.rs`** owns the three FFIs (C ABI, `@_silgen_name` on the Swift side).
   They are `outl_ios_background_sync()` (cap 20s), `outl_ios_background_sync_short()` (cap 12s, refresh-window budget), and `outl_ios_peer_count()` (reads `<root>/.outl/peers.json` fresh from disk, so post-boot pairings count).
   `wire_iroh_transport` registers a `Clone` of the live `IrohSyncTransport` **plus the workspace root** into a re-settable global.
   The sync FFIs fire `sync_now()` (a forced delta-sync against every peer, mobile side initiating, which is NAT-friendly).
   They then poll `completed_sync_passes()` every 250ms, returning as soon as the pass lands — the cap is a fallback, not a fixed sleep.

The FFI + Swift handler can only be validated with a **device build**.
The simulator has no `BGTaskScheduler` daemon, so `submit` always fails there and is swallowed; the Rust side is `cargo check`-clean on its own.

## Hard rule

**This crate adds no business logic.**
If a Tauri command does something that involves the workspace shape (edit, move, todo, journal render), it delegates to `outl-actions`.
If you find yourself writing a tree walk or an op-generating helper inside `lib.rs`, stop — move it to `outl-actions` instead.
The TUI will need it too.

The same rule extends to the **Solid frontend** (`src/`).
Before adding a helper that walks blocks, normalises text, or maps a cursor across `\n`, check:

1. **`@outl/shared`** (`crates/outl-frontend-shared/`) — the cross-client TS lib already owns `<MarkdownInline />`,
   `looksLikeOutline`,
   `utf16OffsetToCharOffset`,
   the autocomplete helpers (`detectRefContext`, `autoClose/DeletePair`, `insertPair/Text`, `applySuggestion`),
   every shared DTO (`@outl/shared/api/types`),
   and the `invoke()` wrappers for the Tauri commands every client calls (`@outl/shared/api/commands`).
   If you find yourself reimplementing one of these in `src/lib/`, stop — the desktop client will need the identical behaviour, and a parallel TS copy is exactly the drift we paid to delete.
2. **`outl-md` / `outl-actions` / `outl-core`** — the Rust side likely already exposes the data through a Tauri command (or could with a tiny addition).

Only write a helper directly under `outl-mobile/src/lib/` when it's genuinely mobile-specific (touch gestures, iOS UIKit bridges, haptics, viewport math).

Workspace-level policy: [`CLAUDE.md`](../../CLAUDE.md#reuse-first-no-parallel-implementations).
Frontend-specific policy: [`outl-frontend-shared/CLAUDE.md`](../outl-frontend-shared/CLAUDE.md).

What this crate **does** own:

- iCloud Ubiquity Container resolution and the `Storage` impl on top.
- Per-device actor id persistence (`<sandbox>/actor`).
- Tauri command surface (argument parsing, error mapping).
- Solid frontend that consumes the commands.

## Opening a ref that may not exist yet

`[[avelino/outl]]`, `[[2026-06-04]]`, `#code-review`, picker entries — every "tap a ref → see a page" path on the frontend goes through **one** Tauri command, `open_ref(target)`, which wraps `outl_actions::page::open_or_create_by_ref`.
The single decision tree (date → journal, else literal/slugified/title match → existing page, else create as page) lives in the shared crate so a frontend regex cannot drift from a backend parser the way it did before `open_ref` existed.

What used to be wrong: the frontend split the journal-vs-page
decision with `/^\d{4}-\d{2}-\d{2}$/` and routed to one of two
strict-validating commands (`open_journal_for` / `open_page_by_slug`).
`[[2026-13-01]]` matched the regex, hit `open_journal_for`, and
surfaced an `invalid date slug` toast — even though falling through
to "create a regular page" was clearly the right behaviour.

`open_page_by_slug` is kept for the picker (the picker already hands the command a clean slug from a known page).
`open_journal_for` stays for date-navigation commands (`previousDay` / `nextDay`) whose input is derived from controlled state, not from a user tap.
Every **ref-click** code path on the frontend (`handleRefClick`, `handleTagClick`) must call `openRef` so the decision tree is single-sourced.

`resolve_ref` survives for autocomplete previews ("this ref will
land on `<page>`") but is **not** the navigation entry point — for
that, always call `openRef`.

## Page switcher — long-press to delete

`PageSwitcher.tsx` renders each page as a row button; spreading `longPressHandlers(p)` on the row arms a 500 ms sustained-touch detector (canceled if the finger moves more than 10 px, so a scroll never false-fires).
On fire, `handleDelete(p)` runs `window.confirm(...)` → `deletePage(slug)` (from `@outl/shared/api/commands`) → navigates to the returned today's journal → refetches the list.
Journals are excluded (`p.kind === "journal"` skips the handlers) — only regular pages can be deleted from the switcher.
The backend `delete_page` Tauri command is the shared `outl_tauri_shared::commands::page::delete_page` body — no mobile-specific logic.
`Action::DeletePage` carries a `g d` chord in the shared catalog (Normal mode), but mobile has no keyboard surface — long-press in the page switcher remains the only trigger on touch devices.

## Opening an external `[label](url)` link

Tapping an external markdown link opens it in the system browser via **`tauri-plugin-opener`** (registered in `src-tauri/src/lib.rs`; the capability grants a scoped `opener:allow-open-url` for `http`/`https`/`mailto` in `capabilities/default.json`).
`Journal.tsx`'s `handleLinkClick` calls the shared `openExternalUrl` wrapper (`@outl/shared/api/commands`) — the same one desktop uses, so the scheme allow-list (`http(s)`/`mailto` only; `file:`/`javascript:` rejected) lives in one place.
`<MarkdownInline />` gets `onLinkClick` threaded from `Journal.tsx` → `BlockRow` (recursively) → the renderer.
`[[ref]]`/`#tag` taps are unchanged — they still route through `openRef` (see above).
Backlink rows stay inert: the whole row is already a tap-to-source button.

## Blockquote chrome

A block whose `text` starts with the CommonMark `"> "` marker renders with a left border (`border-l-2 border-(--color-ios-text-secondary)/40`),
a very faint tint (`bg-(--color-ios-text-secondary)/[0.05]` light / `[0.07]` dark),
a right-rounded corner (`rounded-r-md`),
and **full body colour** — refs / bold / tags / code keep their normal palette so the styled-token affordance isn't lost.
The tint is intentionally ~5% alpha: enough to read as a soft box at a glance, low enough to not fight with surrounding outline rows.

The chrome wrapper sits one level above the bullet — it envelops **both `<BulletOrCheckbox />` *and* the body** in a single flex container.
That order (`│ ☐ body`) matches the TUI exactly and reads as "this is a quoted task" instead of "a task whose body happens to be a quote".
The `<CollapseTriangle />` stays outside the chrome so the gutter isn't double-boxed.
When the block isn't quoted, the wrapper degrades to a plain `flex min-w-0 flex-1 items-start gap-2.5` container, so non-quoted rows render byte-identical.

The earlier `italic text-(--color-ios-text-secondary)` body styling (suggested by issue #64's mock) was dropped after testing:
the muted body erased the cyan of `[[ref]]` underlines and the bold weight of `**abc**` against the background,
which hurts more than it helps when the user is scanning a quoted excerpt.

The check uses **`splitQuote`** from `@outl/shared/markdown` (the TS mirror of `outl_actions::quote::split_quote`) and **`stripQuoteFromTokens`** to remove the `> ` from the first `Plain` token before handing the list to `<MarkdownInline />`
— otherwise the marker would render once in the body and once in the chrome.
The two pieces compose with the existing TODO/DONE checkbox: a `> TODO foo` row paints the checkbox **and** the left border.

Toggling the marker goes through the same Tauri pipeline as TODO/DONE: a `toggleQuote(id)` wrapper in `@outl/shared/api/commands` calls the `toggle_quote` command on each client's `src-tauri`, which delegates to `outl_actions::block::toggle_quote`.
There is **no string surgery on the TS side** — the prefix arithmetic owns the rule and stays in one place.

The TUI applies the same "chrome only, body full colour" policy via `│ ` in `view::inline::render_pretty_block_text_impl` and `view/outline.rs::BlockRowKind::Bullet`.
Three surfaces, one policy.

## Paste from external apps

The textarea in `BlockRow.tsx` intercepts paste events (with formatting only — mobile has no `Cmd+Shift+V`).
A rich clipboard (`text/html`: Slack, Docs, Notion) converts to outl markdown via `htmlToOutlMarkdown` (`@outl/shared/paste`, Turndown), so pasted **bold** / links / lists survive.
Plain text routes to `outl_actions::paste_markdown` (`paste_markdown_at`) when `looksLikeOutline` (bullets) **or** `hasMultipleParagraphs` (blank-line paragraphs) is true.
Multi-paragraph plain text splits into one block per paragraph; single-paragraph falls through to the browser's default splice.

`create_block` has a **stale-anchor fallback**: if `after_id` is not in the tree (`NotInTree`), the block is appended at the end of the page instead of returning an error (mirrors the desktop fix).

## Code execution (`run_code_block`)

Long-press a `` ```lang …``` `` block → the contextual menu shows a `Run <lang>` action that fires `runCodeBlock` (`@outl/shared/api/commands`).
The backend command is in `src-tauri/src/exec.rs` and is a **thin adapter**
— the orchestration (flat-DFS walk, `.md` path resolution, `outl-exec` invocation, DTO build) lives in `outl_actions::exec::run_code_block` so the desktop client shares the exact same flow.
The mobile adapter only parses NodeIds, locks the workspace, calls the action, and wraps the outcome with a refreshed `PageView`.
Adding behaviour to `src-tauri/src/exec.rs` is almost always a smell — promote it to `outl-actions` instead.

Runtimes shipped on iOS: **Lisp, JS, Python, Lua**.
`lang-rust` is deliberately disabled in `outl-mobile/src-tauri/Cargo.toml` — the wasmtime + Cranelift stack adds tens of MB to the IPA and runs into iOS code-signing restrictions on dynamic code generation.
The dependency is declared with `path = "../../outl-exec"` (not `workspace = true`) so we can opt out of the workspace dep's default features without changing them for every other consumer (CLI / TUI / desktop, which all keep Rust).

The "Run code" action only shows up when `@outl/shared/highlight::detectFence` matches the block's raw text
— same detector the read-mode renderer uses,
and the backend re-validates inside `run_block_at_index`,
so a false-positive surfaces as a runtime toast instead of doing damage.

## Plugins

JS plugins (`outl_plugins::PluginHost`) run on mobile; the design is the desktop's.
Read [`outl-desktop/CLAUDE.md` → Plugins](../outl-desktop/CLAUDE.md#plugins) for the shared rationale (`!Send` Boa host on a dedicated thread, `PluginService` in `AppState`, re-projection via `apply_all_pages_md`).
Boa is pure-Rust (no JIT), so it ships under iOS's dynamic-code ban (same as `lang-js`).

**The one divergence from desktop:** mobile's `storage_root` is an owned `PathBuf` (folder swap is a relaunch), absorbed by the shared `StorageRootProvider` trait — a fixed root never triggers the "re-load on root swap" branch.
The host loads plugins once, lazily, from `<root>/.outl/plugins/` on the first request after the workspace opens (`ensure_loaded` + `mark_synced`).

Capabilities honored: `slash-command` + `op-hook` + `ui-render` + `toolbar-button` + `content-transformer:text` + `content-transformer:rich` (no `keybinding` — no chord surface on mobile).
Each must be declared in `client_capabilities()` (`plugin_service.rs`); the host gates contributions on the client∩plugin intersection.
Dropping `ToolbarButton` silently empties `toolbar_buttons("mobile")`; dropping either transformer cap silently filters `transformers()` (a custom-language fence then renders as plain code).
Tauri commands in `commands/plugin.rs` have the **identical shape to desktop** — see [`outl-desktop/CLAUDE.md` → Plugins](../outl-desktop/CLAUDE.md#plugins) for the full command table.

Op-hooks fire at a single post-mutation point: `Journal.tsx`'s `commitEdit` calls `pluginSyncHooks(pid)` after an edit lands.
One call dispatches every op since the last sweep, so it also catches structural ops (indent / move / delete).
The hook-driven `applyView` guards on `!editingId()` so it never resets the textarea.

Frontend: the plugin DTOs + wrappers (`pluginList` / `pluginToolbar` / `pluginRun` / `pluginSyncHooks`, …) live in `@outl/shared/api`.
The stacked-squares header glyph opens `components/PluginSheet.tsx` — a bottom sheet that lists + runs commands and pipes `notify` / errors to the toast.
Toolbar buttons are inline header glyphs.
`Journal.tsx` loads `pluginToolbar()` in `onMount` into a `toolbarButtons()` signal and renders one `<button>` per entry next to the sheet glyph.
`runToolbarButton` → `pluginRun(...)` reuses the sheet's toast / `showPluginViews` / `applyView` path.

### `ui-render` views (the confetti path)

A `ui-render` plugin emits HTML/JS via `ctx.ui.render(html)`; the core gates it onto `PluginRun::views`, propagated as `views` on both `PluginRunReply` (command path) and `PluginSyncReply` (`onOp` hook path).

`components/PluginViewOverlay.tsx` paints each in a **sandboxed, ephemeral `<iframe>`**.
The frame is `sandbox="allow-scripts"` **WITHOUT `allow-same-origin`** — load-bearing.
Plugin JS is untrusted; the missing flag forces an opaque origin so the frame can't reach the app DOM / Tauri bridge — the two flags together defeat the sandbox, so **never** add it.
Content is `srcdoc={html}` (no network), fullscreen, `pointer-events: none`, auto-removed after ~6s.

The overlay exposes an imperative `push(html)` via its `bind` prop.
`Journal.tsx` holds it as `pushPluginView` and feeds it from `showPluginViews(views)` at every source: `PluginSheet`'s `onViews`, `commitEdit`'s `pluginSyncHooks` reply, and `runToolbarButton`.
**End-to-end:** block → DONE → `commitEdit` → `plugin_sync_hooks` → confetti plugin's `onOp` emits HTML → `showPluginViews` → iframe overlay → confetti.

The sandbox attrs + auto-removal are pinned by `PluginViewOverlay.test.ts` (Vitest + happy-dom).
`plugin_service.rs` unit tests cover list / toolbar / transformer / run / unknown-command / empty-host; a real plugin load + the sheet UI + iframe overlay only exercise under `cargo tauri ios dev`.

### Content transformers (custom-language fences)

A plugin transformer claims a code-fence language and turns the body into a render descriptor (`{kind, content}`).
`Journal.tsx`'s `onMount` calls `loadTransformers()` (`@outl/shared/plugins/transformer-registry`) once when the workspace opens, filling a module signal keyed by `lang` (best-effort — failure leaves fences as plain code).
`BlockRow`'s fence branch looks the language up via `transformerFor(lang)`; a match renders `<PluginFence />`, falling back to plain `<HighlightedCode />` while loading or on decline.
`<PluginFence />` runs `pluginTransform(...)` through `runTransform`, **cached by `(block id, body)`** so it runs at most once per distinct body (editing invalidates; a `null` decline is cached too).
Render by `kind`: `text` → inline preformatted text (no frontend markdown-string tokenizer — backend-only in `outl_md`).
`rich` → HTML in a persistent **inline** sandboxed `<iframe>` (`allow-scripts`, never `allow-same-origin` — same posture as `<PluginViewOverlay />`).
The registry + cache glue is shared with the desktop in `@outl/shared/plugins/transformer-registry`, alongside the DTO shapes + `invoke()` wrappers in `@outl/shared/api`.

## Peer / device management (`outl_peer_list` / `outl_peer_remove`)

`commands/peers.rs` exposes two Tauri commands that read and edit the iroh
peers file (`<workspace>/.outl/peers.json`) via `outl_sync_iroh::PeersStore`:

- `outl_peer_list() -> Vec<PeerDto>` — lists paired devices (`node_id`,
  `alias`, `added_at`).
- `outl_peer_remove(id: String) -> bool` — removes peers whose `node_id`
  starts with the given prefix; `true` if any matched.

The peer list is per-**graph**: it lives at `<workspace_root>/.outl/peers.json`
(resolved from `AppState::storage_root` via `outl_sync_iroh::workspace_peers_path`),
NOT next to the device identity.
The device `identity.key` stays per-**install** in the Tauri app local data dir
([`iroh_sync::iroh_dir`]) — one node id per install.
Each command runs `migrate_global_peers_if_absent` first, so a legacy global
`~/.outl/peers.json` is copied into the workspace once on first open.
These are the **only** commands that touch `peers.json` directly instead of
the workspace lock — peer pairing is sync-transport state, but the list is
graph-scoped, so they read `storage_root` without going through
`outl-actions`.

`commands/peers.rs` also exposes `outl_sync_now()` (reads `state.iroh`, calls the transport's `sync_now()`) — the force-sync trigger behind the refresh button.

## Sync dot + refresh (iroh-driven)

The header `<SyncDot>` and the refresh button / `PullToRefresh` reflect and drive the **iroh P2P transport** (outl's default sync), not the iCloud-era `navigator.onLine` signal they started on.

- **Dot state.**
  The PRIMARY input is iroh peer health, polled via `peerStatus()` → the shared `peersOnline()` helper (`@outl/shared/peers`) into a `peersUp` signal.
  The poll runs on mount, every 5s, and after each `peer-ops-changed` (the native ops bridge) plus after a force-sync.
  Derivation: a force-sync in flight → **syncing** (spinner); else `online() && peersUp()` → **synced** (green); else **offline** (orange).
  `navigator.onLine` stays only as a secondary floor (truly no radio → orange regardless).
  Zero paired peers reads as offline — there's nothing to sync with.
- **Refresh.**
  `handleRefresh` (the button **and** `PullToRefresh`) calls `syncNow()` (force a P2P pull — dial every peer now instead of waiting for the 8s catch-up tick) THEN `reloadWorkspace()` (re-render with whatever landed).
  Both calls are wrapped in `withError` (toast on failure, never wedge the local reload), and the `syncing` spinner brackets the whole pass.
- **Auto-sync (no button).**
  `Journal.tsx` shares the refresh core as `pullAndReload()` and fires it automatically: on `onMount` (opening the app), on `visibilitychange` → visible (iOS froze JS in the background), and on the 5s poll tick (alongside the peer-status probe).
  The mobile side initiating the dial is NAT-friendly — waiting for the desktop to reach an iPhone behind carrier NAT is not — so this is what makes a desktop edit show up without the user touching refresh.
  The `workspace-ready` reload skips while a block is being edited (guarded by `editingId()`) so it never resets the textarea mid-edit.

`syncNow()` and `peersOnline()` both live in `@outl/shared` so mobile and desktop derive the dot + drive the refresh identically.
See [`outl-sync-iroh/CLAUDE.md`](../outl-sync-iroh/CLAUDE.md) → "Force-sync trigger (`sync_now`)".

## Cross-runtime contracts (now in `@outl/shared`)

The four TS pieces that mirror Rust canonical sources used to live as copies under `lib/`.
They were extracted to **`crates/outl-frontend-shared/`** so mobile and desktop import the same file — drift between two TS implementations is geometrically impossible.

| Contract | Path | Mirrors (Rust) |
|---|---|---|
| `looksLikeOutline` | `@outl/shared/paste` | `outl_actions::paste::looks_like_outline` |
| `<MarkdownInline />` (renderer of `InlineToken[]`) | `@outl/shared/markdown` | `outl_md::tokenize_owned` (backend produces the tokens; the renderer is a discriminant-to-JSX switch) |
| `detectRefContext` (+ `autoClose/DeletePair`, `insertPair/Text`, `applySuggestion`) | `@outl/shared/autocomplete` | `outl_tui::actions::overlay::detect_trigger` (the `[[` and `((` triggers; TUI also covers `#` and `/`) |
| `autoPairBracket` (auto-pair `(`/`[`/`{` + step over auto-inserted closers; wired through `BlockRow`'s `onBeforeInput` because iOS soft keyboards don't emit reliable per-char `keydown`) | `@outl/shared/autocomplete` | `outl_tui::input::insert` (`insert_pair`) + `EditBuffer::delete_pair_back` |
| `utf16OffsetToCharOffset` | `@outl/shared/paste` | runtime gap, no Rust mirror — `textarea.selectionStart` is UTF-16; the backend expects codepoints. Skipping this conversion shifts the splice by one per supplementary-plane character |

**Adding a new cross-runtime contract = add it in `@outl/shared` from day one.**
Never add it under `outl-mobile/src/lib/` first — the next time desktop catches up to the feature, it has to consume from the same file.

## Logging (device console)

`run()` in `src-tauri/src/lib.rs` installs a `tracing_subscriber` fmt subscriber writing to **stderr** as its very first step (before rustls / Tauri setup).
The `EnvFilter` defaults to `info,outl_sync_iroh=debug,iroh=info` and honors `RUST_LOG`.
On iOS, stderr surfaces in `idevicesyslog` / Xcode.
So the iroh P2P transport's `info!`/`warn!`/`debug!` lines (endpoint bound + node id, each connect attempt's target + outcome, "delta sync received N ops") are visible while debugging device↔device sync.
Init uses `.try_init()` so a double-init can't panic.
See [`outl-sync-iroh/CLAUDE.md`](../outl-sync-iroh/CLAUDE.md) for what the transport logs.

## iCloud layout (opt-in destination)

When the user opts into iCloud, the root is `<ubiquity-container>/Documents/` (`workspace_open::icloud_workspace_root()`) — **one option**, not the default.
The container is already the `outl` namespace, so no extra `outl/` nesting; the TUI uses `--path "<container>/Documents"`.
Layout is the standard `journals/` + `pages/` (`.md` + `.outl` sidecar) + `ops/` (one `ops-<actor>.jsonl` per device).
**iCloud trap:** every path must be undotted — iCloud Documents skips `.`-prefixed paths across devices, so `ops/` (not `.ops/`) and `pages/<slug>.outl`, else the file never leaves its origin.

## Peer-file materialisation (the iCloud catch)

iCloud syncs file metadata aggressively and file content lazily.
When `NSMetadataQuery` fires on a peer's `ops-<actor>.jsonl`, the file's bytes may not be on disk yet — a `std::fs::open` returns an empty placeholder.
The Rust side sees a truncated op log; the merge is wrong; the projection writes a broken `.md` back.

`main.mm`'s `OutlOpsWatcher.onUpdate:` works around this in two steps:

```objc
[fm startDownloadingUbiquitousItemAtURL:url error:&startErr];
NSFileCoordinator *coord = [[NSFileCoordinator alloc] initWithFilePresenter:nil];
[coord coordinateReadingItemAtURL:url
                          options:NSFileCoordinatorReadingForUploading
                            error:&coordErr
                       byAccessor:^(NSURL *u) { (void)u; }];
```

`startDownloadingUbiquitousItemAtURL` requests materialisation; `NSFileCoordinator` blocks until the file is fully on disk.
Only after that does the watcher fire `window.__outlOpsChanged()` so the frontend can call `reload_workspace`.
Skip either step and you race the iCloud download daemon.

## Bundle / signing

- Bundle id: `app.outl.mobile-app`
- Team: `CPEEKT3E77` (paid Apple Developer Program)
- iCloud container: `iCloud.app.outl.mobile-app`
- Display name (Files.app / iCloud Drive): `outl`
- Category: `public.app-category.productivity`
- Entitlements: `com.apple.developer.icloud-services` + `icloud-container-identifiers` + `ubiquity-container-identifiers`

Bundle ID + iCloud container are **global** in the Apple Developer ecosystem.
If you change either, also update:

1. `tauri.conf.json` → `identifier`
2. `src-tauri/src/lib.rs` → `ICLOUD_CONTAINER_ID`
3. `gen/apple/outl-mobile.xcodeproj/project.pbxproj` → `PRODUCT_BUNDLE_IDENTIFIER`
4. `gen/apple/outl-mobile_iOS/outl-mobile_iOS.entitlements`
5. `gen/apple/outl-mobile_iOS/Info.plist` → `NSUbiquitousContainers` key
6. `gen/apple/project.yml` → `bundleIdPrefix` and `PRODUCT_BUNDLE_IDENTIFIER`

## Running

```bash
cd crates/outl-mobile

# iOS simulator
cargo tauri ios dev "iPhone 17 Pro outl"

# Physical device (Mac + iPhone on the same WiFi)
cargo tauri ios dev "<device-name>" --host

# Release archive for TestFlight (local smoke test only — CI ships)
cargo tauri ios build
```

After the first run, the iCloud capability must be confirmed in Xcode (Signing & Capabilities → iCloud → Containers → `iCloud.app.outl.mobile-app`).

## Versioning + TestFlight release

**Single source of truth: `Cargo.toml` workspace `version`.**
To bump the app version, edit `[workspace.package].version` at the repo root — everywhere else inherits:

| Field | Where it lives | How it's resolved |
|-------|----------------|-------------------|
| Rust crate version | `crates/outl-mobile/src-tauri/Cargo.toml` | `version.workspace = true` |
| Tauri config version | `crates/outl-mobile/src-tauri/tauri.conf.json` | Field intentionally **omitted** in the source; CI injects it via `cargo tauri ios build --config '{"version": "<short>"}'` |
| `CFBundleShortVersionString` | iOS `Info.plist` | Tauri propagates from `--config` during `cargo tauri ios build` |
| `MARKETING_VERSION` / `CURRENT_PROJECT_VERSION` | `gen/apple/.../project.pbxproj` | Same — Tauri regenerates from the merged config every build |

**Why `--config` and not just rely on Tauri's `Cargo.toml` fallback?**
The docs say Tauri uses `Cargo.toml` when `version` is missing, but the iOS code path doesn't honor that — it falls back to `1.0.0` instead.
So CI reads the workspace version itself (`awk` against `Cargo.toml` in the `Compute build metadata` step) and passes it via `--config`.
That keeps `Cargo.toml` as the only place a human bumps, and the `Patch archive CFBundleVersion` step has a sanity check that aborts the build if the propagated short version doesn't match what was passed in.

**Never** put `"version": "x.y.z"` back in `tauri.conf.json`.
If it's present, Tauri uses the static value instead of the `--config` override, and the two drift the moment someone bumps the workspace.

### CI release flow

A push to `main` triggers in parallel:

1. **`Release`** (`release.yml`) — auto-bumps `Cargo.toml` locally to `<base>-beta.<run_number>`, cuts a `v<...>-beta.<N>` tag, builds desktop binaries, ships the Homebrew formula (never commits the bump back).
2. **`Mobile`** (`mobile.yml`) — builds the signed IPA from the *unbumped* `Cargo.toml`, uploads it as `outl-ios-release`, triggers `TestFlight`.
3. **`TestFlight`** (`testflight.yml`) — downloads the artifact, uploads to App Store Connect (`xcrun altool`).

### CFBundleVersion (build number) scheme

Apple needs `CFBundleVersion` strictly monotonic across every IPA, but the marketing version (`0.5.1`) repeats across beta builds.
Scheme: `CFBundleShortVersionString = <SHORT_VERSION>` (e.g. `0.5.1`); `CFBundleVersion = <SHORT_VERSION><BETA_PAD>` (e.g. `0.5.1027`, beta number zero-padded to 3 digits).
`BETA` comes from the latest `v<SHORT_VERSION>-beta.<N>` git tag (set by `Release`), falling back to Mobile's `github.run_number`; re-runs append `.<run_attempt>` as a 4th component to dodge Apple's duplicate guard.
The build number is patched into the `.xcarchive`'s embedded `Info.plist` after `cargo tauri ios build` but **before** `xcodebuild -exportArchive` — the only injection point that survives, since Tauri exposes a single `version` field.

### What goes wrong if you forget this

- Stale `"version"` in `tauri.conf.json` → IPA ships that static value (ignores `Cargo.toml` / `--config`) → Apple 409 duplicate.
- Dropping `--config '{"version": "..."}'` from `mobile.yml` → Tauri's iOS path falls back to `1.0.0` (not `Cargo.toml`); the `Patch archive CFBundleVersion` sanity check catches it — don't disable it.
- Patching `gen/apple/.../Info.plist` pre-build is a no-op (Tauri regenerates it). `xcrun altool --type ios` exits 0 even on 409, so `testflight.yml` greps for `ERROR:` — don't simplify that step.

## Deep links (`outl://`)

The mobile app registers the `outl://` scheme so links shared into it (or the Raycast extension on the same Mac, once Handoff is in play) open a specific page or daily note (issue #98).
The scheme contract and the shared parser live in `outl-actions` — see [`docs/clients.md` → Deep links](../../docs/clients.md#deep-links-outl) — so the mobile and desktop handlers can't drift.

Wiring:

- **Plugin.**
  `tauri-plugin-deep-link` is registered in `lib.rs`'s builder.
  No single-instance plugin — iOS is single-instance by construction, so the OS routes the URL to the running app.
- **Scheme registration is the iOS `Info.plist`, not config.**
  Tauri's `plugins.deep-link.desktop.schemes` key is desktop-only.
  For an iOS **custom scheme** the `CFBundleURLTypes` entry is added directly to `gen/apple/outl-mobile_iOS/Info.plist`, alongside the existing `UIBackgroundModes` / iCloud keys this project already hand-maintains there.
  Universal Links (`https://outl.app/…`) would instead need the `mobile` config + an Associated Domains entitlement + a hosted `apple-app-site-association`; that's a separate follow-up.
- **Warm path** (`dispatch_deep_link`, fired by `on_open_url`) mirrors the desktop: parse with `outl_actions::parse_deep_link`, emit `deep-link://navigate` (`{kind:"today"}` / `{kind:"daily",date}` / `{kind:"page",slug}`), focus the window.
  A malformed URL is logged at `warn` and ignored.
- **Cold path** (a URL that *launched* the app) buffers the parsed payload in a managed `PendingDeepLink(Mutex<Option<Value>>)` during `setup()`, because the frontend listener isn't up yet.
  The `take_pending_deep_link` command drains it once `Journal` mounts.
  Same shape as the desktop buffer; only the launch URL populates it.
- **Frontend.**
  `Journal.tsx` registers `listenForDeepLink()` in `onMount` (warm) and, right after `loadTodayWithRetry`, drains `take_pending_deep_link` (cold).
  Both call the shared `navigateDeepLink` helper, which maps onto the same `openTodayJournal` / `openJournalFor` / `openPageBySlug` commands the ref-tap path uses, then `applyView`.
  The warm listener skips while a block is being edited (`editingId()` guard) so it never resets the textarea mid-edit.
  The cold drain runs after the workspace is open, so it overrides today's journal with the launch target.

**Validation needs a device build.**
The Rust side is `cargo check`-clean, but the iOS scheme registration + the OS routing only exercise on a real device / simulator build (`cargo tauri ios dev`), the same constraint the `BGTaskScheduler` and `NSMetadataQuery` paths carry.
Don't mark the mobile half "verified" from a host `cargo check` alone.

## Testing

Two layers cover the mobile crate:

| Layer | Tool | What it covers |
|-------|------|----------------|
| Rust commands + storage | `cargo test -p outl-mobile` | `ICloudStorage`, command shims, page model glue |
| Frontend pure logic | `bun run test` (Vitest + happy-dom) | textarea/native-suggester helpers, future helpers (outline walks are tested in `@outl/shared/outline`) |

Every bug fixed in a pure helper (the tokenize duplicate, refs/tags extraction, fuzzy matching) must land with a unit test before merge so it never regresses.

Native bits (`main.mm` swizzle, BGTaskScheduler, NSMetadataQuery) are not covered by unit tests yet — they're observed via the NSLog probes shown on app boot.
If we add Swift Tests later they belong next to `main.mm` in `gen/apple/Tests/`.

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-mobile -- -D warnings`
3. `cargo test -p outl-mobile`
4. `bun run test` (Vitest, frontend)
5. Build pass: `cargo tauri ios build`
