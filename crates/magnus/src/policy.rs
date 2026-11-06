//! Layered Cedar policy (E1–E4).
//!
//! Entities: `Person::"<id>"` in `Group::"<id>"`; `Tool::"<app>__<tool>"` in
//! `App::"<app>"` with attributes `risk` and `audience`. Context: `untrusted`
//! and `private`, the run's taint. Actions: `Action::"execute"` (act without
//! asking) and `Action::"request_approval"`.
//!
//! An action is permitted only if the organization layer permits it and no
//! lower layer (each of the person's groups, then the user) forbids it. Lower
//! layers can only narrow. Any evaluation error denies.

use crate::config::Person;
use crate::tools::{Audience, Risk};
use anyhow::{anyhow, Result};
use cedar_policy::{
    Authorizer, Context, Decision, Entities, Entity, EntityUid, PolicySet, Request, RestrictedExpression,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::str::FromStr;

/// The organization policy in force until the owner writes one: the agent
/// reads, makes reversible writes and uses its sandbox on its own, and asks
/// for everything else.
pub const DEFAULT_ORG: &str = r#"
permit(principal, action == Action::"execute", resource)
  when { resource.risk == "read" || resource.risk == "reversible_write" || resource.risk == "sandbox" };
permit(principal, action == Action::"request_approval", resource);
"#;

pub struct ToolFacts<'a> {
    pub name: &'a str,
    pub app: &'a str,
    pub risk: Risk,
    pub audience: Audience,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Allow { rule: String },
    Ask { rule: String },
    Deny { reason: String, rule: Option<String> },
}

/// Parse a layer's policy text, so a bad policy is refused when written.
pub fn validate(cedar: &str) -> Result<()> {
    PolicySet::from_str(cedar).map_err(|e| anyhow!("{e}"))?;
    Ok(())
}

fn uid(kind: &str, id: &str) -> Result<EntityUid> {
    let lit = format!("{kind}::{}", serde_json::to_string(id)?);
    EntityUid::from_str(&lit).map_err(|e| anyhow!("entity {lit}: {e}"))
}

fn entities(person: &Person, t: &ToolFacts) -> Result<Entities> {
    let mut groups = HashSet::new();
    let mut list = vec![];
    for g in &person.groups {
        let g = uid("Group", g)?;
        groups.insert(g.clone());
        list.push(Entity::new_no_attrs(g, HashSet::new()));
    }
    list.push(Entity::new_no_attrs(uid("Person", &person.id)?, groups));
    let app = uid("App", t.app)?;
    list.push(Entity::new_no_attrs(app.clone(), HashSet::new()));
    let attrs: HashMap<String, RestrictedExpression> = [
        ("risk".to_string(), RestrictedExpression::new_string(t.risk.as_str().into())),
        ("audience".to_string(), RestrictedExpression::new_string(t.audience.as_str().into())),
    ]
    .into_iter()
    .collect();
    list.push(Entity::new(uid("Tool", t.name)?, attrs, [app].into_iter().collect()).map_err(|e| anyhow!("{e}"))?);
    Entities::from_entities(list, None).map_err(|e| anyhow!("{e}"))
}

/// The evaluation of one policy set: whether it allows, and the rules that
/// decided it.
fn eval(cedar: &str, layer: &str, req: &Request, ents: &Entities) -> Result<(bool, Vec<String>)> {
    let set = PolicySet::from_str(cedar).map_err(|e| anyhow!("policy layer {layer} does not parse: {e}"))?;
    let resp = Authorizer::new().is_authorized(req, &set, ents);
    let errors: Vec<String> = resp.diagnostics().errors().map(|e| e.to_string()).collect();
    if !errors.is_empty() {
        return Err(anyhow!("policy layer {layer} failed to evaluate: {}", errors.join("; ")));
    }
    let rules = resp.diagnostics().reason().map(|id| format!("{layer}:{id}")).collect();
    Ok((resp.decision() == Decision::Allow, rules))
}

/// The layers that apply to `person` in `task`, top first: org, groups,
/// user, task.
pub fn layers_for(policies: &BTreeMap<String, String>, person: &Person, task: Option<&str>) -> Vec<(String, String)> {
    let mut v = vec![("org".to_string(), policies.get("org").cloned().unwrap_or_else(|| DEFAULT_ORG.to_string()))];
    for g in &person.groups {
        let k = format!("group/{g}");
        if let Some(p) = policies.get(&k) {
            v.push((k, p.clone()));
        }
    }
    let k = format!("user/{}", person.id);
    if let Some(p) = policies.get(&k) {
        v.push((k, p.clone()));
    }
    if let Some(p) = task.map(|t| format!("task/{t}")).and_then(|k| policies.get(&k).map(|p| (k, p.clone()))) {
        v.push(p);
    }
    v
}

