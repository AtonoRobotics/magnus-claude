//! The trash. Deleting an item moves it here, out of every view; it can be
//! restored until it is removed, `[trash] keep_days` later or when the
//! person empties the trash. Removing deletes the item's own content from
//! the content store; the log keeps the record (C3). Only the item goes:
//! what it made (files a task delivered, memories it wrote, tasks a
//! conversation started) stays, as the owner decided.

use crate::api::{err, internal, who};
use crate::events::Ev;
use crate::kernel::Kernel;
use crate::state::NotTrashable;
use anyhow::Result;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::sync::Arc;

type K = State<Arc<Kernel>>;

pub fn router() -> Router<Arc<Kernel>> {
    Router::new()
        .route("/v1/trash", get(list).post(put))
        .route("/v1/trash/empty", post(empty))
        .route("/v1/trash/{item}/restore", post(restore))
        .route("/v1/trash/{item}", delete(remove_now))
}

impl Kernel {
    /// Move a person's item to the trash.
    pub async fn trash(&self, person: &str, owner: bool, item: &str) -> Result<Result<(), NotTrashable>> {
        if let Err(why) = self.read(|s| s.may_trash(person, owner, item)) {
            return Ok(Err(why));
        }
        self.emit(&format!("person:{person}"), Ev::Trashed { item: item.into(), person: person.into() }).await?;
        if item.starts_with("kb_") {
            self.index_id(item).await?;
        }
        Ok(Ok(()))
    }

    /// Put a trashed item back. False when it is not in the person's trash.
    pub async fn restore(&self, person: &str, item: &str) -> Result<bool> {
        if !self.read(|s| s.trash.get(item).is_some_and(|t| t.person == person)) {
            return Ok(false);
        }
        self.emit(&format!("person:{person}"), Ev::Restored { item: item.into() }).await?;
        if item.starts_with("kb_") {
            self.index_id(item).await?;
        }
        Ok(true)
    }

    /// Remove a trashed item: its content leaves the store for good. The
    /// record of which events lost their content is written first, so a
    /// removal cut short finishes at the next start.
    pub async fn remove(&self, actor: &str, item: &str) -> Result<()> {
        let Some(holding) = self.read(|s| s.holding(item)) else { return Ok(()) };
        let seqs = self.log.content_seqs(&holding).await?;
        self.emit(actor, Ev::Removed { item: item.into(), seqs: seqs.clone() }).await?;
        self.log.delete_content(&seqs).await?;
        Ok(())
    }

    /// Remove what has been in the trash for `keep_days`.
    pub async fn purge_trash(&self) -> Result<usize> {
        let cutoff = chrono::Utc::now() - chrono::Duration::days(self.cfg.trash.keep_days as i64);
        let due: Vec<String> = self.read(|s| s.trash.iter().filter(|(_, t)| t.at <= cutoff).map(|(id, _)| id.clone()).collect());
        for item in &due {
            self.remove("kernel:trash", item).await?;
        }
        Ok(due.len())
    }
}

/// Remove what is due, hourly from the appliance's start: an item is gone
/// within the hour after its `keep_days`.
pub fn schedule(k: Arc<Kernel>) {
    tokio::spawn(async move {
        let mut t = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            t.tick().await;
            if let Err(e) = k.purge_trash().await {
                tracing::error!("emptying the trash: {e:#}");
            }
        }
    });
}

macro_rules! person {
    ($k:expr, $h:expr) => {
        match who(&$k, &$h) {
            Ok(p) => p,
            Err(r) => return r,
        }
    };
}

/// What is in the person's trash, newest first, and when each goes.
async fn list(State(k): K, h: HeaderMap) -> Response {
    let p = person!(k, h);
    let keep = chrono::Duration::days(k.cfg.trash.keep_days as i64);
    let mut items: Vec<Value> = k.read(|s| {
        s.trash
            .iter()
            .filter(|(_, t)| t.person == p.id)
            .map(|(id, t)| json!({ "item": id, "kind": t.kind(), "title": t.title.chars().take(120).collect::<String>(), "thread": t.thread(), "trashed_at": t.at, "removed_at": t.at + keep }))
            .collect()
    });
    items.sort_by(|a, b| b["trashed_at"].as_str().cmp(&a["trashed_at"].as_str()));
    Json(json!({ "items": items, "keep_days": k.cfg.trash.keep_days })).into_response()
}

/// Move an item to the trash: `{"item": id}`.
async fn put(State(k): K, h: HeaderMap, Json(b): Json<Value>) -> Response {
    let p = person!(k, h);
    let Some(item) = b["item"].as_str() else { return err(StatusCode::BAD_REQUEST, "name the item") };
    trashed(k.trash(&p.id, p.is_owner(), item).await)
}

pub(crate) fn trashed(r: Result<Result<(), NotTrashable>>) -> Response {
    match r {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(NotTrashable::NotFound)) => err(StatusCode::NOT_FOUND, "no such item of yours"),
        Ok(Err(NotTrashable::Running)) => err(StatusCode::CONFLICT, "the task is still running; cancel it first"),
        Err(e) => internal(e),
    }
}

async fn restore(State(k): K, h: HeaderMap, Path(item): Path<String>) -> Response {
    let p = person!(k, h);
    match k.restore(&p.id, &item).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => err(StatusCode::NOT_FOUND, "not in your trash"),
        Err(e) => internal(e),
    }
}

/// Remove one item from the trash now.
async fn remove_now(State(k): K, h: HeaderMap, Path(item): Path<String>) -> Response {
    let p = person!(k, h);
    if !k.read(|s| s.trash.get(&item).is_some_and(|t| t.person == p.id)) {
        return err(StatusCode::NOT_FOUND, "not in your trash");
    }
    match k.remove(&format!("person:{}", p.id), &item).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => internal(e),
    }
}

/// Remove everything in the person's trash now, when space is needed sooner.
async fn empty(State(k): K, h: HeaderMap) -> Response {
    let p = person!(k, h);
    let items: Vec<String> = k.read(|s| s.trash.iter().filter(|(_, t)| t.person == p.id).map(|(id, _)| id.clone()).collect());
    for item in &items {
        if let Err(e) = k.remove(&format!("person:{}", p.id), item).await {
            return internal(e);
        }
    }
    Json(json!({ "removed": items.len() })).into_response()
}
