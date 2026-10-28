//! Tool vocabulary: risk classes, inbound labels, audiences, and the built-in
//! tools every run is offered. App tools come from MCP servers, named
//! `<app>__<tool>`, and are classified by the owner in the config.

use magnus_adapter_proto::ToolSpec;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Read,
    /// Changes only the run's own workspace, which has no network (K42).
    Sandbox,
    ReversibleWrite,
    Irreversible,
    Financial,
    Prohibited,
}

impl Risk {
    pub fn parse(s: &str) -> Option<Risk> {
        Some(match s {
            "read" => Risk::Read,
            "sandbox" => Risk::Sandbox,
            "reversible_write" => Risk::ReversibleWrite,
            "irreversible" => Risk::Irreversible,
            "financial" => Risk::Financial,
            "prohibited" => Risk::Prohibited,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Risk::Read => "read",
            Risk::Sandbox => "sandbox",
            Risk::ReversibleWrite => "reversible_write",
            Risk::Irreversible => "irreversible",
            Risk::Financial => "financial",
            Risk::Prohibited => "prohibited",
        }
    }
    /// State change or external communication, for the Rule of Two (K34).
    pub fn changes_state(self) -> bool {
        !matches!(self, Risk::Read | Risk::Sandbox)
    }
}

/// How content is labeled on the way in (B3). A source can be both private
/// and untrusted: a mailbox holds the person's private mail, written by others.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Label {
    pub untrusted: bool,
    pub private: bool,
}

impl Label {
    pub const PRINCIPAL: Label = Label { untrusted: false, private: false };
    pub const PRIVATE: Label = Label { untrusted: false, private: true };
    pub const UNTRUSTED: Label = Label { untrusted: true, private: false };

    /// `principal`, `private`, `untrusted`, or a combination like `private+untrusted`.
    pub fn parse(s: &str) -> Option<Label> {
        let mut l = Label::PRINCIPAL;
        for part in s.split(['+', ',', ' ']).filter(|p| !p.is_empty()) {
            match part {
                "principal" => {}
                "private" => l.private = true,
                "untrusted" => l.untrusted = true,
                _ => return None,
            }
        }
        Some(l)
    }

    pub fn join(self, o: Label) -> Label {
        Label { untrusted: self.untrusted || o.untrusted, private: self.private || o.private }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Audience {
    Private,
    Internal,
    Public,
}

impl Audience {
    pub fn parse(s: &str) -> Option<Audience> {
        Some(match s {
            "private" => Audience::Private,
            "internal" => Audience::Internal,
            "public" => Audience::Public,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Audience::Private => "private",
            Audience::Internal => "internal",
            Audience::Public => "public",
        }
    }
}

/// Run records the loop writes to the log itself, outside execute() (K21).
pub const RECORDS: &[&str] = &["task_contract", "task_plan", "task_complete", "task_fail", "ask_principal", "propose_policy", "propose_schedule", "wait_until", "notify"];

/// Built-in actions that pass the boundary like app tools.
pub struct Builtin {
    pub name: &'static str,
    pub app: &'static str,
    pub risk: Risk,
    pub label: Label,
    pub audience: Audience,
}

pub const BUILTINS: &[Builtin] = &[
    Builtin { name: "sandbox_exec", app: "sandbox", risk: Risk::Sandbox, label: Label::PRINCIPAL, audience: Audience::Private },
    Builtin { name: "sandbox_write_file", app: "sandbox", risk: Risk::Sandbox, label: Label::PRINCIPAL, audience: Audience::Private },
    Builtin { name: "sandbox_read_file", app: "sandbox", risk: Risk::Sandbox, label: Label::PRINCIPAL, audience: Audience::Private },
    Builtin { name: "web_fetch", app: "web", risk: Risk::Read, label: Label::UNTRUSTED, audience: Audience::Public },
    Builtin { name: "web_search", app: "web", risk: Risk::Read, label: Label::UNTRUSTED, audience: Audience::Public },
    Builtin { name: "browser_navigate", app: "browser", risk: Risk::Read, label: Label::UNTRUSTED, audience: Audience::Public },
    Builtin { name: "browser_snapshot", app: "browser", risk: Risk::Read, label: Label::UNTRUSTED, audience: Audience::Public },
    Builtin { name: "browser_screenshot", app: "browser", risk: Risk::Read, label: Label::UNTRUSTED, audience: Audience::Public },
    Builtin { name: "browser_back", app: "browser", risk: Risk::Read, label: Label::UNTRUSTED, audience: Audience::Public },
    Builtin { name: "browser_click", app: "browser", risk: Risk::ReversibleWrite, label: Label::UNTRUSTED, audience: Audience::Public },
    Builtin { name: "browser_type", app: "browser", risk: Risk::ReversibleWrite, label: Label::UNTRUSTED, audience: Audience::Public },
    Builtin { name: "browser_select", app: "browser", risk: Risk::ReversibleWrite, label: Label::UNTRUSTED, audience: Audience::Public },
    Builtin { name: "deliver_file", app: "sandbox", risk: Risk::Sandbox, label: Label::PRINCIPAL, audience: Audience::Private },
    Builtin { name: "computer_screenshot", app: "sandbox", risk: Risk::Sandbox, label: Label::PRINCIPAL, audience: Audience::Private },
    Builtin { name: "computer_click", app: "sandbox", risk: Risk::Sandbox, label: Label::PRINCIPAL, audience: Audience::Private },
    Builtin { name: "computer_type", app: "sandbox", risk: Risk::Sandbox, label: Label::PRINCIPAL, audience: Audience::Private },
    Builtin { name: "computer_key", app: "sandbox", risk: Risk::Sandbox, label: Label::PRINCIPAL, audience: Audience::Private },
    Builtin { name: "computer_scroll", app: "sandbox", risk: Risk::Sandbox, label: Label::PRINCIPAL, audience: Audience::Private },
    Builtin { name: "memory_write", app: "memory", risk: Risk::ReversibleWrite, label: Label::PRINCIPAL, audience: Audience::Private },
    Builtin { name: "memory_search", app: "memory", risk: Risk::Read, label: Label::PRINCIPAL, audience: Audience::Private },
    Builtin { name: "playbook_update", app: "playbook", risk: Risk::ReversibleWrite, label: Label::PRINCIPAL, audience: Audience::Private },
    // Chat only: start a task for the person, or show them a schedule to
    // confirm. Neither leaves the appliance; the task carries the turn's taint.
    Builtin { name: "create_task", app: "chat", risk: Risk::ReversibleWrite, label: Label::PRINCIPAL, audience: Audience::Private },
    Builtin { name: "create_schedule", app: "chat", risk: Risk::Read, label: Label::PRINCIPAL, audience: Audience::Private },
];

pub fn builtin(name: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.name == name)
}

fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required })
}

