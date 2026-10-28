//! Every kind of event the log holds. The body of each row is one `Ev`.
//! Secrets never appear here: a secret write records only its name.

use crate::tools::Label;
use magnus_adapter_proto::{Block, ErrorClass, Usage};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A run's pinned configuration (K9), recorded at its start.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RunConfig {
    pub adapter: String,
    pub model: String,
    pub max_retries: u32,
    pub max_tokens: u32,
    pub budget: crate::config::Budget,
    pub verifier: bool,
    pub instructions: String,
    pub harness: String,
    /// The playbook version the run started with (I3).
    #[serde(default)]
    pub playbook: u64,
}

/// A task started as one trial of an experiment case (V2).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ExperimentTag {
    pub id: String,
    pub case: String,
    pub trial: u32,
}

/// How to rebuild exactly what one model call was sent (K5): the run's
/// events up to `through_seq`, with these instructions and tools.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ContextRef {
    pub through_seq: i64,
    pub instructions: String,
    pub tools_hash: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Ev {
    KernelBoot {
        version: String,
        pid: u32,
        #[serde(default)]
        instructions: String,
    },
    SecretSet {
        name: String,
    },
    PolicySet {
        layer: String,
        cedar: String,
    },
    KillSwitch {
        engaged: bool,
    },
    /// A person signed in from an app. The token itself is never logged.
    SignedIn {
        person: String,
        session: String,
        client: String,
    },
    SignedOut {
        person: String,
        session: String,
    },
    /// A recurring task a person set up (or accepted from the agent's proposal).
    ScheduleSet {
        id: String,
        person: String,
        text: String,
        /// Minutes between runs, or None with `daily_at`.
        #[serde(default)]
        every_minutes: Option<u32>,
        /// Local time "HH:MM" for a daily run.
        #[serde(default)]
        daily_at: Option<String>,
    },
    ScheduleRemoved {
        id: String,
    },
    ScheduleFired {
        id: String,
        task: String,
    },
    /// The model new tasks run on, chosen by the owner.
    ModelSet {
        adapter: String,
        model: String,
    },
    /// A tool definition approved for use, pinned by hash (B10).
    ToolPinned {
        tool: String,
        hash: String,
        description: String,
        input_schema: Value,
        /// Whether the owner has reviewed this definition's text. Unreviewed
        /// descriptions are third-party text and never reach the model.
        #[serde(default)]
        approved: bool,
    },
    /// A tool's definition no longer matches its pin; it is disabled.
    ToolChanged {
        tool: String,
        hash: String,
        description: String,
        input_schema: Value,
        item: String,
    },
    TaskCreated {
        task: String,
        run: String,
        person: String,
        text: String,
        #[serde(default)]
        attachments: Vec<Value>,
        config: RunConfig,
        #[serde(default)]
        experiment: Option<ExperimentTag>,
    },
    ModelCall {
        run: String,
        step: u32,
        attempt: u32,
        context: ContextRef,
    },
    ModelReply {
        run: String,
        step: u32,
        blocks: Vec<Block>,
        #[serde(default)]
        state: Option<Value>,
        usage: Usage,
        served_model: String,
        stop: String,
    },
    ModelError {
        run: String,
        step: u32,
        attempt: u32,
        class: ErrorClass,
        message: String,
    },
    /// Text the loop shows the model after a step (a malformed reply, a
    /// verification failure without a tool call to answer).
    Notice {
        run: String,
        step: u32,
        text: String,
    },
    /// Recorded before an external call (intent first).
    Intent {
        run: String,
        call_id: String,
        step_id: String,
        key: String,
        tool: String,
        input: Value,
        issuer: String,
    },
    /// A boundary decision, with the stage and rule that produced it (E4).
    Decision {
        run: String,
        call_id: String,
        tool: String,
        outcome: String,
        stage: String,
        reason: String,
        #[serde(default)]
        rule: Option<String>,
    },
    /// The result of a tool call as the model will see it, already redacted.
    ToolResult {
        run: String,
        call_id: String,
        #[serde(default)]
        key: Option<String>,
        status: String,
        text: String,
        is_error: bool,
        label: Label,
        #[serde(default)]
        data: Option<Value>,
        /// The apps the content came from, for the audience rule.
        #[serde(default)]
        sources: Vec<String>,
    },
    Contract {
        run: String,
        version: u32,
        contract: Value,
    },
    /// The agent's plan (K36): ordered units, each with an id, a goal and
    /// when it is done. A revision is a new version.
    Plan {
        run: String,
        version: u32,
        plan: Value,
    },
    Verification {
        run: String,
        criterion: String,
        passed: bool,
        evidence: Value,
        by: String,
    },
    Suspended {
        run: String,
        reason: String,
        #[serde(default)]
        item: Option<String>,
    },
    Resumed {
        run: String,
    },
    Completed {
        run: String,
        summary: String,
        /// Fixed components the agent composed to show the outcome (U8).
        #[serde(default)]
        view: Option<Value>,
        /// The outcome written for listening, read aloud on request (U8).
        #[serde(default)]
        spoken: Option<String>,
    },
    Failed {
        run: String,
        reason: String,
        by: String,
    },
    Cancelled {
        run: String,
    },
    Item {
        id: String,
        item_kind: String,
        task: Option<String>,
        run: Option<String>,
        /// The person the run acts for, or the owner for system items.
        person: String,
        summary: String,
        details: Value,
    },
    ItemResolved {
        id: String,
        decision: String,
        #[serde(default)]
        scope: Option<String>,
        #[serde(default)]
        text: Option<String>,
    },
    Approval {
        id: String,
        item: String,
        person: String,
        task: String,
        tool: String,
        #[serde(default)]
        input_hash: Option<String>,
        scope: String,
        #[serde(default)]
        until: Option<chrono::DateTime<chrono::Utc>>,
        #[serde(default)]
        reason: Option<String>,
    },
    ApprovalUsed {
        id: String,
        run: String,
        call_id: String,
    },
    Wait {
        run: String,
        until: chrono::DateTime<chrono::Utc>,
    },
    BudgetExtended {
        run: String,
        budget: crate::config::Budget,
    },
    /// A file the agent delivered to the person from its workspace (K19).
    Output {
        id: String,
        run: String,
        name: String,
        media_type: String,
        data_b64: String,
    },
    /// A conformance suite run for an adapter version.
    AdapterVerified {
        adapter_id: String,
        adapter: String,
        version: String,
        model: String,
        passed: bool,
        report: Value,
    },
    /// One itemized change to the agent's playbook (I2), made through the
    /// boundary like a memory write and labeled with the writing run's taint.
    Playbook {
        item: String,
        op: String,
        person: String,
        run: String,
        #[serde(default)]
        text: Option<String>,
        /// The failure the item addresses.
        #[serde(default)]
        addresses: Option<String>,
        label: Label,
    },
    /// A grader's verdict on a run, recorded by the evaluation runner (V2).
    Graded {
        run: String,
        grader: String,
        passed: bool,
        #[serde(default)]
        score: Option<f64>,
        #[serde(default)]
        details: Value,
    },
    Memory {
        id: String,
        person: String,
        run: String,
        fact: String,
        #[serde(default)]
        topic: Option<String>,
        /// The writing run's taint; reading the entry back carries it (M1).
        label: Label,
        #[serde(default)]
        sources: Vec<String>,
    },
}

impl Ev {
    pub fn run(&self) -> Option<&str> {
        use Ev::*;
        match self {
            TaskCreated { run, .. }
            | ModelCall { run, .. }
            | ModelReply { run, .. }
            | ModelError { run, .. }
            | Notice { run, .. }
            | Intent { run, .. }
            | Decision { run, .. }
            | ToolResult { run, .. }
            | Contract { run, .. }
            | Plan { run, .. }
            | Playbook { run, .. }
            | Graded { run, .. }
            | Verification { run, .. }
            | Suspended { run, .. }
            | Resumed { run }
            | Completed { run, .. }
            | Failed { run, .. }
            | Cancelled { run }
            | ApprovalUsed { run, .. }
            | Wait { run, .. }
            | BudgetExtended { run, .. }
            | Memory { run, .. }
            | Output { run, .. } => Some(run),
            Item { run, .. } => run.as_deref(),
            _ => None,
        }
    }
}
