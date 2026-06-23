#!/usr/bin/env bash
# PostToolUse hook: nudge toward semantic line breaks (sembr.org) in
# markdown documents that belong to the docs / contributor surface.
#
# Why this exists: docs/contributing.md → "Markdown / documentation
# style" mandates one sentence per line (no 70/80-column reflow). The
# discipline keeps diffs minimal and renders cleanly on every surface.
# Catching violations at edit time is much cheaper than catching them
# at PR review.
#
# Two heuristics, both warn-only (exit 2 with stderr message; never
# block the edit because false positives are inevitable):
#
#   1. Line longer than 250 chars in prose context (not table, not
#      code fence, not frontmatter, not heading).
#      → Almost always wrapped or unintentionally long. The 250 cap
#      lets a single bona-fide technical sentence with parentheticals
#      through; a real paragraph hard-wrapped to 70 cols still trips.
#
#   2. Multiple sentences on one line. Detected as `[.!?] [A-Z][a-z]+ `
#      (capitalized word after sentence terminator + space) somewhere
#      before the line-ending punctuation, after stripping common
#      abbreviations (e.g., i.e., etc., Mr., vs., U.S., ...) and
#      markdown emphasis (`**`, `__`, `*`, `_`) so `log.** If` and
#      similar bold-closing-then-new-sentence patterns are caught.
#      → The sembr violation we keep catching by hand.
#
# Scope: applied to files governed by the markdown-style policy in
# docs/contributing.md. Skipped for: CHANGELOG.md (auto-generated),
# fixtures / note-example outline content (structural markdown is
# data, not docs), node_modules, target.
#
# Reads tool_input.file_path from stdin JSON.

set -uo pipefail

event_json=$(cat)

file_path=$(printf '%s' "$event_json" | sed -n 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')

[ -z "$file_path" ] && exit 0
[ -f "$file_path" ] || exit 0

case "$file_path" in
  *.md|*.mdx) ;;
  *) exit 0 ;;
esac

repo_root="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null)}"
[ -z "$repo_root" ] && exit 0
[ ! -d "$repo_root" ] && exit 0

