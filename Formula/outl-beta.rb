# This file is maintained by `.github/workflows/release.yml`.
# Every push to `main` runs the release workflow, which bumps the
# `version` line (computed from `Cargo.toml` + the workflow run
# number) and the three `sha256` lines below in place. The `# anchor:`
# comments are how the workflow finds the right lines — do not remove
# them.
#
# Values committed here are bootstrap placeholders: `version "0.0.0"`
# and zeroed SHAs make `brew install outl-beta` fail loudly until the
# first release fires. They become real on the next push to `main`.
class OutlBeta < Formula
  desc "Local-first outliner with CRDT sync (beta channel — every push to main)"
  homepage "https://outl.app"
  version "0.7.0-beta.92"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/avelino/outl/releases/download/v#{version}/outl-macos-arm64.tar.gz"
      sha256 "5b00313b3e679f70076ace53e96f88b39130518a1ebcccb5b59ae902409f99c5" # anchor: macos-arm64
    end
    on_intel do
      url "https://github.com/avelino/outl/releases/download/v#{version}/outl-macos-x64.tar.gz"
      sha256 "5f90942518a7537076d19aa49f44610afddeb96e1999167915c2b51a528c16fa" # anchor: macos-x64
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/avelino/outl/releases/download/v#{version}/outl-linux-x64.tar.gz"
      sha256 "44cc5a03c427711a4767beffa7d0444e365b215a6977367a1ed05e77870ac4f2" # anchor: linux-x64
    end
  end

  # Beta and stable share the same `outl` binary name. Refuse to install
  # both side-by-side — `brew unlink outl` (or `outl-beta`) before
  # switching channels.
  conflicts_with "outl", because: "both install the `outl` binary"

  def install
    bin.install "outl"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/outl --version")
  end
end
