//! The event log: one append-only Postgres table, the only source of truth.
//!
//! `migrate` runs with an administrator URL and creates the table and the
//! kernel's role. The role may INSERT and SELECT and nothing else, so the
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
    Ok(())
}

pub struct Log {
    client: Client,
}

impl Log {
    pub async fn open(url: &str) -> Result<Log> {
        let client = connect(url).await?;
        // One kernel per log: a second one would break the single-writer rule.
        let got: bool = client.query_one("SELECT pg_try_advisory_lock(7239001)", &[]).await?.get(0);
        anyhow::ensure!(got, "another kernel holds this event log");
        Ok(Log { client })
    }

    pub async fn append(&self, actor: &str, ev: &Ev) -> Result<Event> {
        let body = serde_json::to_value(ev)?;
        let kind = body["kind"].as_str().unwrap_or_default().to_string();
        let run = ev.run().map(String::from);
        let row = self
            .client
            .query_one(
                "INSERT INTO public.events (kind, actor, run_id, body) VALUES ($1, $2, $3, $4) RETURNING seq, at",
                &[&kind, &actor, &run, &body],
            )
            .await
            .context("append to the event log")?;
        Ok(Event { seq: row.get(0), at: row.get(1), actor: actor.into(), ev: ev.clone() })
    }

    /// One delivered file's content, read straight from the log.
    pub async fn output(&self, run: &str, id: &str) -> Result<Option<(String, String, Vec<u8>)>> {
        let row = self
            .client
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
        let rows = self.client.query("SELECT seq, at, actor, body FROM public.events WHERE run_id = $1 ORDER BY seq", &[&run]).await?;
        rows.into_iter()
            .map(|r| {
                let body: Value = r.get(3);
                let ev: Ev = serde_json::from_value(body)?;
                Ok(Event { seq: r.get(0), at: r.get(1), actor: r.get(2), ev })
            })
            .collect()
    }

    pub async fn load(&self) -> Result<Vec<Event>> {
        let rows = self.client.query("SELECT seq, at, actor, body FROM public.events ORDER BY seq", &[]).await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let body: Value = r.get(3);
            let ev: Ev = serde_json::from_value(body).with_context(|| format!("event {} does not parse", r.get::<_, i64>(0)))?;
            out.push(Event { seq: r.get(0), at: r.get(1), actor: r.get(2), ev });
        }
        Ok(out)
    }
}
