//! The inbox: every human decision (U1). Items reach people through this API
//! only, outside the agent's context (B5); the agent sees only that it waits.

use crate::config::Person;
use crate::events::Ev;
use crate::kernel::{new_id, Kernel};
use crate::state::{Item, State, Status};
use crate::tools::Label;
use anyhow::Result;
use serde_json::{json, Value};
use std::sync::Arc;

pub enum Answer {
    Ok,
    Forbidden(&'static str),
    NotFound,
    Conflict(&'static str),
    Invalid(String),
}

/// May `p` set policy for `layer`? (E7: separate from approving.)
pub fn may_set_policy(p: &Person, layer: &str) -> bool {
    if p.is_owner() {
        return true;
    }
    if let Some(u) = layer.strip_prefix("user/") {
        return u == p.id && p.has_role("policy:user");
    }
    if let Some(g) = layer.strip_prefix("group/") {
        return p.has_role(&format!("policy:group:{g}"));
    }
    false
}

pub fn may_respond(p: &Person, i: &Item) -> bool {
    match i.kind.as_str() {
        "approval" => p.is_owner() || p.has_role(&format!("approver:{}", i.details["risk"].as_str().unwrap_or_default())),
        "policy_proposal" => may_set_policy(p, i.details["layer"].as_str().unwrap_or_default()),
        "tool_change" | "tool_review" | "deletion_request" => p.is_owner(),
        // Only someone with the role who may read the entry makes it a company fact.
        "endorsement" => crate::knowledge::may_endorse(p) && crate::knowledge::can_read(p, &serde_json::from_value::<Vec<Vec<String>>>(i.details["readers"].clone()).unwrap_or_else(|_| vec![vec![]])),
        // A task's questions, reviews and notices are its person's alone.
        _ => i.person == p.id,
    }
}

pub fn visible(p: &Person, i: &Item) -> bool {
    i.person == p.id || may_respond(p, i)
}

/// What an approval's risk means for the person deciding (U6): the
/// consequence of a destructive action is stated on the card, never left to
/// the details.
fn consequence(i: &Item) -> Option<&'static str> {
    if i.kind != "approval" {
        return None;
    }
    if i.details["reason"] == "unknown_outcome" {
        return Some("It may already have happened once. Approving sends it again.");
    }
    match i.details["risk"].as_str().unwrap_or_default() {
        "irreversible" => Some("This cannot be undone once it runs."),
        "financial" => Some("This spends or moves money and cannot be undone."),
        "reversible_write" => Some("This changes data outside the appliance. It can be undone afterwards."),
        _ => None,
    }
}

/// An item blocks when a run is paused waiting on it; the app interrupts
/// only for those and batches the rest (U3).
pub fn blocking(s: &State, i: &Item) -> bool {
    i.resolution.is_none()
        && i.run.as_ref().and_then(|r| s.runs.get(r)).is_some_and(|r| r.status == Status::Suspended && r.waiting_item.as_deref() == Some(&i.id))
}

pub fn render(s: &State, p: &Person, i: &Item) -> Value {
    let tier = match i.kind.as_str() {
        "approval" => i.details["risk"].as_str().unwrap_or("unknown").to_string(),
        k => k.to_string(),
    };
    json!({
        "id": i.id,
        "kind": i.kind,
        "task_id": i.task,
        "run_id": i.run,
        "summary": i.summary,
        "details": i.details,
        "created_at": i.created_at,
        "status": if i.resolution.is_some() { "resolved" } else { "open" },
        "resolution": i.resolution.as_ref().map(|(d, s, t, by)| json!({ "decision": d, "scope": s, "text": t, "by": by })),
        "can_respond": i.resolution.is_none() && may_respond(p, i),
        "blocking": blocking(s, i),
        "tier": tier,
        "consequence": consequence(i),
    })
}

impl Kernel {
    pub async fn respond(self: &Arc<Self>, p: &Person, item_id: &str, body: &Value) -> Result<Answer> {
        let Some(item) = self.read(|s| s.items.get(item_id).cloned()) else { return Ok(Answer::NotFound) };
        if !visible(p, &item) {
            return Ok(Answer::NotFound);
        }
        if !may_respond(p, &item) {
            return Ok(Answer::Forbidden("you do not hold the role this decision needs"));
        }
        if item.resolution.is_some() {
            return Ok(Answer::Conflict("already decided"));
        }
        let decision = body["decision"].as_str().unwrap_or_default().to_string();
        let text = body["text"].as_str().map(String::from);
        let actor = format!("person:{}", p.id);
        let run = item.run.clone().unwrap_or_default();
        let call_id = item.details["call_id"].as_str().unwrap_or_default().to_string();
        let resolve = |scope: Option<String>| Ev::ItemResolved { id: item.id.clone(), decision: decision.clone(), scope, text: text.clone() };

        match (item.kind.as_str(), decision.as_str()) {
            ("approval", "approve") => {
                let scope = body["scope"].as_str().unwrap_or("once").to_string();
                if !["once", "task", "window", "standing"].contains(&scope.as_str()) {
                    return Ok(Answer::Invalid(format!("unknown scope {scope}")));
                }
                let rule = item.details["rule"].as_str().unwrap_or_default();
                let unknown = item.details["reason"] == "unknown_outcome";
                // Web lookups after reading private, untrusted data may be
                // allowed for the whole task (B5); other taint asks only once.
                let lookups = rule == crate::boundary::WEB_LOOKUPS;
                if lookups && !["once", "task"].contains(&scope.as_str()) {
                    return Ok(Answer::Invalid("web lookups are allowed once or for this task".into()));
                }
                if scope != "once" && !lookups && (rule.starts_with("kernel:") || unknown) {
                    return Ok(Answer::Invalid("this action can only be approved once: it is asked because of what the task has read, or because its outcome is unknown".into()));
                }
                // A standing approval is autonomy; only the owner grants it (invariant 6).
                if scope == "standing" && !p.is_owner() {
                    return Ok(Answer::Forbidden("only the owner can approve standing; propose a policy instead"));
                }
                let until = match scope.as_str() {
                    "window" => Some(chrono::Utc::now() + chrono::Duration::minutes(body["minutes"].as_i64().unwrap_or(60).clamp(1, 24 * 60))),
                    _ => None,
                };
                let reason = if unknown { Some("unknown_outcome".to_string()) } else { item.details["rule"].as_str().map(String::from) };
                self.emit(&actor, resolve(Some(scope.clone()))).await?;
                self.emit(
                    &actor,
                    Ev::Approval {
                        id: new_id("appr"),
                        item: item.id.clone(),
                        person: item.person.clone(),
                        task: item.task.clone().unwrap_or_default(),
                        tool: item.details["tool"].as_str().unwrap_or_default().into(),
                        input_hash: item.details["input_hash"].as_str().map(String::from),
                        scope,
                        until,
                        reason,
                    },
                )
                .await?;
                self.resume_after(&run).await?;
            }
            ("approval", "reject") => {
                self.emit(&actor, resolve(None)).await?;
                let unknown = item.details["reason"] == "unknown_outcome";
                let msg = if unknown {
                    format!("{} was treated as not done, by the person's decision. {}", item.details["tool"].as_str().unwrap_or_default(), text.clone().unwrap_or_default())
                } else {
                    format!(
                        "The person declined this action{}. A retry of the same action will not help; decide what to do instead.",
                        text.as_ref().map(|t| format!(": {t}")).unwrap_or_default()
                    )
                };
                self.reply(&run, &call_id, msg, true, Label::PRINCIPAL).await?;
                self.resume_after(&run).await?;
            }
            ("review", "accept") => {
                self.emit(&actor, resolve(None)).await?;
                for c in item.details["criteria"].as_array().cloned().unwrap_or_default() {
                    self.emit(
                        &actor,
                        Ev::Verification {
                            run: run.clone(),
                            criterion: c["id"].as_str().unwrap_or_default().into(),
                            passed: true,
                            evidence: json!({ "reviewed_by": p.id, "note": text }),
                            by: actor.clone(),
                        },
                    )
                    .await?;
                }
                self.reply(&run, &call_id, "The person reviewed the work and accepted it.", false, Label::PRINCIPAL).await?;
                self.emit(
                    &actor,
                    Ev::Completed {
                        run: run.clone(),
                        summary: item.details["summary"].as_str().unwrap_or_default().into(),
                        view: item.details.get("view").filter(|v| !v.is_null()).cloned(),
                        spoken: item.details["spoken"].as_str().map(String::from),
                    },
                )
                .await?;
                self.on_stop(&run, true).await;
            }
            ("review", "reject") => {
                self.emit(&actor, resolve(None)).await?;
                let msg = format!(
                    "The person reviewed the work and did not accept it{}. The task stays open; a retry without changes will not help.",
                    text.as_ref().map(|t| format!(": {t}")).unwrap_or_default()
                );
                self.reply(&run, &call_id, msg, true, Label::PRINCIPAL).await?;
                self.resume_after(&run).await?;
            }
            ("contract_change", "approve") => {
                let version = self.read(|s| s.runs.get(&run).map(|r| r.contract_version)).unwrap_or(0) + 1;
                self.emit(&actor, resolve(None)).await?;
                self.emit(&actor, Ev::Contract { run: run.clone(), version, contract: item.details["new"].clone(), approved_by: Some(actor.clone()) }).await?;
                self.reply(&run, &call_id, format!("The person approved the contract change. Version {version} is in force, and frozen."), false, Label::PRINCIPAL).await?;
                self.resume_after(&run).await?;
            }
            ("contract_change", "reject") => {
                self.emit(&actor, resolve(None)).await?;
                let msg = format!(
                    "The person kept the contract (version {}){}. Its criteria still decide when the task is done: finish against them, or call task_fail.",
                    item.details["version"],
                    text.as_ref().map(|t| format!(": {t}")).unwrap_or_default()
                );
                self.reply(&run, &call_id, msg, true, Label::PRINCIPAL).await?;
                self.resume_after(&run).await?;
            }
            ("question", "answer") => {
                self.emit(&actor, resolve(None)).await?;
                self.reply(&run, &call_id, format!("The person answered: {}", text.clone().unwrap_or_default()), false, Label::PRINCIPAL).await?;
                self.resume_after(&run).await?;
            }
            ("policy_proposal", "accept") => {
                let layer = item.details["layer"].as_str().unwrap_or_default().to_string();
                let add = item.details["cedar"].as_str().unwrap_or_default();
                let current = self.read(|s| s.policies.get(&layer).cloned()).unwrap_or_else(|| {
                    if layer == "org" { crate::policy::DEFAULT_ORG.to_string() } else { String::new() }
                });
                let next = format!("{current}\n{add}\n");
                if let Err(e) = crate::policy::validate(&next) {
                    return Ok(Answer::Invalid(format!("the combined policy does not parse: {e}")));
                }
                self.emit(&actor, resolve(None)).await?;
                self.emit(&actor, Ev::PolicySet { layer, cedar: next }).await?;
            }
            ("schedule_proposal", "accept") => {
                self.emit(&actor, resolve(None)).await?;
                self.emit(
                    &actor,
                    Ev::ScheduleSet {
                        id: new_id("sched"),
                        person: item.person.clone(),
                        text: item.details["text"].as_str().unwrap_or_default().into(),
                        every_minutes: item.details["every_minutes"].as_u64().map(|m| m as u32),
                        daily_at: item.details["daily_at"].as_str().map(String::from),
                        budget: None,
                        card: None,
                        label: None,
                        title: None,
                    },
                )
                .await?;
            }
            // An entry becomes a company fact; its source stays attached, and
            // who endorsed it, from which card, is in the log.
            ("endorsement", "endorse") => {
                let entry = item.details["entry"].as_str().unwrap_or_default().to_string();
                if self.read(|s| s.knowledge.get(&entry).map(|e| e.integrity.clone())).is_none() {
                    return Ok(Answer::Invalid("the entry no longer exists".into()));
                }
                self.emit(&actor, resolve(None)).await?;
                self.endorse(&actor, &p.id, &item.id, &entry, text.clone()).await?;
            }
            ("endorsement", "decline") => {
                self.emit(&actor, resolve(None)).await?;
            }
            // The owner says what became of a request to delete someone's
            // data; the note tells the person.
            ("deletion_request", "done") | ("deletion_request", "decline") => {
                self.emit(&actor, resolve(None)).await?;
            }
            ("schedule_proposal", "reject") | ("policy_proposal", "reject") | ("notice", "dismiss") | ("message", "dismiss") | ("shared", "dismiss") | ("tool_change", "reject") | ("tool_review", "reject") => {
                self.emit(&actor, resolve(None)).await?;
            }
            // Only the kernel's own notices (which name a reason) can continue a
            // run; a message the agent wrote cannot.
            ("notice", "extend") | ("notice", "resume") if item.details["reason"].is_string() && (decision != "extend" || item.details["reason"] == "budget") => {
                self.emit(&actor, resolve(None)).await?;
                if decision == "extend" {
                    if let Some(mut b) = self.read(|s| s.runs.get(&run).map(|r| r.config.budget.clone())) {
                        b.max_model_calls *= 2;
                        b.max_actions *= 2;
                        b.max_usd *= 2.0;
                        self.emit(&actor, Ev::BudgetExtended { run: run.clone(), budget: b }).await?;
                    }
                }
                self.resume_after(&run).await?;
            }
            ("tool_change", "approve") => {
                self.emit(&actor, resolve(None)).await?;
                self.emit(
                    &actor,
                    Ev::ToolPinned {
                        tool: item.details["tool"].as_str().unwrap_or_default().into(),
                        hash: item.details["hash"].as_str().unwrap_or_default().into(),
                        description: item.details["description"].as_str().unwrap_or_default().into(),
                        input_schema: item.details["input_schema"].clone(),
                        approved: true,
                    },
                )
                .await?;
            }
            ("tool_review", "approve") => {
                self.emit(&actor, resolve(None)).await?;
                for t in item.details["tools"].as_array().cloned().unwrap_or_default() {
                    let name = t["tool"].as_str().unwrap_or_default().to_string();
                    // Approve only definitions that are still the pinned ones.
                    let current = self.read(|s| s.pins.get(&name).map(|p| p.hash.clone()));
                    if current.as_deref() == t["hash"].as_str() {
                        self.emit(
                            &actor,
                            Ev::ToolPinned {
                                tool: name,
                                hash: t["hash"].as_str().unwrap_or_default().into(),
                                description: t["description"].as_str().unwrap_or_default().into(),
                                input_schema: t["input_schema"].clone(),
                                approved: true,
                            },
                        )
                        .await?;
                    }
                }
            }
            (kind, d) => return Ok(Answer::Invalid(format!("{d:?} is not a decision for a {kind} item"))),
        }
        Ok(Answer::Ok)
    }

    async fn reply(&self, run: &str, call_id: &str, text: impl Into<String>, is_error: bool, label: Label) -> Result<()> {
        if run.is_empty() || call_id.is_empty() {
            return Ok(());
        }
        self.emit(
            "kernel",
            Ev::ToolResult {
                run: run.into(),
                call_id: call_id.into(),
                key: None,
                status: if is_error { "failed".into() } else { "done".into() },
                text: text.into(),
                is_error,
                label,
                data: None,
                sources: vec![],
                check: false,
            },
        )
        .await?;
        Ok(())
    }

    /// Resume a run that was waiting on a person, unless the kill switch holds.
    async fn resume_after(self: &Arc<Self>, run: &str) -> Result<()> {
        if run.is_empty() {
            return Ok(());
        }
        let (suspended, reason) = self.read(|s| s.runs.get(run).map(|r| (r.status == Status::Suspended, r.reason.clone()))).unwrap_or((false, None));
        if suspended && reason.as_deref() != Some("kill_switch") && !self.read(|s| s.kill_switch) {
            self.emit("kernel", Ev::Resumed { run: run.into() }).await?;
            self.wake(run);
        }
        Ok(())
    }
}
