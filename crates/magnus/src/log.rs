//! The event log: one append-only Postgres table, the only source of truth.
//!
//! `migrate` runs with an administrator URL and creates the table and the
//! kernel's role (and the knowledge base's index, `knowledge::migrate_sql`). The role may INSERT and SELECT and nothing else, so the
//! database itself refuses rewrites of history. Everything else the kernel
//! knows is a projection of these rows, rebuilt on boot.

use crate::events::Ev;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
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
             GRANT SELECT, INSERT ON public.events TO {role};"
        ))
        .await?;
    // The knowledge base's index, filtered for each reader by row security.
    admin.batch_execute(&crate::knowledge::migrate_sql(&role)).await.context("create the knowledge index (pgvector must be installed)")?;
    Ok(())
}

/// The event log. A lost connection (Postgres restarting) is opened again
/// on the next use, with the writer's lock taken again; until then every
/// write fails and health says so.
pub struct Log {
    url: String,
    writer: bool,
    client: tokio::sync::RwLock<Client>,
    /// The last event this process wrote or read, to find one that was
    /// written while its answer was lost.
    last: std::sync::atomic::AtomicI64,
}

/// One kernel per log: a second one would break the single-writer rule.
pub(crate) async fn hold(client: &Client) -> Result<()> {
    let got: bool = client.query_one("SELECT pg_try_advisory_lock(7239001)", &[]).await?.get(0);
    anyhow::ensure!(got, "another kernel holds this event log");
    Ok(())
}

impl Log {
    fn new(url: &str, writer: bool, client: Client) -> Log {
        Log { url: url.into(), writer, client: tokio::sync::RwLock::new(client), last: 0.into() }
    }

