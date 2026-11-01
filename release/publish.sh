#!/bin/sh
# Publish a signed release into an appliance's updates directory, where the
# appliance installs its own releases and serves the desktop app's
# (DECISIONS.md, "Appliance updates"). Run by CI on the build box after the
# release's tests pass; the signing key never leaves that machine.
#
#   release/publish.sh appliance VERSION UPDATES_DIR NOTES_FILE PROGRAM_OR_DIR...
#   release/publish.sh desktop   VERSION UPDATES_DIR NOTES_FILE DIST_DIR [CHANNEL]
#
# The desktop app's CHANNEL (DECISIONS.md, "Release channels") is
# production (the default: tagged releases, in desktop/), test (master, in
# desktop/test/) or development (pull requests, in desktop/development/);
# test and development keep only their latest packages; the appliance's
# directory its latest and the one before.
#
# Every package is signed with `tauri signer sign --app-version`, so the
# signature binds the version it was published as. latest.json is written
# last, by rename, so an appliance never reads a release half-written.
#
# A channel only ever moves forward, in version and in content. A release is
# not published when it is older than the one there (a slower run finishing
# after a newer one), nor when it was built without the commit the one there
# was built from (a pull request lacking another's work, or a tag of an
# older commit): an app would lose what it has. latest.json records that
# commit: MAGNUS_COMMIT (a pull request's head), or HEAD; the release is the
# checkout at HEAD (for a pull request, its merge with master).
set -eu

what=$1 version=$2 updates=$3 notes=$4
shift 4
key=${MAGNUS_RELEASE_KEY:-$HOME/.magnus-release/update.key}
commit=${MAGNUS_COMMIT:-$(git rev-parse HEAD)}
[ -r "$key" ] || { echo "no release key at $key" >&2; exit 1; }

sign() {
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" cargo tauri signer sign -f "$key" -p "" --app-version "$version" "$1" >/dev/null
  cat "$1.sig"
}

# Whether VERSION is older than the one $1/latest.json names, by semantic
# version precedence (semver.org, item 11).
older() {
  [ -f "$1/latest.json" ] || return 1
  python3 -I - "$1/latest.json" "$version" <<'EOF'
import json, sys
def key(v):
    core, _, pre = v.split("+")[0].partition("-")
    ids = [(0, int(i), "") if i.isdigit() else (1, 0, i) for i in pre.split(".")] if pre else []
    return [int(n) for n in core.split(".")], not pre, ids
sys.exit(0 if key(sys.argv[2]) < key(json.load(open(sys.argv[1]))["version"]) else 1)
EOF
}

# The commit $1/latest.json was built from, if it names one.
built_from() {
  [ -f "$1/latest.json" ] && python3 -I -c 'import json, sys; print(json.load(open(sys.argv[1])).get("commit", ""))' "$1/latest.json"
}

# latest.json from its fields, by python so the notes are quoted correctly.
manifest() {
  python3 -I - "$@" <<'EOF'
import json, sys, datetime
out, kind, version, notes, commit = sys.argv[1:6]
m = {"version": version, "notes": open(notes).read().strip(), "pub_date": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"), "commit": commit}
rest = sys.argv[6:]
if kind == "appliance":
    m["file"], m["signature"] = rest[0], rest[1]
else:
    m["platforms"] = {rest[i]: {"file": rest[i + 1], "signature": rest[i + 2]} for i in range(0, len(rest), 3)}
open(out + ".tmp", "w").write(json.dumps(m, indent=2))
EOF
}

case $what in
appliance) dir=$updates/appliance ;;
desktop)
  channel=${2:-production}
  case $channel in
  production) dir=$updates/desktop ;;
  test | development) dir=$updates/desktop/$channel ;;
  *) echo "no channel $channel: production, test or development" >&2; exit 2 ;;
  esac
  ;;
*) echo "publish appliance or desktop" >&2; exit 2 ;;
esac
if older "$dir"; then
  echo "not publishing $what $version: $dir/latest.json names a newer version" >&2
  exit 0
fi
was=$(built_from "$dir" || true)
if [ -n "$was" ] && ! git merge-base --is-ancestor "$was" HEAD 2>/dev/null; then
  echo "not publishing $what $version: it was built without $was, which $dir/latest.json was built from; bring this branch up to date with that commit's work" >&2
  exit 3
fi

case $what in
appliance)
  previous=$( [ -f "$dir/latest.json" ] && python3 -I -c 'import json, sys; print(json.load(open(sys.argv[1]))["file"])' "$dir/latest.json" || true)
  mkdir -p "$dir"
  stage=$(mktemp -d)
  trap 'rm -rf "$stage"' EXIT
  for p in "$@"; do cp -r "$p" "$stage/"; done
  echo "$version" > "$stage/VERSION"
  file=magnus-appliance-$version.tar.gz
  tar -C "$stage" -czf "$dir/$file" .
  manifest "$dir/latest.json" appliance "$version" "$notes" "$commit" "$file" "$(sign "$dir/$file")"
  ;;
desktop)
  dist=$1
  mkdir -p "$dir"
  linux=Magnus_${version}_amd64.AppImage
  windows=Magnus_${version}_x64-setup.exe
  cp "$dist/$linux" "$dist/$windows" "$dir/"
  manifest "$dir/latest.json" desktop "$version" "$notes" "$commit" \
    linux-x86_64 "$linux" "$(sign "$dir/$linux")" \
    windows-x86_64 "$windows" "$(sign "$dir/$windows")"
  ;;
esac
mv "$dir/latest.json.tmp" "$dir/latest.json"
# The appliance's packages: this one and the one before it; the installed
# releases themselves are kept by the appliance (`<dir>.prev`).
if [ "$what" = appliance ]; then
  for f in "$dir"/magnus-appliance-*; do
    case $f in "$dir/$file" | "$dir/$file.sig" | "$dir/$previous" | "$dir/$previous.sig") ;; *) rm -f "$f" ;; esac
  done
fi
if [ "$what" = desktop ] && [ "$channel" != production ]; then
  for f in "$dir"/Magnus_*; do
    case $f in "$dir/$linux" | "$dir/$windows" | "$dir/$linux.sig" | "$dir/$windows.sig") ;; *) rm -f "$f" ;; esac
  done
fi
echo "published $what $version to $dir"
