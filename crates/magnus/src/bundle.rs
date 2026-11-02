//! The org context bundle (the owner's org context build spec, Bundle
//! format; `docs/proposals/org-context.md`, step 1): an OKF v0.2 bundle with
//! the Magnus profile. Seven kinds of document, each with the fields the
//! kernel checks on every write; a write that breaks a rule is refused with
//! the reason and nothing is stored (K8); `/index.md` is at most 16KB (K9);
//! every write is a new version. Documents are events (`BundleWritten`):
//! the log keeps where each is and whose, its frontmatter and body are in
//! the content store, which the kernel reads itself, deciding who may see
//! what (K2).

use crate::kernel::Kernel;
use crate::tools::Label;
use serde_json::{json, Map, Value};

/// The root index's size limit, before filtering, so every person's view
/// of it fits (K9).
pub const INDEX_CAP: usize = 16 * 1024;

/// Where a path puts a document: its kind, and whose it is when it is a
/// person's own (under `/users/<user>/`).
pub fn place(path: &str) -> Result<(&'static str, Option<String>), String> {
    let p = path.strip_prefix('/').ok_or_else(|| format!("{path}: a bundle path starts with /"))?;
    if !p.ends_with(".md") {
        return Err(format!("{path}: a bundle document is a .md file"));
    }
    let segs: Vec<&str> = p.split('/').collect();
    if segs.iter().any(|s| s.is_empty() || *s == "." || *s == ".." || !s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))) {
        return Err(format!("{path}: each part of a path is letters, digits, -, _ or ."));
    }
    let file = *segs.last().unwrap();
    let user = (segs[0] == "users" && segs.len() >= 3).then(|| segs[1].to_string());
    if segs[0] == "users" && user.is_none() {
        return Err(format!("{path}: a person's documents are under /users/<user>/"));
    }
    let kind = match (file, segs.as_slice()) {
        ("index.md", _) => "index",
        ("log.md", s) if s.len() >= 2 => "log",
        ("log.md", _) => return Err(format!("{path}: a log is /<area>/log.md")),
        _ if user.is_some() => "user",
        (_, ["sources", _]) => "source",
        (_, ["types", _]) => "type",
        (_, ["entities", _, _]) => "entity",
        (_, ["conventions", _, _]) => "convention",
        (_, ["people", _]) => "people",
        _ => return Err(format!("{path}: not a place in the bundle (sources/, types/, entities/<type>/, conventions/<area>/, people/, users/<user>/, an index.md or a log.md)")),
    };
    Ok((kind, user))
}

/// The fields each kind needs beyond OKF's `type`, and the `type` it must
/// carry (an entity's is its type document's title).
fn required(kind: &str) -> (&'static [&'static str], Option<&'static str>) {
    match kind {
        "source" => (&["title", "description", "resource", "sources", "generated"], Some("Source")),
        "type" => (&["title", "description", "generated", "status", "system_of_record", "identity_keys", "relations", "cedar_entity"], Some("Entity Type")),
        "entity" => (&["title", "resource", "sources", "generated", "identity", "relations"], None),
        "convention" => (&["title", "description", "sources", "generated", "verified", "status"], Some("Convention")),
        "people" => (&["title", "owner", "ask_channel"], Some("Area")),
        _ => (&[], None),
    }
}

const VERIFICATIONS: [&str; 6] = ["owner_review", "downstream_result", "correction", "rejection", "acceptance", "recheck"];

/// An OKF actor: `human:<id>`, `process:<id>`, or `<producer>/<version>`.
pub fn actor_ok(a: &str) -> bool {
    let plain = |s: &str| !s.is_empty() && !s.chars().any(char::is_whitespace);
    match a.split_once(':') {
        Some(("human" | "process", id)) => plain(id),
        Some(_) => false,
        None => a.split_once('/').is_some_and(|(p, v)| plain(p) && plain(v)),
    }
}

fn time_ok(v: &Value) -> bool {
    v.as_str().is_some_and(|s| chrono::DateTime::parse_from_rfc3339(s).is_ok())
}

