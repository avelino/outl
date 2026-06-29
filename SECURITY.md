# Security Policy

## Reporting a vulnerability

Please do **not** open a public GitHub issue for security problems.

Email **avelinorun@gmail.com** with the subject line `[outl security] <short description>`.
Include:

- A description of the problem.
- The version / commit you reproduced it on.
- Steps to reproduce, ideally a minimal repro repo or workspace.
- Your assessment of severity (information disclosure, data loss, crash, RCE, etc.) and impact.

You can expect:

- An acknowledgment within **3 days**.
- A coordinated disclosure timeline (typically 30 to 90 days depending on severity and complexity of the fix).
- Credit in the release notes if you'd like — say so in your report.

## What's in scope

outl is a local-first outliner.
The interesting attack surface is narrow:

- **Workspace data corruption**: a crafted `.md`, `.outl` sidecar, or `log.db` that can cause silent data loss or process crash on open.
  **In scope.**
- **Path traversal**: imports (`outl import logseq`, `outl import roam`) writing outside the destination workspace.
  **In scope.**
- **Markdown injection**: rendering of `[[refs]]` / `#tags` / etc. in the TUI should never let untrusted input escape into terminal control sequences.
  **In scope.**
- **Workspace lock bypass**: tricks that let two `outl` processes attach to the same workspace simultaneously.
  **In scope.**
- **CRDT correctness regressions**: a sequence of ops that breaks the convergence / no-loss / no-cycle guarantees.
  **In scope** — treat these the same as security issues even when they look like pure bugs.
  Silent data loss is the worst-case outcome.

## What's out of scope

- **Compromised local machine**: outl trusts the operator.
  If your machine is owned, your workspace is owned.
  We can't help.
- **P2P sync transport** (`iroh`): the default transport (QUIC, end-to-end encrypted, no central server).
  Its on-the-wire crypto and pairing flow are in scope; the broader network threat model (relay trust, peer authentication edge cases) is being written up here and will get its own disclosure section as it firms up.
  The `file` transport (iCloud Drive / Syncthing / shared FS) is the opt-in alternative — it has no network surface of its own and inherits the trust model of whatever folder you point it at.
- **Third-party plugin code**: the JavaScript plugin system (Boa engine) is shipped, and plugins run with workspace access.
  We'll document the sandbox and what's expected of plugin authors.
- **Denial-of-service via huge workspaces**: pathological inputs that slow down `outl doctor` are bugs, not vulns.
  File a regular issue.

## What you'll get

- A fixed version, tagged and released, before public disclosure.
- An advisory in the release notes once the fix is out.
- A short post-mortem in the repo if the bug exposed a process gap.

Thanks for keeping outl honest.
