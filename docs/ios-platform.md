# iOS platform integration (mobile client)

What the iOS shell of `outl-mobile` needs from outside Rust.
Three things: the bundle identifiers Apple treats as global, the background-sync wiring across Info.plist / Swift / FFI, and the iCloud catches that only exist when the workspace folder lives in a ubiquity container.
Every item here can only be validated with a **device or simulator build** — a host `cargo check` proves nothing about it.

The crate contract (what stays thin, what delegates to `outl-actions`) lives in [`crates/outl-mobile/CLAUDE.md`](../crates/outl-mobile/CLAUDE.md).
The user-facing background-sync behaviour is [sync.md → Background sync on iOS](sync.md#background-sync-on-ios); the build + release commands are [development.md](development.md#mobile-ios-simulator).

---

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

---

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

---

## iCloud layout (opt-in destination)

When the user opts into iCloud, the root is `<ubiquity-container>/Documents/` (`workspace_open::icloud_workspace_root()`) — **one option**, not the default.
The container is already the `outl` namespace, so no extra `outl/` nesting; the TUI uses `--path "<container>/Documents"`.
Layout is the standard `journals/` + `pages/` (`.md` + `.outl` sidecar) + `ops/` (one `ops-<actor>.jsonl` per device).
**iCloud trap:** every path must be undotted — iCloud Documents skips `.`-prefixed paths across devices, so `ops/` (not `.ops/`) and `pages/<slug>.outl`, else the file never leaves its origin.

---

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
