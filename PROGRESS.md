# Progress

## Now (2026-10-28)

- The invariant tests pass: 45 of 45, on three consecutive full runs and again after every later change.
- **Connectors and the browser (2026-10-28).** Branch `connectors`, PR #2 (stacked on #1).
  - **Connectors:** the owner adds MCP apps in the app (Connectors), from presets (Gmail, Google Calendar, Google Drive, Microsoft 365, GitHub, Files) or by hand.
    - Each person signs in: device code, loopback with PKCE, or API key. Tokens are kept in the vault and renewed.
    - Tools are pinned and classified on the review card. Labels: mail and calendar private+untrusted, files private.
    - Sent messages are signed "Sent by Magnus, a digital worker, on behalf of <name>" (E6). Stdio apps run with a cleared environment.
    - Tests: `connectors.rs` (6), and `connectors_flow.rs` with `desktop/tests/connectors.mjs`, the whole thing in Chromium through the app. Screenshots from that run were sent to the owner.
  - **Browser:** each run's browser is a microVM whose only exit is the kernel's egress proxy, which applies site rules, adds credentials and holds cookies.
    - Tests: `browser_egress.rs` (4) drive real Chromium on a development host through the proxy: a denied site, an injected credential the model never sees, the network Rule of Two, method limits.
    - **Not run here: the browser VM itself**, because this container has no KVM. On the dev machine:
      - run `sandbox/build.sh`, which now also writes `browser-rootfs.ext4`;
      - set `[browser] rootfs` to it;
      - run a browser task and watch for the guest's forwarder and the sidecar starting.
  - **Owner's steps for live apps:**
    - register OAuth clients (Google Desktop app; Entra public client; GitHub App with device flow), per `docs/research/connectors.md`;
    - add the apps from presets in Connectors;
    - sign in;
    - review each app's tools.
  - The reference Files server test (`--ignored`) is written but not run, since npx was not allowed in this session. The Tauri loopback listener's tests pass on their own; the app was not built here (no GTK).
- **Memory and ontology (2026-10-28).** Memory checked against M1–M4:
  - Writes go through execute() with the run's taint (M1). Search-only retrieval (M2) and rebuild from the log (M4) already held.
  - Fixed: a search's taint now comes from the same lookup as its text. Facts now keep where their untrusted content came from (M3).
  - Tests: `tests/scratch/tests/memory.rs` (3 tests). On the previous kernel only the M3 source check fails.
  - New `crates/magnus/src/ontology.rs` (O1). It generates the Cedar schema and the memory_write input schema, and checks policies, entities and requests against the schema. It also checks built-in tool schemas, and contract checks against their tools (O2).
  - Tests: 6 unit tests in `ontology.rs`, including `every_term_is_used` (O4), and `tests/scratch/tests/ontology.rs` (2).
  - Stored policy layers are not re-checked. A layer written before this that does not fit the schema still evaluates as before, but accepting a proposal for it is refused until it is fixed.
- **Review fixes (2026-10-28).** A review of the chat and check-label work found five problems, all fixed with tests:
  - A later chat turn started clean although it reads the thread, so a task it started could send private data on instructions an earlier turn fetched. Turns now start with the thread's taint. `chat.rs: a_later_turn_carries_the_taint_of_the_thread` (on the old kernel the mail was sent).
  - The check resolver labelled stdin-fed interpreters (`sh < run.sh`, `python3 < t.py`), `from os import system`, getattr, `unittest.main`, posix_spawn and awk's `system()` appliance-verified. It is now an allowlist; `sandbox/test_check_tools.py` (67 cases; the old resolver fails 26). Run it with `python3 -I sandbox/test_check_tools.py`. The sandbox image must be rebuilt (`sandbox/build.sh`) for the appliance to use it.
  - Racing confirms of a chat card made several schedules, and racing answers decided an inbox item twice. Decisions now take a lock. `a_card_confirmed_twice_at_once_makes_one_schedule`, and the inbox race in the taint test (fails 2 of 3 runs without the lock).
  - A model-sent `contract.conversation` on a task not from chat was shown as "From the chat". The kernel now owns that field. `a_contract_cannot_claim_a_conversation`.
  - A schedule confirmed from a card now tells each run the card's scope. `a_confirmed_schedule_carries_its_scope`.
  - Run here without KVM: 14 tests that need a sandbox VM fail the same way with and without these changes (contract checks with `sandbox_exec`, inv1/inv2 sandbox tests, the sign-in flows). They still need a run on the dev machine.