/// Whether a contract check may call this tool: anything that only reads, and
/// of the sandbox tools only running a command and reading a file, which a
/// check does in a VM of its own.
pub fn checkable(name: &str, risk: Risk) -> bool {
    risk == Risk::Read || (risk == Risk::Sandbox && matches!(name, "sandbox_exec" | "sandbox_read_file"))
}

/// The tools a chat turn may call besides reads: start a task, or propose a
/// schedule the person confirms in the thread.
pub fn chat_specs() -> Vec<ToolSpec> {
    let desc: Value = serde_json::from_str(include_str!("../instructions/tools.json")).expect("tools.json");
    let d = |n: &str| desc[n].as_str().unwrap_or_default().to_string();
    let budget = json!({ "type": "object", "description": d("create_schedule.budget"), "properties": {
        "max_usd": { "type": "number" }, "max_model_calls": { "type": "integer" }, "max_actions": { "type": "integer" }
    } });
    vec![
        ToolSpec { name: "create_task".into(), description: d("create_task"), input_schema: obj(json!({ "text": { "type": "string" } }), &["text"]) },
        ToolSpec {
            name: "create_schedule".into(),
            description: d("create_schedule"),
            input_schema: obj(
                json!({ "text": { "type": "string" }, "every_minutes": { "type": "integer" }, "daily_at": { "type": "string" }, "scope": { "type": "string" }, "budget": budget }),
                &["text", "scope"],
            ),
        },
    ]
}

