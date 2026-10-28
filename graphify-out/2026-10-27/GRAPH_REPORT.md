# Graph Report - magnus-claude  (2026-10-27)

## Corpus Check
- 92 files · ~90,020 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 1562 nodes · 3580 edges · 122 communities (66 shown, 56 thin omitted)
- Extraction: 97% EXTRACTED · 3% INFERRED · 0% AMBIGUOUS · INFERRED: 97 edges (avg confidence: 0.79)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `5c1c623a`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- steps
- mcp.rs
- Api
- Reply
- never
- inv7_verify.rs
- TestDb
- FakeWeb
- inv3_event_log.rs
- inv8_layers.rs
- files_containing
- adapter-proto/src/lib.rs
- Adapters
- Person
- Run
- .pipeline
- Kernel
- api.rs
- 6. TheAgentCompany (TAC)
- Ev
- Sandboxes
- guest/src/main.rs
- Web
- Decisions
- spawn
- cli.rs
- inv1_boundary.rs
- inv6_policy.rs
- build.sh
- scratch/src/lib.rs
- new_id
- Vault
- auth.rs
- Progress
- transcribe
- invariant-tests-v2.md
- Sender
- HeaderMap
- Response
- complete
- sidecar.js
- package.json
- review.rs
- tests/tools.rs
- HeaderMap
- Response
- Stream
- Web
- ErrorClass
- Usage
- Vault
- Web
- Duration
- never
- Kernel
- Path
- Browser
- Mutex
- BTreeMap
- preload.js
- Into
- Event
- Client
- Event
- ToolSpec
- Log
- grade.py
- run.py
- make_fixtures.py
- restore.sh
- Duration
- complete
- HeaderMap
- Response
- Stream
- ToolSpec
- Usage
- Vec
- Stream
- Person
- Into
- K
- Mutex
- Run
- Status
- Task
- Vault
- Web
- Result
- T
- Utc
- Arc
- BTreeSet
- HeaderMap
- Json
- K
- Kernel
- Response
- Vec
- K
- Kernel
- Path
- Response
- SocketAddr
- AppTool
- Self
- ToolSpec
- Vec
- Speech
- default.json
- which
- String

## God Nodes (most connected - your core abstractions)
1. `steps()` - 42 edges
2. `Kernel` - 36 edges
3. `err()` - 34 edges
4. `who()` - 30 edges
5. `State` - 28 edges
6. `Config` - 26 edges
7. `App` - 24 edges
8. `Person` - 24 edges
9. `Run` - 23 edges
10. `Adapters` - 23 edges

## Surprising Connections (you probably didn't know these)
- `crash_after_a_side_effect_does_not_repeat_it()` --calls--> `steps()`  [INFERRED]
  tests/invariants/tests/inv3_event_log.rs → tests/invariants/src/model.rs
- `the_kernel_cannot_rewrite_the_log()` --calls--> `steps()`  [INFERRED]
  tests/invariants/tests/inv3_event_log.rs → tests/invariants/src/model.rs
- `a_criterion_without_a_check_goes_to_the_person_for_review()` --calls--> `steps()`  [INFERRED]
  tests/invariants/tests/inv7_verify.rs → tests/invariants/src/model.rs
- `a_user_layer_cannot_widen_the_organization_layer()` --calls--> `steps()`  [INFERRED]
  tests/invariants/tests/inv8_layers.rs → tests/invariants/src/model.rs
- `the_agent_never_exceeds_the_persons_own_access()` --calls--> `steps()`  [INFERRED]
  tests/invariants/tests/inv8_layers.rs → tests/invariants/src/model.rs

## Import Cycles
- 1-file cycle: `crates/magnus/src/run/mod.rs -> crates/magnus/src/run/mod.rs`

## Communities (122 total, 56 thin omitted)

### Community 0 - "steps"
Cohesion: 0.09
Nodes (33): Adapter, Appliance, Browser, Budget, Config, Connector, Database, header() (+25 more)

### Community 1 - "mcp.rs"
Cohesion: 0.11
Nodes (33): Handler, Notify, Builder, Call, chat(), crm(), err(), FakeMcp (+25 more)

