//! The agent policy as an admin edits it (admin console spec, Security ›
//! Agent policy): each Cedar policy set (the organisation's, a group's, a
//! person's), a draft of each beside what is live, validation against
//! Magnus's schema with each error's place in the text, a test of any
//! request against the draft or the live policy showing the decision and
//! the rules that decided it, Apply, and every version kept to roll back
//! to.

use crate::config::Person;
use crate::events::Ev;
use crate::kernel::Kernel;
use crate::tools::{Audience, Risk};
use axum::http::StatusCode;
use cedar_policy::{PolicySet, Schema, ValidationMode, Validator};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::str::FromStr;

pub type Refusal = (StatusCode, String);

/// What a policy may name: people in groups, tools in apps with their risk
/// and audience, the two actions, and the run's taint as context.
pub const SCHEMA: &str = r#"
entity Group;
entity Person in [Group];
entity App;
entity Tool in [App] = { "risk": String, "audience": String };
action "execute", "request_approval" appliesTo {
  principal: [Person],
  resource: [Tool],
  context: { "untrusted": Bool, "private": Bool }
};
"#;

fn schema() -> &'static Schema {
    static S: std::sync::OnceLock<Schema> = std::sync::OnceLock::new();
    S.get_or_init(|| Schema::from_cedarschema_str(SCHEMA).expect("Magnus's policy schema parses (a unit test checks)").0)
}

/// Where an offset falls in the text: its line and column, from 1.
fn place(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map(|l| l.chars().count()).unwrap_or(0) + 1;
    (line, col)
}

fn problem(text: &str, d: &dyn miette::Diagnostic, message: String) -> Value {
    let span = d.labels().and_then(|mut l| l.next()).map(|l| (l.offset(), l.len()));
    let (line, column) = span.map(|(o, _)| place(text, o)).unwrap_or((1, 1));
    json!({ "message": message, "line": line, "column": column, "offset": span.map(|s| s.0), "length": span.map(|s| s.1) })
}

/// Parse a policy set and check it against the schema: each error with its
/// place. Empty: it may be applied.
pub fn check(text: &str) -> Vec<Value> {
    let set = match PolicySet::from_str(text) {
        Ok(s) => s,
        Err(e) => return e.iter().map(|x| problem(text, x, x.to_string())).collect(),
    };
    Validator::new(schema().clone()).validate(&set, ValidationMode::Strict).validation_errors().map(|e| problem(text, e, e.to_string())).collect()
}

/// A layer an admin may write: the organisation's, a group's, or a known
/// person's.
pub fn layer_ok(k: &Kernel, layer: &str) -> bool {
    layer == "org" || layer.strip_prefix("user/").is_some_and(|u| k.cfg.person(u).is_some()) || layer.strip_prefix("group/").is_some_and(|g| !g.is_empty() && !g.contains('/') && g.len() <= 100)
}

impl Kernel {
    /// Every policy set: what is live (the organisation's default when none
    /// was written), the draft if there is one, and its versions.
    pub fn policy_sets(&self) -> Vec<Value> {
        let (live, drafts, history) = self.read(|s| (s.policies.clone(), s.policy_drafts.clone(), s.policy_history.clone()));
        let mut layers: Vec<String> = live.keys().chain(drafts.keys()).cloned().collect();
        layers.push("org".into());
        layers.sort();
        layers.dedup();
        layers
            .into_iter()
            .map(|l| {
                let versions: Vec<&crate::state::PolicyVersion> = history.iter().filter(|v| v.layer == l).collect();
                let last = versions.last();
                json!({
                    "id": l,
                    "live": live.get(&l).cloned().or_else(|| (l == "org").then(|| crate::policy::DEFAULT_ORG.trim_start().to_string())),
                    "default": l == "org" && !live.contains_key("org"),
                    "draft": drafts.get(&l),
                    "versions": versions.len(),
                    "changed_at": last.map(|v| v.at), "changed_by": last.map(|v| v.by.clone()),
                })
            })
            .collect()
    }

    /// Keep a draft of a layer (None: drop it). Nothing is in force until
    /// it is applied.
    pub async fn policy_draft(&self, actor: &str, layer: &str, cedar: Option<String>) -> Result<Value, Refusal> {
        if !layer_ok(self, layer) {
            return Err((StatusCode::NOT_FOUND, "no such policy layer: org, group/<name> or user/<person>".into()));
        }
        if cedar.as_ref().is_some_and(|c| c.len() > 256 * 1024) {
            return Err((StatusCode::BAD_REQUEST, "a policy set is at most 256 KB".into()));
        }
        let problems = cedar.as_deref().map(check).unwrap_or_default();
        self.emit(actor, Ev::PolicyDrafted { layer: layer.into(), cedar }).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
        Ok(json!({ "id": layer, "problems": problems }))
    }

