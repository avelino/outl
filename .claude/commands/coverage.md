---
description: Measures coverage in outl-core via cargo-llvm-cov. Focuses on the 4 critical functions (do_op, undo_op, apply_op, creates_cycle) that must be at 100%.
allowed-tools: Bash(cargo llvm-cov:*), Bash(cargo install:*)
argument-hint: "[crate] (default: outl-core)"
---

Target: `${1:-outl-core}`

1. If `cargo llvm-cov` is not installed, install it (`cargo install cargo-llvm-cov --locked`).

2. Run:
   ```bash
   cargo llvm-cov -p ${1:-outl-core} --html --output-dir target/llvm-cov
   cargo llvm-cov -p ${1:-outl-core} --summary-only
   ```

3. Report:
   - Total coverage
   - **Specific coverage of** `tree::do_op`, `tree::undo_op`, `tree::apply_op`, `tree::creates_cycle` (must be 100%)
   - Top 5 functions with the worst coverage

4. If critical coverage < 100%, list the uncovered branches with `cargo llvm-cov report --show-missing-lines -p ${1:-outl-core}`.