### Community 2 - "Api"
Cohesion: 0.09
Nodes (21): Method, StatusCode, TempDir, Api, Appliance, bin(), Connector, free_port() (+13 more)

### Community 3 - "Reply"
Cohesion: 0.15
Nodes (19): content_text(), Conversation, FakeModel, Inner, messages(), Arc, HashMap, HeaderMap (+11 more)

### Community 4 - "never"
Cohesion: 0.18
Nodes (19): Duration, T, contract_unchecked(), eventually(), fail(), never(), F, F (+11 more)

### Community 5 - "inv7_verify.rs"
Cohesion: 0.21
Nodes (15): complete(), contract_checked(), Value, Reply, repeated_crashes_never_repeat_an_effect(), send(), exfiltrate(), a_criterion_without_a_check_goes_to_the_person_for_review() (+7 more)

### Community 6 - "TestDb"
Cohesion: 0.15
Nodes (31): Box, Bytes, base(), bearer(), catalog(), classify(), client_version(), cost() (+23 more)

### Community 7 - "FakeWeb"
Cohesion: 0.19
Nodes (12): Html, FakeWeb, Inner, page(), Arc, HashMap, Mutex, Self (+4 more)

### Community 8 - "inv3_event_log.rs"
Cohesion: 0.36
Nodes (8): all_state_is_rebuilt_from_the_event_log_alone(), crash_after_a_side_effect_does_not_repeat_it(), String, Value, Vec, snapshot_inbox(), snapshot_tasks(), the_kernel_cannot_rewrite_the_log()

### Community 9 - "inv8_layers.rs"
Cohesion: 0.23
Nodes (12): Script, org_policy(), a_denial_goes_back_to_the_model_which_decides_what_next(), a_group_layer_narrows_its_members(), a_user_layer_cannot_widen_the_organization_layer(), a_user_layer_narrows_for_that_user_only(), an_organization_forbid_wins_over_any_lower_permit(), close_42() (+4 more)

### Community 10 - "files_containing"
Cohesion: 0.53
Nodes (5): contains(), encodings(), files_containing(), PathBuf, Vec

### Community 11 - "adapter-proto/src/lib.rs"
Cohesion: 0.09
Nodes (47): BufReader, base(), block(), catalog(), classify_status(), complete(), handle(), main() (+39 more)

### Community 16 - "Adapters"
Cohesion: 0.11
Nodes (28): AtomicU64, AdapterError, Adapters, Arc, Block, Child, Drop, ErrorClass (+20 more)

### Community 17 - "Person"
Cohesion: 0.07
Nodes (40): Addr, Arc, CertificateDer, router(), fingerprint(), hostname(), Listener, load() (+32 more)

### Community 18 - "Run"
Cohesion: 0.11
Nodes (42): Answer, blocking(), consequence(), Kernel, may_respond(), may_set_policy(), render(), Arc (+34 more)

### Community 19 - ".pipeline"
Cohesion: 0.19
Nodes (25): Action, CallFailure, canonical(), computer_command(), err_text(), Facts, Gate, has_rights() (+17 more)

### Community 20 - "Kernel"
Cohesion: 0.28
Nodes (12): AppTool, CallError, CallOutput, definition_hash(), Mcp, Client, Option, Result (+4 more)

### Community 21 - "api.rs"
Cohesion: 0.08
Nodes (114): BTreeSet, Config, ConnectInfo, adapters(), add_schedule(), async_stream(), bearer(), cancel() (+106 more)

### Community 22 - "6. TheAgentCompany (TAC)"
Cohesion: 0.06
Nodes (34): 1. Meta Model API ("Muse" family), 2. xAI Grok: OAuth / SuperGrok sign-in, 3. OpenAI Codex with ChatGPT sign-in (codex-rs, commit eec8bde1), 4. GLM (Z.ai international, and BigModel in China), 5. Anthropic Messages API, 6. TheAgentCompany (TAC), API-key path (api.x.ai), Base URLs (+26 more)

