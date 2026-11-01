#!/usr/bin/env bash
# Put this checkout on the build box's own appliance (DECISIONS.md, "Master
# runs on the build box's appliance"): run by CI for every master build.
#
#   release/to-appliance.sh CONFIG VERSION NOTES_FILE
#
# CONFIG is that appliance's magnus.toml. Its programs (target/release, built
# already), the browser sidecar (`browser/`, which the config's `[browser]
# command` runs from the install directory) and the sandbox image those
# programs were tested with (sandbox/image.sh) go in as a signed appliance
# release, which the appliance installs by itself, with rollback
# (crates/magnus/src/update.rs); this waits until it runs VERSION.
#
# The image goes in as two links, named as the config's `[sandbox]
# kernel_image` and `rootfs`, which name files in the install directory: the
# links point at the appliance's own copy of the image, in `sandbox/` beside
# that directory (never into CI's store, which is pruned). So a release and
# its image are switched, and rolled back, together; the appliance keeps the
# images of its running release and of the one kept for rollback.
#
# The log's tables are brought up to date first when an `admin-url` file (the
# Postgres admin address, readable by its owner only) is beside the config.
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
install=$(dirname "$kernel")
[ "$(dirname "$rootfs")" = "$install" ] && [ -f "$install/VERSION" ] ||
  { echo "the config's [sandbox] kernel_image and rootfs must name files in the install directory (with its VERSION): $kernel, $rootfs" >&2; exit 2; }
own="$(dirname "$install")/sandbox"

admin="$(dirname "$config")/admin-url"
if [ -r "$admin" ]; then
  target/release/magnus migrate --config "$config" --admin-url "$(cat "$admin")"
else
  echo "::warning::no $admin: the log's tables were not migrated"
fi

# The appliance's own copy of the image, and the release's links to it.
image=$(sandbox/image.sh)
h=$(basename "$image")
if [ ! -f "$own/$h/rootfs.ext4" ]; then
  rm -rf "$own/$h.part" && mkdir -p "$own/$h.part"
  cp --sparse=always "$image/vmlinux" "$image/rootfs.ext4" "$own/$h.part/"
  mv -T "$own/$h.part" "$own/$h"
fi
links=$(mktemp -d)
trap 'rm -rf "$links"' EXIT
ln -s "../sandbox/$h/vmlinux" "$links/$(basename "$kernel")"
ln -s "../sandbox/$h/rootfs.ext4" "$links/$(basename "$rootfs")"

npm ci --prefix browser --omit=dev
release/publish.sh appliance "$version" "$updates" "$notes" \
  target/release/magnus target/release/magnus-adapter-anthropic target/release/magnus-adapter-openai browser \
  "$links/$(basename "$kernel")" "$links/$(basename "$rootfs")"
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
echo "the appliance runs $version, with the sandbox image $h"

# Images neither the running release nor the one kept for rollback uses.
keep=" $(readlink -f "$kernel" | xargs -r dirname) $(readlink -f "$install.prev/$(basename "$kernel")" 2>/dev/null | xargs -r dirname) "
for d in "$own"/*; do
  case $keep in *" $d "*) ;; *) rm -rf "$d" ;; esac
done
