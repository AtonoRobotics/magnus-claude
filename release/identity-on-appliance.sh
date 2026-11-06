#!/usr/bin/env bash
# The appliance's own identity provider: a Keycloak beside it, which sign-in
# (`[sign_in.oidc]`) and the admin console depend on (#290). Safe to run again.
#
#   release/identity-on-appliance.sh CONFIG TUNNEL_ORIGIN
#
# CONFIG is the appliance's magnus.toml. Everything else comes from it:
#   - the public name and the realm from `[sign_in.oidc] issuer`
#     (https://<name>/realms/<realm>);
#   - the private admin origin from `status_admin_url` (loopback, HTTPS), and
#     where its CA goes from `status_admin_ca`;
#   - the database server from `[database] url`, entered as the administrator
#     in `admin-url` beside CONFIG (the one `magnus migrate` uses);
#   - the people from `[[people]]`, the owner from the role `owner`, and the
#     appliance's API from `[appliance] listen`;
#   - the admin console's names from `[admin]`.
# TUNNEL_ORIGIN (loopback host:port) is where Keycloak answers plain HTTP for
# the tunnel, which serves the public name (docs/remote-access-and-relay.md).
# The image is the one dev/keycloak.sh pins.
#
# State beside CONFIG in identity/ (owner-only): the database password, the
# bootstrap administrator (admin.json, for config/keycloak-client.py), the
# admin origin's CA and certificate, and each person's first password
# (<id>.password, which Keycloak makes them change at first sign-in).
set -euo pipefail
[ $# -eq 2 ] || { sed -n 5,6p "$0" >&2; exit 2; }
CONFIG=$(realpath "$1"); ORIGIN=$2
HERE=$(cd "$(dirname "$0")/.." && pwd)
CONF_DIR=$(dirname "$CONFIG")
STATE=$CONF_DIR/identity
NAME=magnus-appliance-kc
umask 077
mkdir -p "$STATE/tls"

IMAGE=$(sed -n 's/^IMAGE=//p' "$HERE/dev/keycloak.sh")
[ -n "$IMAGE" ] || { echo "dev/keycloak.sh names no IMAGE" >&2; exit 1; }

eval "$(python3 -I - "$CONFIG" "$ORIGIN" <<'EOF'
import shlex, sys, tomllib
from urllib.parse import urlsplit
c = tomllib.load(open(sys.argv[1], "rb"))
o = c["sign_in"]["oidc"]
public, realm = o["issuer"].split("/realms/", 1)
admin = urlsplit(o["status_admin_url"])
origin = urlsplit("http://" + sys.argv[2])
db = urlsplit(c["database"]["url"])
if admin.scheme != "https" or admin.hostname not in ("127.0.0.1", "::1", "localhost"):
    sys.exit("status_admin_url must be a loopback https origin")
if origin.hostname not in ("127.0.0.1", "::1", "localhost") or not origin.port:
    sys.exit("TUNNEL_ORIGIN must be a loopback host:port")
owners = [p["id"] for p in c.get("people", []) if "owner" in p.get("roles", [])]
if not owners:
    sys.exit("no person holds the role owner")
adm = c.get("admin", {})
port = adm["listen"][0].rsplit(":", 1)[1] if adm.get("listen") else None
names = [adm["hostname"], *adm.get("aliases", [])] if adm.get("hostname") else []
v = {
    "PUBLIC": public, "REALM": realm,
    "ADMIN_URL": f"{admin.scheme}://{admin.netloc}", "ADMIN_HOST": admin.hostname, "ADMIN_PORT": str(admin.port or 443),
    "ADMIN_CA": o["status_admin_ca"], "ORIGIN_HOST": origin.hostname, "ORIGIN_PORT": str(origin.port),
    "CLIENT_ID": o["client_id"], "STATUS_CLIENT": o.get("status_client", ""), "STATUS_CREDENTIAL": o.get("status_credential", ""),
    "DB_HOST": db.hostname, "DB_PORT": str(db.port or 5432),
    "API": "http://" + c["appliance"]["listen"], "OWNER": owners[0],
    "PEOPLE": " ".join(p["id"] for p in c.get("people", [])),
    "CONSOLE_NAMES": " ".join(f"{n}:{port}" if port and port != "443" else n for n in names),
}
for k, x in v.items():
    print(f"{k}={shlex.quote(x)}")
EOF
)"

secret() { [ -s "$1" ] || openssl rand -hex 24 >"$1"; cat "$1"; }