### Community 23 - "Ev"
Cohesion: 0.06
Nodes (50): Adapters, Client, ContextRef, Ev, ExperimentTag, Block, DateTime, Option (+42 more)

### Community 24 - "Sandboxes"
Cohesion: 0.06
Nodes (43): Cfg, complete(), messages(), Block, Client, CompleteParams, Result, Sink (+35 more)

### Community 25 - "guest/src/main.rs"
Cohesion: 0.19
Nodes (16): c_ulong, RawFd, exec(), handle(), init(), listen(), loopback_up(), main() (+8 more)

### Community 26 - "Web"
Cohesion: 0.24
Nodes (9): private_ip(), Client, Option, Response, Result, String, Web, IpAddr (+1 more)

### Community 27 - "Decisions"
Cohesion: 0.18
Nodes (10): Boundary and policy, Decisions, Evaluation, Event log and state, Identity, Models, Process, Runs and verification (+2 more)

### Community 28 - "spawn"
Cohesion: 0.25
Nodes (8): Command, Child, Mutex, Result, Sender, spawn(), spawner(), Job

### Community 29 - "cli.rs"
Cohesion: 0.56
Nodes (10): arg(), config(), conformance(), main(), migrate(), password(), Option, Result (+2 more)

### Community 30 - "inv1_boundary.rs"
Cohesion: 0.14
Nodes (22): call(), Check, fake_provider(), Outcome, params(), Report, CompleteParams, ErrorClass (+14 more)

### Community 31 - "inv6_policy.rs"
Cohesion: 0.16
Nodes (20): AppTool, Connector, tool_specs(), clip(), instructions_version(), Kernel, Arc, Into (+12 more)

### Community 35 - "scratch/src/lib.rs"
Cohesion: 0.18
Nodes (32): AbortHandle, AppHandle, allowed(), api(), App, body_of(), bytes(), connect() (+24 more)

### Community 37 - "Vault"
Cohesion: 0.15
Nodes (15): redact(), BTreeMap, Mutex, Option, Path, PathBuf, Result, String (+7 more)

### Community 38 - "auth.rs"
Cohesion: 0.35
Nodes (14): begin(), claims(), complete(), credential_from(), oauth_bearer(), Client, ErrorClass, Option (+6 more)

### Community 39 - "Progress"
Cohesion: 0.33
Nodes (5): Done, How to resume, Next, Now (2026-10-27), Progress

### Community 40 - "transcribe"
Cohesion: 0.50
Nodes (4): Path, Result, String, transcribe()

### Community 44 - "HeaderMap"
Cohesion: 0.13
Nodes (24): Accounts, dir(), read_json(), BTreeMap, Option, Path, String, Session (+16 more)

### Community 45 - "Response"
Cohesion: 0.26
Nodes (8): connect(), Client, Option, Self, String, Vec, server_url(), TestDb

### Community 46 - "complete"
Cohesion: 0.10
Nodes (19): 1.1 How upstream starts it (read in full before running), 1.2 What we changed, and why, 1.3 Commands (exactly what ran on 2026-10-27), 1.4 Ports the stack occupies (all were free on this host), 1.5 Known stack hazards (the runner must handle these), 1. Server stack, 2. One TAC task, end to end (upstream semantics), 3. The three tasks studied (+11 more)

### Community 47 - "sidecar.js"
Cohesion: 0.24
Nodes (9): { chromium }, fs, handle(), locate(), net, page(), runs, snapshot() (+1 more)

### Community 48 - "package.json"
Cohesion: 0.25
Nodes (7): dependencies, playwright-core, description, main, name, private, playwright-core

### Community 49 - "review.rs"
Cohesion: 0.09
Nodes (30): Api, Appliance, FakeModel, FakeWeb, Output, Reply, chromium(), Flow (+22 more)

### Community 50 - "tests/tools.rs"
Cohesion: 0.75
Nodes (7): numbers(), Result, String, Value, strings(), text(), validate()

### Community 51 - "HeaderMap"
Cohesion: 0.09
Nodes (43): api(), App, AUDIENCE, background, boot(), buttons(), card(), chip() (+35 more)

