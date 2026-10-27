# Graph Report - magnus-claude  (2026-10-27)

## Corpus Check
- 61 files · ~50,628 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 988 nodes · 2417 edges · 46 communities (41 shown, 5 thin omitted)
- Extraction: 96% EXTRACTED · 4% INFERRED · 0% AMBIGUOUS · INFERRED: 105 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `31ab329f`
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

## God Nodes (most connected - your core abstractions)
1. `steps()` - 42 edges
2. `Kernel` - 29 edges
3. `Run` - 28 edges
4. `Person` - 26 edges
5. `Adapters` - 25 edges
6. `err()` - 24 edges
7. `Config` - 24 edges
8. `Ev` - 23 edges
9. `Sandboxes` - 23 edges
10. `who()` - 22 edges

## Surprising Connections (you probably didn't know these)
- `crash_after_a_side_effect_does_not_repeat_it()` --calls--> `steps()`  [INFERRED]
  tests/invariants/tests/inv3_event_log.rs → tests/invariants/src/model.rs
- `the_kernel_cannot_rewrite_the_log()` --calls--> `steps()`  [INFERRED]
  tests/invariants/tests/inv3_event_log.rs → tests/invariants/src/model.rs
- `a_reply_without_a_tool_call_does_not_end_the_run()` --calls--> `steps()`  [INFERRED]
  tests/invariants/tests/inv5_model_decides.rs → tests/invariants/src/model.rs
- `every_app_call_traces_to_a_model_decision()` --calls--> `steps()`  [INFERRED]
  tests/invariants/tests/inv5_model_decides.rs → tests/invariants/src/model.rs
- `a_criterion_without_a_check_goes_to_the_person_for_review()` --calls--> `steps()`  [INFERRED]
  tests/invariants/tests/inv7_verify.rs → tests/invariants/src/model.rs

## Import Cycles
- None detected.

## Communities (46 total, 5 thin omitted)

### Community 0 - "steps"
Cohesion: 0.14
Nodes (23): canary(), permit_execute(), String, steps(), a_changed_tool_definition_is_disabled_and_hidden(), host_addresses(), missing_credential_denies_the_action(), prohibited_tools_never_run_even_if_policy_permits_everything() (+15 more)

### Community 1 - "mcp.rs"
Cohesion: 0.11
Nodes (33): Handler, Notify, Builder, Call, chat(), crm(), err(), FakeMcp (+25 more)

### Community 2 - "Api"
Cohesion: 0.07
Nodes (31): BTreeMap, Child, Method, StatusCode, TempDir, Api, Appliance, bin() (+23 more)

### Community 3 - "Reply"
Cohesion: 0.15
Nodes (21): content_text(), Conversation, FakeModel, Inner, messages(), Reply, Arc, HashMap (+13 more)

### Community 4 - "never"
Cohesion: 0.16
Nodes (21): Duration, contract_unchecked(), eventually(), fail(), never(), F, F, Self (+13 more)

### Community 5 - "inv7_verify.rs"
Cohesion: 0.24
Nodes (12): complete(), contract_checked(), Value, repeated_crashes_never_repeat_an_effect(), a_criterion_without_a_check_goes_to_the_person_for_review(), a_false_report_of_completion_is_caught_by_checking_the_world(), checks_may_only_read(), close_42() (+4 more)

### Community 6 - "TestDb"
Cohesion: 0.07
Nodes (49): Box, Bytes, complete(), messages(), Block, Client, CompleteParams, Result (+41 more)

### Community 7 - "FakeWeb"
Cohesion: 0.19
Nodes (12): Html, FakeWeb, Inner, page(), Arc, HashMap, Mutex, Self (+4 more)

### Community 8 - "inv3_event_log.rs"
Cohesion: 0.31
Nodes (9): all_state_is_rebuilt_from_the_event_log_alone(), crash_after_a_side_effect_does_not_repeat_it(), String, Value, Vec, send(), snapshot_inbox(), snapshot_tasks() (+1 more)

### Community 9 - "inv8_layers.rs"
Cohesion: 0.26
Nodes (11): Script, org_policy(), a_denial_goes_back_to_the_model_which_decides_what_next(), a_group_layer_narrows_its_members(), a_user_layer_cannot_widen_the_organization_layer(), a_user_layer_narrows_for_that_user_only(), an_organization_forbid_wins_over_any_lower_permit(), close_42() (+3 more)

