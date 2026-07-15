# Clients and shared logic

outl has multiple clients today (TUI, mobile, desktop) and more coming (plugins).
They all sit on top of the same workspace and the same op log.
To keep them honest, we route every workspace operation through one shared crate: **`outl-actions`**.
The TS+Solid frontends share `@outl/shared` (`crates/outl-frontend-shared`) for everything pure (DTO types, `<MarkdownInline />`, paste helpers, copy wrappers, autocomplete).

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
│   history (bounded undo/redo stacks + .md snapshot restore)   │
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
| Committed-mutation undo / redo (snapshot stacks + `.md` restore via reconcile) | `outl-actions::history` |
| Code-block execution (runtimes + orchestration) | `outl-exec`            |
| Cross-client "run a fence" glue (`run_code_block`) | `outl-actions::exec` |
| Tauri command bodies, wire DTOs, plugin thread (Boa `!Send`), `AppHost` / `StorageRootProvider` traits — shared by `outl-desktop` and `outl-mobile` src-tauri; both clients are thin wrappers | `outl-tauri-shared` |
| TUI: keymaps, modes, overlays, in-flight AST manipulation | `outl-tui`         |
| Desktop: FS watcher, settings IO, Solid frontend (3-pane, OS-standard shortcuts) | `outl-desktop` |
| Mobile: iCloud container resolution, iOS-native bridges (`NSMetadataQuery`, `BGTaskScheduler`), Solid frontend | `outl-mobile` |
| CLI subcommands                      | `outl-cli`                      |

## When to put logic in `outl-actions`

Yes if any of these are true:

- Two or more clients (today or in the next quarter) need the same op.
- The function takes only `Workspace + HlcGenerator` and returns `Result<_, ActionError>`.
- It produces ops by way of `Workspace::apply` — no direct storage writes, no filesystem touches outside `journal::write_md_atomic`.

No if:

- It manipulates client UI state (selection, modes, toasts, focus, keymaps).
- It manipulates an in-flight `Vec<OutlineNode>` that hasn't been parsed back into a workspace yet.
  Those helpers live in `outl-md::outline_ops`, re-exported through a one-liner shim at `outl-tui/src/outline_ops.rs` because the mobile client needs them too.
  They're workspace-free pure AST manipulation, so they sit in `outl-md` rather than `outl-actions`.
- It's storage-backend-specific (iCloud watcher, future ChronDB) — those implement `outl_core::Storage` in the binary that needs them.

## Surfacing parser warnings on every client

A user can drop a `.md` into the workspace by hand, paste an exported Roam/Logseq tree, or edit a file in vim before outl ever saw it.
When that file doesn't match the outl dialect (e.g. starts with `# heading`, contains a free paragraph, or imports a markdown table), the parser **does not** drop content.
It preserves the line as a regular block and records the recovery in `ParsedPage.warnings: Vec<outl_md::ParseWarning>`.

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

Every client that lets the user execute a `` ```lang ``` `` block (TUI `g x`, desktop `Cmd+Shift+X` / Run button, mobile long-press → "Run code") goes through **one** shared entry point:
`outl_actions::exec::run_code_block(ws, hlc, root, registry, page, block)`.

