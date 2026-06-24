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
   │   ├── lib.rs                  (mod decls + run())
   │   ├── state.rs                (AppState, PageView, WorkspaceSummary, CreateBlockReply, ERR_LOADING)
   │   ├── helpers.rs              (parse_node_id / parse_date / with_ws* / build_page_view / finish_in_page*)
   │   ├── workspace_open.rs       (resolve_storage_root, spawn_workspace_opener, reconcile_orphan_md, storage_is_icloud)
   │   ├── workspace_picker.rs     (set_workspace / pick_in_icloud — folder choice + persistence; native picker deferred)
   │   ├── icloud_path.rs          (NSFileManager bridge + is_inside_icloud — iOS-only, opt-in)
   │   └── commands/               (Tauri command surface — split mirrors outl-desktop)
   │       ├── mod.rs
   │       ├── workspace.rs        (workspace_stats, reload_workspace)
   │       ├── page.rs             (list_all_pages / search_pages / search_persons / outl_emoji_search / open_* / *_day / resolve_ref / legacy compat shims)
   │       ├── block.rs            (create_block / edit_block / toggle_todo / toggle_quote / delete_block / indent_block / outdent_block / move_block_* / set_block_collapsed / paste_markdown_at)
   │       ├── peers.rs            (outl_peer_list / outl_peer_remove — read/edit the iroh peers.json, no workspace lock)
   │       └── exec.rs             (run_code_block — thin shim over outl_actions::exec::run_code_block)
   ├── gen/apple/.../main.mm       (NSMetadataQuery + NSFileCoordinator iCloud watcher)
   └── (frontend in ../src)        (Solid components, Tailwind, Tauri bridge)
```

The split mirrors **`crates/outl-desktop/src-tauri/`** 1:1 — `commands/{workspace,page,block,exec}.rs` + `helpers.rs` + `state.rs` + `workspace_open.rs` — so a contributor who knows one crate's layout immediately knows the other.
The intentional divergences (mobile's `storage_root: PathBuf` instead of `Arc<Mutex<Option<PathBuf>>>`,
the inline orphan reconcile in `spawn_workspace_opener`,
no `settings.rs` / `fs_watcher.rs` / `commands/{shortcuts,theme}.rs`) live entirely inside `workspace_open.rs` + `state.rs`,
so the command files read identically.

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
  Frontend wrapper: `setWorkspace(path) → Promise<boolean>` in `src/lib/api.ts` (returns `is_icloud`).
- `pick_in_icloud()` resolves `<container>/Documents` for the "store in iCloud" opt-in, zero native code; returns `null` when the device isn't signed into iCloud.
  Frontend wrapper: `pickInICloud() → Promise<string | null>` in `src/lib/api.ts`.

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
Storing just the string path in `WorkspaceCfg.last` only works for the sandbox, the local default, and the app's own iCloud container.
The follow-up adds an `objc2` bridge (mirroring `icloud_path.rs`) that presents the picker, serialises a bookmark, persists it next to `actor`, and resolves it on boot before `resolve_storage_root`.
Until then, `set_workspace` works for any path the frontend can already reach without a scoped bookmark, and `pick_in_icloud` covers the iCloud case.

## Change detection: generic, not iCloud-only

- **iroh (primary).**
  The transport fires a reload signal whenever it writes peer ops; `iroh_sync.rs` bridges it to the `workspace-ready` Tauri event.
  This covers a local folder with **no iCloud watcher at all**.
- **iCloud watcher (conditional).**
  `OutlOpsWatcher.swift` (`NSMetadataQuery` + `NSFileCoordinator`) is the iCloud-only detector.
  It is **conditional on the folder being inside iCloud**: its query is scoped to `NSMetadataQueryUbiquitousDocumentsScope`, so for a local folder it matches nothing and stays dormant — it is never *required*.
  `workspace_open::storage_is_icloud()` / `icloud_path::is_inside_icloud()` answer the "is this path in iCloud" question on the Rust side (logged at boot).

**DEFERRED — iCloud + iroh write coexistence.**
When the chosen folder lives in iCloud, both the iCloud daemon and iroh write `ops-*.jsonl`.
iroh already serialises its own writes (append-lock), the parser recovers glued lines, and the watcher already coordinates *reads* via `NSFileCoordinator`.
Wrapping our own op-log *writes* in `NSFileCoordinator` on the iCloud path (so the iCloud daemon can't interleave) is the remaining coexistence hardening; it touches `JsonlStorage`'s append in `outl-core` (shared) and is left as a follow-up.
**The local-folder path has no such concern and is fully clean.**

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

The textarea in `BlockRow.tsx` intercepts paste events whose payload looks like a bullet list (`@outl/shared/paste::looksLikeOutline`) and routes the text to `outl_actions::paste_markdown` via the `paste_markdown_at` Tauri command.
Plain text falls through to the browser's default splice so a one-off URL or code snippet still pastes the way the user expects.

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

## Peer / device management (`outl_peer_list` / `outl_peer_remove`)

`commands/peers.rs` exposes two Tauri commands that read and edit the iroh
peers file (`~/.outl/peers.json`) via `outl_sync_iroh::PeersStore`:

- `outl_peer_list() -> Vec<PeerDto>` — lists paired devices (`node_id`,
  `alias`, `added_at`).
- `outl_peer_remove(id: String) -> bool` — removes peers whose `node_id`
  starts with the given prefix; `true` if any matched.

These are the **only** commands that touch `peers.json` directly instead of
the workspace lock — peer pairing is sync-transport state, not workspace
state, so they don't go through `AppState`/`outl-actions`.
The store path is resolved from `dirs::home_dir()` (not the iCloud
container), matching where the iroh `SyncEngine` writes it.

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

When the user chooses to store the workspace in iCloud, the root is `<ubiquity-container>/Documents/` (resolved by `workspace_open::icloud_workspace_root()`).
This is **one option**, not the default — see "Storage is a chosen folder" above.
The container is already the `outl` namespace at the iCloud Drive level; nesting an extra `outl/` folder underneath is redundant and was removed in v0.
The TUI can point at the same path via `--path "<container>/Documents"`.

```
<container>/Documents/
├── journals/
│   └── YYYY-MM-DD.md            ← daily journal projections
├── pages/
│   ├── <slug>.md                ← regular page projections
│   └── <slug>.outl              ← sidecar (block IDs + hashes)
└── ops/
    ├── ops-<this_device>.jsonl  ← only THIS device writes here
    ├── ops-<other_device>.jsonl
    └── ...
