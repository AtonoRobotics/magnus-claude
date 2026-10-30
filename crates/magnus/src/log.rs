//! The event log: one append-only Postgres table, the source of truth for
//! what happened, and beside it the content store, `content`, holding what
//! people and sources wrote (`events::CONTENT`), where it can be deleted.
//!
//! `migrate` runs with an administrator URL and creates the tables and the
//! kernel's role (and the knowledge base's index, `knowledge::migrate_sql`).
//! On `events` the role may INSERT and SELECT and nothing else, so the
//! database itself refuses rewrites of history. Everything else the kernel
//! knows is a projection of the log and its content, rebuilt on boot.

use crate::events::Ev;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio_postgres::{Client, NoTls};

#[derive(Clone, Debug)]
pub struct Event {
    pub seq: i64,
    pub at: DateTime<Utc>,
    pub actor: String,
    pub ev: Ev,
}

pub async fn connect(url: &str) -> Result<Client> {
    let (client, conn) = tokio_postgres::connect(url, NoTls).await.with_context(|| "connect to Postgres")?;
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            tracing::error!("postgres connection: {e}");
        }
    });
    Ok(client)
}

fn ident(s: &str) -> Result<&str> {
    anyhow::ensure!(
        !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
        "unsafe SQL identifier {s:?}"
    );
    Ok(s)
}

/// Create the log table and the kernel role named in `kernel_url`.
pub async fn migrate(admin_url: &str, kernel_url: &str) -> Result<()> {
    let k = url::Url::parse(kernel_url).context("database.url")?;
    let role = ident(k.username())?.to_string();
    let password = k.password().context("database.url has no password")?.replace('\'', "''");
    let db = ident(k.path().trim_start_matches('/'))?.to_string();
    let admin = connect(admin_url).await?;
    let exists = admin.query_opt("SELECT 1 FROM pg_roles WHERE rolname = $1", &[&role]).await?.is_some();
    if exists {
        admin.batch_execute(&format!("ALTER ROLE {role} LOGIN PASSWORD '{password}'")).await?;
    } else {
        admin
            .batch_execute(&format!("CREATE ROLE {role} LOGIN PASSWORD '{password}' NOSUPERUSER NOCREATEDB NOCREATEROLE"))
            .await?;
    }
    admin
        .batch_execute(&format!(
            "CREATE TABLE IF NOT EXISTS public.events (
                seq    bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                at     timestamptz NOT NULL DEFAULT now(),
                kind   text NOT NULL,
                actor  text NOT NULL,
                run_id text,
                body   jsonb NOT NULL
             );
             CREATE INDEX IF NOT EXISTS events_run ON public.events (run_id, seq);
             REVOKE ALL ON public.events FROM PUBLIC;
             REVOKE ALL ON public.events FROM {role};
             GRANT CONNECT ON DATABASE {db} TO {role};
             GRANT USAGE ON SCHEMA public TO {role};
             REVOKE CREATE ON SCHEMA public FROM PUBLIC;
             GRANT SELECT, INSERT ON public.events TO {role};
             CREATE TABLE IF NOT EXISTS public.content (
                seq  bigint PRIMARY KEY REFERENCES public.events (seq),
                salt bytea NOT NULL,
                body jsonb NOT NULL
             );
             REVOKE ALL ON public.content FROM PUBLIC;
             REVOKE ALL ON public.content FROM {role};
             GRANT SELECT, INSERT ON public.content TO {role};"
        ))
        .await?;
    // The knowledge base's index, filtered for each reader by row security.
    admin.batch_execute(&crate::knowledge::migrate_sql(&role)).await.context("create the knowledge index (pgvector must be installed)")?;
    Ok(())
}

/// One event's content, apart from its record.
pub struct Content {
    /// Random, kept with the content and deleted with it, so the hash left in
    /// the log cannot be matched by guessing short content once it is gone.
    pub salt: Vec<u8>,
    pub body: Value,
}

/// Split an event's body into the record the log keeps and its content,
/// leaving in the record the content's size and salted SHA-256.
pub fn split(mut body: Value) -> (Value, Option<Content>) {
    let kind = body["kind"].as_str().unwrap_or_default().to_string();
    let mut taken = serde_json::Map::new();
    for f in crate::events::content_fields(&kind) {
        let (outer, inner) = match f.split_once('.') {
            Some((o, i)) => (body.get_mut(o), i),
            None => (Some(&mut body), *f),
        };
        let Some(o) = outer.and_then(Value::as_object_mut) else { continue };
        if o.get(inner).is_some_and(|v| !v.is_null()) {
            taken.insert(f.to_string(), o.remove(inner).unwrap());
        }
    }
    if taken.is_empty() {
        return (body, None);
    }
    let content = Content { salt: rand::random::<[u8; 32]>().to_vec(), body: Value::Object(taken) };
    let bytes = serde_json::to_vec(&content.body).expect("JSON serializes");
    body["content"] = json!({ "sha256": hash(&content.salt, &bytes), "bytes": bytes.len() });
    (body, Some(content))
}