/// Specs for run records and built-in actions. Descriptions are part of the
/// versioned instructions (X11): they live in `instructions/tools.json`.
pub fn builtin_specs(sandbox: bool, search: bool, browser: bool) -> Vec<ToolSpec> {
    let desc: Value = serde_json::from_str(include_str!("../instructions/tools.json")).expect("tools.json");
    let d = |n: &str| desc[n].as_str().unwrap_or_default().to_string();
    let criterion = json!({
        "type": "object",
        "properties": {
            "id": { "type": "string" },
            "description": { "type": "string" },
            "check": {
                "type": "object",
                "properties": {
                    "tool": { "type": "string" },
                    "input": { "type": "object" },
                    "expect": { "type": "array", "items": { "type": "object", "properties": {
                        "pointer": { "type": "string" }, "equals": {}, "contains": { "type": "string" }
                    }, "required": ["pointer"] } }
                },
                "required": ["tool", "input", "expect"]
            }
        },
        "required": ["id", "description"]
    });
    let mut v = vec![
        ToolSpec {
            name: "task_contract".into(),
            description: d("task_contract"),
            input_schema: obj(
                json!({
                    "goal": { "type": "string" },
                    "assumptions": { "type": "array", "items": { "type": "string" } },
                    "criteria": { "type": "array", "items": criterion, "minItems": 1 },
                    "irreversible_actions": { "type": "array", "items": { "type": "string" } }
                }),
                &["goal", "criteria"],
            ),
        },
        ToolSpec {
            name: "task_plan".into(),
            description: d("task_plan"),
            input_schema: obj(
                json!({ "units": { "type": "array", "minItems": 1, "items": { "type": "object", "properties": {
                    "id": { "type": "string" }, "goal": { "type": "string" }, "done_when": { "type": "string" },
                    "status": { "type": "string", "enum": ["pending", "active", "done", "dropped"] }
                }, "required": ["id", "goal", "done_when"] } } }),
                &["units"],
            ),
        },
        ToolSpec {
            name: "task_complete".into(),
            description: d("task_complete"),
            input_schema: obj(
                json!({
                    "summary": { "type": "string" },
                    // Every component field is declared: a model that keeps to
                    // the schema cannot otherwise fill one in.
                    "view": { "type": "array", "minItems": 1, "description": d("task_complete.view"), "items": { "type": "object", "properties": {
                        "type": { "type": "string", "enum": crate::compose::KINDS },
                        "text": { "type": "string", "description": "heading, text" },
                        "items": { "type": "array", "items": { "type": "string" }, "description": "list" },
                        "columns": { "type": "array", "items": { "type": "string" }, "description": "table" },
                        "rows": { "type": "array", "items": { "type": "array", "items": { "type": ["string", "number"] } }, "description": "table: one list of cells per row, as many as columns" },
                        "label": { "type": "string", "description": "stat" },
                        "value": { "type": ["string", "number"], "description": "stat" },
                        "unit": { "type": "string", "description": "stat, bar_chart" },
                        "labels": { "type": "array", "items": { "type": "string" }, "description": "bar_chart, line_chart" },
                        "values": { "type": "array", "items": { "type": "number" }, "description": "bar_chart: one per label" },
                        "series": { "type": "array", "items": { "type": "object", "properties": { "name": { "type": "string" }, "values": { "type": "array", "items": { "type": "number" } } }, "required": ["name", "values"] }, "description": "line_chart: 1 to 6, one value per label each" },
                        "file": { "type": "string", "description": "image: the name of a file you delivered" },
                        "title": { "type": "string" },
                        "caption": { "type": "string" }
                    }, "required": ["type"] } },
                    "spoken": { "type": "string", "description": d("task_complete.spoken") }
                }),
                &["summary"],
            ),
        },
        ToolSpec {
            name: "task_fail".into(),
            description: d("task_fail"),
            input_schema: obj(json!({ "reason": { "type": "string" } }), &["reason"]),
        },
        ToolSpec {
            name: "ask_principal".into(),
            description: d("ask_principal"),
            input_schema: obj(
                json!({ "question": { "type": "string" }, "options": { "type": "array", "items": { "type": "string" } } }),
                &["question"],
            ),
        },
        ToolSpec {
            name: "propose_policy".into(),
            description: d("propose_policy"),
            input_schema: obj(
                json!({ "layer": { "type": "string" }, "cedar": { "type": "string" }, "rationale": { "type": "string" } }),
                &["layer", "cedar", "rationale"],
            ),
        },
        ToolSpec {
            name: "propose_schedule".into(),
            description: d("propose_schedule"),
            input_schema: obj(
                json!({ "text": { "type": "string" }, "every_minutes": { "type": "integer" }, "daily_at": { "type": "string" } }),
                &["text"],
            ),
        },
        ToolSpec {
            name: "wait_until".into(),
            description: d("wait_until"),
            input_schema: obj(json!({ "at": { "type": "string" }, "reason": { "type": "string" } }), &["at", "reason"]),
        },
        ToolSpec {
            name: "notify".into(),
            description: d("notify"),
            input_schema: obj(json!({ "message": { "type": "string" } }), &["message"]),
        },
        ToolSpec {
            name: "web_fetch".into(),
            description: d("web_fetch"),
            input_schema: obj(json!({ "url": { "type": "string" } }), &["url"]),
        },
        ToolSpec {
            name: "memory_write".into(),
            description: d("memory_write"),
            input_schema: obj(json!({ "fact": { "type": "string" }, "topic": { "type": "string" } }), &["fact"]),
        },
        ToolSpec {
            name: "memory_search".into(),
            description: d("memory_search"),
            input_schema: obj(json!({ "query": { "type": "string" } }), &["query"]),
        },
        ToolSpec {
            name: "playbook_update".into(),
            description: d("playbook_update"),
            input_schema: obj(
                json!({
                    "op": { "type": "string", "enum": ["add", "edit", "retire"] },
                    "item": { "type": "string", "description": "the item id, for edit and retire" },
                    "text": { "type": "string" },
                    "addresses": { "type": "string", "description": "the failure this item addresses" }
                }),
                &["op"],
            ),
        },
    ];
    if search {
        v.push(ToolSpec {
            name: "web_search".into(),
            description: d("web_search"),
            input_schema: obj(json!({ "query": { "type": "string" } }), &["query"]),
        });
    }
    if browser {
        let target = json!({ "type": "string", "description": "an element ref like e12 from the latest snapshot, or visible text" });
        v.push(ToolSpec { name: "browser_navigate".into(), description: d("browser_navigate"), input_schema: obj(json!({ "url": { "type": "string" } }), &["url"]) });
        v.push(ToolSpec { name: "browser_snapshot".into(), description: d("browser_snapshot"), input_schema: obj(json!({}), &[]) });
        v.push(ToolSpec { name: "browser_click".into(), description: d("browser_click"), input_schema: obj(json!({ "target": target }), &["target"]) });
        v.push(ToolSpec {
            name: "browser_type".into(),
            description: d("browser_type"),
            input_schema: obj(json!({ "target": target, "text": { "type": "string" }, "submit": { "type": "boolean" } }), &["target", "text"]),
        });
        v.push(ToolSpec { name: "browser_select".into(), description: d("browser_select"), input_schema: obj(json!({ "target": target, "value": { "type": "string" } }), &["target", "value"]) });
        v.push(ToolSpec { name: "browser_back".into(), description: d("browser_back"), input_schema: obj(json!({}), &[]) });
        v.push(ToolSpec { name: "browser_screenshot".into(), description: d("browser_screenshot"), input_schema: obj(json!({}), &[]) });
    }
    if sandbox {
        let xy = json!({ "type": "integer" });
        v.push(ToolSpec { name: "computer_screenshot".into(), description: d("computer_screenshot"), input_schema: obj(json!({}), &[]) });
        v.push(ToolSpec {
            name: "computer_click".into(),
            description: d("computer_click"),
            input_schema: obj(json!({ "x": xy, "y": xy, "button": { "type": "string", "enum": ["left", "right", "middle"] }, "double": { "type": "boolean" } }), &["x", "y"]),
        });
        v.push(ToolSpec { name: "computer_type".into(), description: d("computer_type"), input_schema: obj(json!({ "text": { "type": "string" } }), &["text"]) });
        v.push(ToolSpec { name: "computer_key".into(), description: d("computer_key"), input_schema: obj(json!({ "keys": { "type": "string" } }), &["keys"]) });
        v.push(ToolSpec {
            name: "computer_scroll".into(),
            description: d("computer_scroll"),
            input_schema: obj(json!({ "x": xy, "y": xy, "down": { "type": "integer" } }), &["x", "y", "down"]),
        });
        v.push(ToolSpec {
            name: "deliver_file".into(),
            description: d("deliver_file"),
            input_schema: obj(json!({ "path": { "type": "string" }, "name": { "type": "string" } }), &["path"]),
        });
        v.push(ToolSpec {
            name: "sandbox_exec".into(),
            description: d("sandbox_exec"),
            input_schema: obj(
                json!({ "command": { "type": "string" }, "timeout_s": { "type": "integer" } }),
                &["command"],
            ),
        });
        v.push(ToolSpec {
            name: "sandbox_write_file".into(),
            description: d("sandbox_write_file"),
            input_schema: obj(
                json!({ "path": { "type": "string" }, "content": { "type": "string" }, "base64": { "type": "boolean" } }),
                &["path", "content"],
            ),
        });
        v.push(ToolSpec {
            name: "sandbox_read_file".into(),
            description: d("sandbox_read_file"),
            input_schema: obj(json!({ "path": { "type": "string" } }), &["path"]),
        });
    }
    v
}
