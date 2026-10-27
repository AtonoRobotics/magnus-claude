# Graph Report - magnus-claude  (2026-10-27)

## Corpus Check
- 52 files · ~37,863 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 817 nodes · 2048 edges · 36 communities (35 shown, 1 thin omitted)
- Extraction: 95% EXTRACTED · 5% INFERRED · 0% AMBIGUOUS · INFERRED: 93 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `5276392a`
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

## God Nodes (most connected - your core abstractions)
1. `steps()` - 42 edges
2. `Kernel` - 29 edges
3. `Run` - 28 edges
4. `Person` - 26 edges
5. `Adapters` - 24 edges
6. `Config` - 23 edges
7. `err()` - 21 edges
8. `Sandboxes` - 21 edges
9. `who()` - 20 edges
10. `Ev` - 20 edges

## Surprising Connections (you probably didn't know these)
- `a_changed_tool_definition_is_disabled_and_hidden()` --calls--> `steps()`  [INFERRED]
  tests/invariants/tests/inv1_boundary.rs → tests/invariants/src/model.rs
- `missing_credential_denies_the_action()` --calls--> `steps()`  [INFERRED]
  tests/invariants/tests/inv1_boundary.rs → tests/invariants/src/model.rs
- `prohibited_tools_never_run_even_if_policy_permits_everything()` --calls--> `steps()`  [INFERRED]
  tests/invariants/tests/inv1_boundary.rs → tests/invariants/src/model.rs
- `tools_the_owner_has_not_classified_cannot_run()` --calls--> `steps()`  [INFERRED]
  tests/invariants/tests/inv1_boundary.rs → tests/invariants/src/model.rs
- `crash_after_a_side_effect_does_not_repeat_it()` --calls--> `steps()`  [INFERRED]
  tests/invariants/tests/inv3_event_log.rs → tests/invariants/src/model.rs

## Import Cycles
- None detected.

## Communities (36 total, 1 thin omitted)

### Community 0 - "steps"
Cohesion: 0.17
Nodes (20): canary(), org_policy(), permit_execute(), String, steps(), credentials_are_used_but_never_seen(), secrets_can_be_written_but_not_read_back(), a_fetch_cannot_carry_private_data_out_after_untrusted_input() (+12 more)

### Community 1 - "mcp.rs"
Cohesion: 0.11
Nodes (32): Handler, Notify, Builder, Call, chat(), crm(), err(), FakeMcp (+24 more)

### Community 2 - "Api"
Cohesion: 0.09
Nodes (23): BTreeMap, Child, Method, StatusCode, TempDir, Api, Appliance, bin() (+15 more)

### Community 3 - "Reply"
Cohesion: 0.15
Nodes (21): content_text(), Conversation, FakeModel, Inner, messages(), Reply, Arc, HashMap (+13 more)

### Community 4 - "never"
Cohesion: 0.18
Nodes (19): Duration, contract_unchecked(), eventually(), fail(), never(), F, F, Self (+11 more)

### Community 5 - "inv7_verify.rs"
Cohesion: 0.24
Nodes (12): complete(), contract_checked(), Value, repeated_crashes_never_repeat_an_effect(), a_criterion_without_a_check_goes_to_the_person_for_review(), a_false_report_of_completion_is_caught_by_checking_the_world(), checks_may_only_read(), close_42() (+4 more)

### Community 6 - "TestDb"
Cohesion: 0.26
Nodes (8): connect(), Client, Option, Self, String, Vec, server_url(), TestDb

### Community 7 - "FakeWeb"
Cohesion: 0.19
Nodes (12): Html, FakeWeb, Inner, page(), Arc, HashMap, Mutex, Self (+4 more)

### Community 8 - "inv3_event_log.rs"
Cohesion: 0.31
Nodes (9): all_state_is_rebuilt_from_the_event_log_alone(), crash_after_a_side_effect_does_not_repeat_it(), String, Value, Vec, send(), snapshot_inbox(), snapshot_tasks() (+1 more)

### Community 9 - "inv8_layers.rs"
Cohesion: 0.33
Nodes (9): Script, a_group_layer_narrows_its_members(), a_user_layer_cannot_widen_the_organization_layer(), a_user_layer_narrows_for_that_user_only(), an_organization_forbid_wins_over_any_lower_permit(), close_42(), crm_app(), crm_script() (+1 more)

### Community 10 - "files_containing"
Cohesion: 0.48
Nodes (6): Path, contains(), encodings(), files_containing(), PathBuf, Vec

### Community 11 - "adapter-proto/src/lib.rs"
Cohesion: 0.09
Nodes (47): BufReader, base(), block(), catalog(), classify_status(), complete(), handle(), main() (+39 more)

