//! Chat. A person's message is a chat turn: a run without a contract that
//! ends with its answer. The model decides what the message calls for (an
//! answer, a task, a schedule for the person to confirm, or a question);
//! no code classifies it. Whatever a turn does goes through `execute()`.
//! A thread is derived from the log: its turns, the tasks they started with
//! their results, and the schedule cards with what the person decided.

use crate::boundary::Action;
use crate::events::{ChatOrigin, Ev};
use crate::kernel::{new_id, Kernel, NewRun};
use crate::state::{CardState, Run, Status, Task};
use crate::tools::Label;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::sync::Arc;

/// An id fixed by the action's idempotency key, so a call resent after a
/// restart finds what the first one made.
fn derived(prefix: &str, key: &str) -> String {
    use sha2::Digest;
    format!("{prefix}_{}", &hex::encode(sha2::Sha256::digest(key.as_bytes()))[..32])
}

/// Why a card decision was not taken.
pub enum CardRefusal {
    NotFound,
    Decided,
    Invalid(String),
}

impl Kernel {
    /// The person writes in a thread of theirs, or starts one, with the
    /// model they chose for this message (the appliance's when none).
    pub async fn chat_say(self: &Arc<Self>, person: &str, text: String, thread: Option<String>, model: Option<(String, String)>) -> Result<Option<(String, String)>> {
        let thread = match thread {
            Some(t) if self.thread(person, &t).is_empty() => return Ok(None),
            Some(t) => t,
            None => new_id("thread"),
        };
        let conversation = self.conversation(person, &thread);
        // The turn reads the thread so far, so it starts with the taint of
        // every turn and task in it (I2): an earlier answer or a task's
        // result may carry what was read there.
        let label = self.thread(person, &thread).iter().fold(Label::PRINCIPAL, |l, (_, r)| l.join(Label { untrusted: r.untrusted, private: r.private }));
        let o = NewRun {
            chat: Some(ChatOrigin { thread: thread.clone(), turn: true, from_run: None, conversation }),
            label: (label != Label::PRINCIPAL).then_some(label),
            model,
            ..Default::default()
        };
        let (task, _) = self.create_run(&format!("person:{person}"), person, text, vec![], o).await?;
        Ok(Some((thread, task)))
    }

    /// The person's threads, most recent first.
    pub fn threads(&self, person: &str) -> Vec<Value> {
        let mut by: std::collections::BTreeMap<String, (DateTime<Utc>, DateTime<Utc>, String)> = Default::default();
        self.read(|s| {
            for t in s.tasks.values().filter(|t| t.person == person) {
                let Some(c) = t.chat.as_ref() else { continue };
                let e = by.entry(c.thread.clone()).or_insert((t.created_at, t.created_at, t.text.clone()));
                if t.created_at < e.0 && c.turn {
                    (e.0, e.2) = (t.created_at, t.text.clone());
                }
                e.1 = e.1.max(t.created_at);
            }
        });
        let mut v: Vec<_> = by.into_iter().collect();
        v.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
        v.into_iter().map(|(id, (started, updated, first))| json!({ "thread": id, "title": first.chars().take(80).collect::<String>(), "started_at": started, "updated_at": updated })).collect()
    }

    /// A thread's turns and tasks, in the order they were created.
    fn thread(&self, person: &str, thread: &str) -> Vec<(Task, Run)> {
        let mut v: Vec<(Task, Run)> = self.read(|s| {
            s.tasks
                .values()
                .filter(|t| t.person == person && t.chat.as_ref().is_some_and(|c| c.thread == thread))
                .filter_map(|t| s.runs.get(&t.run_id).map(|r| (t.clone(), r.clone())))
                .collect()
        });
        v.sort_by_key(|(t, _)| t.created_at);
        v
    }

