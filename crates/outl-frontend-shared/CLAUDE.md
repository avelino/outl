# CLAUDE.md — outl-frontend-shared

The shared TypeScript + Solid library every outl frontend client (`outl-mobile`, future `outl-desktop`) consumes.
**Lives here so two clients never reimplement the same thing.**

## Why it exists

Mobile and desktop are different *shells* on top of the same Rust backend.
Most of the UI is genuinely client-specific (touch gestures vs. mouse + keyboard, single-pane vs. 3-pane chrome, OS menus), so the **shells stay isolated** in `crates/outl-mobile/src/` and `crates/outl-desktop/src/`.

But a handful of pieces are dumb pure logic the two clients need *identically*:

- The renderer that turns `InlineToken[]` into JSX.
- The "does this look like a markdown outline?" heuristic that mirrors `outl_actions::paste::looks_like_outline`.
- The caret-aware `[[…]]` / `((…))` detector that mirrors `outl_tui::overlay::detect_trigger`.
- UTF-16 ↔ codepoint offset conversion (textarea quirk).
- DTO interfaces the backend serialises (`PageMeta`, `OutlineNode`, `BlockNode`, `Backlink`, `InlineToken`, …).
- Typed `invoke()` wrappers for the Tauri commands every client calls (navigation, mutations, paste, collapsed).