### Community 16 - "Adapters"
Cohesion: 0.07
Nodes (39): AtomicU64, AdapterError, Adapters, Arc, Child, Drop, Event, HashMap (+31 more)

### Community 17 - "Person"
Cohesion: 0.07
Nodes (47): Appliance, Config, Connector, Database, header(), mcp_http(), Model, Person (+39 more)

### Community 18 - "Run"
Cohesion: 0.09
Nodes (39): BTreeSet, status_str(), Flow, Kernel, Arc, Result, Self, clip() (+31 more)

### Community 19 - ".pipeline"
Cohesion: 0.14
Nodes (28): Action, CallFailure, canonical(), err_text(), Facts, Gate, has_rights(), idempotency_key() (+20 more)

### Community 20 - "Kernel"
Cohesion: 0.09
Nodes (30): AppTool, CallError, CallOutput, definition_hash(), Mcp, Client, Option, Result (+22 more)

### Community 21 - "api.rs"
Cohesion: 0.23
Nodes (41): adapters(), cancel(), catalog(), err(), find(), inbox_list(), internal(), kill_switch() (+33 more)

### Community 22 - "6. TheAgentCompany (TAC)"
Cohesion: 0.06
Nodes (34): 1. Meta Model API ("Muse" family), 2. xAI Grok: OAuth / SuperGrok sign-in, 3. OpenAI Codex with ChatGPT sign-in (codex-rs, commit eec8bde1), 4. GLM (Z.ai international, and BigModel in China), 5. Anthropic Messages API, 6. TheAgentCompany (TAC), API-key path (api.x.ai), Base URLs (+26 more)

### Community 23 - "Ev"
Cohesion: 0.11
Nodes (23): Budget, Self, ContextRef, Ev, DateTime, Option, String, Utc (+15 more)

### Community 24 - "Sandboxes"
Cohesion: 0.17
Nodes (17): Cfg, request(), Arc, Child, Drop, HashMap, Mutex, Option (+9 more)

### Community 25 - "guest/src/main.rs"
Cohesion: 0.19
Nodes (16): c_ulong, RawFd, exec(), handle(), init(), listen(), loopback_up(), main() (+8 more)

### Community 26 - "Web"
Cohesion: 0.23
Nodes (8): private_ip(), Client, Option, Result, String, Web, IpAddr, Url

### Community 27 - "Decisions"
Cohesion: 0.20
Nodes (9): Boundary and policy, Decisions, Evaluation, Event log and state, Identity, Models, Process, Runs and verification (+1 more)

### Community 28 - "spawn"
Cohesion: 0.25
Nodes (8): Command, Child, Mutex, Result, Sender, spawn(), spawner(), Job

### Community 29 - "cli.rs"
Cohesion: 0.53
Nodes (8): arg(), config(), main(), migrate(), Option, Result, String, serve()

### Community 30 - "inv1_boundary.rs"
Cohesion: 0.25
Nodes (8): a_changed_tool_definition_is_disabled_and_hidden(), host_addresses(), missing_credential_denies_the_action(), prohibited_tools_never_run_even_if_policy_permits_everything(), String, Vec, sandbox_has_no_network_device_and_reaches_nothing(), tools_the_owner_has_not_classified_cannot_run()

### Community 31 - "inv6_policy.rs"
Cohesion: 0.46
Nodes (7): a_one_action_approval_covers_one_action(), mail_app(), owner_policy_grants_autonomy(), send(), the_agent_proposes_and_only_the_owner_grants(), the_approval_channel_needs_a_person(), without_owner_policy_an_irreversible_action_waits_for_approval()

## Knowledge Gaps
- **37 isolated node(s):** `Process`, `Event log and state`, `Boundary and policy`, `Runs and verification`, `Identity` (+32 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **1 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Config` connect `Person` to `Adapters`, `.pipeline`, `Kernel`, `api.rs`, `Ev`, `cli.rs`?**
  _High betweenness centrality (0.105) - this node is a cross-community bridge._
- **Why does `Ev` connect `Ev` to `.pipeline`, `adapter-proto/src/lib.rs`, `Kernel`?**
  _High betweenness centrality (0.082) - this node is a cross-community bridge._
- **Why does `Spec` connect `Api` to `inv7_verify.rs`, `Ev`?**
  _High betweenness centrality (0.073) - this node is a cross-community bridge._
- **Are the 36 inferred relationships involving `steps()` (e.g. with `a_changed_tool_definition_is_disabled_and_hidden()` and `missing_credential_denies_the_action()`) actually correct?**
  _`steps()` has 36 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Process`, `Event log and state`, `Boundary and policy` to the rest of the system?**
  _37 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `mcp.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.10884353741496598 - nodes in this community are weakly interconnected._
- **Should `Api` be split into smaller, more focused modules?**
  _Cohesion score 0.0880503144654088 - nodes in this community are weakly interconnected._