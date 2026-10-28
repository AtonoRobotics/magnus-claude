//! The projection of the event log: tasks, runs, inbox, approvals, policy.
//! It holds nothing the log does not; `State::apply` is the only writer, and a
//! boot replays every event through it.

use crate::events::{Ev, RunConfig};
use crate::log::Event;
use crate::tools::Label;
use chrono::{DateTime, Utc};
use magnus_adapter_proto::Block;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Running,
    Suspended,
    Completed,
    Failed,
    Cancelled,
}

impl Status {
    pub fn terminal(self) -> bool {
        matches!(self, Status::Completed | Status::Failed | Status::Cancelled)
    }
}

#[derive(Clone, Debug)]
pub struct Reply {
    pub step: u32,
    pub seq: i64,
    pub blocks: Vec<Block>,
    pub state: Option<Value>,
}

impl Reply {
    pub fn calls(&self) -> Vec<(String, String, Value)> {
        self.blocks
            .iter()
            .filter_map(|b| match b {
                Block::ToolCall { id, name, input } => Some((id.clone(), name.clone(), input.clone())),
                _ => None,
            })
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct ResultRec {
    pub seq: i64,
    pub text: String,
    pub is_error: bool,
    pub status: String,
    pub data: Option<Value>,
}

#[derive(Clone, Debug)]
pub struct Intent {
    pub call_id: String,
    pub tool: String,
    pub input: Value,
    pub issuer: String,
    /// pending until a result for this key is recorded
    pub result: Option<ResultRec>,
}

#[derive(Clone, Debug)]
pub struct Run {
    pub id: String,
    pub task: String,
    pub person: String,
    pub config: RunConfig,
    pub status: Status,
    pub reason: Option<String>,
    pub replies: Vec<Reply>,
    pub results: HashMap<String, ResultRec>,
    pub notices: Vec<(u32, i64, String)>,
    pub contract: Option<Value>,
    pub contract_version: u32,
    /// The agent has started the work (its first action that changes anything).
    pub work_started: bool,
    /// The version frozen when work started; later changes need approval.
    pub contract_frozen: Option<u32>,
    /// The run's taint when the contract in force was written: its checks
    /// were written then, so the Rule of Two judges them by it. A version a
    /// person approved carries none.
    pub contract_taint: (bool, bool),
    /// Who approved the contract in force, when it was a change.
    pub contract_approved_by: Option<String>,
    pub verified: BTreeMap<String, (bool, Value, String)>,
    pub untrusted: bool,
    pub private: bool,
    /// Apps whose private content this run has read.
    pub private_from: std::collections::BTreeSet<String>,
    pub model_calls: u32,
    pub actions: u32,
    pub usd: f64,
    pub intents: HashMap<String, Intent>,
    pub wait_until: Option<DateTime<Utc>>,
    pub summary: Option<String>,
    /// Delivered files: (id, name, media type, size in bytes).
    pub outputs: Vec<(String, String, String, usize)>,
    pub last_seq: i64,
    pub experiment: Option<crate::events::ExperimentTag>,
    pub plan: Option<Value>,
    pub plan_version: u32,
    pub outcome_view: Option<Value>,
    pub spoken: Option<String>,
    /// Graders' verdicts: (grader, passed, score, details).
    pub grades: Vec<(String, bool, Option<f64>, Value)>,
    pub created_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    /// Time of the latest event for this run, to spot stalled runs.
    pub last_at: DateTime<Utc>,
    /// Boundary decisions, in order (E4).
    pub decisions: Vec<DecisionRec>,
    /// Every reason this run was ever suspended for.
    pub suspensions: Vec<String>,
    /// Provider-reported tokens over all replies.
    pub tokens: Tokens,
    pub model_errors: u32,
    /// Contract checks that failed, over every completion attempt.
    pub checks_failed: u32,
    /// The inbox item a suspended run waits on.
    pub waiting_item: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DecisionRec {
    pub seq: i64,
    pub call_id: String,
    pub tool: String,
    pub stage: String,
    pub outcome: String,
    pub rule: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Run {
    /// Tool calls from the latest reply that have no result yet, in order.
    pub fn pending_calls(&self) -> Vec<(usize, String, String, Value)> {
        let Some(r) = self.replies.last() else { return vec![] };
        r.calls()
            .into_iter()
            .enumerate()
            .filter(|(_, (id, _, _))| !self.results.contains_key(id))
            .map(|(i, (id, n, v))| (i, id, n, v))
            .collect()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Task {
    pub task_id: String,
    pub run_id: String,
    pub person: String,
    pub text: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    pub kind: String,
    pub task: Option<String>,
    pub run: Option<String>,
    pub person: String,
    pub summary: String,
    pub details: Value,
    pub created_at: DateTime<Utc>,
    pub resolution: Option<(String, Option<String>, Option<String>, String)>,
}

#[derive(Clone, Debug)]
pub struct Approval {
    pub id: String,
    pub person: String,
    pub task: String,
    pub tool: String,
    pub input_hash: Option<String>,
    pub scope: String,
    pub until: Option<DateTime<Utc>>,
    pub used: bool,
    pub by: String,
    /// Why the approval was asked for: the rule, or `unknown_outcome`.
    pub reason: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Pin {
    pub hash: String,
    pub description: String,
    pub input_schema: Value,
    pub approved: bool,
}

#[derive(Clone, Debug)]
pub struct MemoryEntry {
    pub id: String,
    pub person: String,
    pub run: String,
    pub fact: String,
    pub topic: Option<String>,
    pub label: Label,
    pub sources: Vec<String>,
    pub at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct PolicyVersion {
    pub layer: String,
    pub seq: i64,
    pub at: DateTime<Utc>,
    pub by: String,
    pub cedar: String,
}

/// One item of the agent's playbook (I2).
#[derive(Clone, Debug)]
pub struct PlaybookItem {
    pub id: String,
    pub person: String,
    pub text: String,
    pub addresses: Option<String>,
    /// The run that added it, and the runs that edited it.
    pub runs: Vec<String>,
    pub label: Label,
    /// Playbook version at which the item was added, and retired.
    pub added: u64,
    pub retired: Option<u64>,
    pub added_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct Schedule {
    pub id: String,
    pub person: String,
    pub text: String,
    pub every_minutes: Option<u32>,
    pub daily_at: Option<String>,
    pub created: DateTime<Utc>,
    pub last_fired: Option<DateTime<Utc>>,
}

impl Schedule {
    /// The next time this schedule is due after its last run (or creation).
    pub fn next_due(&self) -> Option<DateTime<Utc>> {
        let after = self.last_fired.unwrap_or(self.created);
        if let Some(m) = self.every_minutes {
            return Some(after + chrono::Duration::minutes(m.max(1) as i64));
        }
        let (h, m) = self.daily_at.as_deref()?.split_once(':')?;
        let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
        use chrono::{Local, TimeZone};
        let local = after.with_timezone(&Local);
        let mut day = local.date_naive();
        loop {
            let t = Local.from_local_datetime(&day.and_hms_opt(h, m, 0)?).earliest()?;
            if t > local {
                return Some(t.with_timezone(&Utc));
            }
            day = day.succ_opt()?;
        }
    }
}

#[derive(Default)]
pub struct State {
    pub tasks: BTreeMap<String, Task>,
    pub runs: HashMap<String, Run>,
    pub items: BTreeMap<String, Item>,
    pub approvals: Vec<Approval>,
    pub policies: BTreeMap<String, String>,
    pub kill_switch: bool,
    pub schedules: BTreeMap<String, Schedule>,
    pub model: Option<(String, String)>,
    pub pins: HashMap<String, Pin>,
    /// Tools whose live definition differs from the pin, with the open item.
    pub changed: HashMap<String, (String, Pin)>,
    pub secrets: HashSet<String>,
    pub memory: Vec<MemoryEntry>,
    /// Files attached to each task, by task id.
    pub attachments: HashMap<String, Vec<Value>>,
    /// Latest conformance result per adapter id: (version, model, passed, report).
    pub conformance: HashMap<String, (String, String, bool, Value)>,
    pub playbook: BTreeMap<String, PlaybookItem>,
    /// Every policy a layer ever had, oldest first.
    pub policy_history: Vec<PolicyVersion>,
    /// Model and harness changes, for version notices (U5): (seq, at, what, value).
    pub version_changes: Vec<(i64, DateTime<Utc>, String, String)>,
    /// Increments with every playbook change (I3).
    pub playbook_version: u64,
    pub last_seq: i64,
}

impl State {
    pub fn apply(&mut self, e: &Event) {
        self.last_seq = e.seq;
        if let Some(r) = e.ev.run().and_then(|id| self.runs.get_mut(id)) {
            r.last_seq = e.seq;
            r.last_at = e.at;
        }
        match &e.ev {
            Ev::KernelBoot { version, instructions, .. } => {
                for (what, v) in [("harness", version), ("instructions", instructions)] {
                    if !v.is_empty() && self.version_changes.iter().rev().find(|c| c.2 == what).is_none_or(|c| &c.3 != v) {
                        self.version_changes.push((e.seq, e.at, what.into(), v.clone()));
                    }
                }
            }
            Ev::SecretSet { name } => {
                self.secrets.insert(name.clone());
            }
            Ev::PolicySet { layer, cedar } => {
                self.policies.insert(layer.clone(), cedar.clone());
                self.policy_history.push(PolicyVersion { layer: layer.clone(), seq: e.seq, at: e.at, by: e.actor.clone(), cedar: cedar.clone() });
                let n = self.policy_history.iter().filter(|p| &p.layer == layer).count();
                self.version_changes.push((e.seq, e.at, format!("policy:{layer}"), format!("v{n}")));
            }
            Ev::KillSwitch { engaged } => self.kill_switch = *engaged,
            Ev::SignedIn { .. } | Ev::SignedOut { .. } => {}
            Ev::ScheduleSet { id, person, text, every_minutes, daily_at } => {
                self.schedules.insert(
                    id.clone(),
                    Schedule { id: id.clone(), person: person.clone(), text: text.clone(), every_minutes: *every_minutes, daily_at: daily_at.clone(), created: e.at, last_fired: None },
                );
            }
            Ev::ScheduleRemoved { id } => {
                self.schedules.remove(id);
            }
            Ev::ScheduleFired { id, .. } => {
                if let Some(s) = self.schedules.get_mut(id) {
                    s.last_fired = Some(e.at);
                }
            }
            Ev::ModelSet { adapter, model } => {
                self.model = Some((adapter.clone(), model.clone()));
                self.version_changes.push((e.seq, e.at, "model".into(), format!("{adapter}/{model}")));
            }
            Ev::ToolPinned { tool, hash, description, input_schema, approved } => {
                self.pins.insert(
                    tool.clone(),
                    Pin { hash: hash.clone(), description: description.clone(), input_schema: input_schema.clone(), approved: *approved },
                );
                self.changed.remove(tool);
            }
            Ev::ToolChanged { tool, hash, description, input_schema, item } => {
                self.changed.insert(
                    tool.clone(),
                    (item.clone(), Pin { hash: hash.clone(), description: description.clone(), input_schema: input_schema.clone(), approved: false }),
                );
            }
            Ev::TaskCreated { task, run, person, text, config, attachments, experiment } => {
                // Playbook items the run starts with carry the taint of the
                // runs that wrote them (I2, M1).
                let pb = self.playbook_at(person, config.playbook).into_iter().fold(Label::PRINCIPAL, |l, i| l.join(i.label));
                if !attachments.is_empty() {
                    self.attachments.insert(task.clone(), attachments.clone());
                }
                self.tasks.insert(
                    task.clone(),
                    Task { task_id: task.clone(), run_id: run.clone(), person: person.clone(), text: text.clone(), created_at: e.at },
                );
                self.runs.insert(
                    run.clone(),
                    Run {
                        id: run.clone(),
                        task: task.clone(),
                        person: person.clone(),
                        config: config.clone(),
                        status: Status::Running,
                        reason: None,
                        replies: vec![],
                        results: HashMap::new(),
                        notices: vec![],
                        contract: None,
                        contract_version: 0,
                        work_started: false,
                        contract_frozen: None,
                        contract_taint: (false, false),
                        contract_approved_by: None,
                        verified: BTreeMap::new(),
                        // Attached files are content from elsewhere: data, not instructions.
                        untrusted: !attachments.is_empty() || pb.untrusted,
                        private: pb.private,
                        private_from: Default::default(),
                        model_calls: 0,
                        actions: 0,
                        usd: 0.0,
                        intents: HashMap::new(),
                        wait_until: None,
                        summary: None,
                        outputs: vec![],
                        last_seq: e.seq,
                        experiment: experiment.clone(),
                        plan: None,
                        plan_version: 0,
                        outcome_view: None,
                        spoken: None,
                        grades: vec![],
                        created_at: e.at,
                        ended_at: None,
                        last_at: e.at,
                        decisions: vec![],
                        suspensions: vec![],
                        tokens: Tokens::default(),
                        model_errors: 0,
                        checks_failed: 0,
                        waiting_item: None,
                    },
                );
            }
            Ev::ModelCall { run, .. } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.model_calls += 1;
                }
            }
            Ev::ModelReply { run, step, blocks, state, usage, .. } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.usd += usage.cost_usd;
                    r.tokens.input += usage.input_tokens;
                    r.tokens.output += usage.output_tokens;
                    r.tokens.cache_read += usage.cache_read_tokens;
                    r.tokens.cache_write += usage.cache_write_tokens;
                    r.replies.push(Reply { step: *step, seq: e.seq, blocks: blocks.clone(), state: state.clone() });
                }
            }
            Ev::ModelError { run, .. } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.model_errors += 1;
                }
            }
            Ev::Notice { run, step, text } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.notices.push((*step, e.seq, text.clone()));
                }
            }
            Ev::Intent { run, call_id, key, tool, input, issuer, .. } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.actions += 1;
                    r.intents.insert(
                        key.clone(),
                        Intent { call_id: call_id.clone(), tool: tool.clone(), input: input.clone(), issuer: issuer.clone(), result: None },
                    );
                }
            }
            Ev::Decision { run, call_id, tool, outcome, stage, rule, .. } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.decisions.push(DecisionRec { seq: e.seq, call_id: call_id.clone(), tool: tool.clone(), stage: stage.clone(), outcome: outcome.clone(), rule: rule.clone() });
                }
            }
            Ev::ToolResult { run, call_id, key, status, text, is_error, label, data, sources, check } => {
                if let Some(r) = self.runs.get_mut(run) {
                    let rec = ResultRec { seq: e.seq, text: text.clone(), is_error: *is_error, status: status.clone(), data: data.clone() };
                    if let Some(k) = key {
                        if let Some(i) = r.intents.get_mut(k) {
                            i.result = Some(rec.clone());
                        }
                    }
                    if !call_id.is_empty() {
                        r.results.insert(call_id.clone(), rec);
                    }
                    // A check's result is compared, never read by the model:
                    // taint limits authority, not reading.
                    if label.untrusted && !*check {
                        r.untrusted = true;
                    }
                    if label.private && !*check {
                        r.private = true;
                        r.private_from.extend(sources.iter().cloned());
                        if sources.is_empty() {
                            r.private_from.insert(String::new());
                        }
                    }
                }
            }
            Ev::Contract { run, version, contract, approved_by } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.contract = Some(contract.clone());
                    r.contract_version = *version;
                    r.verified.clear();
                    r.contract_taint = if approved_by.is_some() { (false, false) } else { (r.untrusted, r.private) };
                    if approved_by.is_some() {
                        r.contract_frozen = Some(*version);
                    }
                    r.contract_approved_by = approved_by.clone();
                }
            }
            Ev::WorkStarted { run } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.work_started = true;
                }
            }
            Ev::ContractFrozen { run, version } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.contract_frozen = Some(*version);
                }
            }
            Ev::Plan { run, version, plan } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.plan = Some(plan.clone());
                    r.plan_version = *version;
                }
            }
            Ev::Playbook { item, op, person, run, text, addresses, label } => {
                self.playbook_version += 1;
                let v = self.playbook_version;
                self.version_changes.push((e.seq, e.at, "playbook".into(), format!("v{v}")));
                match op.as_str() {
                    "add" => {
                        self.playbook.insert(
                            item.clone(),
                            PlaybookItem {
                                id: item.clone(),
                                person: person.clone(),
                                text: text.clone().unwrap_or_default(),
                                addresses: addresses.clone(),
                                runs: vec![run.clone()],
                                label: *label,
                                added: v,
                                retired: None,
                                added_at: e.at,
                            },
                        );
                    }
                    "edit" => {
                        if let Some(i) = self.playbook.get_mut(item) {
                            if let Some(t) = text {
                                i.text = t.clone();
                            }
                            if addresses.is_some() {
                                i.addresses = addresses.clone();
                            }
                            i.runs.push(run.clone());
                            i.label = i.label.join(*label);
                        }
                    }
                    "retire" => {
                        if let Some(i) = self.playbook.get_mut(item) {
                            i.retired = Some(v);
                        }
                    }
                    _ => {}
                }
            }
            Ev::Graded { run, grader, passed, score, details } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.grades.push((grader.clone(), *passed, *score, details.clone()));
                }
            }
            Ev::Verification { run, criterion, passed, evidence, by } => {
                if let Some(r) = self.runs.get_mut(run) {
                    if !passed {
                        r.checks_failed += 1;
                    }
                    r.verified.insert(criterion.clone(), (*passed, evidence.clone(), by.clone()));
                }
            }
            Ev::Suspended { run, reason, item } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.suspensions.push(reason.clone());
                    if !r.status.terminal() {
                        r.status = Status::Suspended;
                        r.reason = Some(reason.clone());
                        r.waiting_item = item.clone();
                    }
                }
            }
            Ev::Resumed { run } => {
                if let Some(r) = self.runs.get_mut(run) {
                    if !r.status.terminal() {
                        r.status = Status::Running;
                        r.reason = None;
                        r.wait_until = None;
                        r.waiting_item = None;
                    }
                }
            }
            Ev::Completed { run, summary, view, spoken } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.status = Status::Completed;
                    r.summary = Some(summary.clone());
                    r.outcome_view = view.clone();
                    r.spoken = spoken.clone();
                    r.reason = None;
                    r.ended_at = Some(e.at);
                }
            }
            Ev::Failed { run, reason, .. } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.status = Status::Failed;
                    r.summary = Some(reason.clone());
                    r.reason = None;
                    r.ended_at = Some(e.at);
                }
            }
            Ev::Cancelled { run } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.status = Status::Cancelled;
                    r.reason = None;
                    r.ended_at = Some(e.at);
                }
            }
            Ev::Item { id, item_kind, task, run, person, summary, details } => {
                self.items.insert(
                    id.clone(),
                    Item {
                        id: id.clone(),
                        kind: item_kind.clone(),
                        task: task.clone(),
                        run: run.clone(),
                        person: person.clone(),
                        summary: summary.clone(),
                        details: details.clone(),
                        created_at: e.at,
                        resolution: None,
                    },
                );
            }
            Ev::ItemResolved { id, decision, scope, text } => {
                if let Some(i) = self.items.get_mut(id) {
                    i.resolution = Some((decision.clone(), scope.clone(), text.clone(), e.actor.clone()));
                }
            }
            Ev::Approval { id, person, task, tool, input_hash, scope, until, reason, .. } => self.approvals.push(Approval {
                id: id.clone(),
                person: person.clone(),
                task: task.clone(),
                tool: tool.clone(),
                input_hash: input_hash.clone(),
                scope: scope.clone(),
                until: *until,
                used: false,
                by: e.actor.clone(),
                reason: reason.clone(),
            }),
            Ev::ApprovalUsed { id, .. } => {
                if let Some(a) = self.approvals.iter_mut().find(|a| &a.id == id) {
                    a.used = true;
                }
            }
            Ev::Wait { run, until } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.wait_until = Some(*until);
                }
            }
            Ev::BudgetExtended { run, budget } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.config.budget = budget.clone();
                }
            }
            Ev::Output { id, run, name, media_type, data_b64 } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.outputs.push((id.clone(), name.clone(), media_type.clone(), data_b64.len() * 3 / 4));
                }
            }
            Ev::AdapterVerified { adapter_id, version, model, passed, report, .. } => {
                self.conformance.insert(adapter_id.clone(), (version.clone(), model.clone(), *passed, report.clone()));
            }
            Ev::Memory { id, person, run, fact, topic, label, sources } => self.memory.push(MemoryEntry {
                id: id.clone(),
                person: person.clone(),
                run: run.clone(),
                fact: fact.clone(),
                topic: topic.clone(),
                label: *label,
                sources: sources.clone(),
                at: e.at,
            }),
        }
    }

    /// The person's playbook items that were live at playbook version `v`.
    pub fn playbook_at(&self, person: &str, v: u64) -> Vec<&PlaybookItem> {
        self.playbook.values().filter(|i| i.person == person && i.added <= v && i.retired.is_none_or(|r| r > v)).collect()
    }

    pub fn open_item_for_run(&self, run: &str, kind: &str) -> Option<&Item> {
        self.items
            .values()
            .find(|i| i.run.as_deref() == Some(run) && i.kind == kind && i.resolution.is_none())
    }
}