### Community 53 - "Stream"
Cohesion: 0.06
Nodes (33): app, security, windows, withGlobalTauri, build, frontendDist, bundle, active (+25 more)

### Community 59 - "Duration"
Cohesion: 0.13
Nodes (25): canary(), permit_execute(), String, steps(), a_changed_tool_definition_is_disabled_and_hidden(), host_addresses(), missing_credential_denies_the_action(), prohibited_tools_never_run_even_if_policy_permits_everything() (+17 more)

### Community 60 - "never"
Cohesion: 0.12
Nodes (23): decide(), entities(), eval(), layers_for(), Outcome, BTreeMap, Option, Result (+15 more)

### Community 61 - "Kernel"
Cohesion: 0.25
Nodes (9): BTreeMap, Config, Mutex, Option, PathBuf, Result, String, Store (+1 more)

### Community 64 - "Mutex"
Cohesion: 0.12
Nodes (4): api(), ApiError, errorOf(), webBridge

### Community 65 - "BTreeMap"
Cohesion: 0.22
Nodes (4): FAILURE, n4(), sourcesBar(), SRC_COLORS

### Community 66 - "preload.js"
Cohesion: 0.27
Nodes (15): button(), call(), click(), css(), find(), run(), s(), seen() (+7 more)

### Community 73 - "grade.py"
Cohesion: 0.22
Nodes (3): Graders for the plain-language suite. They check the world (the files the worker, Values of a workbook after LibreOffice recalculates its formulas., recalc()

### Community 76 - "restore.sh"
Cohesion: 0.70
Nodes (4): print_header(), restoreData(), restoreSingleVolume(), restore.sh script

### Community 79 - "HeaderMap"
Cohesion: 0.46
Nodes (7): a_one_action_approval_covers_one_action(), mail_app(), owner_policy_grants_autonomy(), send(), the_agent_proposes_and_only_the_owner_grants(), the_approval_channel_needs_a_person(), without_owner_policy_an_irreversible_action_waits_for_approval()

### Community 80 - "Response"
Cohesion: 0.29
Nodes (6): [base, person, password, apiKey, task], { chromium }, errors, root, shot(), step()

### Community 81 - "Stream"
Cohesion: 0.43
Nodes (6): browser_navigates_clicks_types_and_screenshots(), page_server(), root(), PathBuf, String, sandbox_desktop_takes_screenshots_and_types()

### Community 116 - "Speech"
Cohesion: 0.33
Nodes (7): Child, Mutex, Option, Result, String, Speech, start()

### Community 117 - "default.json"
Cohesion: 0.22
Nodes (8): description, identifier, permissions, $schema, windows, core:default, main, notification:default

### Community 118 - "which"
Cohesion: 0.60
Nodes (4): Option, PathBuf, the_linux_app_pins_signs_in_approves_and_shows_evidence(), which()

## Knowledge Gaps
- **114 isolated node(s):** `$schema`, `identifier`, `description`, `main`, `core:default` (+109 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **56 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Config` connect `steps` to `Adapters`, `.pipeline`, `Ev`, `cli.rs`, `inv1_boundary.rs`?**
  _High betweenness centrality (0.076) - this node is a cross-community bridge._
- **Why does `err()` connect `api.rs` to `Api`?**
  _High betweenness centrality (0.057) - this node is a cross-community bridge._
- **Why does `Spec` connect `Api` to `steps`, `inv7_verify.rs`?**
  _High betweenness centrality (0.046) - this node is a cross-community bridge._
- **Are the 36 inferred relationships involving `steps()` (e.g. with `a_changed_tool_definition_is_disabled_and_hidden()` and `missing_credential_denies_the_action()`) actually correct?**
  _`steps()` has 36 INFERRED edges - model-reasoned connections that need verification._
- **What connects `$schema`, `identifier`, `description` to the rest of the system?**
  _114 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `steps` be split into smaller, more focused modules?**
  _Cohesion score 0.08776595744680851 - nodes in this community are weakly interconnected._
- **Should `mcp.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.10775510204081633 - nodes in this community are weakly interconnected._