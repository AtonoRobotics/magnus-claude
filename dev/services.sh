#!/bin/sh
# Start the build box's dev and test services, `magnus-claude-pg` and
# `magnus-claude-kc` (dev/keycloak.sh), and wait until both answer. CI runs
# this before any test: the runner can take a job before the containers are
# back after a reboot (2026-10-31, an injection run failed so). Both start in
# seconds here (Keycloak in 5-7 s); two minutes is the limit.
set -eu
here=$(dirname "$0")
docker start magnus-claude-pg >/dev/null
"$here/keycloak.sh" >/dev/null
end=$(( $(date +%s) + 120 ))
until docker exec magnus-claude-pg pg_isready -q -U postgres && curl -fsS -o /dev/null http://127.0.0.1:54380/realms/master; do
  [ "$(date +%s)" -lt "$end" ] || { echo "Postgres or Keycloak did not answer within two minutes" >&2; exit 1; }
  sleep 2
done
echo "Postgres and Keycloak are up"
