---
description: Runs fmt + clippy + test + doc on the whole workspace. Use before reporting done.
allowed-tools: Bash(cargo fmt:*), Bash(cargo clippy:*), Bash(cargo test:*), Bash(cargo build:*), Bash(cargo doc:*), Bash(RUSTDOCFLAGS=*:*)
---

Run in sequence and report the result of each step:

1. `cargo fmt --all -- --check` — formatting
2. `cargo clippy --workspace --all-targets -- -D warnings` — lints
3. `cargo test --workspace --all-targets` — tests
4. `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` — docs
   (CI runs this; breaks on intra-doc links to private items, e.g.
   ``[`Foo`]`` where `Foo` is `pub(crate)`. Drop the brackets: `` `Foo` ``.)

If any step fails, **stop** and show the exact output. Do not attempt to fix automatically — only report.

Output format:

```
fmt:     PASS | FAIL (N files)
clippy:  PASS | FAIL (N warnings)
test:    PASS | FAIL (N failures)
doc:     PASS | FAIL (N warnings)

[failure details, if any]
```
