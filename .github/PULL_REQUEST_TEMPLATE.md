## What this PR does

One paragraph.
The *why* first, then the *what*.

## How to verify

```bash
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Plus any feature-specific checks (manual smoke, fixture files, screenshot of TUI state, ...).

## Related issues / docs

Closes #...
Related to #...
Updated docs: `docs/...`

## Anything reviewers should look at carefully

- Is there a CRDT-correctness implication?
  Did `crdt-invariant-checker` pass?
- Is there a markdown-format implication?
  Did `markdown-roundtrip-tester` pass?
- New public API on `outl-core` / `outl-md`?
  Is it documented in the per-crate CLAUDE.md?
- Any change to keymaps in `outl-tui`?
  Updated `docs/tui.md` and the in-app help popup?

## Out of scope for this PR

What this PR is *not* doing, even if related.
Helps reviewers not nudge for scope creep.
