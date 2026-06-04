# CLAUDE.md — outl-cli / cmd / import

Importers for external outliner formats. Today: **Logseq** and **Roam**.
Tomorrow: Bear, Obsidian, Notion, any markdown-shaped graph.

Every importer in this directory **must follow the same pipeline**.
The pipeline exists because each step has an owner upstream — if a new
importer reimplements any of them, the next contributor (or the same
one six months later) will silently regress one of the coercions and
the user is the one who hits the divergence.

## The canonical pipeline

```text
file text on disk
   │
   ▼
1. paste::normalize_external_syntax       ← syntax-level coercion
   (line endings, indent unit, tokens,
    long-form dates, id:: stripping)
   │
   ▼
2. <source>::convert_file                 ← source-specific quirks
   (Logseq `((uid))` → `[[Page]]`,
    Logseq `#+title`/`#+date` strip,
    Roam-specific tokens, etc.)
   │
   ▼
3. import::normalize::normalize_outline   ← outline-level coercion
   (## heading → bullet, multi-paragraph
    block merge, level clamping, fenced
    code dedent + re-indent)
   │
   ▼
4. write_page_md                          ← persist to disk
   │
   ▼
5. seed_sidecars
   │
   ├──► outl_actions::ingest_md_file      ← creates page node + reconciles blocks
   │
   └──► outl_actions::create_missing_ref_pages ← Logseq "implicit pages"
```

The boundary between steps 1 and 3 is **deliberate**: syntax-level
coercion is workspace-agnostic and lives in `outl-actions::paste`
(it's also what powers the clipboard-paste path in the TUI and the
mobile app, so every fix lands in one place). Outline-level
restructuring is import-specific (you don't want clipboard paste to
silently rewrite block structure) and lives here.

## Step ownership (do **not** reimplement these inside an importer)

| Step | Owner | What it does |
|---|---|---|
| 1. syntax coercion | [`outl_actions::paste::normalize_external_syntax`](../../../../outl-actions/src/paste/normalize.rs) | `\r\n` → `\n`, indent 4→2, `id:: <ULID>` strip with Crockford validation, `{{[[TODO]]}}` → `TODO`, `{{embed: ((blk-…))}}` → `!((blk-…))`, `[[June 2nd, 2026]]` → `[[2026-06-02]]`, strip unknown `{{…}}` / `^^…^^` |
| 3. outline coercion | [`normalize::normalize_outline`](./normalize.rs) | `## heading` → `- ## heading`, merge multi-paragraph block bodies (drop blank lines inside a block), clamp level skips, dedent and re-indent fenced code blocks |
| 4. write `.md` | [`super::write_page_md`](../import.rs) | Skip prepending `title::` when the body already opens with a page property (regression: a duplicate `title::` orphans every block below it) |
| 5a. ingest | [`outl_actions::ingest::ingest_md_file`](../../../../outl-actions/src/ingest.rs) | Create the page node under root with `page-slug` / `page-kind` using a deterministic `page_id_from_slug(slug)`, then reconcile blocks under that node. **Never** call `outl_md::reconcile::reconcile_md` directly for fresh files — it doesn't create the page node, and the blocks hang off a phantom id that `page list` never sees (that was issue #43). |
| 5b. implicit pages | [`outl_actions::ingest::create_missing_ref_pages`](../../../../outl-actions/src/ingest.rs) | Walk every block, collect `[[ref]]` targets, create a stub page (title = ref text verbatim, slug = `slugify(ref)`) for any target with no file. Date-shaped refs become journals. Idempotent. |

## What an importer file (`logseq.rs`, `roam.rs`, future) **does** own

Only the **source-specific transforms** that don't generalize:

- Filename decoding (Logseq `%2F` / `___`, Roam `_____` separators).
- Frontmatter-style directives unique to the source (Logseq's `#+title`,
  `#+date`).
- Source-specific reference resolution that needs a side-table built
  during a first pass (Logseq's `((uid))` → `[[Page Title]]` lookup,
  which needs the uid_index that the importer builds).
- The walk over the source's directory layout (`pages/`, `journals/`)
  and dispatching to `convert_file`.

That's it. Everything else routes through the shared primitives above.

## Adding a new importer (Bear, Obsidian, Notion, …)

1. Create `crates/outl-cli/src/cmd/import/<source>.rs`.
2. Register the source in [`run`](../import.rs) and route from the
   CLI subcommand.
3. Inside `convert_file`, after reading the file text:
   - First call `outl_actions::paste::normalize_external_syntax`.
   - Then apply your source-specific transforms.
   - Then call `super::normalize::normalize_outline`.
   - Then `super::write_page_md`.
4. `super::seed_sidecars` already runs at the end of `run` and feeds
   every imported file through `ingest_md_file` +
   `create_missing_ref_pages`. You do **not** call those directly.
5. Tests: every importer ships a `tests/<source>_smoke.rs` (or inline
   `#[cfg(test)]` mod) that imports a tiny fixture graph and asserts:
   - page count > 0,
   - `[[ref]]` resolves (no `PAGE_NOT_FOUND` after import),
   - no `id::` / `((uuid))` artifacts in the resulting `.md`,
   - re-importing the same source is idempotent.

## Anti-patterns (don't do)

- ❌ Reimplement `\r\n` normalization, `id::` stripping, long-form date
  rewriting, or Roam/GitHub token conversion. **Step 1 owns all of
  that.** Adding it again means the next fix has to land in two
  places.
- ❌ Call `outl_md::reconcile::reconcile_md` for a fresh file from an
  importer. Use `ingest_md_file` so the page node materializes.
- ❌ Mint a page id with `NodeId::new()` for an imported file. Use
  `page_id_from_slug(slug)` so two peers importing the same graph
  converge on the same node.
- ❌ Write `title::` into the body unconditionally. Use
  `body_starts_with_page_property` to detect when the body already
  carries page properties (re-importing an outl workspace, or a
  Logseq page that used `title::`).
- ❌ Skip `create_missing_ref_pages`. Without it, `[[Acme]]` and
  `[[@Jane Doe]]` (Logseq's "implicit pages") never materialize and
  backlinks return `PAGE_NOT_FOUND`.

## When the shared primitives don't cover something

If your importer needs a coercion that isn't in `paste::normalize_external_syntax`
**and** it's plausibly useful to clipboard paste too (any other format
emitting the same construct), **add it to `paste::normalize` upstream**
and call it from here, rather than forking. The Shared primitives
catalog in the root `CLAUDE.md` is where that decision is documented.
