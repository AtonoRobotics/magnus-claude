# The admin console against its spec

The owner's "Magnus Admin Console Spec" (rev 12,
https://claude.ai/artifact/3Ej2KCssZbFYfR4p1LEwGP) is built as written,
amended only by the owner's rulings (`docs/proposals/spec-decisions.md`;
ruling 1: no passkeys, admins sign in and step up with their password and
code). The console is done when every line below is checked, each with the
test that proves it. Nothing is reported done before that (owner,
2026-11-02: "do not build min slices and give up before the job is done").

Each line names the pull request that built it, or the one building it.

## Access

- [x] Its own process, `magnus-admin`, bound only to configured internal addresses: never every address, never loopback (#68; `admin_reach.rs`)
- [x] Only the organisation's LAN and VPN subnets; loopback refused, so a misrouted tunnel is refused (#68; `admin_reach.rs`)
- [x] The configured hostname only, else 404; every name it is reached by (#68, #79; `admin_reach.rs`, `admin_on_appliance.rs`)
- [ ] The tunnel's ingress names only the user API and the realm's paths; no admin hostname in public DNS (Operations › Network shows it; Removals)
- [x] TLS 1.3 only (#68; `admin_reach.rs`)
- [ ] A certificate browsers already trust: ACME DNS-01 under a domain the organisation controls, or the organisation's own CA (today: a CA made on the appliance, trusted by hand)
- [x] The kill switch from the appliance's own console: `magnus kill` / `magnus release` (before #68)

## Authentication and session

- [x] The `magnus-admin` confidential client, Authorization Code with PKCE, the one redirect URI per name, `private_key_jwt` (#69, #79)
- [x] Back-channel calls to the provider's internal address, issuer unchanged (#69)
- [x] The realm role `admin`, checked on every request from the session's validated token (#69; `admin_sign_in.rs`)
- [x] Admins sign in and step up with their password and code (ruling 1, no passkeys) (#69)
- [x] Server-side session, cookie `__Host-magnus_admin`, HttpOnly, Secure, SameSite=Strict (#69)
- [x] 30 minutes idle, 12 hours absolute (#69); adjustable under Security › Sign-in ( )
- [x] A CSRF token on every change (#69)
- [x] Sign-out ends the console's session and the provider's (#69)
- [x] An admin turned off at the provider loses the console within 5 minutes (#69; `admin_sign_in.rs`)
- [x] Step-up within 5 minutes for each action the spec names (#69, and each section's own)
- [x] Engaging the kill switch never needs step-up (#71)
- [x] A strict Content-Security-Policy; nothing from anywhere but the appliance (#68)

## Authority and audit

- [x] Only `magnus-admin` holds `magnus_root` (BYPASSRLS) (#70)
- [ ] Every read or write of a person's private data through the console is audited: admin, person, resource, action, time (Users › Data)
- [ ] Deleting a person's data destroys their key (Users › Delete)
- [x] Credentials are write-only everywhere (before #68; checked again under Security › Connectors)
- [x] The last admin cannot be removed, turned off or demoted (#72; `admin_users.rs`)
- [x] Every console action in the append-only log with who, what, when, source and before/after; `magnus_root` cannot change it (#70; `admin_audit.rs`)

## Web UI

### Shell

- [x] Top bar: organisation, status dot, kill switch, the admin's name (#68, #71); the app's own fonts and colours in every look (`admin-shell`)
- [x] The admin's menu: theme (light, dark, match computer; solid or glass) and sign out (`admin-shell`; `admin_shell.rs`)
- [x] Five sections in a sidebar, a bottom tab bar at phone width, the kill switch staying in the top bar (#68)
- [x] "Stop all agents", held for one second; the red banner on every screen with who, when and Release (#71)
- [x] Lists open a detail panel on the right; full screen at phone width (`admin-shell`; `admin_shell.rs`)
- [x] Reversible changes show a toast with Undo (`admin-shell`; `admin_shell.rs`)
- [x] Destructive changes open a confirm sheet; the riskiest ask for the target's name typed (`admin-shell`; `admin_shell.rs`; typed, under Users › Delete)
- [x] Step-up opens the provider's sign-in in a sheet and returns to the action (`admin-shell`; `admin_shell.rs`)
- [x] Live data over server-sent events, `/api/v1/events`: status and the kill switch (`admin-shell`; `admin_shell.rs`); tasks and activity join with their sections
- [x] Designed empty, loading and error states on every screen built so far (`admin-shell`), and on each section as it is built

### Overview

- [x] Status strip: health, kill switch, version (#71)
- [ ] Status strip: the tunnel connected or not (Operations › Network)
- [x] Cards: running tasks, people signed in, knowledge changes in 24 hours (#71)
- [x] The knowledge card opens Activity (`admin-shell`)
- [x] Alerts: a service down, disk over 85% (#71)
- [x] Alerts: a certificate expiring within 14 days (`admin-shell`; `admin_shell.rs`, `admin/cert.rs`)
- [ ] Alerts: the tunnel down; a model provider unreachable (Operations › Network, › Model providers)

### Knowledge

- [ ] Ontology: searchable types; definition, attributes, relations with cardinality, a graph of related types, entries referencing it, history
- [ ] Ontology: create, edit, add or remove a relation, merge, deprecate, delete (refused while in use); every change versioned, diff per version, rollback
- [ ] Context: org profile, structure, terminology, standing facts; who last changed each and when; edit in place; "As agents see it"
- [ ] Memory: searchable table (summary, entities, source, created, last used) with filters; detail with provenance, entities, versions; edit, delete, rollback; bulk delete and rollback with step-up
- [ ] Activity: every agent change, newest first, with before/after and source; revert one; revert a range with step-up; filters

### Users

- [x] List: name, email, role, status, last sign-in, sessions; Invite, Resend, Cancel (#72)
- [x] Overview tab: role (step-up), turn off or on, end all sessions; turning off pauses their schedules (#72)
- [x] Sessions tab, with End (#72)
- [ ] Schedules and triggers tab, with Suspend and Resume
- [ ] Data tab: their tasks, chats, files and memory, each access audited, with the banner saying so (ruling 14)
- [ ] Delete tab: the account and their key, with step-up and their email typed

### Security

- [ ] Sign-in: member methods (password, and whether a code is required), admin method shown fixed, session lifetimes for members and the console, lockout thresholds; applied through the provider's admin API with step-up
- [ ] Agent policy: each Cedar policy set in an editor with highlighting and inline validation errors
- [ ] Agent policy: a test panel (principal, action, resource, context → the decision and the policies that decided it)
- [ ] Agent policy: a diff against the live version; Apply with step-up; history and rollback
- [ ] Connectors and credentials: the organisation's connectors with status, scopes, last used; add, rotate, revoke; credentials write-only, shown "set" and when
- [x] Audit: the whole log, filtered by actor, action, target and time, with CSV (#70)

### Operations

- [x] Kill switch: state, who and when, tasks halted, schedules held (#71)
- [ ] Kill switch: microVMs frozen; Release with step-up from here
- [ ] Modules: name, version, status, what each provides; enable, disable, install from a package file, remove (step-up)
- [ ] Model providers: enabled, endpoint, models, a connection test, last error; never a person's own credentials
- [x] Health: kernel, event log, identity provider, console; CPU, memory, disk; recent errors (#71)
- [ ] Health: cloudflared, running microVMs, live
- [ ] Updates: version, available update with notes, install with step-up; the desktop app's channel and published version
- [ ] Network: tunnel status and public hostnames, the admin hostname, the allowlist (a change that would shut out the admin's own address refused), certificate expiry and renewal

## Server and API

- [x] One Rust binary serving the UI and a JSON API under `/api/v1` (#68)
- [x] The kernel over a local Unix socket (#70)
- [x] Postgres as `magnus_root`, insert and select only on the log (#70)
- [x] The provider's admin API through `magnus-admin-svc` with `private_key_jwt`, realm-management in the realm only (#72)
- [x] The kill switch the kernel's own, persisted, refusing new runs while on (#71, before)
- [ ] Every endpoint in the spec's table (tracked per section above)
- [x] Every change audited before it answers; a step-up endpoint without a fresh check returns 401 `step_up_required` (#70, #69)

## Invariant tests

Reach
- [x] Nothing on the public side serves a console page or admin route (`admin_reach.rs`)
- [x] Loopback refused (`admin_reach.rs`)
- [x] Outside the allowlist refused; the VPN subnet accepted (`admin_reach.rs`)
- [x] The wrong Host gets 404 (`admin_reach.rs`)

Sign-in and authority
- [x] A member's session or token is refused on every admin endpoint (`admin_sign_in.rs`)
- [x] An admin cannot sign in without their password (ruling 1) (`admin_sign_in.rs`)
- [ ] Every step-up endpoint returns `step_up_required` without a fresh check and succeeds with one (grows with each section)
- [x] An admin turned off loses the console within 5 minutes (`admin_sign_in.rs`)
- [x] The last admin cannot be removed, turned off or demoted (`admin_users.rs`)

Data and audit
- [x] As the user API's role, a cross-person read returns nothing (before #68)
- [ ] An admin's read of a person's data succeeds and is in the audit log with admin, person, resource and time
- [ ] No admin endpoint ever returns a credential value
- [x] `magnus_root` cannot change an audit entry (`admin_audit.rs`)

Knowledge
- [ ] An admin edits an ontology type and a memory entry, sees provenance on both, and rolls each back
- [ ] An agent's change to org memory appears in Activity, and reverting it restores the prior state

Kill switch
- [x] Hold-to-stop at phone width halts a running task and a pending schedule within 5 seconds (#71)
- [x] Engaged across a restart, and no new run starts (before #68)
- [x] `magnus kill` engages it with the network down (before #68)
- [x] Release without step-up fails (#71)

UI
- [ ] Every screen in light, dark and glass, and at phone width with no horizontal scroll

## Removals

- [x] No `admin.pyrallon.ai` ingress, no Cloudflare Access application, no admin route on the user API, no admin listener on every address or loopback (checked in #68; Network shows the tunnel's names)
