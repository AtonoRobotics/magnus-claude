#!/usr/bin/env bash
# The admin console on the build box's own appliance (DECISIONS.md, "The
# admin console on strix-alpha"): run by CI after each master build is on
# the appliance (release/to-appliance.sh), and safe to run again.
#
#   release/admin-on-appliance.sh CONFIG PROGRAMS_DIR [--config-only]
#
# --config-only stops once [admin] and its certificate are written (the
# test, tests/scratch/tests/admin_on_appliance.rs). MAGNUS_ADMIN_LAN
# ("address/prefix") and MAGNUS_ADMIN_TAILSCALE ("address", or "none") name
# the networks when `ip` should not be asked.
#
# Once, when CONFIG has no [admin] section, it chooses what the admin
# console spec leaves to the organisation and writes it there:
#   - the hostname: this machine's name on the local network, NAME.local
#     (mDNS), on port $PORT;
#   - listen: the machine's address on the network its default route uses,
#     and its Tailscale address when it has one; never every address or
#     loopback;
#   - allow: those networks only (the LAN's subnet, and Tailscale's
#     100.64.0.0/10);
#   - the certificate: from the organisation's own CA, made here (its key
#     owner-only, beside CONFIG in admin-tls/), naming the hostname and those
#     addresses; the owner trusts the CA once on their computers;
#   - the console's own database role, with a password made here.
# Then, each time: the role in the database (`magnus migrate`), the
# console's client and service account in the realm and the realm role
# `admin` for the appliance's owners (config/keycloak-client.py, as root,
# which reads the realm administrator's file), and the console as a systemd
# user service beside the appliance, restarted on the programs installed.
# When GOOGLE_CLIENT_SECRET is set, Google's client secret goes into the
# appliance's vault through its owner-only API; it is never printed.
#
# It prints what a person needs to reach the console: its address and the
# CA's certificate (public). It prints no secret.
set -euo pipefail
config=$1 programs=$2 only=${3:-}
port=${MAGNUS_ADMIN_PORT:-7443}
cd "$(dirname "$0")/.."
here=$(dirname "$config")
tls="$here/admin-tls"

# The settings this needs, from the appliance's config.
read -r listen state_dir install < <(python3 -I - "$config" <<'EOF'
import sys, tomllib, os
c = tomllib.load(open(sys.argv[1], "rb"))
print(c["appliance"]["listen"], c["appliance"]["state_dir"], os.path.dirname(c.get("sandbox", {}).get("kernel_image", "")) or "-")
EOF
)
has_admin=$(python3 -I -c 'import sys, tomllib; print("admin" in tomllib.load(open(sys.argv[1], "rb")))' "$config")