/// A link the profile accepts: bundle-absolute, or a source's URI.
fn link_ok(l: &str) -> bool {
    l.starts_with('/') || l.split_once(':').is_some_and(|(scheme, _)| !scheme.is_empty() && scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
}

/// Every Markdown link target in a body.
pub fn links(body: &str) -> Vec<String> {
    let mut out = vec![];
    let mut rest = body;
    while let Some(i) = rest.find("](") {
        rest = &rest[i + 2..];
        if let Some(j) = rest.find(')') {
            out.push(rest[..j].split_whitespace().next().unwrap_or_default().to_string());
            rest = &rest[j..];
        } else {
            break;
        }
    }
    out
}

/// Check a document before it is stored. `type_doc` gives an entity's type
/// document's frontmatter (the bundle's current one).
pub fn validate(path: &str, fm: &Value, body: &str, type_doc: impl Fn(&str) -> Option<Value>) -> Result<(), String> {
    let (kind, user) = place(path)?;
    let fm = fm.as_object().cloned().unwrap_or_default();
    for l in links(body) {
        if !link_ok(&l) {
            return Err(format!("{path}: the link {l:?} is neither a bundle path (starting with /) nor a source's URI"));
        }
    }
    if kind == "index" || kind == "log" {
        let allowed = path == "/index.md" && fm.keys().all(|k| k == "okf_version") && fm.get("okf_version").is_none_or(|v| v == "0.2");
        if !fm.is_empty() && !allowed {
            return Err(format!("{path}: an {kind} carries no frontmatter (the root index may carry okf_version: \"0.2\")"));
        }
        if path == "/index.md" && body.len() > INDEX_CAP {
            return Err(format!("/index.md is {} bytes; it may be at most {INDEX_CAP} (16KB), so every person's view of it fits", body.len()));
        }
        return if kind == "index" { index_ok(path, body) } else { log_ok(path, body) };
    }
    let ty = fm.get("type").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    if ty.is_empty() {
        return Err(format!("{path}: no type"));
    }
    let (fields, expect) = required(kind);
    for f in fields {
        if fm.get(*f).is_none_or(|v| v.is_null() || v == "" || v.as_array().is_some_and(|a| a.is_empty() && *f != "relations")) {
            return Err(format!("{path}: a {kind} document needs {f}"));
        }
    }
    if let Some(e) = expect {
        if ty != e {
            return Err(format!("{path}: a {kind} document's type is {e:?}, not {ty:?}"));
        }
    }
    // Scope: a person's own documents are theirs alone; the rest are the org's.
    match (&user, fm.get("scope").and_then(Value::as_str)) {
        (Some(u), Some("user")) if fm.get("owner_user").and_then(Value::as_str) == Some(u) => {}
        (Some(u), _) => return Err(format!("{path}: a document under /users/{u}/ has scope: user and owner_user: {u}")),
        (None, Some("org")) => {}
        (None, _) => return Err(format!("{path}: an org document has scope: org")),
    }
    if let Some(g) = fm.get("generated") {
        if !g["by"].as_str().is_some_and(actor_ok) {
            return Err(format!("{path}: generated.by is an actor: human:<id>, process:<id> or <producer>/<version>"));
        }
        if !time_ok(&g["at"]) {
            return Err(format!("{path}: generated.at is an ISO 8601 time with its offset"));
        }
    }
    if let Some(s) = fm.get("stale_after") {
        if !time_ok(s) {
            return Err(format!("{path}: stale_after is an ISO 8601 time with its offset"));
        }
    }
    if let Some(s) = fm.get("status") {
        if !s.as_str().is_some_and(|s| ["draft", "stable", "deprecated"].contains(&s)) {
            return Err(format!("{path}: status is draft, stable or deprecated"));
        }
    }
    if let Some(v) = fm.get("verified") {
        let list = match v {
            Value::Array(a) => a.clone(),
            Value::Object(_) => vec![v.clone()],
            _ => return Err(format!("{path}: verified is a list of {{by, at, kind, event, note}}")),
        };
        for e in &list {
            if !e["by"].as_str().is_some_and(actor_ok) || !time_ok(&e["at"]) || !e["kind"].as_str().is_some_and(|k| VERIFICATIONS.contains(&k)) {
                return Err(format!("{path}: each verification has by (an actor), at (an ISO 8601 time with its offset) and kind ({})", VERIFICATIONS.join(", ")));
            }
        }
    }
    if let Some(s) = fm.get("sources") {
        let ok = s.as_array().is_some_and(|a| a.iter().all(|x| x["resource"].as_str().is_some_and(link_ok)));
        if !ok {
            return Err(format!("{path}: sources is a list whose every entry has a resource: a bundle path or a source's URI"));
        }
    }
    if let Some(r) = fm.get("resource") {
        if !r.as_str().is_some_and(link_ok) {
            return Err(format!("{path}: resource is a bundle path or a source's URI"));
        }
    }
    if kind == "entity" {
        let tslug = path.trim_start_matches('/').split('/').nth(1).unwrap_or_default();
        let tdoc = type_doc(tslug).ok_or_else(|| format!("{path}: its type /types/{tslug}.md is not in the bundle"))?;
        if tdoc["title"].as_str() != Some(ty) {
            return Err(format!("{path}: an entity's type is its type document's title, {:?}", tdoc["title"].as_str().unwrap_or_default()));
        }
        let keys: Vec<&str> = tdoc["identity_keys"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        for k in fm["identity"].as_object().ok_or_else(|| format!("{path}: identity maps keys to values"))?.keys() {
            if !keys.contains(&k.as_str()) {
                return Err(format!("{path}: identity key {k:?} is not one of its type's identity_keys ({})", keys.join(", ")));
            }
        }
        let rels: Vec<&str> = tdoc["relations"].as_array().into_iter().flatten().filter_map(|r| r.as_str().or(r["type"].as_str()).or(r["name"].as_str())).collect();
        for r in fm["relations"].as_array().ok_or_else(|| format!("{path}: relations is a list of {{type, to}}"))? {
            let t = r["type"].as_str().unwrap_or_default();
            if !rels.contains(&t) || !r["to"].as_str().is_some_and(link_ok) {
                return Err(format!("{path}: relation {t:?} is not one its type declares ({}), or has no to", rels.join(", ")));
            }
        }
    }
    Ok(())
}

/// Headings grouping bulleted links, each with a short description.
fn index_ok(path: &str, body: &str) -> Result<(), String> {
    for l in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let bullet = (l.starts_with("* ") || l.starts_with("- ")) && l.contains("](");
        if !l.starts_with('#') && !bullet {
            return Err(format!("{path}: an index is headings and bulleted links (\"* [Title](/path.md) - description\"); not {l:?}"));
        }
    }
    Ok(())
}

/// Date-grouped entries, newest first.
fn log_ok(path: &str, body: &str) -> Result<(), String> {
    let mut last: Option<chrono::NaiveDate> = None;
    let mut first = true;
    for l in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(d) = l.strip_prefix("## ") {
            let d = chrono::NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d").map_err(|_| format!("{path}: a log's sections are \"## YYYY-MM-DD\"; not {l:?}"))?;
            if last.is_some_and(|p| d > p) {
                return Err(format!("{path}: a log's dates go newest first"));
            }
            last = Some(d);
        } else if first {
            return Err(format!("{path}: a log starts with a \"## YYYY-MM-DD\" section"));
        }
        first = false;
    }
    Ok(())
}

