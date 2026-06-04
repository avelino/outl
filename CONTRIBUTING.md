# Contributing to outl

Thanks for wanting to help.
The short version of how we work is on this page.
The full rules of the game — what reviewers look at, the non-negotiable invariants, the quality bar, what we *won't* block your PR for — live in the docs:

> **[outl.app/docs/contributing](https://outl.app/docs/contributing)**

Read that once before sending a non-trivial PR.
It's the same content the [Copilot reviewer](.github/copilot-instructions.md) runs against, so you won't be surprised by anything in review.

## Quick start

```bash
git clone https://github.com/avelino/outl.git
cd outl
cargo build --workspace
cargo test --workspace
```

You need Rust 1.88+.
`rust-toolchain.toml` pins the exact version, so `rustup` will install it for you on first build.

## Layout

```
crates/
├── outl-core/      Tree CRDT, op log, storage trait — never depends on UI.
├── outl-md/        Parse/render, sidecar, matching, slugify, inline tokens.
├── outl-actions/   UI-agnostic workspace ops (shared by every client).
├── outl-cli/       The `outl` binary.
├── outl-tui/       The terminal UI.
└── outl-mobile/    Tauri 2 mobile app (iOS first).
docs/               User-facing docs; rendered at outl.app/docs.
```

Each crate has its own `CLAUDE.md` describing what it owns and the invariants.
Read it before editing that crate.

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org):

```
feat(tui): visual mode yank
fix(md): preserve sidecar version field on rewrite
docs(sync): walkthrough of the cycle case
refactor(core): extract HLC generator
```

Subject under 70 chars.
Body for the *why* if it's not obvious from the diff.

## Reporting bugs / asking questions

- **Bug:** open an issue with the [bug report template](.github/ISSUE_TEMPLATE/bug_report.md).
  Include repro steps and what you expected.
- **Security issue:** see [SECURITY.md](SECURITY.md).
  Don't open a public issue.
- **Design discussion:** open an issue with the `discussion` label or a draft PR labeled `RFC`.

## License

By contributing, you agree your work is licensed under the [MIT License](LICENSE) (inbound = outbound).
