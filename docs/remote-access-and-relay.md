# pyrallon.ai and the tunnel to StrixAlpha

Plan only. Nothing in this document was created in Cloudflare. Account
`3a1be1145e2314f8612a451e325e0218`, zone `pyrallon.ai` (active; the account
also has `alphavectorrobotics.com`). The appliance is the Linux host
StrixAlpha, on a private network. No tunnel, Access application or private
network route exists yet.

Sources: this repo (`docs/proposals/auth.md`, `docs/connectors.md`,
`docs/admin-console.md`, `docs/proposals/spec-decisions.md`, DECISIONS.md),
`AtonoRobotics/magnus-product` (`deployment/connector-registration.md`,
`workers/connections/oauth_worker.py`), and
`AtonoRobotics/enterprise-agent-spec` (R22: the vendor hosts licensing and
image updates only; customer content stays on the customer's machines).

## 1. What lives on pyrallon.ai

Three different things, on three different names. They are not one service.

| Name | What it is | Where it runs |
| --- | --- | --- |
| `https://pyrallon.ai/` | The public site: homepage, privacy, terms, support contact, and later the Google verification videos. The product registration packet (2026-10-24) says `/privacy` and `/terms` still 404 and the terms are unwritten. Do not publish guessed policy text. | Cloudflare (Pages or a Worker). Not StrixAlpha. Enterprise spec R22: this is vendor material, not customer content. |
| `https://pyrallon.ai/v1/connections/callback` | The OAuth relay callback the product spec names as the confidential web client's only redirect. This repo's connectors spec calls the same role `connect.pyrallon.ai`. Use the product URL as the one providers are given; a second hostname can point at the same Worker later. Staging, when it exists, is `connect.staging.pyrallon.ai`. | A Cloudflare Worker. Not the appliance, and not through the tunnel. The relay has to answer while StrixAlpha is unreachable, and it is the thing that holds client secrets so the appliance can stop holding them. |
| `https://magnus.pyrallon.ai` | The appliance people and the desktop app reach: its HTTPS API and the same UI the appliance already serves. | StrixAlpha, through the named tunnel. |
| `https://identity.pyrallon.ai` | Keycloak, realm `magnus`. Already the issuer. Renaming it to `auth.pyrallon.ai` would change every token's issuer and is not worth it (`docs/proposals/auth.md`). | Only move it onto this tunnel after confirming the name is served from StrixAlpha today. If the name already points somewhere else, leave its DNS alone. |

### What the relay does

The connectors spec (Relay, Connect flow) and the product registration
packet agree on the job, and disagree with a finished design of the handoff:

1. A provider redirects the person's browser to
   `https://pyrallon.ai/v1/connections/callback` with `code`, `state`, and,
   for Google, `iss`.
2. The relay checks `state` (which appliance, which person, which
   connector, a nonce, an expiry). It does not keep the code.
3. It sends the browser on. The connectors spec says: to the appliance's
   completion endpoint at that appliance's pyrallon.ai name, found from an
   appliance registry. `docs/proposals/spec-decisions.md` says that
   registry, and who runs it, is not specified. The line in the connectors
   spec is marked proposed, not ruled.
4. For a confidential client, the relay also proxies the token call
   (exchange, refresh, revoke): the appliance sends the code and the PKCE
   verifier, the relay adds the client secret, and it stores nothing. It
   logs appliance, provider, operation, time and result, never a code or a
   token. Appliances authenticate with the mutual-TLS certificate issued
   at provisioning. That certificate path is not built.
5. The browser page the person sees is "Connected, return to Magnus."

`workers/connections/oauth_worker.py` in magnus-product is not this relay.
It is the OAuth client that runs on a private pipe inside Magnus: it builds
the authorize URL, exchanges the code, refreshes and revokes. Its
`redirect_uri` is configuration. It does not listen on pyrallon.ai.

Why the relay is a Worker and not a process on StrixAlpha: the connectors
spec wants it stateless, in at least two regions, holding client secrets
and no user data, and talking only to providers' token endpoints. Putting
it on the appliance puts the secret back where the spec is taking it off,
and the callback dies when the tunnel dies.

Why it is not implemented in this pull request: the handoff is the part
that has to be right, and it is the part that is not ruled. A Worker that
accepts a `code` and forwards it would be an open redirect for authorization
codes. The current appliance cannot receive that forward anyway:
`POST /v1/connectors/{name}/sign-in/complete` requires the person's own
session, and the desktop app today catches the code on
`http://127.0.0.1:<port>/callback` and posts it itself. A browser that has
just come from Google has neither that session nor a way through Cloudflare
Access. Building the Worker waits on a ruled state format and on the
appliance authenticating to the relay.

Until that exists, provider consoles keep the loopback redirect the desktop
app sends now. Do not switch them to the relay URL before the relay and the
appliance both speak it.

## 2. What on StrixAlpha is reachable

The appliance speaks HTTP/1.1 only (`crates/magnus/src/tls.rs`). A client
that negotiates HTTP/2 gets the connection closed. The tunnel must not
offer HTTP/2 to the origin.

| Service | Address on StrixAlpha | Through the tunnel? |
| --- | --- | --- |
| Appliance API and UI | `https://127.0.0.1:7700` (HTTPS on all interfaces, `tls_listen`). Certificate is the appliance's own, which browsers do not trust. | Yes, as `magnus.pyrallon.ai`. Cloudflare presents its own certificate to clients. The origin check skips verification of that self-signed certificate, and the origin protocol is HTTP/1.1. |
| Appliance plain HTTP | `http://127.0.0.1:7701` | No. Config tokens are accepted only on this listener, and only from loopback (owner's ruling 13). cloudflared connects from loopback, so publishing 7701 would make those tokens work from the internet. |
| Admin console | `https://192.168.0.50:7443` and `https://100.92.130.94:7443` (Tailscale). It refuses loopback, so a tunnel aimed at `127.0.0.1:7443` cannot reach it, and the spec forbids an `admin.pyrallon.ai` name. | No. Tailscale, or the LAN. |
| Keycloak, public | Whatever already answers `identity.pyrallon.ai`. The repo does not record that listen port. | Only if this host is already that origin. Paths `/realms/magnus/` and `/resources/` only; everything else 404, which is how the admin API stays off the public name. |
| Keycloak admin | `https://127.0.0.1:19446` | No. |
| Postgres | `127.0.0.1:54341` (the appliance). Test servers on 54340 stay local too. | No. |
| SSH | `192.168.0.50:22` | Not a public hostname. Optional private route, below. |
| Browser sidecar, search, the vault, cloudflared's own metrics | Loopback. Metrics, once set, `http://127.0.0.1:20241/ready`. | No. |

### Ingress

One named tunnel, remotely managed (hostnames set in the dashboard, not a
config file committed anywhere). Account
`3a1be1145e2314f8612a451e325e0218`, zone `pyrallon.ai`.

| Public hostname | Origin | Origin settings | Who may open it |
| --- | --- | --- | --- |
| `magnus.pyrallon.ai` | `https://127.0.0.1:7700` | No TLS Verify on. HTTP/2 to origin off. HTTP Host Header `magnus.pyrallon.ai`. | Cloudflare Access. Include email `frty130@gmail.com`. Everyone else denied. |
| `identity.pyrallon.ai` | the public Keycloak port, found on the host, never `19446` | HTTP/2 to origin off. Add this row only after confirming the name is served from StrixAlpha. | No Access application. A login page behind Access is a second login. Rate limit the hostname. Path allow only `/realms/magnus/` and `/resources/`. |
| (catch-all) | `http_status:404` | | |

Not in the ingress: `7443`, `7701`, `19446`, `54341`, `22`, and the apex
`pyrallon.ai` (that name is the site and the relay, not the appliance).

After the tunnel answers, the appliance config gains, and only these:

```toml
[admin]
tunnel_ready = "http://127.0.0.1:20241/ready"
public_hostnames = ["magnus.pyrallon.ai"]
```

Add `identity.pyrallon.ai` to `public_hostnames` only if that name is on
this tunnel. The console is never in the list. Operations › Network then
shows the tunnel; Health shows cloudflared.

The desktop app today pins the appliance's self-signed certificate and
talks to the LAN or Tailscale address. `magnus.pyrallon.ai` presents
Cloudflare's certificate, and Access will refuse the app until the app can
complete an Access login. Leave the app on the LAN or Tailscale address.
Do not put a service token in the app. Pinning stays for that private
address (`spec-decisions.md`, question 20).

## 3. Runbook

These are the commands and dashboard steps. They were not run.

### Token

Create one API token on that account, not an all-accounts token. Zone
`pyrallon.ai` only, plus the account permissions below. Do not grant
Workers, SSL administration, or every zone.

| Resource | Permission | Used for |
| --- | --- | --- |
| Account | Cloudflare Tunnel: Edit | Create the tunnel and its public hostnames |
| Account | Access: Apps and Policies: Edit | The Access application and the email policy |
| Account | Access: Organizations, Identity Providers, and Groups: Read | The policy has to see the one-time-PIN identity provider |
| Zone `pyrallon.ai` | DNS: Edit | The CNAME |
| Zone `pyrallon.ai` | Zone: Read | Select the zone |
| Account | Cloudflare One Networks: Edit | Only if the private route for SSH is added |

The dashboard labels match the user's names: Account Cloudflare Tunnel
Edit, Access Apps and Policies Edit, Zone DNS Edit.

### Cloudflare

1. Zero Trust → Networks → Tunnels → Create. Name `strix-alpha`. Type
   cloudflared. Copy the install token once. It is a credential. It does
   not go in git, in the appliance vault, or in this repository's secrets.
2. Public hostname `magnus.pyrallon.ai`. Service type HTTPS, URL
   `127.0.0.1:7700`. Additional settings: No TLS Verify, HTTP/2 connection
   to origin off (or `http2Origin: false`), HTTP Host Header
   `magnus.pyrallon.ai`.
3. Confirm the DNS row that the hostname step writes: `magnus.pyrallon.ai`
   CNAME to `<tunnel-id>.cfargotunnel.com`, proxied. Do not create a second
   A record. Do not change `pyrallon.ai` or `alphavectorrobotics.com`.
4. Leave `identity.pyrallon.ai` until step 8.
5. Access → Applications → Add → Self-hosted. Name `Magnus`. Domain
   `magnus.pyrallon.ai`. Session length one day is enough for a single
   operator. Policy name `owner`: Action Allow, Include Selector Emails,
   value `frty130@gmail.com`. No bypass, no everyone, no country rule as a
   substitute. Identity provider: One-time PIN.
6. Optional private route, for SSH, not for the console. Networks → Routes
   → Add `192.168.0.50/32` on tunnel `strix-alpha`. Access application of
   type Private network, destination `192.168.0.50:22`, same email policy.
   The device runs WARP and is enrolled in this Zero Trust org. Do not
   publish port 22 as a hostname.
7. On the appliance config, set `tunnel_ready` and `public_hostnames` as
   above, and restart the admin console so Operations shows the tunnel.
   Restarting is the owner's; this plan does not change the running host.
8. Identity, only if `dig +short identity.pyrallon.ai` is this host or is
   already a name you intend to move. On StrixAlpha, find the listener that
   is the public Keycloak port (`ss -lptn`) and add that hostname to the
   same tunnel. Never use `127.0.0.1:19446`. In front of that hostname, a
   Cloudflare rule returns 404 except for `/realms/magnus/` and
   `/resources/`. No Access application on it.

### StrixAlpha

cloudflared is a system service so the tunnel is up at boot, not only while
someone is logged in. The appliance itself stays the user service
`magnus-appliance`.

```sh
# The token from step 1. It is not written here.
curl -fsSL https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64 -o /tmp/cloudflared
sudo install -m 755 /tmp/cloudflared /usr/local/bin/cloudflared
sudo cloudflared service install -- '<TOKEN>'
```

The unit `cloudflared.service` then holds the token. Add metrics, which the
token install does not turn on, by a drop-in and not by putting the token
in a file in git:

```ini
# /etc/systemd/system/cloudflared.service.d/metrics.conf
[Service]
Environment=TUNNEL_METRICS=127.0.0.1:20241
```

```sh
sudo systemctl daemon-reload
sudo systemctl restart cloudflared
sudo systemctl status cloudflared
curl -fsS http://127.0.0.1:20241/ready
```

`/ready` returns 200 when the connector is registered. From another
network, `https://magnus.pyrallon.ai` asks for the one-time PIN to
`frty130@gmail.com` and then shows the appliance. From the LAN, the old
address still works and does not go through Cloudflare.

### Redirects to switch, later

Not now. When the relay and the appliance both implement the handoff, the
confidential clients move from `http://127.0.0.1:<port>/callback` to exactly:

`https://pyrallon.ai/v1/connections/callback`

That is one URL for every provider. The connectors spec's
`connect.pyrallon.ai` is the same callback on a second name, not a second
path. Google's current client is a Desktop client, which accepts any
loopback port and does not use this URL; the product spec's web client is
the one that will. Switching Google before the relay answers makes sign-in
fail. Slack, Vercel, Sentry, Asana, GitLab, Hugging Face, Supabase and
Cloudflare, which require an exact redirect, are the ones this URL unblocks.
GitHub can stay on `http://127.0.0.1/callback` until the same cutover.
Microsoft's Entra redirect list gains this URL and drops the loopback one
at the same time.

## 4. Where the code goes

Not in this repository yet, and not as a Worker in this pull request.

The Worker is small only after the state format is ruled. What is ruled is
the role (bounce the callback, proxy the token call, hold the secret, store
no user data, log no code). What is not ruled is how `state` names an
appliance and how the code reaches `POST /v1/connectors/{name}/sign-in/complete`
without becoming a public inbox for authorization codes. magnus-product
already has the in-process OAuth client (`oauth_worker.py`); it is not a
public edge program, and copying it into a Worker would put token handling
on the edge without the private pipe it assumes.

The site (homepage, privacy, terms) is also unwritten. Publishing a Worker
that returns placeholder policy pages would be the guessed text the
registration packet says not to submit.

Next code, when the handoff is ruled, is a Worker in its own repository or
under `relay/` here, plus the appliance changes: signed `state`, mutual TLS
to the relay, and the desktop app posting the completion against its
session after the relay has delivered the code to the appliance. That is
larger than a draft Worker.
