//! The proactive loop's settings and hours (behavior step 7, #237): each
//! person's working and quiet hours, review cadence and interruption
//! budget, resolved from their preferences over the deployment's config.
//! Anything unparsable falls back to the default; settings never break a
//! review.

use chrono::{DateTime, TimeZone, Timelike, Utc};
use crate::tools::Label;
use std::sync::Arc;

/// The ended task that woke a review: its ID, its outcome line, and the
/// taint it ended with, which the review starts with (#383, #390).
pub(crate) struct Ended {
    pub task: String,
    pub outcome: String,
    pub label: Label,
}

/// Minutes since midnight for a "HH:MM" time, in the person's timezone.
fn at(hhmm: &str) -> Option<u32> {
    let (h, m) = hhmm.split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then(|| h * 60 + m)
}

fn window(v: &serde_json::Value, start: &str, end: &str) -> (u32, u32) {
    let s = v
        .get("start")
        .and_then(serde_json::Value::as_str)
        .and_then(at);
    let e = v
        .get("end")
        .and_then(serde_json::Value::as_str)
        .and_then(at);
    match (s, e) {
        (Some(s), Some(e)) => (s, e),
        _ => (at(start).unwrap(), at(end).unwrap()),
    }
}

/// The person's timezone from preferences, UTC without one.
pub fn tz_of(prefs: &serde_json::Value) -> chrono_tz::Tz {
    prefs
        .get("timezone")
        .and_then(serde_json::Value::as_str)
        .and_then(|z| z.parse().ok())
        .unwrap_or(chrono_tz::Tz::UTC)
}

/// The person's resolved loop settings.
pub struct Settings {
    pub tz: chrono_tz::Tz,
    pub working: (u32, u32),
    pub quiet: (u32, u32),
    pub every_minutes: u32,
    pub per_day: u32,
}

/// Resolve settings: preferences over the `[proactive]` config, defaults
/// where either is missing or unparsable. Working 9 to 5, quiet 10pm to
/// 7am, weekly cadence floor of 5 minutes.
pub fn settings(cfg: &crate::config::Proactive, prefs: &serde_json::Value) -> Settings {
    let num = |k: &str, dflt: u32| {
        prefs
            .get(k)
            .and_then(serde_json::Value::as_u64)
            .map(|n| n as u32)
            .filter(|n| *n > 0)
            .unwrap_or(dflt)
    };
    Settings {
        tz: prefs
            .get("timezone")
            .and_then(serde_json::Value::as_str)
            .and_then(|z| z.parse().ok())
            .unwrap_or(chrono_tz::Tz::UTC),
        working: prefs
            .get("working_hours")
            .map(|w| window(w, "09:00", "17:00"))
            .unwrap_or((540, 1020)),
        quiet: prefs
            .get("quiet_hours")
            .map(|w| window(w, "22:00", "07:00"))
            .unwrap_or((1320, 420)),
        every_minutes: num("review_every_minutes", cfg.review_every_minutes).max(5),
        per_day: num("interruptions_per_day", cfg.interruptions_per_day),
    }
}

/// Minutes since midnight where the person is.
pub fn minutes_at(now: DateTime<Utc>, tz: chrono_tz::Tz) -> u32 {
    let local = now.with_timezone(&tz);
    local.hour() * 60 + local.minute()
}

/// Whether minute-of-day `m` falls in the window, which may cross midnight
/// (an end at or before its start runs into the next day).
pub fn in_window(m: u32, (start, end): (u32, u32)) -> bool {
    if end > start {
        m >= start && m < end
    } else {
        m >= start || m < end
    }
}

/// The person's calendar day, for the daily interruption budget.
pub fn day_key(now: DateTime<Utc>, tz: chrono_tz::Tz) -> String {
    now.with_timezone(&tz).date_naive().to_string()
}

