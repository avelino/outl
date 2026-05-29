# CLAUDE.md — outl-mobile

Tauri 2 mobile client (iOS first, Android later). Solid.js + Tailwind
frontend, Rust backend that **must stay thin** — every workspace
operation is delegated to `outl-actions`.

## Layering

```text
outl-core                    (CRDT, op log, storage trait)
outl-md                      (.md parse/render, sidecar)
outl-actions                 (workspace operations + SyncEngine, shared with TUI)
   ↑
outl-mobile (this crate)
   ├── icloud_path.rs        (NSFileManager bridge — iOS-only)
   ├── lib.rs                (Tauri commands: parse args → outl-actions → render)
   ├── gen/apple/.../main.mm (NSMetadataQuery + NSFileCoordinator iCloud watcher)
   └── (frontend in ../src)  (Solid components, Tailwind, Tauri bridge)
```

The op log backend is the shared `outl_core::storage::JsonlStorage`;
there is no `icloud_storage.rs` because the only iCloud-specific work
is resolving the ubiquity container path (via `icloud_path.rs`) and
forcing peer-file materialisation before reads (via `main.mm`). The
storage trait stays generic; the transport gets handled outside it.

## Hard rule

**This crate adds no business logic.** If a Tauri command does
something that involves the workspace shape (edit, move, todo,
journal render), it delegates to `outl-actions`. If you find yourself
writing a tree walk or an op-generating helper inside `lib.rs`, stop
— move it to `outl-actions` instead. The TUI will need it too.

What this crate **does** own:

- iCloud Ubiquity Container resolution and the `Storage` impl on top.
- Per-device actor id persistence (`<sandbox>/actor`).
- Tauri command surface (argument parsing, error mapping).
- Solid frontend that consumes the commands.

## iCloud layout

The workspace root is `<ubiquity-container>/Documents/`. The container
is already the `outl` namespace at the iCloud Drive level; nesting an
extra `outl/` folder underneath is redundant and was removed in v0.
The TUI is expected to point at the same path via
`--path "<container>/Documents"`.

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

- One `ops-*.jsonl` per actor. iCloud syncs files individually, so
  two devices never conflict at the filesystem layer.
- The folder is **`ops/`**, not `.ops/`. iCloud Documents skips paths
  starting with `.` when syncing across devices — using a dotted name
  silently breaks multi-device sync (the per-device jsonl never leaves
  its origin).
- `.md` files are projections regenerated after every mutation. Never
  parse them back to reconstruct workspace state — the op log is the
  source of truth.
- Sidecar files live next to the `.md` as `pages/<slug>.outl` (no
  leading dot). The dotted form was abandoned for the same iCloud
  reason as `.ops/` — dotted paths do not propagate across devices.

## Peer-file materialisation (the iCloud catch)

iCloud syncs file metadata aggressively and file content lazily. When
`NSMetadataQuery` fires on a peer's `ops-<actor>.jsonl`, the file's
bytes may not be on disk yet — a `std::fs::open` returns an empty
placeholder. The Rust side sees a truncated op log; the merge is
wrong; the projection writes a broken `.md` back.

`main.mm`'s `OutlOpsWatcher.onUpdate:` works around this in two steps:

```objc
[fm startDownloadingUbiquitousItemAtURL:url error:&startErr];
NSFileCoordinator *coord = [[NSFileCoordinator alloc] initWithFilePresenter:nil];
[coord coordinateReadingItemAtURL:url
                          options:NSFileCoordinatorReadingForUploading
                            error:&coordErr
                       byAccessor:^(NSURL *u) { (void)u; }];
```

`startDownloadingUbiquitousItemAtURL` requests materialisation;
`NSFileCoordinator` blocks until the file is fully on disk. Only after
that does the watcher fire `window.__outlOpsChanged()` so the
frontend can call `reload_workspace`. Skip either step and you race
the iCloud download daemon.

## Bundle / signing

- Bundle id: `app.outl.mobile-app`
- Team: `CPEEKT3E77` (paid Apple Developer Program)
- iCloud container: `iCloud.app.outl.mobile-app`
- Display name (Files.app / iCloud Drive): `outl`
- Category: `public.app-category.productivity`
- Entitlements: `com.apple.developer.icloud-services` +
  `icloud-container-identifiers` + `ubiquity-container-identifiers`

Bundle ID + iCloud container are **global** in the Apple Developer
ecosystem. If you change either, also update:

1. `tauri.conf.json` → `identifier`
2. `src-tauri/src/lib.rs` → `ICLOUD_CONTAINER_ID`
3. `gen/apple/outl-mobile.xcodeproj/project.pbxproj` →
   `PRODUCT_BUNDLE_IDENTIFIER`
4. `gen/apple/outl-mobile_iOS/outl-mobile_iOS.entitlements`
5. `gen/apple/outl-mobile_iOS/Info.plist` →
   `NSUbiquitousContainers` key
6. `gen/apple/project.yml` → `bundleIdPrefix` and
   `PRODUCT_BUNDLE_IDENTIFIER`

## Running

```bash
cd crates/outl-mobile

# iOS simulator
cargo tauri ios dev "iPhone 17 Pro outl"

# Physical device (Mac + iPhone on the same WiFi)
cargo tauri ios dev "<device-name>" --host

# Release archive for TestFlight
cargo tauri ios build
```

After the first run, the iCloud capability must be confirmed in
Xcode (Signing & Capabilities → iCloud → Containers →
`iCloud.app.outl.mobile-app`).

## Testing

Two layers cover the mobile crate:

| Layer | Tool | What it covers |
|-------|------|----------------|
| Rust commands + storage | `cargo test -p outl-mobile` | `ICloudStorage`, command shims, page model glue |
| Frontend pure logic | `bun run test` (Vitest + happy-dom) | `markdown.tokenize`, `outline.flatten/findBlock/findInsertedAfter`, future helpers |

Every bug fixed in a pure helper (the tokenize duplicate, refs/tags
extraction, fuzzy matching) must land with a unit test before merge so
it never regresses.

Native bits (`main.mm` swizzle, BGTaskScheduler, NSMetadataQuery) are
not covered by unit tests yet — they're observed via the NSLog probes
shown on app boot. If we add Swift Tests later they belong next to
`main.mm` in `gen/apple/Tests/`.

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-mobile -- -D warnings`
3. `cargo test -p outl-mobile`
4. `bun run test` (Vitest, frontend)
5. Build pass: `cargo tauri ios build`
