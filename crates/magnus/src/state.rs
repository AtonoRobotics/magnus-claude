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
    /// The reply's text, its blocks joined.
    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .filter_map(|b| match b {
                Block::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string()
    }

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
    /// When it first ended or stopped (for a person, or to wait): how long a
    /// request took, whatever was done with it afterwards.
    pub first_stop: Option<DateTime<Utc>>,
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
    /// Replies the lost-reset module took out of the model's context.
    pub cuts: Vec<Cut>,
    /// The reply lost-detection reads from: after the latest reset or
    /// escalation, what came before is no longer the run's current attempt.
    pub lost_since: u32,
    /// The stronger model the escalation module moved this run to; its
    /// pinned configuration stays what it started with.
    pub escalated: Option<(String, String)>,
}

#[derive(Clone, Debug)]
pub struct Cut {
    pub from: u32,
    pub to: u32,
    pub note: String,
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
    /// The adapter and model the run's next call goes to.
    pub fn model(&self) -> (&str, &str) {
        match &self.escalated {
            Some((a, m)) => (a, m),
            None => (&self.config.adapter, &self.config.model),
        }
    }

    /// Tool calls from the latest reply that have no result yet, in order.
    /// Every tool call with its result, in the order the model made them.
    pub fn results_in_order(&self) -> Vec<Value> {
        self.replies
            .iter()
            .flat_map(|r| r.calls())
            .filter_map(|(id, name, _)| self.results.get(&id).map(|x| serde_json::json!({ "tool": name, "text": x.text, "is_error": x.is_error, "status": x.status, "seq": x.seq })))
            .collect()
    }

    /// What a chat turn showed below its answer: the components of its
    /// last `show` the kernel accepted.
    pub fn shown(&self) -> Option<Value> {
        self.replies
            .iter()
            .flat_map(|r| r.calls())
            .filter(|(id, name, _)| name == "show" && self.results.get(id).is_some_and(|x| !x.is_error))
            .map(|(_, _, input)| input["components"].clone())
            .last()
    }

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
    pub chat: Option<crate::events::ChatOrigin>,
    /// The short title the model gave it when starting it, if any.
    pub title: Option<String>,
    /// The schedule that started it, if any.
    pub schedule: Option<String>,
    /// The scope the person confirmed for that schedule, which its run is told.
    pub scope: Option<String>,
    /// Library files the person gave it.
    pub files: Vec<String>,
    /// The project it started in, as it stood then.
    pub project: Option<crate::events::ProjectContext>,
    /// The person's latest verdict on its outcome.
    pub verdict: Option<Verdict>,
}

/// A person's verdict on an outcome.
#[derive(Clone, Debug, Serialize)]
pub struct Verdict {
    pub verdict: String,
    pub note: Option<String>,
    pub at: DateTime<Utc>,
}

/// The verdicts a person can give, best first.
/// A secret the kill switch took out: `at_provider` is None until the
/// provider answered.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Revoked {
    pub adapter: Option<String>,
    pub at_provider: Option<bool>,
    pub detail: Option<String>,
}

pub const VERDICTS: [&str; 3] = ["right", "partly", "wrong"];