# The database and its role, on the appliance's own database server.
DB_PASSWORD=$(secret "$STATE/db.password")
ADMIN_DB=$(cat "$CONF_DIR/admin-url")
psql "$ADMIN_DB" -v ON_ERROR_STOP=1 -q -v pw="$DB_PASSWORD" <<'SQL'
SELECT format('CREATE ROLE keycloak LOGIN PASSWORD %L', :'pw') WHERE NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'keycloak') \gexec
SELECT format('ALTER ROLE keycloak PASSWORD %L', :'pw') \gexec
SELECT 'CREATE DATABASE keycloak OWNER keycloak' WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = 'keycloak') \gexec
SQL

# The admin origin's certificate, from a CA of its own.
TLS=$STATE/tls
if [ ! -s "$TLS/ca.pem" ]; then
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 3650 \
    -subj "/CN=Magnus appliance identity CA" -keyout "$TLS/ca.key" -out "$TLS/ca.pem" 2>/dev/null
fi
if [ ! -s "$TLS/server.pem" ] || ! openssl verify -CAfile "$TLS/ca.pem" "$TLS/server.pem" >/dev/null 2>&1; then
  openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -subj "/CN=$ADMIN_HOST" \
    -keyout "$TLS/server.key" -out "$TLS/server.csr" 2>/dev/null
  printf 'subjectAltName=IP:127.0.0.1,IP:::1,DNS:localhost\nextendedKeyUsage=serverAuth\n' >"$TLS/ext"
  openssl x509 -req -in "$TLS/server.csr" -CA "$TLS/ca.pem" -CAkey "$TLS/ca.key" -CAcreateserial \
    -days 825 -extfile "$TLS/ext" -out "$TLS/server.pem" 2>/dev/null
fi
chmod 644 "$TLS/server.pem" "$TLS/ca.pem"; chmod 640 "$TLS/server.key"
install -m 644 "$TLS/ca.pem" "$ADMIN_CA"

# Keycloak: production mode, on loopback only.
ADMIN_PASSWORD=$(secret "$STATE/admin.password")
printf '{"username": "magnus-bootstrap", "password": "%s"}\n' "$ADMIN_PASSWORD" >"$STATE/admin.json"
cat >"$STATE/keycloak.env" <<EOF
KC_DB=postgres
KC_DB_URL=jdbc:postgresql://$DB_HOST:$DB_PORT/keycloak
KC_DB_USERNAME=keycloak
KC_DB_PASSWORD=$DB_PASSWORD
KC_HOSTNAME=$PUBLIC
KC_HOSTNAME_ADMIN=$ADMIN_URL
KC_PROXY_HEADERS=xforwarded
KC_HTTP_ENABLED=true
KC_HTTP_HOST=$ORIGIN_HOST
KC_HTTP_PORT=$ORIGIN_PORT
KC_HTTPS_PORT=$ADMIN_PORT
KC_HTTPS_CERTIFICATE_FILE=/tls/server.pem
KC_HTTPS_CERTIFICATE_KEY_FILE=/tls/server.key
KC_BOOTSTRAP_ADMIN_USERNAME=magnus-bootstrap
KC_BOOTSTRAP_ADMIN_PASSWORD=$ADMIN_PASSWORD
EOF
[ "$ORIGIN_HOST" = "$ADMIN_HOST" ] || { echo "TUNNEL_ORIGIN and status_admin_url must share a loopback host" >&2; exit 1; }
if ! docker container inspect "$NAME" >/dev/null 2>&1; then
  # The certificate key is read by the image's user (uid 1000).
  docker run -d --name "$NAME" --restart unless-stopped --network host --user "$(id -u):$(id -g)" \
    --env-file "$STATE/keycloak.env" -v "$TLS:/tls:ro" "$IMAGE" start >/dev/null
else
  docker start "$NAME" >/dev/null
fi
echo "waiting for Keycloak on $ADMIN_URL"
for _ in $(seq 1 120); do
  curl -sf --cacert "$TLS/ca.pem" "$ADMIN_URL/realms/master" >/dev/null && break
  sleep 2
done
curl -sf --cacert "$TLS/ca.pem" "$ADMIN_URL/realms/master" >/dev/null || { docker logs --tail 30 "$NAME" >&2; exit 1; }

