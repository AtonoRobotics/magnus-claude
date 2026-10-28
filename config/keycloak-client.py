#!/usr/bin/env python3
"""Register the appliance as a client of a Keycloak realm, for sign-in with
the device-code grant (RFC 8628). Run once, as root, on the Keycloak host:

    sudo python3 -I config/keycloak-client.py

It signs in to Keycloak's admin API with the administrator in --admin-file
(JSON with "username" and "password"), then creates the client or updates it
to match. The client is public (no secret): the appliance only reads who
signed in from the ID token and keeps no Keycloak token. Prints what to put
in magnus.toml. Prints no credential.
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
a = p.parse_args()

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
found = send("GET", f"{base}/clients?{urlencode({'clientId': a.client_id})}", token=token)
if found:
    send("PUT", f"{base}/clients/{found[0]['id']}", body={**found[0], **client, "attributes": {**found[0].get("attributes", {}), **client["attributes"]}}, token=token)
    print(f"Updated client {a.client_id} in realm {a.realm}.")
else:
    send("POST", f"{base}/clients", body=client, token=token)
    print(f"Created client {a.client_id} in realm {a.realm}.")

print(f"""
Add to magnus.toml, then restart the appliance:

[sign_in.oidc]
issuer = "{issuer}"
client_id = "{a.client_id}"
""")
