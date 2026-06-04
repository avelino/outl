# Contributing & code review

This is the canonical guide for contributing to outl.
It spells out **the rules of the game** — what every PR is measured against during review, why each rule exists, and what is explicitly *not* a reason to block your PR.

The [root `CONTRIBUTING.md`](https://github.com/avelino/outl/blob/main/CONTRIBUTING.md) on GitHub is a short pointer (clone, build, commit format, license) that links back here.
Everything substantive lives on this page.

We want outl to be a project where you can show up, read this page, and know exactly what you're walking into.
No tribal knowledge, no hidden quality bar.

The same priorities are encoded for automated review in [`.github/copilot-instructions.md`](https://github.com/avelino/outl/blob/main/.github/copilot-instructions.md).
If you ever feel a reviewer comment came out of nowhere, it almost certainly traces back to this page or that file.

---

## Philosophy

outl is a sync engine first and a notes app second.
The CRDT layer underneath has to be **correct**, because the cost of a sync bug is silently-corrupted user data — exactly the failure mode Roam and Logseq ship and exactly the one we exist to not repeat.

That shapes how we review.
We are strict about the things that touch correctness, scalability, and the public contract.
We are deliberately relaxed about the things that don't.

Concretely:

- **Correctness > cleverness.** The CRDT follows [Kleppmann et al. 2022](https://martin.kleppmann.com/papers/move-op.pdf) literally.
  A "smarter" version is a regression.
- **Simple > abstract.** We invoke the Rule of Three before introducing a trait or generic.
  One implementation gets a concrete type.
- **Real problems > hypothetical ones.** A refactor needs to unblock something or pay down a named debt. "Cleaner code" alone is not a merge reason.
- **The user's `.md` is sacred.** It belongs to the user.
  We never write metadata into it, even when it would make our life easier.

If you read this and think "they're going to be brutal in review" — not the goal.
The goal is for you to read this *before* writing the patch, so the review goes "ship it" instead of "back to draft".

---

## Before opening a PR

Reviewers (human and automated) check the PR description **before** the diff.
If the description doesn't answer these, the PR gets a top-level comment requesting changes and the line-level review is deferred.

1. **Link an issue.** `Closes #N`, `Fixes #N`, or `Related to #N`.
   - For typo fixes, doc-only changes, or dependency bumps with a changelog link, you can skip this.
   - For everything else, an issue is the place to debate scope and approach.
     PRs are the place to debate implementation.
2. **State the problem in plain language.** One paragraph.
   The user-facing problem first, then the technical approach.
3. **For a refactor, answer "why now?".** "The code is cleaner" is not enough — name the concrete thing this unblocks or the debt it pays down.
4. **For a bug fix, describe the bug.** Steps to reproduce, observed behaviour, expected behaviour.
   Ideally a failing test that the patch turns green.
5. **For a feature, point at the roadmap or an approved issue.** `docs/roadmap.md` is public; if a feature isn't on it or in an accepted issue, open the issue first.

The template at [`.github/PULL_REQUEST_TEMPLATE.md`](https://github.com/avelino/outl/blob/main/.github/PULL_REQUEST_TEMPLATE.md) walks you through this.
Use it.

---

## Non-negotiable invariants

These are blockers in review.
A PR that violates one will not merge regardless of how clean the code looks.
You're welcome to disagree with any of them — the path for that is an issue, not a PR.

### 1. The op log is the source of truth

Every mutation flows through `Op` → `Workspace::apply` → log.
The materialized tree and the `.md` files are projections of the log.

**Why:** sync replays the log.
If a mutation skips the log, the second device never sees it.
If you write to `.md` to "fix" state, you've created a divergence the next sync round won't catch.

### 2. Markdown stays 100% clean

No `id::` lines.
No inline UUIDs.
No HTML comments carrying state.
Stable IDs live in the `.outl` sidecar (a JSON file next to the `.md`).

**Why:** the `.md` belongs to the user.
They open it in vim, ship it to a wiki, paste it into a chat.
The day we make their file ugly to serve our internal needs is the day we became Logseq.

The sidecar isn't a dotfile, by the way — iCloud silently drops dotted paths during cross-device sync.

### 3. The CRDT matches the paper

`do_op`, `undo_op`, `apply_op`, and `creates_cycle` in `outl-core/src/tree.rs` follow Kleppmann et al. 2022 literally.
These four functions carry a **100% line and branch coverage rule**.

**Why:** the paper has a formal correctness proof.
Any deviation — even one that looks like a perf win — moves us off the proof, and we have no other way to argue we converge.
A new branch without a test is a blocker.

### 4. A cycle-creating move is a deterministic no-op

If a `Move` op would create a cycle, the materialized tree ignores it.
**But the op still goes into the log.**

**Why:** the log is total-ordered, and replaying it on every device must produce the same materialized tree.
If device A drops the op and device B keeps it, a future re-parenting op that *would* have been valid now diverges.
Keep the op, no-op the effect.

### 5. `Storage` is a trait, not a struct

`outl-core` does not import `rusqlite`, doesn't write JSON directly, doesn't know about files.
Everything goes through `dyn Storage`.
Today the only persistent implementation is `JsonlStorage`; `MemoryStorage` is used in tests.

**Why:** ChronDB ([issue #1](https://github.com/avelino/outl/issues/1)) is the next backend, and we've already paid the cost of removing SQLite in 0.5.0.
Locking the core to a concrete backend reintroduces that cost.
A second persistent backend doesn't land without an RFC issue first.

### 6. Delete is `Move(node, TRASH_ROOT)`

There is no physical removal of a node.
Delete moves it under a sentinel root.

**Why:** the algorithm in the paper is defined over a tree where nodes are never removed.
Physical deletion would either break undo, break replay, or both.

### 7. Convergent state goes through the op log

If two actors can disagree about a value and you want them to reconcile, model it as an `Op`.
Writing convergent state into a shared file — including the sidecar — bypasses the CRDT and loses concurrent writes silently.

**Why:** every shared file is last-write-wins under any file-system transport (iCloud, Syncthing, Dropbox).
Per-actor `ops-<actor>.jsonl` files turn that into a non-issue: each device writes its own file, the CRDT merges on replay, with HLC ordering for determinism.
The canonical example is `Op::SetCollapsed` for fold state.

The sidecar is reserved for **structural matching metadata only** — ids, position, content hash, ref handle.
Not a sync surface.

### 8. Layering

- `outl-core` never imports a UI or CLI crate.
- `outl-actions` is the shared workspace-mutation surface.
  Every client (`outl-tui`, `outl-mobile`, future shells) calls into it.
- A client reimplementing logic that belongs in `outl-actions` is a blocker.
  Reviewers will point at the existing function and ask you to call or extend it.

**Why:** the mobile app and the TUI must behave identically for the same semantic operation.
If toggling a TODO in the TUI emits different ops from toggling one in the mobile app, users see ghosts on sync.
One source of truth per concept.

### 9. No reintroduction of SQLite, rusqlite, or binary log formats

Cross-device sync depends on per-actor append-only JSONL.

**Why:** 0.5.0 removed SQLite specifically to enable iCloud / Syncthing / shared-FS workflows.
Binary formats and DB files don't merge across those transports.

### 10. Settled decisions are off-limits in a PR

ULID for IDs, `uhlc` for time, MIT license, JSONL-per-actor, Tauri for mobile, iCloud as v0 transport, `comrak` for markdown.
These were debated and chosen in phase 0.
The full table is in the root [`CLAUDE.md`](https://github.com/avelino/outl/blob/main/CLAUDE.md#decisions-you-dont-get-to-revisit) and [`CONTRIBUTING.md`](https://github.com/avelino/outl/blob/main/CONTRIBUTING.md#decisions-you-dont-get-to-revisit).

**Why:** these are foundational.
Changing one ripples through every crate.
The path to revisit is an issue with rationale, not an inline review comment.

---

## What reviewers will look at

### Rust quality

The bar is **production code**, not "it compiles":

- **No `.unwrap()` outside `#[cfg(test)]`.** Use `.expect("explicit reason")` or propagate with `?`.
  The `expect` message must name the invariant being asserted — "should not fail" is not a reason.
- **No `.unwrap_or_default()` that masks an error path.** If the default would be a silent data-loss bug, return the error.
- **No `unsafe` in `outl-core`** without a `// SAFETY:` comment naming the invariants the caller upholds.
- **`thiserror` in libraries** (`outl-core`, `outl-md`, `outl-actions`) so callers can match on variants.
  `anyhow` is for binary boundaries (`outl-cli`, `outl-tui`).
- **Async hygiene.** No blocking calls inside `async fn`.
  No `Mutex` / `RwLock` held across `.await`.
- **API ergonomics.** Public API prefers `&str` over `String`, `&[T]` over `Vec<T>` when ownership isn't needed.
- **Public API changes are documented.** If you change a function or type that other crates use, update the doc-comment and the per-crate `CLAUDE.md`.

### Performance — hot paths only

We care about performance in the paths that actually run a lot.
Allocations in setup, error paths, or one-shot CLI commands are not worth a review comment.

The hot paths in outl:

- `outl_core::tree` — every op apply, every materialized-tree walk.
- `outl_core::log` — every append, every replay (boot, sync pull).
- `outl_md::parse` / `outl_md::render` — every `.md` read/write, every TUI buffer refresh.
- `outl_md::index` — backlink index rebuild, scales with workspace size.
- `outl_tui` render loop — runs on every keystroke.
- `outl_actions::SyncEngine` work loop — runs on every file event.

In those paths, reviewers will flag:

- `.clone()` on `String`, `Vec`, or large structs where a borrow works and the clone is per-call (not one-time setup).
- `.to_string()` / `format!()` when `&str` or deferred `Display` would do.
- `Vec::new()` + repeated `push` in a loop where capacity is knowable (`Vec::with_capacity`).
- Re-parsing the same markdown or re-walking the same subtree on every keystroke — propose caching with a clear invalidation story.
- Big-O regressions on tree ops or backlink computation.

If you're unsure whether code is on a hot path, ask in the PR — we'd rather a question than a guess.

### Architecture, scalability, extensibility

This is where review earns its keep:

- **Reuse-first.** Before adding a helper, grep upstream crates (`outl-core` → `outl-md` → `outl-actions`) for what already does the same thing.
  Two implementations of "compute backlinks" eventually disagree, and the user is the one who notices.
  The fix is to call or extend the existing function.
- **New `Op` variants come with the full checklist.** A new variant touches `apply_op`, `undo_op` (the inverse must be exact), the sidecar serializer, the markdown projection, replay tests, and per-crate docs.
  See `/new-op` for the walkthrough.
- **Trait surface stays implementable.** A `Storage` method that assumes file semantics (paths, flock) locks out ChronDB.
  Push back on that.
- **Sidecar / op-log format changes need a migration story.** Existing workspaces on disk must still load.
  Either the change is backward-compatible (new optional field) or the PR ships a versioned migration.
- **File size discipline.** Past 600 lines, plan a split by responsibility.
  Past 900, refactor before the next non-trivial edit.
  The `refactor-architect` agent will propose a split.
- **Premature abstraction is rejected.** A new trait or generic with one impl and no named second caller doesn't merge.
  Rule of Three — concrete first, abstract on the third caller.

### Simplicity

- A new dependency for two functions of `std` code needs a real justification — crate size, maintenance status, transitive deps, licence.
- A configuration knob without a concrete user asking for it doesn't merge.
  Defaults that are right for the 90% case beat knobs nobody tunes.
- Cleverness loses to readability.
  If a reviewer has to run code in their head to understand it, the next maintainer pays.

### Testing

- **Bug fix without a regression test → blocker.** The test must fail on `main` and pass with the patch.
- **Critical path touched without coverage proof.** Run `/coverage outl-core` (or the relevant crate) and paste the result. 100% coverage on the four CRDT functions and on `outl_md::reconcile_md` is non-negotiable.
- **Tests assert behaviour, not implementation.** A test that breaks on any refactor is a maintenance tax.
  Assert against the public surface (op log contents, materialized tree shape, rendered markdown), not internal helpers.
- **Integration tests use the real backend.** Mocked storage in a path that should hit `JsonlStorage` hides exactly the bugs that matter.

---

## What we will *not* block your PR for

These are noise.
If a reviewer comments on one of these without a behavioural reason, push back politely:

- Style and formatting — `cargo fmt`, `cargo clippy -D warnings`, and rustdoc warnings handle them in CI.
- `if let Some(x) = y` vs `match y { Some(x) => ... }` with no behavioural difference.
- "I would have named this differently" without a concrete clarity win.
- Speculation about a future architecture that nobody asked for.
- Re-litigating settled decisions (ULID, MIT, JSONL, Tauri, etc.).
- Adding TODOs the author already acknowledged in "Out of scope".

---

## What we are not building yet

The project is in Phase 0–1.
Reviewers will push back on PRs that try to introduce these without an explicit issue and a roadmap entry:

- P2P sync transport (`iroh`) — iCloud is the v0 transport.
- Query DSL (`{{query: ...}}`).
- Tauri desktop shells beyond the existing mobile crate.
- Plugin system (`rhai`).
- `ChronDbStorage` backend ([issue #1](https://github.com/avelino/outl/issues/1)).
- Android mobile build (iOS only today).
- Per-page op log shards (Phase A in `docs/sync.md`, only when the workspace hits 10k pages).

If your PR genuinely belongs to one of these, link the tracking issue and the relevant `docs/roadmap.md` section.

---

## Disagreeing with a review

We want this project to be inclusive, which means we want to hear when you think a reviewer is wrong.
The expectation is:

- Disagree on the substance, in the thread.
  Cite the invariant or the decision you think the reviewer is misapplying.
- For settled decisions ("ULID is wrong for us"), open an issue with the rationale — those debates belong in design discussions, not in a code-review thread.
- For new opinions ("this trait surface is over-abstracted"), the PR itself is the right place.
  Reviewers will engage.

The job of a reviewer here is to protect the invariants and the user's data, not to enforce taste.
If a comment crosses that line, say so.

---

## The agents that help

We use specialised review agents (configured in `.claude/agents/`):

- **`crdt-invariant-checker`** runs after any change in `outl-core/src/{tree,log,op}.rs`.
  Validates convergence, idempotency, cycle handling, coverage.
- **`paper-verifier`** compares the Rust against the paper line by line after edits to the four critical CRDT functions.
- **`markdown-roundtrip-tester`** runs after `outl-md/` changes.
  Validates roundtrip stability and matching invariants.
- **`refactor-architect`** is invoked when a file crosses the 600-line threshold.
  Proposes a split by responsibility.
- **`doc-keeper`** runs at the end of every feature that changes public API, markdown syntax, TUI shortcut, sidecar format, or user-observable behaviour.

If you're using Claude Code locally, these run automatically.
If not, they run in CI on the PR.

---

## Where to look next

- Root [`CLAUDE.md`](https://github.com/avelino/outl/blob/main/CLAUDE.md) — project-wide invariants and conventions.
- Per-crate `CLAUDE.md` (e.g.
  [`crates/outl-core/CLAUDE.md`](https://github.com/avelino/outl/blob/main/crates/outl-core/CLAUDE.md)) — invariants specific to that crate.
- [`docs/architecture.md`](architecture.md) — design decisions.
- [`docs/crdt.md`](crdt.md) — the algorithm.
- [`docs/markdown-format.md`](markdown-format.md) — the markdown dialect and sidecar spec.
- [`docs/storage.md`](storage.md) — the `Storage` trait and roadmap.
- [`docs/roadmap.md`](roadmap.md) — what's planned, what's done.

Welcome aboard.