# The realm, the status client, the people; the appliance's clients through
# config/keycloak-client.py.
export ADMIN_URL REALM STATUS_CLIENT OWNER PEOPLE STATE TLS
STATUS_SECRET=$(python3 -I - <<'EOF'
import json, os, secrets, ssl, sys
from urllib.error import HTTPError
from urllib.parse import quote, urlencode
from urllib.request import Request, urlopen
E = os.environ
ctx = ssl.create_default_context(cafile=E["TLS"] + "/ca.pem")
adm = json.load(open(E["STATE"] + "/admin.json"))
def send(method, path, body=None, form=None, token=None, ok404=False):
    h, data = {}, None
    if token: h["Authorization"] = "Bearer " + token
    if form is not None: data = urlencode(form).encode()
    elif body is not None: h["Content-Type"] = "application/json"; data = json.dumps(body).encode()
    try:
        with urlopen(Request(E["ADMIN_URL"] + path, data=data, method=method, headers=h), context=ctx, timeout=30) as r:
            raw = r.read(); return json.loads(raw) if raw else None
    except HTTPError as e:
        if ok404 and e.code == 404: return None
        sys.exit(f"{method} {path}: {e.code} {e.read()[:300].decode(errors='replace')}")
tok = send("POST", "/realms/master/protocol/openid-connect/token",
           form={"client_id": "admin-cli", "grant_type": "password", "username": adm["username"], "password": adm["password"]})["access_token"]
realm = E["REALM"]; base = f"/admin/realms/{quote(realm)}"
if send("GET", base, token=tok, ok404=True) is None:
    send("POST", "/admin/realms", body={"realm": realm, "enabled": True, "bruteForceProtected": True}, token=tok)
    print(f"created realm {realm}", file=sys.stderr)
out = ""
if E["STATUS_CLIENT"]:
    cid = E["STATUS_CLIENT"]
    found = send("GET", f"{base}/clients?{urlencode({'clientId': cid})}", token=tok)
    value = secrets.token_urlsafe(32)
    client = {"clientId": cid, "name": "Magnus appliance (account status)", "protocol": "openid-connect", "enabled": True,
              "publicClient": False, "clientAuthenticatorType": "client-secret", "secret": value,
              "standardFlowEnabled": False, "implicitFlowEnabled": False, "directAccessGrantsEnabled": False, "serviceAccountsEnabled": True}
    if found: send("PUT", f"{base}/clients/{found[0]['id']}", body={**found[0], **client}, token=tok)
    else: send("POST", f"{base}/clients", body=client, token=tok)
    uid = send("GET", f"{base}/clients?{urlencode({'clientId': cid})}", token=tok)[0]["id"]
    account = send("GET", f"{base}/clients/{uid}/service-account-user", token=tok)["id"]
    rm = send("GET", f"{base}/clients?{urlencode({'clientId': 'realm-management'})}", token=tok)[0]["id"]
    send("POST", f"{base}/users/{account}/role-mappings/clients/{rm}", body=[send("GET", f"{base}/clients/{rm}/roles/view-users", token=tok)], token=tok)
    out = value
for person in E["PEOPLE"].split():
    if [u for u in send("GET", f"{base}/users?{urlencode({'username': person, 'exact': 'true'})}", token=tok) if u["username"] == person]:
        continue
    pw = secrets.token_urlsafe(12)
    send("POST", f"{base}/users", body={"username": person, "enabled": True,
         "credentials": [{"type": "password", "value": pw, "temporary": True}]}, token=tok)
    with open(f"{E['STATE']}/{person}.password", "w") as f: f.write(pw + "\n")
    print(f"created {person}; first password in identity/{person}.password", file=sys.stderr)
print(out)
EOF
)

if [ -n "$STATUS_SECRET" ] && [ -n "$STATUS_CREDENTIAL" ]; then
  TOKEN=$(python3 -I -c "import sys,tomllib;c=tomllib.load(open(sys.argv[1],'rb'));print([p for p in c['people'] if p['id']==sys.argv[2]][0]['token'])" "$CONFIG" "$OWNER")
  printf '{"value": "%s"}' "$STATUS_SECRET" | curl -sf -X PUT -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
    --data-binary @- "$API/v1/secrets/$STATUS_CREDENTIAL" >/dev/null && echo "status client secret stored in the vault as $STATUS_CREDENTIAL"
fi

KC=(python3 -I "$HERE/config/keycloak-client.py" --admin-url "$ADMIN_URL" --admin-file "$STATE/admin.json" --ca "$TLS/ca.pem" --realm "$REALM")
"${KC[@]}" --client-id "$CLIENT_ID" >/dev/null && echo "client $CLIENT_ID ready"
if [ -n "$CONSOLE_NAMES" ] && [ -x "$CONF_DIR/bin/magnus-admin" ]; then
  "$CONF_DIR/bin/magnus-admin" jwk --config "$CONFIG" >"$STATE/console.jwks"
  args=(); for n in $CONSOLE_NAMES; do args+=(--admin-hostname "$n"); done
  "${KC[@]}" --admin-console "$STATE/console.jwks" "${args[@]}" --assign-admin "$OWNER" | sed '/^$/d'
fi
echo "Keycloak for $PUBLIC/realms/$REALM is up: admin $ADMIN_URL, tunnel origin http://$ORIGIN_HOST:$ORIGIN_PORT"
