#!/bin/sh
# Which tests a pull request's changed paths reach (owner, 2026-11-04:
# "Why does every test need to run on every PR? Most of them are small
# changes"). The changed paths come on standard input, one per line; it
# prints, for $GITHUB_ENV:
#
#   MAGNUS_SELECT=full   everything: installers, every suite, the update
#                        builds and the installed-app tests (desktop/, the
#                        build, the workflow, or any path it does not know);
#   MAGNUS_SELECT=tests  only the suites that MAGNUS_FILTER (a cargo-nextest
#                        filterset) picks; no installers or installed app;
#   MAGNUS_SELECT=none   no tests: documents only.
#
# Master's run and the merge queue's run everything; only a pull request's
# run is selected (.github/workflows/desktop.yml). A path it does not know
# runs everything, so a new directory is never left untested.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
full=0
terms=""
add() { terms="$terms
$1"; }
scratch() { add "binary_id(/^magnus-scratch::($1)/)"; }

while IFS= read -r f || [ -n "$f" ]; do
  [ -n "$f" ] || continue
  case $f in
  # Documents: nothing to test.
  *.md | docs/* | LICENSE*) ;;
  # The pull request rules and their workflows: their own test.
  release/pr-rules.sh | release/pr-stale.sh | .github/workflows/pr-rules.yml | .github/workflows/pr-stale.yml) add "binary_id(=magnus-scratch::pr_rules)" ;;
  # Workflows other than this one run their own jobs.
  .github/workflows/injection.yml | .github/workflows/appliance.yml | .github/pull_request_template.md) ;;
  # The installed-app tests and the update test run with the installers.
  tests/scratch/tests/desktop_app.rs) full=1 ;;
  # One scratch test file: that test (a deleted one: the scratch tests,
  # since a filter naming a test that is gone fails).
  tests/scratch/tests/*.rs)
    t=${f#tests/scratch/tests/}
    if [ -f "$root/$f" ]; then add "binary_id(=magnus-scratch::${t%.rs})"; else add "package(=magnus-scratch)"; fi
    ;;
  tests/scratch/*) add "package(=magnus-scratch)" ;;
  # The invariants' helpers are the scratch tests' too.
  tests/invariants/*) add "package(=magnus-invariants)"; add "package(=magnus-scratch)" ;;
  # A connector: its crate and its scratch tests.
  brokered/gmail/*) add "package(=magnus-gmail)"; scratch "brokered|gmail" ;;
  brokered/calendar/*) add "package(=magnus-calendar)"; scratch "brokered|calendar" ;;
  brokered/slack/*) add "package(=magnus-slack)"; scratch "brokered|slack" ;;
  brokered/chat/*) add "package(=magnus-chat)"; scratch "brokered|google_chat" ;;
  brokered/google/*) add "package(=magnus-brokered-google)"; scratch "brokered|google|gmail|calendar" ;;
  # What every connector shares: every connector crate and scratch test.
  brokered/*) add "package(/^magnus-(brokered-.*|gmail|calendar|slack|chat)$/)"; add "package(=magnus-scratch)" ;;
  # The appliance's release scripts are exercised by the scratch tests.
  release/admin-on-appliance.sh | release/admin-section.py | release/to-appliance.sh | release/appliance-programs | release/brokered-images/*) add "package(=magnus-scratch)" ;;
  # The kernel and its adapters (and what is compiled into them): every
  # suite, still without the installers.
  crates/* | catalog/* | config/* | eval/*) add "all()" ;;
  # desktop/, sandbox/, browser/, dev/, Cargo.toml, Cargo.lock, the
  # toolchain, .config/, this workflow, release/publish.sh, this script,
  # and anything new.
  *) full=1 ;;
  esac
done

if [ "$full" = 1 ]; then
  echo "MAGNUS_SELECT=full"
elif [ -z "$terms" ]; then
  echo "MAGNUS_SELECT=none"
elif printf '%s\n' "$terms" | grep -qx 'all()'; then
  echo "MAGNUS_SELECT=tests"
  echo "MAGNUS_FILTER=all()"
else
  echo "MAGNUS_SELECT=tests"
  echo "MAGNUS_FILTER=$(printf '%s\n' "$terms" | sed '/^$/d' | sort -u | awk 'NR > 1 { printf " | " } { printf "%s", $0 }')"
fi
