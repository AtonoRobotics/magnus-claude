#!/bin/sh
# Pull requests that nobody touches pile up and waste CI (CLAUDE.md, "How
# work lands"). For each open pull request, activity is the latest of its
# opening, its commits, its reviews and its comments, ignoring this
# script's own comments. Then:
# - MAGNUS_STALE_HOURS (default 24): one comment naming what it waits on;
# - MAGNUS_STALE_LABEL_HOURS (default 72): the `stale` label;
# - MAGNUS_STALE_CLOSE_HOURS (default 120): close it. The branch is kept.
#   The `hold` label stops the close.
# The comment is not activity, so a pull request still ages while this
# script reminds it. Run by .github/workflows/pr-stale.yml with GH_TOKEN.
#
#   release/pr-stale.sh --decide HOURS [hold]
# prints the actions for that age, one per line: comment, label, close.
set -eu

comment_hours=${MAGNUS_STALE_HOURS:-24}
label_hours=${MAGNUS_STALE_LABEL_HOURS:-72}
close_hours=${MAGNUS_STALE_CLOSE_HOURS:-120}
marker='<!-- magnus-stale -->'

decide() {
  hours=$1
  hold=${2:-}
  if [ "$hours" -ge "$comment_hours" ]; then echo comment; fi
  if [ "$hours" -ge "$label_hours" ]; then echo label; fi
  if [ "$hours" -ge "$close_hours" ] && [ "$hold" != hold ]; then echo close; fi
}

if [ "${1:-}" = --decide ]; then
  decide "${2:?hours}" "${3:-}"
  exit 0
fi

git fetch -q origin master
now=$(date -u +%s)
gh label create stale --color B60205 --description "No activity for three days. Closed after five." >/dev/null 2>&1 || true

gh pr list --state open --limit 100 --json number -q '.[].number' |
while read -r n; do
  json=$(gh pr view "$n" --json createdAt,commits,reviews,comments,labels,headRefName,title)
  parsed=$(printf '%s' "$json" | python3 -I -c '
import json, sys
d = json.load(sys.stdin)
marker = "<!-- magnus-stale -->"
times = [d.get("createdAt") or ""]
times += [(c.get("committedDate") or "") for c in d.get("commits") or []]
times += [(r.get("submittedAt") or "") for r in d.get("reviews") or []]
comments = d.get("comments") or []
times += [(c.get("createdAt") or "") for c in comments if marker not in (c.get("body") or "")]
times = [t for t in times if t]
stale = [(c.get("createdAt") or "") for c in comments if marker in (c.get("body") or "")]
stale = [t for t in stale if t]
labels = [l.get("name") for l in d.get("labels") or []]
print(max(times))
print("hold" if "hold" in labels else "open")
print(max(stale) if stale else "")
print(d.get("headRefName") or "")
')
  iso=$(printf '%s\n' "$parsed" | sed -n 1p)
  hold=$(printf '%s\n' "$parsed" | sed -n 2p)
  last_note=$(printf '%s\n' "$parsed" | sed -n 3p)
  branch=$(printf '%s\n' "$parsed" | sed -n 4p)
  age=$(( (now - $(date -u -d "$iso" +%s)) / 3600 ))
  do_comment=0 do_label=0 do_close=0
  for action in $(decide "$age" "$hold"); do
    case $action in
    comment) do_comment=1 ;;
    label) do_label=1 ;;
    close) do_close=1 ;;
    esac
  done
  if [ -n "$last_note" ]; then
    note_age=$(( (now - $(date -u -d "$last_note" +%s)) / 3600 ))
    [ "$note_age" -ge 20 ] || do_comment=0
  fi
  # Closing says the same thing; one comment is enough.
  [ "$do_close" = 0 ] || do_comment=0

  waits=""
  # Behind master is not a reason to merge master in (CLAUDE.md, "How work
  # lands"); a conflict is.
  if [ "$(gh pr view "$n" --json mergeable -q .mergeable)" = CONFLICTING ]; then
    waits="$waits
- It conflicts with master: merge master into it to resolve the conflict."
  fi
  checks=0
  gh pr checks "$n" >/dev/null 2>&1 || checks=$?
  case $checks in
  0) waits="$waits
- Its checks pass: its session merges it once no review thread waits on an answer." ;;
  8) waits="$waits
- Its checks are still running." ;;
  *) waits="$waits
- A check fails: fix it, or close this pull request." ;;
  esac

  if [ "$do_label" = 1 ]; then
    gh pr edit "$n" --add-label stale >/dev/null
  fi
  if [ "$do_comment" = 1 ]; then
    gh pr comment "$n" --body "$marker
This pull request has had no activity for $age hours. What it waits on:$waits

It is labelled stale after three days and closed after five; the branch is kept. The hold label stops the close (CLAUDE.md, \"How work lands\")."
  fi
  if [ "$do_close" = 1 ]; then
    gh pr comment "$n" --body "$marker
Closing: no activity for $age hours. The branch \`$branch\` is kept. Reopen this pull request, or take what is worth keeping onto a new branch from current master.

What it was waiting on:$waits"
    gh pr close "$n" >/dev/null
  fi
done
