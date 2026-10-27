# Graph Report - magnus-claude  (2026-10-27)

## Corpus Check
- 72 files · ~60,447 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 1114 nodes · 2611 edges · 76 communities (51 shown, 25 thin omitted)
- Extraction: 96% EXTRACTED · 4% INFERRED · 0% AMBIGUOUS · INFERRED: 95 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `c54d71c3`
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

## God Nodes (most connected - your core abstractions)
1. `steps()` - 42 edges
2. `Kernel` - 32 edges
3. `err()` - 28 edges
4. `Run` - 28 edges
5. `who()` - 27 edges
6. `State` - 23 edges
7. `Adapters` - 23 edges
8. `Ev` - 22 edges
9. `Sandboxes` - 21 edges
10. `Config` - 20 edges

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
- None detected.

## Communities (76 total, 25 thin omitted)

### Community 0 - "steps"
Cohesion: 0.12
Nodes (26): canary(), permit_execute(), String, steps(), a_changed_tool_definition_is_disabled_and_hidden(), host_addresses(), missing_credential_denies_the_action(), prohibited_tools_never_run_even_if_policy_permits_everything() (+18 more)

### Community 1 - "mcp.rs"
Cohesion: 0.11
Nodes (32): Handler, Notify, Builder, Call, chat(), crm(), err(), FakeMcp (+24 more)

### Community 2 - "Api"
Cohesion: 0.09
Nodes (21): Method, StatusCode, TempDir, Api, Appliance, bin(), Connector, free_port() (+13 more)

### Community 3 - "Reply"
Cohesion: 0.15
Nodes (21): content_text(), Conversation, FakeModel, Inner, messages(), Reply, Arc, HashMap (+13 more)

### Community 4 - "never"
Cohesion: 0.17
Nodes (19): Duration, complete(), contract_checked(), contract_unchecked(), fail(), never(), Value, F (+11 more)

### Community 5 - "inv7_verify.rs"
Cohesion: 0.39
Nodes (8): a_criterion_without_a_check_goes_to_the_person_for_review(), a_false_report_of_completion_is_caught_by_checking_the_world(), checks_may_only_read(), close_42(), closed_42_contract(), completion_reflects_the_world_at_the_time_of_checking(), crm_spec(), no_completion_and_no_action_without_a_contract()

### Community 6 - "TestDb"
Cohesion: 0.14
Nodes (30): Box, Bytes, base(), bearer(), catalog(), classify(), cost(), finish() (+22 more)

### Community 7 - "FakeWeb"
Cohesion: 0.19
Nodes (12): Html, FakeWeb, Inner, page(), Arc, HashMap, Mutex, Self (+4 more)

### Community 8 - "inv3_event_log.rs"
Cohesion: 0.27
Nodes (10): all_state_is_rebuilt_from_the_event_log_alone(), crash_after_a_side_effect_does_not_repeat_it(), crash_against_an_app_without_idempotency_asks_instead_of_resending(), String, Value, Vec, send(), snapshot_inbox() (+2 more)

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
Cohesion: 0.06
Nodes (50): Adapter, Appliance, Browser, Budget, Config, Connector, Database, header() (+42 more)

### Community 18 - "Run"
Cohesion: 0.11
Nodes (38): BTreeMap, BTreeSet, status_str(), Answer, Kernel, may_respond(), may_set_policy(), render() (+30 more)

### Community 19 - ".pipeline"
Cohesion: 0.18
Nodes (26): Connector, Action, CallFailure, canonical(), computer_command(), err_text(), Facts, Gate (+18 more)

### Community 20 - "Kernel"
Cohesion: 0.28
Nodes (12): AppTool, CallError, CallOutput, definition_hash(), Mcp, Client, Option, Result (+4 more)

### Community 21 - "api.rs"
Cohesion: 0.17
Nodes (58): adapters(), add_schedule(), async_stream(), cancel(), catalog(), conformance(), err(), find() (+50 more)

### Community 22 - "6. TheAgentCompany (TAC)"
Cohesion: 0.06
Nodes (34): 1. Meta Model API ("Muse" family), 2. xAI Grok: OAuth / SuperGrok sign-in, 3. OpenAI Codex with ChatGPT sign-in (codex-rs, commit eec8bde1), 4. GLM (Z.ai international, and BigModel in China), 5. Anthropic Messages API, 6. TheAgentCompany (TAC), API-key path (api.x.ai), Base URLs (+26 more)

### Community 23 - "Ev"
Cohesion: 0.05
Nodes (53): Adapters, AppTool, Browser, Budget, Client, Config, ContextRef, Ev (+45 more)

### Community 24 - "Sandboxes"
Cohesion: 0.08
Nodes (33): Cfg, Child, complete(), input(), Client, CompleteParams, Result, Sink (+25 more)

