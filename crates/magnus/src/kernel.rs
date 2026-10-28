//! The kernel: wiring, the single writer, boot, and the sweeper that keeps
//! runs moving. Every change goes through `emit`: append to the log first,
//! then apply to the projection.

use crate::adapters::Adapters;
use crate::apps::{AppTool, Mcp};
use crate::config::{Config, Person};
use crate::events::Ev;
use crate::log::{Event, Log};
use crate::sandbox::Sandboxes;
use crate::state::{State, Status};
use crate::vault::Vault;
use crate::web::Web;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const HARNESS_VERSION: &str = concat!("magnus-", env!("CARGO_PKG_VERSION"));

pub struct Kernel {
    pub cfg: Config,
    log: Log,
    state: Mutex<State>,
    write: tokio::sync::Mutex<()>,
    /// Held while a person's decision is checked and recorded, so an inbox
    /// item or a chat card is decided once however many answers race in.
    pub(crate) decide: tokio::sync::Mutex<()>,
    pub vault: Arc<Vault>,
    pub accounts: crate::accounts::Accounts,
    /// Sign-in through an identity provider, when one is configured.
    pub oidc: Option<crate::oidc::Provider>,
    pub adapters: Adapters,
    pub mcp: Mcp,
    pub web: Web,
    pub sandboxes: Sandboxes,
    pub browser: crate::browser::Browser,
    drivers: Mutex<HashSet<String>>,
    pub(crate) leases: Mutex<HashMap<String, String>>,
    /// Live tool lists, by `<connector>:<person>` (or the connector alone
    /// for one shared credential).
    pub(crate) live_tools: tokio::sync::Mutex<HashMap<String, (Instant, Vec<AppTool>)>>,
    pub(crate) connect: crate::connect::Flows,
    /// The browser's only way out.
    pub egress: crate::egress::Egress,
    pub live_text: tokio::sync::broadcast::Sender<(String, String)>,
    /// How long each log append took since boot, in microseconds (V6).
    pub capture: Mutex<Capture>,
}

/// Capture overhead: the cost of writing each step to the log (V6).
#[derive(Default, Clone, serde::Serialize)]
pub struct Capture {
    pub appends: u64,
    pub total_us: u64,
    pub max_us: u64,
    /// The most recent append times, for percentiles.
    #[serde(skip)]
    pub recent: std::collections::VecDeque<u64>,
}

pub fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
}

/// How a new run differs from a plain task.
#[derive(Default)]
pub struct NewRun {
    pub experiment: Option<crate::events::ExperimentTag>,
    pub chat: Option<crate::events::ChatOrigin>,
    pub label: Option<crate::tools::Label>,
    pub budget: Option<crate::config::Budget>,
    /// Fixed ids, so a resent request finds the task it already created.
    pub ids: Option<(String, String)>,
}

impl Kernel {
    pub async fn boot(cfg: Config) -> Result<Arc<Kernel>> {
        std::fs::create_dir_all(&cfg.appliance.state_dir)?;
        let log = Log::open(&cfg.database.url).await?;
        let mut state = State::default();
        for e in log.load().await? {
            state.apply(&e);
        }
        let vault = Arc::new(Vault::open(&cfg.appliance.state_dir)?);
        let accounts = crate::accounts::Accounts::open(&cfg.appliance.state_dir, cfg.sign_in.clone())?;
        let oidc = cfg.sign_in.oidc.clone().map(crate::oidc::Provider::new).transpose()?;
        let adapters = Adapters::new(&cfg.appliance.state_dir, vault.clone(), &cfg.adapters);
        let (live_text, _) = tokio::sync::broadcast::channel(1024);
        let k = Arc::new(Kernel {
            web: Web::new(&cfg.web),
            sandboxes: Sandboxes::new(cfg.sandbox.clone(), &cfg.appliance.state_dir),
            browser: crate::browser::Browser::new(cfg.browser.clone(), &cfg.appliance.state_dir),
            mcp: Mcp::new(&cfg.appliance.state_dir),
            adapters,
            vault,
            accounts,
            oidc,
            log,
            state: Mutex::new(state),
            write: tokio::sync::Mutex::new(()),
            decide: tokio::sync::Mutex::new(()),
            drivers: Mutex::new(HashSet::new()),
            leases: Mutex::new(HashMap::new()),
            live_tools: tokio::sync::Mutex::new(HashMap::new()),
            connect: Default::default(),
            egress: crate::egress::Egress::open(&cfg.appliance.state_dir, cfg.web.allow_private_networks, &cfg.web.extra_ca)?,
            live_text,
            capture: Mutex::new(Capture::default()),
            cfg,
        });
        k.emit("kernel", Ev::KernelBoot { version: HARNESS_VERSION.into(), pid: std::process::id(), instructions: crate::run::instructions_version() }).await?;
        // Guest processes did not survive the restart; tell runs that used a sandbox (K20).
        let restarted: Vec<(String, u32)> = k.read(|s| {
            s.runs
                .values()
                .filter(|r| !r.status.terminal() && r.intents.values().any(|i| i.tool.starts_with("sandbox_")))
                .map(|r| (r.id.clone(), r.replies.len() as u32))
                .collect()
        });
        for (run, step) in restarted {
            k.emit(
                "kernel",
                Ev::Notice {
                    run,
                    step,
                    text: "The appliance restarted. Processes in your sandbox were stopped; files in /workspace are kept.".into(),
                },
            )
            .await?;
        }
        Ok(k)
    }