rel=${file_path#"${repo_root}/"}

# Hard skips: fixtures, outline content, generated artifacts, deps.
case "$rel" in
  note-example/*|*/note-example/*) exit 0 ;;
  */fixtures/*) exit 0 ;;
  */target/*) exit 0 ;;
  */node_modules/*) exit 0 ;;
  CHANGELOG.md) exit 0 ;;
esac

# In-scope: the markdown-style policy explicitly listed in
# docs/contributing.md. Default-off for everything else (project may
# have third-party docs that follow different conventions).
in_scope=0
case "$rel" in
  CLAUDE.md|*/CLAUDE.md) in_scope=1 ;;
  docs/*.md) in_scope=1 ;;
  .github/*.md) in_scope=1 ;;
  .claude/agents/*.md|.claude/commands/*.md) in_scope=1 ;;
  README.md|CONTRIBUTING.md|SECURITY.md) in_scope=1 ;;
esac

[ "$in_scope" = "0" ] && exit 0

# --------------------------------------------------------------------
# Scan with awk. Tracks code-fence and frontmatter state so we don't
# false-positive on a long URL inside a fenced block or a YAML value.
#
# All output (per-line diagnostics + summary marker) goes to awk stdout
# so the shell can surface it to /dev/stderr in one ordered block.
# --------------------------------------------------------------------

# LC_ALL=C makes awk treat the file as a byte stream — `length()` returns
# byte count instead of char count, which is fine for the >250 threshold
# and avoids "multibyte conversion failure" on docs with emoji / box-drawing
# characters (architecture.md trips this without the byte-mode fallback).
awk_output=$(LC_ALL=C awk '
BEGIN {
  in_fence = 0
  in_fm = 0
  long_count = 0
  multi_count = 0
  max_report = 8
}

# Track ``` opening / closing.
/^```/ {
  in_fence = !in_fence
  next
}
in_fence { next }

# YAML frontmatter only valid at file start.
NR == 1 && $0 == "---" { in_fm = 1; next }
in_fm {
  if ($0 == "---") in_fm = 0
  next
}

# Lines we never flag.
$0 == "" { next }
/^#/ { next }                     # heading
/^---+$/ { next }                 # horizontal rule
/^\|/ { next }                    # table row
/^[[:space:]]*\|/ { next }        # indented table row
/^</ { next }                     # HTML block
/^>/ { next }                     # blockquote (block-level)
/^\[.*\]:[[:space:]]/ { next }    # reference link definition
/^[[:space:]]*-[[:space:]]\[[ xX]\]/ { next }  # task list (one item)

# --- Heuristic 1: line > 250 chars.
{
  raw_len = length($0)
  if (raw_len > 250) {
    long_count++
    if (long_count <= max_report) {
      preview = substr($0, 1, 120)
      gsub(/\t/, "  ", preview)
      printf("  L%d (%d chars): %s...\n", NR, raw_len, preview)
    }
  }
}

# --- Heuristic 2: multiple sentences on one line.
{
  body = $0
  # Strip common abbreviations so their inner periods do not count.
  gsub(/e\.g\./, "eg",   body)
  gsub(/i\.e\./, "ie",   body)
  gsub(/etc\./,  "etc",  body)
  gsub(/cf\./,   "cf",   body)
  gsub(/vs\./,   "vs",   body)
  gsub(/Mr\./,   "Mr",   body)
  gsub(/Mrs\./,  "Mrs",  body)
  gsub(/Ms\./,   "Ms",   body)
  gsub(/Dr\./,   "Dr",   body)
  gsub(/St\./,   "St",   body)
  gsub(/U\.S\./, "US",   body)
  gsub(/U\.K\./, "UK",   body)
  gsub(/No\./,   "No",   body)
  gsub(/Inc\./,  "Inc",  body)
  gsub(/Co\./,   "Co",   body)
  gsub(/p\.s\./, "ps",   body)
  # Strip markdown emphasis runs so `log.** If` (bold close + new
  # sentence) and `foo._ Bar_` (italic close) match the heuristic.
  gsub(/\*\*/, "", body)
  gsub(/__/,   "", body)
  # Strip the leading list-marker so `1. Foo bar` or `- Foo bar` is not
  # mistaken for "sentence ending in 1, new sentence starting at Foo".
  sub(/^[[:space:]]*[0-9]+\.[[:space:]]+/,   "", body)   # numbered list
  sub(/^[[:space:]]*[-*+][[:space:]]+/,      "", body)   # bullet list
  # Strip the trailing sentence terminator(s) plus quote/paren/brackets.
  sub(/[.!?]+["'"'"'`)\]>]*[[:space:]]*$/, "", body)

  # `. ` followed by any capital letter — the common shape of "Sentence
  # ends. Next sentence starts." Catches `If`, `A`, `Run`, `Foo`. Known
  # false positive: inline initials like `J. Smith` mid-sentence, which
  # we accept because (a) abbreviations covered by the gsub list above
  # handle the common cases (Mr., Mrs., U.S., ...) and (b) docs in this
  # repo rarely embed bare initials.
  if (body ~ /[.!?][[:space:]]+[A-Z]/) {
    multi_count++
    if (multi_count <= max_report) {
      preview = substr($0, 1, 120)
      gsub(/\t/, "  ", preview)
      printf("  L%d (multi-sentence): %s...\n", NR, preview)
    }
  }
}

END {
  if (long_count > 0 || multi_count > 0) {
    printf("__SEMBR_VIOLATIONS__ long=%d multi=%d\n", long_count, multi_count)
  }
}
' "$file_path")

summary=$(printf '%s\n' "$awk_output" | grep -E '^__SEMBR_VIOLATIONS__' || true)

[ -z "$summary" ] && exit 0

long=$(printf '%s' "$summary"  | sed -n 's/.*long=\([0-9]*\).*/\1/p')
multi=$(printf '%s' "$summary" | sed -n 's/.*multi=\([0-9]*\).*/\1/p')

# Per-line diagnostics (everything except the summary marker).
diagnostics=$(printf '%s\n' "$awk_output" | grep -v -E '^__SEMBR_VIOLATIONS__' || true)

{
  printf 'SEMBR WARNING: %s violates the semantic line break policy.\n' "$rel"
  printf '\n'
  [ "${long:-0}" -gt 0 ]  && printf '  - %d line(s) longer than 250 chars (likely hard-wrapped or unbroken prose).\n' "$long"
  [ "${multi:-0}" -gt 0 ] && printf '  - %d line(s) carry multiple sentences (`. [A-Z]` mid-line).\n' "$multi"
  printf '\n'
  if [ -n "$diagnostics" ]; then
    printf '%s\n' "$diagnostics"
    printf '\n'
  fi
  printf 'Rule: one sentence per line; no fixed-column wrap. Tables, code\n'
  printf 'fences, frontmatter and headings are exempt.\n'
  printf 'See docs/contributing.md → "Markdown / documentation style".\n'
} >&2

exit 2
