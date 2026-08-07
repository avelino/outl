---
applyTo: "**/*.md"
---

## Markdown / documentation style

Flag when a `*.md` change introduces hard-wrapped prose (lines broken at ~70/80/100 chars mid-sentence).
Every prose `*.md` in this repo uses [semantic line breaks](https://sembr.org/): one sentence per line, breaking after `.`/`!`/`?` (and sometimes `:`), never at an arbitrary column.

What to flag:

- Prose paragraphs hard-wrapped at a column width.
- A sentence split across two or more lines for no reason.
- Tables rewritten to span multiple lines per row (must stay one row per line).

What to leave alone:

- Code fences, YAML frontmatter, ASCII tree diagrams — preserve exactly.
- Outline content (anything under `note-example/`, real workspace pages, fixtures) — this is data in the outl dialect, not prose docs.
- Single-line list items, headings, link references.

Scope: root `CLAUDE.md`, per-crate `CLAUDE.md`, `docs/*.md`, `README.md`, `CHANGELOG.md`, `CONTRIBUTING.md`, `SECURITY.md`, `.github/*.md`, `.claude/agents/*.md`, `.claude/commands/*.md`.
Root `CLAUDE.md` has the canonical rule under "Markdown / documentation style".
## One owner per fact — link, don't duplicate

Every user-facing fact lives in exactly one `docs/*.md`. `CLAUDE.md` files **link** to it instead of copying the table or chord list.

When reviewing a PR, flag duplication of these surfaces between `docs/*.md` and any `CLAUDE.md`:

| Fact | Canonical home |
|---|---|
| Every keyboard shortcut (TUI + desktop + mobile) | `docs/shortcuts.md` |
| `outl` CLI subcommands | `docs/cli.md` |
| TUI manual (modes, overlays) | `docs/tui.md` |
| Outl markdown dialect + sidecar | `docs/markdown-format.md` |
| CRDT algorithm + invariants | `docs/crdt.md` |
| Storage trait + JSONL backend | `docs/storage.md` |
| Sync model | `docs/sync.md` |
| MCP wiring + recipes | `docs/mcp.md` + `docs/mcp-recipes.md` |
| Config file | `docs/config.md` |
| Theming palette | `docs/theming.md` |
| Dev loop | `docs/development.md` |
| Contributing policy | `docs/contributing.md` |

What a `CLAUDE.md` *should* carry: invariants, architectural decisions you don't get to revisit, crate-specific contracts, the reasoning behind a choice — things a contributor needs *before* touching code, not user reference.

When you spot a PR copying a `docs/` table (or row of it) into a `CLAUDE.md`, **request a change**: replace with a link.
Reverse direction is fine — `docs/*.md` linking *into* a `CLAUDE.md` for architectural depth is welcome.

Canonical rule lives at root `CLAUDE.md` → "One owner per fact — link, don't duplicate".

---