fn hash(salt: &[u8], bytes: &[u8]) -> String {
    hex::encode(Sha256::new().chain_update(salt).chain_update(bytes).finalize())
}

/// An event's body as it was written: its record with its content put back.
/// Without its content (deleted), the record alone.
pub fn join(mut body: Value, content: Option<Value>) -> Value {
    let Some(Value::Object(c)) = content else { return body };
    if let Some(o) = body.as_object_mut() {
        o.remove("content");
    }
    for (f, v) in c {
        match f.split_once('.') {
            Some((outer, inner)) => {
                if let Some(o) = body.get_mut(outer).and_then(Value::as_object_mut) {
                    o.insert(inner.into(), v);
                }
            }
            None => body[f] = v,
        }
    }
    body
}

/// The event log. A lost connection (Postgres restarting) is opened again
/// on the next use, with the writer's lock taken again; until then every
/// write fails and health says so.
pub struct Log {
    url: String,
    writer: bool,
    /// The content store exists. A release installed without `migrate`
    /// since has none yet, and keeps content in the log as before.
    store: bool,
    client: tokio::sync::RwLock<Client>,
    /// The last event this process wrote or read, to find one that was
    /// written while its answer was lost.
    last: std::sync::atomic::AtomicI64,
}

/// One kernel per log: a second one would break the single-writer rule.
async fn hold(client: &Client) -> Result<()> {
    let got: bool = client.query_one("SELECT pg_try_advisory_lock(7239001)", &[]).await?.get(0);
    anyhow::ensure!(got, "another kernel holds this event log");
    Ok(())
}

impl Log {
    async fn new(url: &str, writer: bool, client: Client) -> Result<Log> {
        let store: bool = client.query_one("SELECT to_regclass('public.content') IS NOT NULL", &[]).await?.get(0);
        if !store {
            tracing::warn!("no content store: content stays in the event log until `magnus migrate` runs");
        }
        Ok(Log { url: url.into(), writer, store, client: tokio::sync::RwLock::new(client), last: 0.into() })
    }

    pub async fn open(url: &str) -> Result<Log> {
        let client = connect(url).await?;
        hold(&client).await?;
        Log::new(url, true, client).await
    }

