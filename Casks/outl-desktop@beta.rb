# This file is maintained by `.github/workflows/release.yml`.
# Every push to `main` runs the release workflow, which bumps the
# `version` line (computed from `Cargo.toml` + the workflow run
# number) and the two `sha256` lines below in place. The
# `# anchor:` comments are how the workflow finds the right lines —
# do not remove them.
#
# The dmg shipped here is **unsigned**. On first launch macOS
# Gatekeeper will refuse the app; users can right-click → "Open"
# (or run `xattr -dr com.apple.quarantine /Applications/outl.app`)
# to dismiss the warning. Once we wire an Apple Developer ID +
# notarisation (release.yml step pending), this caveat goes away.
cask "outl-desktop@beta" do
  version "0.0.0"
  sha256 arm:   "0000000000000000000000000000000000000000000000000000000000000000", # anchor: macos-arm64
         intel: "0000000000000000000000000000000000000000000000000000000000000000"  # anchor: macos-x64

  arch arm: "arm64", intel: "x64"

  url "https://github.com/avelino/outl/releases/download/v#{version}/outl-desktop-macos-#{arch}.dmg"
  name "outl Desktop"
  desc "Local-first outliner with CRDT sync (desktop beta — every push to main)"
  homepage "https://outl.app"

  livecheck do
    url :url
    strategy :github_latest
  end

  app "outl.app"

  # We share the `outl` binary name with the CLI / TUI formula on
  # /usr/local/bin. The cask installs `outl.app` to /Applications,
  # which doesn't collide on PATH, but a future "open outl" CLI
  # alias would. Flag the relationship so users running both stay
  # aware.
  conflicts_with cask: "outl-desktop"

  caveats <<~EOS
    The dmg is **unsigned**. macOS Gatekeeper will refuse the app on
    first launch. Right-click the .app in /Applications and choose
    "Open" once; subsequent launches work normally.

    Signing + notarisation lands together with the first GA release.
  EOS

  zap trash: [
    "~/Library/Application Support/app.outl.desktop",
    "~/Library/Preferences/app.outl.desktop.plist",
    "~/Library/Caches/app.outl.desktop",
    "~/Library/Logs/outl",
  ]
end
