# CLAUDE.md — outl-cli / cmd / import

Importers for external outliner formats.
Today: **Logseq**, **Roam**, and **Obsidian**.
Tomorrow: Bear, Notion, any markdown-shaped graph.

Every importer in this directory **follows the same pipeline**.
The pipeline exists because each step has an owner upstream — if a new importer reimplements any of them, the next contributor (or the same one six months later) will silently regress one of the coercions and the user is the one who hits the divergence.

## The canonical pipeline

```text
file text on disk
   │
   ▼
1. <source>::convert_file          ← source-specific quirks only
   (Logseq `((uid))` → `[[Page]]`,
    Logseq `#+title` / `#+date` strip,
    Roam tokens, `#tag`-vs-`#tag/`,
    Obsidian YAML frontmatter → `key:: value` properties,
    Obsidian wiki-link variant collapse, …)
   │
   ▼
2. common::write_page_md            ← persist to disk
   (slug-based filename, prepends `title:: <name>\n\n`
   for non-journals, journals carry no `title::`)
   │
   ▼
3. common::seed_sidecars            ← one pass at the end of `run`
   (acquires workspace + per-actor write locks,
    opens JsonlStorage, calls `outl_md::reconcile::reconcile_md`
    on every imported file so sidecars get stamped)
```

> **History note.** An earlier draft of this doc described two more
> pipeline steps — a `paste::normalize_external_syntax` call before
> the source-specific step, and a `normalize::normalize_outline` step
> after it — plus routing `seed_sidecars` through
> `outl_actions::ingest::ingest_md_file` and
> `create_missing_ref_pages`.
> None of those calls exist in the codebase today.
> `paste::normalize_external_syntax` exists in `outl-actions` and is
> the right owner for syntax-level coercion (line endings, indent,
> Logseq `id::` stripping, Roam tokens, long-form date rewriting) but
> no importer currently routes through it; an `ingest::*` module is
> planned but not landed.
> When those primitives arrive, this doc and the existing importers
> should be updated together in the same PR.

## Step ownership (do **not** reimplement these inside an importer)

| Step | Owner | What it does |
|---|---|---|
| write `.md` | [`common::write_page_md`](common.rs) / [`common::write_page_md_with_stem`](common.rs) | Slug-based filename, prepends `title:: <name>\n\n` for non-journals, journals carry no `title::`. `write_page_md_with_stem` takes an optional `stem_override` so importers that disambiguate collisions before writing can pass a unique stem; the user-visible `title::` still comes from `title`. **Callers must not include `title::` in `body`** — the helper always prepends it for non-journals. |
| seed sidecars | [`common::seed_sidecars`](common.rs) | Acquires `WorkspaceLock` + per-actor `ActorWriteLock`, opens `JsonlStorage`, calls `outl_md::reconcile::reconcile_md` on every imported file so the sidecar JSON is stamped. Idempotent. |
| parse journal dates | [`common::parse_journal_date`](common.rs) | Thin wrapper over `outl_actions::parse_flexible_date` — the one owner of human-typed date parsing. |
| resolve `((uid))` refs | [`common::resolve_uid_ref`](common.rs) / [`common::rewrite_uid_refs`](common.rs) (+ `UidIndex` / `ResolvedUid` / `truncate`) | Known UID → `[[Page Title]]` (artifact count); unknown → `((unresolved:uid))` (unresolved count). `rewrite_uid_refs` sweeps a whole text; Roam keeps its own scanner (it interleaves Roam-only tokens) and calls `resolve_uid_ref` per hit. |
| shallow `.md` walk | [`common::md_files_shallow`](common.rs) | Depth-1 listing of the `.md` files in a directory (Logseq `pages/` + `journals/`, sidecar seeding). |
| YAML frontmatter / wiki-link variants / image links | `outl_md::frontmatter` + `outl_md::wikilink` | Generic markdown parsing/rewriting lives in `outl-md` (see the [shared primitives catalog](../../../../../docs/shared-primitives.md) §6). Importers keep only source policy (e.g. Obsidian's dropped-keys list + date normalization in `obsidian.rs::parse_obsidian_frontmatter`). |

## What an importer module (`logseq.rs`, `roam.rs`, `obsidian.rs` + `obsidian/{stems,tests}.rs`, future) **does** own

Only the **source-specific transforms** that don't generalize:

- Filename decoding (Logseq `%2F` / `___`, Roam `_____` separators, Obsidian ISO date detection).
- Source-specific frontmatter / directives unique to the source (Logseq's `#+title` / `#+date`, Roam's JSON shape, Obsidian's YAML block).
- Source-specific reference resolution that needs a side-table built during a first pass (Logseq's `((uid))` → `[[Page Title]]` lookup, which needs the uid_index the importer builds).
- The walk over the source's directory layout (`pages/`, `journals/`, vault root, JSON backup) and dispatching to `convert_file`.

That's it.
Everything else routes through the shared primitives above.

## Adding a new importer (Bear, Notion, …)

1. Create `crates/outl-cli/src/cmd/import/<source>.rs`.
2. Register the source in [`run`](../import.rs) and route from the CLI subcommand (`crates/outl-cli/src/main.rs` `Import::format`).
3. Inside `convert_file`, after reading the file text:
   - Apply your source-specific transforms.
   - Call `super::write_page_md`.
4. `super::seed_sidecars` already runs at the end of `run`; you do **not** call it directly.
5. Tests: every importer ships an inline `#[cfg(test)] mod tests` (or `tests/<source>_smoke.rs`) that imports a tiny fixture graph and asserts:
   - page count > 0,
   - `[[ref]]` resolves (no `PAGE_NOT_FOUND` after import),
   - no `id::` / `((uuid))` artifacts in the resulting `.md`,
   - re-importing the same source is idempotent.

## Anti-patterns (don't do)

- ❌ Reimplement `\r\n` normalization, indent unit detection, Logseq `id::` stripping, or Roam token conversion inside an importer.
  Those belong in `outl_actions::paste::normalize_external_syntax` (which the importers should eventually route through).
- ❌ Mint a page id with `NodeId::new()` for an imported file.
  `reconcile_md` derives IDs deterministically; minting fresh ones breaks convergence with a peer importing the same source.
- ❌ Write `title::` into the body unconditionally.
  `write_page_md` already prepends it for non-journals; only emit additional `key:: value` properties (tags, date, source-specific keys) into the body.
- ❌ Skip `seed_sidecars` at the end of `run`.
  Without it the sidecar JSON never lands on disk and the user has to open the TUI once to stamp IDs.
- ❌ Drop user content silently when a frontmatter parser fails or a wiki-link variant is unrecognised — fall back to verbatim pass-through.

## When the shared primitives don't cover something

If your importer needs a coercion that **isn't** source-specific and is plausibly useful to clipboard paste too (any other format emitting the same construct), **add it to `outl_actions::paste::normalize` upstream** and call it from here, rather than forking.
The Shared primitives catalog in the root `CLAUDE.md` is where that decision is documented.