/// When the current quiet hours end, for delaying an interruption's
/// notification until then. None when it is not quiet now.
pub fn quiet_ends_at(
    now: DateTime<Utc>,
    tz: chrono_tz::Tz,
    quiet: (u32, u32),
) -> Option<DateTime<Utc>> {
    if !in_window(minutes_at(now, tz), quiet) {
        return None;
    }
    let local = now.with_timezone(&tz);
    let mut day = local.date_naive();
    if quiet.1 <= quiet.0 && local.hour() * 60 + local.minute() >= quiet.0 {
        day = day.succ_opt()?;
    }
    let naive = day.and_hms_opt(quiet.1 / 60, quiet.1 % 60, 0)?;
    Some(
        tz.from_local_datetime(&naive)
            .earliest()?
            .with_timezone(&Utc),
    )
}

/// The window's next start, for holding an interruption's notification
/// until working hours. None when in the window now.
pub fn window_next_start(
    now: DateTime<Utc>,
    tz: chrono_tz::Tz,
    window: (u32, u32),
) -> Option<DateTime<Utc>> {
    if in_window(minutes_at(now, tz), window) {
        return None;
    }
    let local = now.with_timezone(&tz);
    let mut day = local.date_naive();
    if window.1 > window.0 && local.hour() * 60 + local.minute() >= window.0 {
        day = day.succ_opt()?;
    }
    let naive = day.and_hms_opt(window.0 / 60, window.0 % 60, 0)?;
    Some(
        tz.from_local_datetime(&naive)
            .earliest()?
            .with_timezone(&Utc),
    )
}

/// A coming-up schedule's wake reason, carrying its id for the once-a-day
/// mark the log keeps (#237).
pub fn schedule_reason(id: &str) -> String {
    format!("schedule {id} coming up")
}

/// A due commitment's wake reason, carrying its id likewise.
pub fn commitment_reason(id: &str) -> String {
    format!("commitment {id} due")
}

/// A stale approval's wake reason, carrying its item likewise.
pub fn approval_reason(id: &str) -> String {
    format!("approval {id} waiting")
}

impl crate::kernel::Kernel {
    /// Wake review runs whose time has come: cadence in working hours, once
    /// overnight, coming-up schedules, due commitments and stale approvals
    /// (#237). Called from the sweep.
    pub async fn review_due(self: &Arc<Self>, now: DateTime<Utc>) {
        if !self.cfg.proactive.on {
            return;
        }
        let people: Vec<String> = self.cfg.people.iter().map(|p| p.id.clone()).collect();
        for person in people {
            if self.read(|s| {
                s.turned_off.contains(&person) || s.review_runs.values().any(|p| p == &person)
            }) {
                continue;
            }
            let prefs = self.read(|s| {
                s.preferences
                    .get(&person)
                    .cloned()
                    .unwrap_or(serde_json::Value::Null)
            });
            let st = settings(&self.cfg.proactive, &prefs);
            let m = minutes_at(now, st.tz);
            let today = day_key(now, st.tz);
            let (last, overnight) = self.read(|s| {
                (
                    s.reviews_last.get(&person).cloned(),
                    s.reviews_overnight.get(&person).cloned(),
                )
            });
            // One sweep starts at most one review per person: an event
            // first (it is marked, so the cadence cannot starve it, and it
            // is the more urgent), else the periodic one (#381).
            let event: Option<(String, Option<Ended>)> = self.read(|s| {
                for sc in s
                    .schedules
                    .values()
                    .filter(|sc| sc.person == person && sc.paused.is_none())
                {
                    if sc.next_due().is_some_and(|d| {
                        d > now && d <= now + chrono::Duration::minutes(st.every_minutes as i64)
                    }) && !s
                        .wake_marks
                        .get(&format!("{person}:schedule {}", sc.id))
                        .is_some_and(|d| d == &today)
                    {
                        return Some((schedule_reason(&sc.id), None));
                    }
                }
                for c in s
                    .commitments
                    .values()
                    .filter(|c| c.person == person && c.status == "open")
                {
                    if c.by
                        .as_deref()
                        .and_then(crate::commitments::due)
                        .is_some_and(|d| d > now && d <= now + chrono::Duration::hours(24))
                        && !s
                            .wake_marks
                            .get(&format!("{person}:commitment {}", c.id))
                            .is_some_and(|d| d == &today)
                    {
                        return Some((commitment_reason(&c.id), None));
                    }
                }
                for i in s.items.values().filter(|i| {
                    i.person == person && i.kind == "approval" && i.resolution.is_none()
                }) {
                    if now.signed_duration_since(i.created_at).num_minutes() >= 30
                        && !s
                            .wake_marks
                            .get(&format!("{person}:approval {}", i.id))
                            .is_some_and(|d| d == &today)
                    {
                        return Some((approval_reason(&i.id), None));
                    }
                }
                None
            });
            if let Some((reason, ended)) = event {
                self.start_review(&person, reason, ended).await;
            } else if in_window(m, st.working)
                && last.is_none_or(|t| {
                    now.signed_duration_since(t).num_minutes() >= st.every_minutes as i64
                })
            {
                self.start_review(&person, "cadence".into(), None).await;
            } else if !in_window(m, st.working) && overnight.as_deref() != Some(today.as_str()) {
                self.start_review(&person, "overnight".into(), None).await;
            }
        }
    }