/// A document as OKF writes it: YAML frontmatter, then the body.
pub fn render(fm: &Value, body: &str) -> String {
    let empty = fm.as_object().is_none_or(Map::is_empty);
    if empty {
        return body.to_string();
    }
    let yaml = serde_yaml_ng::to_string(fm).unwrap_or_default();
    format!("---\n{yaml}---\n\n{body}")
}

impl Kernel {
    /// Whether `person` may see a document (K2): their own, or an org
    /// document whose every source they can read. Drawn from the bundle or
    /// from people's answers, everyone can. An item in one of the org's
    /// sources is decided by what the source said (K3, `check_sources`
    /// asks first); one whose source the kernel cannot ask (or has not yet
    /// asked) is seen only by the person whose run read it.
    pub fn bundle_visible(&self, person: &str, d: &crate::state::BundleDoc) -> bool {
        if d.scope == "user" {
            return d.owner_user.as_deref() == Some(person);
        }
        let ttl = std::time::Duration::from_secs(self.cfg.bundle.permission_seconds);
        let writer = d.writer.as_deref() == Some(person);
        d.sources.iter().all(|s| {
            if s.starts_with('/') || s.starts_with("human:") {
                return true;
            }
            match self.askable(s).then(|| crate::sources::permit(person, s, ttl)).flatten() {
                Some(ok) => ok,
                None => writer,
            }
        })
    }