pub fn decide(
    policies: &BTreeMap<String, String>,
    person: &Person,
    task: Option<&str>,
    tool: &ToolFacts,
    untrusted: bool,
    private: bool,
) -> Result<Outcome> {
    let ents = entities(person, tool)?;
    let layers = layers_for(policies, person, task);
    let ctx = Context::from_pairs([
        ("untrusted".to_string(), RestrictedExpression::new_bool(untrusted)),
        ("private".to_string(), RestrictedExpression::new_bool(private)),
    ])
    .map_err(|e| anyhow!("{e}"))?;

    let mut forbidden: Option<String> = None;
    let mut results = vec![];
    for action in ["execute", "request_approval"] {
        let req = Request::new(uid("Person", &person.id)?, uid("Action", action)?, uid("Tool", tool.name)?, ctx.clone(), None)
            .map_err(|e| anyhow!("{e}"))?;
        let (org_ok, org_rules) = eval(&layers[0].1, "org", &req, &ents)?;
        let mut ok = org_ok;
        let mut rule = org_rules.first().cloned();
        if !org_ok && !org_rules.is_empty() {
            forbidden.get_or_insert_with(|| org_rules[0].clone());
        }
        for (name, text) in &layers[1..] {
            // With a permit-everything rule added, the layer denies only if one
            // of its own forbids matched.
            let narrowed = format!("{text}\npermit(principal, action, resource);");
            let (allows, rules) = eval(&narrowed, name, &req, &ents)?;
            if !allows {
                ok = false;
                rule = rules.first().cloned();
                forbidden.get_or_insert_with(|| rules.first().cloned().unwrap_or_else(|| name.clone()));
            }
        }
        results.push((ok, rule));
    }
    let (exec_ok, exec_rule) = results[0].clone();
    let (ask_ok, ask_rule) = results[1].clone();
    Ok(if exec_ok {
        Outcome::Allow { rule: exec_rule.unwrap_or_default() }
    } else if ask_ok {
        Outcome::Ask { rule: ask_rule.unwrap_or_default() }
    } else {
        let reason = match &forbidden {
            Some(r) => format!("policy rule {r} forbids {} for {}", tool.name, person.id),
            None => format!("no policy permits {} for {}", tool.name, person.id),
        };
        Outcome::Deny { reason, rule: forbidden }
    })
}

/// A grant's policy (#235), written by the kernel from what the person
/// chose: their own actions in the grant's apps, less what stays excluded.
/// Moving money always stays excluded (the kernel cannot measure an
/// amount, so no spend limit could hold), as does anything prohibited; so
/// do irreversible actions (deleting, committing them to anything) and
/// sends (a change that reaches someone beyond the person), unless the
/// person lifted that exclusion (`allow`: `irreversible`, `send`).
pub fn grant_cedar(person: &str, apps: &[String], allow: &[String]) -> Result<String> {
    let q = |s: &str| serde_json::to_string(s).map_err(|e| anyhow!("{e}"));
    let within = apps.iter().map(|a| Ok(format!("resource in App::{}", q(a)?))).collect::<Result<Vec<_>>>()?.join(" || ");
    if within.is_empty() {
        return Err(anyhow!("a grant covers at least one app"));
    }
    let mut p = format!("permit(principal == Person::{}, action == Action::\"execute\", resource)\n  when {{ {within} }};\n", q(person)?);
    p.push_str("forbid(principal, action, resource) when { resource.risk == \"financial\" };\n");
    p.push_str("forbid(principal, action, resource) when { resource.risk == \"prohibited\" };\n");
    if !allow.iter().any(|a| a == "irreversible") {
        p.push_str("forbid(principal, action, resource) when { resource.risk == \"irreversible\" };\n");
    }
    if !allow.iter().any(|a| a == "send") {
        p.push_str("forbid(principal, action, resource) when { resource.audience != \"private\" && resource.risk != \"read\" && resource.risk != \"sandbox\" };\n");
    }
    validate(&p)?;
    Ok(p)
}

/// Whether a grant's policy lets `person` take this action without asking:
/// its own rules alone decide, evaluated as any layer is.
pub fn grant_covers(cedar: &str, person: &Person, tool: &ToolFacts) -> Result<bool> {
    let ents = entities(person, tool)?;
    let req = Request::new(uid("Person", &person.id)?, uid("Action", "execute")?, uid("Tool", tool.name)?, Context::empty(), None).map_err(|e| anyhow!("{e}"))?;
    Ok(eval(cedar, "grant", &req, &ents)?.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(id: &str) -> Person {
        serde_json::from_value(serde_json::json!({ "id": id, "groups": ["staff"] })).unwrap()
    }

    /// A grant covers its person's actions in its apps, never moving money
    /// or anyone else's actions, and sends and irreversible actions only
    /// once lifted.
    #[test]
    fn a_grant_covers_its_apps_less_what_stays_excluded() {
        let carol = person("carol");
        let t = |name: &'static str, app: &'static str, risk: Risk, audience: Audience| ToolFacts { name, app, risk, audience };
        let save = t("crm__update_record", "crm", Risk::ReversibleWrite, Audience::Private);
        let post = t("team__post", "team", Risk::ReversibleWrite, Audience::Internal);
        let delete = t("crm__delete_all", "crm", Risk::Irreversible, Audience::Private);
        let pay = t("crm__pay", "crm", Risk::Financial, Audience::Private);
        let other = t("notes__save", "notes", Risk::ReversibleWrite, Audience::Private);
        let apps = vec!["crm".to_string(), "team".to_string()];
        let none = grant_cedar("carol", &apps, &[]).unwrap();
        assert!(grant_covers(&none, &carol, &save).unwrap());
        for x in [&post, &delete, &pay, &other] {
            assert!(!grant_covers(&none, &carol, x).unwrap(), "{} covered by default", x.name);
        }
        assert!(!grant_covers(&none, &person("bob"), &save).unwrap(), "someone else's action covered");
        let all = grant_cedar("carol", &apps, &["send".into(), "irreversible".into()]).unwrap();
        assert!(grant_covers(&all, &carol, &post).unwrap() && grant_covers(&all, &carol, &delete).unwrap());
        assert!(!grant_covers(&all, &carol, &pay).unwrap(), "moving money covered");
        assert!(grant_cedar("carol", &[], &[]).is_err(), "a grant over no app");
        // An app's name is quoted, never read as policy.
        let odd = grant_cedar("carol", &["crm\" }; permit(principal, action, resource); //".into()], &[]).unwrap();
        assert!(!grant_covers(&odd, &carol, &other).unwrap());
    }
}