    /// The thread as the person sees it, from the log.
    pub fn thread_entries(&self, person: &str, thread: &str) -> Option<Vec<Value>> {
        let runs = self.thread(person, thread);
        if runs.is_empty() {
            return None;
        }
        let mut out: Vec<(DateTime<Utc>, i64, Value)> = vec![];
        for (t, r) in &runs {
            let turn = t.is_chat_turn();
            let status = crate::api::status_str(r.status);
            if turn {
                out.push((t.created_at, 0, json!({ "kind": "message", "text": t.text, "at": t.created_at, "task_id": t.task_id })));
                let mut cards: Vec<_> = r.results.values().filter_map(|res| res.data.as_ref().filter(|d| d["card"].is_string()).map(|d| (res.seq, d.clone()))).collect();
                cards.sort_by_key(|(seq, _)| *seq);
                for (seq, d) in cards {
                    let card = d["card"].as_str().unwrap_or_default().to_string();
                    let (state, schedule) = match self.read(|s| s.cards.get(&card).cloned()) {
                        Some(CardState::Confirmed(id)) => ("confirmed", Some(id)),
                        Some(CardState::Declined) => ("declined", None),
                        None => ("open", None),
                    };
                    let mut e = d.clone();
                    e["kind"] = json!("schedule_card");
                    e["state"] = json!(state);
                    e["schedule"] = json!(schedule);
                    e["acts_for"] = json!(t.person);
                    e["taint"] = json!({ "untrusted": r.untrusted, "private": r.private });
                    out.push((t.created_at, seq, e));
                }
                match r.status {
                    Status::Completed => out.push((
                        r.ended_at.unwrap_or(t.created_at),
                        i64::MAX,
                        json!({ "kind": "answer", "text": r.summary, "at": r.ended_at, "task_id": t.task_id, "model": { "adapter": r.config.adapter, "model": r.config.model } }),
                    )),
                    _ => out.push((
                        r.ended_at.unwrap_or(t.created_at),
                        i64::MAX,
                        json!({ "kind": "turn", "status": status, "waiting_on": r.reason, "waiting_item": r.waiting_item, "text": r.summary, "task_id": t.task_id, "at": r.ended_at }),
                    )),
                }
            } else {
                out.push((t.created_at, 0, json!({ "kind": "task", "task_id": t.task_id, "text": t.text, "status": status, "waiting_on": r.reason, "waiting_item": r.waiting_item, "at": t.created_at })));
                if let Some(end) = r.ended_at {
                    out.push((end, 0, json!({ "kind": "result", "task_id": t.task_id, "text": t.text, "status": status, "summary": r.summary, "at": end })));
                }
            }
        }
        out.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
        Some(out.into_iter().map(|(.., v)| v).collect())
    }

