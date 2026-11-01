# The owner's specs against Magnus today: what needs your decision

Status: questions for the owner, each with a proposed answer. Sources: the
owner's "Magnus Connectors Spec" (rev 17), "Magnus Admin Console Spec"
(rev 12) and "Magnus Authentication & Isolation Design" (rev 6), all of
2026-10-31, read against master on 2026-11-01. "DEC n" is a line of
DECISIONS.md as of that date; plans already written are `auth.md` and
`admin.md` beside this file.

**Where things stand.** The admin console is almost entirely unbuilt. About
a sixth of the connectors spec exists (one owner-added MCP server per app,
with OAuth, refresh and revocation). The authentication design reverses
several decisions recorded on 2026-10-30 and 2026-10-31. One bug the review
found is fixed separately (two owners demoting each other at once left no
owner; branch `last-owner`). Everything below waits on a ruling; nothing in
it is built until you rule, and the four marked **invariant** also need your
approval to change a protected test.

## Decisions that change recorded ones

| # | Question | The specs say | Today | Proposed |
| --- | --- | --- | --- | --- |
| 1 | How does a person sign in to the app? | The system browser, passkeys first; the app never collects a password (auth §2). | The app takes the username and password itself (DEC 274, 276; "just like logging into muse or grok bot", 2026-10-30), with the browser as fallback. | Follow the spec: the browser with passkeys, and record that it supersedes DEC 274 and 276. The typed password goes. |
| 2 | Does the owner keep the backup password? | The admin signs in with a passkey only; Magnus keeps no passwords (admin test 6, auth §5). | The owner's backup password with a one-time machine code works when the provider is down (DEC 278, 339). | Keep it only as the way back in when the provider is down, on this machine, and never for the admin console. |
| 3 | Who is a person? | The token's `sub` (auth §3). | The `preferred_username` claim (`config.rs:166`). A Keycloak account deleted and made again, or renamed, under the same username becomes the old person, with their tasks, memory and approvals. | Bind each person to their `sub` at their next sign-in and refuse a different `sub` after that. Existing people keep their records. |
| 4 | Which roles are there? | `admin` and `member` (auth §2). | `owner`, `approver:<risk>`, `policy:*`, `developer`, `tester`, `knowledge:endorse`; the invariants rely on `owner` and `approver:*`. | Map Keycloak's `admin` to `owner` and `member` to a person; keep the other roles as Magnus's own, set in Settings or by group (accounts.md step 4). |
| 5 | Can a person relax an approval the organisation set? | "A user or admin can relax" the default for local code tools (connectors). | Lower layers only narrow (DEC 28; invariants 6 and 8). | No: a person can only tighten. The admin relaxes for everyone. |
| 6 | Does installing an appliance update need the admin's step-up? | Yes (admin, Operations > Updates). | Updates install by themselves, with rollback (DEC 177-180, 365). | Keep automatic updates; step-up only for a manual install or a rollback. |

## Decisions the invariants depend on

| # | Question | The specs say | Today | Proposed |
| --- | --- | --- | --- | --- |
| 7 | **invariant** Who classifies a connector's tools? | A catalog's annotations set each tool's class, and a changed tool list is followed (connectors). | The owner classifies every tool before it runs, and a changed definition is disabled until re-approved (invariant 1: `tools_the_owner_has_not_classified_cannot_run`, `a_changed_tool_definition_is_disabled_and_hidden`; DEC 36, 102, 172). | A signed catalog manifest's classes count as the owner's classification; a changed definition arrives as a new manifest version, which the owner approves. The invariant tests stay as they are. |
| 8 | **invariant** Can the user API keep the owner's controls? | Owner routes and long-lived config tokens leave the user API for the admin console (admin). | The kill switch, secrets and policy are driven through the user API, as invariants 1, 2 and 6 do. | Build the console beside them first; move each control only with its invariant test moved too, with your approval each time. |
| 9 | **invariant** Per-person row-level security, and the log rebuilt at start | Row-level security on `sub`; a cross-person read returns nothing (auth §4, admin test 10). | The kernel rebuilds all state by reading every event at start (DEC 20). | Row-level security on the per-person content store only (what a person wrote and what was written for them), with replay reading through its own role. An architecture change: its own plan first. |
| 10 | **invariant** What does the kill switch do from the console? | Halt, suspend, pause VMs; release needs step-up (admin). Silent on the vault and sign-ins. | It also seals the vault and revokes every OAuth sign-in (DEC 86, 308). | The console's switch is the same switch: it seals and revokes as today, and release needs step-up. |