    /// Put a layer's policy in force, as `actor`: only one that parses and
    /// fits the schema. Its draft goes.
    pub async fn policy_apply(&self, actor: &str, layer: &str, cedar: &str) -> Result<Value, Refusal> {
        if !layer_ok(self, layer) {
            return Err((StatusCode::NOT_FOUND, "no such policy layer".into()));
        }
        let problems = check(cedar);
        if !problems.is_empty() {
            return Err((StatusCode::BAD_REQUEST, format!("the policy has {} problem{}: {}", problems.len(), if problems.len() == 1 { "" } else { "s" }, problems[0]["message"].as_str().unwrap_or_default())));
        }
        let had_draft = self.read(|s| s.policy_drafts.contains_key(layer));
        let mut evs = vec![(actor, Ev::PolicySet { layer: layer.into(), cedar: cedar.into() })];
        if had_draft {
            evs.push((actor, Ev::PolicyDrafted { layer: layer.into(), cedar: None }));
        }
        let done = self.emit_all(evs).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
        Ok(json!({ "id": layer, "version": self.read(|s| s.policy_history.iter().filter(|v| v.layer == layer).count()), "seq": done.first().map(|e| e.seq) }))
    }

    /// Every version of a layer, newest first.
    pub fn policy_history(&self, layer: &str) -> Vec<Value> {
        let mut v: Vec<Value> = self.read(|s| {
            s.policy_history.iter().filter(|v| v.layer == layer).enumerate().map(|(i, v)| json!({ "version": i + 1, "seq": v.seq, "at": v.at, "by": v.by, "cedar": v.cedar })).collect()
        });
        v.reverse();
        v
    }

    /// A request tried against the policy as it is, or with drafts in place
    /// of what is live: the decision, and the rules that decided it.
    pub fn policy_test(&self, b: &Value) -> Result<Value, Refusal> {
        let s = |f: &str| b[f].as_str().unwrap_or_default().trim().to_string();
        let principal = s("principal");
        let Some(person): Option<Person> = self.person(&principal) else { return Err((StatusCode::BAD_REQUEST, format!("no person {principal:?} on this appliance"))) };
        let tool = s("resource");
        if tool.is_empty() {
            return Err((StatusCode::BAD_REQUEST, "name the tool (the resource)".into()));
        }
        let known = crate::boundary::facts(self, &tool);
        let risk = Risk::parse(&s("risk")).or(known.as_ref().map(|f| f.risk)).ok_or((StatusCode::BAD_REQUEST, "give the tool's risk: read, sandbox, reversible_write, irreversible, financial or prohibited".to_string()))?;
        let audience = Audience::parse(&s("audience")).or(known.as_ref().map(|f| f.audience)).unwrap_or(Audience::Private);
        let app = tool.split_once("__").map(|(a, _)| a.to_string()).unwrap_or_else(|| tool.clone());
        let facts = crate::policy::ToolFacts { name: &tool, app: &app, risk, audience };
        let mut policies: BTreeMap<String, String> = self.read(|s| s.policies.clone());
        if b["drafts"].as_bool().unwrap_or(false) {
            for (l, d) in self.read(|s| s.policy_drafts.clone()) {
                policies.insert(l, d);
            }
        }
        if let Some(over) = b["cedar"].as_object() {
            for (l, c) in over {
                if let Some(c) = c.as_str() {
                    policies.insert(l.clone(), c.to_string());
                }
            }
        }
        let layers: Vec<String> = crate::policy::layers_for(&policies, &person).into_iter().map(|(l, _)| l).collect();
        let outcome = crate::policy::decide(&policies, &person, &facts, b["untrusted"].as_bool().unwrap_or(false), b["private"].as_bool().unwrap_or(false)).map_err(|e| (StatusCode::BAD_REQUEST, format!("{e:#}")))?;
        let (decision, rule, reason) = match outcome {
            crate::policy::Outcome::Allow { rule } => ("allow", Some(rule), None),
            crate::policy::Outcome::Ask { rule } => ("ask", Some(rule), None),
            crate::policy::Outcome::Deny { reason, rule } => ("deny", rule, Some(reason)),
        };
        Ok(json!({ "decision": decision, "rule": rule, "reason": reason, "layers": layers, "risk": risk.as_str(), "audience": audience.as_str() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schema_parses_and_the_default_policy_fits_it() {
        let _ = schema();
        assert!(check(crate::policy::DEFAULT_ORG).is_empty(), "{:?}", check(crate::policy::DEFAULT_ORG));
    }

    #[test]
    fn a_problem_names_its_line_and_column() {
        let p = check("permit(principal, action, resource);\nforbid(principal, action, resource) when { resource.colour == \"red\" };");
        assert_eq!(p.len(), 1, "{p:?}");
        assert_eq!(p[0]["line"], 2, "{p:?}");
        let p = check("permit(principal, action, resource)\n  when { context.untrusted && ;");
        assert!(!p.is_empty() && p[0]["line"] == 2, "{p:?}");
        assert!(check("permit(principal, action == Action::\"execute\", resource) when { context.private == false };").is_empty());
    }
}
