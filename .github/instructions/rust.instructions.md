---
applyTo: "**/*.rs"
---

## 3. Rust quality bar

Comment when the diff introduces any of the following.
Skip when the existing surrounding code already does it (that's a separate cleanup).

- **`.unwrap()` outside `#[cfg(test)]`** — require `.expect("explicit reason")` or `?` propagation.
  The `expect` message must name the invariant being asserted, not just "should not fail".
- **`.unwrap_or_default()` masking an error path** — if the default is a silent data-loss bug, flag it.
- **`unsafe` in `outl-core`** without a `// SAFETY:` comment naming the invariants the caller relies on.
- **`anyhow` in a library crate** (`outl-core`, `outl-md`, `outl-actions`).
  Libraries use `thiserror` so callers can match on variants.
  `anyhow` is only OK at binary boundaries (`outl-cli`, `outl-tui`).
- **`Box<dyn Error>` as a public return type** — same reason.
- **`String` where `&str` works**, **`Vec<T>` where `&[T]` works**, **owned arg where borrowed works** — but only in public APIs and hot paths; do not bikeshed this on internal helpers.
- **`async fn` with a blocking call inside** (`std::fs`, `std::thread::sleep`, large CPU loop without `spawn_blocking`).
- **Holding a `Mutex`/`RwLock` across an `.await`** — deadlock waiting to happen.
- **Public API change on `outl-core`, `outl-md`, or `outl-actions` without doc-comment update** — the per-crate `CLAUDE.md` should also reflect it.

Skip these (CI / fmt / clippy handle them):

- Import ordering, line width, brace placement.
- Naming conventions clippy already lints.
- `mod` declaration order.

---

## 4. Performance — hot paths only

Comment on performance only when the code is on a path that runs frequently or scales with workspace size.
**Do not flag allocations in setup, error paths, or one-shot CLI commands.**

Paths that are hot in outl:

- `outl_core::tree` — every op apply, every materialized-tree walk.
- `outl_core::log` — every append, every replay (workspace boot, sync pull).
- `outl_md::parse` / `outl_md::render` — every `.md` read/write, every TUI refresh of a buffer.
- `outl_md::index` — backlink index rebuild scales with workspace size.
- `outl_tui` render loop — runs on every keystroke.
- `outl_actions::SyncEngine` work loop — runs on every file event.

In those paths, flag:

- `.clone()` on `String`, `Vec`, or large structs where a borrow would work, and the clone is per-call (not one-time setup).
- `.to_string()` / `format!()` when the caller only needs `&str` or `Display` deferral.
- `Vec::new()` followed by repeated `push` inside a loop where capacity is knowable (`Vec::with_capacity`).
- `HashMap` for small fixed key sets where a `match` or array would do.
- Re-parsing the same markdown / re-walking the same subtree on every keystroke — propose caching with a clear invalidation story.
- Big-O regressions on tree ops or backlink computation.
  Walk the algorithm in the comment.

If unsure whether it's a hot path, ask in the comment — do not assert.

---

## 7. Testing bar

- **Bug fix without a regression test → blocker.**
  The test must fail on `main` and pass with the patch.
  Ask for it explicitly.
- **Critical path touched without coverage proof.** `outl_core::tree::{do_op, undo_op, apply_op, creates_cycle}` and `outl_md::reconcile_md` carry 100% line and branch coverage rules.
  New branches need new tests.
  Ask the author to run `/coverage outl-core` (or the relevant crate) and paste the result.
- **Test asserts implementation, not behaviour.**
  A test that breaks on any refactor is a maintenance tax.
  Suggest asserting against the public surface (op log contents, materialized tree shape, rendered markdown), not internal helpers.
- **Mocked storage in an integration test that should hit `JsonlStorage`.**
  Real-file integration is cheap; mocks hide the bugs that matter.
- **`#[ignore]` or `#[should_panic]` added without a comment** explaining the invariant being protected.

---
