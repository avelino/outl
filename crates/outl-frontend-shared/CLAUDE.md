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
    └── autocomplete/
        ├── index.ts        # autoClosePair, autoDeletePair, insertPair, insertText, detectRefContext, applySuggestion
        └── autocomplete.test.ts
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

1. **Bun workspaces** (root `package.json` lists `crates/outl-frontend-shared` first). Bun dedupes `solid-js` and `@tauri-apps/api` across all clients — **critical for Solid**, because two copies of the framework in different `node_modules` directories silently break reactivity (signals diverge).
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

1. **Does the OTHER client also need it identically?** If yes, it goes here.
2. **Is it a pure function or stateless component?** If yes, it can go here.
3. **Is it the wire shape of something the Rust backend serialises?** If yes, it goes here as a type.
4. **Is the client shell tightly coupled to it (touch handlers, OS chrome, modes)?** Stays in the client.

When in doubt, ship in the client; promote later when the second client appears.
**Never** add something here speculatively — premature shared code becomes harder to evolve than two parallel copies.

### Today's surface (Phase −1 → 0)

| Concept | Entry | Mirrors (Rust) |
|---|---|---|
| `<MarkdownInline />` | `@outl/shared/markdown` | output of `outl_md::tokenize_owned` |
| `looksLikeOutline` | `@outl/shared/paste` | `outl_actions::paste::looks_like_outline` |
| `utf16OffsetToCharOffset` | `@outl/shared/paste` | (runtime gap — UTF-16 ↔ codepoint, no Rust mirror) |
| `detectRefContext`, `autoClose/DeletePair`, `insertPair/Text`, `applySuggestion` | `@outl/shared/autocomplete` | `outl_tui::actions::overlay::detect_trigger` |
| DTOs (`PageMeta`, `OutlineNode`, `BlockNode`, `Backlink`, `InlineToken`, `PageView`, `CreateBlockReply`, `WorkspaceSummary`, …) | `@outl/shared/api/types` | the corresponding `serde`-serialised Rust structs |
| `invoke<T>()` wrappers (navigation: `listPages`, `searchPages`, `openTodayJournal`, `openJournalFor`, `openPageBySlug`, `openRef`, `previousDay`, `nextDay`, `todaySlug`, `dateTitle`, `resolveRef`, `workspaceStats`; mutation: `createBlock` → `CreateBlockReply` (returns `{ view, new_id }` so the client puts the new block straight into edit mode without diffing the outline), `editBlock`, `toggleTodo`, `deleteBlock`, `indentBlock`, `outdentBlock`, `moveBlockUp`, `moveBlockDown`, `reloadWorkspace`, `pasteMarkdown`, `setBlockCollapsed`) | `@outl/shared/api/commands` | the matching Tauri command in each client's `src-tauri/src/lib.rs` |

## What does NOT enter the library

- **Chrome.** `<Sidebar />`, `<Picker />`, `<BacklinksPanel />`, `<BlockRow />`, app shells — they diverge between mobile (single-pane, touch) and desktop (3-pane, mouse + vim mode).
- **Stateful stores.** Each client's Solid `createStore()` carries client-specific shape (mobile has swipe state, desktop has panel collapse state).
- **Keybindings.** Cmd-based on desktop, gesture-based on mobile.
- **Client-specific Tauri commands.** `pick_workspace_dir` and `run_code_block` belong to `outl-desktop`; the iCloud peer-files watcher and gestures glue belong to `outl-mobile`. Wrap those in the client's own `lib/api.ts`.
- **Tailwind config.** Each client has its own theme; could be shared later if the palettes converge. Low priority.

## Theming note

The `<MarkdownInline />` component currently uses iOS-themed CSS custom properties (`--color-ios-accent`, `--color-iosd-*`).
The mobile client defines them in its stylesheet; **the desktop client must mirror the same token names** until we refactor to neutral `--color-outl-*` tokens.
If desktop's palette diverges first, introduce the abstraction in this library and have each client map its theme to the neutral tokens.

## Adding a new piece

1. **Search first.** Before writing a helper in any client `lib/`, `rg` here and in `outl-mobile/src/lib/` for a comparable name or symbol.
2. **If the other client has it locally**, promote in the same PR (move to `src/<area>/`, update both clients' imports, delete the local copy).
3. **If it's a brand-new concept that only one client needs today**, write it in the client. When the second client wants it, promote in the move PR.
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