### Community 10 - "files_containing"
Cohesion: 0.48
Nodes (6): Path, contains(), encodings(), files_containing(), PathBuf, Vec

### Community 11 - "adapter-proto/src/lib.rs"
Cohesion: 0.09
Nodes (47): BufReader, base(), block(), catalog(), classify_status(), complete(), handle(), main() (+39 more)

### Community 16 - "Adapters"
Cohesion: 0.11
Nodes (28): AtomicU64, AdapterError, Adapters, Arc, Block, Child, Drop, ErrorClass (+20 more)

### Community 17 - "Person"
Cohesion: 0.07
Nodes (49): Adapter, Appliance, Config, Connector, Database, header(), mcp_http(), Model (+41 more)

### Community 18 - "Run"
Cohesion: 0.20
Nodes (24): BTreeSet, status_str(), Approval, Intent, Item, MemoryEntry, Pin, Reply (+16 more)

### Community 19 - ".pipeline"
Cohesion: 0.12
Nodes (30): Action, CallFailure, canonical(), err_text(), Facts, Gate, has_rights(), idempotency_key() (+22 more)

### Community 20 - "Kernel"
Cohesion: 0.28
Nodes (12): AppTool, CallError, CallOutput, definition_hash(), Mcp, Client, Option, Result (+4 more)

### Community 21 - "api.rs"
Cohesion: 0.15
Nodes (49): adapters(), async_stream(), cancel(), catalog(), conformance(), err(), find(), health() (+41 more)

### Community 22 - "6. TheAgentCompany (TAC)"
Cohesion: 0.06
Nodes (34): 1. Meta Model API ("Muse" family), 2. xAI Grok: OAuth / SuperGrok sign-in, 3. OpenAI Codex with ChatGPT sign-in (codex-rs, commit eec8bde1), 4. GLM (Z.ai international, and BigModel in China), 5. Anthropic Messages API, 6. TheAgentCompany (TAC), API-key path (api.x.ai), Base URLs (+26 more)

### Community 23 - "Ev"
Cohesion: 0.08
Nodes (31): Budget, Self, ContextRef, Ev, Block, DateTime, ErrorClass, Option (+23 more)

### Community 24 - "Sandboxes"
Cohesion: 0.07
Nodes (36): Cfg, Kernel, Arc, Event, HashMap, HashSet, Mutex, Option (+28 more)

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
Cohesion: 0.46
Nodes (7): a_one_action_approval_covers_one_action(), mail_app(), owner_policy_grants_autonomy(), send(), the_agent_proposes_and_only_the_owner_grants(), the_approval_channel_needs_a_person(), without_owner_policy_an_irreversible_action_waits_for_approval()

### Community 36 - "new_id"
Cohesion: 0.17
Nodes (16): new_id(), clip(), instructions_version(), Kernel, Arc, Into, Message, Option (+8 more)

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

## Knowledge Gaps
- **43 isolated node(s):** `Process`, `Event log and state`, `Boundary and policy`, `Runs and verification`, `Identity` (+38 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **5 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Config` connect `Person` to `Adapters`, `.pipeline`, `api.rs`, `Ev`, `Sandboxes`, `cli.rs`, `inv1_boundary.rs`?**
  _High betweenness centrality (0.116) - this node is a cross-community bridge._
- **Why does `eval()` connect `Person` to `adapter-proto/src/lib.rs`?**
  _High betweenness centrality (0.094) - this node is a cross-community bridge._
- **Why does `Request` connect `adapter-proto/src/lib.rs` to `Person`?**
  _High betweenness centrality (0.092) - this node is a cross-community bridge._
- **Are the 36 inferred relationships involving `steps()` (e.g. with `a_changed_tool_definition_is_disabled_and_hidden()` and `missing_credential_denies_the_action()`) actually correct?**
  _`steps()` has 36 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Process`, `Event log and state`, `Boundary and policy` to the rest of the system?**
  _43 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `steps` be split into smaller, more focused modules?**
  _Cohesion score 0.14153846153846153 - nodes in this community are weakly interconnected._
- **Should `mcp.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.10775510204081633 - nodes in this community are weakly interconnected._