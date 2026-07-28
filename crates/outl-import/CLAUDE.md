# CLAUDE.md — outl-import

Adapter-based graph importers.
Today: **Roam** (JSON backup), **Logseq** (graph dir), **Obsidian** (vault) — every source the CLI accepts routes through this crate; the legacy string pipeline in `outl-cli/src/cmd/import/` is gone.
`outl import auto <src> <dst>` picks the adapter via each adapter's `detect()`.

## Architecture: three stages, one owner each

```text
source on disk (JSON backup / graph dir / vault)
   │
   ▼
[1] SourceAdapter::parse           ← ALL dialect knowledge lives here
   │    produces the typed IR (ir.rs) — no source syntax escapes
   ▼
[2] emit::render                   ← single owner of outl output syntax
   │    IR → .md; refs/embeds become inert placeholders
   │    ((outl-import:<uid>)) / !((outl-import:<uid>))
   ▼
[3] emit::resolve                  ← 2-pass handle resolution
        reconcile_md stamps sidecars → uid → NodeId → ref_handle
        rewrite goes through the op log (edit_text + re-render)
        collapsed flags land as Op::SetCollapsed
```

## Why the placeholder + 2-pass dance

`((blk-XXXXXX))` handles derive from `NodeId`s, and `NodeId`s only exist after `reconcile_md` stamps the sidecar.
So pass A writes markdown with placeholders outl's tokenizer treats as plain text, reconciles everything, then pass B maps each source UID to its handle through the sidecar's depth-first block list (the same order the renderer counted).

**Pass B mutates through ops, never by rewriting `.md`.**
Editing the file and re-reconciling would push changed blocks through the 3-level matcher, which can re-identify a block (level 3 → fresh id) and dangle every handle already written elsewhere.
`edit_text` + `apply_page_md_with_sidecar` keeps ids stable by construction (root invariant #1: op log is source of truth, files are projections).

Two guards protect the index mapping:

- sidecar block count must equal the renderer's depth-first count, or the whole page degrades to `[[Title]]` links (counted + warned, never mis-wired);
- the target block's `content_hash` must match the rendered text before any edit lands.

## The IR is deliberately minimal

A construct only gets an `Inline` variant when the emitter must **transform or resolve** it (`BlockRef`, `Embed`, `Component`, `CodeSpan`).
Everything already valid in outl (`[[page]]`, `#tag`, `**bold**`, links) travels as `Inline::Text` verbatim — outl's own parser tokenizes it after emission.
Don't add variants for passthrough syntax.

Two page-body shapes (`PageBody`):

- `Outline(Vec<ImportBlock>)` — structured sources (Roam, Logseq); every block typed, UIDs DFS-mapped to sidecar entries.
- `Raw(String)` — free-markdown sources (Obsidian); text-level rewrites only, outl's permissive parser owns the structure downstream.
  Raw pages carry no UIDs, so the resolve pass never touches them.

Adapters with their own collision policy (Obsidian's path-derived suffixes in `adapters/obsidian/stems.rs`) pin the on-disk stem via `ImportPage::stem_override`; everyone else lets the emitter slugify the title.

## The report is the fidelity contract

Everything translated, degraded, or dropped is counted in `ImportReport` — per-feature, serializable, with located warnings.
`outl import roam <backup> <dst> --dry-run --json` answers "what would I lose" with numbers before any migration.
**Silent loss is a bug**: if you add a lossy path, count it.

## Files

```
src/
├── lib.rs             # run_import / dry_run, ImportDest, ImportOptions
├── adapter.rs         # SourceAdapter trait + ImportError
├── ir.rs              # ImportGraph / ImportPage / PageBody / ImportBlock / Inline
├── report.rs          # ImportReport (per-feature counts, warnings)
├── emit/
│   ├── mod.rs         # pipeline orchestration + dry-run simulation
│   ├── render.rs      # IR → markdown, placeholder emission, DFS bookkeeping
│   └── resolve.rs     # pass B: uid → handle, edit_text, SetCollapsed
└── adapters/
    ├── scan.rs        # shared low-level scanners (balanced, is_uid, alias_link)
    ├── roam/          # mod.rs (JSON → IR) + inline.rs (dialect scanner) + tests.rs
    ├── logseq/        # mod.rs (outline parser) + inline.rs + tests.rs
    └── obsidian/      # mod.rs (frontmatter/wikilink policy) + stems.rs (collisions)

tests/
├── common/mod.rs      # shared harness: real Workspace over JsonlStorage in a tempdir
├── roam_e2e.rs
├── logseq_e2e.rs
└── obsidian_e2e.rs    # ports every scenario the legacy pipeline covered
```

Dialect boundaries worth remembering:

- Roam `__x__` is *italic* → translated to `*x*`; Logseq is CommonMark, so `__x__` stays bold — no formatting swaps there.
- Roam components are colon-shaped (`{{[[embed]]: ((uid))}}`); Logseq's are space-shaped (`{{embed ((uid))}}`).
- Roam's flat `{and:}` queries translate to ` ```query ` fences; Logseq's sexp queries stay verbatim components.

## Adding a new adapter (Bear, Notion, …)

1. `src/adapters/<source>/mod.rs` implementing `SourceAdapter` — parse only, never write.
2. All inline dialect translation in the adapter (see `roam/inline.rs` / `logseq/inline.rs` for the scanner shape); reuse `adapters/scan.rs` primitives.
3. Line-level source constructs map to IR fields, not text: task states → `TaskState`, `collapsed:: true` → `collapsed`, `key:: value` under a bullet → `props`, `SCHEDULED:`/`DEADLINE:` → `[[date]]` links in text (issue #63 model).
4. Outliner source → `PageBody::Outline`; free-markdown source → `PageBody::Raw` + text-level rewrites (see the Obsidian adapter).
5. Golden tests: one case per fidelity-matrix row, plus an e2e in `tests/` on the shared harness.
6. Route the CLI source in `outl-cli/src/cmd/import.rs::run` and wire `detect()` into `auto_detect` in the same PR.

## Anti-patterns

- ❌ Emitting `((blk-…))` directly from render — handles don't exist yet; that's resolve's job.
- ❌ Rewriting `.md` files in pass B instead of going through `edit_text`.
- ❌ Minting `NodeId`s for imported blocks — `reconcile_md` derives them; fresh ids break convergence.
- ❌ Dropping content silently when a construct doesn't parse — `BlockContent::Verbatim` + a report count is the floor.
- ❌ Writing dialect knowledge into `emit/` — if the emitter needs to know what `__` means, the adapter failed to translate.

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-import --all-targets -- -D warnings`
3. `cargo test -p outl-import`
4. Smoke a real backup with `--dry-run --json` and read the numbers.