    /// Every org document, its sources asked about for `person` (K3).
    async fn checked(&self, person: &str) {
        let docs: Vec<crate::state::BundleDoc> = self.read(|s| s.bundle.values().filter(|d| d.scope == "org").cloned().collect());
        self.check_sources(person, &docs).await;
    }

    /// A document for `person`: its frontmatter and body, links to what
    /// they cannot see removed, and the taint it was written under.
    pub async fn bundle_read(&self, person: &str, path: &str) -> anyhow::Result<Option<(Value, String, Label, u32)>> {
        self.checked(person).await;
        let Some(d) = self.read(|s| s.bundle.get(path).cloned()) else { return Ok(None) };
        if !self.bundle_visible(person, &d) {
            return Ok(None);
        }
        let c = self.log.contents(&[d.seq()]).await?.remove(&d.seq()).unwrap_or_default();
        let body = self.without_hidden_links(person, c["body"].as_str().unwrap_or_default());
        let mut fm = c["frontmatter"].clone();
        // An item it lists changed at its source (K10): the reader is told.
        if let (Some(at), Some(o)) = (d.stale_after, fm.as_object_mut()) {
            o.insert("stale_after".into(), json!(at.to_rfc3339()));
        }
        Ok(Some((fm, body, d.label, d.version())))
    }

    /// A body with every link to a document `person` cannot see turned into
    /// its text; a bullet whose links are all hidden is dropped (K2).
    pub fn without_hidden_links(&self, person: &str, body: &str) -> String {
        let hidden = |target: &str| self.read(|s| s.bundle.get(target).cloned()).is_some_and(|d| !self.bundle_visible(person, &d));
        let mut out = vec![];
        for line in body.lines() {
            let targets: Vec<String> = links(line).into_iter().filter(|t| t.starts_with('/')).collect();
            let gone: Vec<&String> = targets.iter().filter(|t| hidden(t)).collect();
            if !targets.is_empty() && gone.len() == targets.len() && (line.trim_start().starts_with("* ") || line.trim_start().starts_with("- ")) {
                continue;
            }
            let mut l = line.to_string();
            for t in gone {
                // [text](target) becomes text.
                while let Some(end) = l.find(&format!("]({t})")) {
                    let Some(start) = l[..end].rfind('[') else { break };
                    let text = l[start + 1..end].to_string();
                    l.replace_range(start..end + t.len() + 3, &text);
                }
            }
            out.push(l);
        }
        out.join("\n")
    }