Keeping these in a separate library is the Rust "Reuse-first" policy ([root CLAUDE.md](../../CLAUDE.md#reuse-first-no-parallel-implementations)) applied to TS — one owner, every client wraps.

## Layout

```
crates/outl-frontend-shared/
├── package.json            # name "@outl/shared", peerDeps solid-js + @tauri-apps/api
├── tsconfig.json
├── vitest.config.ts
└── src/
    ├── index.ts            # barrel re-export
    ├── api/
    │   ├── types.ts        # PageMeta, OutlineNode, BlockNode, Backlink, InlineToken, …
    │   └── commands.ts     # invoke<T>() wrappers for shared Tauri commands
    ├── markdown/
    │   ├── MarkdownInline.tsx
    │   └── index.ts
    ├── paste/
    │   ├── index.ts        # looksLikeOutline, utf16OffsetToCharOffset
    │   └── paste.test.ts
    ├── autocomplete/
    │   ├── index.ts        # autoClosePair, autoPairBracket, autoDeletePair, insertPair, insertText, detectRefContext, applySuggestion
    │   └── autocomplete.test.ts
    ├── onboarding/
    │   ├── index.ts        # first-run copy (STORAGE_STEP, SYNC_STEP, FINISH_CTA) — plain data, no invoke
    │   └── onboarding.test.ts
    ├── journal/
    │   ├── index.ts        # parseJournalSlug, formatJournalSlug, journalSlugToDate, daysInMonth, MONTH_NAMES, DAY_LABELS*, mondayIndex, prev/nextMonth
    │   └── journal.test.ts
    ├── outline/
    │   ├── index.ts        # rawTextWithTodo, findBlock, flattenNodes, countDescendants + id walks (flattenVisible/All/Parents, next/previousVisibleId, visualRange*)
    │   └── outline.test.ts
    ├── plugins/
    │   └── transformer-registry.ts  # content-transformer registry + (blockId, body) result cache
    ├── toolbar/
    │   ├── actions.ts       # ToolbarAction catalog + TOOLBAR_META + DEFAULT_ORDER + PINNED_FIRST/LAST
    │   ├── mfu.ts           # most-frequently-used ordering (localStorage) — port of Swift ToolbarMFU
    │   ├── mfu.test.ts
    │   └── index.ts         # barrel (@outl/shared/toolbar)
    └── peers/
        ├── index.ts        # PairingQR, PeerList, ticketToSvg (barrel)
        ├── PairingQR.tsx    # ticket → scannable QR (owns its own encoding; no invoke)
        ├── PeerList.tsx     # pure list of paired devices (data + onRemove via props)
        ├── qr.ts            # ticketToSvg — pure ticket → SVG string (wraps `qrcode`)
        └── styles.css       # neutral baseline (@outl/shared/peers/styles)
```

## How clients consume it

```ts
// In a client component:
import type { Backlink, PageMeta } from "@outl/shared/api/types";
import { listPages, openRef } from "@outl/shared/api/commands";
import { MarkdownInline } from "@outl/shared/markdown";
import { looksLikeOutline } from "@outl/shared/paste";
import { autoClosePair, detectRefContext } from "@outl/shared/autocomplete";
```

Resolution happens through:

1. **Bun workspaces** (root `package.json` lists `crates/outl-frontend-shared` first).
   Bun dedupes `solid-js` and `@tauri-apps/api` across all clients — **critical for Solid**, because two copies of the framework in different `node_modules` directories silently break reactivity (signals diverge).
2. **`paths` in each client's `tsconfig.json`**:
   ```jsonc
   "paths": {
     "@outl/shared": ["../outl-frontend-shared/src/index.ts"],
     "@outl/shared/*": ["../outl-frontend-shared/src/*"]
   }
   ```
3. **`resolve.alias` in each client's `vite.config.ts` and `vitest.config.ts`** so Vite/HMR and Vitest resolve the same path the editor does.

## What enters the library

Decision rule (in order):

1. **Does the OTHER client also need it identically?**
   If yes, it goes here.
2. **Is it a pure function or stateless component?**
   If yes, it can go here.
3. **Is it the wire shape of something the Rust backend serialises?**
   If yes, it goes here as a type.
4. **Is the client shell tightly coupled to it (touch handlers, OS chrome, modes)?**
   Stays in the client.

When in doubt, ship in the client; promote later when the second client appears.
**Never** add something here speculatively — premature shared code becomes harder to evolve than two parallel copies.

### Today's surface

| Concept | Entry | Mirrors (Rust) |
|---|---|---|
| `<MarkdownInline />` (refs/tags fire `onRefClick`/`onTagClick`; external `[label](url)` links fire the optional `onLinkClick(href)` — when wired, the link is a keyboard-operable button (`role`/`tabindex`/Enter+Space); when omitted it's a plain inert `<span>`, no fake button) | `@outl/shared/markdown` | output of `outl_md::tokenize_owned` |
| `splitQuote`, `isQuote`, `QUOTE_PREFIX`, `stripQuoteFromTokens` | `@outl/shared/markdown` (re-exported) | `outl_actions::quote::{split_quote, is_quote, QUOTE_PREFIX}` |
| `<QuoteWrap />`, `isBlockQuoted` | `@outl/shared/markdown` | Wraps a quoted block's body in blockquote chrome (left border + faint tint), while each client keeps its outline bullet outside. Each client passes its theme tokens via `baseClass` + `chromeClass` props (Tailwind string literals for JIT discovery). |
| `looksLikeOutline` | `@outl/shared/paste` | `outl_actions::paste::looks_like_outline` |
| `hasMultipleParagraphs` | `@outl/shared/paste` | mirror of `split_paragraphs(...).length > 1` in `outl_actions::paste` — gate that decides whether plain text needs the structured backend path |
| `htmlToOutlMarkdown` | `@outl/shared/paste` | Rich-clipboard `text/html` → outl markdown via **Turndown**, configured for the outl dialect (`*italic*` not `_italic_`, `**bold**`, `- ` bullets collapsed to 2-space nesting, `~~strike~~`, inline `<img alt>` → its alt text so Slack `:emoji:` survives). No Rust mirror — HTML only reaches the GUI webview clients; the resulting markdown then rides the same `paste_markdown_at` backend path as any paste |
| `choosePasteRoute(html, plain)` → `PasteRoute` | `@outl/shared/paste` | The one owner of the paste-with-formatting routing decision (`rich` = HTML converted to markdown; `structured` = plain outline / multi-paragraph; `native` = trivial, let the browser splice). Desktop `handlePaste` and mobile `onPaste` both call it, so the gate can't drift between clients — it used to be duplicated inline in each handler |
| `utf16OffsetToCharOffset` | `@outl/shared/paste` | (runtime gap — UTF-16 ↔ codepoint, no Rust mirror) |
| `detectRefContext`, `autoClose/DeletePair`, `insertPair/Text`, `applySuggestion` | `@outl/shared/autocomplete` | `outl_tui::actions::overlay::detect_trigger` |
| `detectSlashContext` / `applySlashContext` (+ `SlashContext`) — block-initial `/command` trigger + token removal on accept, powering the desktop's inline slash menu (Notion-style); mirrors the TUI `/` slash overlay but inline in a block | `@outl/shared/autocomplete` | `outl_tui::actions::overlay::slash_candidates` (same command universe, different surface) |
| `autoPairBracket` (single `(`/`[`/`{` auto-pair + closer step-over; `autoDeletePair` also collapses empty `()`/`[]`/`{}`) | `@outl/shared/autocomplete` | `outl_tui::input::insert` (`insert_pair`) + `EditBuffer::delete_pair_back` |
| `<ParseWarningsBanner />` + `@outl/shared/warnings/styles` CSS | `@outl/shared/warnings` | TUI `view::warnings_banner` (visual parity, neutral chrome). Clients **must** `@import "@outl/shared/warnings/styles"` from their root stylesheet — without it the banner renders with unstyled neutral classes and looks invisible against the page. |
| `ParseWarning` / `ParseWarningKind` (DTO of `PageView.warnings`) | `@outl/shared/api/types` | `outl_md::ParseWarning` / `ParseWarningKind` |
| `<PairingQR ticket=… />` (renders a pairing ticket as a scannable QR; owns its own async encoding via `ticketToSvg`, **no invoke inside** — host passes the ticket from `peerPairHost()`) + `<PeerList peers=… statusByNodeId? onRemove? />` (pure list of paired devices with online/offline/unknown status dot + optional remove button; **all data + callbacks via props, no invoke**) + `ticketToSvg` (pure ticket → SVG string, wraps the `qrcode` npm dep) + `peersOnline(statuses)` (pure: `true` when any peer has `online === true`; accepts the `PeerStatusDto[]` from `peerStatus()` or the desktop's `Map<node_id, …>`; both clients derive the sync dot from it identically) | `@outl/shared/peers` (+ `@outl/shared/peers/styles` CSS baseline) | the `outl_peer_*` commands in each client's `commands/peers.rs` (wrappers in `@outl/shared/api/commands`; `outl_sync_iroh::PeerEntry`/`PeerStatus`) |
| `PeerDto` (`node_id`, `alias`, `added_at`) / `PeerStatusDto` (`node_id`, `alias`, `online`, `rtt_ms`) | `@outl/shared/api/types` | Rust `PeerDto` / `PeerStatusDto` in both clients' `commands/peers.rs` |
| First-run onboarding copy (`STORAGE_STEP`, `SYNC_STEP`, `FINISH_CTA`) — plain `as const` data, **no invoke / no JSX**; the only piece of onboarding that's identical between clients (the honest, no-account "where do your notes live" + "sync is peer-to-peer, one device is fine" wording). The chrome is client-specific (mobile: full-screen bottom-sheet-styled `Onboarding.tsx` + haptics; desktop: `Onboarding.tsx` wrapping `<WorkspacePicker />` + `<SyncPanel />`). | `@outl/shared/onboarding` | no Rust mirror — UI copy. The storage facts it tracks live in `outl-mobile/src-tauri/workspace_picker.rs` / `outl-desktop` workspace commands |
| DTOs (`PageMeta`, `OutlineNode`, `BlockNode`, `Backlink`, `InlineToken`, `PageView` — incl. `backlinks_order: BacklinksOrder`, `CreateBlockReply`, `WorkspaceSummary`, …) | `@outl/shared/api/types` | the corresponding `serde`-serialised Rust structs |
| `BacklinksOrder` (`"newest"` \| `"oldest"`) | `@outl/shared/api/types` | `outl_config::BacklinksOrder` |
| Plugin DTOs (`PluginCommand`, `PluginToolbarButton`, `PluginRunReply`, `PluginSyncHooksReply`, `PluginTransformer`, `PluginTransformResult`) + wrappers (`pluginList`, `pluginRun`, `pluginSyncHooks`, `pluginToolbar`, `pluginTransformers`, `pluginTransform`) — both clients register the identical `plugin_*` commands (thin shims over `PluginService`), so the wire shapes + wrappers live here once. The desktop-only chord surface (`PluginKeybinding` / `pluginKeybindings`) stays in `outl-desktop/src/lib/api.ts` (mobile has no keybindings) | `@outl/shared/api/types` + `@outl/shared/api/commands` | each client's `src-tauri/src/commands/plugin.rs` DTOs |
| Content-transformer registry + cache (`loadTransformers`, `transformerFor(lang)` → `PluginTransformer \| null`, `runTransform(blockId, transformer, body)` cached by `(blockId, body)`) — the `lang → transformer` Solid signal both clients load once per workspace open, plus the per-fence result cache (a failed transform drops its key so a later render retries; a reload clears the cache since results may now differ) | `@outl/shared/plugins/transformer-registry` | no Rust mirror — client lifecycle glue over `plugin_transformers` / `plugin_transform`, identical on both clients |
| Mobile keyboard-toolbar **logic** — `ToolbarAction` catalog + `TOOLBAR_META` (label + symbol/text style) + `DEFAULT_ORDER` + `PINNED_FIRST`/`PINNED_LAST`, plus the MFU ordering (`orderedMiddleActions(counts)` pure; `recordToStore`/`readCountsFromStore`/`orderedMiddleFromStore` over `localStorage` under `outl.toolbar.mfu.v1`). The action string ids are the wire contract the iOS native bar ships via `window.__outlToolbar(action)`; **kept byte-identical to `OutlKit/Toolbar/ToolbarAction.swift`** (rename on both sides in one commit). Rendering (icons, capsule, keyboard docking) is client chrome, NOT here | `@outl/shared/toolbar` | port of `OutlKit/Toolbar/{ToolbarAction,ToolbarMFU}.swift` (the iOS native bar keeps its Swift copy because it renders before the webview) |
| `rawTextWithTodo(block)` — wire-format text with the TODO/DONE prefix reattached, what every client's editor shows so the user can erase / type the prefix | `@outl/shared/outline` | mirror of `outl_actions::split_todo` in reverse (keep in sync with `TODO_PREFIX` / `DONE_PREFIX`) |
| Outline walks — `findBlock`, `flattenNodes` (DFS preorder, returns **`BlockNode`s**), `countDescendants`, plus the id-returning selection walks: `flattenVisible` (skips collapsed subtrees), `flattenAll` (every id, `zR`), `flattenParents` (`zM` fold-all targets — mirror of outl-tui's `collect_collapse_candidates`), `nextVisibleId` / `previousVisibleId` (vim `j`/`k`; previous returns `null` at the top, never the current block), `visualRangeIds` / `visualRangeSet` (memoise the Set at the parent — per-row predicates are O(N²)) / `isInVisualRange` | `@outl/shared/outline` | `outl-tui`'s outline walks (`collect_collapse_candidates` for `flattenParents`); pure functions over `BlockNode[]`, no invoke |
| `focusSubtree(blocks, blockId)` → `FocusView { root, breadcrumb } \| null` (+ `FocusCrumb { id, text }`) — zoom/focus: the subtree to render as the new root plus the ancestor breadcrumb (page-top first, parent last). `null` = stale zoom target (block deleted/moved) → caller drops the zoom. Zoom is **local view state**, never a Tauri round-trip (the client already holds the whole `outline`) | `@outl/shared/outline` | no Rust mirror — zoom is frontend-only view state |
| Journal slug + calendar math — `parseJournalSlug` / `formatJournalSlug` / `journalSlugToDate` (local-time parse; `new Date("YYYY-MM-DD")` is midnight UTC and renders the previous day in negative-offset timezones), `daysInMonth`, `MONTH_NAMES`, `DAY_LABELS` (Sunday-first, mobile sheet) / `DAY_LABELS_MONDAY_FIRST` (TUI-style, desktop sidebar), `mondayIndex`, `prevMonth` / `nextMonth` (pure year-rollover). The calendar **chrome** stays per-client — only the math/parsing is shared. `monthIndex` is 0-based everywhere (JS `Date` convention) | `@outl/shared/journal` | the `YYYY-MM-DD` journal slug contract (`outl_actions` date slugs); no Rust mirror for the grid math |
| `refReplacement(page, opts?)` — the page name spliced into `[[…]]` when a ref suggestion is accepted: journals insert their ISO slug, everything else (and every `@` mention) inserts the **title** (bug #88 was the chip strip writing the slug) | `@outl/shared/autocomplete` | no Rust mirror — pairs with `applySuggestion` |
| `invoke<T>()` wrappers (navigation: `listPages`, `searchPages`, `searchPersons`, `searchEmojis` → `EmojiHit[]` (powers the `:shortcode:` autocomplete in every client; backed by `outl_md::emoji::search` so TUI / mobile / desktop rank identically), `searchBlocks` → `BlockHit[]` (powers the `((…))` block-ref autocomplete; backed by `outl_md::WorkspaceIndex::search_block_text`; caller inserts each hit's `handle` wrapped in `((…))`, never the display `text`), `openTodayJournal`, `openJournalFor`, `openPageBySlug`, `openRef`, `previousDay`, `nextDay`, `todaySlug`, `dateTitle`, `resolveRef`, `workspaceStats`; mutation: `createBlock` → `CreateBlockReply` (returns `{ view, new_id }` so the client puts the new block straight into edit mode without diffing the outline), `editBlock`, `toggleTodo`, `deleteBlock`, `indentBlock`, `outdentBlock`, `moveBlockUp`, `moveBlockDown`, `reloadWorkspace`, `pasteMarkdown`, `pastePlain(pageId, blockId, caret, text)` (invokes `paste_plain_at` — paste without formatting: raw text as a single block, no normalisation or paragraph splitting), `copyMarkdown` (serialises a block selection + subtrees as clean outl markdown for the OS clipboard — the copy-out inverse of `pasteMarkdown`), `setBlockCollapsed`, `deletePage(slug) → Promise<PageView>` (delegates to the shared `delete_page` command; returns today's-journal `PageView` so every caller navigates away from the deleted slug identically — desktop hover `×`, desktop `DeletePage` action handler, and mobile long-press all call this one wrapper), `setBacklinksOrder(order, slug) → Promise<PageView>` (delegates to `set_backlinks_order`; persists `[display] backlinks_order` and returns the re-sorted view — the desktop `InlineBacklinks` and mobile `BacklinksSection` header buttons both call this one wrapper, issue #142); execution: `runCodeBlock` → `RunCodeBlockReply` (refreshed `PageView` + stdout/stderr/exit so the caller swaps the outline in one round-trip); peers/pairing: `peerList` → `PeerDto[]`, `peerRemove(id)` → `bool` (prefix match), `peerStatus` → `PeerStatusDto[]` (async iroh probe), `peerPairHost(alias?)` → `string` (ticket; completion surfaces via the backend `peer-paired` event — desktop's Rust command is being aligned to the mobile ticket-return shape), `peerPairJoin(ticket, alias?)` → `PeerDto`, `syncNow()` → `void` (force an immediate iroh sync pass against every peer — pull-to-refresh / Refresh; no-op when iroh isn't wired); external links: `openExternalUrl(href)` (opens `http(s)`/`mailto` in the system browser via `tauri-plugin-opener`; rejects other schemes — the host must register the opener plugin + grant `opener:allow-open-url`)) | `@outl/shared/api/commands` | the matching Tauri command in each client's `src-tauri/src/lib.rs` (`openExternalUrl` wraps the `@tauri-apps/plugin-opener` JS API, not a custom command) |

## What does NOT enter the library

- **Chrome.** `<Sidebar />`, `<Picker />`, `<BacklinksPanel />`, `<BlockRow />`, app shells — they diverge between mobile (single-pane, touch) and desktop (3-pane, mouse + vim mode).
- **Stateful stores.**
  Each client's Solid `createStore()` carries client-specific shape (mobile has swipe state, desktop has panel collapse state).
- **Keybindings.**
  Cmd-based on desktop, gesture-based on mobile.
- **Client-specific Tauri commands.**
  `pick_workspace_dir` belongs to `outl-desktop`; the iCloud peer-files watcher and gestures glue belong to `outl-mobile`.
  Wrap those in the client's own `lib/api.ts`.
  (`run_code_block` *used* to live here too; mobile picked up the same command in v0.6.x — long-press → "Run code" — so the wrapper is now in `@outl/shared/api/commands`.
  Desktop's `lib/api.ts` re-exports it for backward-compatible imports.)
- **Tailwind config.**
  Each client has its own theme; could be shared later if the palettes converge.
  Low priority.

## Theming note

The `<MarkdownInline />` component currently uses iOS-themed CSS custom properties (`--color-ios-accent`, `--color-iosd-*`).
The mobile client defines them in its stylesheet; **the desktop client must mirror the same token names** until we refactor to neutral `--color-outl-*` tokens.
If desktop's palette diverges first, introduce the abstraction in this library and have each client map its theme to the neutral tokens.

## Adding a new piece

1. **Search first.**
   Before writing a helper in any client `lib/`, `rg` here and in `outl-mobile/src/lib/` for a comparable name or symbol.
2. **If the other client has it locally**, promote in the same PR (move to `src/<area>/`, update both clients' imports, delete the local copy).
3. **If it's a brand-new concept that only one client needs today**, write it in the client.
   When the second client wants it, promote in the move PR.
4. **Update the table above** when promoting.

## Running tests

```bash
bun install                        # at repo root, hoists deps via workspaces
bun --filter @outl/shared test     # just this library
bun --filter outl-mobile test      # mobile (consumes this library)
```

## When you're done editing

1. `bunx tsc --noEmit` from this crate (type check)
2. `bun --filter @outl/shared test` (Vitest)
3. `bun --filter outl-mobile test` (paridade — mobile consume idêntico)
4. If you changed the public surface (a new file in `src/`, a new export in `package.json` `exports`), update:
   - This file's "Today's surface" table
   - Each consuming client's `CLAUDE.md` if the contract is new
   - Root `CLAUDE.md` "Shared primitives catalog" (frontend section)
