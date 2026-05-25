# Contributing to outl

Thanks for wanting to help. This document explains how to find your way
around the repo, what the bar is for a merged PR, and where to talk
about bigger changes before sending code.

## Quick start

```bash
git clone https://github.com/avelino/outl.git
cd outl
cargo build --workspace
cargo test --workspace
```

You need Rust 1.88+. `rust-toolchain.toml` pins the exact version, so
`rustup` will install it for you on first build.

## Layout

```
crates/
├── outl-core/    Tree CRDT, op log, storage trait — never depends on UI.
├── outl-md/     Parse/render, sidecar, matching, slugify, inline tokens.
├── outl-cli/    The `outl` binary (subcommands + workspace orchestration).
└── outl-tui/    The terminal UI; reused by the `outl` binary.
docs/            User-facing docs; rendered with GitBook.
```

Each crate has its own `CLAUDE.md` describing what it owns and the
invariants. Read it before editing that crate. The root `CLAUDE.md`
covers project-wide conventions.

## The bar

Every PR has to land green:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

The PostToolUse hook in `.claude/settings.json` runs fmt + clippy on
the touched crate after every edit if you're using Claude Code. The
CI workflow runs the full triple on every PR.

### Critical-path coverage

Some code has a 100% coverage rule:

- `outl_core::tree::{do_op, undo_op, apply_op, creates_cycle}` —
  the heart of the CRDT. Adding a branch without a test fails review.
- `outl_md::reconcile_md` — silent block loss is a P0. Every code
  path that could delete a block needs a test that asserts the orphan
  is logged first.

The agents in `.claude/agents/` (crdt-invariant-checker, paper-verifier,
markdown-roundtrip-tester) are there to enforce this — invoke them
on PRs that touch the critical files.

## Decisions you don't get to revisit

These were settled in phase 0. Don't unilaterally pivot in a PR; open
an issue if you think one is wrong:

| Decision | Why |
|----------|-----|
| `.md` stays clean (no `id::`, no UUIDs inline) | The Logseq mistake. IDs live in `.outl/<file>.outl` sidecars. |
| Op log is source of truth | `.md` and the materialized tree are projections. |
| `Storage` is a trait, not a struct | So ChronDB (issue #1) can slot in without touching the CRDT. |
| ULID for IDs | Lexicographically sortable, no central server. |
| MIT license | Simple, no patent-grant gymnastics. |

## Workflow

1. **Open an issue first for non-trivial changes.** Bug reports and
   small fixes can go straight to PR; anything that touches the CRDT,
   the storage trait, or the markdown dialect should be discussed
   first.
2. **Branch off `main`.** Name it after what you're doing: `fix/lock-file-leak`,
   `feat/visual-mode-yank`, `docs/sync-walkthrough`.
3. **One concern per PR.** A refactor + a bug fix in the same PR
   makes review hard.
4. **Run the local triple before pushing.** CI will catch you if
   you don't, but it wastes the round-trip.
5. **PR description follows the template** (`.github/PULL_REQUEST_TEMPLATE.md`).
   Tell us what changed and why; how to verify it; what's still open.

## Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org):

```
feat(tui): visual mode yank
fix(md): preserve sidecar version field on rewrite
docs(sync): walkthrough of the cycle case
refactor(core): extract HLC generator
```

The type tags help the changelog. Subject under 70 chars. Body for
the *why* if it's not obvious from the diff.

## What to work on

- **Open issues labeled `good first issue`** — small wins to get
  familiar.
- **The roadmap** (`docs/roadmap.md`) lists everything tracked
  publicly. Issues #1–#4 cover the big future pieces (ChronDB
  backend, Tauri desktop, mobile, plugin system).
- **Bug reports** — file one if you have repro steps, attach one if
  you can write a test.

## Reporting bugs / asking questions

- Bug: open an issue with the template. Include repro steps and what
  you expected.
- Security issue: see [SECURITY.md](SECURITY.md). Don't open a
  public issue.
- Design discussion: open an issue with the `discussion` label or
  a draft PR labeled `RFC`.

## License

By contributing, you agree your work is licensed under the
[MIT License](LICENSE) (inbound = outbound).