```

- One `ops-*.jsonl` per actor. iCloud syncs files individually, so two devices never conflict at the filesystem layer.
- The folder is **`ops/`**, not `.ops/`. iCloud Documents skips paths starting with `.` when syncing across devices — using a dotted name silently breaks multi-device sync (the per-device jsonl never leaves its origin).
- `.md` files are projections regenerated after every mutation.
  Never parse them back to reconstruct workspace state — the op log is the source of truth.
- Sidecar files live next to the `.md` as `pages/<slug>.outl` (no leading dot).
  The dotted form was abandoned for the same iCloud reason as `.ops/` — dotted paths do not propagate across devices.

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
To bump the app version, edit `[workspace.package].version` at the repo root and that's it.
Everywhere else inherits:

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

A push to `main` triggers two workflows in parallel:

1. **`Release`** (`release.yml`) — auto-bumps `Cargo.toml` locally to `<base>-beta.<run_number>` (e.g.
   `0.5.1-beta.27`), cuts a tag `v0.5.1-beta.27`, builds desktop binaries, ships the Homebrew formula.
   Never commits the bump back.
2. **`Mobile`** (`mobile.yml`) — builds the signed iOS IPA from the *unbumped* `Cargo.toml`, then uploads it as the `outl-ios-release` artifact.
   Triggers `TestFlight` on success.
3. **`TestFlight`** (`testflight.yml`) — downloads the IPA artifact and uploads to App Store Connect via `xcrun altool`.

### CFBundleVersion (build number) scheme

Apple needs `CFBundleVersion` strictly monotonic across **every** IPA ever uploaded.
We can't reuse `tauri.conf.json.version` directly because the marketing version (`0.5.1`) repeats across many beta builds.
The scheme:

```
CFBundleShortVersionString = <SHORT_VERSION>            e.g. 0.5.1
CFBundleVersion            = <SHORT_VERSION><BETA_PAD>  e.g. 0.5.1027
                                              ^^^
                              beta number zero-padded to 3 digits
```

Where `BETA` comes from the latest `v<SHORT_VERSION>-beta.<N>` git tag (set by the `Release` workflow).
Fallback to Mobile's `github.run_number` when no beta tag exists for the current `SHORT_VERSION`.
Re-runs append `.<run_attempt>` as a 4th component to dodge Apple's duplicate guard.

The build number is set by patching the `.xcarchive`'s embedded `Info.plist` after `cargo tauri ios build` produces the archive, but **before** `xcodebuild -exportArchive` re-signs and exports the IPA.
This is the only injection point that survives the build because Tauri only exposes a single `version` field.

### What goes wrong if you forget this

- `tauri.conf.json` left with stale `"version"`: IPA ships with that static value regardless of `Cargo.toml` or `--config`.
  Apple sees a value that hasn't been bumped → 409 duplicate.
- Dropping `--config '{"version": "..."}'` from `cargo tauri ios build` in `mobile.yml`: Tauri's iOS path falls back to `1.0.0` (not to `Cargo.toml` as the docs imply).
  The sanity check in the `Patch archive CFBundleVersion` step catches this — don't disable it.
- Patching `gen/apple/.../Info.plist` directly before the build: Tauri regenerates the file from the merged config on every build.
  No-op.
- `xcrun altool --type ios` returns exit 0 even on 409 errors.
  The `Upload IPA to TestFlight` step in `testflight.yml` greps for `ERROR:` and exits non-zero explicitly — don't simplify that step.

## Testing

Two layers cover the mobile crate:

| Layer | Tool | What it covers |
|-------|------|----------------|
| Rust commands + storage | `cargo test -p outl-mobile` | `ICloudStorage`, command shims, page model glue |
| Frontend pure logic | `bun run test` (Vitest + happy-dom) | `markdown.tokenize`, `outline.flatten/findBlock/findInsertedAfter`, future helpers |

Every bug fixed in a pure helper (the tokenize duplicate, refs/tags extraction, fuzzy matching) must land with a unit test before merge so it never regresses.

Native bits (`main.mm` swizzle, BGTaskScheduler, NSMetadataQuery) are not covered by unit tests yet — they're observed via the NSLog probes shown on app boot.
If we add Swift Tests later they belong next to `main.mm` in `gen/apple/Tests/`.

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-mobile -- -D warnings`
3. `cargo test -p outl-mobile`
4. `bun run test` (Vitest, frontend)
5. Build pass: `cargo tauri ios build`