```text
client gesture (TUI chord / Cmd+Shift+X / long-press)
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
The duplication that used to live in `outl-desktop/src-tauri/src/commands/exec.rs` and `outl-mobile/src-tauri/src/exec.rs` was collapsed into this single function.
`flat_index_for_block` and the path lookup were the canonical "two parallel implementations" case the workspace-level Reuse-first policy exists to prevent.

The runtime catalog is selected per-binary via `outl-exec` features:

- `outl-cli`, `outl-tui`, `outl-desktop` — default features (Lisp + JS + Python + Lua + Rust via wasmtime).
- `outl-mobile` — opts out of `lang-rust` (wasmtime is heavy and trips iOS code-signing restrictions on dynamic code generation).
- `outl-actions` — `default-features = false` so it never drags `wasmtime` into the mobile IPA via the back door.

## Structural templates

A **template** is any page with a non-empty `template::` property; its outline is a body every client can deep-copy under a target block (see [`docs/templates.md`](templates.md) for the authoring model).
Instantiation is reachable from every surface (TUI/CLI `/template <name>`, MCP), and both GUI clients wrap the **same** two shared command bodies in `outl_tauri_shared::commands::template` — no plugin needed:

- `list_templates` → `Vec<TemplateDto>` (`{ name, slug, duplicate }`) — wraps `outl_actions::list_templates`.
- `instantiate_template_at(name, target_block)` → refreshed `PageView`.
  It resolves the target block's enclosing page (slug + journal date), calls `outl_actions::instantiate_template`, reprojects, and announces the ops.
  An unknown name or stale block id is a typed error the client toasts.

Both clients register the wire command as `list_templates_cmd` plus `instantiate_template_at`.
The `_cmd` suffix dodges a glob-import collision with the `outl_actions::list_templates` re-export.
The TS wrappers are `listTemplates()` / `instantiateTemplateAt(name, targetBlockId)` in `@outl/shared/api/commands`.

Client affordances (chrome per-client, one backend):

- **Desktop** — the block-initial `/` slash menu lists `template: <name>` entries alongside plugin commands.
  They are injected via `templateSlashCommands` under a reserved `@outl/template` sentinel `plugin_id`.
  Picking one instantiates under the selected block; `OutlineView`'s `onRunPluginCommand` intercepts the sentinel and calls `instantiateTemplateAt` instead of `pluginRun`.
- **Mobile** — the block long-press menu has an "Insert template" action that opens `TemplateSheet` (a bottom sheet listing templates).
  Picking a template instantiates it under the long-pressed block and applies the returned `PageView`.

## Copy and paste

Every client supports copying blocks as clean outl markdown and pasting markdown from external apps.

### Copy out

| Client | How to copy | What is copied |
|---|---|---|
| TUI | `yy` / `Y` (Normal) or `y` (Visual range) | Selected block(s) + full subtrees as canonical outl markdown, written to the OS clipboard via arboard (X11/Wayland/macOS) with an OSC 52 fallback for SSH / tmux / Crostini. Status line confirms: `yanked N block(s) → clipboard` or `(clipboard unavailable)`. |
| Desktop | `Y` (Normal) or `y` (Visual range) | Same serialisation via the `copy_markdown` Tauri command (`outl_actions::copy_markdown`) + `navigator.clipboard.writeText`. |
| Mobile | Long-press → "Copy" in the context menu | Block + full subtree via the `copy_markdown` Tauri command, written to the iOS clipboard. |

The serialisation is handled by `outl_actions::copy_markdown` on the Rust side and `copyMarkdown` (`@outl/shared/api/commands`) on the TS side.
The format is canonical outl markdown: `- ` bullets, 2-space indent, inline block props alpha-sorted, `TODO`/`DONE`/`> ` prefixes verbatim.
Pasting the result back into any outl client reconstructs the same tree.

### Paste in

Every client distinguishes **paste with formatting** from **paste without formatting**.

**With formatting** (`Cmd/Ctrl+V` on desktop, `p` in the TUI, the default paste on mobile) routes the clipboard through `outl_actions::paste_markdown` when the content looks structured.
It applies these conversions:

- Roam `{{[[TODO]]}}` / `{{[[DONE]]}}` → `TODO` / `DONE` prefix.
- GitHub `- [ ]` / `- [x]` → `TODO` / `DONE` prefix.
- Logseq `id::` metadata → stripped.
- 4-space indent → 2-space indent.
- Multi-line plain text (two or more non-blank lines) → one block per non-blank line (blank lines are ignored).
- Single-paragraph plain text → falls through to the browser/terminal default splice.

**Without formatting** (`Cmd/Ctrl+Shift+V` on desktop, `P` in the TUI; not available on mobile) calls `outl_actions::paste_plain`.
The raw clipboard text is inserted as a single block at the anchor with no normalisation, outline parsing, or paragraph splitting.
Use this when the text contains underscores, brackets, or other characters that would be misread as markdown syntax.

The routing decision uses two helpers from `@outl/shared/paste`:
`looksLikeOutline` detects bullet structure;
`hasMultipleParagraphs` is true when the text has two or more non-blank lines (blank lines are ignored).
Either condition sends the paste to the backend (structured path) on `Cmd/Ctrl+V`.

| Client | With formatting | Without formatting |
|---|---|---|
| TUI | `p` — routes to backend when outline or multi-paragraph; else native splice | `P` — raw text, single block |
| Desktop | `Cmd/Ctrl+V` — routes to backend when outline or multi-paragraph; else native splice | `Cmd/Ctrl+Shift+V` — raw text, single block |
| Mobile | paste — routes to backend when outline or multi-paragraph; else native splice | not available |

### TUI mouse drag copy (opt-in)

With `[tui] mouse_capture = true` in `~/.config/outl/config.toml`, dragging across blocks in the TUI selects a range and copies it as outl markdown on release.
See [tui.md → Mouse capture](tui.md#mouse-capture-opt-in) and [config.md → `[tui]`](config.md#tui) for details.

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
Composition with TODO/DONE follows a canonical order: `"TODO > body"` (task state before quote marker), so the backend's `split_todo` still surfaces `block.todo` in the DTO when the block is also quoted.
Both `toggle_quote` and `cycle_todo` peel both prefixes off and re-emit in canonical order — an externally authored `"> TODO foo"` gets normalised to `"TODO > foo"` on the first toggle from a client.
Multi-line quote bodies keep the `> ` on every continuation line so the `.md` stays a valid CommonMark blockquote.
Children of a quoted block are **not** implicitly quoted — the marker lives on the block, not on its subtree.
GUI clients render the outline bullet outside the quote chrome, so the quote is visually the body's content rather than a nested list item.
Inline tokens (`**bold**`, `[[ref]]`, `#tag`, `((blk-…))`) continue to tokenize **inside** the body — the wrapper is transparent.

