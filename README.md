# outl

Local-first outliner. Markdown is the source of truth. Sync that
doesn't corrupt your tree when two devices edit offline.

Inspired by [Roam Research](https://roamresearch.com) and
[Logseq](https://logseq.com). Picks what they got right (graph,
backlinks, daily journal, block-level thinking) and fixes the part
they didn't.

## The bet — a sync that's *provably* correct

This is the differentiator. Everything else builds on it.

Roam keeps your notes on their servers; if two devices edit while
offline, the later write silently wins. Logseq scatters `id::` UUIDs
through your `.md` so its rsync-flavoured "sync" has something to
match on; concurrent moves still lose data. `git`-as-sync produces
conflict markers across nested bullets every time.

outl uses the **[Kleppmann et al. 2022 tree CRDT][paper]** — the
same family of algorithm that backs Automerge and Y.js, adapted
specifically for trees. Two devices that edit a workspace offline
and then sync produce **exactly the same tree**, with **no data
loss**, **without a server**, and **without polluting your
`.md`**. The IDs CRDTs need to operate live in a separate sidecar
file (`.foo.outl`), so the markdown you see is the markdown you
wrote — no `id::` lines, no UUIDs, no HTML comments.

Five formal guarantees, each backed by a test in
[`crates/outl-core/tests/`](crates/outl-core/tests/):

1. **Strong eventual consistency** — same set of ops → same tree, any order.
2. **Commutative after reordering** — late arrivals don't break the result.
3. **Idempotent** — applying an op twice is the same as once.
4. **Tree invariant always holds** — no node ever has two parents, no cycles.
5. **No silent loss** — every op stays in the log, even ones turned into no-ops by cycle detection.

→ **[Sync, done right](docs/sync.md)** walks through *why* Roam,
Logseq and Git fail, then the algorithm step by step.

→ **[Tree CRDT walkthrough](docs/crdt.md)** is the algorithm with code.

P2P sync transport (phase 2) is on [the roadmap](docs/roadmap.md);
the algorithm and the op-log infrastructure are already in.

[paper]: https://martin.kleppmann.com/papers/move-op.pdf

## What's in the box today (0.1.0)

- **TUI** — journal-first, vim-style keys, slash commands (`/`),
  fuzzy switcher (`Ctrl+P`), workspace search, multi-line blocks,
  fenced code blocks, themes, hot-reload on external `.md` edits.
- **Markdown clean as you wrote it** — `title::`, `icon::`, `tags::`
  properties live in plain `key:: value` lines at the top; outline
  is standard CommonMark bullets. No metadata smuggled in.
- **Page icons** — `icon:: 🚀` on a page surfaces everywhere it's
  referenced (header, switcher, backlinks panel, `[[ref]]` inline).
- **Block references and embeds** — `((blk-XXXXXX))` resolves inline
  to the source block's text + page icon. `!((blk-XXXXXX))` expands
  the source block **and its children** read-only below the carrying
  block (each row prefixed with `↳ `, children indented to align with
  the source's text). `Enter` on any handle jumps to the source page
  and lands the cursor on the referenced block. Short, deterministic,
  sidecar-backed handles — the `.md` stays human-typeable. `((` in
  Insert mode pops a fuzzy-match autocomplete; `y r` (or `/refer` /
  `/refer-embed`) copies the current block's handle to the OS
  clipboard for paste anywhere.
- **Code blocks that run** — ` ```lisp / ```js / ```python / ```lua /
  ```rust `, the result lands as a `> **result:**` subblock under
  the source. Re-runs are idempotent. Set `auto-run::` on a block
  and it re-runs whenever you open the page (cache-aware by source
  hash). Powered by [`outl-exec`](crates/outl-exec/) — language
  registry is plugin-shaped, more languages drop in as 80-line
  adapters.
- **Importers** — `outl import logseq` and `outl import roam`
  strip `id::` lines, resolve `((uid))` block refs, slugify
  filenames, seed sidecars.
- **Bench harness** — `cargo bench -p outl-md` measures parse +
  index over synthetic workspaces from 15 files up to 10.500. CI
  runs the smaller tiers on every PR; the 10k-file tier on a
  weekly cron.

## Quick start

```bash
git clone https://github.com/avelino/outl.git && cd outl
cargo build --release

./target/release/outl init ~/notes
./target/release/outl --path ~/notes
```

`outl` (no subcommand) opens the TUI on the workspace and lands on
today's journal. Press `?` for keymap, `:` for the command palette,
`Ctrl+P` to fuzzy-jump to any page.

## Coming from Logseq or Roam?

```bash
outl import logseq ~/path/to/logseq-graph ~/notes
outl import roam ~/Downloads/backup.json ~/notes
```

The importer strips `id::` lines, resolves `((uid))` block refs to
page links, slugifies filenames, and seeds the sidecars. Anything it
can't resolve stays as `((unresolved:UID))` for manual triage.

## Status

**0.1.0 — single-device editor, daily-driver-ready.** Algorithm and
op log infrastructure for sync are in place; the network transport
(phase 2) is the next major piece. Desktop (Tauri, phase 5) and
mobile (uniffi, phase 6) reuse the same `outl-core` + `outl-md`
crates that the TUI already drives — see
[the roadmap](docs/roadmap.md).

## Docs

Want to actually learn how this works?

→ **[docs.outl.app](docs/README.md)** — full GitBook, with the sync
algorithm walked through step by step, the TUI manual, theming, and
contributing notes.

## License

[MIT](LICENSE).
