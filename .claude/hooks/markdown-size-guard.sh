#!/usr/bin/env bash
# PostToolUse hook: enforce sensible size budgets on markdown files
# that LLMs (and humans) load wholesale.
#
# Why this exists: every CLAUDE.md is loaded into the model context at
# session start. Anthropic's guideline is that beyond ~40k characters
# performance degrades — the model spends attention budget on the
# instructions instead of the task. The same logic applies to
# .github/copilot-instructions.md (GitHub Copilot reads it whole) and
# to docs/*.md (users read whole pages; if a doc has many
# responsibilities it should be split).
#
# Three tiers:
#
#   CLAUDE.md (root + per-crate) | .github/*.md
#     - 40_000 chars → BLOCK (exit 2). Hard limit, Anthropic guideline.
#
#   docs/*.md
#     - 30_000 chars → WARN (informational, exit 0).
#     - 50_000 chars → BLOCK (exit 2). A doc this big has accreted
#       multiple responsibilities; split before the next edit.
#
#   Everything else (README, CHANGELOG, fixtures, note-example, ...)
#     - No-op. Those have their own rules (changelogs grow forever,
#       fixtures must stay literal, etc.).
#
# Reads tool_input.file_path from stdin JSON. Skips files that don't
# exist (e.g. an Edit that was reverted before the hook ran).

set -uo pipefail

event_json=$(cat)

file_path=$(printf '%s' "$event_json" | sed -n 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')

[ -z "$file_path" ] && exit 0
[ -f "$file_path" ] || exit 0

case "$file_path" in
  *.md) ;;
  *) exit 0 ;;
esac

repo_root="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null)}"
[ -z "$repo_root" ] && exit 0
[ ! -d "$repo_root" ] && exit 0

rel=${file_path#"${repo_root}/"}

# Skip fixtures + note-example + node_modules + target.
case "$rel" in
  note-example/*|*/note-example/*) exit 0 ;;
  */target/*) exit 0 ;;
  */node_modules/*) exit 0 ;;
  */fixtures/*) exit 0 ;;
esac

chars=$(wc -c < "$file_path" | tr -d ' ')

# --------------------------------------------------------------------
# Tier 1 — CLAUDE.md (root or per-crate) and .github/*.md
# Hard limit: 40_000 chars. Block when exceeded.
# --------------------------------------------------------------------

is_strict=0
case "$rel" in
  CLAUDE.md|*/CLAUDE.md) is_strict=1 ;;
  .github/*.md) is_strict=1 ;;
esac

if [ "$is_strict" = "1" ]; then
  if [ "$chars" -gt 40000 ]; then
    printf 'STOP: %s is %d chars (limit: 40000).\n' "$rel" "$chars" >&2
    printf '\n' >&2
    printf 'CLAUDE.md and .github/*.md are loaded into the LLM context wholesale.\n' >&2
    printf 'Anthropic guideline: beyond 40k chars, model attention degrades on the\n' >&2
    printf 'real task because too much budget is spent re-reading the instructions.\n' >&2
    printf '\n' >&2
    printf 'Fix: move user-facing reference tables (shortcuts, CLI subcommands,\n' >&2
    printf 'config keys, primitive catalogs, agent lists, slash command lists)\n' >&2
    printf 'into a dedicated docs/*.md file and link from here.\n' >&2
    printf 'See docs/contributing.md → "One owner per fact" for the policy.\n' >&2
    exit 2
  fi
  # Soft notice at 80% (32_000) so we see drift coming.
  if [ "$chars" -gt 32000 ]; then
    printf 'note: %s is %d chars (80%% of the 40k limit). Plan extractions soon.\n' \
      "$rel" "$chars" >&2
  fi
  exit 0
fi

# --------------------------------------------------------------------
# Tier 2 — docs/*.md
# Warn at 30k, block at 50k. A doc that crosses 50k is almost always
# multiple responsibilities sharing a page.
# --------------------------------------------------------------------

case "$rel" in
  docs/*.md) ;;
  *) exit 0 ;;
esac

if [ "$chars" -lt 30000 ]; then
  exit 0
fi

if [ "$chars" -lt 50000 ]; then
  printf 'note: %s is %d chars (>30k). Watch for accumulation;\n' "$rel" "$chars" >&2
  printf 'a doc page past 30k usually has 2+ responsibilities. Plan a split\n' >&2
  printf 'when the next responsibility lands.\n' >&2
  exit 0
fi

# >= 50k — block.
printf 'STOP: %s is %d chars (limit: 50000).\n' "$rel" "$chars" >&2
printf '\n' >&2
printf 'A docs/*.md page past 50k chars is almost always multiple\n' >&2
printf 'responsibilities sharing a page. Readers (human + LLM) load it\n' >&2
printf 'whole; cohesion suffers.\n' >&2
printf '\n' >&2
printf 'Before the next non-trivial edit:\n' >&2
printf '  1. Identify 2-3 distinct topics inside this doc.\n' >&2
printf '  2. Split each into its own docs/*.md.\n' >&2
printf '  3. Update docs/SUMMARY.md with the new entries.\n' >&2
printf '  4. Update CLAUDE.md / per-crate CLAUDE.md links if they\n' >&2
printf '     pointed at the old monolith.\n' >&2
exit 2
