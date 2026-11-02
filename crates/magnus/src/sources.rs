//! The org's sources (org context build spec, step 4): `org_search` runs
//! each source's own search in parallel and `org_fetch` reads one item, each
//! under the requester's own grant (K1): their own sign-in to the app, or
//! the organisation's key for an app that takes one, never anyone else's.
//! A source they may not use (its access groups), have not connected, or
//! whose tool definition changed since it was approved is not reached. What
//! comes back carries each source's label, as a direct call would.

use crate::config::Connector;
use crate::kernel::Kernel;
use crate::tools::Label;
use anyhow::Result;
use serde_json::{json, Value};

/// Whether a person can read a source item, as the source last said (K3),
/// and when it was asked; and a count of changes, for caches built on it.
type Permits = std::collections::HashMap<(String, String), (bool, std::time::Instant)>;

fn permits() -> &'static std::sync::Mutex<(Permits, u64)> {
    static P: std::sync::OnceLock<std::sync::Mutex<(Permits, u64)>> = std::sync::OnceLock::new();
    P.get_or_init(Default::default)
}

/// What is known of `person` reading `item`, while it is fresh.
pub fn permit(person: &str, item: &str, ttl: std::time::Duration) -> Option<bool> {
    let p = permits().lock().unwrap();
    p.0.get(&(person.to_string(), item.to_string())).filter(|(_, at)| at.elapsed() < ttl).map(|(ok, _)| *ok)
}

pub fn set_permit(person: &str, item: &str, ok: bool) {
    let mut p = permits().lock().unwrap();
    let before = p.0.insert((person.to_string(), item.to_string()), (ok, std::time::Instant::now())).map(|x| x.0);
    if before != Some(ok) {
        p.1 += 1;
    }
}

/// Changes to what is known, so a cache of what someone may see can tell
/// when to look again.
pub fn permits_generation() -> u64 {
    permits().lock().unwrap().1
}

/// A locator names the source and the item's id there: `<source>://<id>`.
pub fn locator(source: &str, id: &str) -> String {
    format!("{source}://{id}")
}

pub fn parse_locator(l: &str) -> Option<(&str, &str)> {
    let (s, id) = l.split_once("://")?;
    (!s.is_empty() && !id.is_empty()).then_some((s, id))
}

/// The first value of `keys` an object has, as text.
fn first(o: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| match &o[*k] {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Object(m) => m.get("name").or(m.get("email")).or(m.get("displayName")).and_then(|v| v.as_str()).map(String::from),
        Value::Array(a) => a.first().and_then(|x| x.as_str().map(String::from).or_else(|| x["name"].as_str().or(x["email"].as_str()).map(String::from))),
        _ => None,
    })
}

/// The items a source's search returned: the first list of objects in
/// what it answered, outermost first.
fn items(v: &Value) -> Vec<Value> {
    let mut queue = vec![v.clone()];
    while !queue.is_empty() {
        let mut next = vec![];
        for x in queue {
            match x {
                Value::Array(a) if a.iter().any(Value::is_object) => return a.into_iter().filter(Value::is_object).collect(),
                Value::Array(a) => next.extend(a),
                Value::Object(m) => next.extend(m.into_values()),
                _ => {}
            }
        }
        queue = next;
    }
    vec![]
}

/// A source's search results as hits: source, locator, title, snippet,
/// author, last modified (the spec's fields).
pub fn hits(source: &str, id_field: &str, answer: &Value, limit: usize) -> Vec<Value> {
    items(answer)
        .iter()
        .filter_map(|o| {
            let id = first(o, &[id_field, "id"])?;
            let snippet: Option<String> = first(o, &["snippet", "summary", "description", "text", "content"]).map(|s| s.chars().take(300).collect());
            Some(json!({
                "source": source,
                "locator": locator(source, &id),
                "title": first(o, &["title", "name", "subject"]),
                "snippet": snippet,
                "author": first(o, &["author", "owner", "owners", "sender", "from", "createdBy"]),
                "last_modified": first(o, &["last_modified", "modifiedTime", "modified", "updated", "updatedAt", "date"]),
            }))
        })
        .take(limit)
        .collect()
}

