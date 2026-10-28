# Graph Report - magnus-claude  (2026-10-28)

## Corpus Check
- 93 files · ~91,139 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 1515 nodes · 3634 edges · 72 communities (69 shown, 3 thin omitted)
- Extraction: 96% EXTRACTED · 4% INFERRED · 0% AMBIGUOUS · INFERRED: 161 edges (avg confidence: 0.79)
- Token cost: 0 input · 0 output

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
- Duration
- never
- Kernel
- Mutex
- BTreeMap
- preload.js
- Client
- grade.py
- restore.sh
- HeaderMap
- Response
- Arc
- Speech
- default.json
- which
- Magnus desktop app

## God Nodes (most connected - your core abstractions)
1. `steps()` - 55 edges
2. `State` - 46 edges
3. `Kernel` - 36 edges
4. `Run` - 35 edges
5. `err()` - 34 edges
6. `who()` - 30 edges
7. `Adapters` - 25 edges
8. `internal()` - 24 edges
9. `App` - 24 edges
10. `Sandboxes` - 23 edges

## Surprising Connections (you probably didn't know these)
- `api()` --references--> `State`  [EXTRACTED]
  desktop/src-tauri/src/lib.rs → crates/magnus/src/state.rs
- `connect()` --references--> `State`  [EXTRACTED]
  desktop/src-tauri/src/lib.rs → crates/magnus/src/state.rs
- `download()` --references--> `State`  [EXTRACTED]
  desktop/src-tauri/src/lib.rs → crates/magnus/src/state.rs
- `fetch_bytes()` --references--> `State`  [EXTRACTED]
  desktop/src-tauri/src/lib.rs → crates/magnus/src/state.rs
- `live_start()` --references--> `State`  [EXTRACTED]
  desktop/src-tauri/src/lib.rs → crates/magnus/src/state.rs

## Import Cycles
- None detected.

## Communities (72 total, 3 thin omitted)

### Community 0 - "steps"
Cohesion: 0.06
Nodes (52): Accounts, dir(), read_json(), BTreeMap, DateTime, Mutex, Option, Path (+44 more)

### Community 1 - "mcp.rs"
Cohesion: 0.11
Nodes (32): Handler, Notify, Builder, Call, chat(), crm(), err(), FakeMcp (+24 more)

### Community 2 - "Api"
Cohesion: 0.08
Nodes (25): Method, TempDir, Api, Appliance, bin(), Connector, free_port(), Person (+17 more)

### Community 3 - "Reply"
Cohesion: 0.08
Nodes (33): content_text(), Conversation, FakeModel, Inner, messages(), Reply, Arc, HashMap (+25 more)

### Community 4 - "never"
Cohesion: 0.15
Nodes (24): complete(), contract_checked(), contract_unchecked(), eventually(), fail(), never(), Duration, F (+16 more)

### Community 5 - "inv7_verify.rs"
Cohesion: 0.36
Nodes (9): a_criterion_without_a_check_goes_to_the_person_for_review(), a_false_report_of_completion_is_caught_by_checking_the_world(), checks_may_only_read(), close_42(), closed_42_contract(), completion_reflects_the_world_at_the_time_of_checking(), crm_spec(), no_completion_and_no_action_without_a_contract() (+1 more)

### Community 6 - "TestDb"
Cohesion: 0.18
Nodes (26): Box, Bytes, base(), bearer(), catalog(), classify(), client_version(), cost() (+18 more)

### Community 7 - "FakeWeb"
Cohesion: 0.14
Nodes (17): browser_navigates_clicks_types_and_screenshots(), page_server(), root(), PathBuf, String, sandbox_desktop_takes_screenshots_and_types(), Html, FakeWeb (+9 more)

### Community 8 - "inv3_event_log.rs"
Cohesion: 0.24
Nodes (11): all_state_is_rebuilt_from_the_event_log_alone(), crash_after_a_side_effect_does_not_repeat_it(), crash_against_an_app_without_idempotency_asks_instead_of_resending(), Reply, String, Value, Vec, send() (+3 more)

### Community 9 - "inv8_layers.rs"
Cohesion: 0.23
Nodes (12): Script, a_group_layer_narrows_its_members(), a_user_layer_cannot_widen_the_organization_layer(), a_user_layer_narrows_for_that_user_only(), an_organization_forbid_wins_over_any_lower_permit(), close_42(), crm_app(), crm_script() (+4 more)

### Community 10 - "files_containing"
Cohesion: 0.48
Nodes (6): contains(), encodings(), files_containing(), Path, PathBuf, Vec

