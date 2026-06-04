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

The same rule extends to the **Solid frontend** (`src/`). Before
adding a helper that walks blocks, normalises text, or maps a
cursor across `\n`, check `outl-md`/`outl-actions` — the Rust
side likely already exposes it through a Tauri command or could
with a tiny addition. The two cross-runtime contracts already
documented below (`looksLikeOutline` mirroring
`outl_actions::paste::looks_like_outline`, and the UTF-16 caret
conversion) are *examples of contracts we explicitly maintain*,
not green-lights to keep cloning Rust logic into TS.

Workspace-level policy:
[`CLAUDE.md`](../../CLAUDE.md#reuse-first-no-parallel-implementations).

What this crate **does** own:

- iCloud Ubiquity Container resolution and the `Storage` impl on top.
- Per-device actor id persistence (`<sandbox>/actor`).
- Tauri command surface (argument parsing, error mapping).
- Solid frontend that consumes the commands.

## Opening a ref that may not exist yet

`[[avelino/outl]]`, `[[2026-06-04]]`, `#code-review`, picker entries
— every "tap a ref → see a page" path on the frontend goes through
**one** Tauri command, `open_ref(target)`, which wraps
`outl_actions::page::open_or_create_by_ref`. The single decision
tree (date → journal, else literal/slugified/title match → existing
page, else create as page) lives in the shared crate so a frontend
regex cannot drift from a backend parser the way it did before
`open_ref` existed.

What used to be wrong: the frontend split the journal-vs-page
decision with `/^\d{4}-\d{2}-\d{2}$/` and routed to one of two
strict-validating commands (`open_journal_for` / `open_page_by_slug`).
`[[2026-13-01]]` matched the regex, hit `open_journal_for`, and
surfaced an `invalid date slug` toast — even though falling through
to "create a regular page" was clearly the right behaviour.

`open_page_by_slug` is kept for the picker (the picker already
hands the command a clean slug from a known page). `open_journal_for`
stays for date-navigation commands (`previousDay` / `nextDay`) whose
input is derived from controlled state, not from a user tap. Every
**ref-click** code path on the frontend (`handleRefClick`,
`handleTagClick`) must call `openRef` so the decision tree is
single-sourced.

`resolve_ref` survives for autocomplete previews ("this ref will
land on `<page>`") but is **not** the navigation entry point — for
that, always call `openRef`.

## Paste from external apps

The textarea in `BlockRow.tsx` intercepts paste events whose payload
looks like a bullet list (`lib/paste.ts::looksLikeOutline`) and routes
the text to `outl_actions::paste_markdown` via the `paste_markdown_at`
Tauri command. Plain text falls through to the browser's default
splice so a one-off URL or code snippet still pastes the way the user
expects.

Cross-runtime contracts live here. All TS copies of Rust logic must
stay in sync with their Rust canonical source:

1. **`looksLikeOutline`** mirrors `outl_actions::paste::looks_like_outline`.
   Extending the Rust detector (e.g. accept `*` bullets or ordered
   lists) requires the same change in `lib/paste.ts` plus a Vitest case.
2. **Inline tokens.** No TS tokenizer. The backend runs
   `outl_md::tokenize_owned` on every block before it leaves the
   workspace and attaches the result as `BlockNode.tokens` /
   `Backlink.source_block.tokens`. `lib/markdown.tsx::MarkdownInline`
   consumes those tokens directly and renders each variant to JSX.
   Adding a token variant means extending `outl_md::InlineTok` plus
   `outl_md::InlineToken` plus the `InlineToken` TS union in `lib/api.ts`
   plus the renderer switch — but there is no TS regex to keep in
   sync, only a discriminant-to-render mapping.
3. **`detectRefContext`** in `lib/autocomplete.ts` mirrors
   `outl_tui::actions::overlay::detect_trigger` (the `[[` and `((`
   triggers; TUI also covers `#` and `/`). Local copy keeps the
   autocomplete popup off the Tauri round-trip per keystroke. Same
   sync rule as `looksLikeOutline`.
4. **Caret offset.** `textarea.selectionStart` is a UTF-16 code unit
   offset; the Rust backend expects a Unicode codepoint count.
   `lib/paste.ts::utf16OffsetToCharOffset` does the conversion before
   the Tauri call so pasting after an emoji lands the splice at the
   right place. Skip this and supplementary-plane characters shift
   the splice by one per char. (Not a mirror — runtime gap. Listed
   alongside the mirrors because the cross-runtime concern is the
   same: don't let the frontend drift from what the backend assumes.)

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

# Release archive for TestFlight (local smoke test only — CI ships)
cargo tauri ios build
```

After the first run, the iCloud capability must be confirmed in
Xcode (Signing & Capabilities → iCloud → Containers →
`iCloud.app.outl.mobile-app`).

## Versioning + TestFlight release

**Single source of truth: `Cargo.toml` workspace `version`.** To bump
the app version, edit `[workspace.package].version` at the repo root
and that's it. Everywhere else inherits:

| Field | Where it lives | How it's resolved |
|-------|----------------|-------------------|
| Rust crate version | `crates/outl-mobile/src-tauri/Cargo.toml` | `version.workspace = true` |
| Tauri config version | `crates/outl-mobile/src-tauri/tauri.conf.json` | Field intentionally **omitted** in the source; CI injects it via `cargo tauri ios build --config '{"version": "<short>"}'` |
| `CFBundleShortVersionString` | iOS `Info.plist` | Tauri propagates from `--config` during `cargo tauri ios build` |
| `MARKETING_VERSION` / `CURRENT_PROJECT_VERSION` | `gen/apple/.../project.pbxproj` | Same — Tauri regenerates from the merged config every build |

**Why `--config` and not just rely on Tauri's `Cargo.toml` fallback?**
The docs say Tauri uses `Cargo.toml` when `version` is missing, but
the iOS code path doesn't honor that — it falls back to `1.0.0`
instead. So CI reads the workspace version itself (`awk` against
`Cargo.toml` in the `Compute build metadata` step) and passes it via
`--config`. That keeps `Cargo.toml` as the only place a human bumps,
and the `Patch archive CFBundleVersion` step has a sanity check that
aborts the build if the propagated short version doesn't match what
was passed in.

**Never** put `"version": "x.y.z"` back in `tauri.conf.json`. If it's
present, Tauri uses the static value instead of the `--config`
override, and the two drift the moment someone bumps the workspace.

### CI release flow

A push to `main` triggers two workflows in parallel:

1. **`Release`** (`release.yml`) — auto-bumps `Cargo.toml` locally to
   `<base>-beta.<run_number>` (e.g. `0.5.1-beta.27`), cuts a tag
   `v0.5.1-beta.27`, builds desktop binaries, ships the Homebrew
   formula. Never commits the bump back.
2. **`Mobile`** (`mobile.yml`) — builds the signed iOS IPA from the
   *unbumped* `Cargo.toml`, then uploads it as the
   `outl-ios-release` artifact. Triggers `TestFlight` on success.
3. **`TestFlight`** (`testflight.yml`) — downloads the IPA artifact
   and uploads to App Store Connect via `xcrun altool`.

### CFBundleVersion (build number) scheme

Apple needs `CFBundleVersion` strictly monotonic across **every** IPA
ever uploaded. We can't reuse `tauri.conf.json.version` directly
because the marketing version (`0.5.1`) repeats across many beta
builds. The scheme:

```
CFBundleShortVersionString = <SHORT_VERSION>            e.g. 0.5.1
CFBundleVersion            = <SHORT_VERSION><BETA_PAD>  e.g. 0.5.1027
                                              ^^^
                              beta number zero-padded to 3 digits
```

Where `BETA` comes from the latest `v<SHORT_VERSION>-beta.<N>` git
tag (set by the `Release` workflow). Fallback to Mobile's
`github.run_number` when no beta tag exists for the current
`SHORT_VERSION`. Re-runs append `.<run_attempt>` as a 4th component
to dodge Apple's duplicate guard.

The build number is set by patching the `.xcarchive`'s embedded
`Info.plist` after `cargo tauri ios build` produces the archive, but
**before** `xcodebuild -exportArchive` re-signs and exports the IPA.
This is the only injection point that survives the build because
Tauri only exposes a single `version` field.

### What goes wrong if you forget this

- `tauri.conf.json` left with stale `"version"`: IPA ships with that
  static value regardless of `Cargo.toml` or `--config`. Apple sees a
  value that hasn't been bumped → 409 duplicate.
- Dropping `--config '{"version": "..."}'` from `cargo tauri ios build`
  in `mobile.yml`: Tauri's iOS path falls back to `1.0.0` (not to
  `Cargo.toml` as the docs imply). The sanity check in the
  `Patch archive CFBundleVersion` step catches this — don't disable
  it.
- Patching `gen/apple/.../Info.plist` directly before the build: Tauri
  regenerates the file from the merged config on every build. No-op.
- `xcrun altool --type ios` returns exit 0 even on 409 errors. The
  `Upload IPA to TestFlight` step in `testflight.yml` greps for
  `ERROR:` and exits non-zero explicitly — don't simplify that step.

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
