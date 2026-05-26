---
description: Runs the md ↔ ops roundtrip battery for outl-md and triggers markdown-roundtrip-tester for extra validation.
allowed-tools: Bash(cargo test:*)
---

```bash
cargo test -p outl-md
```

If everything passes, invoke the `markdown-roundtrip-tester` agent for additional checks (property tests, sidecar validity, orphan logging).

If it fails, stop and show the exact output.
