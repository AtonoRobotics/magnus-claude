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

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Label {
    Principal,
    Private,
    Untrusted,
}

impl Label {
    pub fn parse(s: &str) -> Option<Label> {
        Some(match s {
            "principal" => Label::Principal,
            "private" => Label::Private,
            "untrusted" => Label::Untrusted,
            _ => return None,
        })
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
pub const RECORDS: &[&str] = &["task_contract", "task_complete", "task_fail", "ask_principal", "propose_policy", "wait_until", "notify"];

/// Built-in actions that pass the boundary like app tools.
pub struct Builtin {
    pub name: &'static str,
    pub app: &'static str,
    pub risk: Risk,
    pub label: Label,
    pub audience: Audience,
}

pub const BUILTINS: &[Builtin] = &[
    Builtin { name: "sandbox_exec", app: "sandbox", risk: Risk::Sandbox, label: Label::Principal, audience: Audience::Private },
    Builtin { name: "sandbox_write_file", app: "sandbox", risk: Risk::Sandbox, label: Label::Principal, audience: Audience::Private },
    Builtin { name: "sandbox_read_file", app: "sandbox", risk: Risk::Sandbox, label: Label::Principal, audience: Audience::Private },
    Builtin { name: "web_fetch", app: "web", risk: Risk::Read, label: Label::Untrusted, audience: Audience::Public },
    Builtin { name: "web_search", app: "web", risk: Risk::Read, label: Label::Untrusted, audience: Audience::Public },
    Builtin { name: "memory_write", app: "memory", risk: Risk::ReversibleWrite, label: Label::Principal, audience: Audience::Private },
    Builtin { name: "memory_search", app: "memory", risk: Risk::Read, label: Label::Principal, audience: Audience::Private },
];

pub fn builtin(name: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.name == name)
}

fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required })
}

/// Specs for run records and built-in actions. Descriptions are part of the
/// versioned instructions (X11): they live in `instructions/tools.json`.
pub fn builtin_specs(sandbox: bool, search: bool) -> Vec<ToolSpec> {
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
            name: "task_complete".into(),
            description: d("task_complete"),
            input_schema: obj(json!({ "summary": { "type": "string" } }), &["summary"]),
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
    ];
    if search {
        v.push(ToolSpec {
            name: "web_search".into(),
            description: d("web_search"),
            input_schema: obj(json!({ "query": { "type": "string" } }), &["query"]),
        });
    }
    if sandbox {
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