    /// Write a version of a document for `person`'s run: checked first, and
    /// refused with the reason when it breaks a rule (K8, K9).
    pub async fn bundle_write(&self, person: &str, run: &str, path: &str, fm: &Value, body: &str) -> Result<u32, String> {
        let (kind, user) = place(path)?;
        if user.as_deref().is_some_and(|u| u != person) {
            return Err(format!("{path}: another person's document"));
        }
        let (current, existing) = self.read(|s| (s.bundle.get(path).map(|d| d.version()), s.bundle.get(path).cloned()));
        if existing.as_ref().is_some_and(|d| !self.bundle_visible(person, d)) {
            return Err(format!("{path}: a document you cannot see"));
        }
        // An entity is checked against its type document as it is now.
        let tslug = path.trim_start_matches('/').split('/').nth(1).unwrap_or_default().to_string();
        let tdoc = match (kind, self.read(|s| s.bundle.get(&format!("/types/{tslug}.md")).cloned())) {
            ("entity", Some(d)) => self.log.contents(&[d.seq()]).await.map_err(|e| format!("{e:#}"))?.remove(&d.seq()).map(|c| c["frontmatter"].clone()),
            _ => None,
        };
        validate(path, fm, body, |_| tdoc.clone())?;
        // A type becomes a Cedar entity: only an admin makes one stable.
        let was_stable = match &existing {
            Some(d) if kind == "type" => self.log.contents(&[d.seq()]).await.map_err(|e| format!("{e:#}"))?.remove(&d.seq()).is_some_and(|c| c["frontmatter"]["status"] == "stable"),
            _ => false,
        };
        if kind == "type" && fm["status"] == "stable" && !was_stable {
            return Err(format!("{path}: a type becomes stable only with an admin's approval in the console; write it as draft"));
        }
        let label = self.read(|s| s.runs.get(run).map(|r| Label { untrusted: r.untrusted, private: r.private })).unwrap_or(Label::PRINCIPAL);
        let sources: Vec<String> = fm["sources"].as_array().into_iter().flatten().filter_map(|x| x["resource"].as_str().map(String::from)).collect();
        let version = current.unwrap_or(0) + 1;
        let ev = crate::events::Ev::BundleWritten {
            path: path.into(),
            version,
            doc_kind: kind.into(),
            scope: if user.is_some() { "user".into() } else { "org".into() },
            owner_user: user,
            actor: format!("magnus/{}", crate::update::version()),
            run: Some(run.into()),
            label,
            sources,
            frontmatter: fm.clone(),
            body: body.into(),
        };
        self.emit("kernel", ev).await.map_err(|e| format!("{e:#}"))?;
        Ok(version)
    }

    /// Documents `person` can see whose path, title, description or body
    /// has every word of `query`, optionally only some kinds.
    pub async fn bundle_search(&self, person: &str, query: &str, kinds: &[String], limit: usize) -> anyhow::Result<(Vec<Value>, Label)> {
        self.checked(person).await;
        let docs: Vec<crate::state::BundleDoc> = self.read(|s| s.bundle.values().cloned().collect());
        let seen: Vec<crate::state::BundleDoc> = docs.into_iter().filter(|d| (kinds.is_empty() || kinds.contains(&d.kind)) && self.bundle_visible(person, d)).collect();
        let contents = self.log.contents(&seen.iter().map(|d| d.seq()).collect::<Vec<_>>()).await?;
        let words: Vec<String> = query.to_lowercase().split_whitespace().map(String::from).collect();
        let mut label = Label::PRINCIPAL;
        let mut hits = vec![];
        for d in &seen {
            let c = contents.get(&d.seq()).cloned().unwrap_or_default();
            let hay = format!("{} {} {} {}", d.path, c["frontmatter"]["title"].as_str().unwrap_or_default(), c["frontmatter"]["description"].as_str().unwrap_or_default(), c["body"].as_str().unwrap_or_default()).to_lowercase();
            if words.iter().all(|w| hay.contains(w)) {
                label.untrusted |= d.label.untrusted;
                label.private |= d.label.private;
                hits.push(json!({ "path": d.path, "kind": d.kind, "title": c["frontmatter"]["title"], "description": c["frontmatter"]["description"], "status": c["frontmatter"]["status"], "version": d.version() }));
            }
            if hits.len() >= limit {
                break;
            }
        }
        Ok((hits, label))
    }
}

