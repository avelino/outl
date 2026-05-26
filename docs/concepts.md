# Workspace anatomy

A workspace is a directory. Everything else is a convention on top.

## Layout

```
~/notes/                            # workspace root
├── .outl/
│   ├── log.db                      # SQLite op log
│   ├── config.toml                 # workspace identity + settings
│   ├── peers.toml                  # P2P peers (phase 2+)
│   └── orphans.log                 # blocks that lost their ID during external edits
├── pages/
│   ├── avelino.md                  # clean markdown
│   ├── .avelino.outl               # JSON sidecar with stable IDs
│   ├── meu-projeto.md
│   └── .meu-projeto.outl
├── journals/
│   ├── 2026-05-25.md
│   └── .2026-05-25.outl
└── templates/
    └── journal.md                  # applied to new journals
```

## The concepts

### Workspace

The top-level directory. Holds everything. `outl init <path>` creates
one. There's no concept of "switching workspaces" inside the TUI —
each `outl` process is bound to one workspace.

### Page

A named container for an outline. One `.md` file in `pages/` is one
page. The filename is the [slug](#slugs); the human-visible name lives
in the `title::` property.

```markdown
title:: Avelino
type:: person

- works on outl
- can be reached at [[email]]
```

### Journal

A page keyed by date. Files live in `journals/YYYY-MM-DD.md`.
Created automatically when you reference a date — typing `[[2026-05-25]]`
and pressing `Enter` over the link makes the file if it doesn't exist.

The TUI opens on today's journal by default. `[` / `]` navigate days.

### Block

A node in the outline tree. One bullet line:

```markdown
- this is a block
  priority:: high       ← this is a property OF the block above
  - this is a child block
```

Every block has a stable ULID. The ID is **never** in the `.md` —
it's in the sidecar.

### Property

A `key:: value` pair attached to a page (when at the top of the file)
or a block (when nested under one).

```markdown
title:: My project       ← page property
status:: active          ← page property

- objective              ← block
  priority:: high        ← block property
  owner:: [[avelino]]    ← block property
```

Properties drive queries (phase 3) and influence display.

### Tag

A page reference with classification semantics. `#urgent` resolves to
the same underlying file as `[[urgent]]`, but the UI treats them
differently: tags appear in filter sidebars and counts; `[[refs]]`
appear in backlinks.

### Sidecar

A JSON file paired with each `.md`. Stores the stable block IDs and
content hashes:

```json
{
  "version": 2,
  "page_id": "01J...",
  "last_synced_hash": "sha256:...",
  "last_synced_at": "2026-05-25T...",
  "blocks": [
    {"id": "01J...", "line": 3, "indent": 0, "content_hash": "sha256:...", "ref_handle": "blk-r6s4a1"},
    {"id": "01J...", "line": 4, "indent": 1, "content_hash": "sha256:...", "ref_handle": "blk-r6s4a2"}
  ]
}
```

Filename is a dotfile: `pages/avelino.md` ↔ `pages/.avelino.outl`.
Hidden from `ls` by default; gitignorable if you want (but you'd lose
ID stability across devices).

`ref_handle` is the short, stable handle used by inline block
references (`((blk-XXXXXX))`) and embeds (`!((blk-XXXXXX))`). See
[`docs/markdown-format.md`](markdown-format.md#block-refs-and-embeds).

### Op log

The sequence of mutations that produced the current state. Lives in
`.outl/log.db` (SQLite). Every block creation, every move, every
text edit is one row. The tree is a projection over this log.

This is the **source of truth** — if your markdown gets corrupted,
`outl doctor` regenerates the pages from the log.

## Slugs

`[[Avelino]]` → `pages/avelino.md`. The slug rule:

- Lowercase
- Strip accents: `[[São Paulo]]` → `pages/sao-paulo.md`
- Non-alphanumeric → `-`, collapsed
- Empty result → `untitled`

The original name is preserved in `title::`. The autocomplete on `[[`
searches by title (not slug), so users type the way they think and
outl figures out the filename.

## What's NOT in a workspace

- **Trash isn't a directory.** Deleted blocks are moved to a
  `TRASH_ROOT` node in the op log, not deleted from any file.
- **No `archive/` folder.** Archived pages are just pages you stopped
  referencing — they're still in `pages/`.
- **No per-workspace plugins / config beyond `config.toml`.** Plugin
  system is phase 4 ([issue #4](https://github.com/avelino/outl/issues/4)).

## Sharing a workspace

Today: drag the directory between devices and reopen. The sidecar
files carry the IDs, the op log carries the history.

Phase 2: `outl share` generates a pairing ticket, the other device
runs `outl join <ticket>`, P2P sync starts.

Phase 2+ doesn't change the file layout. The wire protocol just keeps
the two directories converging.
