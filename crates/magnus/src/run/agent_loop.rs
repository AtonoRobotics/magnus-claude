//! The loop contract (K2). Call the model, send every action through
//! `execute()`, write every step to the log. The loop holds no state: each
//! step starts from the log, so a new loop resumes any run.
//!
//! Code here only limits: it stops at the model-call budget, retries an
//! unavailable provider a bounded number of times, and suspends with a notice.
//! What to do next is always the model's decision.

use crate::boundary::{Action, Issuer, Verdict};
use crate::events::{ContextRef, Ev};
use crate::kernel::Kernel;
use crate::state::Status;
use anyhow::Result;
use magnus_adapter_proto::ErrorClass;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

pub enum Flow {
    Continue,
    Stop,
}

impl Kernel {
    pub async fn drive(self: Arc<Self>, run: &str) -> Result<()> {
        loop {
            match self.step(run).await? {
                Flow::Continue => continue,
                Flow::Stop => return Ok(()),
            }
        }
    }

    async fn step(self: &Arc<Self>, run_id: &str) -> Result<Flow> {
        let Some(run) = self.read(|s| s.runs.get(run_id).cloned()) else { return Ok(Flow::Stop) };
        if run.status != Status::Running {
            return Ok(Flow::Stop);
        }
        if self.read(|s| s.kill_switch) {
            self.suspend(run_id, "kill_switch", None).await?;
            return Ok(Flow::Stop);
        }

        // Finish the tool calls of the latest reply, in order.
        if let Some((index, call_id, name, input)) = run.pending_calls().into_iter().next() {
            let step = run.replies.last().map(|r| r.step).unwrap_or(0);
            let step_id = format!("s{step}.{index}");
            if crate::tools::RECORDS.contains(&name.as_str()) {
                return self.record_call(&run, &call_id, &step_id, &name, &input).await;
            }
            let v = self
                .execute(Action { run: run_id.into(), call_id, step_id, tool: name, input, issuer: Issuer::Model })
                .await?;
            return Ok(match v {
                Verdict::Done { .. } => Flow::Continue,
                Verdict::Suspended => Flow::Stop,
            });
        }

        // A removable module: a run going in circles is reset, escalated, or stopped.
        if let Some(flow) = self.when_lost(&run).await? {
            return Ok(flow);
        }

        // Ask the model for the next step.
        let budget = &run.config.budget;
        if run.model_calls >= budget.max_model_calls {
            self.suspend_with_notice(
                run_id,
                "budget",
                format!("Stopped after {} model calls, the task's limit.", run.model_calls),
                json!({ "model_calls": run.model_calls, "limit": budget.max_model_calls, "usd": run.usd }),
            )
            .await?;
            return Ok(Flow::Stop);
        }
        let (system, messages) = self.context(&run);
        let chat = self.is_chat_turn(&run);
        let tools = self.tools_for(&run).await;
        let tools_hash = {
            use sha2::Digest;
            hex::encode(&sha2::Sha256::digest(serde_json::to_vec(&tools)?)[..16])
        };
        let step = run.replies.len() as u32;
        let (adapter, model) = run.model();
        let mut attempt = 0u32;
        loop {
            let calls = self.read(|s| s.runs.get(run_id).map(|r| r.model_calls)).unwrap_or(0);
            if calls >= budget.max_model_calls {
                self.suspend_with_notice(
                    run_id,
                    "budget",
                    format!("Stopped after {calls} model calls, the task's limit."),
                    json!({ "model_calls": calls, "limit": budget.max_model_calls }),
                )
                .await?;
                return Ok(Flow::Stop);
            }
            self.emit(
                "kernel",
                Ev::ModelCall {
                    run: run_id.into(),
                    step,
                    attempt,
                    context: ContextRef { through_seq: run.last_seq, instructions: run.config.instructions.clone(), tools_hash: tools_hash.clone() },
                },
            )
            .await?;
            let live = self.live_text.clone();
            let rid = run_id.to_string();
            let out = self
                .adapters
                .complete(adapter, model, &system, messages.clone(), tools.clone(), run.config.max_tokens, |t| {
                    let _ = live.send((rid.clone(), t.to_string()));
                })
                .await;
            match out {
                Ok(t) => {
                    let has_call = t.blocks.iter().any(|b| matches!(b, magnus_adapter_proto::Block::ToolCall { .. }));
                    let answer: String = t
                        .blocks
                        .iter()
                        .filter_map(|b| match b {
                            magnus_adapter_proto::Block::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                        .trim()
                        .to_string();
                    self.emit(
                        "model",
                        Ev::ModelReply {
                            run: run_id.into(),
                            step,
                            blocks: t.blocks,
                            state: t.state,
                            usage: t.usage,
                            served_model: t.served_model,
                            stop: t.stop,
                        },
                    )
                    .await?;
                    // A chat turn ends with its answer: no contract, no Done
                    // check. What it did on the way went through execute().
                    if !has_call && chat {
                        self.emit("kernel", Ev::Completed { run: run_id.into(), summary: answer, view: None, spoken: None }).await?;
                        self.on_stop(run_id, true).await;
                        return Ok(Flow::Stop);
                    }
                    if !has_call {
                        self.emit(
                            "kernel",
                            Ev::Notice {
                                run: run_id.into(),
                                step: step + 1,
                                text: "Your reply called no tool, so nothing happened. Every reply must call at least one tool: continue the work, or call task_complete or task_fail.".into(),
                            },
                        )
                        .await?;
                    }
                    return Ok(Flow::Continue);
                }
                Err(e) => {
                    self.emit(
                        "kernel",
                        Ev::ModelError { run: run_id.into(), step, attempt, class: e.class, message: e.message.clone() },
                    )
                    .await?;
                    match e.class {
                        ErrorClass::Unavailable if attempt < run.config.max_retries => {
                            attempt += 1;
                            tokio::time::sleep(Duration::from_millis(500 * 2u64.pow(attempt.min(6)))).await;
                        }
                        ErrorClass::Malformed => {
                            self.emit(
                                "kernel",
                                Ev::Notice {
                                    run: run_id.into(),
                                    step,
                                    text: format!("Your previous reply could not be used: {}. Reply again.", e.message),
                                },
                            )
                            .await?;
                            return Ok(Flow::Continue);
                        }
                        class => {
                            let (reason, what) = match class {
                                ErrorClass::Unavailable => ("provider", "The model provider is unavailable"),
                                ErrorClass::UsageLimit => ("usage_limit", "The model provider's usage limit was reached"),
                                ErrorClass::AuthExpired => ("auth", "The model provider needs you to sign in again"),
                                _ => ("invalid_request", "The model provider rejected the request"),
                            };
                            self.suspend_with_notice(
                                run_id,
                                reason,
                                format!("{what}. The task is paused; resume it when this is fixed."),
                                json!({ "class": class, "message": e.message, "attempts": attempt + 1, "retry_after_s": e.retry_after_s }),
                            )
                            .await?;
                            return Ok(Flow::Stop);
                        }
                    }
                }
            }
        }
    }
}
