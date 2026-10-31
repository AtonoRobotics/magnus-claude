#!/bin/sh
# Remove pull requests' review channels from an appliance's updates
# directory (DECISIONS.md, "Release channels"); the apps on a removed
# channel move to preview. Run by CI on the build box: for a pull request
# closed without merging, and after each preview, for every pull request no
# longer open (a merged one's changes are in that preview).
#
#   release/retire.sh UPDATES_DIR pr-<n>...             remove these channels
#   release/retire.sh UPDATES_DIR --except [pr-<n>...]  remove all but these
set -eu

desktop=$1/desktop
shift
pr() { case $1 in pr-[0-9]*) [ -z "$(echo "${1#pr-}" | tr -d 0-9)" ] ;; *) false ;; esac; }

if [ "${1:-}" = --except ]; then
  shift
  for d in "$desktop"/pr-*; do
    [ -d "$d" ] || continue
    c=${d##*/}
    pr "$c" || continue
    case " $* " in *" $c "*) continue ;; esac
    rm -rf "$d"
    echo "retired $c"
  done
else
  for c in "$@"; do
    pr "$c" || { echo "not a pull request channel: $c" >&2; exit 2; }
    [ -d "$desktop/$c" ] || continue
    rm -rf "$desktop/$c"
    echo "retired $c"
  done
fi