## Zoom / focus on a block (Roam/Workflowy)

Zooming makes one block the outline root so only its subtree renders — the Roam/Workflowy "focus" gesture.
It is **local view state**, never an `Op` and never a Tauri round-trip: the client already holds the whole `outline`, so zoom is a pure decision about what to render, and it does not converge across devices (each device focuses independently).
On the GUI clients (desktop + mobile) the subtree lookup + ancestor breadcrumb is the shared `focusSubtree(blocks, blockId)` in `@outl/shared/outline`, returning `{ root, breadcrumb }` or `null` when the id is gone.
This is pure view state with no Rust mirror — the id never leaves the frontend; the TUI reaches the same behaviour through its own path-based zoom stack in `outl-tui/src/actions/zoom.rs`.
Every client resolves the focused id against the **live** outline on each render, so an edit / collapse inside the zoom stays reflected, and a stale target (block deleted or moved to another page → `null`) transparently falls back to the full page.
Switching pages drops the zoom, so focus is scoped to the page it was set on.

On mobile the gesture is touch, not a chord: a tap on a block's **plain bullet dot** zooms in (`Journal.tsx` holds one `focusBlockId` signal; `BlockRow` → `BulletOrCheckbox` fires `onFocusBlock` when wired).
That repurposes the plain dot's old mark-as-TODO tap — TODO stays reachable in the long-press context menu — while the TODO/DONE checkbox and the collapse triangle keep their taps, so no gesture collides.
Zoom-out is stackless: a `← Back` header button plus a tappable ancestor breadcrumb, derived by re-resolving `focusSubtree` and stepping to `breadcrumb.at(-1)` (the parent) or exiting when already top-level.

## Keyboard accessory bar (mobile)

The edit toolbar docked above the soft keyboard — a Bear-style pill of `+` / indent / bold / `[[` / TODO / delete / hide-keyboard — plus the ref/emoji chip strip, exist in **two renderings**.
iOS is native: `OutlToolbarView` supplied as the private `WKContentView`'s `inputAccessoryView` via the `OutlSwizzle` swizzle, suggester in `OutlSuggestOverlay` / `OutlSuggestView`.
Android (and iOS later) is web: `KeyboardAccessory.tsx` renders `<SuggesterStrip />` + `<KeyboardToolbar />` in the webview, bottom-anchored at `useKeyboardInset()`, gated on `isAndroid && editingId() !== null`.
There is no web equivalent to iOS's docked accessory view, so the native bar stays until the web one is proven on a device.

The logic is shared, never triplicated.
The action catalog + most-frequently-used ordering live in `@outl/shared/toolbar`, a port of `crates/outl-mobile/swift/OutlKit/Toolbar/{ToolbarAction,ToolbarMFU}.swift`.
The action string ids (`newLine`, `indent`, `todo`, …) are the wire contract the iOS native bar ships to JS via `window.__outlToolbar(action)`.
So the Swift `ToolbarAction` copy and the TS catalog stay byte-identical — rename on both sides in one commit — until the native bar retires.
Both bars dispatch through the same `Journal.tsx` `dispatchToolbarAction` (the native bridge just assigns it to `window.__outlToolbar`).
Both accept a chip through the same `window.__outlSuggesterPicked` callback.
The web strip reads the reactive `nativeSuggesterState` signal that `setNativeSuggesterState` feeds alongside the `window.__outlSuggesterState` global the native strip polls.

Two invariants the web bar must keep.
Every button calls `e.preventDefault()` on `onPointerDown` so the tap can't blur the textarea and dismiss the keyboard.
`AndroidManifest.xml`'s activity sets `android:windowSoftInputMode="adjustResize"`.
With the `interactive-widget=resizes-content` meta the visual viewport then shrinks on keyboard show and `useKeyboardInset()` collapses to ~0, so `bottom: inset` rests the bar on the keys; drop `adjustResize` and it floats behind the keyboard.
Validation needs an Android device/emulator build — docking and `visualViewport` don't exercise under host `cargo check` or `bun run dev`.