    /// Wake one review for an event (a task done or failed), unless one is
    /// already going or this task already woke one (#237, #383). The wake
    /// used to skip when any review had woken within five minutes, which
    /// silently dropped failures after a cadence review or a boot; a
    /// per-task mark is the precise dedup (a task ends once).
    pub async fn review_wake(self: &Arc<Self>, person: &str, reason: String, ended: Option<Ended>) {
        if !self.cfg.proactive.on {
            return;
        }
        if self.read(|s| {
            s.review_runs.values().any(|p| p == person)
                || ended.as_ref().is_some_and(|e| s.wake_marks.contains_key(&format!("{person}:task {}", e.task)))
        }) {
            return;
        }
        self.start_review(person, reason, ended).await;
    }

    /// Start the review run itself: the review instruction over the
    /// person's state, on a small wake budget (#237).
    async fn start_review(self: &Arc<Self>, person: &str, reason: String, ended: Option<Ended>) {
        let context = self.review_context(person, ended.as_ref()).await;
        let text = format!("Review the person's state ({reason}).\n\n{context}");
        let budget = crate::config::Budget {
            max_model_calls: 10,
            max_actions: 10,
            max_usd: self.cfg.proactive.wake_max_usd,
        };
        match self
            .create_run(
                "review",
                person,
                text,
                vec![],
                crate::kernel::NewRun {
                    budget: Some(budget),
                    // The outcome can quote what the task read: the review
                    // starts with the ended run's taint, never clean (#390).
                    label: ended.map(|e| e.label),
                    ..Default::default()
                },
            )
            .await
        {
            Ok((task, run)) => {
                let _ = self
                    .emit(
                        "kernel",
                        crate::events::Ev::ReviewWoken {
                            run,
                            task,
                            person: person.into(),
                            reason,
                        },
                    )
                    .await;
            }
            Err(e) => tracing::warn!("review wake for {person}: {e:#}"),
        }
    }

