# Markdown Format

The outl markdown dialect, the sidecar `.outl` format, and the 3-level matching algorithm that bridges them.

## Why this matters

The user sees the `.md`.
The op log uses stable IDs.
We need both, without visible metadata in the file.
Every design decision here serves that.

---

## The .md file

```markdown
title:: meu projeto x
type:: project
status:: active
tags:: #produto #2026-q2
created:: [[2026-05-24]]

- objetivo principal #okr
  priority:: high
  owner:: [[avelino]]
  - métrica: 30% redução de custo
  - prazo: [[2026-06-30]]
- riscos
  - dependência de [[fornecedor y]] #risco
```

That is the **entire file**.
No IDs, no UUIDs, no HTML comments, no frontmatter delimiter.
What you see is what's on disk.

### Page properties (top of file)

Lines at the start of the file in the form `key:: value` are page-level properties.
Parsing stops at the first blank line or first `-` outline item.

```
title:: my page
type:: project
status:: active

- first block...
```

The `type::` key carries the page's semantic kind and is consumed by surfaces that filter pages by role — `type:: person` is the canonical example, recognised by the `@` mention autocomplete (see [§Mentions](#mentions-name)). Other types (`type:: project`, `type:: meeting`, …) are free-form today; the autocomplete only filters on `person`.

### Outline items

Standard markdown unordered lists:

```
- top level block
  - child block
    - grandchild block
- another top level
```

Indent is **two spaces** per level.
Tab is normalized to two spaces on parse and rendered as two spaces.

### Multi-line block text (continuation lines)

A bullet's text can span multiple lines.
Subsequent lines are indented **one level deeper** than the bullet and contain no marker of their own:

```
- first line of the block
  second line of the same block
  third line
```

Parsed as a single block with `text = "first line of the block\nsecond line of the same block\nthird line"`.
Renders back identically.

Continuation **ends** at the first line that is a block marker (`-`), a property (`key:: value`), or an unindent.
After that point, plain indented text becomes "unrecognized" and is skipped — keeps the grammar unambiguous.

In the TUI: `Alt+Enter` (or `Ctrl+J`, or `Shift+Enter` in kitty-protocol-aware terminals) inserts a soft newline inside the current block.
Plain `Enter` commits and creates a sibling — unless the cursor is inside an open fenced code block, in which case `Enter` auto-detects and inserts a soft newline instead (see below).

### Block-level prefixes (TODO / DONE / quote)

A block's *kind* (open task, completed task, blockquote) is encoded as a **text prefix** on the bullet's body, not as a separate AST field.
This keeps the wire format stable, round-trips through any CommonMark renderer, and lets a user drop the marker by erasing the prefix in their editor.

| Prefix | Meaning | Helpers |
|---|---|---|
| `TODO ` | Open task | `outl_actions::todo::TODO_PREFIX` / `split_todo` / `cycle_todo` |
| `DONE ` | Completed task | `outl_actions::todo::DONE_PREFIX` / `split_todo` / `cycle_todo` |
| `> ` | Blockquote (CommonMark-compatible) | `outl_actions::quote::QUOTE_PREFIX` / `split_quote` / `toggle_quote` |

Rules:

- One space follows the marker.
  `>foo` (no space) is **not** a quote — same CommonMark rule that decides `>foo` is a literal paragraph and `> foo` is a blockquote.
- Markers compose in a **canonical order**: `"TODO > body"` / `"DONE > body"` (TODO/DONE before the quote marker).
  `toggle_quote` and `cycle_todo` peel both prefixes off and re-emit in canonical order, so a user who authors them in the other order (`"> TODO foo"`) gets normalised on the next toggle.
  Why canonical order matters: the backend's `split_todo` reads from the **start** of the text, so a `"> TODO foo"` would surface in the `OutlineNode` DTO as `todo = null` and the literal `"> TODO foo"` in `text` — checkbox disappears mid-flight on mobile / desktop.
  The TUI's `split_block_prefixes` still accepts either order for *display* (so externally authored `.md` reads correctly), but every mutation in `outl-actions` emits the canonical form.