/// What an MCP tool answered, as JSON: its structured content, else its
/// text parsed, else the text itself.
fn answer(text: &str, structured: Option<Value>) -> Value {
    structured.unwrap_or_else(|| serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.into())))
}

impl Kernel {
    /// Whether an item's locator names a source the kernel can ask.
    pub fn askable(&self, item: &str) -> bool {
        parse_locator(item).is_some_and(|(s, _)| self.connector(s).is_some_and(|c| c.source.as_ref().is_some_and(|x| x.fetch.is_some())))
    }

    /// K3: whether `person` can read `item`, from what the source last said
    /// while fresh, else asked of the source now under their grant (their
    /// reader refused, or the source out of their reach, is a no).
    pub async fn can_read(&self, person: &str, item: &str) -> bool {
        let ttl = std::time::Duration::from_secs(self.cfg.bundle.permission_seconds);
        if let Some(ok) = permit(person, item, ttl) {
            return ok;
        }
        let ok = match parse_locator(item) {
            Some((source, id)) => match self.sources_for(person, &[source.to_string()]).into_iter().next() {
                Some((c, s)) => match &s.fetch {
                    Some(fetch) => {
                        let mut args: serde_json::Map<String, Value> = s.fetch_args.clone().into_iter().collect();
                        args.insert(s.id.clone(), json!(id));
                        self.source_call(person, &c, fetch, &Value::Object(args)).await.is_ok()
                    }
                    None => false,
                },
                None => false,
            },
            None => false,
        };
        set_permit(person, item, ok);
        ok
    }

    /// Ask the sources about every item `docs` list that `person` has no
    /// fresh answer for, so visibility (K2) can be decided from what is known.
    pub async fn check_sources(&self, person: &str, docs: &[crate::state::BundleDoc]) {
        let ttl = std::time::Duration::from_secs(self.cfg.bundle.permission_seconds);
        let mut items: Vec<String> = docs.iter().filter(|d| d.scope == "org").flat_map(|d| d.sources.clone()).filter(|s| self.askable(s) && permit(person, s, ttl).is_none()).collect();
        items.sort();
        items.dedup();
        futures::future::join_all(items.iter().map(|i| self.can_read(person, i))).await;
    }

    /// The sources `person` may reach now, with each one's search facts.
    fn sources_for(&self, person: &str, only: &[String]) -> Vec<(Connector, crate::config::Source)> {
        let Some(p) = self.person(person) else { return vec![] };
        self.connectors()
            .into_iter()
            .filter(|c| only.is_empty() || only.contains(&c.name))
            .filter_map(|c| c.source.clone().map(|s| (c, s)))
            .filter(|(c, s)| crate::boundary::facts(self, &format!("{}__{}", c.name, s.search)).is_some_and(|f| crate::boundary::has_rights(&p, &f)))
            .collect()
    }

    /// Call one source tool for `person` under their own grant, as the
    /// boundary would: connected, its definition the approved one.
    async fn source_call(&self, person: &str, c: &Connector, tool: &str, input: &Value) -> std::result::Result<(Value, Label), String> {
        let name = format!("{}__{tool}", c.name);
        let Some(f) = crate::boundary::facts(self, &name) else { return Err(format!("{} does not offer {tool}, or it is not classified", c.name)) };
        if f.risk != crate::tools::Risk::Read {
            return Err(format!("{name} is not classified as a read"));
        }
        if !self.connected(c, person) {
            return Err(format!("{} is not connected for the person: they connect it with their own account in the Library", c.name));
        }
        self.usable(c, &f, person).await?;
        let secret = self.connector_secret(c, person).await.map_err(|e| format!("{e:#}"))?;
        if c.credential.is_some() && secret.is_none() {
            return Err(format!("{}'s sign-in has expired: the person signs in again in the Library", c.name));
        }
        match self.mcp.call(&c.url, secret.as_ref().map(|s| s.expose()), tool, input, None).await {
            Ok(o) if o.is_error => Err(format!("{} refused: {}", c.name, o.text.chars().take(300).collect::<String>())),
            Ok(o) => Ok((answer(&o.text, o.structured), f.label)),
            Err(e) => Err(format!("{} did not answer: {e:?}", c.name)),
        }
    }

