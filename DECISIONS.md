# Decisions

One line per choice the brief left open, with its reason. The brief and the owner's later instructions win over this file; the decision record (rationale) is background.

## Process

- Built independently of other `~/magnus*` attempts on this machine; no code or decisions are copied. Reason: this workspace was started fresh for this brief.
- The invariant tests drive the appliance as a real process through its outside surfaces only (config file, `migrate`/`serve` CLI, HTTP API, Postgres log) plus the external protocols it speaks. Reason: the tests are a fixed contract that must survive internal rewrites, and crash tests need a process that can be SIGKILLed.
- Every external system in the invariant tests is a fake: a model provider speaking the Anthropic Messages API, MCP servers for the person's apps, and a website. Reason: the tests exercise the real adapter and MCP client paths without live accounts.
- The fake model is scripted and often scripted to misbehave (lie about completion, follow injected text, exfiltrate). Reason: the invariants must hold whatever the model decides, so the tests cannot rely on a well-behaved model.
- Edits to `tests/invariants/` are denied in `.claude/settings.json`. Reason: the owner approves changes there, and a permission rule enforces it structurally (X3).
- Dev and test Postgres is the `magnus-claude-pg` container on 127.0.0.1:54340. Reason: other projects on this machine use 54329 and 54330.
- Rust toolchain pinned to 1.99.1. Reason: the system cargo is 1.75, too old for current crates.
- I write the agent loop, because the brief asks me to build the whole system (the record's X14 says the principal writes it). It lives in one module (`crates/magnus/src/run/loop.rs`); once the owner has reviewed it, edits to it are denied like the tests. Reason: the brief overrides the background record, and X14's protection still applies after review.

## Event log and state

- `public.events` is the only source of truth. Every other table is a derived projection that can be truncated, and `serve` rebuilds it from the log on boot. Reason: invariant 3 says the log is the source of truth, and truncation tests it directly.
- The kernel connects as a role that may only INSERT and SELECT on `events`; `migrate` creates that role and the schema with an admin URL. Reason: append-only is enforced by the database, not by kernel code.
- The vault is an encrypted file in the appliance state directory, not a database table. Reason: secrets must stay out of the log, and dropping derived tables must not lose them.

## Boundary and policy

- Cedar actions: `execute` (act without asking) and `request_approval`. Outcome: Allow if `execute` is permitted, Ask if only `request_approval` is, otherwise Deny. Reason: Cedar has permit and forbid but no ask effect.
- Cedar entities: `Person` in `Group`; `Tool::"<app>__<tool>"` in `App`, with attributes `risk` and `audience`; context carries the run's `untrusted` and `private` taint. Reason: these are the facts the boundary decides on (B4, B9, A6).
- Layering: an action is permitted only if the organization layer permits it and no layer (org, each of the person's groups, user, task) forbids it. A permit in a lower layer never widens. Reason: E1 makes the organization the ceiling and lets lower layers only narrow; deny wins.
- With no organization policy written, the default is execute for read, reversible_write and sandbox, and Ask for everything else. Reason: nothing irreversible runs without owner-written policy, and the product stays usable out of the box.
- An accepted policy proposal appends its rules to the target layer. Reason: a proposal is a change to review, not a replacement of the owner's whole policy.
- The `prohibited` class is a hard Deny that no policy or approval lifts. A tool the owner has not classified cannot be called. Reason: fail closed (B9).
- Rule of Two is fixed in the kernel, per K47. When a run holds both untrusted and private taint, every non-read action and every web fetch or search asks. Reason: a fetched URL can carry private data out (EchoLeak), and K47 allows stricter-than-K34.
- Audience rule: a run holding private taint that sends to an `internal` or `public` destination asks, even when the destination is allowlisted. Reason: B4.
- Approvals lift Ask only, never Deny. Approvers need the `owner` role or `approver:<risk class>`. Scope is once, task, time window or standing, and every approval is bound to the exact action. Reason: B5, E7.
- A person's rights come from the groups the identity provider gives them; an app lists `access_groups`. Missing rights is Deny, not Ask. Reason: E2; an approval must not get around the person's own access.
- Tool definitions are pinned by hash on first use; a changed definition is disabled and hidden from the model until the owner re-approves it. Reason: B10.
- The idempotency key reaches apps as an `Idempotency-Key` HTTP header (and in MCP `_meta`). After a crash, an app without idempotency support gets no resend: the intent is marked unknown and the person is asked. Reason: B7.

## Runs and verification

- The agent must record a task contract (`task_contract`) before any non-read action and before completing. Reads are allowed before it so the agent can scope the task. Reason: A1 says contract before acting; reading to write a good contract is not acting.
- Contract checks must call read-only tools. The kernel runs them as written when the agent reports completion, and records the result as evidence. Reason: K26, K39.
- A criterion without an executable check goes to the person as a review card when the verifier is off; the run suspends. Reason: K37.
- A model reply with no tool call is a malformed step. It goes back to the model as an error and does not end the run. Reason: K29; only the model's `task_complete` or `task_fail`, or the principal, ends a run (K40).
- An exhausted budget or a persistent provider failure suspends the run with an inbox notice. The principal resumes it on the pinned model. Reason: K24, K12.

## Identity

- In v1 the appliance config lists people with their groups and roles (`owner`, `approver:<risk>`, `policy:user`, `policy:group:<id>`); each person has an API token for the desktop app. Reason: K46 requires groups from an identity provider, the appliance needs a local one to start, and OIDC can replace it behind the same interface.

## Models

- Model providers are separate adapter processes behind one versioned contract the kernel speaks over a local socket. Reason: owner instruction (2026-10-27).

## Sandbox

- Guest kernel: Firecracker CI `vmlinux-6.1.155` (v1.15 artifacts). Reason: upstream-tested with Firecracker 1.16 and has vsock and virtio-blk built in.
- The jailer is used when the appliance runs as root. Tests run Firecracker unjailed as an unprivileged user in the `kvm` group. Reason: the jailer needs root for chroot and cgroups.

## Evaluation

- TheAgentCompany: a handful of tasks while developing, then one full run of all 175 for the reported score; spend cap asked once per-task cost is measured. Reason: published scores are over all 175, and at a ~30% pass rate a 40-task sample has a ±14-point 95% interval against ±7 for the full set.
- Live provider sign-in happens through the app (API-key entry, OAuth links); tokens go straight to the vault. Reason: owner's choice (2026-10-27).
- Two adapters cover the five providers: `anthropic` (Messages API) and one OpenAI-compatible adapter configured per provider (Chat Completions or Responses, API key or OAuth, base URL, extra headers) for GLM (api.z.ai), Muse (api.meta.ai), Codex (chatgpt.com/backend-api/codex) and Grok (cli-chat-proxy.grok.com). Reason: owner instruction that OpenAI-compatible providers share one adapter; research in docs/research/providers-and-tac.md.
- The adapter contract adds a fifth error class, `invalid_request` (for example, context too long), to the four the owner named. Reason: the kernel must tell a rejected request (no retry) from an unavailable provider (retry).
- One connection per adapter request over a Unix socket, newline-delimited JSON. Reason: a replaced adapter takes new requests on its new socket while in-flight requests finish on the old one, with no kernel restart.
- The kernel passes the vault credential in each request and stores any refreshed credential the adapter returns; adapters keep no token state. Reason: tokens come from the vault, and a replaced adapter must not lose sign-ins.
- While the kill switch is engaged, the boundary refuses every action and the credential stage releases no secret; the vault itself is kept. Provider-side revocation of OAuth tokens is left to the adapters. Reason: B11's intent is that nothing can act; deleting the vault would also delete the model key and every sign-in, so releasing the switch would leave a dead appliance.
- Canonical JSON (keys sorted at every level) for every hash: approval bindings, idempotency, tool pins. Reason: Postgres jsonb reorders object keys, so a call read back from the log must hash like the original.
- The audience rule asks only when private content read from a different app goes to an internal or public audience; content returning to the app it came from reaches no one new. The Rule of Two is unchanged and still gates every outbound action once untrusted and private content have both been read. Reason: B4's documented leaks (GitHub MCP, EchoLeak) all involved injected content, which the Rule of Two stops; without the exemption, posting to a team chat twice would ask because the chat's own acknowledgement carries the chat's label.
- Files attached to a task are labeled untrusted: the model sees them (images and PDFs as content, every file at /workspace/inputs/), but they instruct nothing. Reason: a document from elsewhere can carry injected text; only the person's task text instructs (invariant 4).
- Voice input becomes text on the appliance (local speech-to-text), and the person sends that text as the task, so the instruction is what the person confirmed. Reason: voice is a way to type, not a separate channel.

## Security review fixes (2026-10-27, clean-context review)

- Labels are a set: a source can be `private+untrusted` (a mailbox: the person's mail, written by others). Reason: with one label per source, an injected email plus a URL fetch passed the Rule of Two (EchoLeak).
- Memory entries keep the taint and source apps of the run that wrote them, and a search result carries the combined taint of what it returns. Reason: memory otherwise launders private or injected content into later runs (M1).
- Taint asks (Rule of Two, audience) override a policy ask and accept only a one-action approval of the exact input; task, window and standing scopes cover policy asks only. Only the owner grants standing approvals; windows last at most 24 hours. Reason: a broad approval given before an injection must not cover what the injection asks for, and standing approval is autonomy (invariant 6).
- Approvals record why they were asked for; an approval for an unknown outcome is used only to resend that call. Reason: otherwise approving a resend could be consumed as a policy approval, and the person would be asked forever.
- With both taints, a call to any app with an audience wider than the person counts as transmitting, reads included. Reason: a search query carries its inputs to the app.
- Web fetch follows redirects itself, checks each hop's address, and connects to the address it checked. Reason: redirects and DNS rebinding otherwise reach private and cloud-metadata addresses.
- Contract checks may not use memory tools, may not expect null or an empty substring, and fail when the pointer does not resolve; a contract has at most 20 criteria. Reason: checks must look at the world (invariant 7), and a vacuous check is no check.
- Redaction happens at one place, inside the MCP client, for every string an app returns (results, structured data, errors, tool descriptions and schemas). The kill switch is checked before any stage that touches a credential. Reason: echoed credentials leaked through error paths and tool listings.
- The text of a tool definition is shown to the model only after the owner approves it (one review item per app on first sight); until then the model sees the tool's name, risk class and a schema without descriptions. Reason: tool descriptions are third-party text (B10); pinning only caught later changes.
- Apps are not assumed to deduplicate: `idempotency` defaults to `none`, and only `header` allows a resend after a crash. Reason: a wrong assumption repeats a side effect.
- Run-record inbox items (questions, proposals, messages) take their id from the call, so a crash cannot duplicate them. Agent notes are `message` items that show the run's taint, and only the kernel's own notices can extend a budget or continue a run. Reason: an injected agent must not be able to impersonate the kernel's notices.
- Firecracker starts with an empty environment. The jailer is still to be wired for the root-run appliance (PROGRESS.md). Reason: defense in depth against a VMM escape.
- The browser is Chromium in a host-side sidecar (Node, playwright-core) reached over a Unix socket, one isolated context per run, with no saved logins. Navigation and snapshots are reads; clicks, typing and selects are reversible writes by default. Everything it returns is untrusted, and with both taints every browser call asks. Reason: the sandbox has no network, so web use must run outside it behind the boundary; with fresh contexts per run the browser holds no sessions that could pay or post as the person.
- Computer use runs in the run's own microVM: an Xvfb desktop with LibreOffice and other X apps, driven with xdotool, screenshots returned as images. Reason: desktop apps need no network, and the VM is already the isolation boundary.
- The guest agent stops collecting a command's output shortly after the shell exits, even if background processes keep the pipes open. Reason: `app &` is how the model starts desktop apps, and it must not hang the call.
- Files leave a run only through `deliver_file`, which copies them into the log as outputs the person downloads from the task view; the workspace is deleted when the run ends. Reason: K19, and a non-technical person needs the spreadsheet, not a path inside a VM.
- Recurring tasks are created by people; the agent can only propose one (`propose_schedule`), which starts only if the person accepts. A scheduled task acts for the person who set it up. Reason: a schedule is a standing instruction, and an injected agent must not be able to plant future tasks (invariant 4); K54.
- The desktop app is an Electron shell around the appliance's web app, adding OS notifications for new inbox items and catching the ChatGPT sign-in redirect on localhost:1455. Reason: one UI for browser and desktop; Tauri would need WebKit development packages installed system-wide.
- Web search is a local SearXNG instance. Reason: no API key or third-party search account, and queries stay under the appliance's control.
