//! The appliance's ontology (O1): the one vocabulary of terms the kernel
//! decides with. The Cedar schema is generated from it, and every policy is
//! checked against that schema when it is written; the entities and requests
//! the kernel builds are checked against it when a policy is evaluated; the
//! memory fact's tool schema is generated from it; every built-in tool's
//! input schema is checked against its value types; and every check a
//! contract asks the kernel to run is checked against its tool's input
//! schema when the contract is recorded (O2). App tools' schemas come from
//! their MCP servers and are pinned as they are.
//!
//! A term is added when a task needs one (O4): each term here is one the
//! kernel builds, a policy can name, or a tool reads or writes. There is no
//! taxonomy beyond them, and `every_term_is_used` keeps it so.

use crate::tools::{Audience, Risk};
use serde_json::{json, Map, Value};
use std::sync::OnceLock;

/// A value's type.
#[derive(Clone, Copy, Debug)]
pub enum Ty {
    Str,
    Int,
    Num,
    Bool,
    /// A string from a fixed list of words.
    OneOf(fn() -> Vec<&'static str>),
}

pub struct Attr {
    pub name: &'static str,
    pub ty: Ty,
}

/// An entity type a policy can name.
pub struct EntityTy {
    pub name: &'static str,
    pub member_of: &'static [&'static str],
    pub attrs: &'static [Attr],
}

/// What a person asks the kernel to do with a tool.
pub struct ActionTy {
    pub name: &'static str,
    pub principal: &'static str,
    pub resource: &'static str,
}

fn risks() -> Vec<&'static str> {
    Risk::ALL.iter().map(|r| r.as_str()).collect()
}

fn audiences() -> Vec<&'static str> {
    Audience::ALL.iter().map(|a| a.as_str()).collect()
}

/// People, the identity provider's groups they are in, the apps their tools
/// belong to, and the tools with how they are classified.
pub const ENTITIES: &[EntityTy] = &[
    EntityTy { name: "Group", member_of: &[], attrs: &[] },
    EntityTy { name: "Person", member_of: &["Group"], attrs: &[] },
    EntityTy { name: "App", member_of: &[], attrs: &[] },
    EntityTy { name: "Tool", member_of: &["App"], attrs: &[Attr { name: "risk", ty: Ty::OneOf(risks) }, Attr { name: "audience", ty: Ty::OneOf(audiences) }] },
];

/// Act without asking, or ask the person first.
pub const ACTIONS: &[ActionTy] = &[ActionTy { name: "execute", principal: "Person", resource: "Tool" }, ActionTy { name: "request_approval", principal: "Person", resource: "Tool" }];

/// What a request carries besides its entities: the run's taint.
pub const CONTEXT: &[Attr] = &[Attr { name: "untrusted", ty: Ty::Bool }, Attr { name: "private", ty: Ty::Bool }];

/// A field of a record a tool writes.
pub struct Field {
    pub name: &'static str,
    pub ty: Ty,
    pub required: bool,
}

/// What the model writes when it remembers something. The kernel adds the
/// rest, never the model: the writing run, its taint and sources, and when.
pub const FACT: &[Field] = &[Field { name: "fact", ty: Ty::Str, required: true }, Field { name: "topic", ty: Ty::Str, required: false }];

fn cedar_ty(t: Ty) -> &'static str {
    match t {
        Ty::Str | Ty::OneOf(_) => "String",
        Ty::Int | Ty::Num => "Long",
        Ty::Bool => "Bool",
    }
}

fn json_ty(t: Ty) -> Value {
    match t {
        Ty::Str => json!({ "type": "string" }),
        Ty::Int => json!({ "type": "integer" }),
        Ty::Num => json!({ "type": "number" }),
        Ty::Bool => json!({ "type": "boolean" }),
        Ty::OneOf(words) => json!({ "type": "string", "enum": words() }),
    }
}

