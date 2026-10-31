#!/bin/sh
# Comment on each open pull request with no activity for MAGNUS_STALE_HOURS
# (default 24): how long it has waited and what it waits on (behind master,
# its checks, or a person's merge). The comment is activity, so each gets
# at most one a day. Run by .github/workflows/pr-stale.yml with GH_TOKEN.
set -eu
hours=${MAGNUS_STALE_HOURS:-24}
git fetch -q origin master
now=$(date -u +%s)
gh pr list --state open --limit 100 --json number,updatedAt,headRefOid -q '.[] | "\(.number) \(.updatedAt) \(.headRefOid)"' |
while read -r n updated head; do
  age=$(( (now - $(date -u -d "$updated" +%s)) / 3600 ))
  [ "$age" -ge "$hours" ] || continue
  git fetch -q origin "pull/$n/head" || true
  waits=""
  git merge-base --is-ancestor origin/master "$head" 2>/dev/null || waits="$waits
- It does not hold master's current commit: merge master into it, test, and push."
  checks=0
  gh pr checks "$n" >/dev/null 2>&1 || checks=$?
  case $checks in
  0) waits="$waits
- Its checks pass: it waits on the owner's merge." ;;
  8) waits="$waits
- Its checks are still running." ;;
  *) waits="$waits
- A check fails: fix it, or close this pull request." ;;
  esac
  gh pr comment "$n" --body "This pull request has had no activity for $age hours. What it waits on:$waits

A pull request is merged or closed within a day of going green, and the next one opens only after it (CLAUDE.md, \"How work lands\"). Finish it today, or close it and say what was kept."
done