    pub async fn open(url: &str) -> Result<Log> {
        let client = connect(url).await?;
        hold(&client).await?;
        Ok(Log::new(url, true, client))
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

    /// The writer's appends are serialized, so the first seq of a new
    /// append follows the last one this process applied. A gap that holds
    /// events means an append committed whose caller never applied it (its
    /// future was dropped mid-statement): start again from the log. A gap
    /// without events is only the sequence skipping a failed insert.
    async fn check_contiguous(&self, first: i64) -> Result<()> {
        let last = self.last.load(std::sync::atomic::Ordering::SeqCst);
        if !self.writer || first <= last + 1 {
            return Ok(());
        }
        let c = self.client().await?;
        let missed: i64 = c.query_one("SELECT count(*) FROM public.events WHERE seq > $1 AND seq < $2", &[&last, &first]).await?.get(0);
        if missed > 0 {
            tracing::error!("the log has {missed} events this process did not apply; restarting from the log");
            c.batch_execute("SELECT pg_advisory_unlock_all()").await?;
            crate::update::restart_in_place();
        }
        Ok(())
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
        Ok(Log::new(url, false, connect(url).await?))
    }

    pub async fn append(&self, actor: &str, ev: &Ev) -> Result<Event> {
        let body = serde_json::to_value(ev)?;
        let kind = body["kind"].as_str().unwrap_or_default().to_string();
        let run = ev.run().map(String::from);
        let row = self
            .client()
            .await?
            .query_one(
                "INSERT INTO public.events (kind, actor, run_id, body) VALUES ($1, $2, $3, $4) RETURNING seq, at",
                &[&kind, &actor, &run, &body],
            )
            .await
            .context("append to the event log")?;
        self.check_contiguous(row.get(0)).await?;
        self.saw(row.get(0));
        Ok(Event { seq: row.get(0), at: row.get(1), actor: actor.into(), ev: ev.clone() })
    }

    /// Several events in one statement: all are in the log or none is.
    pub async fn append_all(&self, actor: &str, evs: &[Ev]) -> Result<Vec<Event>> {
        let bodies = evs.iter().map(serde_json::to_value).collect::<Result<Vec<Value>, _>>()?;
        let kinds: Vec<String> = bodies.iter().map(|b| b["kind"].as_str().unwrap_or_default().to_string()).collect();
        let runs: Vec<Option<String>> = evs.iter().map(|e| e.run().map(String::from)).collect();
        let rows = self
            .client()
            .await?
            .query(
                "INSERT INTO public.events (kind, actor, run_id, body)
                 SELECT k, $2, r, b FROM unnest($1::text[], $3::text[], $4::jsonb[]) WITH ORDINALITY AS e(k, r, b, n) ORDER BY n
                 RETURNING seq, at",
                &[&kinds, &actor, &runs, &bodies],
            )
            .await
            .context("append to the event log")?;
        let mut at: Vec<(i64, chrono::DateTime<chrono::Utc>)> = rows.iter().map(|r| (r.get(0), r.get(1))).collect();
        at.sort_by_key(|(seq, _)| *seq);
        if let Some((seq, _)) = at.first() {
            self.check_contiguous(*seq).await?;
        }
        if let Some((seq, _)) = at.last() {
            self.saw(*seq);
        }
        Ok(at.into_iter().zip(evs).map(|((seq, at), ev)| Event { seq, at, actor: actor.into(), ev: ev.clone() }).collect())
    }

    /// One delivered file's content, read straight from the log.
    pub async fn output(&self, run: &str, id: &str) -> Result<Option<(String, String, Vec<u8>)>> {
        let row = self
            .client()
            .await?
            .query_opt("SELECT body FROM public.events WHERE run_id = $1 AND kind = 'output' AND body->>'id' = $2", &[&run, &id])
            .await?;
        let Some(row) = row else { return Ok(None) };
        let b: Value = row.get(0);
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(b["data_b64"].as_str().unwrap_or_default())?;
        Ok(Some((b["name"].as_str().unwrap_or("file").into(), b["media_type"].as_str().unwrap_or("application/octet-stream").into(), bytes)))
    }

    /// One run's events in order: the developer view's full trace (V4).
    pub async fn run_events(&self, run: &str) -> Result<Vec<Event>> {
        let rows = self.client().await?.query("SELECT seq, at, actor, body FROM public.events WHERE run_id = $1 ORDER BY seq", &[&run]).await?;
        rows.into_iter()
            .map(|r| {
                let body: Value = r.get(3);
                let ev: Ev = serde_json::from_value(body)?;
                Ok(Event { seq: r.get(0), at: r.get(1), actor: r.get(2), ev })
            })
            .collect()
    }

    /// Every row as stored, for a backup.
    pub async fn dump(&self) -> Result<Vec<Value>> {
        let rows = self.client().await?.query("SELECT seq, at, kind, actor, run_id, body FROM public.events ORDER BY seq", &[]).await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let at: DateTime<Utc> = r.get(1);
                serde_json::json!({ "seq": r.get::<_, i64>(0), "at": at.to_rfc3339(), "kind": r.get::<_, String>(2), "actor": r.get::<_, String>(3), "run_id": r.get::<_, Option<String>>(4), "body": r.get::<_, Value>(5) })
            })
            .collect())
    }

    pub async fn load(&self) -> Result<Vec<Event>> {
        let rows = self.client().await?.query("SELECT seq, at, actor, body FROM public.events ORDER BY seq", &[]).await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let body: Value = r.get(3);
            let seq: i64 = r.get(0);
            self.saw(seq);
            let Some(ev) = parse_event(&body).with_context(|| format!("event {seq} does not parse"))? else {
                tracing::warn!("event {seq} is a kind this release does not know ({}); skipped", body["kind"]);
                continue;
            };
            out.push(Event { seq, at: r.get(1), actor: r.get(2), ev });
        }
        Ok(out)
    }
}

/// An event from the log: `None` when its kind is one this release does not
/// know (written by a later release it was rolled back from, so nothing it
/// acts on). Any other failure is an error, an unknown value inside a known
/// kind included: serde words both "unknown variant", and skipping the
/// second would drop part of a run's history.
pub fn parse_event(body: &Value) -> Result<Option<Ev>> {
    match serde_json::from_value::<Ev>(body.clone()) {
        Ok(ev) => Ok(Some(ev)),
        Err(e) => {
            let kind_unknown = serde_json::from_value::<Ev>(serde_json::json!({ "kind": body["kind"] })).err().is_some_and(|k| k.to_string().starts_with("unknown variant"));
            if kind_unknown {
                Ok(None)
            } else {
                Err(e.into())
            }
        }
    }
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn only_an_unknown_kind_is_skipped() {
        let known = serde_json::to_value(Ev::MemoryForgotten { id: "m1".into() }).unwrap();
        assert!(matches!(parse_event(&known), Ok(Some(_))));
        assert!(matches!(parse_event(&serde_json::json!({ "kind": "from_a_later_release", "x": 1 })), Ok(None)));
        // A known kind holding a value this release does not know (a block
        // type from a later release) stops the boot; serde words it the same.
        let nested = serde_json::json!({ "kind": "memory_pass", "run": "r", "model": "m", "blocks": [{ "type": "thinking_v9", "text": "x" }], "usage": {} });
        let err = serde_json::from_value::<Ev>(nested.clone()).unwrap_err().to_string();
        assert!(err.starts_with("unknown variant"), "{err}");
        assert!(parse_event(&nested).is_err());
    }
}