### Community 25 - "guest/src/main.rs"
Cohesion: 0.19
Nodes (16): c_ulong, RawFd, exec(), handle(), init(), listen(), loopback_up(), main() (+8 more)

### Community 26 - "Web"
Cohesion: 0.22
Nodes (10): private_ip(), Client, Option, Response, Result, String, Web, IpAddr (+2 more)

### Community 27 - "Decisions"
Cohesion: 0.18
Nodes (10): Boundary and policy, Decisions, Evaluation, Event log and state, Identity, Models, Process, Runs and verification (+2 more)

### Community 28 - "spawn"
Cohesion: 0.25
Nodes (8): Command, Child, Mutex, Result, Sender, spawn(), spawner(), Job

### Community 29 - "cli.rs"
Cohesion: 0.56
Nodes (9): arg(), config(), conformance(), main(), migrate(), Option, Result, String (+1 more)

### Community 30 - "inv1_boundary.rs"
Cohesion: 0.14
Nodes (22): call(), Check, fake_provider(), Outcome, params(), Report, CompleteParams, ErrorClass (+14 more)

### Community 31 - "inv6_policy.rs"
Cohesion: 0.17
Nodes (17): new_id(), clip(), instructions_version(), Kernel, AppTool, Arc, Into, Option (+9 more)

### Community 37 - "Vault"
Cohesion: 0.15
Nodes (15): redact(), BTreeMap, Mutex, Option, Path, PathBuf, Result, String (+7 more)

### Community 38 - "auth.rs"
Cohesion: 0.33
Nodes (15): b64url(), begin(), claims(), complete(), credential_from(), oauth_bearer(), random(), Client (+7 more)

### Community 39 - "Progress"
Cohesion: 0.33
Nodes (5): Done, How to resume, Next, Now, Progress

### Community 40 - "transcribe"
Cohesion: 0.50
Nodes (4): Path, Result, String, transcribe()

### Community 45 - "Response"
Cohesion: 0.26
Nodes (8): connect(), Client, Option, Self, String, Vec, server_url(), TestDb

### Community 46 - "complete"
Cohesion: 0.20
Nodes (11): complete(), messages(), Block, Client, CompleteParams, Result, Sink, String (+3 more)

### Community 47 - "sidecar.js"
Cohesion: 0.24
Nodes (9): { chromium }, fs, handle(), locate(), net, page(), runs, snapshot() (+1 more)

### Community 48 - "package.json"
Cohesion: 0.25
Nodes (7): dependencies, playwright-core, description, main, name, private, playwright-core

### Community 50 - "tests/tools.rs"
Cohesion: 0.43
Nodes (6): browser_navigates_clicks_types_and_screenshots(), page_server(), root(), PathBuf, String, sandbox_desktop_takes_screenshots_and_types()

### Community 59 - "Duration"
Cohesion: 0.31
Nodes (10): T, eventually(), F, a_one_action_approval_covers_one_action(), mail_app(), owner_policy_grants_autonomy(), send(), the_agent_proposes_and_only_the_owner_grants() (+2 more)

### Community 60 - "never"
Cohesion: 0.18
Nodes (10): description, devDependencies, electron, main, name, private, scripts, start (+2 more)

### Community 61 - "Kernel"
Cohesion: 0.22
Nodes (5): { app, BrowserWindow, Notification, ipcMain, shell }, configPath, fs, http, path

### Community 73 - "grade.py"
Cohesion: 0.22
Nodes (3): Graders for the plain-language suite. They check the world (the files the worker, Values of a workbook after LibreOffice recalculates its formulas., recalc()

### Community 74 - "run.py"
Cohesion: 0.33
Nodes (5): Api, main(), Run the plain-language suite against a Magnus appliance and grade it.      pytho, run_task(), token_from()

## Knowledge Gaps
- **66 isolated node(s):** `Process`, `Event log and state`, `Boundary and policy`, `Runs and verification`, `Identity` (+61 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **25 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `State` connect `Run` to `.pipeline`, `api.rs`, `Ev`?**
  _High betweenness centrality (0.092) - this node is a cross-community bridge._
- **Why does `eval()` connect `Person` to `adapter-proto/src/lib.rs`?**
  _High betweenness centrality (0.091) - this node is a cross-community bridge._
- **Why does `Request` connect `adapter-proto/src/lib.rs` to `Person`?**
  _High betweenness centrality (0.090) - this node is a cross-community bridge._
- **Are the 36 inferred relationships involving `steps()` (e.g. with `a_changed_tool_definition_is_disabled_and_hidden()` and `missing_credential_denies_the_action()`) actually correct?**
  _`steps()` has 36 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Process`, `Event log and state`, `Boundary and policy` to the rest of the system?**
  _66 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `steps` be split into smaller, more focused modules?**
  _Cohesion score 0.12315270935960591 - nodes in this community are weakly interconnected._
- **Should `mcp.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.11170212765957446 - nodes in this community are weakly interconnected._