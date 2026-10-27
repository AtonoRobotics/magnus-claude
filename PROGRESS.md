# Progress

## Now

The invariant tests are written and compile (`tests/invariants/`, 45 tests over 8 files). They fail because the kernel is not built yet: `magnus migrate` exits "not implemented".

## Done

- Workspace: `crates/magnus` (kernel, placeholder), `crates/adapter-anthropic` (placeholder), `tests/invariants` (harness and tests).
- Invariant harness: fake Anthropic API (scripted, SSE and JSON), fake MCP apps (mail, crm, chat, notes, finance) with idempotency, holds and rug pulls, fake website, per-test Postgres database, appliance process control, leak scanner.

## Next

1. Kernel core: event log schema and migrate, config, HTTP API skeleton, vault.
2. Boundary: the eight stages with Cedar policy layers, taint and audience rules, approvals.
3. Run loop over the adapter contract; Anthropic adapter.
4. Firecracker sandbox (guest image build in `sandbox/`).
5. Make the invariant tests pass, then: the other adapters with conformance suites, tools (web, browser, documents, computer use, memory, schedules), desktop app, TheAgentCompany runner.

## How to resume

- Toolchain: `export PATH=$HOME/.cargo/bin:$PATH` (rust-toolchain.toml pins 1.99.1).
- Postgres: `docker start magnus-claude-pg` (127.0.0.1:54340, password `magnus_dev`).
- Run the invariant tests: `cargo test -p magnus-invariants`.
- Read `DECISIONS.md` before changing behavior. Do not edit `tests/invariants/` without the owner's approval.
