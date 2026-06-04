# Homebrew tap

outl ships pre-built binaries through the same `avelino/outl` repo — no separate `homebrew-outl` repo.
Two surfaces live on `main`:

| Path | What it ships |
|---|---|
| [`/Formula/outl.rb`](../Formula/outl.rb) | GA CLI / TUI, semver tag `vX.Y.Z`. |
| [`/Formula/outl@beta.rb`](../Formula/outl@beta.rb) | Beta CLI / TUI, rebuilt on every push to `main`. |
| [`/Casks/outl-desktop@beta.rb`](../Casks/outl-desktop@beta.rb) | Beta **desktop app** (.dmg), rebuilt on every push to `main`. |

## Install the CLI / TUI

```bash
brew tap avelino/outl https://github.com/avelino/outl
brew install outl@beta     # beta channel — every push to main
# brew install outl        # GA channel — semver tags only
```

The custom-URL form (`brew tap <name> <url>`) tells Homebrew to use this repo as the tap directly, without the usual `homebrew-<name>` naming convention.

`outl` and `outl@beta` install the same `outl` binary and conflict by design.
Switching channels:

```bash
brew unlink outl@beta && brew install outl       # to GA
brew unlink outl  && brew install outl@beta      # to beta
```

## Install the desktop app

```bash
brew tap avelino/outl https://github.com/avelino/outl   # same tap as the CLI
brew install --cask outl-desktop@beta
```

That drops `outl.app` into `/Applications` via the dmg the release workflow builds for macOS arm64 + x86_64.
The cask sits in [`/Casks/outl-desktop@beta.rb`](../Casks/outl-desktop@beta.rb) and is bumped automatically alongside the CLI formula on every push to `main`.

The CLI formula (`outl@beta`) and the desktop cask coexist without conflicts — the formula installs `/usr/local/bin/outl`, the cask installs `/Applications/outl.app`.
Both point at the same workspace on disk if you configure the same path; the op log (`ops/ops-<actor>.jsonl`) reconciles edits made from either surface.

### First-launch Gatekeeper warning

The desktop dmg is **unsigned** today.
On the first launch macOS Gatekeeper refuses the app.
Two ways through:

- **Right-click** `outl.app` in `/Applications` and choose **Open**.
  Confirm once; subsequent launches work.
- Or drop the quarantine attribute from the terminal:
  ```bash
  xattr -dr com.apple.quarantine /Applications/outl.app
  ```

Signing + notarisation will land with the first GA release (the cask's `caveats` block reminds the user of this on every install until then).

### Updating

```bash
brew update
brew upgrade --cask outl-desktop@beta
```

The `livecheck` block in the cask points Homebrew at the GitHub releases feed, so `brew outdated` knows when a new beta dmg lands.

## How updates happen

The release workflow ([`/.github/workflows/release.yml`](../.github/workflows/release.yml)) has an `update_tap` job that runs after `publish_release` succeeds on a prerelease.
It does four things:

1. Checks out `main`.
2. Downloads every `.sha256` sidecar from the GitHub release (CLI tarballs **and** desktop dmgs).
3. Uses `sed` to bump the version + sha lines in both files in place:
   - **`Formula/outl@beta.rb`** — `version` plus three `sha256` lines tagged `# anchor: macos-arm64`, `# anchor: macos-x64`, `# anchor: linux-x64`.
   - **`Casks/outl-desktop@beta.rb`** — `version` plus two `sha256` lines tagged `# anchor: macos-arm64`, `# anchor: macos-x64`.
4. Commits the bumped formula + cask back to `main` with `[skip ci]` in the message — that prevents the commit from re-triggering the release workflow itself.

The version comes from the `prepare` job, which reads `workspace.package.version` out of `Cargo.toml` and appends `-beta.<run_number>`.
So the only source of truth for the base version is `Cargo.toml`; nothing is hardcoded in the formula.

`Formula/outl@beta.rb` and `Casks/outl-desktop@beta.rb` are both **committed with bootstrap placeholders** (`version "0.0.0"`, zeroed SHAs).
The first beta release after each lands bumps it to real values.
Until then, `brew install outl@beta` and `brew install --cask outl-desktop@beta` both 404 on the download URL.

`Formula/outl.rb` and `Casks/outl-desktop.rb` (the GA channels) don't exist yet.
When the first non-prerelease tag (`vX.Y.Z` with no `-beta`) ships, bump them by hand the first time; after that we can add a GA-flavored `update_tap` job if it's worth automating.

## Authentication

The job uses the default `GITHUB_TOKEN` (with `contents: write` permission scoped to the job).
No PAT, no extra secret to manage.

## Troubleshooting

**Tap update commits but `brew install` still pulls the old version** — run `brew update` first; Homebrew caches tap state.

**`brew install outl@beta` fails with `SHA256 mismatch`** — the formula on `main` drifted from the release asset.
Usually means someone re-uploaded an asset on the GitHub release.
Re-trigger the failed release via `workflow_dispatch` and `update_tap` will re-render with the current SHAs.

**A manual edit on `Formula/*.rb` got overwritten** — by design.
The job templates the whole file on every release.
If you need to change the formula structure (add a platform, change the test, etc), edit the heredoc in `release.yml`.
The next release picks it up.

**Renaming the binary or adding a second binary** — `bin.install "outl"` in the heredoc only ships the `outl` executable.
If/when outl grows additional binaries (e.g.
`outl-tui` as a separate binary), update the heredoc accordingly.