    /// The thread as the model reads it in the next turn: who said what.
    fn conversation(&self, person: &str, thread: &str) -> Vec<Value> {
        self.thread_entries(person, thread)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|e| {
                let text = |k: &str| e[k].as_str().unwrap_or_default().to_string();
                let (role, said) = match e["kind"].as_str()? {
                    "message" => ("person", text("text")),
                    "answer" => ("you", text("text")),
                    "task" => ("you", format!("Started a task: {}", text("text"))),
                    "result" => ("task result", format!("{} ({}): {}", text("text"), text("status"), text("summary"))),
                    "schedule_card" => ("you", format!("Proposed a schedule ({}): {}", text("state"), text("text"))),
                    _ => return None,
                };
                Some(json!({ "role": role, "text": said }))
            })
            .collect()
    }

    /// `create_task` from a chat turn: the task starts at once for the
    /// person, carries the conversation, the turn's taint and the turn's
    /// model, and posts its result to the thread.
    pub(crate) async fn chat_task(self: &Arc<Self>, a: &Action, key: &str) -> Result<(String, bool, Option<Value>)> {
        let text = a.input["text"].as_str().map(str::trim).unwrap_or_default().to_string();
        if text.is_empty() {
            return Ok(("create_task needs the task's text. A retry without it will not help.".into(), true, None));
        }
        let Some((task, run)) = self.read(|s| s.runs.get(&a.run).and_then(|r| s.tasks.get(&r.task).map(|t| (t.clone(), r.clone())))) else {
            anyhow::bail!("unknown run {}", a.run)
        };
        let Some(chat) = task.chat.clone().filter(|c| c.turn) else {
            return Ok(("create_task is for chat.".into(), true, None));
        };
        let (task_id, run_id) = (derived("task", key), derived("run", key));
        if !self.read(|s| s.tasks.contains_key(&task_id)) {
            let mut conversation = chat.conversation.clone();
            conversation.push(json!({ "role": "person", "text": task.text }));
            let o = NewRun {
                chat: Some(ChatOrigin { thread: chat.thread.clone(), turn: false, from_run: Some(a.run.clone()), conversation }),
                label: Some(Label { untrusted: run.untrusted, private: run.private }),
                ids: Some((task_id.clone(), run_id)),
                model: Some((run.config.adapter.clone(), run.config.model.clone())),
                ..Default::default()
            };
            self.create_run(&format!("chat:{}", a.run), &run.person, text.clone(), vec![], o).await?;
        }
        Ok((format!("Started task {task_id}: {text}. It is in the person's Tasks and will post its result to this thread."), false, Some(json!({ "task": task_id }))))
    }

    /// `create_schedule` from a chat turn: a card in the thread with the
    /// timing, scope and budget. Nothing is scheduled until the person
    /// confirms it.
    pub(crate) fn chat_card(&self, a: &Action, key: &str) -> (String, bool, Option<Value>) {
        if !self.read(|s| s.runs.get(&a.run).and_then(|r| s.tasks.get(&r.task)).is_some_and(|t| t.is_chat_turn())) {
            return ("create_schedule is for chat; in a task, use propose_schedule.".into(), true, None);
        }
        let text = a.input["text"].as_str().map(str::trim).unwrap_or_default();
        let scope = a.input["scope"].as_str().map(str::trim).unwrap_or_default();
        if text.is_empty() || scope.is_empty() {
            return ("create_schedule needs text and scope. A retry without them will not help.".into(), true, None);
        }
        let (every, daily) = match crate::api::rule_for(&a.input) {
            Ok(r) => r,
            Err(m) => return (format!("{m}. A retry with the same values will not help."), true, None),
        };
        let budget = match self.card_budget(&a.input["budget"]) {
            Ok(b) => b,
            Err(m) => return (format!("{m}. A retry with the same budget will not help."), true, None),
        };
        let card = derived("card", key);
        let data = json!({ "card": card, "text": text, "every_minutes": every, "daily_at": daily, "scope": scope, "budget": budget });
        (format!("A card for this schedule ({card}) is in the thread. Nothing is scheduled until the person confirms it."), false, Some(data))
    }

    /// A schedule's budget per run: the appliance's, tightened where the
    /// model asked, never loosened.
    fn card_budget(&self, b: &Value) -> std::result::Result<crate::config::Budget, String> {
        let mut out = self.cfg.budget.clone();
        if b.is_null() {
            return Ok(out);
        }
        if let Some(v) = b["max_usd"].as_f64() {
            if v <= 0.0 || v > out.max_usd {
                return Err(format!("max_usd must be above 0 and at most the appliance's {}", out.max_usd));
            }
            out.max_usd = v;
        }
        for (k, slot) in [("max_model_calls", &mut out.max_model_calls), ("max_actions", &mut out.max_actions)] {
            if let Some(v) = b[k].as_u64() {
                if v == 0 || v > *slot as u64 {
                    return Err(format!("{k} must be above 0 and at most the appliance's {slot}"));
                }
                *slot = v as u32;
            }
        }
        Ok(out)
    }

    /// The person who started the thread confirms or declines a schedule
    /// card. A confirmed schedule acts for them (K54), within the card's
    /// budget, and its runs start with the taint the turn had.
    pub async fn decide_card(&self, person: &str, card: &str, decision: &str) -> Result<std::result::Result<Value, CardRefusal>> {
        let _decide = self.decide.lock().await;
        let found = self.read(|s| {
            s.runs.values().find_map(|r| {
                let t = s.tasks.get(&r.task).filter(|t| t.is_chat_turn() && t.person == person)?;
                let d = r.results.values().find_map(|res| res.data.clone().filter(|d| d["card"] == card))?;
                Some((t.clone(), r.clone(), d))
            })
        });
        let Some((task, run, d)) = found else { return Ok(Err(CardRefusal::NotFound)) };
        if self.read(|s| s.cards.contains_key(card)) {
            return Ok(Err(CardRefusal::Decided));
        }
        let actor = format!("person:{person}");
        match decision {
            "confirm" => {
                let id = new_id("sched");
                let budget = serde_json::from_value(d["budget"].clone()).ok();
                // The person agreed to the card's scope, so each run is told it.
                let text = match (d["text"].as_str().unwrap_or_default(), d["scope"].as_str().filter(|s| !s.is_empty())) {
                    (t, Some(scope)) => format!("{t}\n\nThe person confirmed this schedule for this scope; keep to it: {scope}"),
                    (t, None) => t.to_string(),
                };
                self.emit(
                    &actor,
                    Ev::ScheduleSet {
                        id: id.clone(),
                        person: task.person.clone(),
                        text,
                        every_minutes: d["every_minutes"].as_u64().map(|m| m as u32),
                        daily_at: d["daily_at"].as_str().map(String::from),
                        budget,
                        card: Some(card.into()),
                        label: Some(Label { untrusted: run.untrusted, private: run.private }),
                    },
                )
                .await?;
                Ok(Ok(json!({ "schedule": id })))
            }
            "decline" => {
                self.emit(&actor, Ev::CardDeclined { card: card.into() }).await?;
                Ok(Ok(json!({})))
            }
            other => Ok(Err(CardRefusal::Invalid(format!("decision is confirm or decline, not {other}")))),
        }
    }
}
