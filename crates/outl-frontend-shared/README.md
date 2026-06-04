# @outl/shared

Shared TypeScript + Solid library consumed by the outl frontend clients (`outl-mobile`, `outl-desktop`).

Hosts everything that 2+ clients would otherwise implement identically:

- **`@outl/shared/api/types`** — DTO interfaces returned by the Rust backend (`PageMeta`, `OutlineNode`, `BlockNode`, `Backlink`, `InlineToken` union, …).
- **`@outl/shared/api/commands`** — typed `invoke()` wrappers for the Tauri commands every client uses (navigation, mutations, paste).
- **`@outl/shared/markdown`** — `<MarkdownInline />` Solid component that renders an `InlineToken[]` array produced by `outl_md::tokenize_owned`.
- **`@outl/shared/paste`** — `looksLikeOutline`, `utf16OffsetToCharOffset` (pure functions; mirrors of Rust `outl_actions::paste::looks_like_outline`).
- **`@outl/shared/autocomplete`** — caret-aware helpers for `[[…]]` / `((…))` (mirror of `outl_tui::actions::overlay::detect_trigger`).

## Rule of thumb

Before adding a helper in `outl-mobile/src/lib/` or `outl-desktop/src/lib/`:

1. Look here first.
2. If the other client has an equivalent, promote it here in the same PR — never let two parallel TS implementations live.
3. Only keep code local when it's genuinely client-specific (touch gestures, OS chrome, etc).

See the workspace root `CLAUDE.md` "Reuse-first" policy for the equivalent rule on the Rust side.

## Running tests

```bash
bun install              # from the repo root, hoists deps via workspaces
bun --filter @outl/shared test
```
