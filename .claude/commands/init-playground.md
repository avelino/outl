---
description: Creates a test outl workspace at ./playground and generates fixture data (a few pages + journals) for manual testing.
allowed-tools: Bash(cargo run:*), Bash(mkdir:*), Bash(rm:*), Bash(ls:*), Bash(find:*)
---

Set up a test workspace for manual smoke testing.

```bash
rm -rf ./playground
cargo run --bin outl -- init ./playground
```

Confirm the structure created:

```bash
find ./playground -type f | head -20
```

Expected:
- `./playground/.outl/log.db`
- `./playground/.outl/config.toml`
- `./playground/pages/` (empty)
- `./playground/journals/<today>.md` (created if journal-on-init is enabled)
- `./playground/templates/journal.md`

If anything is missing, report what is missing.