## Backlinks order (issue #142)

Every client sorts the "Linked from" list the same way: `outl_actions::sort_backlinks` groups backlinks by source page (each page's blocks stay contiguous, in document order) and orders the pages by how recently each was referenced.
The direction — `newest` (default, most recently referenced page on top) or `oldest` — is a pure display preference stored in `[display] backlinks_order` (`config.toml`), same non-converging policy as `theme.preset`.

| Client | Toggle | Persistence |
|---|---|---|
| TUI | `Ctrl+O` (Normal mode) | writes `config.toml` directly via `outl_config::save` |
| Desktop | direction button in the `InlineBacklinks` header | `set_backlinks_order` Tauri command |
| Mobile | direction button in the `BacklinksSection` header | `set_backlinks_order` Tauri command |

See [config.md → `[display]`](config.md#display) for the schema and [shortcuts.md](shortcuts.md) for the TUI chord.

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

### Split-brain slug repair (boot)

`outl_actions::merge_duplicate_slug_roots(ws, hlc)` is the boot-time repair for the split-brain bug where two creators minted **different** ids for the same slug.
Two `2026-07-10` journal roots split a day's content across two roots, so the client flickers between them.
For each slug with more than one root it picks a canonical survivor: the `page_id_from_slug` id if one is present, else the root with the most descendants, tie-broken by smallest id.
It then re-parents every child of the other roots under the survivor preserving order (no data loss), and trashes the emptied duplicates.
Every step is an `Op` through `Workspace::apply`, so running it on **any** client converges on every device via the CRDT; it's idempotent (returns 0 on a clean workspace) and safe to call on every boot.
Clients call it once at startup alongside `migrate_legacy_into_today`.
Belt-and-suspenders: `find_by_slug` already resolves the same canonical winner deterministically, so the UI stops flickering even before the merge runs.

### Doubled journal title repair (background)

Page and journal roots get a **deterministic** id (`page_id_from_slug`), so two devices that create the same slug offline mint the same root and its `Op::Create` converges cleanly.
Before the fix, each device also wrote the title straight into the root's Yrs text.
Those two concurrent inserts at position 0 concatenated instead of converging — a `2026-06-25` journal opened offline on two devices ended up titled `2026-06-252026-06-25`.
`open_or_create` now writes the title into a `title::` property instead (`Op::SetProp`, last-write-wins by HLC), and only when the title differs from the slug.
A journal's title always equals its slug, so journals carry no `title::` property and no `title::` line lands in their `.md`.
Regular pages created in-app now render `title:: <title>` at the top of their `.md`.
`outl_actions::repair_doubled_journal_titles(ws, hlc)` cleans up journals corrupted before the fix.
Any journal root whose text is its slug repeated two or more times gets that text cleared — an `Op`, so the repair converges to every device — and the title falls back to the slug.
Idempotent, journal-only, and run on the **background** reconcile pass, not boot, since it scales with page count.
Desktop's `spawn_background_reconcile` and mobile's `spawn_workspace_opener` both call it.

### Peer reachability indicator (P2P / iroh transport)

When the iroh transport is running, the desktop / mobile "online / offline" dot reads `SyncTransport::peer_health()` — a reachability snapshot the transport fills from its **own** dials (boot connect, catch-up loop, gossip-triggered sync).
The GUI must **never** stand up a second iroh endpoint to probe peers: a second endpoint sharing the device identity hijacks the relay route from the live sync endpoint, and inbound sync gets refused.
The `outl_peer_status` command merges the snapshot onto the full `peers.json` list, so a peer the transport hasn't dialed yet shows offline.
The CLI's `outl peer status` is the lone exception (no running transport), so it keeps the transient-endpoint probe.
See `crates/outl-sync-iroh/CLAUDE.md` → "One endpoint per identity".

See `crates/outl-mobile/CLAUDE.md` for the full bundle ID, signing team, container ID set required to build it, and the `NSFileCoordinator`-based peer-file materialisation step that has to run before any read of a peer `ops-*.jsonl`.

## Opening a page from a user-typed ref

When a click on `[[avelino/outl]]`, `#code-review`, `[[2026-06-04]]`
or a picker field hands a client a string, the client must not split
the "journal vs page" decision between a frontend regex and a
backend parser.
The two will drift.
They already did:
`[[2026-13-01]]` matched the mobile frontend's `^\d{4}-\d{2}-\d{2}$`
shape regex, the command then fed `2026-13-01` into the strict date
parser, and the user got an `invalid date slug` toast for what
should have been a regular page.

The canonical entry point is
`outl_actions::resolve::open_or_create_by_ref(target)` (re-exported at the crate root).
It runs the whole decision tree in one place:

1. Date-shaped target → journal (semantic validator, not the regex
   shape — `2026-13-01` falls through).
2. Literal slug match → existing page (clean slug from the picker).
3. Slugified slug match → existing page (`[[avelino/outl]]` finds
   `pages/avelino-outl.md` even if the ref was typed before the
   page existed).
4. **`@`-prefixed mention sugar** (`[[@avelino]]`, `[[@Thiago Avelino]]`) — strip the `@` and resolve via steps 1-3 against the bare name.
   When nothing matches, create the page and set `type:: person` automatically so the next `@` mention surfaces it in the autocomplete popup without any property editing.
   The `@` is purely the link affordance; page identity does not carry it.
5. Case-insensitive title match → existing page.
6. Fallback: create a fresh page via `open_or_create_by_name` (slugifies disk path, keeps the typed string as title).

### `@` mention autocomplete

Every client surfaces a person picker on a word-initial `@`.
The popup is filtered to pages where `type:: person` is set, ranked through the shared `outl_actions::search_persons(query)` helper owned by the `person` module.
The TUI calls it directly; desktop and mobile expose it as the `search_persons` Tauri command.
Accepting a candidate inserts `[[@<title>]]`, a regular wikilink whose target carries the `@` as a visual prefix only.
Mentions ride every existing page-ref code path (render, roundtrip, navigation), so adding the trigger costs no new render/matching/reconcile branch.

A person's backlinks panel surfaces both forms (`[[avelino]]` and `[[@avelino]]`).
`backlinks_for_page` scans the `@`-prefixed alias when `meta.page_type == Some("person")`.
Plain pages do not, so a stray `[[@projeto]]` in a non-person page's backlinks does not trigger false positives.

Every client that turns a tap on a ref / tag / picker entry into a page view should wrap this single helper.
There is no client-side discrimination to maintain.
The `open_or_create_by_name(name, kind)` variant stays for callers that already know they want a regular page (no date branch).

## Deep links (`outl://`)

External launchers open a client at a specific page or daily note through an `outl://` URL.
The Raycast extension's "Enter → open in app" is the first consumer; links shared into the mobile app are the second.

The scheme is tiny and identical on every platform:

| URL | Opens |
|---|---|
| `outl://daily/today` | today's journal |
| `outl://daily/2026-06-25` | the daily for that ISO date |
| `outl://page/<slug>` | the page (slug may nest: `outl://page/ai-agent/learning`) |

Parsing lives in **one** place: `outl_actions::parse_deep_link` returns a `DeepLinkTarget` (`Today` / `Daily(date)` / `Page(slug)`).
Each client maps that target onto the same `open_*` command its UI already calls (`open_today_journal` / `open_journal_for` / `open_page_by_slug`) and focuses its window.
The parser never touches a `Workspace` — it is pure string → enum — so the desktop and mobile handlers cannot drift on the contract the way two hand-rolled URL parsers would.

A malformed URL (wrong scheme, unknown kind, bad date, path-traversal slug) returns `DeepLinkError`; the client logs it and no-ops.
It must never crash the app or materialise a stray page.

Registration is per-client transport, not shared logic:
the desktop registers the scheme via `tauri-plugin-deep-link` (+ `tauri-plugin-single-instance` so the URL reaches an already-running instance on Linux/Windows);
iOS registers the same scheme through the plugin's mobile config, which injects `CFBundleURLTypes` into the generated `Info.plist`.
Universal Links (`https://outl.app/…`) are a later addition — they need an Associated Domains entitlement and a hosted `apple-app-site-association`, so the custom scheme ships first.

## Adding a new client

The pattern is small:

1. Take a dependency on `outl-core`, `outl-md`, `outl-actions`.
2. Open a `JsonlStorage` rooted at `<workspace>/ops/`, or bring your own `Storage` impl.
3. Open a `Workspace` with that storage; hold one `HlcGenerator` per device.
4. Call into `outl-actions` for every user-visible mutation.
5. Call `outl_actions::apply_journal_md` (or the per-page equivalent when we add it) if you want the `.md` projection on disk.

What you write in your client crate: command surface (Tauri, keyboard, HTTP, …), UI state, navigation, animations.
Nothing else.
