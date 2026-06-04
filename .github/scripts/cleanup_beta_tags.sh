#!/usr/bin/env bash
# Prune old `vX.Y.Z-beta.N` tags + GitHub Releases, keeping the
# `$KEEP` most recent betas per base version.
#
# Driven by `.github/workflows/cleanup-tags.yml`. Runs after every
# successful `Release` workflow on `main`.
#
# Env vars (all required unless noted):
#
#     GH_TOKEN              GitHub token with `contents: write`
#     GITHUB_REPOSITORY     owner/name (set by Actions runner)
#     KEEP                  how many recent betas to keep per base (default: 2)
#     DRY_RUN               "true" to log without deleting (default: false)
#
# Scope (intentional):
# - Touches ONLY `vX.Y.Z-beta.N` tags. GA tags like `v0.5.3`
#   (no `-beta.` suffix) are never considered for deletion.
# - Deletes both the GitHub Release (with its built binaries) and
#   the git tag — `gh release delete --cleanup-tag` does both atomically.
# - Orphan tags (tag exists but the release is already gone) get
#   pushed as `:refs/tags/<tag>` to drop the ref alone.

set -euo pipefail

KEEP="${KEEP:-2}"
DRY_RUN="${DRY_RUN:-false}"

if [[ -z "${GITHUB_REPOSITORY:-}" ]]; then
  echo "::error::GITHUB_REPOSITORY is not set"
  exit 1
fi

# Local tags can be stale; trust origin.
git fetch --tags --prune --prune-tags --force origin

# Build prune list:
#   1. Filter to `vX.Y.Z-beta.N` tags only (skip GA + malformed).
#   2. Emit "<base> <beta_num> <full_tag>".
#   3. Group by base, sort beta_num desc, drop the top KEEP.
TO_DELETE_FILE="$(mktemp)"
trap 'rm -f "$TO_DELETE_FILE"' EXIT

git tag --list 'v*-beta.*' |
  awk -F'-beta\.' '
    NF == 2 && $1 ~ /^v[0-9]+\.[0-9]+\.[0-9]+$/ && $2 ~ /^[0-9]+$/ {
      print $1 " " $2 " " $0
    }
  ' |
  sort -k1,1 -k2,2nr |
  awk -v keep="$KEEP" '
    {
      if ($1 != prev) { prev = $1; count = 0 }
      count++
      if (count > keep) print $3
    }
  ' > "$TO_DELETE_FILE"

if [[ ! -s "$TO_DELETE_FILE" ]]; then
  echo "::notice::Nothing to prune (keep=$KEEP per base)."
  exit 0
fi

COUNT=$(wc -l < "$TO_DELETE_FILE" | tr -d ' ')
echo "::notice::Will prune ${COUNT} tag(s) (keep=$KEEP per base):"
sed 's/^/  - /' "$TO_DELETE_FILE"

if [[ "$DRY_RUN" == "true" ]]; then
  echo "::notice::Dry run: nothing deleted."
  exit 0
fi

FAILED=0
while IFS= read -r tag; do
  [[ -z "$tag" ]] && continue
  echo "::group::Delete $tag"
  if gh release view "$tag" --repo "$GITHUB_REPOSITORY" >/dev/null 2>&1; then
    # `--cleanup-tag` drops the git ref alongside the release.
    if ! gh release delete "$tag" \
      --repo "$GITHUB_REPOSITORY" \
      --cleanup-tag \
      --yes; then
      echo "::error::Failed to delete release $tag"
      FAILED=$((FAILED + 1))
    fi
  else
    # Release already gone; nuke the orphan tag directly.
    echo "No release for $tag; deleting orphan tag ref."
    if ! git push origin ":refs/tags/$tag"; then
      echo "::error::Failed to delete tag $tag"
      FAILED=$((FAILED + 1))
    fi
  fi
  echo "::endgroup::"
done < "$TO_DELETE_FILE"

if [[ "$FAILED" -gt 0 ]]; then
  echo "::error::Cleanup finished with $FAILED failure(s)."
  exit 1
fi
echo "::notice::Pruned ${COUNT} tag(s) successfully."