/// The Cedar schema, in Cedar's own syntax, generated from the terms above.
pub fn cedar_schema_text() -> String {
    let mut out = String::new();
    for e in ENTITIES {
        out.push_str("entity ");
        out.push_str(e.name);
        if !e.member_of.is_empty() {
            out.push_str(&format!(" in [{}]", e.member_of.join(", ")));
        }
        if !e.attrs.is_empty() {
            let attrs: Vec<String> = e.attrs.iter().map(|a| format!("{}: {}", a.name, cedar_ty(a.ty))).collect();
            out.push_str(&format!(" {{ {} }}", attrs.join(", ")));
        }
        out.push_str(";\n");
    }
    let context: Vec<String> = CONTEXT.iter().map(|a| format!("{}: {}", a.name, cedar_ty(a.ty))).collect();
    for a in ACTIONS {
        out.push_str(&format!(
            "action \"{}\" appliesTo {{ principal: [{}], resource: [{}], context: {{ {} }} }};\n",
            a.name,
            a.principal,
            a.resource,
            context.join(", ")
        ));
    }
    out
}

pub fn cedar_schema() -> &'static cedar_policy::Schema {
    static SCHEMA: OnceLock<cedar_policy::Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| cedar_policy::Schema::from_cedarschema_str(&cedar_schema_text()).expect("the generated Cedar schema parses").0)
}

/// Check a policy against the schema: it parses, names only the entity
/// types, attributes, actions and context the ontology has, compares them
/// with values of their type, and compares risk and audience only with
/// words they can hold. A policy that fails would never match, or fail when
/// evaluated, which denies everything.
pub fn check_policy(cedar: &str) -> Result<(), String> {
    use std::str::FromStr;
    let set = cedar_policy::PolicySet::from_str(cedar).map_err(|e| format!("{e}"))?;
    let result = cedar_policy::Validator::new(cedar_schema().clone()).validate(&set, cedar_policy::ValidationMode::Strict);
    let errors: Vec<String> = result.validation_errors().map(|e| e.to_string()).collect();
    if !errors.is_empty() {
        return Err(format!("it does not fit the policy schema: {}", errors.join("; ")));
    }
    for p in set.policies() {
        let est = p.to_json().map_err(|e| format!("{e}"))?;
        let mut bad = vec![];
        words_compared(&est, &mut bad);
        if let Some((attr, word, allowed)) = bad.into_iter().next() {
            return Err(format!("it compares {attr} with \"{word}\", which {attr} never holds (it is one of {})", allowed.join(", ")));
        }
    }
    Ok(())
}

/// The attribute a policy expression reads, when it is `x.attr`.
fn attr_of(v: &Value) -> Option<&str> {
    v.get(".")?.get("attr")?.as_str()
}

fn word_of(v: &Value) -> Option<&str> {
    v.get("Value")?.as_str()
}

fn allowed_words(attr: &str) -> Option<Vec<&'static str>> {
    ENTITIES.iter().flat_map(|e| e.attrs.iter()).find(|a| a.name == attr).and_then(|a| match a.ty {
        Ty::OneOf(words) => Some(words()),
        _ => None,
    })
}