## Decisions on behaviour

| # | Question | Today | Proposed |
| --- | --- | --- | --- |
| 11 | New work while the kill switch is on | A task or chat turn is accepted, its run suspended at once; on release its person gets "Stopped by the kill switch" with Continue (DEC 336). The admin spec says new runs are refused. | Refuse: the person is told the kill switch is on, and nothing is queued. |
| 12 | A person turned off in Keycloak | Their sessions end within 10 minutes (DEC 341), but their schedules go on running for them. | Their schedules pause, and resume only when they are turned back on and resume them. The check covers everyone with a schedule, every 5 minutes, and records each session it ends. |
| 13 | Long-lived config tokens | Accepted on every listener, the LAN's HTTPS port included. | Only on the plain loopback listener, where the owner's scripts and the tests use them. |
| 14 | Can the admin read a person's private data? | The admin spec gives root a Data tab over tasks, chats, files and memory; the connectors spec says admins see only a connection's metadata, never what it returns. | One rule: root reads a person's data only with step-up, each read recorded and shown to the person; connector results are part of that data. |

## Smaller conflicts

| # | Question | Proposed |
| --- | --- | --- |
| 15 | The auth design bans mTLS and device certificates; the connectors relay uses mTLS; accounts.md plans device certificates. | The ban is for people's devices; the relay keeps mTLS; accounts.md's device-certificate step is dropped. |
| 16 | "One public client" beside the console's confidential client, its service account, and the status client. | Name them all: the app's public client, the console's confidential client, and service accounts for the console and the status check. |
| 17 | One `magnus.` and one `auth.pyrallon.ai` for many organisations' appliances. | Each appliance gets its own name under the organisation (`<org>.pyrallon.ai`, `auth.<org>.pyrallon.ai`); the registry that maps them needs a plan. |
| 18 | Org-level connectors in the console; none in the connectors spec. | Leave them out until the connectors spec defines one. |
| 19 | A persistent VM per person (auth §4) against a VM per run (DEC 109, 43). | A VM per run, never shared across people, as today. |
| 20 | Certificate pinning (DEC 111, 121) behind Cloudflare, which terminates TLS. | Behind the tunnel the apps trust Cloudflare's public certificate; pinning stays for the LAN address. Say in the trust model that Cloudflare sees the traffic. |
| 21 | "Done means … on Windows and Linux" (connectors) against no Windows tests (DEC 167). | Linux tested, Windows by the owner's recorded run, as for the app. |
| 22 | "Library" (files today; connectors in the spec) and "module" (removable run modules; installed packages; connectors). | "Library" stays the files; connectors are the "connector catalog"; "module" stays the removable run modules. |

## Not yet specified

- The appliance registry and the hostname of the connect flow's completion endpoint: who runs it, and how the relay knows an appliance.
- The "curator service identity" (auth §4): the memory pass, the knowledge extractor, or a new agent?
- The console's Context and Ontology: what they hold, and how agents receive them under pinned instructions.
- Which ACR value means "passkey" for step-up, and Keycloak's mapping for it.
- A per-VM egress proxy for brokered connectors.
- How an admin-added server's tools are approved once nobody classifies them by hand (see 7).

## Order once ruled

Known bugs first (the owner race is done), then: identity by `sub` (3), the
kill switch and new work (11), schedules of a person turned off (12),
config tokens on loopback (13); then the authentication foundations, the
admin console, and the connectors, each in its own pull request, as
`auth.md` and `admin.md` order them.