- **Chat (2026-10-28).** The desktop app has a Chat section. A message is a run without a contract that ends with its answer; the model decides whether to answer, start a task (`create_task`), propose a schedule (`create_schedule`, confirmed on a card in the thread), or ask. Chat turns may only read; tasks from chat start at once, inherit the turn's taint, carry the conversation into their contract, appear in Tasks and post their result to the thread. Covered by `tests/scratch/tests/chat.rs` (8 tests, including restarts while a chat task runs, while it waits for approval, and before a schedule is due) and a chat step in `desktop/tests/flow.mjs`. Three mutants (read-only rule, inherited taint, creator-only card) are each caught. Deployed to the dev appliance; installers rebuilt and replaced on the `desktop-v0.1.0` pre-release (commit 22b3bf0, checksums verified after upload). Routing eval: `eval/chat/run.py` asks 24 messages (8 questions, 8 one-off research, 8 recurring) and reports how often the model's choice matched; it signs in with device code, cancels the tasks it starts and declines the cards. First live run pending the owner's sign-in.
- **Verification can no longer be talked around (2026-10-28).** The "Research, architect, and build your replacement" run reached Done by rewriting its contract to a trivial criterion. Now: criteria freeze when work starts; a later change is a `contract_change` inbox card (old and new side by side) and counts only once approved; checks may run sandbox commands and read untrusted results as data without tainting the run; a check that cannot run goes to the review card unchanged; the plan view shows no unit as active after Done. Covered by `tests/scratch/tests/contract_freeze.rs` (5 tests) and a plan check in `desktop/tests/flow.mjs`.
  - Rerun of the exact task on `grok-4.7` (`task_ae11613f1a9e4a749f288bca810b65f3`, 24 model calls, $0.55): the contract froze at version 1 with four sandbox checks (architecture doc, `python3 -m pytest -q`, the demo, the README). The agent asked to swap pytest (absent in the sandbox) for its own runner; the owner kept the contract and said to use unittest. It reached Done on version 1, all four checks verified by the kernel. **Gap found:** it passed the pytest check by writing a stand-in `pytest` module (`_pytest_shim/`, put on the user site path) that runs its unittest suite. It said so in its summary, and the 10 tests are real `unittest` tests, but a check that runs a command in an environment the agent controls can be satisfied by changing the environment. Not yet fixed; options are with the owner.
  - **Checks can no longer be satisfied by changing the environment (2026-10-28).** Each sandbox check runs in a fresh VM from the image with a throwaway copy of the workspace, and its tools must be in the image: a contract naming a workspace tool is refused when recorded. Covered by `contract_freeze.rs` (8 tests: the rerun's stand-in pytest no longer passes; a workspace-only checking tool is refused; a check leaves nothing behind). Mutants caught: checks run in the run's VM (2 tests fail), tool resolution skipped. Not tested in isolation: each guest-side protection (fresh HOME, no user site, safe path, `sitecustomize.py`) separately, since each alone stops the stand-in in that test.
  - **Checks are labelled, not refused (2026-10-28).** A workspace runner is allowed; the kernel labels each check at freeze: `appliance-verified` when the image's tools and the contract's expected values decide it, `self-checked` when it executes a workspace file (scripts, the agent's tests under the image's pytest, demos), with the reason. The labels show on the contract card and contract-change card; a finished task reads "Done: N appliance-verified, M self-checked". Refused still: bare programs the image lacks, changed lookup variables, workspace files executed directly. The instruction line naming `python3 tests/check.py` as allowed is gone. Covered by `contract_freeze.rs` (9 tests, including `a_check_executing_a_workspace_script_is_labeled_self_checked`) and the done line in `desktop/tests/flow.mjs`.
  - **Invariant tests to apply:** `docs/proposals/invariant-tests-tightened.patch` adds the freeze test to invariant 7 and tightens four tests the mutation check found weak (crash repeat with restarts during an in-flight call; the intent write alone refused; a reused one-time approval for an identical action; a rejected review returned to the agent). With it, 46 tests; the patched files pass in full, and each new or tightened test kills its mutant (M11, M14, X01, M32, X05, and the four freeze mutants). `tests/invariants/` is closed to Claude by `.claude/settings.json`, so the owner applies it: `git apply docs/proposals/invariant-tests-tightened.patch`. Afterwards delete `tests/scratch/tests/proposed_invariants.rs` and the first test in `contract_freeze.rs`, which duplicate it.
  - One unexplained failure: a full scratch run had one failing test (output not kept); nine full runs since are clean.
- **Invariant tests mutation-checked (2026-10-28).** 52 kernel mutants (56 runs): 48 killed, 8 survived; 44 of 45 tests fail on the bug they exist for, and every invariant's core mutation is killed. Two survivors are masked by a second kernel guard. Six are test gaps, for the owner to decide on: crash-repeat (no idempotency key sent; key changes across restarts), intent not recorded (only the intent write made best-effort), a once-approval reused for an identical action, a rejected review failing the run itself, and an injected policy proposal in inv4. Details and every mutant's diff: `docs/invariant-mutations.md`.
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
4. Jailer for the root-run appliance. Live mail, calendar and files: the owner registers OAuth clients, then adds the apps from presets (see Now). Boot the browser VM on the dev machine.
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
  - `python3 -I sandbox/test_check_tools.py` (the check resolver)
  - Browser tests need Chromium: `MAGNUS_CHROMIUM=<chrome>`; without browser/node_modules, `MAGNUS_NODE_PATH=<dir with playwright-core>` and `MAGNUS_PLAYWRIGHT=<playwright>` for the app flows.
  - `cargo test -p magnus-scratch` (regressions)
  - `cargo test -p magnus --test tools` (browser and desktop)
- Desktop app: see `desktop/README.md`. Build in `desktop/src-tauri` with `cargo tauri build`. Tests: `cargo test -p magnus-scratch --test desktop_flow`, and `--test desktop_app -- --ignored`.
  - The app test needs `tauri-driver`, `xvfb-run` and WebKitWebDriver. WebKitWebDriver is not installed system-wide. Get it with `apt download webkit2gtk-driver && dpkg-deb -x` and set `MAGNUS_WEBKIT_DRIVER`.
- The scratch tests start the appliance binary from `magnus-invariants`. Run `cargo build -p magnus-invariants --bins` first, or they use a stale kernel.
- Read `DECISIONS.md` before changing behavior. Do not edit `tests/invariants/` without the owner's approval.