/// Every word a policy compares an enumerated attribute with that the
/// attribute cannot hold: `==`, `!=`, and `[...].contains(x)`.
fn words_compared(v: &Value, bad: &mut Vec<(String, String, Vec<&'static str>)>) {
    match v {
        Value::Object(m) => {
            for (op, body) in m {
                let (l, r) = (&body["left"], &body["right"]);
                let pairs: Vec<(&str, &str)> = match op.as_str() {
                    "==" | "!=" => [(l, r), (r, l)].into_iter().filter_map(|(a, w)| Some((attr_of(a)?, word_of(w)?))).collect(),
                    "contains" => match (l.get("Set").and_then(Value::as_array), attr_of(r)) {
                        (Some(set), Some(a)) => set.iter().filter_map(word_of).map(|w| (a, w)).collect(),
                        _ => vec![],
                    },
                    _ => vec![],
                };
                for (attr, word) in pairs {
                    if let Some(allowed) = allowed_words(attr).filter(|ws| !ws.contains(&word)) {
                        bad.push((attr.to_string(), word.to_string(), allowed));
                    }
                }
                words_compared(body, bad);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| words_compared(x, bad)),
        _ => {}
    }
}

/// A tool input schema for a record's fields.
pub fn input_schema(fields: &[Field]) -> Value {
    let props: Map<String, Value> = fields.iter().map(|f| (f.name.to_string(), json_ty(f.ty))).collect();
    let required: Vec<&str> = fields.iter().filter(|f| f.required).map(|f| f.name).collect();
    json!({ "type": "object", "properties": props, "required": required })
}

const JSON_TYPES: &[&str] = &["string", "integer", "number", "boolean", "object", "array"];

/// Check a tool's input schema uses only the ontology's value types: every
/// property typed (or left open with `{}`), every required name declared.
pub fn check_input_schema(schema: &Value) -> Result<(), String> {
    check_schema_at(schema, "the input")
}

fn check_schema_at(s: &Value, at: &str) -> Result<(), String> {
    let Some(m) = s.as_object() else { return Err(format!("{at} has no schema")) };
    if m.is_empty() {
        return Ok(());
    }
    let tys = types(s);
    if tys.is_empty() {
        return Err(format!("{at} has no type"));
    }
    if let Some(ty) = tys.iter().find(|t| !JSON_TYPES.contains(t)) {
        return Err(format!("{at} has type {ty}, which is not one of {}", JSON_TYPES.join(", ")));
    }
    if let Some(props) = m.get("properties").and_then(Value::as_object) {
        for (k, v) in props {
            check_schema_at(v, &format!("{at}.{k}"))?;
        }
        for r in m.get("required").and_then(Value::as_array).into_iter().flatten() {
            let r = r.as_str().unwrap_or_default();
            if !props.contains_key(r) {
                return Err(format!("{at} requires {r}, which it does not declare"));
            }
        }
    }
    if let Some(items) = m.get("items") {
        check_schema_at(items, &format!("{at}[]"))?;
    }
    Ok(())
}

/// Check a value against a tool input schema (O2): the required fields are
/// there, and each declared field present has its type (and word, for an
/// enum). What the schema does not describe is left open.
pub fn check_value(schema: &Value, v: &Value) -> Result<(), String> {
    value_at(schema, v, "input")
}

/// A schema's type, or its list of types.
fn types(s: &Value) -> Vec<&str> {
    match &s["type"] {
        Value::String(t) => vec![t.as_str()],
        Value::Array(ts) => ts.iter().filter_map(Value::as_str).collect(),
        _ => vec![],
    }
}

fn value_at(s: &Value, v: &Value, at: &str) -> Result<(), String> {
    let tys = types(s);
    if tys.is_empty() {
        return Ok(());
    }
    let fits = |ty: &str| match ty {
        "string" => v.is_string(),
        "integer" => v.is_i64() || v.is_u64(),
        "number" => v.is_number(),
        "boolean" => v.is_boolean(),
        "object" => v.is_object(),
        "array" => v.is_array(),
        _ => true,
    };
    if !tys.iter().any(|t| fits(t)) {
        return Err(format!("{at} must be {}, not {}", tys.join(" or "), short(v)));
    }
    if let Some(words) = s["enum"].as_array() {
        if !words.contains(v) {
            return Err(format!("{at} must be one of {}", words.iter().map(|w| w.to_string()).collect::<Vec<_>>().join(", ")));
        }
    }
    if let (Some(props), Some(obj)) = (s["properties"].as_object(), v.as_object()) {
        for r in s["required"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            if obj.get(r).is_none_or(Value::is_null) {
                return Err(format!("{at} has no {r}"));
            }
        }
        for (k, sub) in props {
            if let Some(x) = obj.get(k).filter(|x| !x.is_null()) {
                value_at(sub, x, &format!("{at}.{k}"))?;
            }
        }
    }
    if let (Some(items), Some(arr)) = (s.get("items"), v.as_array()) {
        for (i, x) in arr.iter().enumerate() {
            value_at(items, x, &format!("{at}[{i}]"))?;
        }
    }
    Ok(())
}

fn short(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_generated_schema_parses_and_the_default_policy_fits_it() {
        let _ = cedar_schema();
        check_policy(crate::policy::DEFAULT_ORG).unwrap();
    }

    #[test]
    fn a_policy_naming_what_the_ontology_lacks_is_refused() {
        for (cedar, why) in [
            (r#"permit(principal, action == Action::"execute", resource) when { resource.rsik == "read" };"#, "rsik"),
            (r#"permit(principal, action == Action::"exec", resource);"#, "exec"),
            (r#"forbid(principal in Team::"x", action, resource);"#, "Team"),
            (r#"forbid(principal, action, resource) when { context.tainted };"#, "tainted"),
            (r#"forbid(principal, action, resource) when { context.untrusted == "yes" };"#, "Bool"),
            (r#"permit(principal, action == Action::"execute", resource) when { resource.risk == "Read" };"#, "Read"),
            (r#"forbid(principal, action, resource) when { resource.audience != "everyone" };"#, "everyone"),
            (r#"permit(principal, action, resource) when { ["read", "irreversible_writes"].contains(resource.risk) };"#, "irreversible_writes"),
        ] {
            let e = check_policy(cedar).expect_err(cedar);
            assert!(e.contains(why), "{cedar}: {e}");
        }
    }

    #[test]
    fn policies_in_the_ontology_s_terms_are_accepted() {
        for cedar in [
            r#"forbid(principal, action == Action::"execute", resource == Tool::"deliver_file");"#,
            r#"forbid(principal in Group::"interns", action, resource in App::"mail") when { context.untrusted && context.private };"#,
            r#"permit(principal == Person::"alice", action == Action::"execute", resource) when { resource.risk == "irreversible" && resource.audience == "internal" };"#,
            r#"permit(principal, action, resource) when { ["read", "sandbox"].contains(resource.risk) };"#,
        ] {
            check_policy(cedar).unwrap_or_else(|e| panic!("{cedar}: {e}"));
        }
    }

    /// O4: each entity type, action and context field is one the kernel
    /// builds for a decision; each enumerated word is one a tool can have.
    #[test]
    fn every_term_is_used() {
        let built = crate::policy::terms_built();
        let mut declared: Vec<String> = ENTITIES.iter().map(|e| e.name.to_string()).collect();
        declared.extend(ACTIONS.iter().map(|a| format!("Action::{}", a.name)));
        declared.extend(CONTEXT.iter().map(|c| format!("context.{}", c.name)));
        declared.extend(ENTITIES.iter().flat_map(|e| e.attrs.iter().map(move |a| format!("{}.{}", e.name, a.name))));
        declared.sort();
        assert_eq!(declared, built, "the ontology and the terms the kernel builds differ");
    }

    #[test]
    fn every_tool_input_schema_uses_the_ontology_s_types() {
        let mut specs = crate::tools::builtin_specs(true, true, true);
        specs.extend(crate::tools::chat_specs());
        for t in specs {
            check_input_schema(&t.input_schema).unwrap_or_else(|e| panic!("{}: {e}", t.name));
        }
    }

    #[test]
    fn a_value_is_checked_against_a_schema() {
        let s = json!({ "type": "object", "properties": { "command": { "type": "string" }, "timeout_s": { "type": "integer" }, "risk": { "type": "string", "enum": ["read"] } }, "required": ["command"] });
        check_value(&s, &json!({ "command": "true" })).unwrap();
        assert!(check_value(&s, &json!({ "cmd": "true" })).unwrap_err().contains("no command"));
        assert!(check_value(&s, &json!({ "command": ["true"] })).unwrap_err().contains("input.command must be string"));
        assert!(check_value(&s, &json!({ "command": "x", "timeout_s": "10" })).unwrap_err().contains("integer"));
        assert!(check_value(&s, &json!({ "command": "x", "risk": "write" })).unwrap_err().contains("one of"));
    }
}
