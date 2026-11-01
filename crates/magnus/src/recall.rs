//! Recall: a person's content read from the content store when a request
//! needs it, through the per-person role, never taken from what the kernel
//! holds (owner, 2026-11-01: "recall only"; `docs/proposals/recall-only.md`,
//! step 2). Each read runs in a transaction as `{role}_person` with
//! `magnus.person` set to the person it is for, so row-level security
//! returns only that person's rows: a read asked for the wrong person
//! returns nothing, not someone else's content.

use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use tokio_postgres::Client;

pub struct Recall {
    url: String,
    reader: String,
    client: tokio::sync::Mutex<Option<Client>>,
}

impl Recall {
    pub fn new(url: &str) -> Recall {
        let role = url::Url::parse(url).map(|u| u.username().to_string()).unwrap_or_default();
        Recall { url: url.into(), reader: format!("{role}_person"), client: tokio::sync::Mutex::new(None) }
    }

    /// The content of these events that belongs to `person`, by event seq.
    /// Events whose content is not theirs, or was deleted, are not in it.
    pub async fn content(&self, person: &str, seqs: &[i64]) -> Result<HashMap<i64, Value>> {
        if seqs.is_empty() {
            return Ok(HashMap::new());
        }
        let mut g = self.client.lock().await;
        if g.as_ref().map_or(true, |c| c.is_closed()) {
            *g = Some(crate::log::connect(&self.url).await?);
        }
        let c = g.as_mut().context("recall connection")?;
        let tx = c.transaction().await?;
        tx.batch_execute(&format!("SET LOCAL ROLE {}", self.reader)).await.context("recall as the person (has `magnus migrate` run since this release was installed?)")?;
        tx.execute("SELECT set_config('magnus.person', $1, true)", &[&person]).await?;
        let rows = tx.query("SELECT seq, body FROM public.content WHERE seq = ANY($1)", &[&seqs]).await?;
        tx.commit().await?;
        Ok(rows.iter().map(|r| (r.get::<_, i64>(0), r.get::<_, Value>(1))).collect())
    }
}

/// A remembered fact as it is shown: its record and what it says.
pub struct Remembered {
    pub entry: crate::state::MemoryEntry,
    pub fact: String,
    pub topic: Option<String>,
}

impl crate::kernel::Kernel {
    /// A person's remembered facts that `keep` selects, newest last, each
    /// with what it says recalled for them. A fact the content store does
    /// not give them (deleted, or not theirs) is left out.
    pub async fn remembered(&self, person: &str, keep: impl Fn(&crate::state::MemoryEntry) -> bool) -> Result<Vec<Remembered>> {
        let entries: Vec<crate::state::MemoryEntry> = self.read(|s| s.memory.iter().filter(|m| m.person == person && keep(m)).cloned().collect());
        let seqs: Vec<i64> = entries.iter().map(|m| m.seq).collect();
        let content = self.recall.content(person, &seqs).await?;
        Ok(entries
            .into_iter()
            .filter_map(|entry| {
                let c = content.get(&entry.seq)?;
                let fact = c["fact"].as_str()?.to_string();
                let topic = c["topic"].as_str().map(String::from);
                Some(Remembered { entry, fact, topic })
            })
            .collect())
    }
}

impl crate::kernel::Kernel {
    /// A library file's bytes, recalled for its person from the event that
    /// holds them. None when its content is gone or not theirs.
    pub async fn file_bytes(&self, f: &crate::state::LibFile) -> Result<Option<Vec<u8>>> {
        use base64::Engine;
        let Some(b64) = self.file_b64(f).await? else { return Ok(None) };
        Ok(Some(base64::engine::general_purpose::STANDARD.decode(b64)?))
    }

    /// A library file's content as base64, recalled as `file_bytes` is.
    pub async fn file_b64(&self, f: &crate::state::LibFile) -> Result<Option<String>> {
        let mut c = self.recall.content(&f.person, &[f.seq]).await?;
        Ok(c.remove(&f.seq).and_then(|body| body["data_b64"].as_str().map(String::from)))
    }

    /// The files a run's task was given, with their content recalled for
    /// the task's person, as `{name, media_type, data_b64}`.
    pub async fn given_files(&self, task: &str) -> Result<Vec<Value>> {
        let given: Vec<crate::state::LibFile> = self.read(|s| s.tasks.get(task).into_iter().flat_map(|t| &t.files).filter_map(|id| s.files.get(id)).cloned().collect());
        let mut out = vec![];
        for f in given {
            if let Some(data) = self.file_b64(&f).await? {
                out.push(serde_json::json!({ "name": f.name, "media_type": f.media_type, "data_b64": data }));
            }
        }
        Ok(out)
    }

    /// The search text of a person's text files (in lower case, up to what
    /// a page shows), recalled for them, by file id. Only when a search
    /// has words to match: a listing needs none.
    pub async fn file_texts(&self, person: &str, terms: &[String]) -> Result<HashMap<String, String>> {
        if terms.is_empty() {
            return Ok(HashMap::new());
        }
        let files: Vec<(String, i64, String)> = self.read(|s| s.files.values().filter(|f| f.person == person).map(|f| (f.id.clone(), f.seq, f.media_type.clone())).collect());
        let seqs: Vec<i64> = files.iter().map(|(_, seq, _)| *seq).collect();
        let content = self.recall.content(person, &seqs).await?;
        Ok(files
            .into_iter()
            .filter_map(|(id, seq, media)| content.get(&seq).map(|c| (id, crate::state::search_text(&media, c["data_b64"].as_str().unwrap_or_default()))))
            .collect())
    }
}
