# Progress

## Now (2026-10-28)

- The invariant tests pass: 45 of 45, on three consecutive full runs and again after every later change.
- **Verification can no longer be talked around (2026-10-28).** The "Research, architect, and build your replacement" run reached Done by rewriting its contract to a trivial criterion. Now: criteria freeze when work starts; a later change is a `contract_change` inbox card (old and new side by side) and counts only once approved; checks may run sandbox commands and read untrusted results as data without tainting the run; a check that cannot run goes to the review card unchanged; the plan view shows no unit as active after Done. Covered by `tests/scratch/tests/contract_freeze.rs` (5 tests) and a plan check in `desktop/tests/flow.mjs`.
  - The requested invariant test (`criteria_changed_after_work_begins_need_an_approved_contract_change`) is written and passing, and 4 of 4 mutants against it were caught (frozen run accepts a new contract; contract never freezes; a declined change takes effect; the approval is not recorded). `tests/invariants/` is closed to Claude by permission settings, so the test sits in the scratch file and `docs/proposals/inv7-contract-freeze.patch` adds it to invariant 7: `git apply docs/proposals/inv7-contract-freeze.patch`.
- A clean-context security review found 14 kernel holes the tests did not catch. All are fixed and covered by `tests/scratch/tests/review.rs` (7 regressions, passing). The test weaknesses it found are proposed as contract changes in `docs/proposals/invariant-tests-v2.md`; these need the owner's approval.
- **Live models.** The owner signed in to Codex and Grok (device sign-in). Both pass the full conformance suite against the live provider (2026-10-27): Codex with `gpt-6.1-sol`, Grok with `grok-4.7`. Claude, GLM and Muse are not signed in. Next: a plain-language run. The task model is currently set to `grok`/`4.7`, which Grok does not offer; pick `grok-4.7` (or a Codex model) in Models first.
- **Desktop app (2026-10-27).** It is built and tested; the owner's recorded runs (one on Linux, one on Windows) are still to do. Steps are in `desktop/README.md`.
  - Installers are also on the private GitHub pre-release `desktop-v0.1.0` (https://github.com/AtonoRobotics/magnus-claude/releases/tag/desktop-v0.1.0), for getting the Windows app onto the PC: browsers on the PC refuse the self-signed certificate and bounce between https and http.
  - Built locally at: `desktop/src-tauri/target/release/bundle/deb/Magnus_0.1.0_amd64.deb`, `.../appimage/Magnus_0.1.0_amd64.AppImage`, and `desktop/src-tauri/target/x86_64-pc-windows-gnu/release/bundle/nsis/Magnus_0.1.0_x64-setup.exe`.
  - Tested by behavior: the shared frontend in Chromium (`desktop_flow`) and the built Linux app through tauri-driver over HTTPS (`desktop_app`, `--ignored`). Both pass. The Windows build passes its unit tests under Wine; its window is untested until the owner's run.
  - Sign-in is through Keycloak (realm `magnus` at identity.pyrallon.ai), with the device-code grant; passwords stay as the backup. Built and tested against a fake provider (kernel tests, browser flow, real Linux app). The owner registered the `magnus-claude` client with `sudo python3 -I config/keycloak-client.py` (2026-10-28); against the real Keycloak the appliance now starts a device sign-in and polls it (pending until approved). A full sign-in by a person is not yet observed. The Keycloak account `samuel` still has a required "update account information" step, done in the browser at first sign-in.
  - A backup password is set in a real terminal (a `!` command has no stdin): `read -rs pw; printf '%s\n' "$pw" | ~/.magnus-appliance/bin/magnus password --config ~/.magnus-appliance/magnus.toml --person samuel; unset pw`. Claude needs an Anthropic API key, pasted in Models.
- The dev appliance serves HTTPS on all interfaces, port 7700 (https://192.168.0.50:7700; certificate fingerprint printed in `~/.magnus-appliance/serve.log`). Plain HTTP is on loopback only, at 127.0.0.1:7701. The owner opened 7700/tcp in ufw (2026-10-27).
  - The owner has the `developer` role.
  - A `user/samuel` policy layer asks before `deliver_file`, so the recorded run has an approval. Clear that layer to stop it.
  - Plain `http://` on port 7700 redirects to `https://` (live since 2026-10-28). Browsers still warn about the self-signed certificate (Advanced, then continue); the desktop app pins it instead. A trusted name (Cloudflare tunnel or Tailscale) would remove the warning; that is the owner's call.
  - Previous config and binaries are kept as `bin/prev-20261028080553/` (before the redirect), `magnus.toml.bak-20261028073104` and `bin/prev-20261028073104/` (before Keycloak sign-in), and `...-20261027232730` (before HTTPS).
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
  - `openai`: Chat Completions and Responses, with API key or device sign-in (Codex and Grok: approve on the provider's site, Magnus finishes by itself). Presets for GLM, Muse, Codex and Grok in `config/providers.toml`. Codex and Grok client versions are read from each CLI's published release, not stored.
  - Conformance suite: `magnus conformance`, `POST /v1/adapters/{id}/conformance`.
- Sign-in and sessions (Argon2id, held outside the log), HTTPS with a self-signed certificate that apps pin, the `developer` role.
- Developer API (`/v1/dev/*`): run inspector, raw log, experiments (pass@k, pass^k, paired comparison, failure tags), live panels, policy history with reliability, playbook. Also plans (`task_plan`), playbook items with taint (`playbook_update`), composed outcomes with a spoken script, blocking and tier flags on inbox items, and `/v1/versions`.
- Desktop app (`desktop/`, Tauri, Linux and Windows):
  - operator view: Home, Inbox, New task, Tasks, Task detail, Models
  - developer view: Runs, Experiments, System, Policy, Playbook
  - The appliance serves the same frontend to browsers.
- Evaluation:
  - `eval/plain`: 14 plain-language tasks, graders, runner. With a developer token the runner records each trial as an experiment and posts its grade, so the developer view compares configurations. The graders have not been checked against known correct and wrong answers; there is no record of that check.
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
  - The owner's access key is the `token` in that config. It works on the loopback port (7701) for scripts such as `eval/plain/run.py`, which reads the address from the config.
  - People sign in through Keycloak, or with a backup password, over HTTPS (port 7700).
  - It is not a system service; the owner declined persistence.
- Tests:
  - `cargo test -p magnus-invariants --no-fail-fast` (the contract)
  - `cargo test -p magnus-scratch` (regressions)
  - `cargo test -p magnus --test tools` (browser and desktop)
- Desktop app: see `desktop/README.md`. Build in `desktop/src-tauri` with `cargo tauri build`. Tests: `cargo test -p magnus-scratch --test desktop_flow`, and `--test desktop_app -- --ignored`.
  - The app test needs `tauri-driver`, `xvfb-run` and WebKitWebDriver. WebKitWebDriver is not installed system-wide. Get it with `apt download webkit2gtk-driver && dpkg-deb -x` and set `MAGNUS_WEBKIT_DRIVER`.
- The scratch tests start the appliance binary from `magnus-invariants`. Run `cargo build -p magnus-invariants --bins` first, or they use a stale kernel.
- Read `DECISIONS.md` before changing behavior. Do not edit `tests/invariants/` without the owner's approval.
