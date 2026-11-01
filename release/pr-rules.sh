#!/bin/sh
# The rules every pull request is checked against in CI (CLAUDE.md, "How
# work lands"; DECISIONS.md, "One pull request at a time"). Run in the
# pull request's checkout, with master fetched:
#
#   release/pr-rules.sh PR HEAD BASE_BRANCH MASTER
#
# PR is this pull request's number, HEAD its head commit, BASE_BRANCH the
# branch it asks to merge into, MASTER master's current commit.
# MAGNUS_OPEN_PRS lists the open pull requests' numbers (CI asks GitHub).
#
# It fails (exit 4), naming each rule broken, when:
# - a pull request opened before this one is still open: one at a time, in
#   the order opened, so nothing waits behind work that is never finished;
# - it asks to merge into anything but master: no stacked pull requests;
# - its branch does not hold master's current commit: merge master into it.
# It warns, without failing, when the change is large (more than
# MAGNUS_PR_LINES lines, default 800, outside tests and documents): a
# large change is reviewed slowly and goes stale; split it if it can be.
set -eu

pr=$1 head=$2 base_branch=$3 master=$4
broken=0
fail() { echo "::error::$1"; broken=1; }

for n in ${MAGNUS_OPEN_PRS:-}; do
  if [ "$n" -lt "$pr" ]; then
    fail "pull request #$n was opened before this one and is still open: one pull request at a time. Merge or close #$n first; this one waits."
  fi
done
if [ "$base_branch" != master ]; then
  fail "this pull request merges into $base_branch: every pull request merges into master, from a branch made from current master (no stacked pull requests)."
fi
if ! git merge-base --is-ancestor "$master" "$head" 2>/dev/null; then
  fail "this branch does not hold master's current commit $(git rev-parse --short "$master" 2>/dev/null || echo "$master"): merge master into it (git fetch origin master && git merge origin/master), test, and push."
fi

lines=$(git diff --numstat "$(git merge-base "$master" "$head")" "$head" -- . ':(exclude)tests' ':(exclude)*.md' ':(exclude)docs' 2>/dev/null | awk '$1 != "-" { n += $1 + $2 } END { print n + 0 }')
if [ "$lines" -gt "${MAGNUS_PR_LINES:-800}" ]; then
  echo "::warning::this pull request changes $lines lines outside tests and documents: large changes are reviewed slowly and go stale; split it into pull requests that each stand alone, if it can be."
fi

[ "$broken" = 0 ] || exit 4
echo "pull request #$pr follows the rules ($lines lines outside tests and documents)"
