#!/usr/bin/env bash
# PostToolUse: when a code edit lands without updating the
# documentation that describes that surface, warn so Claude treats
# doc maintenance as part of the same change — not a separate chore
# the user has to remember.
#
# Rationale: CLAUDE.md (root + per-crate), .github/copilot-instructions.md,
# and docs/*.md are how the next contributor (human or LLM) learns
# this codebase. Code drift without doc drift is how the next PR
# arrives reimplementing something that just shipped. The
# `paste::normalize_external_syntax` duplication in PR #47 is the
# canonical incident.
#
# Three rules, in order of severity:
#
#   1. CATALOG. A new top-level `pub fn|struct|enum|const` in any
#      shared crate (outl-core / outl-md / outl-actions) must appear
#      by name in root CLAUDE.md § "Shared primitives catalog". The
#      symbol is the canonical reuse handle for the workspace.
#
#   2. PER-CRATE. Any non-test edit in `crates/<crate>/src/` should
#      reflect in `crates/<crate>/CLAUDE.md` when it touches the
#      public surface (the edit adds `pub`, or it's a >20-line block).
#      Internal refactors with no public-surface change pass silently.
#
#   3. HIGH-LEVEL DOCS. Specific source files map to specific
#      `docs/*.md` (op log → crdt.md, sidecar → markdown-format.md,
#      TUI keymap → tui.md, CLI cmd → cli.md, MCP → mcp.md, storage →
#      storage.md). An edit in one of those files should bring its
#      doc along.
#
# Non-blocking: exit 2 with a structured message. Claude reads it and
# either updates the docs in the same response, or replies confirming
# the edit is internal-only.
#
# Reads tool_input.file_path from stdin JSON.

set -uo pipefail

event_json=$(cat)
file_path=$(printf '%s' "$event_json" | sed -n 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')

[ -z "$file_path" ] && exit 0
# Accept `.rs` (catalog + per-crate + high-level mappings all apply)
# plus `.ts`/`.tsx` for the desktop/mobile frontend shortcut wiring
# — keybinding changes can land entirely on the JS side (e.g. a new
# handler in `action-handlers.ts`) and would still leave the desktop
# CLAUDE.md shortcut table out of sync.
case "$file_path" in *.rs|*.ts|*.tsx) ;; *) exit 0 ;; esac

repo_root="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null)}"
[ -z "$repo_root" ] && exit 0
[ ! -d "$repo_root" ] && exit 0

rel=${file_path#"${repo_root}/"}

# Skip tests, target/, examples/, build scripts.
case "$rel" in
  */tests/*|*_tests.rs|*_test.rs) exit 0 ;;
  */target/*|*/examples/*|build.rs) exit 0 ;;
esac

# Only act on edits inside crates/<crate>/src/.
case "$rel" in
  crates/*/src/*) ;;
  *) exit 0 ;;
esac

crate_dir=$(printf '%s' "$rel" | sed -E 's|^(crates/[^/]+)/.*|\1|')

# --------------------------------------------------------------------
# Diff inspection helpers.
# --------------------------------------------------------------------

# True if `file` has working-tree changes vs HEAD (or is untracked).
touched() {
  local f=$1
  if ! git ls-files --error-unmatch -- "$f" >/dev/null 2>&1; then
    [ -f "$repo_root/$f" ] && return 0 || return 1
  fi
  ! git diff --quiet --no-color -- "$f" 2>/dev/null
}

# Count of `+pub fn|struct|enum|const ...` lines added in the working
# tree diff of $file_path. Used by rule 1 + rule 2.
new_pub_count() {
  local f=$1
  if git ls-files --error-unmatch -- "$f" >/dev/null 2>&1; then
    git diff --no-color -U0 -- "$f" 2>/dev/null \
      | grep -cE '^\+pub (fn|struct|enum|const) [A-Za-z_]' || true
  else
    # New file: count its public symbols straight from the file.
    grep -cE '^pub (fn|struct|enum|const) [A-Za-z_]' "$f" 2>/dev/null || true
  fi
}

# Total lines added by the working tree diff (excluding context).
added_line_count() {
  local f=$1
  git ls-files --error-unmatch -- "$f" >/dev/null 2>&1 || {
    wc -l < "$f" 2>/dev/null | tr -d ' '
    return
  }
  git diff --no-color -U0 -- "$f" 2>/dev/null \
    | grep -cE '^\+[^+]' || true
}

# New symbol names extracted from the working tree diff.
new_symbols_of() {
  local f=$1
  local input
  if git ls-files --error-unmatch -- "$f" >/dev/null 2>&1; then
    input=$(git diff --no-color -U0 -- "$f" 2>/dev/null)
  else
    input=$(sed 's/^/+/' "$f" 2>/dev/null)
  fi
  printf '%s\n' "$input" \
    | grep -E '^\+pub (fn|struct|enum|const) [A-Za-z_][A-Za-z0-9_]*' \
    | sed -E 's/^\+pub (fn|struct|enum|const) ([A-Za-z_][A-Za-z0-9_]*).*/\2/' \
    | grep -vxE 'new|default|from|with_state' \
    | sort -u
}

