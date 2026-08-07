# RFC 0002 — Every GUI client is Tauri 2 over one Rust surface

| | |
|---|---|
| **Status** | Shipped |
| **Issue** | [#2](https://github.com/avelino/outl/issues/2), [#3](https://github.com/avelino/outl/issues/3) (supporting: [#98](https://github.com/avelino/outl/issues/98), [#112](https://github.com/avelino/outl/issues/112)) |
| **PR** | — |
| **Date** | 2026-08-06 |
| **Reference doc** | [architecture.md § 12–13](../architecture.md#12-tauri-for-desktop), [clients.md](../clients.md) |
| **Invariant** | none of the numbered ones; root `CLAUDE.md` → "Decisions you don't get to revisit" (the two Tauri rows) plus the two shared-logic policy sections |
| **Guarded by** | `parses_today`, `parses_hierarchical_page`, `rejects_wrong_scheme`, `rejects_unknown_kind`, `rejects_invalid_dates`, `rejects_path_traversal_slug` (`crates/outl-actions/src/deeplink.rs`) — these pin the `outl://` contract of #98, **not** the framework choice; for that: **none found — gap**, held by the decision table and review only |

## Why

[#2](https://github.com/avelino/outl/issues/2) and [#3](https://github.com/avelino/outl/issues/3) ask the same thing twice: a client outside the terminal that opens the *same* workspace as the CLI and TUI, no migration, no conversion.

#3 shipped with a plan in its own body — "native UI per platform (SwiftUI / Compose), shared Rust core via a stable FFI surface" — and that plan is what this RFC replaces.
Its cost was structural.
Every user-visible operation would exist three times: once in Rust, once in Swift, once in Kotlin, so a fold or indent fix on iOS reaches Android only when someone ports it by hand.
outl already paid that bill smaller: `outl-tauri-shared` exists because two Tauri clients kept near-identical copies of nine files and every fix was ported twice.

## What we chose

Tauri 2 for mobile (#3) and desktop (#2), with the shared surface pushed as far up as it goes:

- **`outl-actions`** owns every workspace mutation — clients call it, never build ops themselves.
- **`outl-tauri-shared`** owns the Tauri layer both GUI clients embed: command bodies generic over `AppHost`, wire DTOs, the `ProjectionWriter`, the plugin thread.
- **`@outl/shared`** owns the pure Solid + TypeScript pieces (renderers, typed `invoke<T>()`, DTO interfaces); chrome stays per client.
- **`outl-shortcuts`** owns the (chord → action) catalog, so `Cmd+P` and `Ctrl+P` cannot drift.

Frontend is Solid + Tailwind.
The only hand-written platform bridge is the ObjC iCloud watcher, because `NSMetadataQuery` has no cross-platform equivalent.

[#98](https://github.com/avelino/outl/issues/98) is the policy applied to a public contract.
`outl://` is an external promise, so its parser landed **once** as `outl_actions::parse_deep_link` returning a `DeepLinkTarget`, and each client only maps that onto the `open_*` command it already had.
Registration is the sole per-client part, because it is genuinely OS transport — see [`docs/clients.md` → Deep links](../clients.md#deep-links-outl).

## Why not the alternatives

**uniffi + SwiftUI / Compose (the original #3 plan).**
Three implementations of one semantics, plus an FFI surface to re-cut on every new action: the engine would be shared, the part users touch would not.

**Electron on desktop.**
#2 set a sub-50 MB per-platform target, and Chromium per app misses it by an order of magnitude.

**A native desktop toolkit (egui, GTK, SwiftUI).**
Shares the Rust core but nothing with mobile's frontend, so `@outl/shared` gets no second consumer and the drift returns one layer up.

**A web app.**
Explicitly out of scope in #2, and local-first wants the filesystem, not a server.

## The opposite direction

**What got worse: the webview matrix is now ours.**
WKWebView, WebKitGTK and WebView2 do not render or handle input identically, so a CSS or focus regression can be platform-specific in a way a native toolkit would not allow.
Packaging is the same story — Linux desktop assets only existed after [#112](https://github.com/avelino/outl/issues/112) asked, because the release matrix became maintainer work.

**The mirrored case, stated explicitly.**
Converging the GUI clients opens a gap the TUI can fall into: it cannot consume `@outl/shared`, so anything landing in TypeScript is invisible in the terminal.
That is why the reminder schedule owner is Rust (`outl_actions::reminders::next_fire_at`), not a TS helper.
The gap runs the other way too, and does today: desktop Normal mode has only a selected block id, so char-level vim ops nudge instead of firing while the TUI has them (root `CLAUDE.md` → "What you're NOT building yet").

**`!Send` leaks into the plumbing.**
Boa's `Context` is single-threaded, so GUI clients run `PluginHost` on a dedicated thread behind an `mpsc` hop while the TUI drives it inline — one host, two plumbings, forever ([RFC 0025](0025-plugin-system.md)).

## How it cannot regress

1. **The rules.**
   Root `CLAUDE.md` carries both Tauri rows in the do-not-revisit table, plus the "Shared logic" and "Shared frontend" sections that say where a helper goes *before* its first use.
   The `outl-desktop`, `outl-mobile`, `outl-tauri-shared` and `outl-frontend-shared` `CLAUDE.md` files restate it at the point of edit.
2. **The tests.**
   The deep-link suite pins the one shared contract an external tool depends on.
   The framework decision has no mechanical guard, and this RFC is the record that the question was asked.

## Scope

**Not covered — Android**, which needs an `NSMetadataQuery` equivalent for the file transport; only iOS ships.

**Not covered — Universal Links** (`https://outl.app/…`), needing an Associated Domains entitlement plus a hosted `apple-app-site-association`; the custom scheme shipped first.

**Not covered — a character cursor in desktop Normal mode**, and `outl://block/<id>` / `outl://search?q=…`, both deferred on #98.
