# Accounts: Keycloak through one identity interface

Status: building (owner's spec, 2026-10-30, which replaces the built-in-first
plan of the same day). Step 1 is built. Device certificates (same day) have
open decisions D1–D5. Keycloak changes the owner makes are listed at the end.

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

### Device certificates (owner's spec, 2026-10-30: required)

> Build device certificates now; they're required, not optional (the passkey
> and Keycloak decisions both rely on them as the second factor).
> - At pairing, the app generates a key pair on the device (in the OS
>   keystore: Windows CNG/TPM where available, Linux Secret Service) and the
>   appliance issues it a client certificate.
> - The appliance requires mutual TLS: every connection presents a valid,
>   unrevoked device certificate, and only then is the sign-in token
>   accepted. A valid token from an unpaired device is refused.
> - Devices are listed per user with name, last seen and revoke; revocation
>   takes effect immediately, including over the relay.
> - Deactivating a user (local or via Keycloak) revokes all their devices.
>
> Tests: an unpaired device holding a valid token is refused; a revoked device
> is refused directly and through the relay; a device key copied to another
> machine fails where the key is hardware-bound.

**Today.** The app pins the appliance's certificate. The appliance never
checks which device is calling: any holder of a session token gets in.

**Pairing is the first sign-in on a device.**
1. **The app makes a key pair in the computer's keystore.** It's ECDSA P-256,
   which every TPM 2.0 and Windows' platform provider support. The private
   key can't be exported:
   - Windows: CNG, with the Microsoft Platform Crypto Provider (the TPM)
     where there is one, otherwise the software provider, non-exportable;
   - Linux: see decision D2.
2. **It sends a certificate request with its sign-in.** The request is signed
   by the device key, which proves the app holds it.
3. **The appliance issues the certificate when the sign-in finishes.** It
   comes from a device certificate authority of the appliance's own, whose
   key is in the vault. The certificate names the person and the device. The
   session is bound to that device.

**Mutual TLS on the HTTPS listener.**
- The handshake asks every client for a certificate.
- Only pairing works without one: the sign-in method, starting and finishing
  a sign-in with a certificate request, and health.
- Every other request needs a valid, unrevoked device certificate. Its token
  must be for that device's person and that device. So a token from another
  device, or from no device, is refused.
- Revocation is checked on every request, not only at the handshake, so it
  takes effect at once on a connection that is already open. Live streams on
  a revoked device are closed.

**Devices.**
- Each person sees their devices: name (the computer's), system, where the
  key is kept (hardware or software), paired, last seen. They can revoke any
  of them.
- The owner sees and revokes anyone's.
- Revoking ends the device's sessions. The device must pair again.
- Deactivation (step 3) revokes all of a person's devices.

**Tests.**
- A real TLS client without a certificate, presenting a valid token from a
  paired device, is refused.
- A revoked device is refused at once, on an open connection too.
- The installed app pairs on its first sign-in.
- A device key copied to another machine fails where the key is
  hardware-bound. The key's TPM blob only loads in the TPM that made it, so
  this needs a second real TPM (D2).

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

## Open decisions on device certificates

- **D1. The appliance's own page in a browser.** Spec: every connection
  presents a device certificate, so browsers can't use the appliance's page
  any more. Then only the app signs in, and the device-code sign-in goes.
  - Recommended: app only, as the spec says.
  - The alternative: a browser imports a device certificate by hand.
- **D2. Where Linux keeps the device key.** The spec names the Secret
  Service. It stores software keys that can be copied, so they aren't
  hardware-bound.
  - Recommended: the TPM 2.0 where there is one, hardware-bound as on
    Windows; this machine has one, `/dev/tpmrm0`. The Secret Service
    otherwise.
  - Using the TPM needs the app's user in the `tss` group, a packaging step.
- **D3. The relay.** This build has no relay. "Through the relay" needs one
  to exist: which relay is meant, and should I build it?
- **D4. The appliance's loopback listener** (127.0.0.1, HTTP), which the
  `magnus` command, local scripts and the tests use on the appliance's own
  machine.
  - Recommended: no device certificate there. Only someone already on the
    appliance machine can reach it.
- **D5. Script tokens** (`[[people]] token`). Recommended: over the network
  they need a device certificate too, issued on the appliance with
  `magnus device issue`.

## Build order

1. The identity interface and sign-in by code and PKCE in the appliance and
   the app; tests against a real Keycloak. **Built.**
2. Device certificates: keys in the keystore, pairing at first sign-in,
   mutual TLS, devices and revoke (after D1–D5).
3. Groups to roles, read at sign-in and refresh.
4. Short-lived sessions with refresh, the status check, and deactivation:
   suspended schedules and inbox notices, every device revoked.

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