/// A file in a person's library: one they uploaded, or one a task delivered.
#[derive(Clone, Debug)]
pub struct LibFile {
    pub id: String,
    pub person: String,
    pub name: String,
    pub media_type: String,
    pub size: usize,
    pub at: DateTime<Utc>,
    /// An upload's content, kept for the runs given it (inputs, images,
    /// PDFs). A delivered file's is read from the log when needed.
    pub data_b64: Option<String>,
    /// The run that delivered it.
    pub run: Option<String>,
    /// The chat thread it was uploaded in.
    pub thread: Option<String>,
    /// Its content's taint: an upload is private and untrusted; a delivered
    /// file has the taint its run had when it delivered it.
    pub label: Label,
    /// A text file's content in lower case, for search, up to the length a
    /// page shows of it.
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Project {
    pub id: String,
    pub person: String,
    pub name: String,
    pub instructions: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Task {
    /// A chat turn: answered in the thread, not listed as a task.
    pub fn is_chat_turn(&self) -> bool {
        self.chat.as_ref().is_some_and(|c| c.turn)
    }
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

#[derive(Clone, Debug, Serialize)]
pub struct KnowledgeEntry {
    pub id: String,
    pub title: String,
    pub text: String,
    pub readers: Vec<Vec<String>>,
    pub integrity: String,
    pub source: Value,
    pub author: String,
    pub run: Option<String>,
    pub verification: Option<Value>,
    pub value: Option<Value>,
    pub endorsed: Option<Value>,
    pub at: DateTime<Utc>,
    /// The last event that changed it, so the index knows it is current.
    pub version: i64,
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
    /// The memory pass, or None for the run's own model.
    pub by: Option<String>,
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
    pub budget: Option<crate::config::Budget>,
    pub card: Option<String>,
    pub label: Label,
    pub title: Option<String>,
    /// What the person confirmed each run does, on the chat card.
    pub scope: Option<String>,
    /// Firings that could not start a task: when, and why.
    pub failures: Vec<(DateTime<Utc>, String)>,
    pub until: Option<DateTime<Utc>>,
    pub max_runs: Option<u32>,
    /// Runs it has started.
    pub started: u32,
    /// The latest firing that started nothing, because the last run was
    /// still open: when, which task, why.
    pub skipped: Option<(DateTime<Utc>, String, String)>,
    /// Why it stopped starting runs, until its person resumes it.
    pub paused: Option<String>,
    /// The task whose run it is paused on, waiting on a person.
    pub paused_on: Option<String>,
    /// From when failures count toward pausing it again.
    pub resumed: Option<DateTime<Utc>>,
}

/// A thread branched from another at one of its entries.
#[derive(Clone, Debug)]
pub struct Branch {
    pub person: String,
    pub from: String,
    pub upto: String,
    /// The entries up to `upto`, as they were shown then.
    pub entries: Vec<Value>,
    pub title: String,
    pub at: DateTime<Utc>,
}

/// What became of a schedule proposed in chat.
#[derive(Clone, Debug, PartialEq)]
pub enum CardState {
    Confirmed(String),
    Declined,
}

impl Schedule {
    /// Whether it may start no more runs: past its end, or out of runs.
    pub fn finished(&self, now: DateTime<Utc>) -> Option<String> {
        if let Some(m) = self.max_runs.filter(|m| self.started >= *m) {
            return Some(format!("it ran {m} time{}, as set", if m == 1 { "" } else { "s" }));
        }
        self.until.filter(|u| *u <= now).map(|u| format!("it reached its end, {}", u.to_rfc3339()))
    }

    /// The next time this schedule is due after its last run (or creation),
    /// unless it is paused or over.
    pub fn next_due(&self) -> Option<DateTime<Utc>> {
        if self.paused.is_some() || self.max_runs.is_some_and(|m| self.started >= m) {
            return None;
        }
        let next = self.next_time()?;
        self.until.is_none_or(|u| next < u).then_some(next)
    }

    fn next_time(&self) -> Option<DateTime<Utc>> {
        let after = self.last_fired.unwrap_or(self.created);
        if let Some(m) = self.every_minutes {
            return Some(after + chrono::Duration::minutes(m.max(1) as i64));
        }
        let (h, m) = self.daily_at.as_deref()?.split_once(':')?;
        let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
        next_daily(after, self.last_fired.is_some(), h, m, &chrono::Local)
    }
}

/// The next time a daily schedule at `h:m` (wall-clock time in `tz`) is due
/// after `after`, which is its last firing when `fired`, else its creation.
/// When clocks go back, the time comes twice: the schedule is due at the first
/// still ahead, once a day (made in between, at the second; never at both).
/// When clocks go forward past it, it is due as the clocks resume, an hour on.
/// The two times of a repeated hour are taken in time order, whatever order
/// the time zone gives them in.
fn next_daily<Tz: chrono::TimeZone>(after: DateTime<Utc>, fired: bool, h: u32, m: u32, tz: &Tz) -> Option<DateTime<Utc>> {
    use chrono::LocalResult;
    let local = after.with_timezone(tz);
    let mut day = local.date_naive();
    for _ in 0..3 {
        let at = day.and_hms_opt(h, m, 0)?;
        let t = match tz.from_local_datetime(&at) {
            LocalResult::Single(t) => t,
            // In time order: chrono's `Local` gives the later first.
            LocalResult::Ambiguous(x, y) => {
                let (first, second) = if x <= y { (x, y) } else { (y, x) };
                if first > local {
                    first
                } else if !fired {
                    second
                } else {
                    day = day.succ_opt()?;
                    continue;
                }
            }
            LocalResult::None => {
                // An hour on is never in a gap; take its earlier time if repeated.
                match tz.from_local_datetime(&(at + chrono::Duration::hours(1))) {
                    LocalResult::Single(t) => t,
                    LocalResult::Ambiguous(x, y) => x.min(y),
                    LocalResult::None => return None,
                }
            }
        };
        if t > local {
            return Some(t.with_timezone(&Utc));
        }
        day = day.succ_opt()?;
    }
    None
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Update {
    /// "updated", "rolled_back" or "failed".
    pub what: String,
    pub from: String,
    pub to: String,
    pub detail: String,
    pub at: DateTime<Utc>,
}

/// An item in the trash: out of every view until it is restored or removed.
#[derive(Clone, Debug)]
pub struct Trashed {
    pub person: String,
    pub title: String,
    pub at: DateTime<Utc>,
    held: Held,
}

/// What a trashed item took out of the state, to put back if restored. A
/// chat message or answer takes nothing: its thread leaves it out.
#[derive(Clone, Debug)]
enum Held {
    Task(Box<Task>, Option<Box<Run>>, Vec<Item>),
    Thread { id: String, turns: Vec<(Task, Option<Run>)>, branch: Option<Branch> },
    Entry { thread: String },
    File(Box<LibFile>),
    Project(Project),
    Memory(MemoryEntry),
    Knowledge(Box<KnowledgeEntry>),
}

impl Trashed {
    pub fn kind(&self) -> &'static str {
        match self.held {
            Held::Task(..) => "task",
            Held::Thread { .. } => "conversation",
            Held::Entry { .. } => "message",
            Held::File(_) => "file",
            Held::Project(_) => "project",
            Held::Memory(_) => "memory",
            Held::Knowledge(_) => "knowledge",
        }
    }

    /// The thread a trashed message or answer was in.
    pub fn thread(&self) -> Option<&str> {
        match &self.held {
            Held::Entry { thread } => Some(thread),
            _ => None,
        }
    }
}

/// Why an item cannot go to the trash.
#[derive(Debug, PartialEq)]
pub enum NotTrashable {
    /// No such item of the person's.
    NotFound,
    /// A task still running: cancel it first, so a crash can resume it (C3).
    Running,
}

#[derive(Default)]
pub struct State {
    /// Trashed items by id.
    pub trash: BTreeMap<String, Trashed>,
    /// Conversations removed from the trash: their tasks stay, the thread is gone.
    pub removed_threads: HashSet<String>,
    pub tasks: BTreeMap<String, Task>,
    pub runs: HashMap<String, Run>,
    pub items: BTreeMap<String, Item>,
    pub approvals: Vec<Approval>,
    pub policies: BTreeMap<String, String>,
    pub kill_switch: bool,
    /// Everyone who has ever signed in here. Someone not in it signs in the
    /// first time on the identity provider's own page, which can walk them
    /// through what their account still needs (a new password, two-factor).
    pub signed_in_before: HashSet<String>,
    /// The latest backup's restore check: file, whether it restored, detail, when.
    pub last_backup: Option<(String, bool, String, DateTime<Utc>)>,
    /// An appliance update in progress: from, to, and whether it is a rollback.
    pub updating: Option<(String, String, bool)>,
    pub last_update: Option<Update>,
    /// Releases rolled back from: not installed again by themselves.
    pub skipped_releases: std::collections::BTreeSet<String>,
    /// Apps the owner added from the app (config-file apps are not here).
    pub connectors: BTreeMap<String, crate::config::Connector>,
    /// Secrets the kill switch emptied from the vault, latest engagement
    /// only: what the owner signs in to or enters again.
    pub revoked: BTreeMap<String, Revoked>,
    /// Secrets the kill switch kept, sealed: only their provider can revoke them.
    pub sealed: Vec<String>,
    /// Groups and roles from the identity provider, per person, as of their
    /// latest sign-in.
    pub provider_rights: HashMap<String, (Option<Vec<String>>, Option<Vec<String>>)>,
    /// The identity provider account (`sub`) each person is bound to.
    pub subjects: HashMap<String, String>,
    /// Roles the owner set in Settings, by person.
    pub roles_set: HashMap<String, Vec<String>>,
    /// Whether people's data is private from the admin (by default it is not).
    pub data_private: bool,
    /// Each person's own settings, by person.
    pub preferences: HashMap<String, Value>,
    /// Entries people deleted from their threads, by thread.
    pub hidden: HashMap<String, std::collections::HashSet<String>>,
    /// Threads branched from another, by thread.
    pub branches: HashMap<String, Branch>,
    pub schedules: BTreeMap<String, Schedule>,
    /// Schedule cards from chat the person answered, by card id.
    pub cards: HashMap<String, CardState>,
    pub model: Option<(String, String)>,
    pub pins: HashMap<String, Pin>,
    /// Tools whose live definition differs from the pin, with the open item.
    pub changed: HashMap<String, (String, Pin)>,
    pub secrets: HashSet<String>,
    pub memory: Vec<MemoryEntry>,
    /// The company knowledge base; its searchable index is in Postgres.
    pub knowledge: HashMap<String, KnowledgeEntry>,
    /// Files attached to each task, by task id.
    pub attachments: HashMap<String, Vec<Value>>,
    /// Every person's library, by file id.
    pub files: BTreeMap<String, LibFile>,
    pub projects: BTreeMap<String, Project>,
    /// Items the person moved into a project (Some) or out of one (None).
    pub moved: HashMap<String, Option<String>>,
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

/// A text file's content in lower case for library search, up to as much as
/// a page shows of a file; other files are found by name.
fn search_text(media_type: &str, data_b64: &str) -> String {
    use base64::Engine;
    let textual = media_type.starts_with("text/") || ["application/json", "application/xml", "application/csv"].contains(&media_type);
    let Some(bytes) = textual.then(|| base64::engine::general_purpose::STANDARD.decode(data_b64).ok()).flatten() else { return String::new() };
    String::from_utf8_lossy(&bytes).chars().take(crate::compose::PREVIEW_CHARS).collect::<String>().to_lowercase()
}

impl State {
    /// The project an item is in: where the person moved it, else where it
    /// began. A task begins in the project it started in; a thread in its
    /// first message's; a delivered file in its task's; a file uploaded in
    /// chat in its thread's.
    pub fn project_of(&self, item: &str) -> Option<String> {
        if let Some(p) = self.moved.get(item) {
            return p.clone().filter(|p| self.projects.contains_key(p));
        }
        if let Some(t) = self.tasks.get(item) {
            return t.project.as_ref().map(|p| p.id.clone());
        }
        if let Some(f) = self.files.get(item) {
            if let Some(task) = f.run.as_ref().and_then(|r| self.runs.get(r)).map(|r| r.task.clone()) {
                return self.project_of(&task);
            }
            return f.thread.as_ref().and_then(|t| self.project_of(t));
        }
        self.tasks
            .values()
            .filter(|t| t.chat.as_ref().is_some_and(|c| c.thread == item && c.turn))
            .min_by_key(|t| t.created_at)
            .and_then(|t| t.project.as_ref().map(|p| p.id.clone()))
    }

    /// A schedule's run that is waiting on a person (an approval, a
    /// question, a review, a stop for its budget or an error): the task and
    /// why. Waiting for a time, an update or the kill switch is not.
    pub fn held_by_person(&self, schedule: &str) -> Option<(String, String)> {
        self.tasks.values().filter(|t| t.schedule.as_deref() == Some(schedule)).find_map(|t| {
            let r = self.runs.get(&t.run_id).filter(|r| r.ended_at.is_none() && r.status == Status::Suspended)?;
            let reason = r.reason.as_deref().unwrap_or("someone");
            (!["timer", "update", "kill_switch"].contains(&reason)).then(|| (t.task_id.clone(), format!("its last run is waiting on you ({reason})")))
        })
    }

    /// A schedule's run that has not ended, and why it is still open.
    pub fn open_run_of(&self, schedule: &str) -> Option<(String, String)> {
        self.tasks.values().filter(|t| t.schedule.as_deref() == Some(schedule)).find_map(|t| {
            let r = self.runs.get(&t.run_id).filter(|r| r.ended_at.is_none())?;
            let why = match (r.status == Status::Suspended, r.reason.as_deref()) {
                (true, Some("timer")) => "its last run is waiting for a time it set".to_string(),
                (true, Some(reason)) => format!("its last run is waiting on someone ({reason})"),
                _ => "its last run is still running".to_string(),
            };
            Some((t.task_id.clone(), why))
        })
    }

    /// How many of a schedule's latest firings in a row failed (since it was
    /// last resumed): runs that ended failed, and firings that started none.
    pub fn failures_in_a_row(&self, schedule: &str) -> u32 {
        let Some(sc) = self.schedules.get(schedule) else { return 0 };
        let since = sc.resumed.unwrap_or(sc.created);
        let mut ends: Vec<(DateTime<Utc>, bool)> = sc.failures.iter().filter(|(at, _)| *at >= since).map(|(at, _)| (*at, true)).collect();
        ends.extend(
            self.tasks
                .values()
                .filter(|t| t.schedule.as_deref() == Some(schedule) && t.created_at >= since)
                .filter_map(|t| self.runs.get(&t.run_id))
                .filter(|r| r.ended_at.is_some())
                .map(|r| (r.ended_at.unwrap_or_default(), r.status == Status::Failed)),
        );
        ends.sort_by(|a, b| b.0.cmp(&a.0));
        ends.iter().take_while(|(_, failed)| *failed).count() as u32
    }

    /// Whether `person` (`owner` or not) may put `item` in the trash now.
    pub fn may_trash(&self, person: &str, owner: bool, item: &str) -> Result<(), NotTrashable> {
        use NotTrashable::*;
        let ended = |run: &str| self.runs.get(run).is_none_or(|r| r.ended_at.is_some());
        let mine = match item.split_once(':') {
            Some(("m" | "a", task)) => {
                let t = self.tasks.get(task).filter(|t| t.person == person && t.is_chat_turn()).ok_or(NotFound)?;
                return if ended(&t.run_id) { Ok(()) } else { Err(Running) };
            }
            _ if item.starts_with("task_") => {
                let t = self.tasks.get(item).filter(|t| t.person == person && !t.is_chat_turn()).ok_or(NotFound)?;
                return if ended(&t.run_id) { Ok(()) } else { Err(Running) };
            }
            _ if item.starts_with("thread_") => {
                let turns: Vec<&Task> = self.tasks.values().filter(|t| t.person == person && t.chat.as_ref().is_some_and(|c| c.turn && c.thread == item)).collect();
                if turns.is_empty() && !self.branches.get(item).is_some_and(|b| b.person == person) {
                    return Err(NotFound);
                }
                return if turns.iter().all(|t| ended(&t.run_id)) { Ok(()) } else { Err(Running) };
            }
            _ => match () {
                _ if self.files.get(item).is_some_and(|f| f.person == person) => true,
                _ if self.projects.get(item).is_some_and(|p| p.person == person) => true,
                _ if self.memory.iter().any(|m| m.id == item && m.person == person) => {
                    // Deleting a memory deletes where it was written in its
                    // run, so the run must have ended (invariant 3).
                    let run = self.memory.iter().find(|m| m.id == item).map(|m| m.run.clone()).unwrap_or_default();
                    return if ended(&run) { Ok(()) } else { Err(Running) };
                }
                _ => self.knowledge.get(item).is_some_and(|k| {
                    owner || k.author == format!("person:{person}") || k.author.strip_prefix("agent:").and_then(|r| self.runs.get(r)).is_some_and(|r| r.person == person)
                }),
            },
        };
        if mine {
            Ok(())
        } else {
            Err(NotFound)
        }
    }

    /// Take an item out of every view, into the trash.
    fn take(&mut self, item: &str) -> Option<(String, Held)> {
        if let Some((_, task)) = item.split_once(':') {
            let t = self.tasks.get(task)?;
            let thread = t.chat.as_ref()?.thread.clone();
            let title = match item.starts_with("m:") {
                true => t.text.clone(),
                false => self.runs.get(&t.run_id).and_then(|r| r.summary.clone()).unwrap_or_default(),
            };
            return Some((title, Held::Entry { thread }));
        }
        let title_of = |t: &Task| t.title.clone().unwrap_or_else(|| t.text.clone());
        if item.starts_with("task_") {
            let t = self.tasks.remove(item)?;
            let r = self.runs.remove(&t.run_id).map(Box::new);
            let ids: Vec<String> = self.items.values().filter(|i| i.task.as_deref() == Some(item)).map(|i| i.id.clone()).collect();
            let items = ids.iter().filter_map(|id| self.items.remove(id)).collect();
            return Some((title_of(&t), Held::Task(Box::new(t), r, items)));
        }
        if item.starts_with("thread_") {
            let ids: Vec<String> = self.tasks.values().filter(|t| t.chat.as_ref().is_some_and(|c| c.turn && c.thread == item)).map(|t| t.task_id.clone()).collect();
            let mut turns: Vec<(Task, Option<Run>)> = ids.iter().filter_map(|id| self.tasks.remove(id)).map(|t| { let r = self.runs.remove(&t.run_id); (t, r) }).collect();
            turns.sort_by_key(|(t, _)| t.created_at);
            let branch = self.branches.remove(item);
            let title = turns.first().map(|(t, _)| t.text.clone()).or_else(|| branch.as_ref().map(|b| b.title.clone()))?;
            return Some((title, Held::Thread { id: item.into(), turns, branch }));
        }
        if let Some(f) = self.files.remove(item) {
            return Some((f.name.clone(), Held::File(Box::new(f))));
        }
        if let Some(p) = self.projects.remove(item) {
            return Some((p.name.clone(), Held::Project(p)));
        }
        if let Some(i) = self.memory.iter().position(|m| m.id == item) {
            let m = self.memory.remove(i);
            return Some((m.fact.clone(), Held::Memory(m)));
        }
        let k = self.knowledge.remove(item)?;
        Some((k.title.clone(), Held::Knowledge(Box::new(k))))
    }

    /// Which events hold a trashed item's own content. A task's or a
    /// conversation's runs keep what they made that is an item of its own:
    /// files they delivered, memories and playbook items they wrote.
    pub fn holding(&self, item: &str) -> Option<crate::log::Holding> {
        use crate::log::Holding;
        const OWN: [&str; 3] = ["output", "memory", "playbook"];
        let named = |kinds: Vec<&'static str>, field: &'static str, id: &str| Holding { kinds, field, id: id.into(), ..Default::default() };
        // What belongs to a task but carries no run: its verdict's note and
        // the person's answers to its cards.
        let theirs = |task: &str, items: &mut dyn Iterator<Item = &Item>| -> Vec<(&'static str, &'static str, String)> {
            let mut m = vec![("verdict", "task", task.to_string())];
            m.extend(items.map(|i| ("item_resolved", "id", i.id.clone())));
            m
        };
        let items_of = |task: &str| self.items.values().filter(|i| i.task.as_deref() == Some(task)).cloned().collect::<Vec<_>>();
        Some(match &self.trash.get(item)?.held {
            Held::Task(t, _, items) => Holding { runs: vec![t.run_id.clone()], except: OWN.to_vec(), matches: theirs(&t.task_id, &mut items.iter()), ..Default::default() },
            Held::Thread { id, turns, .. } => {
                let matches = turns.iter().flat_map(|(t, _)| theirs(&t.task_id, &mut items_of(&t.task_id).iter())).collect();
                Holding { runs: turns.iter().map(|(t, _)| t.run_id.clone()).collect(), except: OWN.to_vec(), matches, ..named(vec!["thread_branched"], "thread", id) }
            }
            Held::Entry { .. } => match item.split_once(':')? {
                ("m", task) => named(vec!["task_created"], "task", task),
                (_, task) => Holding {
                    runs: vec![self.tasks.get(task)?.run_id.clone()],
                    except: [&OWN[..], &["task_created"]].concat(),
                    matches: theirs(task, &mut items_of(task).iter()),
                    ..Default::default()
                },
            },
            Held::File(_) => named(vec!["file_uploaded", "output"], "id", item),
            Held::Project(_) => named(vec!["project_set"], "id", item),
            // A memory and where it was written: the call that wrote it, and
            // the reply that made the call (the run's model, or the memory
            // pass), found by the fact's text in that run.
            Held::Memory(m) => {
                let text = serde_json::to_string(&m.fact).unwrap_or_default();
                let text = text.trim_matches('"').to_string();
                let containing = ["intent", "tool_result", "model_reply", "memory_pass"].into_iter().map(|k| (k, m.run.clone(), text.clone())).filter(|_| !m.fact.trim().is_empty()).collect();
                Holding { containing, ..named(vec!["memory"], "id", item) }
            }
            Held::Knowledge(_) => named(vec!["knowledge_added"], "id", item),
        })
    }

    /// A trashed item back where it was.
    fn put_back(&mut self, held: Held) {
        match held {
            Held::Task(t, r, items) => {
                if let Some(r) = r {
                    self.runs.insert(r.id.clone(), *r);
                }
                for i in items {
                    self.items.insert(i.id.clone(), i);
                }
                self.tasks.insert(t.task_id.clone(), *t);
            }
            Held::Thread { id, turns, branch } => {
                for (t, r) in turns {
                    if let Some(r) = r {
                        self.runs.insert(r.id.clone(), r);
                    }
                    self.tasks.insert(t.task_id.clone(), t);
                }
                if let Some(b) = branch {
                    self.branches.insert(id, b);
                }
            }
            Held::Entry { .. } => {}
            Held::File(f) => {
                self.files.insert(f.id.clone(), *f);
            }
            Held::Project(p) => {
                self.projects.insert(p.id.clone(), p);
            }
            Held::Memory(m) => {
                let at = self.memory.partition_point(|x| x.at <= m.at);
                self.memory.insert(at, m);
            }
            Held::Knowledge(k) => {
                self.knowledge.insert(k.id.clone(), *k);
            }
        }
    }

    /// A person's file, or a delivered file of a task they may see.
    pub fn file_for(&self, person: &str, owner: bool, id: &str) -> Option<&LibFile> {
        self.files.get(id).filter(|f| f.person == person || (owner && f.run.is_some()))
    }

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
                self.revoked.remove(name);
            }
            Ev::PolicySet { layer, cedar } => {
                self.policies.insert(layer.clone(), cedar.clone());
                self.policy_history.push(PolicyVersion { layer: layer.clone(), seq: e.seq, at: e.at, by: e.actor.clone(), cedar: cedar.clone() });
                let n = self.policy_history.iter().filter(|p| &p.layer == layer).count();
                self.version_changes.push((e.seq, e.at, format!("policy:{layer}"), format!("v{n}")));
            }
            Ev::KillSwitch { engaged } => self.kill_switch = *engaged,
            Ev::BackupMade { .. } => {}
            Ev::UpdateStarted { from, to, rollback } => self.updating = Some((from.clone(), to.clone(), *rollback)),
            Ev::UpdateFinished { from, to } => {
                self.updating = None;
                self.last_update = Some(Update { what: "updated".into(), from: from.clone(), to: to.clone(), detail: String::new(), at: e.at });
            }
            Ev::UpdateRolledBack { from, to, reason } => {
                self.updating = None;
                self.skipped_releases.insert(from.clone());
                self.last_update = Some(Update { what: "rolled_back".into(), from: from.clone(), to: to.clone(), detail: reason.clone(), at: e.at });
            }
            Ev::UpdateFailed { to, reason } => {
                self.updating = None;
                self.last_update = Some(Update { what: "failed".into(), from: String::new(), to: to.clone(), detail: reason.clone(), at: e.at });
            }
            Ev::BackupChecked { file, ok, detail } => {
                self.last_backup = Some((file.clone(), *ok, detail.clone(), e.at));
            }
            Ev::ConnectorAdded { connector } => {
                self.connectors.insert(connector.name.clone(), connector.clone());
            }
            Ev::ConnectorRemoved { name } => {
                self.connectors.remove(name);
            }
            Ev::ToolClassified { connector, tool, risk } => {
                if let Some(c) = self.connectors.get_mut(connector) {
                    match risk {
                        Some(r) => {
                            c.tools.insert(tool.clone(), crate::config::ToolClass { risk: r.clone(), label: None });
                        }
                        None => {
                            c.tools.remove(tool);
                        }
                    }
                }
            }
            Ev::ContextReset { run, from, to, note, .. } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.cuts.push(Cut { from: *from, to: *to, note: note.clone() });
                    r.lost_since = *to;
                }
            }
            Ev::Escalated { run, adapter, model, .. } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.escalated = Some((adapter.clone(), model.clone()));
                    r.lost_since = r.replies.len() as u32;
                }
            }
            Ev::VaultSealed { revoking, kept } => {
                self.revoked = revoking.iter().map(|n| (n.clone(), Revoked::default())).collect();
                self.sealed = kept.clone();
                for n in revoking {
                    self.secrets.remove(n);
                }
            }
            Ev::SecretRevoked { name, adapter, at_provider, detail } => {
                self.revoked.insert(name.clone(), Revoked { adapter: Some(adapter.clone()), at_provider: Some(*at_provider), detail: Some(detail.clone()) });
            }
            Ev::SubjectBound { person, subject } => match subject {
                Some(sub) => {
                    self.subjects.insert(person.clone(), sub.clone());
                }
                None => {
                    self.subjects.remove(person);
                }
            },
            Ev::RightsFromProvider { person, groups, roles } => {
                self.provider_rights.insert(person.clone(), (groups.clone(), roles.clone()));
            }
            Ev::DataAccessSet { private } => self.data_private = *private,
            Ev::PersonDataRead { .. } => {}
            Ev::RolesSet { person, roles } => {
                self.roles_set.insert(person.clone(), roles.clone());
            }
            Ev::PreferencesSet { person, prefs } => {
                let p = self.preferences.entry(person.clone()).or_insert_with(|| serde_json::json!({}));
                if let (Some(p), Some(new)) = (p.as_object_mut(), prefs.as_object()) {
                    for (k, v) in new {
                        p.insert(k.clone(), v.clone());
                    }
                }
            }
            Ev::ChatHidden { thread, entries, .. } => {
                self.hidden.entry(thread.clone()).or_default().extend(entries.iter().cloned());
            }
            Ev::ThreadBranched { person, thread, from, upto, entries, title } => {
                self.branches.insert(thread.clone(), Branch { person: person.clone(), from: from.clone(), upto: upto.clone(), entries: entries.clone(), title: title.clone(), at: e.at });
            }
            Ev::SignedIn { person, .. } => {
                self.signed_in_before.insert(person.clone());
            }
            Ev::SignedOut { .. } => {}
            Ev::ScheduleSet { id, person, text, every_minutes, daily_at, budget, card, label, title, until, max_runs, scope } => {
                self.schedules.insert(
                    id.clone(),
                    Schedule {
                        id: id.clone(),
                        person: person.clone(),
                        text: text.clone(),
                        every_minutes: *every_minutes,
                        daily_at: daily_at.clone(),
                        created: e.at,
                        last_fired: None,
                        budget: budget.clone(),
                        card: card.clone(),
                        label: label.unwrap_or_default(),
                        title: title.clone(),
                        scope: scope.clone(),
                        failures: vec![],
                        until: *until,
                        max_runs: *max_runs,
                        started: 0,
                        skipped: None,
                        paused: None,
                        paused_on: None,
                        resumed: None,
                    },
                );
                if let Some(c) = card {
                    self.cards.insert(c.clone(), CardState::Confirmed(id.clone()));
                }
            }
            Ev::CardDeclined { card } => {
                self.cards.insert(card.clone(), CardState::Declined);
            }
            Ev::MemoryForgotten { id } => self.memory.retain(|m| &m.id != id),
            Ev::Trashed { item, person } => {
                if let Some((title, held)) = self.take(item) {
                    self.trash.insert(item.clone(), Trashed { person: person.clone(), title, at: e.at, held });
                }
            }
            Ev::Restored { item } => {
                if let Some(t) = self.trash.remove(item) {
                    self.put_back(t.held);
                }
            }
            Ev::Removed { item, .. } => match self.trash.remove(item).map(|t| t.held) {
                Some(Held::Entry { thread }) => {
                    self.hidden.entry(thread).or_default().insert(item.clone());
                }
                Some(Held::Thread { id, .. }) => {
                    self.removed_threads.insert(id);
                }
                _ => {}
            },
            Ev::KnowledgeAdded { id, title, text, readers, integrity, source, author, run, verification, value } => {
                self.knowledge.insert(
                    id.clone(),
                    KnowledgeEntry {
                        id: id.clone(),
                        title: title.clone(),
                        text: text.clone(),
                        readers: readers.clone(),
                        integrity: integrity.clone(),
                        source: source.clone(),
                        author: author.clone(),
                        run: run.clone(),
                        verification: verification.clone(),
                        value: value.clone(),
                        endorsed: None,
                        at: e.at,
                        version: e.seq,
                    },
                );
            }
            Ev::KnowledgeEndorsed { id, by, item, note } => {
                if let Some(k) = self.knowledge.get_mut(id) {
                    k.integrity = "company".into();
                    k.endorsed = Some(serde_json::json!({ "by": by, "item": item, "note": note, "at": e.at }));
                    k.version = e.seq;
                }
            }
            Ev::ScheduleRemoved { id } | Ev::ScheduleEnded { id, .. } => {
                self.schedules.remove(id);
            }
            Ev::ScheduleSkipped { id, task, reason } => {
                if let Some(s) = self.schedules.get_mut(id) {
                    s.last_fired = Some(e.at);
                    s.skipped = Some((e.at, task.clone(), reason.clone()));
                }
            }
            Ev::SchedulePaused { id, reason, task } => {
                if let Some(s) = self.schedules.get_mut(id) {
                    s.paused = Some(reason.clone());
                    s.paused_on = task.clone();
                }
            }
            Ev::ScheduleResumed { id } => {
                if let Some(s) = self.schedules.get_mut(id) {
                    s.paused = None;
                    // Failures count afresh only after a person resumes a
                    // schedule paused for them.
                    if s.paused_on.take().is_none() {
                        s.resumed = Some(e.at);
                    }
                    // The next run is due from now, not from long ago.
                    s.last_fired = Some(e.at);
                }
            }
            // A firing that failed still counts as fired: the next one is due as usual.
            Ev::ScheduleFailed { id, reason } => {
                if let Some(s) = self.schedules.get_mut(id) {
                    s.last_fired = Some(e.at);
                    s.failures.push((e.at, reason.clone()));
                }
            }
            Ev::ScheduleFired { id, task } => {
                // A run its schedule created was counted when it was created
                // (below), so a firing lost after it never lets one more run.
                let counted = self.tasks.get(task).is_some_and(|t| t.schedule.as_deref() == Some(id.as_str()));
                if let Some(s) = self.schedules.get_mut(id) {
                    s.last_fired = Some(e.at);
                    if !counted {
                        s.started += 1;
                    }
                    s.skipped = None;
                }
                if let Some(t) = self.tasks.get_mut(task) {
                    t.schedule = Some(id.clone());
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
            Ev::TaskCreated { task, run, person, text, config, attachments, experiment, chat, label, title, files, project, scope } => {
                // Playbook items the run starts with carry the taint of the
                // runs that wrote them (I2, M1), and a task from chat that of
                // the turn that started it.
                let pb = self.playbook_at(person, config.playbook).into_iter().fold(label.unwrap_or_default(), |l, i| l.join(i.label));
                let uploads = !attachments.is_empty() || !files.is_empty();
                // A schedule's run counts against it as it is created.
                if let Some(s) = e.actor.strip_prefix("schedule:").and_then(|id| self.schedules.get_mut(id)) {
                    s.last_fired = Some(e.at);
                    s.started += 1;
                    s.skipped = None;
                }
                if !attachments.is_empty() {
                    self.attachments.insert(task.clone(), attachments.clone());
                }
                self.tasks.insert(
                    task.clone(),
                    Task {
                        task_id: task.clone(),
                        run_id: run.clone(),
                        person: person.clone(),
                        text: text.clone(),
                        created_at: e.at,
                        chat: chat.clone(),
                        title: title.clone(),
                        // Known from the start, so its first step knows it is scheduled.
                        schedule: e.actor.strip_prefix("schedule:").map(String::from),
                        scope: scope.clone(),
                        files: files.clone(),
                        project: project.clone(),
                        verdict: None,
                    },
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
                        // Uploaded files are the person's private data, and
                        // content from elsewhere: data, not instructions.
                        untrusted: uploads || pb.untrusted,
                        private: uploads || pb.private,
                        // Privacy carried in from where the run started (a
                        // thread, a schedule's source) has no app to name:
                        // the unnamed source, as for a result without one,
                        // so readers and the audience rule still see it.
                        private_from: if uploads {
                            ["library".to_string()].into()
                        } else if pb.private {
                            [String::new()].into()
                        } else {
                            Default::default()
                        },
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
                        first_stop: None,
                        decisions: vec![],
                        suspensions: vec![],
                        tokens: Tokens::default(),
                        model_errors: 0,
                        checks_failed: 0,
                        waiting_item: None,
                        cuts: vec![],
                        lost_since: 0,
                        escalated: None,
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
            Ev::VerifierReply { run, usage, .. } | Ev::MemoryPass { run, usage, .. } | Ev::SpokenWritten { run, usage, .. } | Ev::KnowledgeExtraction { run, usage, .. } => {
                if let Some(r) = self.runs.get_mut(run) {
                    if let Ev::SpokenWritten { script, .. } = &e.ev {
                        r.spoken = Some(script.clone());
                    }
                    r.model_calls += 1;
                    r.usd += usage.cost_usd;
                    r.tokens.input += usage.input_tokens;
                    r.tokens.output += usage.output_tokens;
                    r.tokens.cache_read += usage.cache_read_tokens;
                    r.tokens.cache_write += usage.cache_write_tokens;
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
            Ev::Verdict { task, person, verdict, note } => {
                let trial = self.tasks.get(task).and_then(|t| self.runs.get(&t.run_id)).is_some_and(|r| r.experiment.is_some());
                if let Some(t) = self.tasks.get_mut(task).filter(|t| &t.person == person || trial) {
                    t.verdict = Some(Verdict { verdict: verdict.clone(), note: note.clone(), at: e.at });
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
                    r.first_stop.get_or_insert(e.at);
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
                    r.first_stop.get_or_insert(e.at);
                }
            }
            Ev::Failed { run, reason, .. } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.status = Status::Failed;
                    r.summary = Some(reason.clone());
                    r.reason = None;
                    r.ended_at = Some(e.at);
                    r.first_stop.get_or_insert(e.at);
                }
            }
            Ev::Cancelled { run } => {
                if let Some(r) = self.runs.get_mut(run) {
                    r.status = Status::Cancelled;
                    r.reason = None;
                    r.ended_at = Some(e.at);
                    r.first_stop.get_or_insert(e.at);
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
                    // A stopped run's notice put away: the run waits on no
                    // card now, so its own Resume (or Cancel) is what is left.
                    if decision == "dismiss" {
                        if let Some(r) = i.run.as_ref().and_then(|r| self.runs.get_mut(r)).filter(|r| r.waiting_item.as_deref() == Some(id.as_str())) {
                            r.waiting_item = None;
                        }
                    }
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
                    let f = LibFile {
                        id: id.clone(),
                        person: r.person.clone(),
                        name: name.clone(),
                        media_type: media_type.clone(),
                        size: data_b64.len() * 3 / 4,
                        at: e.at,
                        data_b64: None,
                        run: Some(run.clone()),
                        thread: None,
                        label: Label { untrusted: r.untrusted, private: r.private },
                        text: search_text(media_type, data_b64),
                    };
                    self.files.insert(id.clone(), f);
                }
            }
            Ev::FileUploaded { id, person, name, media_type, data_b64, thread } => {
                let f = LibFile {
                    id: id.clone(),
                    person: person.clone(),
                    name: name.clone(),
                    media_type: media_type.clone(),
                    size: data_b64.len() * 3 / 4,
                    at: e.at,
                    data_b64: Some(data_b64.clone()),
                    run: None,
                    thread: thread.clone(),
                    label: Label { untrusted: true, private: true },
                    text: search_text(media_type, data_b64),
                };
                self.files.insert(id.clone(), f);
            }
            Ev::ProjectSet { id, person, name, instructions } => {
                let created_at = self.projects.get(id).map_or(e.at, |p| p.created_at);
                self.projects.insert(id.clone(), Project { id: id.clone(), person: person.clone(), name: name.clone(), instructions: instructions.clone(), created_at });
            }
            Ev::Moved { item, project } => {
                self.moved.insert(item.clone(), project.clone());
            }
            Ev::AdapterVerified { adapter_id, version, model, passed, report, .. } => {
                self.conformance.insert(adapter_id.clone(), (version.clone(), model.clone(), *passed, report.clone()));
            }
            Ev::Memory { id, person, run, fact, topic, label, sources, by } => self.memory.push(MemoryEntry {
                id: id.clone(),
                person: person.clone(),
                run: run.clone(),
                fact: fact.clone(),
                topic: topic.clone(),
                label: *label,
                sources: sources.clone(),
                by: by.clone(),
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

#[cfg(test)]
mod tests {
    use super::State;
    use crate::log::Event;
    use serde_json::json;

    /// A daily time in Los Angeles across both of its clock changes (owner's
    /// CI, 2026-11-01: schedules made in the repeated hour did not fire).
    #[test]
    fn a_daily_schedule_is_due_once_a_day_across_clock_changes() {
        use chrono::{TimeZone, Utc};
        let la = chrono_tz::America::Los_Angeles;
        let utc = |s: &str| chrono::DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        // Clocks go back at 02:00 PDT on 2026-11-01; 01:00-02:00 comes twice.
        // Made in the first 01:14 (PDT), due at the first 01:16.
        assert_eq!(super::next_daily(utc("2026-11-01T08:14:00Z"), false, 1, 16, &la), Some(utc("2026-11-01T08:16:00Z")));
        // Made in the second 01:14 (PST), due at the second 01:16, not tomorrow.
        assert_eq!(super::next_daily(utc("2026-11-01T09:14:00Z"), false, 1, 16, &la), Some(utc("2026-11-01T09:16:00Z")));
        // Having fired at the first 01:16, not again at the second: tomorrow.
        assert_eq!(super::next_daily(utc("2026-11-01T08:16:00Z"), true, 1, 16, &la), Some(utc("2026-11-02T09:16:00Z")));
        // Fired the day before: the first 01:16.
        assert_eq!(super::next_daily(utc("2026-10-31T08:16:00Z"), true, 1, 16, &la), Some(utc("2026-11-01T08:16:00Z")));
        // Clocks go forward at 02:00 PST on 2027-03-14; 02:30 does not happen.
        // Due as they resume, at 03:30 PDT, and on the next day at 02:30 again.
        assert_eq!(super::next_daily(utc("2027-03-14T09:00:00Z"), false, 2, 30, &la), Some(utc("2027-03-14T10:30:00Z")));
        assert_eq!(super::next_daily(utc("2027-03-14T10:30:00Z"), true, 2, 30, &la), Some(utc("2027-03-15T09:30:00Z")));
        // Whatever order the time zone gives the two in (chrono's `Local`
        // gives the later first; on the owner's build box, the first 01:31
        // came out an hour late).
        let rev = Reversed(la);
        assert_eq!(super::next_daily(utc("2026-11-01T08:29:00Z"), false, 1, 31, &rev), Some(utc("2026-11-01T08:31:00Z")));
        assert_eq!(super::next_daily(utc("2026-11-01T09:29:00Z"), false, 1, 31, &rev), Some(utc("2026-11-01T09:31:00Z")));
        assert_eq!(super::next_daily(utc("2026-11-01T08:31:00Z"), true, 1, 31, &rev), Some(utc("2026-11-02T09:31:00Z")));
        // An ordinary day.
        assert_eq!(super::next_daily(la.with_ymd_and_hms(2026, 6, 1, 9, 0, 0).unwrap().with_timezone(&Utc), false, 8, 0, &la), Some(utc("2026-06-02T15:00:00Z")));
    }

    /// A time zone that gives a repeated time's two instants latest first,
    /// as chrono's `Local` does.
    #[derive(Clone, Copy)]
    struct Reversed(chrono_tz::Tz);
    impl chrono::TimeZone for Reversed {
        type Offset = <chrono_tz::Tz as chrono::TimeZone>::Offset;
        fn from_offset(o: &Self::Offset) -> Self {
            Reversed(chrono_tz::Tz::from_offset(o))
        }
        fn offset_from_local_date(&self, d: &chrono::NaiveDate) -> chrono::LocalResult<Self::Offset> {
            self.0.offset_from_local_date(d)
        }
        fn offset_from_local_datetime(&self, d: &chrono::NaiveDateTime) -> chrono::LocalResult<Self::Offset> {
            match self.0.offset_from_local_datetime(d) {
                chrono::LocalResult::Ambiguous(a, b) => chrono::LocalResult::Ambiguous(b, a),
                r => r,
            }
        }
        fn offset_from_utc_date(&self, d: &chrono::NaiveDate) -> Self::Offset {
            self.0.offset_from_utc_date(d)
        }
        fn offset_from_utc_datetime(&self, d: &chrono::NaiveDateTime) -> Self::Offset {
            self.0.offset_from_utc_datetime(d)
        }
    }

    fn ev(seq: i64, actor: &str, ev: serde_json::Value) -> Event {
        Event { seq, at: chrono::Utc::now(), actor: actor.into(), ev: serde_json::from_value(ev).unwrap() }
    }

    /// A schedule's run counts against its limit as it is created, so a
    /// crash before the firing is recorded never lets it run once more.
    #[test]
    fn a_scheduled_run_counts_when_it_is_created() {
        let mut s = State::default();
        s.apply(&ev(1, "person:alice", json!({ "kind": "schedule_set", "id": "s1", "person": "alice", "text": "Say hi.", "every_minutes": 60, "max_runs": 1 })));
        let config = json!({ "adapter": "a", "model": "m", "max_retries": 0, "max_tokens": 1, "budget": { "max_model_calls": 1, "max_actions": 1, "max_usd": 1.0 }, "verifier": false, "instructions": "", "harness": "" });
        s.apply(&ev(2, "schedule:s1", json!({ "kind": "task_created", "task": "t1", "run": "r1", "person": "alice", "text": "Say hi.", "config": config })));
        // The firing itself was lost: the schedule is still out of runs.
        assert_eq!(s.schedules["s1"].started, 1);
        assert!(s.schedules["s1"].next_due().is_none(), "a lost firing let the schedule run again");
        // And when the firing is recorded, it is not counted twice.
        s.apply(&ev(3, "kernel", json!({ "kind": "schedule_fired", "id": "s1", "task": "t1" })));
        assert_eq!(s.schedules["s1"].started, 1);
    }

    /// Dismissing the notice a stopped run waits on leaves the run waiting on
    /// no card, so the task's own Resume is not refused for it.
    #[test]
    fn a_dismissed_stop_notice_frees_the_run() {
        let mut s = State::default();
        let config = json!({ "adapter": "a", "model": "m", "max_retries": 0, "max_tokens": 1, "budget": { "max_model_calls": 1, "max_actions": 1, "max_usd": 1.0 }, "verifier": false, "instructions": "", "harness": "" });
        s.apply(&ev(1, "person:alice", json!({ "kind": "task_created", "task": "t1", "run": "r1", "person": "alice", "text": "Say hi.", "config": config })));
        s.apply(&ev(2, "kernel", json!({ "kind": "item", "id": "i1", "item_kind": "notice", "task": "t1", "run": "r1", "person": "alice", "summary": "Stopped", "details": { "reason": "budget" } })));
        s.apply(&ev(3, "kernel", json!({ "kind": "suspended", "run": "r1", "reason": "budget", "item": "i1" })));
        assert_eq!(s.runs["r1"].waiting_item.as_deref(), Some("i1"));
        s.apply(&ev(4, "person:alice", json!({ "kind": "item_resolved", "id": "i1", "decision": "dismiss" })));
        assert_eq!(s.runs["r1"].waiting_item, None, "the run still waits on a card put away");
        assert_eq!(s.runs["r1"].reason.as_deref(), Some("budget"), "dismissing it resumed nothing");
    }
}