impl Kernel {
    /// `entity.resolve`: entity documents `person` can see whose identity
    /// has `key` among its values, or whose title is `name` (any case),
    /// optionally of one type, each with its identity (K2 decides).
    pub async fn entity_resolve(&self, person: &str, key: &str, name: &str, kind: &str) -> anyhow::Result<(Vec<Value>, Label)> {
        let (key, name, kind) = (key.trim().to_lowercase(), name.trim().to_lowercase(), kind.trim().to_lowercase());
        self.checked(person).await;
        let docs: Vec<crate::state::BundleDoc> = self.read(|s| s.bundle.values().filter(|d| d.kind == "entity").cloned().collect());
        let seen: Vec<crate::state::BundleDoc> = docs.into_iter().filter(|d| self.bundle_visible(person, d)).collect();
        let contents = self.log.contents(&seen.iter().map(|d| d.seq()).collect::<Vec<_>>()).await?;
        let mut label = Label::PRINCIPAL;
        let mut out = vec![];
        for d in &seen {
            let Some(c) = contents.get(&d.seq()) else { continue };
            let fm = &c["frontmatter"];
            if !kind.is_empty() && fm["type"].as_str().is_none_or(|t| t.to_lowercase() != kind) {
                continue;
            }
            let ids: Vec<String> = fm["identity"].as_object().into_iter().flatten().map(|(_, v)| v.as_str().map(String::from).unwrap_or_else(|| v.to_string()).to_lowercase()).collect();
            let by_key = !key.is_empty() && ids.iter().any(|v| *v == key);
            let by_name = !name.is_empty() && fm["title"].as_str().is_some_and(|t| t.to_lowercase() == name);
            if by_key || by_name {
                label = label.join(d.label);
                out.push(json!({ "path": d.path, "type": fm["type"], "title": fm["title"], "identity": fm["identity"], "resource": fm["resource"], "version": d.version() }));
            }
        }
        Ok((out, label))
    }
}

/// Rendered indexes, per person and per the documents they were rendered
/// from: any write to the bundle makes a new key, so nothing stale is reused.
fn index_cache() -> &'static std::sync::Mutex<std::collections::HashMap<(String, i64, i64, i64), String>> {
    static C: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<(String, i64, i64, i64), String>>> = std::sync::OnceLock::new();
    C.get_or_init(Default::default)
}

impl Kernel {
    /// The top index as `person` may see it, with their own memory index
    /// after it (org context build spec, Context assembly): each as it stood
    /// at `at`, the run's start, so a run's context only grows at its end
    /// (C1). Lines whose links are all to what they cannot see are dropped,
    /// and such links removed (K2). Empty when there is no index yet.
    pub async fn rendered_index(&self, person: &str, at: chrono::DateTime<chrono::Utc>) -> anyhow::Result<String> {
        let as_of = |path: &str| self.read(|s| s.bundle.get(path).and_then(|d| d.versions.iter().rev().find(|v| v.2 <= at).map(|v| v.1)));
        let root = as_of("/index.md").unwrap_or(0);
        let mine = as_of(&format!("/users/{person}/index.md")).unwrap_or(0);
        if root == 0 && mine == 0 {
            return Ok(String::new());
        }
        self.checked(person).await;
        // Any write, or any change in what a source says someone can read,
        // renders afresh.
        let generation = self.read(|s| s.bundle.values().map(|d| d.seq()).max().unwrap_or(0)) + crate::sources::permits_generation() as i64 * 1_000_000_000;
        let key = (person.to_string(), root, mine, generation);
        if let Some(hit) = index_cache().lock().unwrap().get(&key) {
            return Ok(hit.clone());
        }
        let seqs: Vec<i64> = [root, mine].into_iter().filter(|s| *s > 0).collect();
        let c = self.log.contents(&seqs).await?;
        let body = |seq: i64| c.get(&seq).and_then(|v| v["body"].as_str().map(String::from)).unwrap_or_default();
        let mut out = vec![];
        if root > 0 {
            out.push(self.without_hidden_links(person, &body(root)));
        }
        if mine > 0 {
            out.push(format!("Your notes for this person (/users/{person}/index.md):\n{}", self.without_hidden_links(person, &body(mine))));
        }
        let text = out.join("\n\n").trim().to_string();
        let mut cache = index_cache().lock().unwrap();
        if cache.len() > 1000 {
            cache.clear();
        }
        cache.insert(key, text.clone());
        Ok(text)
    }

