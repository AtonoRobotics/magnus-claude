# Accounts: Keycloak through one identity interface

Status: building (owner's spec, 2026-10-30, which replaces the built-in-first
plan of the same day). Keycloak changes the owner makes are listed at the end.

## The owner's spec

Use my Keycloak as the identity provider now, through the identity interface
(this is the enterprise SSO path, built early):

- OIDC authorization code + PKCE through the system browser with a loopback
  redirect; works on Windows and Linux. Passkeys and two-factor are handled by
  Keycloak.
- Map Keycloak groups to Magnus roles (E2, E7, K46). Paired device
  certificates still required.
- Deactivation: short-lived access tokens, plus a periodic user-status check
  via Keycloak's admin API (service account, least privilege); a disabled user
  loses access and has their schedules and triggers suspended within minutes
  (K54).
- Keep built-in local accounts as a fallback for appliances without an
  identity provider.

Tests: sign in through Keycloak on both platforms; a group change alters
approval rights; disabling a user in Keycloak revokes access and suspends their
schedules.

## What exists

- **Sign-in through Keycloak** by device code (`oidc.rs`): the appliance
  collects the ID token and opens a Magnus session that lasts `session_days`.
  Groups and roles come from claims the owner names. A disabled user keeps
  their session, and their schedules keep running.
- **Password sign-in** for people in `[[people]]`: the local fallback the spec
  keeps.
- **Every decision reads `Kernel::person(id)`:** the boundary, policy layers
  and approvals.
- **The app pins the appliance's certificate** when it first connects. It has
  no certificate of its own.

## Design

### The identity interface

- **Directory:** who exists, their groups and roles, and whether they are
  active.
- **Authenticator:** how someone proves who they are. It ends in a Magnus
  session.

Keycloak (OIDC) supplies both. Built-in accounts are the fallback. SAML and
SCIM plug into the same two parts later. Decisions keep reading
`Kernel::person(id)`.

### Sign-in: authorization code + PKCE, loopback redirect (RFC 8252)

1. **The app starts it.** It listens on `127.0.0.1` on a free port and asks
   the appliance to start a sign-in with
   `redirect_uri = http://127.0.0.1:<port>/callback`.
2. **The appliance prepares it.** It makes the PKCE verifier (S256), the
   `state` and the `nonce`, keeps them, and returns Keycloak's authorization
   address.
3. **The person signs in in their own browser**, which the app opens. Passkeys
   and two-factor are Keycloak's, so they work on Linux too: the browser has
   WebAuthn, and the app's Linux webview does not (measured: WebKitGTK 2.52
   has no `navigator.credentials`).
4. **Keycloak sends the browser back** to the loopback with `code` and
   `state`. The app passes them to the appliance over the pinned connection.
5. **The appliance finishes it.** It exchanges the code with the verifier,
   checks the ID token (issuer, audience, expiry, nonce) and opens a Magnus
   session.

The code alone is useless: only the appliance holds the verifier. The browser
gets a page saying it can be closed.

This replaces the device-code flow.

### Paired devices

Sign-in still runs only over the app's pinned connection to the appliance. A
Keycloak sign-in never replaces pairing: an app that hasn't pinned the
appliance's certificate can't start one.

(The app has no client certificate today. If "paired device certificates"
means one per device, a mutual-TLS certificate issued when the app pairs and
revocable per device, that is a separate step to add.)

### Groups to roles (E2, E7, K46)

`[sign_in.oidc.group_roles]` maps a Keycloak group to Magnus roles, for
example `"approvers-financial" = ["approver:financial"]`.

Groups and the roles they map to are read:
- at each sign-in;
- at each refresh (below).

So a group change in Keycloak changes approval rights within one access-token
lifetime, without signing in again. A role a group maps to wins over one set
in Settings, as the provider's roles do today.

### Short-lived access and the status check (K54)

**Short-lived access.** A Magnus session lasts only as long as the person's
Keycloak session.
- The appliance keeps the refresh token in the vault and refreshes it when
  the access token expires. The time comes from Keycloak's `expires_in`
  (Keycloak's default is 5 minutes).
- A refused refresh ends the Magnus session: user disabled, session ended in
  Keycloak, or password reset.

**The status check.** Every `status_minutes` (default 5, matching the access
token's default lifetime), a service account asks Keycloak's admin API for
each person who signed in through it. Schedules act for people who may not be
signed in at all, which is why the check is needed.
- The account needs only the `view-users` role.
- **Disabled or deleted:** the person is deactivated (`PersonDeactivated`):
  - every Magnus session and API token is refused;
  - every schedule acting for them is suspended, and so are their running
    tasks;
  - the inbox of people with the owner role gets one notice per schedule,
    with **Reassign** or **End**. Reassigning waits for the new person to
    confirm it on a card, because the schedule would then act with their
    rights.
- **Enabled again:** they can sign in again. Each schedule stays suspended
  until someone resumes it.
- **Keycloak unreachable:** the check is retried and nobody is deactivated.
  The owner sees on Home that the check is failing.

The two together mean a disabled person loses access within about 5 minutes,
and their schedules are suspended within about 5 minutes.

### The local fallback

With no `[sign_in.oidc]`, people in `[[people]]` sign in with passwords, as
today. The built-in setup, invites and two-factor of the earlier plan wait
for an appliance that has no identity provider.

## Tests

- **Scratch tests against a real Keycloak** (26.8.1, in a container with a
  test realm):
  - sign-in by code and PKCE;
  - a wrong `state` or a reused code is refused;
  - a group change alters approval rights after a refresh;
  - disabling a user ends their session and suspends their schedules within
    one check, with the inbox notice; Reassign and End work.
- **Installed app (Linux):** sign in through Keycloak. The test's "system
  browser" fills in Keycloak's login form.
- **Windows:** the owner's test.

## Build order

1. The identity interface and sign-in by code and PKCE in the appliance and
   the app; tests against a real Keycloak.
2. Groups to roles, read at sign-in and refresh.
3. Short-lived sessions with refresh, the status check, deactivation with
   suspended schedules and inbox notices.

## Keycloak changes for strix-alpha (the owner, or me with a realm admin account)

1. **Client `magnus-claude`:**
   - turn on Standard flow;
   - set the redirect URI to `http://127.0.0.1/*` (checked today: Keycloak
     rejects a loopback redirect for this client);
   - require PKCE method S256.
2. **A new confidential client, `magnus-claude-status`:**
   - turn on Service accounts only;
   - give its service account just `realm-management` → `view-users`;
   - its secret goes into the appliance's vault.
3. **A "Group Membership" mapper** on the client's ID token, so the token
   carries the groups. Then create the groups to map.
