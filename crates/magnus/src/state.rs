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
    pub last_seq: i64,
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

#[derive(Default)]
pub struct State {
    pub tasks: BTreeMap<String, Task>,
    pub runs: HashMap<String, Run>,
    pub items: BTreeMap<String, Item>,
    pub approvals: Vec<Approval>,
    pub policies: BTreeMap<String, String>,
    pub kill_switch: bool,
    pub pins: HashMap<String, Pin>,
    /// Tools whose live definition differs from the pin, with the open item.
    pub changed: HashMap<String, (String, Pin)>,
    pub secrets: HashSet<String>,
    pub memory: Vec<MemoryEntry>,
    /// Files attached to each task, by task id.
    pub attachments: HashMap<String, Vec<Value>>,
    /// Latest conformance result per adapter id: (version, model, passed, report).
    pub conformance: HashMap<String, (String, String, bool, Value)>,
    pub last_seq: i64,
}

impl State {
    pub fn apply(&mut self, e: &Event) {
        self.last_seq = e.seq;
        if let Some(r) = e.ev.run().and_then(|id| self.runs.get_mut(id)) {
            r.last_seq = e.seq;
        }
        match &e.ev {
            Ev::KernelBoot { .. } => {}
            Ev::SecretSet { name } => {
                self.secrets.insert(name.clone());
            }
            Ev::PolicySet { layer, cedar } => {
                self.policies.insert(layer.clone(), cedar.clone());
            }
            Ev::KillSwitch { engaged } => self.kill_switch = *engaged,
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
            Ev::TaskCreated { task, run, person, text, config, attachments } => {
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
                        verified: BTreeMap::new(),
                        // Attached files are content from elsewhere: data, not instructions.
                        untrusted: !attachments.is_empty(),
                        private: false,
                        private_from: Default::default(),
                        model_calls: 0,
                        actions: 0,
                        usd: 0.0,
                        intents: HashMap::new(),
                        wait_until: None,
                        summary: None,
                        last_seq: e.seq,
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
                    r.replies.push(Reply { step: *step, seq: e.seq, blocks: blocks.clone(), state: state.clone() });
                }
            }
            Ev::ModelError { .. } => {}
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
            Ev::Decision { .. } => {}
            Ev::ToolResult { run, call_id, key, status, text, is_error, label, data, sources } => {
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
                    if label.untrusted {
                        r.untrusted = true;
                    }
                    if label.private {
                        r.private = true;
                        r.private_from.extend(sources.iter().cloned());
                        if sources.is_empty() {
                            r.private_from.insert(String::new());
                        }
                    }
                }
            }
            Ev::Contract { run, version, contract } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.contract = Some(contract.clone());
                    r.contract_version = *version;
                    r.verified.clear();
                }
            }
            Ev::Verification { run, criterion, passed, evidence, by } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.verified.insert(criterion.clone(), (*passed, evidence.clone(), by.clone()));
                }
            }
            Ev::Suspended { run, reason, .. } => {
                if let Some(r) = self.runs.get_mut(run) {
                    if !r.status.terminal() {
                        r.status = Status::Suspended;
                        r.reason = Some(reason.clone());
                    }
                }
            }
            Ev::Resumed { run } => {
                if let Some(r) = self.runs.get_mut(run) {
                    if !r.status.terminal() {
                        r.status = Status::Running;
                        r.reason = None;
                        r.wait_until = None;
                    }
                }
            }
            Ev::Completed { run, summary } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.status = Status::Completed;
                    r.summary = Some(summary.clone());
                    r.reason = None;
                }
            }
            Ev::Failed { run, reason, .. } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.status = Status::Failed;
                    r.summary = Some(reason.clone());
                    r.reason = None;
                }
            }
            Ev::Cancelled { run } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.status = Status::Cancelled;
                    r.reason = None;
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

    pub fn open_item_for_run(&self, run: &str, kind: &str) -> Option<&Item> {
        self.items
            .values()
            .find(|i| i.run.as_deref() == Some(run) && i.kind == kind && i.resolution.is_none())
    }
}