    /// One of the bundle tools, for the run's person: what it says back,
    /// whether it is an error, and the taint of what it returned (read by
    /// the boundary, as for a knowledge search).
    pub async fn bundle_tool(&self, run: &str, tool: &str, input: &Value) -> anyhow::Result<(String, bool, Option<Value>)> {
        let person = self.read(|s| s.runs.get(run).map(|r| r.person.clone())).unwrap_or_default();
        let s = |k: &str| input[k].as_str().unwrap_or_default().to_string();
        Ok(match tool {
            "bundle_read" => match self.bundle_read(&person, &s("path")).await? {
                Some((fm, body, label, version)) => {
                    // A document drawn from source items tells what they say:
                    // they count among what the run read (K5).
                    let items: Vec<String> = fm["sources"].as_array().into_iter().flatten().filter_map(|x| x["resource"].as_str()).filter(|r| self.askable(r)).map(String::from).collect();
                    (render(&fm, &body), false, Some(json!({ "path": s("path"), "version": version, "untrusted": label.untrusted, "private": label.private, "items": items })))
                }
                None => (format!("{}: no such document, or not one the person can see", s("path")), true, Some(json!({ "untrusted": false, "private": false }))),
            },
            "entity_resolve" => {
                if s("key").trim().is_empty() && s("name").trim().is_empty() {
                    return Ok(("Give a key (an identity value such as a domain or an id) or a name.".into(), true, Some(json!({ "untrusted": false, "private": false }))));
                }
                let (found, label) = self.entity_resolve(&person, &s("key"), &s("name"), &s("type")).await?;
                let text = if found.is_empty() { "No entity the person can see matches.".to_string() } else { json!({ "entities": found }).to_string() };
                (text, false, Some(json!({ "entities": found, "untrusted": label.untrusted, "private": label.private })))
            }
            "bundle_search" => {
                let kinds: Vec<String> = input["kinds"].as_array().into_iter().flatten().filter_map(|k| k.as_str().map(String::from)).collect();
                let limit = input["limit"].as_u64().unwrap_or(20).clamp(1, 100) as usize;
                let (hits, label) = self.bundle_search(&person, &s("query"), &kinds, limit).await?;
                (json!({ "hits": hits }).to_string(), false, Some(json!({ "hits": hits, "untrusted": label.untrusted, "private": label.private })))
            }
            _ => match self.bundle_write(&person, run, &s("path"), &input["frontmatter"], &s("body")).await {
                Ok(v) => (format!("wrote {} (version {v})", s("path")), false, Some(json!({ "path": s("path"), "version": v }))),
                Err(why) => (format!("Not written: {why}"), true, None),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gen() -> Value {
        json!({ "by": "magnus/0.2.4", "at": "2026-11-02T10:00:00+00:00" })
    }

    fn customer_type(_: &str) -> Option<Value> {
        Some(json!({ "title": "Customer", "identity_keys": ["hubspot_id", "domain"], "relations": ["owned_by", { "type": "parent" }] }))
    }

    #[test]
    fn every_place_has_its_kind() {
        assert_eq!(place("/index.md").unwrap().0, "index");
        assert_eq!(place("/sales/log.md").unwrap().0, "log");
        assert_eq!(place("/sources/hubspot.md").unwrap().0, "source");
        assert_eq!(place("/types/customer.md").unwrap().0, "type");
        assert_eq!(place("/entities/customer/acme.md").unwrap().0, "entity");
        assert_eq!(place("/conventions/sales/discounts.md").unwrap().0, "convention");
        assert_eq!(place("/people/finance.md").unwrap().0, "people");
        assert_eq!(place("/users/alice/notes.md").unwrap(), ("user", Some("alice".into())));
        for bad in ["index.md", "/a/../b.md", "/x.txt", "/random.md", "/log.md", "/users/notes.md", "/types/a b.md", "/entities/customer.md"] {
            assert!(place(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_malformed_document_is_refused_with_the_reason() {
        let no_type = validate("/sources/hubspot.md", &json!({ "title": "HubSpot" }), "", customer_type).unwrap_err();
        assert!(no_type.contains("no type"), "{no_type}");
        let fm = json!({ "type": "Source", "title": "HubSpot", "description": "CRM", "resource": "hubspot://acct/1", "sources": [{ "resource": "hubspot://acct/1" }], "generated": { "by": "magnus/0.2.4", "at": "2026-11-02 10:00" }, "scope": "org" });
        let bad_time = validate("/sources/hubspot.md", &fm, "", customer_type).unwrap_err();
        assert!(bad_time.contains("generated.at"), "{bad_time}");
        let mut ok = fm.clone();
        ok["generated"] = gen();
        validate("/sources/hubspot.md", &ok, "Search by company domain.", customer_type).unwrap();
        let mut actor = ok.clone();
        actor["generated"]["by"] = json!("Magnus");
        assert!(validate("/sources/hubspot.md", &actor, "", customer_type).unwrap_err().contains("actor"));
        let entity = |identity: Value, relations: Value| json!({ "type": "Customer", "title": "Acme", "resource": "hubspot://company/42", "sources": [{ "resource": "hubspot://company/42" }], "generated": gen(), "identity": identity, "relations": relations, "scope": "org" });
        validate("/entities/customer/acme.md", &entity(json!({ "domain": "acme.example" }), json!([{ "type": "owned_by", "to": "/people/sales.md" }])), "", customer_type).unwrap();
        let key = validate("/entities/customer/acme.md", &entity(json!({ "vat": "1" }), json!([])), "", customer_type).unwrap_err();
        assert!(key.contains("identity key \"vat\""), "{key}");
        let rel = validate("/entities/customer/acme.md", &entity(json!({ "domain": "acme.example" }), json!([{ "type": "competes_with", "to": "/entities/customer/b.md" }])), "", customer_type).unwrap_err();
        assert!(rel.contains("competes_with"), "{rel}");
        assert!(validate("/entities/customer/acme.md", &entity(json!({}), json!([])), "", |_| None).unwrap_err().contains("not in the bundle"));
        // Relative links are not the profile's.
        assert!(validate("/sources/hubspot.md", &ok, "See [types](../types/customer.md).", customer_type).unwrap_err().contains("link"));
        // Scope follows the path.
        let mut user = ok.clone();
        user["scope"] = json!("user");
        assert!(validate("/sources/hubspot.md", &user, "", customer_type).unwrap_err().contains("scope: org"));
        assert!(validate("/users/alice/n.md", &json!({ "type": "Note", "scope": "user", "owner_user": "bob" }), "", customer_type).is_err());
        validate("/users/alice/n.md", &json!({ "type": "Note", "scope": "user", "owner_user": "alice" }), "", customer_type).unwrap();
    }

    #[test]
    fn indexes_and_logs_keep_their_shape_and_the_root_index_its_size() {
        validate("/index.md", &json!({ "okf_version": "0.2" }), "# Sales\n* [HubSpot](/sources/hubspot.md) - the CRM\n", customer_type).unwrap();
        assert!(validate("/sales/index.md", &json!({ "okf_version": "0.2" }), "# Sales\n", customer_type).is_err());
        assert!(validate("/index.md", &json!({}), "Some prose.\n", customer_type).unwrap_err().contains("headings and bulleted links"));
        let big = format!("# All\n{}", "* [x](/sources/x.md) - a source\n".repeat(600));
        let e = validate("/index.md", &json!({}), &big, customer_type).unwrap_err();
        assert!(e.contains("16KB") && e.contains(&big.len().to_string()), "{e}");
        validate("/sales/log.md", &json!({}), "## 2026-11-02\nAdded HubSpot.\n## 2026-11-01\nStarted.\n", customer_type).unwrap();
        assert!(validate("/sales/log.md", &json!({}), "## 2026-11-01\nA.\n## 2026-11-02\nB.\n", customer_type).unwrap_err().contains("newest first"));
    }

    #[test]
    fn actors_are_okf_forms() {
        for ok in ["human:alice", "process:curator", "magnus/0.2.4"] {
            assert!(actor_ok(ok), "{ok}");
        }
        for bad in ["alice", "human:", "robot:x", "magnus/", "/0.2", "magnus 0.2/x"] {
            assert!(!actor_ok(bad), "{bad}");
        }
    }
}