    /// `org_search` for the run's person: every source they may reach,
    /// searched at once. The data says the combined taint and which sources
    /// answered, for the boundary.
    pub async fn org_search(&self, run: &str, input: &Value) -> Result<(String, bool, Option<Value>)> {
        let person = self.read(|s| s.runs.get(run).map(|r| r.person.clone())).unwrap_or_default();
        let query = input["query"].as_str().unwrap_or_default().trim().to_string();
        if query.is_empty() {
            return Ok(("A search needs a query.".into(), true, Some(json!({ "untrusted": false, "private": false }))));
        }
        let only: Vec<String> = input["sources"].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(String::from)).collect();
        let limit = input["limit"].as_u64().unwrap_or(10).clamp(1, 50) as usize;
        let sources = self.sources_for(&person, &only);
        if sources.is_empty() {
            return Ok(("No source the person can reach is searchable (none is connected, or none named is theirs to use).".into(), false, Some(json!({ "hits": [], "untrusted": false, "private": false }))));
        }
        let calls = sources.iter().map(|(c, s)| {
            let q = match &s.template {
                Some(t) => t.replace("{q}", &query.replace('\'', "\\'")),
                None => query.clone(),
            };
            let person = person.clone();
            async move { (c.name.clone(), s.id.clone(), self.source_call(&person, c, &s.search, &json!({ s.query.clone(): q })).await) }
        });
        let answers = futures::future::join_all(calls).await;
        let (mut all, mut status, mut label, mut apps) = (vec![], vec![], Label::PRINCIPAL, vec![]);
        for (name, id_field, r) in answers {
            match r {
                Ok((v, l)) => {
                    let h = hits(&name, &id_field, &v, limit);
                    // The source found them for the person: they can read them.
                    for x in &h {
                        set_permit(&person, x["locator"].as_str().unwrap_or_default(), true);
                    }
                    status.push(json!({ "source": name, "hits": h.len() }));
                    label = label.join(l);
                    apps.push(name);
                    all.extend(h);
                }
                Err(why) => status.push(json!({ "source": name, "not_searched": why })),
            }
        }
        let text = json!({ "hits": all, "sources": status }).to_string();
        Ok((text, false, Some(json!({ "hits": all, "untrusted": label.untrusted, "private": label.private, "apps": apps }))))
    }

    /// `org_fetch`: one item by its locator, under the person's own grant.
    pub async fn org_fetch(&self, run: &str, input: &Value) -> Result<(String, bool, Option<Value>)> {
        let person = self.read(|s| s.runs.get(run).map(|r| r.person.clone())).unwrap_or_default();
        let clean = Some(json!({ "untrusted": false, "private": false }));
        let l = input["locator"].as_str().unwrap_or_default();
        let Some((source, id)) = parse_locator(l) else { return Ok((format!("{l:?} is not a locator (source://id, as a search hit gives it)."), true, clean)) };
        let Some((c, s)) = self.sources_for(&person, &[source.to_string()]).into_iter().next() else {
            return Ok((format!("{source} is not a source the person can reach."), true, clean));
        };
        let Some(fetch) = s.fetch.clone() else { return Ok((format!("{source} has no reader: open what its search gave."), true, clean)) };
        let mut args: serde_json::Map<String, Value> = s.fetch_args.clone().into_iter().collect();
        args.insert(s.id.clone(), json!(id));
        let r = self.source_call(&person, &c, &fetch, &Value::Object(args)).await;
        set_permit(&person, l, r.is_ok());
        match r {
            Ok((v, label)) => {
                let content = match &v {
                    Value::String(t) => t.clone(),
                    other => serde_json::to_string_pretty(other).unwrap_or_default(),
                };
                let o = items(&v).into_iter().next().unwrap_or_else(|| v.clone());
                let read = json!({
                    "source": source, "locator": l, "author": first(&o, &["author", "owner", "owners", "sender", "from"]),
                    "last_modified": first(&o, &["last_modified", "modifiedTime", "modified", "updated", "date"]), "read_at": chrono::Utc::now(),
                });
                Ok((format!("{read}\n\n{content}"), false, Some(json!({ "untrusted": label.untrusted, "private": label.private, "apps": [source] }))))
            }
            Err(why) => Ok((why, true, clean)),
        }
    }
}

