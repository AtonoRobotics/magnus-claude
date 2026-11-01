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
# The test server is throwaway (each test makes its own database and drops
# it), so it runs without durability, as Postgres's documentation advises
# for that case ("Non-Durable Settings"): every test's DROP DATABASE forces
# a checkpoint, and with fsync each waited up to 33 s on a disk the builds
# beside the tests were writing, while every test's commits waited behind
# it (CI run 41266235376). Applied on reload; never on the appliance's own
# Postgres (config/install-postgres.sh). After a power cut its data may be
# damaged: it holds only test databases, so it is made again empty, from
# dev/postgres.Dockerfile on a new volume.
for s in "fsync = off" "synchronous_commit = off" "full_page_writes = off"; do
  docker exec magnus-claude-pg psql -q -U postgres -c "ALTER SYSTEM SET $s"
done
docker exec magnus-claude-pg psql -q -U postgres -c "SELECT pg_reload_conf()" >/dev/null
echo "Postgres and Keycloak are up"