    /// Append an event, then apply it. Only correctness writes come here (K6).
    pub async fn emit(&self, actor: &str, ev: Ev) -> Result<Event> {
        let _g = self.write.lock().await;
        let t = Instant::now();
        let e = self.log.append(actor, &ev).await?;
        let us = t.elapsed().as_micros() as u64;
        {
            let mut c = self.capture.lock().unwrap();
            c.appends += 1;
            c.total_us += us;
            c.max_us = c.max_us.max(us);
            c.recent.push_back(us);
            if c.recent.len() > 2000 {
                c.recent.pop_front();
            }
        }
        self.state.lock().unwrap().apply(&e);
        Ok(e)
    }

    pub async fn run_events(&self, run: &str) -> Result<Vec<Event>> {
        self.log.run_events(run).await
    }

    pub async fn output_bytes(&self, run: &str, id: &str) -> Result<Option<(String, String, Vec<u8>)>> {
        self.log.output(run, id).await
    }

    pub fn read<T>(&self, f: impl FnOnce(&State) -> T) -> T {
        f(&self.state.lock().unwrap())
    }

    pub fn person(&self, id: &str) -> Option<&Person> {
        self.cfg.person(id)
    }

    pub fn owners(&self) -> Vec<&Person> {
        self.cfg.people.iter().filter(|p| p.is_owner()).collect()
    }

    /// Start driving a run if nothing is.
    pub fn wake(self: &Arc<Self>, run: &str) {
        let files = self.read(|s| {
            let task = s.runs.get(run).map(|r| r.task.clone()).unwrap_or_default();
            s.attachments.get(&task).cloned().unwrap_or_default()
        });
        if !files.is_empty() {
            use base64::Engine;
            let decoded = files
                .iter()
                .filter_map(|a| {
                    let data = base64::engine::general_purpose::STANDARD.decode(a["data_b64"].as_str()?).ok()?;
                    Some((a["name"].as_str()?.to_string(), data))
                })
                .collect();
            self.sandboxes.set_inputs(run, decoded);
        }
        if !self.drivers.lock().unwrap().insert(run.to_string()) {
            return;
        }
        let k = self.clone();
        let run = run.to_string();
        tokio::spawn(async move {
            let r = k.clone().drive(&run).await;
            k.drivers.lock().unwrap().remove(&run);
            if let Err(e) = r {
                tracing::error!("run {run}: {e:#}");
            }
        });
    }

    pub fn driving(&self, run: &str) -> bool {
        self.drivers.lock().unwrap().contains(run)
    }

    /// Create a task for `person` with the current model and budgets.
    pub async fn create_task(self: &Arc<Self>, actor: &str, person: &str, text: String, attachments: Vec<serde_json::Value>) -> Result<(String, String)> {
        self.create_tagged(actor, person, text, attachments, None).await
    }

    /// Create a task, optionally as a trial of an experiment case (V2).
    pub async fn create_tagged(
        self: &Arc<Self>,
        actor: &str,
        person: &str,
        text: String,
        attachments: Vec<serde_json::Value>,
        experiment: Option<crate::events::ExperimentTag>,
    ) -> Result<(String, String)> {
        self.create_run(actor, person, text, attachments, NewRun { experiment, ..Default::default() }).await
    }

