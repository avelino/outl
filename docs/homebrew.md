# Homebrew tap

outl ships pre-built binaries through the same `avelino/outl` repo —
no separate `homebrew-outl` repo. The formulas live in
[`/Formula`](../Formula/) on `main`:

- **`outl`** — latest GA release (semver tag `vX.Y.Z`).
- **`outl@beta`** — latest beta (rebuilt on every push to `main`).

## Install

```bash
brew tap avelino/outl https://github.com/avelino/outl
brew install outl@beta     # beta channel — every push to main
# brew install outl        # GA channel — semver tags only
```

The custom-URL form (`brew tap <name> <url>`) tells Homebrew to use
this repo as the tap directly, without the usual `homebrew-<name>`
naming convention.

`outl` and `outl@beta` install the same `outl` binary and conflict
by design. Switching channels:

```bash
brew unlink outl@beta && brew install outl       # to GA
brew unlink outl  && brew install outl@beta      # to beta
```

## How updates happen

The release workflow ([`/.github/workflows/release.yml`](../.github/workflows/release.yml))
has an `update_tap` job that runs after `publish_release` succeeds
on a prerelease. It does four things:

1. Checks out `main`.
2. Downloads the `.sha256` sidecars from the GitHub release.
3. Uses `sed` to bump four lines of `Formula/outl@beta.rb` in place:
   the `version` and the three `sha256` lines (each tagged with a
   stable `# anchor: <arch>` comment).
4. Commits the bumped formula back to `main` with `[skip ci]` in the
   message — that prevents the commit from re-triggering the release
   workflow itself.

The version comes from the `prepare` job, which reads
`workspace.package.version` out of `Cargo.toml` and appends
`-beta.<run_number>`. So the only source of truth for the base
version is `Cargo.toml`; nothing is hardcoded in the formula.

`Formula/outl@beta.rb` is **committed with bootstrap placeholders**
(`version "0.0.0"`, zeroed SHAs). The first beta release after this
lands will bump it to real values. Until then, `brew install
outl@beta` will fail with a 404 on the download URL.

`Formula/outl.rb` (the GA formula) doesn't exist yet. When the first
non-prerelease tag (`vX.Y.Z` with no `-beta`) ships, bump it by hand
the first time; after that we can add a GA-flavored `update_tap`
job if it's worth automating.

## Authentication

The job uses the default `GITHUB_TOKEN` (with `contents: write`
permission scoped to the job). No PAT, no extra secret to manage.

## Troubleshooting

**Tap update commits but `brew install` still pulls the old
version** — run `brew update` first; Homebrew caches tap state.

**`brew install outl@beta` fails with `SHA256 mismatch`** — the
formula on `main` drifted from the release asset. Usually means
someone re-uploaded an asset on the GitHub release. Re-trigger the
failed release via `workflow_dispatch` and `update_tap` will
re-render with the current SHAs.

**A manual edit on `Formula/*.rb` got overwritten** — by design.
The job templates the whole file on every release. If you need to
change the formula structure (add a platform, change the test, etc),
edit the heredoc in `release.yml`. The next release picks it up.

**Renaming the binary or adding a second binary** — `bin.install
"outl"` in the heredoc only ships the `outl` executable. If/when
outl grows additional binaries (e.g. `outl-tui` as a separate
binary), update the heredoc accordingly.
