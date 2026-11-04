#!/bin/sh
# The rules every pull request is checked against in CI (CLAUDE.md, "How
# work lands"; DECISIONS.md, "Several pull requests at once"). Run in a
# checkout that can see the pull request and master:
#
#   release/pr-rules.sh PR HEAD BASE_BRANCH MASTER
#
# PR is this pull request's number, HEAD its head commit, BASE_BRANCH the
# branch it asks to merge into, MASTER master's current commit.
#
# MAGNUS_OPEN_PRS lists the open pull requests as "number:agent" or
# "number:human" (CI asks GitHub). An agent pull request is one a coding
# agent opened: a cursor/ branch, a bot author, the agent label, or a
# Claude Code body. MAGNUS_PR_AGENT=1 marks this pull request as one when
# it is not in that list yet. MAGNUS_PR_CAP (default 6) is how many agent
# pull requests may be open; MAGNUS_PR_WARN (default 4) is where a warning
# starts. Only a pull request past the cap fails, and only if it is an
# agent pull request: the older ones stay green, and a person's does too.
#
# It fails (exit 4), naming each rule broken, when:
# - this agent pull request is past the cap of open agent pull requests;
# - it asks to merge into anything but master: no stacked pull requests.
# It warns, without failing, when:
# - the branch does not hold master's current commit (merge master in
#   before merging; the development channel will not publish over a build
#   this lacks);
# - this agent pull request is past the warning line but within the cap;
# - the change is large (more than MAGNUS_PR_LINES lines, default 800,
#   outside tests and documents).
set -eu

pr=$1 head=$2 base_branch=$3 master=$4
broken=0
fail() { echo "::error::$1"; broken=1; }
warn() { echo "::warning::$1"; }

cap=${MAGNUS_PR_CAP:-6}
soft=${MAGNUS_PR_WARN:-4}
agents=""
kind=""
for item in ${MAGNUS_OPEN_PRS:-}; do
  n=${item%%:*}
  k=${item#*:}
  case $k in
  agent|human) ;;
  *) k=human ;;
  esac
  if [ "$n" = "$pr" ]; then kind=$k; fi
  if [ "$k" = agent ]; then agents="$agents $n"; fi
done
if [ -z "$kind" ]; then
  if [ "${MAGNUS_PR_AGENT:-0}" = 1 ]; then
    kind=agent
    agents="$agents $pr"
  else
    kind=human
  fi
fi

if [ "$kind" = agent ] && [ -n "$agents" ]; then
  rank=0 pos=0 count=0 older=""
  for n in $(printf '%s\n' $agents | sort -n); do
    count=$((count + 1))
    if [ "$count" -le "$cap" ]; then older="$older #$n"; fi
    if [ "$n" = "$pr" ]; then rank=$count; fi
  done
  if [ "$rank" -gt "$cap" ]; then
    fail "this is agent pull request $rank of $count open, past the cap of $cap. Already open:$older. Merge or close one of those before opening another; this check does not fail them."
  elif [ "$rank" -gt "$soft" ]; then
    warn "this is agent pull request $rank of $count open. The cap is $cap; finish one before opening another."
  fi
fi

if [ "$base_branch" != master ]; then
  fail "this pull request merges into $base_branch: every pull request merges into master, from a branch made from current master (no stacked pull requests)."
fi
if ! git merge-base --is-ancestor "$master" "$head" 2>/dev/null; then
  warn "this branch does not hold master's current commit $(git rev-parse --short "$master" 2>/dev/null || echo "$master"): merge master into it before merging (git fetch origin master && git merge origin/master), unless the merge queue lands it. Development is not published while that would replace the owner's app with a build that lacks work it already has."
fi

lines=$(git diff --numstat "$(git merge-base "$master" "$head")" "$head" -- . ':(exclude)tests' ':(exclude)*.md' ':(exclude)docs' 2>/dev/null | awk '$1 != "-" { n += $1 + $2 } END { print n + 0 }')
if [ "$lines" -gt "${MAGNUS_PR_LINES:-800}" ]; then
  warn "this pull request changes $lines lines outside tests and documents: large changes are reviewed slowly and go stale; split it into pull requests that each stand alone, if it can be."
fi

[ "$broken" = 0 ] || exit 4
echo "pull request #$pr follows the rules ($lines lines outside tests and documents)"
