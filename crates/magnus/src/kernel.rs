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
    pub(crate) log: Log,
    state: Mutex<State>,
    write: tokio::sync::Mutex<()>,
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
    pub(crate) live_tools: tokio::sync::Mutex<HashMap<String, (Instant, Vec<AppTool>)>>,
    pub live_text: Arc<Live>,
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
    /// The adapter and model the person chose; the appliance's otherwise.
    pub model: Option<(String, String)>,
    /// A short title the model wrote when starting it.
    pub title: Option<String>,
    /// Library files the person gave it.
    pub files: Vec<String>,
    /// The project it starts in.
    pub project: Option<String>,
    /// Removable modules switched on or off for this run alone, over the
    /// model's settings (an ablation trial).
    pub modules: std::collections::BTreeMap<String, bool>,
}

impl Kernel {
    pub async fn boot(cfg: Config) -> Result<Arc<Kernel>> {
        std::fs::create_dir_all(&cfg.appliance.state_dir)?;
        // Just switched releases: the one before may still be letting go of the log.
        let log = match crate::update::switching(&cfg) {
            true => Log::open_waiting(&cfg.database.url, Duration::from_secs(15)).await?,
            false => Log::open(&cfg.database.url).await?,
        };
        let mut state = State::default();
        for e in log.load().await? {
            state.apply(&e);
        }
        let vault = Arc::new(Vault::open(&cfg.appliance.state_dir)?);
        if state.kill_switch {
            vault.seal();
        }
        let accounts = crate::accounts::Accounts::open(&cfg.appliance.state_dir, cfg.sign_in.clone())?;
        let oidc = cfg.sign_in.oidc.clone().map(crate::oidc::Provider::new).transpose()?;
        let adapters = Adapters::new(&cfg.appliance.state_dir, vault.clone(), &cfg.adapters);
        let live_text = Arc::new(Live { tx: tokio::sync::broadcast::channel(1024).0, so_far: Mutex::new(HashMap::new()) });
        let k = Arc::new(Kernel {
            web: Web::new(&cfg.web),
            sandboxes: Sandboxes::new(cfg.sandbox.clone(), &cfg.appliance.state_dir),
            browser: crate::browser::Browser::new(cfg.browser.clone(), &cfg.appliance.state_dir),
            mcp: Mcp::new(),
            adapters,
            vault,
            accounts,
            oidc,
            log,
            state: Mutex::new(state),
            write: tokio::sync::Mutex::new(()),
            drivers: Mutex::new(HashSet::new()),
            leases: Mutex::new(HashMap::new()),
            live_tools: tokio::sync::Mutex::new(HashMap::new()),
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

    /// Append several events in one write and apply them together: no
    /// reader sees some of them without the rest.
    pub async fn emit_all(&self, actor: &str, evs: Vec<Ev>) -> Result<Vec<Event>> {
        let _g = self.write.lock().await;
        let es = self.log.append_all(actor, &evs).await?;
        self.capture.lock().unwrap().appends += es.len() as u64;
        let mut s = self.state.lock().unwrap();
        for e in &es {
            s.apply(e);
        }
        Ok(es)
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

    /// A person as every decision sees them: from the config, with the
    /// groups and roles the identity provider gave at their latest sign-in
    /// in place of the config's (E2).
    /// A person's rights: the identity provider's where it names them, else
    /// the roles the owner set in Settings, else the config's.
    pub fn person(&self, id: &str) -> Option<Person> {
        let mut p = self.cfg.person(id)?.clone();
        if let Some(r) = self.read(|s| s.roles_set.get(id).cloned()) {
            p.roles = r;
        }
        if let Some((groups, roles)) = self.read(|s| s.provider_rights.get(id).cloned()) {
            if let Some(g) = groups {
                p.groups = g;
            }
            if let Some(r) = roles {
                p.roles = r;
            }
        }
        Some(p)
    }

    pub fn owners(&self) -> Vec<Person> {
        self.cfg.people.iter().filter_map(|p| self.person(&p.id)).filter(|p| p.is_owner()).collect()
    }

    /// Start driving a run if nothing is.
    pub fn wake(self: &Arc<Self>, run: &str) {
        let files = self.read(|s| {
            let task = s.runs.get(run).map(|r| r.task.clone()).unwrap_or_default();
            let mut v = s.attachments.get(&task).cloned().unwrap_or_default();
            v.extend(s.tasks.get(&task).into_iter().flat_map(|t| &t.files).filter_map(|id| s.files.get(id)).map(|f| serde_json::json!({ "name": f.name, "data_b64": f.data_b64 })));
            v
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
                // Stop it with a notice rather than retry it unseen (K24).
                let running = k.read(|s| s.runs.get(&run).is_some_and(|r| r.status == Status::Running));
                if running {
                    let title = k.read(|s| s.runs.get(&run).cloned()).map(|r| k.title_of(&r)).unwrap_or_default();
                    if let Err(e2) = k.suspend_with_notice(&run, "error", format!("Stopped by an error: {title}"), serde_json::json!({ "error": format!("{e:#}") })).await {
                        tracing::error!("run {run}: could not record the error: {e2:#}");
                    }
                }
            }
        });
    }

    pub fn any_driving(&self) -> bool {
        !self.drivers.lock().unwrap().is_empty()
    }

    pub fn driving(&self, run: &str) -> bool {
        self.drivers.lock().unwrap().contains(run)
    }

    /// Create a task for `person` with the current model and budgets.
    pub async fn create_task(self: &Arc<Self>, actor: &str, person: &str, text: String, attachments: Vec<serde_json::Value>) -> Result<(String, String)> {
        self.create_tagged(actor, person, text, attachments, None, None, Default::default()).await
    }

    /// Create a task, optionally as a trial of an experiment case (V2).
    pub async fn create_tagged(
        self: &Arc<Self>,
        actor: &str,
        person: &str,
        text: String,
        attachments: Vec<serde_json::Value>,
        experiment: Option<crate::events::ExperimentTag>,
        model: Option<(String, String)>,
        modules: std::collections::BTreeMap<String, bool>,
    ) -> Result<(String, String)> {
        self.create_run(actor, person, text, attachments, NewRun { experiment, model, modules, ..Default::default() }).await
    }

    /// The adapter and model new runs use unless the person chooses one.
    pub fn default_model(&self) -> (String, String) {
        let m = &self.cfg.model;
        self.read(|s| s.model.clone()).unwrap_or_else(|| (m.adapter.clone(), m.name.clone()))
    }

    /// The models a person may choose for a task or a chat message: each
    /// adapter's catalog as its provider lists it now. An adapter whose
    /// provider does not answer (not signed in, unreachable) offers none.
    pub async fn models(&self) -> Vec<(String, Vec<String>)> {
        let ids: Vec<String> = self.adapters.list().await.into_iter().map(|(id, ..)| id).collect();
        let lists = futures::future::join_all(ids.iter().map(|id| self.adapters.catalog(id))).await;
        ids.into_iter()
            .zip(lists)
            .filter_map(|(id, ev)| match ev {
                Ok(magnus_adapter_proto::Event::Catalog { models }) if !models.is_empty() => Some((id, models.into_iter().map(|m| m.id).collect())),
                _ => None,
            })
            .collect()
    }

    /// A model the person chose, checked against its adapter's catalog. The
    /// appliance's own default needs no check.
    pub async fn check_model(&self, adapter: &str, model: &str) -> std::result::Result<(), String> {
        if self.default_model() == (adapter.to_string(), model.to_string()) {
            return Ok(());
        }
        if self.adapters.list().await.iter().all(|(id, ..)| id != adapter) {
            return Err(format!("no adapter {adapter}"));
        }
        match self.adapters.catalog(adapter).await {
            Ok(magnus_adapter_proto::Event::Catalog { models }) if models.iter().any(|m| m.id == model) => Ok(()),
            Ok(magnus_adapter_proto::Event::Catalog { .. }) => Err(format!("{adapter} does not offer {model}")),
            Ok(_) => Err(format!("{adapter} sent no model list")),
            Err(e) => Err(format!("{adapter} cannot list its models: {e:#}")),
        }
    }

    /// Create a task or a chat turn and start it.
    pub async fn create_run(self: &Arc<Self>, actor: &str, person: &str, text: String, attachments: Vec<serde_json::Value>, o: NewRun) -> Result<(String, String)> {
        let (task, run) = match &o.ids {
            Some((t, r)) => (t.clone(), r.clone()),
            None => (new_id("task"), new_id("run")),
        };
        let m = &self.cfg.model;
        let (adapter, model) = o.model.clone().unwrap_or_else(|| self.default_model());
        let (mut modules, escalate_to) = self.cfg.modules_for(&adapter, &model);
        modules.extend(o.modules.clone());
        let config = crate::events::RunConfig {
            adapter,
            model,
            max_retries: m.max_retries,
            max_tokens: m.max_tokens,
            budget: o.budget.unwrap_or_else(|| self.cfg.budget.clone()),
            verifier: modules.get("verifier").copied().unwrap_or(false),
            modules,
            escalate_to,
            lost: self.cfg.modules.lost.clone(),
            verifier_model: self.cfg.verifier.model.clone(),
            instructions: if o.chat.as_ref().is_some_and(|c| c.turn) { crate::run::chat_instructions_version() } else { crate::run::instructions_version() },
            harness: HARNESS_VERSION.into(),
            playbook: self.read(|s| s.playbook_version),
        };
        // The project's instructions and reference files as they stand now.
        let project = o.project.as_ref().and_then(|id| {
            self.read(|s| {
                let p = s.projects.get(id).filter(|p| p.person == person)?;
                let mut files: Vec<&crate::state::LibFile> = s.files.values().filter(|f| f.person == person && s.project_of(&f.id).as_deref() == Some(id)).collect();
                files.sort_by_key(|f| f.at);
                Some(crate::events::ProjectContext { id: p.id.clone(), name: p.name.clone(), instructions: p.instructions.clone(), files: files.into_iter().map(|f| f.id.clone()).collect() })
            })
        });
        self.emit(
            actor,
            Ev::TaskCreated {
                task: task.clone(),
                run: run.clone(),
                person: person.into(),
                text,
                attachments,
                config,
                experiment: o.experiment,
                chat: o.chat,
                label: o.label,
                title: o.title,
                files: o.files,
                project,
            },
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
                // Its runs start in the project the schedule is in now.
                let o = NewRun {
                    budget: sc.budget.clone(),
                    label: (sc.label != crate::tools::Label::PRINCIPAL).then_some(sc.label),
                    project: self.read(|s| s.project_of(&sc.id)),
                    ..Default::default()
                };
                match self.create_run(&format!("schedule:{}", sc.id), &sc.person, sc.text.clone(), vec![], o).await {
                    Ok((task, _)) => {
                        let _ = self.emit("kernel", Ev::ScheduleFired { id: sc.id.clone(), task }).await;
                    }
                    Err(e) => {
                        tracing::error!("schedule {}: {e:#}", sc.id);
                        let _ = self.emit("kernel", Ev::ScheduleFailed { id: sc.id.clone(), reason: format!("{e:#}") }).await;
                        let _ = self
                            .post_notice(None, None, sc.person.clone(), format!("A schedule could not start: {}", sc.title.clone().unwrap_or_else(|| sc.text.chars().take(80).collect())), serde_json::json!({ "reason": "schedule_failed", "evidence": { "schedule": sc.id, "error": format!("{e:#}") } }))
                            .await;
                    }
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
            self.browser.close(run).await;
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

    /// Stop a run on an inbox item (`item`, when it is new) in one write:
    /// nobody sees the item while the run still shows as running.
    pub async fn suspend_on(&self, run: &str, reason: &str, id: String, item: Option<Ev>) -> Result<()> {
        let stop = Ev::Suspended { run: run.into(), reason: reason.into(), item: Some(id) };
        self.emit_all("kernel", item.into_iter().chain([stop]).collect()).await?;
        self.on_stop(run, false).await;
        Ok(())
    }

    /// Code stops a run and tells the person why, with the evidence (K24).
    pub async fn suspend_with_notice(&self, run: &str, reason: &str, summary: String, details: serde_json::Value) -> Result<()> {
        let (task, person) = self.read(|s| s.runs.get(run).map(|r| (r.task.clone(), r.person.clone()))).unwrap_or_default();
        let (id, ev) = notice_event(Some(task), Some(run.into()), person, summary, serde_json::json!({ "reason": reason, "evidence": details }));
        self.suspend_on(run, reason, id, Some(ev)).await
    }

    /// A notice in the person's inbox: something ended or stopped, with the evidence.
    pub async fn post_notice(&self, task: Option<String>, run: Option<String>, person: String, summary: String, details: serde_json::Value) -> Result<String> {
        let (id, ev) = notice_event(task, run, person, summary, details);
        self.emit("kernel", ev).await?;
        Ok(id)
    }

    pub async fn notice(&self, run: &crate::state::Run, summary: String, details: serde_json::Value) -> Result<String> {
        self.post_notice(Some(run.task.clone()), Some(run.id.clone()), run.person.clone(), summary, details).await
    }

    /// The task's title, or the start of what the person asked.
    pub fn title_of(&self, run: &crate::state::Run) -> String {
        self.read(|s| s.tasks.get(&run.task).map(|t| t.title.clone().unwrap_or_else(|| t.text.chars().take(80).collect()))).unwrap_or_default()
    }

    /// The run's last few tool results, for a notice's evidence.
    pub fn last_results(&self, run: &crate::state::Run) -> Vec<serde_json::Value> {
        let r = self.read(|s| s.runs.get(&run.id).cloned()).unwrap_or_else(|| run.clone());
        r.results_in_order().into_iter().rev().take(3).collect()
    }

    /// In a chat turn: the actions the person asked for (start a task,
    /// propose a schedule) whose every attempt failed, with the reason.
    pub fn unfinished_chat_actions(&self, run: &crate::state::Run) -> Vec<serde_json::Value> {
        let r = self.read(|s| s.runs.get(&run.id).cloned()).unwrap_or_else(|| run.clone());
        let mut out: Vec<serde_json::Value> = vec![];
        for tool in ["create_task", "create_schedule"] {
            let tried: Vec<serde_json::Value> = r.results_in_order().into_iter().filter(|v| v["tool"] == tool).collect();
            if !tried.is_empty() && tried.iter().all(|v| v["is_error"] == true) {
                out.push(serde_json::json!({ "tool": tool, "attempts": tried.len(), "error": tried.last().map(|v| v["text"].clone()) }));
            }
        }
        out
    }
}

fn notice_event(task: Option<String>, run: Option<String>, person: String, summary: String, details: serde_json::Value) -> (String, Ev) {
    let id = new_id("item");
    (id.clone(), Ev::Item { id, item_kind: "notice".into(), task, run, person, summary, details })
}

/// Model text as it streams, before the reply is appended to the log (K6),
/// and each run's reply so far, so a view that opens mid-reply starts with
/// what was already said.
pub struct Live {
    tx: tokio::sync::broadcast::Sender<(String, String)>,
    so_far: Mutex<HashMap<String, String>>,
}

impl Live {
    pub fn send(&self, run: &str, text: &str) {
        let mut so_far = self.so_far.lock().unwrap();
        so_far.entry(run.to_string()).or_default().push_str(text);
        let _ = self.tx.send((run.to_string(), text.to_string()));
    }

    /// The run's reply is complete (or failed): nothing more streams for it.
    pub fn end(&self, run: &str) {
        self.so_far.lock().unwrap().remove(run);
    }

    /// The run's reply so far and what streams after it, with nothing lost
    /// or repeated between the two.
    pub fn subscribe(&self, run: &str) -> (String, tokio::sync::broadcast::Receiver<(String, String)>) {
        let so_far = self.so_far.lock().unwrap();
        (so_far.get(run).cloned().unwrap_or_default(), self.tx.subscribe())
    }
}