    /// The review's context: the ended task that woke it, if any, then
    /// unfinished work, schedules, approvals, disagreements and
    /// commitments (#237, #383).
    async fn review_context(&self, person: &str, ended: Option<&Ended>) -> String {
        let (tasks, schedules, approvals, disagreements, commitments) = self.read(|s| {
            let tasks = s
                .tasks
                .values()
                .filter(|t| {
                    t.person == person
                        && !t.review
                        && s.runs.get(&t.run_id).is_some_and(|r| !r.status.terminal())
                })
                .map(|t| (t.person.clone(), t.seq))
                .collect::<Vec<_>>();
            let schedules = s
                .schedules
                .values()
                .filter(|sc| sc.person == person)
                .map(|sc| {
                    format!(
                        "{}: {}{}",
                        sc.id,
                        sc.paused
                            .as_deref()
                            .map(|r| format!("paused ({r})"))
                            .unwrap_or_else(|| format!(
                                "next {}",
                                sc.next_due()
                                    .map(|d| d.to_rfc3339())
                                    .unwrap_or_else(|| "unscheduled".into())
                            )),
                        if sc.failures.is_empty() {
                            ""
                        } else {
                            " (failing)"
                        }
                    )
                })
                .collect::<Vec<_>>();
            let approvals = s
                .items
                .values()
                .filter(|i| i.person == person && i.kind == "approval" && i.resolution.is_none())
                .map(|i| i.summary.clone())
                .collect::<Vec<_>>();
            let disagreements = s
                .disagreements
                .values()
                .filter(|d| d.person == person && d.outcome.is_none())
                .map(|d| {
                    format!(
                        "{} -> {}",
                        d.position.chars().take(120).collect::<String>(),
                        d.decision
                    )
                })
                .collect::<Vec<_>>();
            let commitments = s
                .commitments
                .values()
                .filter(|c| c.person == person && c.status == "open")
                .map(|c| {
                    format!(
                        "{} (by {}){}",
                        c.id,
                        c.by.as_deref().unwrap_or("no date"),
                        if c.by
                            .as_deref()
                            .and_then(crate::commitments::due)
                            .is_some_and(|d| d < Utc::now())
                        {
                            ", OVERDUE"
                        } else {
                            ""
                        }
                    )
                })
                .collect::<Vec<_>>();
            (tasks, schedules, approvals, disagreements, commitments)
        });
        let mut open = vec![];
        if let Ok(words) = self.words(tasks).await {
            open = self.read(|s| {
                s.tasks
                    .values()
                    .filter(|t| {
                        t.person == person
                            && !t.review
                            && s.runs.get(&t.run_id).is_some_and(|r| !r.status.terminal())
                    })
                    .map(|t| {
                        format!(
                            "{}: {}",
                            t.task_id,
                            words.text(t.seq).chars().take(160).collect::<String>()
                        )
                    })
                    .collect::<Vec<_>>()
            });
        }
        let section = |title: &str, items: Vec<String>| {
            if items.is_empty() {
                String::new()
            } else {
                format!(
                    "{title}:\n{}\n\n",
                    items
                        .iter()
                        .take(10)
                        .map(|i| format!("- {i}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            }
        };
        // The ended task first: the unfinished-work section excludes it,
        // and the review cannot judge the failure without it (#383).
        let woke = ended.map(|e| format!("The task that woke this review:\n- {}: {}\n\n", e.task, e.outcome.chars().take(500).collect::<String>())).unwrap_or_default();
        format!(
            "{woke}{}{}{}{}{}",
            section("Unfinished work", open),
            section("Schedules", schedules),
            section("Approvals waiting", approvals),
            section("Open disagreements", disagreements),
            section("Open commitments", commitments)
        )
        .chars()
        .take(6000)
        .collect()
    }

    /// Now, or when quiet ends while it is quiet: when an observation or
    /// interruption may ask for attention (#237).
    pub fn notify_due(&self, person: &str) -> DateTime<Utc> {
        let prefs = self.read(|s| {
            s.preferences
                .get(person)
                .cloned()
                .unwrap_or(serde_json::Value::Null)
        });
        let st = settings(&self.cfg.proactive, &prefs);
        let now = Utc::now();
        quiet_ends_at(now, st.tz, st.quiet).unwrap_or(now)
    }

    /// Deliver an interruption, or queue it as an observation: outside
    /// working hours or over budget it waits for the next session; in
    /// quiet hours it posts now and pings when quiet ends (#237). What
    /// the `interrupt` record says back.
    pub async fn interrupt_for(
        &self,
        run: &crate::state::Run,
        what: &str,
        why: &str,
        call_id: &str,
    ) -> anyhow::Result<String> {
        let prefs = self.read(|s| {
            s.preferences
                .get(&run.person)
                .cloned()
                .unwrap_or(serde_json::Value::Null)
        });
        let st = settings(&self.cfg.proactive, &prefs);
        let now = Utc::now();
        if !in_window(minutes_at(now, st.tz), st.working) {
            let at = window_next_start(now, st.tz, st.working).unwrap_or(now);
            let item = self.item(run, "observation", what.to_string(), serde_json::json!({ "why": why, "evidence": "queued: outside working hours", "call_id": call_id, "notify_at": at.to_rfc3339() })).await?;
            return Ok(format!(
                "Outside working hours: queued as observation {item}; it pings at {}.",
                at.with_timezone(&st.tz).format("%H:%M")
            ));
        }
        let today = day_key(now, st.tz);
        let n = self.read(|s| {
            s.interrupts
                .get(&run.person)
                .filter(|(d, _)| *d == today)
                .map(|(_, n)| *n)
                .unwrap_or(0)
        });
        if n >= st.per_day {
            let at = quiet_ends_at(now, st.tz, st.quiet).unwrap_or(now);
            let item = self.item(run, "observation", what.to_string(), serde_json::json!({ "why": why, "evidence": "queued: interruption budget spent", "call_id": call_id, "notify_at": at.to_rfc3339() })).await?;
            return Ok(format!(
                "Interruption budget spent ({n} today): queued as observation {item} for the next session."
            ));
        }
        let at = quiet_ends_at(now, st.tz, st.quiet).unwrap_or(now);
        let item = self
            .item(
                run,
                "interrupt",
                what.to_string(),
                serde_json::json!({ "why": why, "call_id": call_id, "notify_at": at.to_rfc3339() }),
            )
            .await?;
        if at > now {
            Ok(format!(
                "Posted as interruption {item}; it pings when quiet ends at {}.",
                at.with_timezone(&st.tz).format("%H:%M")
            ))
        } else {
            Ok(format!("Delivered as interruption {item}."))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows hold across midnight; garbage falls back to defaults.
    #[test]
    fn hours_windows_and_fallbacks() {
        assert!(in_window(600, (540, 1020)) && !in_window(1020, (540, 1020)));
        assert!(
            in_window(1400, (1320, 420))
                && in_window(100, (1320, 420))
                && !in_window(500, (1320, 420))
        );
        let cfg = crate::config::Proactive::default();
        let s = settings(
            &cfg,
            &serde_json::json!({ "working_hours": { "start": "xx", "end": "17:00" }, "timezone": "Mars/Olympus", "review_every_minutes": 0 }),
        );
        assert_eq!(
            ((s.working, s.tz, s.every_minutes)),
            ((540, 1020), chrono_tz::Tz::UTC, 60)
        );
        assert_eq!(
            day_key(
                DateTime::parse_from_rfc3339("2026-11-07T06:30:00Z")
                    .unwrap()
                    .into(),
                chrono_tz::America::Los_Angeles
            ),
            "2026-11-06"
        );
        let ends = quiet_ends_at(
            DateTime::parse_from_rfc3339("2026-11-07T06:30:00Z")
                .unwrap()
                .into(),
            chrono_tz::Tz::UTC,
            (1320, 420),
        )
        .unwrap();
        assert_eq!(ends.to_rfc3339(), "2026-11-07T07:00:00+00:00");
        assert!(quiet_ends_at(
            DateTime::parse_from_rfc3339("2026-11-07T12:00:00Z")
                .unwrap()
                .into(),
            chrono_tz::Tz::UTC,
            (1320, 420)
        )
        .is_none());
        let work = (540, 1020);
        assert!(window_next_start(
            DateTime::parse_from_rfc3339("2026-11-07T12:00:00Z")
                .unwrap()
                .into(),
            chrono_tz::Tz::UTC,
            work
        )
        .is_none());
        assert_eq!(
            window_next_start(
                DateTime::parse_from_rfc3339("2026-11-07T08:00:00Z")
                    .unwrap()
                    .into(),
                chrono_tz::Tz::UTC,
                work
            )
            .unwrap()
            .to_rfc3339(),
            "2026-11-07T09:00:00+00:00"
        );
        assert_eq!(
            window_next_start(
                DateTime::parse_from_rfc3339("2026-11-07T18:00:00Z")
                    .unwrap()
                    .into(),
                chrono_tz::Tz::UTC,
                work
            )
            .unwrap()
            .to_rfc3339(),
            "2026-11-08T09:00:00+00:00"
        );
    }
}