- Children of a quoted block are **not** implicitly quoted — the marker lives on the block, not on its subtree.
  Same policy as TODO/DONE.
- Multi-line quote bodies keep the `> ` on every continuation line so the file stays a valid CommonMark blockquote when an external tool opens it.

Single-line:

```
- > the only way to do great work is to love what you do
- regular block
```

Multi-line:

```
- > quote line one
  > quote line two
  > quote line three
- next regular block
```

Inline tokens (`**bold**`, `[[ref]]`, `#tag`, `((blk-…))`) tokenize **inside** the quoted body — the wrapper is transparent to the inline tokenizer.

### Fenced code blocks inside a bullet

CommonMark code fences are preserved literally:

```
- intro paragraph
  ```lisp
  (+ 1 2)
  ```
- next bullet
```

The opening ` ``` ` may live on the bullet line itself (`` - ```lisp ``) — parser, renderer, and the [`outl-exec`] engine all handle that shape correctly.

What makes fences different from regular continuation:

- Content between the opener and closer is preserved **verbatim**.
  No `-`, no `key::`, no inline syntax recognition.
- The closer is a line whose trimmed content is exactly `` ``` `` (with optional trailing backticks) at the same indent as the opener.
- A missing closer is gracefully synthesized at EOF so the rendered output stays well-formed; the parser also breaks out of a fence when a sibling bullet outdents below the fence indent — better than swallowing the rest of the document.

[`outl-exec`]: ../crates/outl-exec/

#### Language tag aliases

The opening fence's info-string (`` ```rs `` / `` ```javascript `` / `` ```py3 ``) gets canonicalised through a single shared alias table — [`outl_md::lang::canonical`] in Rust, [`@outl/shared/highlight::canonical`] in TypeScript.
Both layers use the same canonical names so what runs at the backend, what gets syntax-highlighted in the desktop / mobile editor, and what the user types in the fence all line up.

A handful of the common aliases:

| You write | Resolves to | Notes |
|---|---|---|
| `js`, `javascript`, `node`, `nodejs` | `js` | Maps to the `js` runtime in `outl-exec`. Before the alias table, `` ```javascript `` failed with "no runtime registered". |
| `rs`, `rust` | `rust` | |
| `py`, `python`, `python3` | `python` | |
| `sh`, `bash`, `zsh` | `shell` | Highlight only — no runtime yet. |
| `yml` | `yaml` | |
| `md` | `markdown` | |
| `c++`, `cxx`, `cc`, `cpp` | `cpp` | |
| `cs`, `c#` | `csharp` | |
| `clj` | `clojure` | |

The full table lives in `crates/outl-md/src/lang.rs::KNOWN_ALIASES`; the TS mirror is `crates/outl-frontend-shared/src/highlight/aliases.ts`.
Add a row in both files in the same commit — the `doc-sync-guard` hook treats this as a shortcut-level change and refuses the edit otherwise.

#### Syntax highlighting (desktop + mobile)

