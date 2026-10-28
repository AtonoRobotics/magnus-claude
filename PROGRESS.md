# Progress

## Now (2026-10-27)

- The invariant tests pass: 45 of 45, on three consecutive full runs and again after every later change.
- A clean-context security review found 14 kernel holes the tests did not catch. All are fixed and covered by `tests/scratch/tests/review.rs` (7 regressions, passing). The test weaknesses it found are proposed as contract changes in `docs/proposals/invariant-tests-v2.md`; these need the owner's approval.
- **Blocked on live models.** The dev appliance listens on all interfaces, port 7700 (http://192.168.0.50:7700 on the LAN). The owner opened 7700/tcp in ufw (2026-10-27). It is waiting for the owner to sign in to at least one provider (Settings → Models). Until then, no adapter conformance suite, plain-language score or TheAgentCompany score can be produced.
- TheAgentCompany preparation is with a background agent: service stack, three sample tasks, and a runner design in `eval/tac/`.

## Done

- Kernel (`crates/magnus`):
  - event log (Postgres, append-only role); boundary (eight stages, layered Cedar)
  - Rule of Two and audience rules, multi-label taint, approvals bound to their reason
  - vault with central redaction; adapter registry with hot swap gated by conformance
  - run loop, contract checks against the world, inbox, HTTP API
  - schedules (people create them, the agent proposes them), memory with taint, deliverable outputs
- Tools:
  - Firecracker sandbox (no network; Python, office, PDF, OCR)
  - computer use (Xvfb desktop in the VM)
  - browser (Chromium sidecar on the host)
  - web fetch (SSRF-safe) and search (local SearXNG)
  - MCP apps with first-sight tool review
  - image and PDF input; voice input (needs a whisper model, below)
- Adapters:
  - `anthropic`: Messages API.
  - `openai`: Chat Completions and Responses, with API key, OAuth PKCE or device sign-in. Presets for GLM, Muse, Codex and Grok in `config/providers.toml`.
  - Conformance suite: `magnus conformance`, `POST /v1/adapters/{id}/conformance`.
- Desktop app (`desktop/`, Electron): inbox, task list, task view with evidence and downloads, settings, OS notifications, ChatGPT sign-in capture.
- Evaluation:
  - `eval/plain`: 14 plain-language tasks, graders validated against correct and wrong answers, runner.
  - `eval/tac`: in progress.

## Next

1. Owner signs in, then: conformance suite per adapter (live), plain-language run (`python3 eval/plain/run.py`), first TheAgentCompany tasks, then a spend cap for the full 175.
2. TheAgentCompany runner and connectors to its services (GitLab, RocketChat, ownCloud, Plane).
3. Context compaction (C5) and context reset (A5) for long tasks; verifier module (removable, off by default).
4. Jailer for the root-run appliance. MCP servers for the person's mail, calendar and files (needs the owner's choice of servers and OAuth apps).
5. Voice: install whisper.cpp and a model (`MAGNUS_WHISPER`, `MAGNUS_WHISPER_MODEL`).

## How to resume

- Toolchain: `export PATH=$HOME/.cargo/bin:$PATH` (rust-toolchain.toml pins 1.99.1).
- Postgres: `docker start magnus-claude-pg` (127.0.0.1:54340, password `magnus_dev`). Search: `docker start magnus-searxng`.
- Sandbox assets: `sandbox/build.sh` writes `sandbox/out/{vmlinux,rootfs.ext4}` (gitignored). Grader image: `docker build --target rootfs -t magnus-grader sandbox/cache`.
- Dev appliance: config, state and binaries in `~/.magnus-appliance/`.
  - Start: `~/.magnus-appliance/bin/magnus serve --config ~/.magnus-appliance/magnus.toml`.
  - The owner's access key is the `token` in that config.
  - It serves plain HTTP, so access keys cross the network unencrypted. TLS is not built yet.
  - It is not a system service; the owner declined persistence.
- Tests:
  - `cargo test -p magnus-invariants --no-fail-fast` (the contract)
  - `cargo test -p magnus-scratch` (regressions)
  - `cargo test -p magnus --test tools` (browser and desktop)
- Desktop app: `cd desktop && npm install && npx electron .` (add `--no-sandbox` on a dev box without the setuid helper).
- Read `DECISIONS.md` before changing behavior. Do not edit `tests/invariants/` without the owner's approval.
