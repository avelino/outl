# CLAUDE.md — outl-md

The boundary between **what the user sees** (clean markdown) and **what the core processes** (op log with stable IDs).

If this crate misroutes a block during matching, the user perceives "outl deleted my work" — even if the op log still has it.
Treat matching with the same paranoia as the CRDT.

> The **canonical reuse index** for the whole workspace is the ["Shared primitives catalog" in the root `CLAUDE.md`](../../CLAUDE.md#shared-primitives-catalog) (mirrored at [`.github/copilot-instructions.md`](../../.github/copilot-instructions.md) §5.1).
> The detailed list below describes this crate's responsibilities; the root catalog is the "intent → use this" cross-crate index you should grep first when adding any helper.

## What this crate owns

- Parse `.md` (clean, no IDs) → outline AST.
  Parser is **permissive**:
  lines that don't match the outl dialect (e.g. a leading `# heading`, a stray paragraph, an HTML snippet at depth 0) are preserved verbatim as regular blocks and recorded in `ParsedPage.warnings: Vec<ParseWarning>` (kind `UnrecognizedBlockMarker`).
  Nothing is silently dropped; surfaces surface the warning list so the user can clean the file at their pace.
  Multi-line bodies (including `> ` blockquote continuation lines, `TODO ` / `DONE ` continuations, and free-text continuations) land verbatim in `OutlineNode.text` separated by `\n`;
  the prefix on each continuation line is preserved by the same "trim leading indent, append to text" path so blockquote bodies round-trip cleanly as CommonMark.
- Render outline AST → `.md` (clean, no IDs).
  Each line in `OutlineNode.text` after the first is emitted at `indent + 1`; the renderer **does not invent** prefixes on continuation lines — whatever the user (or the parser) put in `text` round-trips as-is.
  Block-kind markers (`TODO `, `DONE `, `> `) are owned by `outl-actions` (`todo.rs`, `quote.rs`); this crate only preserves them verbatim.
- Read/write `.outl` sidecar (JSON, dotfile) — current version `2`, reads v1 transparently (handles backfilled on load).
  The sidecar is **structural metadata only** (id, line, indent, content hash, ref handle).
  State that must converge between devices (fold flags, pinned, etc.) goes through the op log in `outl-core`, never here.
- The 3-level matching algorithm (external edit → reconstruct IDs)
- Diff (old AST + new AST + old sidecar blocks) → minimal sequence of `Op`s, preserving `ref_handle` verbatim on level-1/2 matches.
  The old-handle lookup is O(N) overall (HashMap by id), not O(N²) — never reintroduce a linear scan per new block.
  **`diff_to_ops` only emits structural ops (Create / Move / SetProp); a second pass inside `reconcile_md::sync_block_text` walks the AST + new sidecar in lockstep and emits one `Op::Edit` per block whose text differs from the workspace.
  Skipping that pass silently zeroes text across devices — local stays fine, peer replays an empty tree, iCloud ships the empty `.md` back.**
- **`outline_ops`** — pure `Vec<OutlineNode>` AST helpers (`flat_count`, `path_for_index`, `insert_sibling_after/before`, `indent_at_path`, `outdent_at_path`, `delete_at_path`, `move_up_at_path`, `move_down_at_path`, …).
  They operate on an in-flight AST that hasn't been parsed back into a workspace yet, so they sit in `outl-md` (UI-agnostic, no `Workspace`) rather than in `outl-actions`.
  The TUI re-exports them through a one-line shim at `outl-tui/src/outline_ops.rs`; the mobile client consumes them directly.
  **Insert helpers clamp**:
  `insert_sibling_after/before` clamp the computed position to `siblings.len()` instead of panicking when a caller passes a path the live tree no longer satisfies,
  (typical case: page parsed to zero blocks because its content didn't start with a `- ` marker, but the TUI's `selected` cursor defaulted to `[0]`).
  Falling back to "append at the end" is the right shape — the user's intent ("create a new block") is satisfied, no data is lost.
- **Emoji catalog** (`emoji.rs`) — GitHub gemoji catalog (backed by the [`emojis`] crate).
  `shortcode_to_unicode("tada") → Some("🎉")` is the one-way resolver every renderer uses;
  `search(query, limit) → Vec<EmojiHit>` powers the `:shortcode:` autocomplete shared by TUI / mobile / desktop through the `outl_emoji_search` Tauri command,
  and **iterates every alias** (`emoji.shortcodes()`, not `shortcode()`) so the autocomplete returns the same set the parser accepts (`:+1:` and `:thumbsup:` both surface).
  `is_valid_shortcode_char(c)` is the char-level alphabet check — exported so consumers walking buffers char-by-char (`try_emoji`, TUI's `detect_trigger`) avoid allocating a 1-char `String` per keystroke.
  The parser only tokenizes `:foo:` when `shortcode_to_unicode` finds `foo`, so unknown input (`:notarealemoji:`, `meeting at 14:00`) stays plain.
  **Never retro-translate `glyph → shortcode`** — multiple shortcodes can alias the same codepoint (`:+1:` and `:thumbsup:` both → 👍) so the disk form would become lossy.
- **Inline tokenization** (`inline.rs`) — `**bold**`, `[[refs]]`, `#tags`, `((blk-XXXXXX))`, `!((blk-XXXXXX))`, `:shortcode:` — and `ref_at_cursor` (resolves to `RefTarget::Page`, `Journal`, `Tag`, or `Block`).
  **UI-agnostic.**
  TUI, future Tauri GUI, and mobile clients all consume the same `InlineTok` / `RefTarget` types and map them to their own primitives (`Span`, HTML, `AttributedString`, `AnnotatedString`).
  Two forms:
  - `InlineTok<'a>` + `tokenize` — borrowed, zero-copy.
    Use inside Rust where the source string outlives the tokens.
  - `InlineToken` (owned) + `tokenize_owned` — Serde-friendly, suitable for wire payloads.
    `outl-actions` attaches the result to `OutlineNode.tokens` so mobile renders without a TS tokenizer.
    Adding a variant to `InlineTok` requires adding the matching variant to `InlineToken` plus the conversion in `InlineToken::from_borrowed` in the same change.
  **Underscore emphasis rule (CommonMark):** `_` does not open or close emphasis when it appears inside a word (no surrounding whitespace/punctuation on both sides).
  `chamados_chat`, `inc_lag1`, `prod.ml_atendimento` stay literal.
  `*` is not subject to this restriction — it works mid-word.
  Enforced in `try_italic_under` / `try_bold_under` via the `closing_underscore` helper.
  **`inline.rs` is over 900 lines** — known refactor debt; invoke `refactor-architect` before adding further features to this file.
- **External frontmatter** (`frontmatter.rs`) — metadata extraction for markdown authored by other tools.
  `split_frontmatter` splits the leading `---` fence off a `.md` body (CRLF-safe, honours the `...` end marker; no closing fence → whole file stays body).
  `parse_frontmatter(yaml, drop_keys) → Frontmatter { title, props, dropped }` flattens the YAML into `key:: value` properties: `title` lifted, `tags` normalized to `#name`, caller-supplied drop-list, values verbatim.
  Date normalization is caller policy because the flexible date parser lives in `outl-actions`, which depends on this crate.
  `extract_leading_h1` lifts a leading `# H1` line into a title (first non-blank line only).
  Consumed by the CLI importers (Obsidian today); source-specific key policy stays with the caller.
- **External wiki-link rewriting** (`wikilink.rs`) — `rewrite_wikilinks` / `clean_wikilink_target` collapse `[[Note|alias]]` / `[[Note#heading]]` / `[[Note^block-id]]` / `[[folder/Note]]` to canonical `[[Note]]`;
  `convert_image_links` / `is_image_target` turn image wiki-links and embeds (`![[img.png]]`, `[[a/b.jpeg|cap]]`) into standard CommonMark links with the folder path preserved.
  Pure text → text; no vault layout or routing policy.
- **Tag predicate** (`tag.rs`) — `text_contains_tag(text, tag)`: boundary-correct "does this text mention `#tag`?" built on the tokenizer.
  `#tag-longer` / `#tagged` never match `tag`; a `#tag` inside a `` `code` `` span is not a tag.
  Consumers must use this instead of `text.contains("#tag")` (the substring form is the false-positive bug this module deleted from the CLI).
- **Block index** (`block_index.rs`) — `NodeId → BlockEntry`, `ref_handle → NodeId`, `NodeId → [BlockReference]` (reverse refs), `(slug, dfs_path) → NodeId` for location lookup.
  Population is two-pass (`collect_page_blocks` then `collect_page_refs`) so reverse edges survive arbitrary page-load order during the initial build.
  Lookups are O(1).
- **Workspace index** (`index.rs`) — page-level (`slug → PageEntry`, backlinks) plus block-level (re-exports the `BlockIndex` API).
  Public surface includes `resolve_block_ref(handle)`, `block_by_id`, `block_at_location(slug, &[usize])`, `block_refs_to(id)`, `iter_blocks`, `block_count`, `search_block_text(query, limit)`.
  `block_index()` borrows the inner `BlockIndex` so a consumer that already holds a `WorkspaceIndex` can reuse its primitives through one value.
  `block_at_location` is the O(1) replacement for scanning `iter_blocks()` to find the entry for a known `(page, dfs_path)`, e.g. when the TUI translates a keyboard chord onto a specific block.
  `PageEntry` carries the page-level metadata every UI surface reads (`slug`, `title`, `icon`, `is_journal`, `pinned`, **`page_type`**);
  `pages_by_type(t)` filters pages by their `type::` property (case-insensitive), powering the `@` mention autocomplete that lists `type:: person` pages.
- **Slugify** (`slug.rs`) — `[[Avelino]]` → `pages/avelino.md`.
  The user-facing name is preserved verbatim in the page's `title::` property.
- **`derive_ref_handle(NodeId) -> String`** (`sidecar.rs`) — deterministic: `blk-` + last 6 chars of the ULID's Crockford base32, lowercased.
  Same input always yields the same handle so two devices agree on what `((blk-XXXXXX))` means.
  On a collision inside a single workspace, the **second** block to land gets its handle lazily expanded one character at a time (drawing from the same ULID tail) until unique — both the winner and the loser stay independently resolvable.
  The sidecar still records the deterministic 6-char form; the expanded handle lives in `BlockEntry.ref_handle` in memory and in the workspace handle map.
- **`BlockEntry.text_fold: String`** — lowercased cache of the block's `text`, populated at index build.
  Powers `search_block_text` without allocating per keystroke.
  Public field, but consumers must not build `BlockEntry` by hand — go through the index population path so `text_fold` stays consistent with `text`.

## What this crate does NOT own

- The op log → `outl-core`
- File watching / debounce → `outl-cli`
- Reconcile TUI → `outl-tui`
- Network sync → `outl-sync-iroh` (P2P via iroh, default transport; file/iCloud opt-in)

## The 3-level matching algorithm

When an external save lands on `pages/foo.md`:

1. **Parse** new `.md` → AST without IDs.
2. **Load** `foo.outl` (sibling of `foo.md`) → AST with old IDs and content hashes.
3. **Match** new ↔ old blocks at 3 confidence levels:

| Level | Confidence | Criteria | Action |
|-------|-----------|----------|--------|
| 1 | High | `content_hash` exact match, same parent (by hash) or identical structure | Preserve ID, emit `Move` if position changed |
| 2 | Medium | Normalized Levenshtein similarity > 80%, same parent OR position within ±2 lines | Preserve ID, emit `Edit` (+ `Move` if needed), log warning |
| 3 | Low / no match | Falls through | New ULID for new block; old block becomes `Delete` (`Move` to `TRASH_ROOT`); record in `.outl/orphans.log` |

**Hard rule:** a block that drops to level 3 must appear in `orphans.log` before being deleted.
**Silent deletion is a P0 bug.**

## Sidecar format

Current version: `2`.
Full spec in [`docs/markdown-format.md`](../../docs/markdown-format.md#the-outl-sidecar).

```json
{
  "version": 2,
  "page_id": "01HXY8KJZQ9T8M7VN3P2R6S4A0",
  "last_synced_hash": "sha256:...",
  "last_synced_at": "2026-05-24T11:22:00-03:00",
  "blocks": [
    {
      "id": "01HXY8KJZQ9T8M7VN3P2R6S4A1",
      "line": 1,
      "indent": 0,
      "content_hash": "sha256:...",
      "ref_handle": "blk-r6s4a1"
    }
  ]
}
```

- `content_hash` = SHA-256 of the **block's textual content** (not children).
- `ref_handle` = short user-typeable handle for `((blk-XXXXXX))`. v1 sidecars (no field) load fine — the handle is backfilled in memory via `derive_ref_handle`.
  The next write persists v2.
  On collision, expansion may produce a 7+ char form (see `derive_ref_handle` above).

**Sidecar is not a sync surface.**
UI state that must converge between devices — fold flags, pinned, selection, anything user-meaningful — goes through the op log (`outl-core`), not here.
iCloud / Syncthing sync the sidecar file as one blob with last-write-wins semantics, so two devices flipping different fields in the same window lose data.
The op log gives each actor its own jsonl, lets the FS sync per-file without conflict, and reconverges through the CRDT.
See the root `CLAUDE.md` invariant 7.
- Sidecar lives next to the `.md` as `pages/<slug>.outl` (no leading dot).
  Replicated between devices alongside the `.md`.
  Don't gitignore by default.
  The dotfile form (`.foo.outl`) was abandoned because iCloud Documents skips dotted paths during cross-device sync.
- **Stale entries are skipped during index build.**
  When a sidecar block's `content_hash` no longer matches the corresponding block in the `.md`, that entry is left out of the workspace index instead of polluting it with a wrong subtree.
  The block reappears in the index after the next reconcile updates the sidecar.

## Outl markdown dialect

```markdown
title:: example
status:: active
tags:: #project

- top level block
  priority:: high
  - child block with [[page reference]]
  - child block with ((blk-r6s4a1))
  - expanded inline: !((blk-r6s4a1))
- another top level
```

- `key:: value` lines at top of file = page properties (frontmatter outliner-style).
- `key:: value` lines nested as children of a block = block properties.
- `[[name]]` = page reference (bidirectional link).
- `[[2026-05-24]]` = journal reference (renders as date).
- `#tag` = tag (page reference with classification semantics).
- `((blk-XXXXXX))` = inline block reference (renders as the source block's text).
- `!((blk-XXXXXX))` = block embed (renders source block expanded with subtree).
- `{{query: ...}}` = saved query (query DSL not yet implemented, parse as opaque for now).

**No `id::`, no UUID, no HTML comments** — IDs go in the sidecar only.

## Files

```
src/
├── lib.rs
├── parse.rs        # md → AST (no IDs)
├── render.rs       # AST → md (clean)
├── sidecar.rs      # read/write .outl JSON, derive_ref_handle, content_hash
├── matching.rs     # 3-level matching algorithm
├── diff.rs         # AST diff → Op sequence (takes old_blocks to preserve ref_handle)
├── inline.rs       # InlineTok (Plain/Bold/.../BlockRef/Embed/Emoji), RefTarget, ref_at_cursor
├── emoji.rs        # shortcode_to_unicode, search, is_valid_shortcode, EmojiHit
├── frontmatter.rs  # split_frontmatter, parse_frontmatter, extract_leading_h1 (external md metadata)
├── wikilink.rs     # rewrite_wikilinks, clean_wikilink_target, convert_image_links, is_image_target
├── lang.rs         # canonical(fence) — alias table shared by outl-exec + frontend syntax highlighter
├── index.rs        # WorkspaceIndex — page-level + block-level facade
├── block_index.rs  # BlockEntry, BlockReference, BlockIndex (id ↔ handle ↔ reverse refs)
├── reconcile.rs    # high-level reconcile_md (parse → match → diff → apply)
├── slug.rs         # slugify page names
├── tag.rs          # text_contains_tag — boundary-correct #tag predicate over the tokenizer
├── view.rs         # render helpers consumed by UIs
└── atomic.rs       # crash-safe write_atomic

tests/
├── roundtrip.rs              # render(parse(md)) == md (property test)
├── external_edit.rs          # light external edit preserves IDs
├── duplicate_block.rs        # Ctrl+D in vscode → first keeps ID, second gets new
├── identical_blocks_swap.rs  # two identical blocks change parents
└── heavy_edit.rs             # >20% content change → level 2 warning

benches/
└── block_index.rs            # resolve / search_block_text on 100k blocks
```

## Bench harness

`cargo bench -p outl-md --bench block_index` measures the cost the `((blk-XXXXXX))` path adds to the index.
Today's numbers (M-series laptop):

- `resolve(handle)` — ~17 ns at 100k indexed blocks.
  O(1) HashMap hit.
- `search_block_text(query, limit)` — ~12 ms at 100k blocks (linear scan with case-fold + position scoring).
  Suitable for the autocomplete popup the TUI uses today; future fzf-style scoring can drop in behind the same signature.

## Invariants

1. **Roundtrip stability.** `render(parse(md))` produces a semantically identical `.md` (same tree, properties, content; whitespace may normalize).
2. **No silent block loss.**
   A block falling to level 3 is always in `orphans.log`.
3. **Sidecar is JSON-valid.**
   Always.
   If you can't write valid JSON, you fail.
4. **Sidecar `version` field always present.**
   Future migrations.
5. **`content_hash`** is `sha256(block.content_text())` consistently.
   Same hash function across read and write.
6. **`ref_handle` is preserved across level-1 and level-2 matches.**
   `diff_to_ops` reads it from the previous sidecar's block list and reuses it verbatim,
   so a `((blk-XXXXXX))` already written in another `.md` keeps resolving even if the handle was once expanded past the default 6-char tail.
7. **`derive_ref_handle` is deterministic** — same `NodeId` in, same handle out.
   Two devices building the sidecar independently must agree on what `((blk-XXXXXX))` means.

## Things to never do here

- ❌ Write IDs into the `.md` file (use sidecar)
- ❌ Delete a block in matching without recording in `orphans.log` first
- ❌ Match on similarity > 80% across **different parents** without warning
- ❌ Skip the property test in `roundtrip.rs`
- ❌ Use a different hash function in sidecar read vs write
- ❌ Drop sidecar version 1 support when adding version 2 (always backward read)
- ❌ Block on a corrupt sidecar — fall back to "regenerate from op log" via `outl doctor`

## Reuse-first

This crate owns the **shared parsing and view primitives** every client needs (`char_to_line_col` / `line_col_to_char`, `block_to_rows`, `tokenize`, `slugify`, `derive_ref_handle`, …).
Clients should *wrap* these, not re-derive them.

When you add a primitive, pair it:
`char_to_line_col` already existed;
the recent `line_col_to_char` addition made the pair complete so `outl-tui::EditBuffer::move_up` / `move_down` could be 3-line wrappers instead of duplicating the line/column scan.
**Inverses, encoders/decoders, and parser/renderer pairs always ship together** so the next consumer doesn't have to re-derive half of one.

If you find a client (`outl-tui`, `outl-mobile`, `outl-actions`) hand-rolling something that's already here, move the call to your API and delete the duplicate.
The root [`CLAUDE.md`](../../CLAUDE.md#reuse-first-no-parallel-implementations) documents this at the workspace level.

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-md -- -D warnings`
3. `cargo test -p outl-md`
4. `/roundtrip` to invoke the markdown-roundtrip-tester agent
5. Manual smoke: create a fixture md, render it back, diff