### Community 11 - "adapter-proto/src/lib.rs"
Cohesion: 0.24
Nodes (19): base(), block(), catalog(), classify_status(), complete(), handle(), main(), prices() (+11 more)

### Community 16 - "Adapters"
Cohesion: 0.13
Nodes (24): AtomicU64, AdapterError, Adapters, Arc, Child, Drop, Event, HashMap (+16 more)

### Community 17 - "Person"
Cohesion: 0.14
Nodes (23): CryptoProvider, build(), client(), describe(), fingerprint(), look(), Pinned, Arc (+15 more)

### Community 18 - "Run"
Cohesion: 0.11
Nodes (44): Answer, blocking(), consequence(), Kernel, may_respond(), may_set_policy(), render(), Arc (+36 more)

### Community 19 - ".pipeline"
Cohesion: 0.20
Nodes (25): Action, CallFailure, canonical(), computer_command(), err_text(), Facts, Gate, has_rights() (+17 more)

### Community 20 - "Kernel"
Cohesion: 0.28
Nodes (12): AppTool, CallError, CallOutput, definition_hash(), Mcp, Client, Option, Result (+4 more)

### Community 21 - "api.rs"
Cohesion: 0.14
Nodes (76): ConnectInfo, adapters(), add_schedule(), async_stream(), bearer(), cancel(), catalog(), conformance() (+68 more)

### Community 22 - "6. TheAgentCompany (TAC)"
Cohesion: 0.06
Nodes (34): 1. Meta Model API ("Muse" family), 2. xAI Grok: OAuth / SuperGrok sign-in, 3. OpenAI Codex with ChatGPT sign-in (codex-rs, commit eec8bde1), 4. GLM (Z.ai international, and BigModel in China), 5. Anthropic Messages API, 6. TheAgentCompany (TAC), API-key path (api.x.ai), Base URLs (+26 more)

### Community 23 - "Ev"
Cohesion: 0.08
Nodes (38): Config, Capture, Kernel, new_id(), Arc, Browser, Event, HashMap (+30 more)

### Community 24 - "Sandboxes"
Cohesion: 0.16
Nodes (18): request(), Arc, Cfg, Child, Drop, Duration, HashMap, Mutex (+10 more)

### Community 25 - "guest/src/main.rs"
Cohesion: 0.06
Nodes (42): Addr, c_ulong, Browser, Cfg, Child, Mutex, Option, Path (+34 more)

### Community 26 - "Web"
Cohesion: 0.22
Nodes (10): private_ip(), Client, Option, Response, Result, SocketAddr, String, Web (+2 more)

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
Cohesion: 0.09
Nodes (30): call(), Check, fake_provider(), Outcome, params(), Report, Event, Option (+22 more)

### Community 31 - "inv6_policy.rs"
Cohesion: 0.11
Nodes (18): complete(), messages(), Client, Result, String, Value, Vec, text_of() (+10 more)

### Community 35 - "scratch/src/lib.rs"
Cohesion: 0.16
Nodes (32): AbortHandle, AppHandle, allowed(), api(), App, body_of(), bytes(), connect() (+24 more)

### Community 36 - "new_id"
Cohesion: 0.15
Nodes (13): BufReader, Call, Into, Path, PathBuf, Result, Self, serve() (+5 more)

### Community 37 - "Vault"
Cohesion: 0.39
Nodes (13): Block, CompleteParams, Envelope, Event, Message, ModelInfo, Request, Role (+5 more)

### Community 38 - "auth.rs"
Cohesion: 0.42
Nodes (12): begin(), claims(), complete(), credential_from(), oauth_bearer(), Client, Option, Result (+4 more)

### Community 39 - "Progress"
Cohesion: 0.33
Nodes (5): Done, How to resume, Next, Now (2026-10-27), Progress

### Community 40 - "transcribe"
Cohesion: 0.50
Nodes (4): Path, Result, String, transcribe()

### Community 43 - "Sender"
Cohesion: 0.30
Nodes (11): a_fetch_cannot_carry_private_data_out_after_untrusted_input(), appliance(), content_claiming_authority_changes_nothing(), exfiltrate(), injected_page_cannot_exfiltrate_private_data(), private_data_sent_to_a_wider_audience_asks(), private_data_with_an_external_action_to_a_private_audience_is_allowed(), Reply (+3 more)

