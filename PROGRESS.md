# Progress

## Now

All 45 invariant tests pass (`cargo test -p magnus-invariants`), on three consecutive full runs (2026-10-27). A clean-context security review of tests and kernel is running; its findings go below when it reports.

Next up: live model access (OpenAI-compatible adapter with OAuth and API-key sign-in, conformance suites), then the desktop app, then the remaining tools and TheAgentCompany.

## Done

- `crates/magnus` kernel:
  - Event log in Postgres (`public.events`, append-only role created by `magnus migrate`); the in-memory projection is rebuilt on boot.
  - Execution boundary (`boundary.rs`) with identify, lease, budget, policy, approval, idempotency, credentials and execute stages. Policy layers are Cedar (`policy.rs`). The Rule of Two and audience rules are fixed in the kernel. Approvals are out of band (`inbox.rs`).
  - Vault encrypted at rest, with redaction of echoed secrets.
  - Run loop (`run/agent_loop.rs`), context and run records (`run/mod.rs`). Contract checks run against the world.
  - HTTP API (`api.rs`); MCP app client (`apps.rs`); web fetch (`web.rs`); Firecracker sandbox (`sandbox.rs`).
  - Adapter registry with hot replacement (`adapters.rs`).
- `crates/adapter-proto`: the `magnus.adapter/1` contract.
- `crates/adapter-anthropic`: Messages API adapter (streaming, thinking state, error classes, cost).
- `sandbox/`: guest agent (PID 1, vsock), image build (`build.sh`: Firecracker CI kernel 6.1.155 plus a Debian/Python rootfs with office, PDF, OCR and X tools).
- `tests/invariants`: the contract. `tests/scratch`: free-form debugging scenarios on the same harness.

## Next

1. OpenAI-compatible adapter (Chat Completions and Responses; API key and OAuth device/PKCE flows) for GLM, Muse, Codex and Grok. Conformance suite per adapter against the live provider, once the owner signs in.
2. Desktop app: inbox, task list, task view with evidence, sign-in screens.
3. Tools: browser (Chromium on the host behind the boundary), web search (SearXNG), computer use (Xvfb in the sandbox), image/PDF/voice input, schedules and triggers, notifications.
4. TheAgentCompany runner; plain-language end-to-end runs with scores.

## How to resume

- Toolchain: `export PATH=$HOME/.cargo/bin:$PATH` (rust-toolchain.toml pins 1.99.1).
- Postgres: `docker start magnus-claude-pg` (127.0.0.1:54340, password `magnus_dev`).
- Sandbox assets: `sandbox/build.sh` writes `sandbox/out/{vmlinux,rootfs.ext4}` (gitignored).
- Run the invariant tests: `cargo test -p magnus-invariants --no-fail-fast`.
- Debug a scenario: add a test to `tests/scratch/tests/debug.rs` and run it with `-- --nocapture`. Build `magnus-invariants --bins` first so the appliance binary is current.
- Read `DECISIONS.md` before changing behavior. Do not edit `tests/invariants/` without the owner's approval.
