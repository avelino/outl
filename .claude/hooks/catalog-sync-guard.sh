#!/usr/bin/env bash
# PostToolUse hook: warn when the "Shared primitives catalog" gets
# edited on one side (the docs/ catalog OR
# .github/copilot-instructions.md) without the other being touched in
# the same working-tree change.
#
# The catalog is intentionally duplicated because it has two audiences:
# - the docs/ catalog drives Claude Code + human contributors
# - .github/instructions/shared-primitives.instructions.md drives GitHub
#   Copilot PR review (path-scoped, `applyTo: crates/**`)
#
# The docs/ side is ONE logical document split across four files:
#   docs/shared-primitives.md      — hub / index (links, no rows)
#   docs/primitives-core.md        — op log, tree, sync, storage, backups
#   docs/primitives-markdown.md    — parse, render, sidecar, index, assets
#   docs/primitives-actions.md     — block/page mutations, exec, templates, …
# Editing ANY of them counts as editing the catalog; the glob
# docs/primitives-*.md picks up future parts without a hook change.
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

# Only act on the mirrored files.
#
# The Copilot side moved out of copilot-instructions.md into a
# path-scoped instructions file (`applyTo: crates/**`): the repo-wide
# file was 40k chars against GitHub's ~2-page guidance, and the catalog
# was 62% of it. `copilot-instructions.md` stays matched here so an edit
# that puts catalog rows back into it is still caught.
case "$file_path" in
  */docs/shared-primitives.md|*/docs/primitives-*.md) ;;
  */.github/instructions/shared-primitives.instructions.md) ;;
  */copilot-instructions.md) ;;
  *) exit 0 ;;
esac

repo_root="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null)}"
if [ -z "$repo_root" ] || [ ! -d "$repo_root" ]; then
  exit 0
fi

copilot="${repo_root}/.github/instructions/shared-primitives.instructions.md"
copilot_legacy="${repo_root}/.github/copilot-instructions.md"

# Every file that makes up the docs/ side of the catalog.
catalog_files=()
for c in "${repo_root}"/docs/shared-primitives.md "${repo_root}"/docs/primitives-*.md; do
  [ -f "$c" ] && catalog_files+=("$c")
done

# The thing being edited must be one of the two sides.
edited_side=""
[ "$file_path" = "$copilot" ] && edited_side="copilot"
[ "$file_path" = "$copilot_legacy" ] && edited_side="copilot"
for c in "${catalog_files[@]}"; do
  [ "$file_path" = "$c" ] && edited_side="catalog"
done
[ -z "$edited_side" ] && exit 0

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

# Determine the mirror side of what was just edited. The docs/ side is
# a set: a row added to ANY part satisfies the copilot->docs direction.
if [ "$edited_side" = "catalog" ]; then
  mirror_files=("$copilot")
  rel_mirror=".github/instructions/shared-primitives.instructions.md"
else
  mirror_files=("${catalog_files[@]}")
  rel_mirror="docs/primitives-*.md (the catalog parts)"
fi

# If the edit didn't touch the catalog itself, no sync needed.
if ! touched_catalog "$file_path"; then
  exit 0
fi

# Catalog was edited. Check the mirror side — any file counts.
for m in "${mirror_files[@]}"; do
  if touched_catalog "$m"; then
    # Both sides touched. Likely already in sync. Pass.
    exit 0
  fi
done

# Edited one side of the catalog without touching the mirror.
rel_edited=${file_path#"${repo_root}/"}

printf 'WARNING: %s edited the "Shared primitives catalog" table\n' "$rel_edited" >&2
printf 'but its mirror at %s has no matching working-tree change.\n' "$rel_mirror" >&2
printf '\n' >&2
printf 'The catalog is intentionally duplicated for two audiences:\n' >&2
printf '  - %s drives Claude Code + human contributors\n' "docs/primitives-*.md (index: docs/shared-primitives.md)" >&2
printf '  - %s drives GitHub Copilot PR review\n' ".github/instructions/shared-primitives.instructions.md" >&2
printf '\n' >&2
printf 'Drift between them is exactly how PR #47 slipped through (paste::normalize\n' >&2
printf 'duplication invisible to the reviewer). Update %s in the\n' "$rel_mirror" >&2
printf 'same change so both LLMs see the catalog you just edited.\n' >&2
exit 2
