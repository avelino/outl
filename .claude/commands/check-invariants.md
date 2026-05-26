---
description: Runs only the tree CRDT invariant test battery in outl-core. Faster than /check, focused on what can break sync.
allowed-tools: Bash(cargo test:*)
---

Run in sequence:

```bash
cargo test -p outl-core --test convergence -- --nocapture
cargo test -p outl-core --test cycle
cargo test -p outl-core --test cycle_chain
cargo test -p outl-core --test concurrent_edit_move
cargo test -p outl-core --test concurrent_delete_edit
cargo test -p outl-core --test late_op
cargo test -p outl-core --test idempotency
cargo test -p outl-core --test fractional_index
cargo test -p outl-core --test large_log
cargo test -p outl-core --test property_based
```

Stop on the first failure. Report the exact failure output.

If they all pass, finish by invoking the `crdt-invariant-checker` agent for extra validation (coverage + static diff).