    /// Create a task or a chat turn and start it.
    pub async fn create_run(self: &Arc<Self>, actor: &str, person: &str, text: String, attachments: Vec<serde_json::Value>, o: NewRun) -> Result<(String, String)> {
        let (task, run) = match &o.ids {
            Some((t, r)) => (t.clone(), r.clone()),
            None => (new_id("task"), new_id("run")),
        };
        let m = &self.cfg.model;
        let (adapter, model) = self.read(|s| s.model.clone()).unwrap_or_else(|| (m.adapter.clone(), m.name.clone()));
        let config = crate::events::RunConfig {
            adapter,
            model,
            max_retries: m.max_retries,
            max_tokens: m.max_tokens,
            budget: o.budget.unwrap_or_else(|| self.cfg.budget.clone()),
            verifier: self.cfg.verifier.enabled,
            instructions: if o.chat.as_ref().is_some_and(|c| c.turn) { crate::run::chat_instructions_version() } else { crate::run::instructions_version() },
            harness: HARNESS_VERSION.into(),
            playbook: self.read(|s| s.playbook_version),
        };
        self.emit(
            actor,
            Ev::TaskCreated { task: task.clone(), run: run.clone(), person: person.into(), text, attachments, config, experiment: o.experiment, chat: o.chat, label: o.label },
        )
        .await?;
        self.wake(&run);
        Ok((task, run))
    }

    /// Keep runs moving: restart drivers for running runs, wake due timers,
    /// start scheduled tasks.
    pub async fn sweep(self: Arc<Self>) {
        loop {
            let now = chrono::Utc::now();
            let due_schedules: Vec<crate::state::Schedule> =
                self.read(|s| if s.kill_switch { vec![] } else { s.schedules.values().filter(|x| x.next_due().is_some_and(|t| t <= now)).cloned().collect() });
            for sc in due_schedules {
                // A scheduled task acts for the person who set it up (K54),
                // within the schedule's budget.
                let o = NewRun { budget: sc.budget.clone(), label: (sc.label != crate::tools::Label::PRINCIPAL).then_some(sc.label), ..Default::default() };
                match self.create_run(&format!("schedule:{}", sc.id), &sc.person, sc.text.clone(), vec![], o).await {
                    Ok((task, _)) => {
                        let _ = self.emit("kernel", Ev::ScheduleFired { id: sc.id.clone(), task }).await;
                    }
                    Err(e) => tracing::error!("schedule {}: {e:#}", sc.id),
                }
            }
            let (running, due): (Vec<String>, Vec<(String, u32)>) = self.read(|s| {
                let running = s.runs.values().filter(|r| r.status == Status::Running).map(|r| r.id.clone()).collect();
                let due = s
                    .runs
                    .values()
                    .filter(|r| r.status == Status::Suspended && r.reason.as_deref() == Some("timer") && r.wait_until.is_some_and(|t| t <= now))
                    .map(|r| (r.id.clone(), r.replies.len() as u32))
                    .collect();
                (running, due)
            });
            for (run, step) in due {
                let ok = self
                    .emit("kernel", Ev::Notice { run: run.clone(), step, text: format!("It is now {}. Your wait has ended.", now.to_rfc3339()) })
                    .await
                    .is_ok()
                    && self.emit("kernel", Ev::Resumed { run: run.clone() }).await.is_ok();
                if ok {
                    self.wake(&run);
                }
            }
            for run in running {
                if !self.driving(&run) {
                    self.wake(&run);
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Release a run's resources when it stops moving.
    pub async fn on_stop(&self, run: &str, terminal: bool) {
        self.leases.lock().unwrap().retain(|_, holder| holder != run);
        if terminal {
            self.browse_close(run).await;
            self.sandboxes.destroy(run).await;
        } else {
            self.sandboxes.stop(run).await;
        }
    }

    pub async fn suspend(&self, run: &str, reason: &str, item: Option<String>) -> Result<()> {
        self.emit("kernel", Ev::Suspended { run: run.into(), reason: reason.into(), item }).await?;
        self.on_stop(run, false).await;
        Ok(())
    }

    /// Code stops a run and tells the person why, with the evidence (K24).
    pub async fn suspend_with_notice(&self, run: &str, reason: &str, summary: String, details: serde_json::Value) -> Result<()> {
        let (task, person) = self.read(|s| s.runs.get(run).map(|r| (r.task.clone(), r.person.clone()))).unwrap_or_default();
        let id = new_id("item");
        self.emit(
            "kernel",
            Ev::Item {
                id: id.clone(),
                item_kind: "notice".into(),
                task: Some(task),
                run: Some(run.into()),
                person,
                summary,
                details: serde_json::json!({ "reason": reason, "evidence": details }),
            },
        )
        .await?;
        self.suspend(run, reason, Some(id)).await
    }
}
