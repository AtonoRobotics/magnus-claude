#!/usr/bin/env python3
"""Register the appliance as a client of a Keycloak realm, for sign-in with
the device-code grant (RFC 8628). Run once, as root, on the Keycloak host:

    sudo python3 -I config/keycloak-client.py

It signs in to Keycloak's admin API with the administrator in --admin-file
(JSON with "username" and "password"), then creates the client or updates it
to match. The client is public (no secret): the appliance only reads who
signed in from the ID token and keeps no Keycloak token. Prints what to put
in magnus.toml. Prints no credential.

With --admin-console JWKS_FILE, it registers the admin console instead
(admin console spec): a confidential client, the code flow with PKCE, its
one redirect https://<--admin-hostname>/auth/callback, proving itself with
the console's own key, whose public half `magnus-admin jwk --config
magnus.toml` prints into JWKS_FILE; and the person's realm roles in its ID
token, since only the realm role `admin` opens the console. It also
registers the console's service account, magnus-admin-svc, proving itself
with the same key and holding only user management in this realm
(manage-users, view-users, query-users, view-realm): the Users screen
invites, turns off and ends the sessions of the realm's people through it.
"""
import argparse
import json
import ssl
import sys
from urllib.error import HTTPError
from urllib.parse import quote, urlencode
from urllib.request import Request, urlopen

p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
p.add_argument("--admin-url", default="https://127.0.0.1:19446", help="Keycloak's private administration origin")
p.add_argument("--admin-file", default="/var/lib/magnus-production-operator/identity-admin.json")
p.add_argument("--admin-realm", default="master")
p.add_argument("--ca", default="/etc/magnus-production-origin/ca.pem", help="CA that signed the administration origin's certificate")
p.add_argument("--realm", default="magnus")
p.add_argument("--client-id", default="magnus-claude")
p.add_argument("--name", default="Magnus appliance")
p.add_argument("--admin-console", metavar="JWKS_FILE", help="register the admin console's client, with the key `magnus-admin jwk` printed")
p.add_argument("--admin-hostname", help="the admin console's hostname, with :port if not 443")
a = p.parse_args()
if a.admin_console:
    if not a.admin_hostname:
        p.error("--admin-console needs --admin-hostname")
    if a.client_id == "magnus-claude":
        a.client_id = "magnus-admin"
    if a.name == "Magnus appliance":
        a.name = "Magnus admin console"

ctx = ssl.create_default_context(cafile=a.ca)
admin = json.load(open(a.admin_file))


def send(method, url, body=None, form=None, token=None):
    headers = {}
    data = None
    if token:
        headers["Authorization"] = "Bearer " + token
    if form is not None:
        data = urlencode(form).encode()
    elif body is not None:
        headers["Content-Type"] = "application/json"
        data = json.dumps(body).encode()
    try:
        with urlopen(Request(url, data=data, method=method, headers=headers), context=ctx, timeout=15) as r:
            raw = r.read()
            return json.loads(raw) if raw else None
    except HTTPError as e:
        sys.exit(f"{method} {url}: {e.code} {e.read()[:300].decode(errors='replace')}")


token = send("POST", f"{a.admin_url}/realms/{a.admin_realm}/protocol/openid-connect/token",
             form={"client_id": "admin-cli", "grant_type": "password", "username": admin["username"], "password": admin["password"]})["access_token"]
base = f"{a.admin_url}/admin/realms/{quote(a.realm)}"
issuer = send("GET", f"{a.admin_url}/realms/{quote(a.realm)}/.well-known/openid-configuration")["issuer"]

client = {
    "clientId": a.client_id,
    "name": a.name,
    "protocol": "openid-connect",
    "enabled": True,
    "publicClient": True,
    # Only the device-code grant: no redirects, no passwords through the client.
    "standardFlowEnabled": False,
    "implicitFlowEnabled": False,
    "directAccessGrantsEnabled": False,
    "serviceAccountsEnabled": False,
    "attributes": {"oauth2.device.authorization.grant.enabled": "true"},
}
if a.admin_console:
    jwks = json.load(open(a.admin_console))
    client = {
        "clientId": a.client_id,
        "name": a.name,
        "protocol": "openid-connect",
        "enabled": True,
        "publicClient": False,
        "clientAuthenticatorType": "client-jwt",
        "standardFlowEnabled": True,
        "implicitFlowEnabled": False,
        "directAccessGrantsEnabled": False,
        "serviceAccountsEnabled": False,
        "redirectUris": [f"https://{a.admin_hostname}/auth/callback"],
        "attributes": {
            "pkce.code.challenge.method": "S256",
            "use.jwks.string": "true",
            "jwks.string": json.dumps(jwks),
            "token.endpoint.auth.signing.alg": "ES256",
        },
        "protocolMappers": [{
            "name": "realm roles", "protocol": "openid-connect", "protocolMapper": "oidc-usermodel-realm-role-mapper",
            "config": {"id.token.claim": "true", "access.token.claim": "true", "userinfo.token.claim": "true", "claim.name": "realm_access.roles", "multivalued": "true"},
        }],
    }
found = send("GET", f"{base}/clients?{urlencode({'clientId': a.client_id})}", token=token)
if found:
    send("PUT", f"{base}/clients/{found[0]['id']}", body={**found[0], **client, "attributes": {**found[0].get("attributes", {}), **client["attributes"]}}, token=token)
    print(f"Updated client {a.client_id} in realm {a.realm}.")
else:
    send("POST", f"{base}/clients", body=client, token=token)
    print(f"Created client {a.client_id} in realm {a.realm}.")

if a.admin_console:
    svc = {
        "clientId": "magnus-admin-svc",
        "name": "Magnus admin console (people)",
        "protocol": "openid-connect",
        "enabled": True,
        "publicClient": False,
        "clientAuthenticatorType": "client-jwt",
        "standardFlowEnabled": False,
        "implicitFlowEnabled": False,
        "directAccessGrantsEnabled": False,
        "serviceAccountsEnabled": True,
        "attributes": {"use.jwks.string": "true", "jwks.string": json.dumps(jwks), "token.endpoint.auth.signing.alg": "ES256"},
    }
    found = send("GET", f"{base}/clients?{urlencode({'clientId': svc['clientId']})}", token=token)
    if found:
        send("PUT", f"{base}/clients/{found[0]['id']}", body={**found[0], **svc, "attributes": {**found[0].get("attributes", {}), **svc["attributes"]}}, token=token)
    else:
        send("POST", f"{base}/clients", body=svc, token=token)
    cid = send("GET", f"{base}/clients?{urlencode({'clientId': svc['clientId']})}", token=token)[0]["id"]
    account = send("GET", f"{base}/clients/{cid}/service-account-user", token=token)["id"]
    rm = send("GET", f"{base}/clients?{urlencode({'clientId': 'realm-management'})}", token=token)[0]["id"]
    roles = [send("GET", f"{base}/clients/{rm}/roles/{r}", token=token) for r in ("manage-users", "view-users", "query-users", "view-realm")]
    send("POST", f"{base}/users/{account}/role-mappings/clients/{rm}", body=roles, token=token)
    print(f"The console's service account magnus-admin-svc may manage the people of realm {a.realm}, and no more.")
    print(f"""
The admin console signs in through {issuer} as {a.client_id}. Give the
realm role "admin" to the people who run the appliance; only they enter.
""")
    sys.exit(0)

print(f"""
Add to magnus.toml, then restart the appliance:

[sign_in.oidc]
issuer = "{issuer}"
client_id = "{a.client_id}"
""")