# --------------------------------------------------------------------
# Findings collector — one message at the end.
# --------------------------------------------------------------------

warnings=()

# --------------------------------------------------------------------
# Rule 1 — Shared primitives catalog.
# --------------------------------------------------------------------

case "$crate_dir" in
  crates/outl-core|crates/outl-md|crates/outl-actions)
    new_syms=$(new_symbols_of "$file_path")
    if [ -n "$new_syms" ]; then
      catalog="$repo_root/CLAUDE.md"
      copilot="$repo_root/.github/copilot-instructions.md"
      missing=()
      while IFS= read -r sym; do
        [ -z "$sym" ] && continue
        grep -qE "\b${sym}\b" "$catalog" 2>/dev/null || missing+=("$sym")
      done <<< "$new_syms"
      if [ ${#missing[@]} -gt 0 ]; then
        msg="rule 1 — Shared primitives catalog: %s added new public symbol(s) missing from CLAUDE.md catalog:\n"
        for sym in "${missing[@]}"; do
          msg+="    - pub ${sym}\n"
        done
        msg+="  fix: add an entry under the matching sub-table in CLAUDE.md AND .github/copilot-instructions.md §5.1.\n"
        msg+="       (catalog-sync-guard.sh verifies the two stay in sync.)"
        warnings+=("$(printf "$msg" "$rel")")
      fi
    fi
    ;;
esac

# --------------------------------------------------------------------
# Rule 2 — per-crate CLAUDE.md.
# --------------------------------------------------------------------

crate_claude="$crate_dir/CLAUDE.md"
if [ -f "$repo_root/$crate_claude" ]; then
  new_pub=$(new_pub_count "$file_path")
  added=$(added_line_count "$file_path")
  significant=0
  [ "${new_pub:-0}" -gt 0 ] && significant=1
  [ "${added:-0}" -gt 20 ] && significant=1
  if [ "$significant" = "1" ] && ! touched "$crate_claude"; then
    msg="rule 2 — per-crate doc: %s changed (new pub: ${new_pub:-0}, lines added: ${added:-0}) but ${crate_claude} has no matching change.\n"
    msg+="  fix: update the 'Public surface' table / 'What this crate owns' list / invariants section in ${crate_claude} to reflect the new behavior.\n"
    msg+="       if the change is genuinely internal-only (private helper rename, internal refactor), say so explicitly and continue."
    warnings+=("$(printf "$msg" "$rel")")
  fi
fi

# --------------------------------------------------------------------
# Rule 3 — high-level docs.
# --------------------------------------------------------------------

docs_to_check=()

case "$rel" in
  crates/outl-core/src/op.rs|crates/outl-core/src/tree/*.rs|crates/outl-core/src/log.rs)
    docs_to_check+=("docs/crdt.md")
    ;;
esac
case "$rel" in
  crates/outl-core/src/storage/*|crates/outl-core/src/storage.rs)
    docs_to_check+=("docs/storage.md")
    ;;
esac
case "$rel" in
  crates/outl-md/src/sidecar.rs|crates/outl-md/src/parse.rs|crates/outl-md/src/render.rs|crates/outl-md/src/inline.rs)
    docs_to_check+=("docs/markdown-format.md")
    ;;
esac
case "$rel" in
  crates/outl-tui/src/keymap*.rs|crates/outl-tui/src/actions/*|crates/outl-tui/src/modes/*)
    docs_to_check+=("docs/tui.md")
    ;;
esac
case "$rel" in
  crates/outl-tui/src/theme*.rs)
    docs_to_check+=("docs/theming.md")
    ;;
esac
case "$rel" in
  crates/outl-cli/src/cmd/*.rs|crates/outl-cli/src/output.rs)
    docs_to_check+=("docs/cli.md")
    ;;
esac
case "$rel" in
  crates/outl-cli/src/mcp/*.rs)
    docs_to_check+=("docs/mcp.md")
    ;;
esac
case "$rel" in
  crates/outl-actions/src/sync.rs)
    docs_to_check+=("docs/sync.md")
    ;;
esac
case "$rel" in
  crates/outl-actions/src/*)
    docs_to_check+=("docs/clients.md")
    ;;
esac
# Shortcut catalog: every binding edit lands on at least three
# user-facing surfaces (the catalog crate's own doc, the desktop
# client's help table, the TUI doc with its parallel keymap). We
# bypass the ≥10-line threshold further down because a one-line
# `Binding::new` swap is exactly the kind of change that ships
# without doc updates if we let it slide.
shortcut_change=0
case "$rel" in
  crates/outl-shortcuts/src/defaults.rs|crates/outl-shortcuts/src/action.rs)
    docs_to_check+=("crates/outl-shortcuts/CLAUDE.md")
    docs_to_check+=("crates/outl-desktop/CLAUDE.md")
    docs_to_check+=("docs/tui.md")
    shortcut_change=1
    ;;
esac
case "$rel" in
  crates/outl-tui/src/input/*)
    docs_to_check+=("docs/tui.md")
    docs_to_check+=("crates/outl-shortcuts/CLAUDE.md")
    shortcut_change=1
    ;;
esac
# Frontend wiring for the desktop shortcut dispatcher + action
# handlers. Catches `Cmd+T` swaps that live entirely in JS or in
# the per-block textarea `onKeyDown` (the `Cmd+Enter` race we just
# undid is the canonical incident).
case "$rel" in
  crates/outl-desktop/src/lib/shortcuts.ts \
  | crates/outl-desktop/src/lib/action-handlers.ts \
  | crates/outl-desktop/src/components/BlockRow.tsx)
    docs_to_check+=("crates/outl-desktop/CLAUDE.md")
    docs_to_check+=("crates/outl-shortcuts/CLAUDE.md")
    shortcut_change=1
    ;;
esac

# Dedupe and filter to existing + untouched docs.
if [ ${#docs_to_check[@]} -gt 0 ]; then
  uniq_docs=$(printf '%s\n' "${docs_to_check[@]}" | sort -u)
  stale=()
  added=$(added_line_count "$file_path")
  # Rule 3 normally requires ≥10 added lines (avoid pestering on
  # tiny refactors). Shortcut / binding edits bypass the gate: a
  # single-line chord swap is the most likely change to ship without
  # the user-facing tables being updated, which is exactly what we
  # learned by missing the `Cmd+T` → `Cmd+J` swap in the last
  # round-trip.
  threshold_met=0
  [ "${added:-0}" -ge 10 ] && threshold_met=1
  [ "$shortcut_change" = "1" ] && threshold_met=1
  if [ "$threshold_met" = "1" ]; then
    while IFS= read -r doc; do
      [ -z "$doc" ] && continue
      [ ! -f "$repo_root/$doc" ] && continue
      touched "$doc" || stale+=("$doc")
    done <<< "$uniq_docs"
    if [ ${#stale[@]} -gt 0 ]; then
      msg="rule 3 — high-level docs: %s changed (lines added: ${added}) but its user-facing doc has no matching change:\n"
      for doc in "${stale[@]}"; do
        msg+="    - ${doc}\n"
      done
      msg+="  fix: update the listed doc(s) to reflect the new behavior, OR confirm explicitly that this change is invisible to readers of those docs (internal refactor)."
      warnings+=("$(printf "$msg" "$rel")")
    fi
  fi
fi

# --------------------------------------------------------------------
# Emit.
# --------------------------------------------------------------------

[ ${#warnings[@]} -eq 0 ] && exit 0

printf 'DOC DRIFT WARNING — %s was edited without matching doc updates.\n' "$rel" >&2
printf '\n' >&2
for w in "${warnings[@]}"; do
  printf '%s\n\n' "$w" >&2
done
printf 'CLAUDE.md / per-crate CLAUDE.md / .github/copilot-instructions.md / docs/*.md\n' >&2
printf 'are how the next contributor (human or LLM) learns this codebase.\n' >&2
printf 'Treat doc maintenance as part of the same change, not a separate chore.\n' >&2
exit 2