### Community 44 - "HeaderMap"
Cohesion: 0.27
Nodes (7): Api, main(), Run the plain-language suite against a Magnus appliance and grade it.      pytho, The appliance's plain-HTTP address from its config, reached on loopback., run_task(), token_from(), url_from()

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
Cohesion: 0.25
Nodes (14): Output, a_browser_session_needs_the_client_header_to_change_anything(), a_model_change_is_a_version_change(), repeated_wrong_passwords_lock_the_name(), Api, Appliance, StatusCode, Value (+6 more)

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
Nodes (19): steps(), a_changed_tool_definition_is_disabled_and_hidden(), action_is_not_executed_when_its_intent_cannot_be_recorded(), host_addresses(), missing_credential_denies_the_action(), prohibited_tools_never_run_even_if_policy_permits_everything(), String, Vec (+11 more)

### Community 60 - "never"
Cohesion: 0.12
Nodes (23): decide(), entities(), eval(), layers_for(), Outcome, BTreeMap, Option, Person (+15 more)

### Community 61 - "Kernel"
Cohesion: 0.25
Nodes (9): Config, BTreeMap, Mutex, Option, PathBuf, Result, String, Store (+1 more)

### Community 64 - "Mutex"
Cohesion: 0.12
Nodes (4): api(), ApiError, errorOf(), webBridge

### Community 65 - "BTreeMap"
Cohesion: 0.22
Nodes (4): FAILURE, n4(), sourcesBar(), SRC_COLORS

### Community 66 - "preload.js"
Cohesion: 0.27
Nodes (15): button(), call(), click(), css(), find(), run(), s(), seen() (+7 more)

### Community 69 - "Client"
Cohesion: 0.10
Nodes (28): Budget, ContextRef, Ev, ExperimentTag, DateTime, Option, String, Utc (+20 more)

### Community 73 - "grade.py"
Cohesion: 0.22
Nodes (3): Graders for the plain-language suite. They check the world (the files the worker, Values of a workbook after LibreOffice recalculates its formulas., recalc()

### Community 76 - "restore.sh"
Cohesion: 0.70
Nodes (4): print_header(), restoreData(), restoreSingleVolume(), restore.sh script

### Community 79 - "HeaderMap"
Cohesion: 0.33
Nodes (9): a_one_action_approval_covers_one_action(), mail_app(), owner_policy_grants_autonomy(), Appliance, Reply, send(), the_agent_proposes_and_only_the_owner_grants(), the_approval_channel_needs_a_person() (+1 more)

### Community 80 - "Response"
Cohesion: 0.29
Nodes (6): [base, person, password, apiKey, task], { chromium }, errors, root, shot(), step()

### Community 99 - "Arc"
Cohesion: 0.13
Nodes (43): status_str(), add_trial(), choose(), clip(), config_key(), experiment(), experiments(), ExpQuery (+35 more)

### Community 116 - "Speech"
Cohesion: 0.33
Nodes (7): Child, Mutex, Option, Result, String, Speech, start()

### Community 117 - "default.json"
Cohesion: 0.22
Nodes (8): description, identifier, permissions, $schema, windows, core:default, main, notification:default

### Community 118 - "which"
Cohesion: 0.18
Nodes (12): canary(), org_policy(), permit_execute(), String, a_denial_goes_back_to_the_model_which_decides_what_next(), people_set_policy_only_within_their_role(), String, uuid() (+4 more)

### Community 122 - "Magnus desktop app"
Cohesion: 0.29
Nodes (6): Before the first sign-in (owner, on the appliance), Build, Layout, Magnus desktop app, Tests, The recorded run

## Knowledge Gaps
- **120 isolated node(s):** `name`, `private`, `description`, `main`, `playwright-core` (+115 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **3 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `State` connect `Run` to `mcp.rs`, `Reply`, `scratch/src/lib.rs`, `Arc`, `FakeWeb`, `Ev`?**
  _High betweenness centrality (0.240) - this node is a cross-community bridge._
- **Why does `messages()` connect `Reply` to `Run`?**
  _High betweenness centrality (0.129) - this node is a cross-community bridge._
- **Why does `Kernel` connect `Ev` to `steps`, `Client`, `Adapters`, `Run`, `Kernel`, `Sandboxes`, `inv1_boundary.rs`?**
  _High betweenness centrality (0.117) - this node is a cross-community bridge._
- **Are the 49 inferred relationships involving `steps()` (e.g. with `a_changed_tool_definition_is_disabled_and_hidden()` and `missing_credential_denies_the_action()`) actually correct?**
  _`steps()` has 49 INFERRED edges - model-reasoned connections that need verification._
- **What connects `name`, `private`, `description` to the rest of the system?**
  _120 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `steps` be split into smaller, more focused modules?**
  _Cohesion score 0.05627545353572751 - nodes in this community are weakly interconnected._
- **Should `mcp.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.11139455782312925 - nodes in this community are weakly interconnected._