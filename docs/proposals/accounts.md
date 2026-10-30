# User accounts behind one identity interface

Status: proposal (owner's spec, 2026-10-30). Decisions for the owner are at
the end; the passkey one blocks step 4 only.

## The owner's spec

User accounts, through one identity interface that enterprise SSO will later
plug into:

- First-run setup creates the owner account (no default passwords) and shows
  a one-time recovery code.
- Admins invite users by one-time link or code and assign roles at invite (E7).
- Passkeys preferred, password fallback, optional two-factor. Per-user session
  and device list with revoke.
- Built-in groups serve as the identity provider's groups, so E2 and K46 apply
  unchanged.
- Deactivating a user suspends every schedule and trigger acting for them
  (K54) and posts an admin inbox notice to reassign or end each; deleting
  their data uses per-person key destruction (K11).
- Design the interface for OIDC, SAML and SCIM providers; build them after
  the alpha gate, before the first enterprise pilot.

Installed-app tests: first-run setup, invite and sign-in with a passkey, role
limits enforced, and deactivating a user suspends their schedules.

## What exists

- People are listed in `magnus.toml` (`[[people]]`: id, name, groups, roles,
  an optional API token). Nobody can be added without editing the config.
- Password sign-in (Argon2id, lockout after `max_failures`) and sessions
  (`accounts.rs`). Hashes live in the state directory, never in the log.
- Sign-in through an OpenID Connect provider (device code, `oidc.rs`). The
  ID token must name a configured person. Groups and roles from the provider
  replace the config's at each sign-in.
- Roles set in Settings are events (`RolesSet`). Every decision reads one
  place, `Kernel::person(id)`: the boundary, policy layers and approvals.
- Schedules act for the person who set them up (K54). There are no other
  triggers yet.
- There are no per-person keys. The vault has one key (XChaCha20-Poly1305).

## Design

### One identity interface

Two parts, so a provider can supply either or both:

- **Directory**: who exists, their name, groups and roles, and whether they
  are active.
- **Authenticator**: how someone proves who they are, and what it opens: a
  Magnus session.

| Provider | Directory | Authenticator |
|---|---|---|
| Built-in (this plan) | people in the log | passkey, password, two-factor |
| OIDC (exists; moved behind the interface in step 1) | config, or ID token claims | device code |
| SAML (after the alpha gate) | assertion attributes | browser flow, as for passkeys |
| SCIM 2.0 (after the alpha gate) | pushed by the provider (RFC 7643/7644) | none: pairs with OIDC or SAML |

Decisions keep reading `Kernel::person(id)`, so the boundary, policy layers,
approvals (E2, E7) and read rights don't change.

### People in the log

New events:
- `PersonAdded {id, name, groups, roles, by}`;
- `GroupsSet {person, groups}`;
- `PersonDeactivated {person, by}` and `PersonReactivated {person, by}`;
- `PersonDeleted {person, by}`.

`[[people]]` in the config keeps working: as seed people, and for API tokens.

Credentials stay out of the log (invariant 2), in the state directory as
passwords are today:
- passkey public keys;
- password hashes;
- two-factor secrets (in the vault);
- hashes of the recovery code and of invite codes.

The log records only what happened: an invite was made, a passkey was added,
a session was revoked.

### First-run setup

Setup runs while no owner exists.
- **Setup code:** whoever reaches a fresh appliance first could claim it. So
  `magnus serve` prints a one-time setup code and writes it to a file only
  the appliance's user can read. The setup page asks for it.
- **Owner account:** setup creates the owner with a passkey (or a password).
  There are no default passwords.
- **Recovery code:** shown once, 128 bits, in groups of characters; stored as
  an Argon2 hash. It signs the owner in once, and only to set up new
  credentials. A new recovery code is then shown.

### Invites

- **Who:** a person with the new `people:admin` role, which the owner has,
  invites someone.
- **Roles and groups:** set at invite (E7). They apply from the person's first
  sign-in.
- **The invite:** a link, `https://<appliance>/join#<code>`, and the same code
  to type into the app. One use, 128 bits, stored as a SHA-256 hash.
- **Expiry:** after `[accounts] invite_days`, default 7 (GitHub's invitations
  expire after 7 days: long enough for a week away, short enough to limit a
  leaked link). An admin can withdraw an invite before it is used.

### Passkeys

Done with WebAuthn, using `webauthn-rs`.
- **Measured on strix-alpha:** the Linux app's webview (WebKitGTK 2.52) has no
  WebAuthn. `navigator.credentials` and `PublicKeyCredential` are undefined,
  and the page's origin is `tauri://localhost`.
- **Where the ceremony runs:** on the appliance's own page in the person's
  browser, the same pattern as provider sign-in today. The app opens the page
  and waits for that one sign-in to finish, then gets its session. Windows'
  webview has WebAuthn, but one path for both systems keeps the tested path
  (Linux) the one people use.
- **What browsers require:** WebAuthn only on a secure origin whose host is a
  domain name. An IP address is not allowed, and Chrome refuses on a
  certificate error. The appliance today serves a self-signed certificate
  that the app pins. **Passkeys need the appliance reachable at a domain name
  with a certificate browsers trust** (decision 1).
- **Phones:** Chrome and Edge also offer a phone's passkey by QR code.

### Password fallback and two-factor

- Passwords stay as they are today: Argon2id, `min_password_length`, lockout.
- Two-factor is optional per person: TOTP (RFC 6238 defaults: 30 seconds,
  6 digits, which every authenticator app uses). The secret goes in the vault.
- An admin can require two-factor for everyone who uses a password.
- A passkey sign-in doesn't ask for it, because the passkey already checks
  the device and a PIN or biometric.

### Sessions and devices

- **What each session records:** the client (app or browser), the device name
  the app reports, when it was opened, when it was last used, and the
  address.
- **Who can revoke:** each person sees their own sessions, with "this device"
  marked, and can revoke any of them. An admin can revoke all of someone's
  sessions.
- **The log:** a revocation is an event (`SessionRevoked`), beside the
  existing `SignedIn` and `SignedOut`.

### Groups

- Admins set built-in groups (`GroupsSet`). They fill the same field a
  provider's groups do, so app `access_groups`, group policy layers and read
  rights work unchanged (E2).
- The built-in directory is the identity provider (K46). With an external
  provider, its groups win, as they do today.

### Deactivation

`PersonDeactivated`:
- revokes every session and refuses the person's API token;
- suspends every schedule acting for them (K54). Future triggers will use the
  same "acts for" rule.
- suspends their running tasks (decision 4).

The admin inbox gets one notice per schedule: **Reassign** or **End**.
- Reassigning changes whose rights the schedule uses, so the new person
  confirms it on a card first (decision 3), as a schedule from chat is
  confirmed today.
- Reactivating restores sign-in. Schedules stay suspended until each is
  resumed.

### Deleting a person's data (K11)

Each person gets a 256-bit data key. Their content rows in the content store
(deletion step 1) are encrypted with it, using the vault's cipher. Content
with no person uses an appliance key.

Deleting a person:
1. **Their items are removed** through the trash's removal (deletion step 2),
   which frees the disk.
2. **Their key is destroyed.** Any copy left anywhere becomes unreadable:
   rows that were missed, and backups.

Keys don't go in data backups. They are backed up separately and kept for
`[trash] keep_days` (decision 5). After that, no copy of a deleted person's
key exists. The log keeps a record that the person existed and was deleted,
with no content.

Company knowledge a person wrote and that was endorsed belongs to the
company. It is re-encrypted under the appliance key when it is endorsed
(decision 6).

## Tests

Installed app (Linux):
- **First-run setup:**
  - setup requires the setup code;
  - the owner is created;
  - the recovery code is shown once and works once;
  - no default credential signs in.
- **Invite and sign in with a passkey:** the test's Chrome uses a WebDriver
  virtual authenticator. The admin invites with roles, the invitee joins and
  registers a passkey, then signs in again with it.
- **Role limits enforced:**
  - an invited approver for the irreversible tier can't approve a financial
    action and isn't offered it;
  - without `people:admin` there's no People section and invites are refused.
- **Deactivation suspends schedules:**
  - a person sets up a schedule and is deactivated;
  - the schedule doesn't run at its time;
  - the admin's inbox shows Reassign and End, and each works.

Scratch tests cover each API, including the following:
- invites expire and work once;
- a revoked session is refused;
- two-factor lockout;
- after a person's key is destroyed, a restored backup can't read their
  content.

## Build order

1. People in the log and the identity interface (OIDC moved behind it), first-run setup and recovery code.
2. Invites with roles and groups, and a People section in Settings.
3. Sessions and devices with revoke, and two-factor for passwords.
4. Passkeys (after decision 1).
5. Deactivation: schedules and tasks suspended, inbox notices, reassign and end.
6. Per-person keys and deleting a person.
7. The SAML and SCIM providers, after the alpha gate and before the first enterprise pilot.

strix-alpha keeps Keycloak sign-in working throughout (decision 7).

## Open decisions for the owner

1. **A domain for passkeys.** Recommended: a name under a domain you own
   (for example `magnus.pyrallon.ai`), with a Let's Encrypt certificate from
   a DNS challenge. Passkeys then work in every browser and from phones. I
   need the name and a DNS API token for the challenge. The alternatives:
   - a local name with your own certificate authority, installed on each
     computer;
   - security keys only, over USB, through the app itself: no phone or
     synced passkeys on Linux.
2. **A `people:admin` role** for invites, deactivation and groups, apart from
   the owner. Recommended: yes; the owner has it.
3. **Reassigning a schedule needs the new person's confirmation.** Recommended:
   yes.
4. **A deactivated person's running tasks are suspended too.** Recommended:
   yes.
5. **Keys are backed up apart from data and kept for `keep_days` (30).**
   Recommended: yes, so a deleted person's data is unreadable everywhere
   after that.
6. **Endorsed company knowledge stays when its author is deleted.**
   Recommended: yes.
7. **strix-alpha.** Recommended: keep Keycloak and add built-in accounts
   beside it.