impl Kernel {
    /// `people.ask`: the question to the area's owner (from its people page,
    /// which the asker must be able to see) or to the person named, in the
    /// app under Magnus's name, saying whom it is for. The run waits; the
    /// answer is this call's result. Why not, when it cannot be asked.
    pub async fn people_ask(&self, a: &crate::boundary::Action, run: &crate::state::Run) -> Result<std::result::Result<(), String>> {
        use sha2::Digest;
        let question = a.input["question"].as_str().unwrap_or_default().trim().to_string();
        if question.is_empty() {
            return Ok(Err("A question is needed. A retry without one will not help.".into()));
        }
        let (area, user) = (a.input["area"].as_str().unwrap_or_default().trim(), a.input["user"].as_str().unwrap_or_default().trim());
        let (to, through) = if !area.is_empty() {
            let path = if area.starts_with('/') { area.to_string() } else { format!("/people/{area}.md") };
            let Some((fm, _, _, _)) = self.bundle_read(&run.person, &path).await? else {
                return Ok(Err(format!("{path}: no such people page, or not one the person can see. bundle_search finds areas.")));
            };
            let channel = fm["ask_channel"].as_str().unwrap_or_default().to_string();
            if !matches!(channel.as_str(), "magnus" | "app") {
                return Ok(Err(format!("{path} asks through {channel:?}, which Magnus cannot send to yet: only questions in the app (ask_channel: magnus). Ask the person instead, or tell the person you work for whom to ask.")));
            }
            (fm["owner"].as_str().unwrap_or_default().to_string(), Some(path))
        } else {
            (user.to_string(), None)
        };
        if to.is_empty() || self.person(&to).is_none() {
            return Ok(Err(format!("{to:?} is not someone at the organisation who can be asked.")));
        }
        if to == run.person {
            return Ok(Err("That is the person you work for: ask them with ask_principal.".into()));
        }
        let asker = self.person(&run.person).map(|p| p.name.clone()).unwrap_or_else(|| run.person.clone());
        let id = format!("item_{}", &hex::encode(sha2::Sha256::digest(format!("{}\n{}\npeople_ask", run.id, a.call_id).as_bytes()))[..32]);
        if self.read(|s| s.items.contains_key(&id)) {
            return Ok(Ok(()));
        }
        let ev = crate::events::Ev::Item {
            id: id.clone(),
            item_kind: "question".into(),
            task: Some(run.task.clone()),
            run: Some(run.id.clone()),
            person: to.clone(),
            summary: format!("Magnus asks, for {asker}: {question}"),
            details: json!({ "call_id": a.call_id, "question": question, "asked_for": run.person, "area": through, "taint": { "untrusted": run.untrusted, "private": run.private } }),
        };
        self.suspend_on(&run.id, "question", id, Some(ev)).await?;
        Ok(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_come_from_the_first_list_of_items_in_any_shape() {
        let drive = json!({ "files": [{ "id": "f1", "title": "Q3 plan", "snippet": "Targets", "owners": [{ "name": "Ann" }], "modifiedTime": "2026-10-01T00:00:00Z" }], "nextPageToken": "x" });
        let h = hits("google-drive", "id", &drive, 10);
        assert_eq!(h[0]["locator"], "google-drive://f1");
        assert_eq!(h[0]["title"], "Q3 plan");
        assert_eq!(h[0]["author"], "Ann");
        let wrapped = json!({ "result": { "threads": [{ "id": "t9", "subject": "Renewal", "sender": "bo@x.example" }] } });
        assert_eq!(hits("gmail", "id", &wrapped, 10)[0]["title"], "Renewal");
        assert!(hits("x", "id", &json!({ "count": 0 }), 10).is_empty());
        assert_eq!(parse_locator("wiki://a/b"), Some(("wiki", "a/b")));
        assert_eq!(parse_locator("nope"), None);
    }
}