if [ "$has_admin" != True ]; then
  host="$(hostname -s).local"
  # The default route's interface and address, and that network.
  lan=${MAGNUS_ADMIN_LAN:-}
  if [ -z "$lan" ]; then
    dev=$(ip -o route get 1.1.1.1 | sed -n 's/.* dev \([^ ]*\).*/\1/p')
    lan=$(ip -o -4 addr show dev "$dev" | awk '{print $4}' | head -1)
  fi
  addr=${lan%/*}
  net=$(python3 -I -c 'import ipaddress, sys; print(ipaddress.ip_interface(sys.argv[1]).network)' "$lan")
  listen_list="\"$addr:$port\""
  allow_list="\"$net\""
  sans="DNS:$host,IP:$addr"
  ts=${MAGNUS_ADMIN_TAILSCALE:-$(ip -o -4 addr show dev tailscale0 2>/dev/null | awk '{print $4}' | head -1 || true)}
  [ "$ts" = none ] && ts=""
  if [ -n "$ts" ]; then
    listen_list="$listen_list, \"${ts%/*}:$port\""
    allow_list="$allow_list, \"100.64.0.0/10\""
    sans="$sans,IP:${ts%/*}"
  fi

  # The organisation's CA and the console's certificate from it.
  mkdir -p "$tls" && chmod 700 "$tls"
  if [ ! -f "$tls/ca.pem" ]; then
    openssl req -x509 -new -nodes -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -days 3650 \
      -subj "/O=Magnus appliance $(hostname -s)/CN=Magnus organisation CA" \
      -addext "basicConstraints=critical,CA:true,pathlen:0" -addext "keyUsage=critical,keyCertSign,cRLSign" \
      -keyout "$tls/ca.key" -out "$tls/ca.pem" 2>/dev/null
  fi
  openssl req -new -nodes -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -subj "/CN=$host" \
    -keyout "$tls/console.key" -out "$tls/console.csr" 2>/dev/null
  printf 'subjectAltName=%s\nbasicConstraints=critical,CA:false\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\n' "$sans" > "$tls/console.ext"
  openssl x509 -req -in "$tls/console.csr" -CA "$tls/ca.pem" -CAkey "$tls/ca.key" -CAcreateserial \
    -days 825 -extfile "$tls/console.ext" -out "$tls/console.pem" 2>/dev/null
  chmod 600 "$tls"/*.key
  rm -f "$tls/console.csr"

  # The console's own database role: its password made here, in the config
  # only (owner-only, as the config is).
  db=$(python3 -I -c 'import sys, tomllib, urllib.parse as u; d = u.urlsplit(tomllib.load(open(sys.argv[1], "rb"))["database"]["url"]); print(f"{d.hostname}:{d.port}{d.path}")' "$config")
  root_password=$(openssl rand -hex 24)

  cp -p "$config" "$config.bak-$(date +%Y%m%d%H%M%S)"
  cat >> "$config" <<EOF

# The admin console (written by release/admin-on-appliance.sh; admin console spec).
[admin]
hostname = "$host"
listen = [$listen_list]
allow = [$allow_list]
tls_cert = "$tls/console.pem"
tls_key = "$tls/console.key"
database_url = "postgres://magnus_root:$root_password@$db"
EOF
  echo "wrote [admin] to $config: $host, listening on $listen_list, allowing $allow_list"
fi

[ "$only" = --config-only ] && exit 0

# The console's database role (and the log's tables, as each release does).
if [ -r "$here/admin-url" ]; then
  "$programs/magnus" migrate --config "$config" --admin-url "$(cat "$here/admin-url")"
fi

# The console's client and service account in the realm, and the realm role
# admin for the appliance's owners, with the console's own key.
host=$(python3 -I -c 'import sys, tomllib; print(tomllib.load(open(sys.argv[1], "rb"))["admin"]["hostname"])' "$config")
owners=$(python3 -I -c 'import sys, tomllib; print(" ".join(p["id"] for p in tomllib.load(open(sys.argv[1], "rb")).get("people", []) if "owner" in p.get("roles", [])))' "$config")
jwks=$(mktemp)
trap 'rm -f "$jwks"' EXIT
"$programs/magnus-admin" jwk --config "$config" > "$jwks"
assign=()
for o in $owners; do assign+=(--assign-admin "$o"); done
sudo -n python3 -I config/keycloak-client.py --admin-console "$jwks" --admin-hostname "$host:$port" "${assign[@]}"

# The console as a user service beside the appliance, on the installed
# programs; restarted so it runs this release.
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
unit="$HOME/.config/systemd/user/magnus-admin.service"
mkdir -p "$(dirname "$unit")"
cat > "$unit.new" <<EOF
[Unit]
Description=Magnus admin console
After=network-online.target

[Service]
ExecStart=$install/magnus-admin --config $config
Restart=always
RestartSec=5

[Install]
WantedBy=default.target
EOF
mv "$unit.new" "$unit"
systemctl --user daemon-reload
systemctl --user enable magnus-admin.service >/dev/null
systemctl --user restart magnus-admin.service
sleep 3
systemctl --user is-active magnus-admin.service

# NAME.local answers on the network when mDNS runs here.
if systemctl list-unit-files avahi-daemon.service >/dev/null 2>&1; then
  sudo -n systemctl enable --now avahi-daemon.service >/dev/null 2>&1 || true
fi

# Google's client secret, into the appliance's vault through its owner-only
# API, when the repository's secret is set.
if [ -n "${GOOGLE_CLIENT_SECRET:-}" ]; then
  token=$(python3 -I -c 'import sys, tomllib; print(next(p["token"] for p in tomllib.load(open(sys.argv[1], "rb"))["people"] if "owner" in p.get("roles", []) and p.get("token")))' "$config")
  code=$(python3 -I -c 'import json, os; print(json.dumps({"client_secret": os.environ["GOOGLE_CLIENT_SECRET"]}))' |
    curl -sS -o /dev/null -w '%{http_code}' -X PUT -H "Authorization: Bearer $token" -H 'Content-Type: application/json' --data-binary @- "http://$listen/v1/registrations/google")
  echo "Google's client secret into the appliance's vault: HTTP $code"
else
  echo "GOOGLE_CLIENT_SECRET is not set in the repository: Google's connectors cannot finish signing in yet"
fi

# What a person needs to reach it.
first=$(python3 -I -c 'import sys, tomllib; print(tomllib.load(open(sys.argv[1], "rb"))["admin"]["listen"][0].rsplit(":", 1)[0])' "$config")
echo "The admin console: https://$host:$port/ (on the network; $host is $first)"
echo "If $host does not resolve on a computer, add this line to its hosts file: $first $host"
if [ -f "$tls/ca.pem" ]; then
  echo "Trust this CA once on each computer that opens the console (it is public):"
  cat "$tls/ca.pem"
  echo "SHA-256 fingerprint: $(openssl x509 -in "$tls/ca.pem" -noout -fingerprint -sha256 | cut -d= -f2)"
fi
