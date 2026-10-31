#!/usr/bin/env bash
# Put this checkout on the build box's own appliance (DECISIONS.md, "Master
# runs on the build box's appliance"): run by CI for every master build.
#
#   release/to-appliance.sh CONFIG VERSION NOTES_FILE
#
# CONFIG is that appliance's magnus.toml. Its programs (target/release, built
# already) and the browser sidecar (`browser/`, which the config's `[browser]
# command` runs from the install directory) go in as a signed appliance
# release, which the appliance installs by itself, with rollback
# (crates/magnus/src/update.rs); this waits until it runs VERSION. Then the sandbox image those programs were tested with
# (sandbox/image.sh) replaces the one the config names, as a link, so a
# running guest keeps the image it started with. The log's tables are
# brought up to date first when an `admin-url` file (the Postgres admin
# address, readable by its owner only) is beside the config.
set -euo pipefail
config=$1 version=$2 notes=$3
cd "$(dirname "$0")/.."

# The settings this needs, with the appliance's own defaults.
read -r listen updates minutes automatic kernel rootfs < <(python3 -I - "$config" <<'EOF'
import sys, tomllib, os
c = tomllib.load(open(sys.argv[1], "rb"))
a, u, s = c["appliance"], c.get("updates", {}), c["sandbox"]
print(a["listen"], u.get("dir") or os.path.join(a["state_dir"], "updates"), u.get("check_minutes", 60),
      str(u.get("automatic", True)).lower(), s["kernel_image"], s["rootfs"])
EOF
)

admin="$(dirname "$config")/admin-url"
if [ -r "$admin" ]; then
  target/release/magnus migrate --config "$config" --admin-url "$(cat "$admin")"
else
  echo "::warning::no $admin: the log's tables were not migrated"
fi

npm ci --prefix browser --omit=dev
release/publish.sh appliance "$version" "$updates" "$notes" \
  target/release/magnus target/release/magnus-adapter-anthropic target/release/magnus-adapter-openai browser
published=$(python3 -I -c 'import json, sys; print(json.load(open(sys.argv[1]))["version"])' "$updates/appliance/latest.json")
[ "$published" = "$version" ] || exit 0
if [ "$automatic" != true ] || [ "$minutes" = 0 ]; then
  echo "::warning::the appliance does not look for releases by itself: install $version from the app"
  exit 0
fi

# A look every `check_minutes`, a minute for running tasks' current step,
# and the start.
deadline=$(( $(date +%s) + (minutes + 3) * 60 ))
until [ "$(curl -fsS "http://$listen/v1/version" 2>/dev/null | python3 -I -c 'import json, sys; print(json.load(sys.stdin)["version"])' 2>/dev/null)" = "$version" ]; do
  [ "$(date +%s)" -lt "$deadline" ] || { echo "the appliance did not start $version within $((minutes + 3)) minutes" >&2; exit 1; }
  sleep 10
done
echo "the appliance runs $version"

image=$(sandbox/image.sh)
for f in "vmlinux $kernel" "rootfs.ext4 $rootfs"; do
  set -- $f
  ln -sfn "$image/$1" "$2.new" && mv -Tf "$2.new" "$2"
done
echo "its sandbox image is $image"
