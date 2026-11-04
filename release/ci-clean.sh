#!/bin/sh
# What tests left behind on the build box's test services, cleared at the
# start of each release-checks run (.github/workflows/desktop.yml). A test
# that is killed (its time limit, a cancelled run) does not get to delete
# its Keycloak realm or stop its mail sink; fifty such realms slowed every
# sign-in on the shared test Keycloak (2026-11-04).
#
#   release/ci-clean.sh
#
# Deletes, on the test Keycloak (`magnus-claude-kc`, dev/keycloak.sh), the
# test realms (t and a UUID's hex) it has seen for MAGNUS_CLEAN_AGE seconds
# (default three hours, past any run's time limit), and stops the mail-sink
# containers (tests/scratch/src/mail.rs) that old. Keycloak keeps no
# creation time for a realm, so a realm's age is from the first run that
# saw it, recorded in MAGNUS_CLEAN_SEEN (default
# ~/magnus-ci/keycloak-realms-seen). Nothing else is touched: not the
# master realm, not the appliance's own Keycloak. It never fails the run.
#
#   release/ci-clean.sh --decide SEEN NOW
#
# The decision alone: the realm names on standard input; prints those to
# delete and rewrites SEEN ("name first-seen" per line) with the rest.
set -u
age=${MAGNUS_CLEAN_AGE:-10800}

decide() {
  seen=$1 now=$2
  touch "$seen"
  next="$seen.next"
  : > "$next"
  while IFS= read -r name; do
    # A test's realm: "t" and a UUID's 32 hex digits (keycloak.rs).
    printf '%s\n' "$name" | grep -Eqx 't[0-9a-f]{32}' || continue
    first=$(awk -v n="$name" '$1 == n { print $2; exit }' "$seen")
    if [ -n "$first" ] && [ $((now - first)) -ge "$age" ]; then
      echo "$name"
    else
      echo "$name ${first:-$now}" >> "$next"
    fi
  done
  mv "$next" "$seen"
}

if [ "${1:-}" = --decide ]; then
  decide "$2" "$3"
  exit 0
fi

now=$(date +%s)
kc=${MAGNUS_TEST_KEYCLOAK:-http://127.0.0.1:54380}
seen=${MAGNUS_CLEAN_SEEN:-$HOME/magnus-ci/keycloak-realms-seen}
mkdir -p "$(dirname "$seen")"

# The test Keycloak's admin, as the tests sign in (tests/scratch/src/keycloak.rs).
token=$(curl -sf --max-time 20 "$kc/realms/master/protocol/openid-connect/token" \
  -d grant_type=password -d client_id=admin-cli \
  --data-urlencode "username=${MAGNUS_TEST_KEYCLOAK_ADMIN:-admin}" \
  --data-urlencode "password=${MAGNUS_TEST_KEYCLOAK_PASSWORD:-magnus_dev}" |
  python3 -I -c 'import json, sys; print(json.load(sys.stdin)["access_token"])' 2>/dev/null) || token=""
if [ -n "$token" ]; then
  names=$(curl -sf --max-time 60 -H "Authorization: Bearer $token" "$kc/admin/realms?briefRepresentation=true" |
    python3 -I -c 'import json, sys; print("\n".join(r["realm"] for r in json.load(sys.stdin)))' 2>/dev/null) || names=""
  n=0
  for realm in $(printf '%s\n' "$names" | decide "$seen" "$now"); do
    curl -sf --max-time 60 -X DELETE -H "Authorization: Bearer $token" "$kc/admin/realms/$realm" >/dev/null && n=$((n + 1))
  done
  echo "test Keycloak: deleted $n test realms left for $((age / 3600))h or more; $(wc -l < "$seen") newer ones kept"
else
  echo "test Keycloak: not answering its admin, nothing deleted"
fi

kc_id=$(docker inspect -f '{{.Id}}' magnus-claude-kc 2>/dev/null) || kc_id=""
n=0
if [ -n "$kc_id" ]; then
  for c in $(docker ps -q --no-trunc); do
    set -- $(docker inspect -f '{{.Path}} {{.HostConfig.NetworkMode}} {{.Created}}' "$c" 2>/dev/null) || continue
    [ "${1:-}" = /mail-sink ] || continue
    case ${2:-} in "container:$kc_id" | container:magnus-claude-kc) ;; *) continue ;; esac
    created=$(date -d "${3:-}" +%s 2>/dev/null) || continue
    if [ $((now - created)) -ge "$age" ]; then
      docker rm -f "$c" >/dev/null 2>&1 && n=$((n + 1))
    fi
  done
fi
echo "mail sinks: stopped $n left for $((age / 3600))h or more"
exit 0