`outl-desktop` and `outl-mobile` both render code fences in read mode through the shared `<HighlightedCode />` component, which lazy-loads [`highlight.js`'s "common" bundle][hljs-common] (~30 popular languages, ~80 KB) and applies the brand palette defined in `crates/outl-frontend-shared/src/highlight/styles.css`.

Unknown / empty languages fall back to a plain `<pre>` with the brand-dark canvas — we never use highlight.js's `"auto"` detection because the misclassification cost (Bash highlighted as Perl) is worse than visual flatness.

The TUI renders fences as monospace text without syntax colouring today; the planned approach when this lands is `syntect` keyed on the same canonical names from `outl_md::lang`.

[`outl_md::lang::canonical`]: ../crates/outl-md/src/lang.rs
[`@outl/shared/highlight::canonical`]: ../crates/outl-frontend-shared/src/highlight/aliases.ts
[hljs-common]: https://github.com/highlightjs/highlight.js/blob/main/src/index.js

### Block properties

A line in the form `key:: value` *as a child of an outline item* is a block property:

```
- objective
  priority:: high
  owner:: [[avelino]]
  - this is a regular child block
```

`priority::` and `owner::` are properties of `objective`, not children.
The third line (`- this is a regular child block`) is a real child.

### Inline syntax

| Syntax | Meaning |
|--------|---------|
| `[[name]]` | Reference to page named "name" |
| `[[2026-05-24]]` | Reference to journal "2026-05-24" (rendered as date) |
| `[[@name]]` | Mention — reference to the person page `name` (page-level `type:: person`); the `@` is the link affordance, not part of the page identity |
| `#name` | Tag (page reference with classification semantics) |
| `((blk-XXXXXX))` | Block reference — renders as the source block's text, links to it |
| `!((blk-XXXXXX))` | Block embed — renders the source block expanded with its subtree |
| `{{query: ...}}` | Saved query (phase 3 — parse as opaque) |
| `**bold**`, `*italic*`, `\`code\`` | Standard CommonMark |

#### Block refs and embeds

`((blk-XXXXXX))` is an inline reference to another block.
The handle is short, lowercase, and human-typeable — it's the last 6 Crockford base32 characters of the block's ULID, prefixed with `blk-`.
Renderers resolve the handle through the workspace index and display the source block's text in place; the on-disk `.md` keeps the literal `((blk-XXXXXX))`.

`!((blk-XXXXXX))` is the embed form.
Same lookup, but the consumer expands the source block **and its subtree** inline.
Mirrors the markdown image syntax (`![alt](url)` → "expand") so the `!` reads as the visual hint for inflation.

```
- decide which database to use #decision
- in [[Project X]], see ((blk-r6s4a1)) for context
- the whole thread: !((blk-r6s4a1))
```

Handles are persisted in the sidecar (see [§sidecar](#the-outl-sidecar)) so a future change to the derivation scheme cannot break references already living in `.md` files.
An orphaned handle (citation points at a block that no longer exists) renders dimmed in the TUI and is flagged by `outl doctor`.

Handle collisions are vanishingly unlikely — 6 lowercase base32 chars is ~30 bits, ~5×10⁻⁶ birthday probability at 100k blocks.
When two blocks do land on the same base handle, the second block's handle is lazily expanded one character at a time (from the ULID's Crockford base32 tail) until unique within the workspace, so both the winner and the loser stay resolvable through their own (distinct) handles.
The on-disk sidecar still records the deterministic 6-char handle — the divergence lives in memory until a future reconcile rewrites it.
Workspaces that ever expanded a handle to 7+ characters keep working forever because lookup goes through the in-memory handle, not the literal sidecar field.

#### Mentions (`[[@name]]`)

`[[@name]]` is **not a new token**. It is a regular page reference whose target literal happens to start with `@`. Roundtrip, render, matching, and reconciliation flow through the same code path as `[[ref]]` — no separate parser branch.

What makes it act as a mention is policy on top of the existing primitive:

- **Page identity does not carry the `@`.** The page is `pages/<slug>.md` with `title:: <name>` (no `@`); the `@` belongs to the link affordance, like the `!` in `!((blk-XXXXXX))` belongs to the embed affordance.
- **Reference resolution strips the `@`.** Opening `[[@avelino]]` resolves to the page `avelino` via the same decision tree as any `[[ref]]` (slug match → slugified match → case-insensitive title match → create). The `@` is consumed by the resolver before the lookup.
- **Create-on-miss marks the new page as a person.** When the target doesn't exist yet, the resolver creates `pages/<slug>.md` and sets `type:: person` on it automatically — the next mention surfaces the page in the autocomplete popup without the user editing properties by hand.
- **Backlinks recognise the `@`-aliased form.** A person page's backlinks panel surfaces blocks that wrote `[[@name]]` even though the page slug is `name`. Plain (non-person) pages do not scan the `@`-aliased form, so `[[@projeto]]` in a block does not accidentally show up under a non-person `projeto` page.

#### Autocomplete trigger

While the user is editing a block, every client opens a person picker on a **word-initial `@`** (the same word-initial rule the `#` tag trigger uses: `@` preceded by start-of-line or whitespace). The popup lists pages where `type:: person` is set, fuzzy-matched against whatever was typed after the `@`. Accepting a candidate inserts `[[@<title>]]` at the caret.

- Composite names work: `@Thiago Avelino` is a valid query — the autocomplete query allows spaces, unlike `#tag` which terminates on the first non-word character.
- Mid-word `@` is ignored — `a@b.com` (an email) is not a mention.
- When no existing person matches the query, the popup offers the query itself as a "create new" candidate so a fresh mention can be minted without leaving the keyboard.

The `type::` page-level property is what scopes the autocomplete candidate list (and marks the page semantically); the `@`-prefixed link text is what makes the rendered reference visually a mention.

### What is **not** in the file

- ❌ `id::` lines (Logseq-style) — IDs go in the sidecar
- ❌ `<!-- block-uid: ... -->` — no HTML comments for metadata
- ❌ YAML frontmatter (`---`) — page properties use `::` syntax instead
- ❌ `\`\`\`outl` fenced metadata blocks

### Permissive parsing & warnings

A hand-written or imported `.md` may contain lines that don't fit the dialect — typically a leading `# heading`, a paragraph, an HTML snippet, or a table.
The parser is **permissive**: every such line at depth 0 is preserved verbatim as a regular outline block, and the recovery is recorded in `ParsedPage.warnings: Vec<ParseWarning>`.

Each warning carries:

- `line` — 1-based source line number.
- `raw` — the offending line, verbatim (no trim).
- `kind` — currently `UnrecognizedBlockMarker` (line didn't start with `- ` and isn't a recognized property).

Two consequences worth knowing:

- **No silent data loss.** Open a file, save it, the content survives.
  Render is still clean: blocks created from recovered lines render as `- <raw>\n`, so the next save normalizes the file to the dialect.
- **Surfaces show the warnings.** The TUI banner, the mobile / desktop overlay, and `outl doctor` all read `ParsedPage.warnings` and present them as actionable hints (line number + first 60 chars of the raw text).
  Users can edit the file at their pace; outl never blocks on a "dirty" page.

---

## The .outl sidecar

For every `pages/foo.md` there is `pages/foo.outl` (sibling file, not a dotfile).
The dotted form was abandoned in v0: iCloud Documents silently skips paths starting with `.` when syncing across devices, so a dotted sidecar never reached peers and "sync" appeared to lose block IDs.
Same rule keeps the op directory at `ops/` rather than `.ops/`.

Format: JSON.

```json
{
  "version": 2,
  "page_id": "01HXY8KJZQ9T8M7VN3P2R6S4A0",
  "last_synced_hash": "sha256:e3b0c44298fc1c14...",
  "last_synced_at": "2026-05-24T11:22:00-03:00",
  "blocks": [
    {
      "id": "01HXY8KJZQ9T8M7VN3P2R6S4A1",
      "line": 7,
      "indent": 0,
      "content_hash": "sha256:abc123...",
      "ref_handle": "blk-r6s4a1"
    },
    {
      "id": "01HXY8KJZQ9T8M7VN3P2R6S4A2",
      "line": 8,
      "indent": 1,
      "content_hash": "sha256:def456...",
      "ref_handle": "blk-r6s4a2"
    }
  ]
}
```

> The sidecar is **structural matching metadata only** — block id, position, content hash, ref handle.
> State that must converge between devices (fold flags, etc.) lives in the op log (`outl-core`), never here. iCloud syncs the sidecar with LWW per-file, which would silently drop concurrent writes.

### Fields

- `version`: always present, integer.
  Future migrations check this.
- `page_id`: ULID of the page itself (the top-level container).
- `last_synced_hash`: SHA-256 of the full `.md` file at last sync.
  Used as a fast "did this change?" check.
- `last_synced_at`: ISO 8601 timestamp with timezone, set on last write.
- `blocks`: array, in tree order (depth-first, preorder).
  Each entry:
  - `id`: ULID of the block.
  - `line`: 1-indexed line number in the `.md` at last sync.
  - `indent`: 0 for top-level outline items, 1 for first child, etc.
  - `content_hash`: SHA-256 of the block's **textual content only**, not including children or property lines that belong to it.
  - `ref_handle`: short, stable, user-typeable handle for `((blk-XXXXXX))` references.
    Default-derived from the id (last 6 chars of the ULID's Crockford base32, lowercased, with the `blk-` prefix).
    Persisted verbatim so future changes to the derivation cannot invalidate references already in the wild.

### Content hash

```
content_hash(block) = sha256(block.text_content.trim().normalize())
```

Where `normalize` collapses internal whitespace to single space and strips trailing whitespace.
This makes the hash robust to whitespace-only edits in external editors.

**Same hash function on read and write.** Diverging hashes silently break matching.

### Sidecar versioning

Current version is **`2`** (added `ref_handle` per block to power `((blk-XXXXXX))` references).
Reading is backward-compatible:

- A v1 sidecar (no `ref_handle` field) loads fine; the field is backfilled in memory by deriving it from the block id.
- The first write after a load upgrades the on-disk payload to the current version.
- Sidecars below `MIN_READABLE_SIDECAR_VERSION` (currently `1`) fail loudly — old workspaces in the wild stay supported until that constant moves.

When v3 ever ships, v1 + v2 read paths stay until they're explicitly retired.
Silent format drops are not allowed.

---

## Roundtrip

```
parse(render(ast)) == ast
render(parse(md)) ≈ md
```

The second is "semantically equivalent", not byte-equivalent.
We may normalize:

- Tabs → two spaces.
- Trailing whitespace on lines stripped.
- Final newline added if missing.
- Property lines with leading whitespace normalized.

We never:

- Reorder blocks.
- Change content.
- Drop properties.
- Change inline link syntax.

Roundtrip is a **property test** in `crates/outl-md/tests/roundtrip.rs`.
Treat it as part of the spec — if your parse/render changes break it, either the test is wrong (rare) or your change is.

---

## 3-level matching

When a file save lands on disk and the `.md` differs from the sidecar's `last_synced_hash`:

1. **Parse** new `.md` → `new_ast` (no IDs).
2. **Read** sidecar → `old_ast` (with IDs, with hashes).
3. **Match** blocks `new ↔ old` at three confidence levels.

### Level 1 — High confidence

Block matches by:
- `content_hash` exact match between `old_block` and `new_block`, AND
- parent matches (by hash of parent, or both are root-level)

→ Preserve ID.
If position changed, emit `Op::Move`.

### Level 2 — Medium confidence

Block matches by:
- Normalized Levenshtein similarity > 80%, AND
- (same parent OR position within ±2 lines)

→ Preserve ID.
Emit `Op::Edit` (and `Op::Move` if needed).
Log a warning to `.outl/orphans.log`:

```
2026-05-24T11:22:01-03:00 medium-confidence match block=01HXY... similarity=0.83
```

### Level 3 — No match

Block in `new_ast` has no match in `old_ast`:

→ New ULID assigned.
Emit `Op::Create`.

Block in `old_ast` has no match in `new_ast`:

→ Move to TRASH_ROOT (`Op::Move` to trash).
Emit before deletion:

```
2026-05-24T11:22:01-03:00 orphan block=01HXY... content="começava com..."
```

**Hard rule:** every level-3 deletion must hit `orphans.log` before the op is committed.
Silent deletion is a P0 bug.

### Tiebreakers

When two new blocks would match the same old block (or vice versa) at the same confidence:

1. Prefer matches at the same position.
2. Prefer matches with the same parent.
3. Prefer matches where the parent chain matches deepest.
4. If still tied: pick the one that minimizes total moves across the matching as a whole (greedy is fine for phase 1; optimal can come later).

---

## Edge cases

### Duplicated block (Ctrl+D)

User selects a block in VS Code and presses Ctrl+D.
Now there are two blocks with identical content.

- First one matches the old `content_hash` at level 1.
  Keeps ID.
- Second one has the same hash but its **position** differs from the old one.
  After the first match is consumed, no other old block has this hash. → Level 3.
  New ULID.

### Two identical blocks swap parents

A and B both contain "TODO".
A was under page X, B under page Y. After edit, A is under Y, B is under X.

- Pure hash match alone is ambiguous (both new blocks match both old blocks).
- Tiebreaker: parent matches → A stays under "Y" matches the old A under Y?
  No — the old A was under X. The "parent matches" tiebreaker breaks.
- Fall back: minimize total moves.
  Either pairing requires one move.
- Then minimize position diff.
  Pick the assignment that's lexicographically smallest.

This is the case tested in `identical_blocks_swap.rs`.

### Heavy edit (>20% content change)

The content hash is gone.
Similarity may drop below 80%.

- If still > 80%: level 2, log warning.
- If below 80% but parent and position match: still level 2, more prominent warning.
- If below 80% AND parent doesn't match: level 3, treat as new block.

### Rename of header with many children

Heading text changes.
Children unchanged.

- Header block: hash mismatch.
  Probably level 2 (similarity > 80% if rename is partial).
  New ID if rename is total.
- Children: hash matches.
  Stay under the (possibly new-ID) header because they were always under "the block at this position".

The structure tiebreaker handles this: we match parent chains, not parent IDs, when parents are themselves ambiguous.

---

## `outl reconcile` (manual resolution)

If the matching produces orphans or level-2 warnings, the user runs:

```
outl reconcile
```

A TUI opens showing one orphan at a time with candidates.
Keys:

| Key | Action |
|-----|--------|
| `j` / `k` | next/prev candidate |
| `enter` | accept match |
| `d` | confirm delete (orphan stays as `Move` to trash) |
| `s` | skip (revisit later) |
| `q` | quit |

---

## External paste → outl syntax

When the user pastes clipboard markdown from another outliner / note app into outl, `outl_actions::paste_markdown` (in `outl-actions`) normalises the input before parsing it as bullets.
The same pipeline runs in the TUI (bracketed-paste handler) and the mobile client (textarea `onPaste`).

| Input (external) | Output (outl) | Origin |
|------------------|---------------|--------|
| `{{[[TODO]]}} foo` | `TODO foo` | Roam |
| `{{[[DONE]]}} foo` | `DONE foo` | Roam |
| `- [ ] foo` | `- TODO foo` | GitHub / CommonMark task list |
| `- [x] foo` / `- [X] foo` | `- DONE foo` | GitHub / CommonMark |
| `{{embed: ((blk-XXXXXX))}}` | `!((blk-XXXXXX))` | Roam |
| `{{[[query]]: foo}}` | `{{query: foo}}` | Roam |
| `^^highlight^^` | (stripped) | Roam |
| `{{video: url}}` and other unknown `{{…}}` | (stripped) | various |
| `id:: <26-char Crockford ULID>` (alone on a line) | (line dropped) | Logseq |
| `[[June 2nd, 2026]]`, `[[Apr 22nd, 2026]]`, `[[2026/04/22]]` | `[[2026-06-02]]` etc. | Roam / mixed |
| 4-space indent | 2-space indent | Roam / Notion export |

Unknown tokens (`{{…}}` and `^^…^^` that aren't outl-native) are stripped on purpose so blocks land clean.
Block properties parsed off the source (`key:: value` indented under a bullet) become `Op::SetProp` on the newly-created node so they converge across devices like every other op.

Date refs `[[…]]` whose inner text parses as a date land as the ISO slug outl uses for journals.
Supported forms: long month (`June 2nd, 2026`), short month (`Apr 22nd, 2026`), slashed ISO (`2026/04/22`).
Plain page refs (`[[Avelino]]`) and ambiguous dates (`[[June 2nd]]` without a year) pass through untouched.

The `id::` line strip is strict.
Only 26-character Crockford base32 strings count.
A random 26-character alphanumeric label (e.g.
`id:: IIIILLLLOOOO0000000000000A`) is not a ULID and stays on the page.

Heuristic: when no line is either a bare `-` or starts with `- ` (after leading whitespace), the paste is treated as plain text.
The clipboard payload is spliced into the current block at the caret, no tree conversion.
The bare `-` form matches the parser, which treats a lone `-` on a line as an empty bullet.

Caret offsets in the mobile client are converted from UTF-16 code units (what `textarea.selectionStart` reports) into Unicode codepoints before the Tauri round-trip, so pasting after an emoji or other supplementary-plane character lands the splice at the right spot.

The orphan log is cleared as items are resolved.
