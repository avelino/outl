#!/usr/bin/env bash
# PostToolUse hook: warn when the "Shared primitives catalog" gets
# edited in one place (root CLAUDE.md OR .github/copilot-instructions.md)
# without the other being touched in the same working-tree change.
#
# The catalog is intentionally duplicated because it has two audiences:
# - root CLAUDE.md drives Claude Code (and human contributors)
# - .github/copilot-instructions.md drives GitHub Copilot PR review
#
# Both must stay in sync or one of them goes stale and the LLM on that
# side starts approving (or generating) duplicate helpers the other
# side would block — exactly the failure mode that surfaced in PR #47.
#
# This is a soft signal (exit 2 with message). It doesn't block — the
# tool execution already happened. It just nudges Claude to update the
# other side in the same change.
#
# Reads tool_input.file_path from stdin JSON.

set -uo pipefail

event_json=$(cat)

file_path=$(printf '%s' "$event_json" | sed -n 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')

# Only act on the two mirrored files.
case "$file_path" in
  */CLAUDE.md|*/copilot-instructions.md) ;;
  *) exit 0 ;;
esac

# Root CLAUDE.md only (per-crate CLAUDE.md don't carry the full catalog).
# We accept the root one specifically by checking it's at the repo root.
repo_root="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null)}"
if [ -z "$repo_root" ] || [ ! -d "$repo_root" ]; then
  exit 0
fi

root_claude="${repo_root}/CLAUDE.md"
copilot="${repo_root}/.github/copilot-instructions.md"

# The thing being edited must be one of the two mirrors.
case "$file_path" in
  "$root_claude"|"$copilot") ;;
  *) exit 0 ;;
esac

# Marker that identifies the catalog table in either file. Pick a
# string that lives ONLY in the catalog section so we don't trigger on
# unrelated edits to either file.
marker="Shared primitives catalog"

# Does the edited file actually contain the catalog right now?
if ! grep -q "$marker" "$file_path" 2>/dev/null; then
  exit 0
fi

# Has the catalog section been touched in the working tree (vs HEAD)
# in the file the user just edited? Use git diff with grep on the
# patch for the marker plus nearby table-row syntax (`|` heavy lines).
touched_catalog() {
  local f=$1
  # If the file is untracked entirely, treat as touched.
  if ! git ls-files --error-unmatch -- "$f" >/dev/null 2>&1; then
    return 0
  fi
  git diff --no-color -U0 -- "$f" 2>/dev/null \
    | grep -E '^[+-](\|.*\|.*\|.*\||.*Shared primitives catalog)' \
    | grep -q .
}

# Determine which file is the mirror of what was just edited.
case "$file_path" in
  "$root_claude") mirror="$copilot" ;;
  "$copilot")     mirror="$root_claude" ;;
esac

# If the edit didn't touch the catalog itself, no sync needed.
if ! touched_catalog "$file_path"; then
  exit 0
fi

# Catalog was edited. Check the mirror.
if touched_catalog "$mirror"; then
  # Both sides touched. Likely already in sync. Pass.
  exit 0
fi

# Edited one side of the catalog without touching the mirror.
rel_edited=${file_path#"${repo_root}/"}
rel_mirror=${mirror#"${repo_root}/"}

printf 'WARNING: %s edited the "Shared primitives catalog" table\n' "$rel_edited" >&2
printf 'but its mirror at %s has no matching working-tree change.\n' "$rel_mirror" >&2
printf '\n' >&2
printf 'The catalog is intentionally duplicated for two audiences:\n' >&2
printf '  - %s drives Claude Code + human contributors\n' "CLAUDE.md" >&2
printf '  - %s drives GitHub Copilot PR review\n' ".github/copilot-instructions.md" >&2
printf '\n' >&2
printf 'Drift between them is exactly how PR #47 slipped through (paste::normalize\n' >&2
printf 'duplication invisible to the reviewer). Update %s in the\n' "$rel_mirror" >&2
printf 'same change so both LLMs see the catalog you just edited.\n' >&2
exit 2