    /// The events joined to their content, as `e` and `c`.
    fn rows(&self) -> &'static str {
        match self.store {
            true => "public.events e LEFT JOIN public.content c USING (seq)",
            false => "public.events e CROSS JOIN (SELECT NULL::bytea AS salt, NULL::jsonb AS body) c",
        }
    }

    fn split(&self, ev: &Ev) -> Result<(Value, Option<Content>)> {
        let body = serde_json::to_value(ev)?;
        Ok(if self.store { split(body) } else { (body, None) })
    }

    /// Whether the log can be written: the connection is up, or opens again.
    pub async fn up(&self) -> bool {
        self.client().await.is_ok()
    }

    /// The connection, opened again if it was lost.
    async fn client(&self) -> Result<tokio::sync::RwLockReadGuard<'_, Client>> {
        {
            let c = self.client.read().await;
            if !c.is_closed() {
                return Ok(c);
            }
        }
        let mut w = self.client.write().await;
        if w.is_closed() {
            let c = connect(&self.url).await?;
            if self.writer {
                hold(&c).await?;
                // An append whose answer was lost may be in the log but not
                // in this process's state: start again from the log.
                let max: i64 = c.query_one("SELECT coalesce(max(seq), 0) FROM public.events", &[]).await?.get(0);
                if max != self.last.load(std::sync::atomic::Ordering::SeqCst) {
                    tracing::error!("the log has events this process did not apply; restarting from the log");
                    c.batch_execute("SELECT pg_advisory_unlock_all()").await?;
                    crate::update::restart_in_place();
                }
            }
            tracing::warn!("the event log's connection was lost and is open again");
            *w = c;
        }
        Ok(w.downgrade())
    }

    fn saw(&self, seq: i64) {
        self.last.fetch_max(seq, std::sync::atomic::Ordering::SeqCst);
    }

    /// Open, waiting up to `wait` for another kernel to let go of the log.
    pub async fn open_waiting(url: &str, wait: std::time::Duration) -> Result<Log> {
        let start = std::time::Instant::now();
        loop {
            match Log::open(url).await {
                Err(e) if start.elapsed() < wait => {
                    tracing::info!("waiting for the log: {e:#}");
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
                r => return r,
            }
        }
    }

    /// Read the log without holding it, as a release about to be switched
    /// to does while the running one keeps writing.
    pub async fn read_only(url: &str) -> Result<Log> {
        Log::new(url, false, connect(url).await?).await
    }

    /// One event, and its content beside it, in one statement.
    pub async fn append(&self, actor: &str, ev: &Ev) -> Result<Event> {
        let (body, content) = self.split(ev)?;
        let kind = body["kind"].as_str().unwrap_or_default().to_string();
        let run = ev.run().map(String::from);
        let (salt, content) = content.map(|c| (c.salt, c.body)).unzip();
        let c = self.client().await?;
        let row = match self.store {
            true => c.query_one(
                "WITH e AS (INSERT INTO public.events (kind, actor, run_id, body) VALUES ($1, $2, $3, $4) RETURNING seq, at),
                      c AS (INSERT INTO public.content (seq, salt, body) SELECT seq, $5, $6 FROM e WHERE $6::jsonb IS NOT NULL)
                 SELECT seq, at FROM e",
                &[&kind, &actor, &run, &body, &salt, &content],
            )
            .await,
            false => c.query_one(
                "INSERT INTO public.events (kind, actor, run_id, body) VALUES ($1, $2, $3, $4) RETURNING seq, at",
                &[&kind, &actor, &run, &body],
            )
            .await,
        }
        .context("append to the event log")?;
        self.saw(row.get(0));
        Ok(Event { seq: row.get(0), at: row.get(1), actor: actor.into(), ev: ev.clone() })
    }

    /// Several events in one statement: all are in the log or none is.
    pub async fn append_all(&self, actor: &str, evs: &[Ev]) -> Result<Vec<Event>> {
        let (bodies, contents): (Vec<Value>, Vec<Option<Content>>) = evs.iter().map(|e| self.split(e)).collect::<Result<Vec<_>>>()?.into_iter().unzip();
        let (salts, contents): (Vec<Option<Vec<u8>>>, Vec<Option<Value>>) = contents.into_iter().map(|c| c.map(|c| (c.salt, c.body)).unzip()).unzip();
        let kinds: Vec<String> = bodies.iter().map(|b| b["kind"].as_str().unwrap_or_default().to_string()).collect();
        let runs: Vec<Option<String>> = evs.iter().map(|e| e.run().map(String::from)).collect();
        let c = self.client().await?;
        // Identities are given in insert order, so the n-th seq is the n-th event.
        let rows = match self.store {
            true => c.query(
                "WITH e AS (INSERT INTO public.events (kind, actor, run_id, body)
                            SELECT k, $2, r, b FROM unnest($1::text[], $3::text[], $4::jsonb[]) WITH ORDINALITY AS e(k, r, b, n) ORDER BY n
                            RETURNING seq, at),
                      o AS (SELECT seq, at, row_number() OVER (ORDER BY seq) AS n FROM e),
                      c AS (INSERT INTO public.content (seq, salt, body)
                            SELECT o.seq, x.s, x.b FROM o JOIN unnest($5::bytea[], $6::jsonb[]) WITH ORDINALITY AS x(s, b, n) ON x.n = o.n
                            WHERE x.b IS NOT NULL)
                 SELECT seq, at FROM o",
                &[&kinds, &actor, &runs, &bodies, &salts, &contents],
            )
            .await,
            false => c.query(
                "INSERT INTO public.events (kind, actor, run_id, body)
                 SELECT k, $2, r, b FROM unnest($1::text[], $3::text[], $4::jsonb[]) WITH ORDINALITY AS e(k, r, b, n) ORDER BY n
                 RETURNING seq, at",
                &[&kinds, &actor, &runs, &bodies],
            )
            .await,
        }
        .context("append to the event log")?;
        let mut at: Vec<(i64, chrono::DateTime<chrono::Utc>)> = rows.iter().map(|r| (r.get(0), r.get(1))).collect();
        at.sort_by_key(|(seq, _)| *seq);
        if let Some((seq, _)) = at.last() {
            self.saw(*seq);
        }
        Ok(at.into_iter().zip(evs).map(|((seq, at), ev)| Event { seq, at, actor: actor.into(), ev: ev.clone() }).collect())
    }

    /// One delivered file's content, read straight from the log and the store.
    pub async fn output(&self, run: &str, id: &str) -> Result<Option<(String, String, Vec<u8>)>> {
        let sql = format!("SELECT e.body, c.body FROM {} WHERE e.run_id = $1 AND e.kind = 'output' AND e.body->>'id' = $2", self.rows());
        let row = self.client().await?.query_opt(&sql, &[&run, &id]).await?;
        let Some(row) = row else { return Ok(None) };
        let b = join(row.get(0), row.get(1));
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(b["data_b64"].as_str().unwrap_or_default())?;
        Ok(Some((b["name"].as_str().unwrap_or("file").into(), b["media_type"].as_str().unwrap_or("application/octet-stream").into(), bytes)))
    }

    /// One run's events in order: the developer view's full trace (V4).
    pub async fn run_events(&self, run: &str) -> Result<Vec<Event>> {
        let sql = format!("SELECT e.seq, e.at, e.actor, e.body, c.body FROM {} WHERE e.run_id = $1 ORDER BY e.seq", self.rows());
        let rows = self.client().await?.query(&sql, &[&run]).await?;
        rows.into_iter()
            .map(|r| {
                let ev: Ev = serde_json::from_value(join(r.get(3), r.get(4)))?;
                Ok(Event { seq: r.get(0), at: r.get(1), actor: r.get(2), ev })
            })
            .collect()
    }

    /// Every row as stored, with its content and salt, for a backup.
    pub async fn dump(&self) -> Result<Vec<Value>> {
        let sql = format!("SELECT e.seq, e.at, e.kind, e.actor, e.run_id, e.body, c.salt, c.body FROM {} ORDER BY e.seq", self.rows());
        let rows = self.client().await?.query(&sql, &[]).await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let at: DateTime<Utc> = r.get(1);
                let salt: Option<Vec<u8>> = r.get(6);
                json!({ "seq": r.get::<_, i64>(0), "at": at.to_rfc3339(), "kind": r.get::<_, String>(2), "actor": r.get::<_, String>(3), "run_id": r.get::<_, Option<String>>(4), "body": r.get::<_, Value>(5), "salt": salt.map(hex::encode), "content": r.get::<_, Option<Value>>(7) })
            })
            .collect())
    }

    pub async fn load(&self) -> Result<Vec<Event>> {
        let sql = format!("SELECT e.seq, e.at, e.actor, e.body, c.body FROM {} ORDER BY e.seq", self.rows());
        let rows = self.client().await?.query(&sql, &[]).await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let body = join(r.get(3), r.get(4));
            let seq: i64 = r.get(0);
            self.saw(seq);
            let ev: Ev = match serde_json::from_value(body.clone()) {
                Ok(ev) => ev,
                // Written by a later release this one was rolled back from:
                // a kind of event it does not know, so nothing it acts on.
                Err(e) if e.to_string().starts_with("unknown variant") => {
                    tracing::warn!("event {seq} is a kind this release does not know ({}); skipped", body["kind"]);
                    continue;
                }
                Err(e) => return Err(anyhow::Error::from(e).context(format!("event {seq} does not parse"))),
            };
            out.push(Event { seq, at: r.get(1), actor: r.get(2), ev });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_leaves_the_record_and_joins_back_as_written() {
        let ev = serde_json::json!({ "kind": "memory", "id": "m1", "person": "alice", "run": "r1", "fact": "Salary is 150k.", "topic": null, "label": { "untrusted": false, "private": true } });
        let (record, content) = split(ev.clone());
        let content = content.expect("a memory has content");
        assert!(!record.to_string().contains("150k"), "the fact stayed in the log: {record}");
        assert_eq!(record["topic"], Value::Null, "a field without content stays as it was");
        let bytes = serde_json::to_vec(&content.body).unwrap();
        assert_eq!(record["content"]["sha256"], hash(&content.salt, &bytes));
        assert_ne!(record["content"]["sha256"], hex::encode(Sha256::digest(&bytes)), "the hash is not salted");
        assert_eq!(join(record.clone(), Some(content.body)), ev);
        assert_eq!(join(record.clone(), None), record, "deleted content leaves the record");

        let nested = serde_json::json!({ "kind": "task_created", "text": "Plan the offsite.", "chat": { "thread": "t1", "turn": false, "conversation": ["earlier"] } });
        let (record, content) = split(nested.clone());
        assert_eq!(record["chat"], serde_json::json!({ "thread": "t1", "turn": false }));
        assert_eq!(join(record, content.map(|c| c.body)), nested);

        let plain = serde_json::json!({ "kind": "resumed", "run": "r1" });
        assert!(matches!(split(plain.clone()), (r, None) if r == plain), "an event without content is stored whole");
    }
}
