// The operator view: Home, Inbox, New Task, Task Detail and Models, all from
// derived views of the log (K45). The developer view is in dev.js.
"use strict";

const $ = (s, el = document) => el.querySelector(s);
const $$ = (s, el = document) => [...el.querySelectorAll(s)];
const esc = s => String(s ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const api = (m, p, b) => Magnus.api(m, p, b);
const enc = encodeURIComponent;

const App = {
  me: null,
  appliance: "",
  inbox: [],
  versions: null,
  timers: [],
  leave: [],
  routes: [],
};

function toast(msg, ms = 3200) {
  const t = document.createElement("div");
  t.className = "toast"; t.textContent = msg; t.setAttribute("role", "status");
  document.body.append(t);
  setTimeout(() => t.remove(), ms);
}

function when(t) {
  if (!t) return "";
  const d = new Date(t), now = Date.now(), s = (now - d.getTime()) / 1000;
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return d.toLocaleString();
}

const isOwner = () => App.me?.roles?.includes("owner");
const isDev = () => App.me?.roles?.includes("developer");

// Replace a region's markup only when it changed, keeping what the person
// was typing (anything marked data-draft) and where the focus was.
const drafts = {};
function paint(el, html) {
  if (!el || el._html === html) return false;
  $$("[data-draft]", el).forEach(i => { drafts[i.dataset.draft] = i.value; });
  const focused = document.activeElement?.dataset?.draft;
  el.innerHTML = html;
  el._html = html;
  $$("[data-draft]", el).forEach(i => { if (drafts[i.dataset.draft] !== undefined) i.value = drafts[i.dataset.draft]; });
  if (focused) $(`[data-draft="${CSS.escape(focused)}"]`, el)?.focus();
  return true;
}

function every(ms, fn) { const t = setInterval(fn, ms); App.timers.push(t); }
function onLeave(fn) { App.leave.push(fn); }

const STATUS = {
  running: ["run", "Working"], suspended: ["warn", "Paused"], completed: ["ok", "Done"], failed: ["bad", "Failed"], cancelled: ["", "Cancelled"],
};
const WAITING = {
  approval: "waiting for an approval", review: "waiting for your review", question: "waiting for your answer", budget: "paused: budget reached",
  provider: "paused: the model provider is unavailable", usage_limit: "paused: model usage limit", auth: "paused: sign in to the model again",
  kill_switch: "stopped by the kill switch", timer: "waiting until a set time", invalid_request: "paused: the model rejected the request",
};
const chip = st => { const [c, l] = STATUS[st] || ["", st]; return `<span class="chip ${c}">${esc(l)}</span>`; };

// ---------- shell ----------

function nav() {
  const route = location.hash || "#/home";
  const blocking = App.inbox.filter(i => i.status === "open" && i.can_respond && i.blocking).length;
  const batched = App.inbox.filter(i => i.status === "open" && i.can_respond && !i.blocking).length;
  const link = (href, label, extra = "") => `<a href="${href}" class="${route === href || route.startsWith(href + "/") ? "on" : ""}">${label}${extra}</a>`;
  const badges = (blocking ? `<span class="badge" title="Needs you now">${blocking}</span>` : "") + (batched ? ` <span class="badge quiet" title="When you have a moment">${batched}</span>` : "");
  return `
    <div class="brand">Magnus</div>
    ${link("#/home", "Home")}
    ${link("#/inbox", "Inbox", `<span>${badges}</span>`)}
    ${link("#/new", "New task")}
    ${link("#/tasks", "Tasks")}
    ${isOwner() ? link("#/models", "Models") : ""}
    ${isDev() ? `<div class="group">Developer</div>
      ${link("#/dev/runs", "Runs")}
      ${link("#/dev/experiments", "Experiments")}
      ${link("#/dev/system", "System")}
      ${link("#/dev/policy", "Policy")}
      ${link("#/dev/playbook", "Playbook")}` : ""}
    <div class="who"><div>${esc(App.me?.name)} <span class="faint">(${esc(App.me?.id)})</span></div>
      <div class="faint small">${esc(App.appliance)}</div>
      <button class="link small" data-act="sign-out" style="margin-top:6px">Sign out</button></div>`;
}

function shell() {
  document.body.innerHTML = `<div class="shell"><nav class="side" aria-label="Sections"></nav><main id="main" tabindex="-1"></main></div>`;
  $("nav.side").addEventListener("click", async e => {
    if (e.target.closest("[data-act=sign-out]")) {
      stop();
      await Magnus.signOut();
      location.hash = "";
      start();
    }
  });
  paint($("nav.side"), nav());
}

function route(re, view) { App.routes.push([re, view]); }

async function go() {
  App.timers.forEach(clearInterval); App.timers = [];
  App.leave.forEach(f => { try { f(); } catch { /* ignore */ } }); App.leave = [];
  const h = location.hash || "#/home";
  paint($("nav.side"), nav());
  const main = $("#main");
  main._html = null;
  for (const [re, view] of App.routes) {
    const m = h.match(re);
    if (m) {
      try { await view(main, ...m.slice(1).map(decodeURIComponent)); }
      catch (e) { paint(main, `<div class="banner bad">${esc(e.message)}</div>`); }
      return;
    }
  }
  location.hash = "#/home";
}

// ---------- sign-in ----------

async function signInScreen(note) {
  document.body.innerHTML = `<div class="signin"><h1>Magnus</h1><p class="muted">Sign in to your appliance.</p><div class="card" id="si"></div></div>`;
  const box = $("#si");
  const ask = async (stage, data = {}) => {
    if (stage === "address") {
      paint(box, `${note ? `<p class="small" style="color:var(--bad)">${esc(note)}</p>` : ""}
        <form id="f"><label for="addr">Appliance address</label>
        <input id="addr" style="width:100%" placeholder="https://magnus.local:7443" value="${esc(data.url || localStorage.getItem("magnus.address") || "")}" required>
        <p class="small muted">The owner gives you this address.</p>
        <div class="row" style="margin-top:14px"><button class="primary">Continue</button></div></form>`);
      $("#f").onsubmit = async e => {
        e.preventDefault();
        const url = $("#addr").value.trim().replace(/\/+$/, "");
        try {
          const c = await Magnus.connect(url);
          localStorage.setItem("magnus.address", url);
          if (c.status === "trusted") ask("method", { url });
          else ask("pin", { url, ...c });
        } catch (err) { note = err.message || String(err); ask("address", { url }); }
      };
    } else if (stage === "pin") {
      const changed = data.status === "changed";
      paint(box, `<h3>${changed ? "The appliance's certificate changed" : "Check the appliance"}</h3>
        <p class="small">${changed ? "This is not the certificate you trusted before. Continue only if the owner told you it was replaced." : "This is the first connection to this appliance. Compare this fingerprint with the one the owner sees on the appliance."}</p>
        <div class="fp" id="fp">${esc(data.fingerprint)}</div>
        <div class="row" style="margin-top:14px"><button class="${changed ? "danger" : "primary"}" id="ok">They match</button><button id="no">Cancel</button></div>`);
      $("#ok").onclick = async () => { await Magnus.trust(data.url, data.fingerprint); ask("method", data); };
      $("#no").onclick = () => ask("address", data);
    } else if (stage === "method") {
      paint(box, `<p class="small muted">${esc(data.url || location.origin)}</p><p class="small muted">Connecting…</p>`);
      try {
        const m = await Magnus.signInMethod();
        ask(m.method === "oidc" ? "provider" : "password", { ...data, label: m.label });
      } catch (err) { note = err.message || String(err); ask(Magnus.kind === "app" ? "address" : "password", data); }
    } else if (stage === "provider") {
      paint(box, `<p class="small muted">${esc(data.url || location.origin)}</p>
        ${data.error ? `<p class="small" style="color:var(--bad)" role="alert">${esc(data.error)}</p>` : ""}
        <p>You sign in to this appliance with your <strong>${esc(data.label)}</strong> account.</p>
        <div class="row" style="margin-top:16px"><button class="primary" id="go">Sign in with ${esc(data.label)}</button><button type="button" class="link" id="pwd">Use a password instead</button>${Magnus.kind === "app" ? `<button type="button" class="link" id="back">Other appliance</button>` : ""}</div>`);
      $("#back")?.addEventListener("click", () => ask("address", data));
      $("#pwd").onclick = () => ask("password", data);
      $("#go").onclick = async () => {
        $("#go").disabled = true;
        let s;
        try { s = await Magnus.deviceStart(); } catch (err) { return ask("provider", { ...data, error: err.message || String(err) }); }
        const link = s.verification_uri_complete || s.verification_uri;
        let stop = false;
        paint(box, `<p class="small muted">${esc(data.url || location.origin)}</p>
          <p>Approve the sign-in on the ${esc(s.label)} page that just opened.</p>
          <p class="small">Check that it shows this code:</p><div class="fp" id="code">${esc(s.user_code)}</div>
          <div class="row" style="margin-top:14px"><button data-url="${esc(link)}" id="open">Open the sign-in page again</button><button class="link" id="cancel">Cancel</button></div>
          <p class="small muted" id="wait" role="status">Waiting for you to approve…</p>`);
        $("#open").onclick = () => Magnus.openExternal(link).catch(() => {});
        $("#cancel").onclick = () => { stop = true; ask("provider", data); };
        Magnus.openExternal(link).catch(() => {});
        const until = Date.now() + s.expires_in * 1000;
        const every = Math.max(1, s.interval) * 1000;
        while (!stop && Date.now() < until) {
          await new Promise(r => setTimeout(r, every));
          if (stop) return;
          try {
            const r = await Magnus.devicePoll(s.flow);
            if (r.pending) continue;
            App.me = r.person; App.appliance = data.url || location.origin;
            return boot();
          } catch (err) { return ask("provider", { ...data, error: err.message || String(err) }); }
        }
        if (!stop) ask("provider", { ...data, error: "The sign-in code expired. Start again." });
      };
    } else {
      paint(box, `<form id="f"><p class="small muted">${esc(data.url || location.origin)}</p>
        ${data.error ? `<p class="small" style="color:var(--bad)" role="alert">${esc(data.error)}</p>` : ""}
        <label for="who">Your name on the appliance</label><input id="who" style="width:100%" autocomplete="username" value="${esc(data.person || localStorage.getItem("magnus.person") || "")}" required>
        <label for="pw">Password</label><input id="pw" type="password" style="width:100%" autocomplete="current-password" required>
        <div class="row" style="margin-top:16px"><button class="primary" id="go">Sign in</button>${data.label ? `<button type="button" class="link" id="prov">Sign in with ${esc(data.label)}</button>` : ""}${Magnus.kind === "app" ? `<button type="button" class="link" id="back">Other appliance</button>` : ""}</div></form>`);
      $("#back")?.addEventListener("click", () => ask("address", data));
      $("#prov")?.addEventListener("click", () => ask("provider", { ...data, error: undefined }));
      $("#f").onsubmit = async e => {
        e.preventDefault();
        const person = $("#who").value.trim();
        $("#go").disabled = true;
        try {
          const me = await Magnus.signIn(person, $("#pw").value);
          localStorage.setItem("magnus.person", person);
          App.me = me; App.appliance = data.url || location.origin;
          boot();
        } catch (err) { ask("password", { ...data, person, error: err.message || String(err) }); }
      };
    }
  };
  if (Magnus.kind === "web") ask("method", { url: location.origin });
  else ask("address");
}

// ---------- background: inbox, notifications, versions ----------

function seenKey(k) { return `magnus.${k}.${App.appliance}.${App.me?.id}`; }
function load(k, d) { try { return JSON.parse(localStorage.getItem(seenKey(k))) ?? d; } catch { return d; } }
function save(k, v) { try { localStorage.setItem(seenKey(k), JSON.stringify(v)); } catch { /* private window */ } }

async function pollInbox() {
  try {
    const { items } = await api("GET", "/v1/inbox");
    App.inbox = items;
    // Interrupt only for decisions a task is waiting on; the rest wait in
    // the inbox until the person looks (U3).
    const told = new Set(load("told", []));
    const fresh = items.filter(i => i.status === "open" && i.can_respond && i.blocking && !told.has(i.id));
    for (const i of fresh) Magnus.notify("A task needs you", i.summary);
    if (fresh.length) save("told", [...told, ...fresh.map(i => i.id)].slice(-500));
    paint($("nav.side"), nav());
  } catch (e) { if (e.status === 401 && App.me) { stop(); start(); } }
}

async function pollVersions() {
  try {
    const v = await api("GET", "/v1/versions");
    App.versions = v;
    const seen = load("version", null);
    if (seen === null) { save("version", v.latest_seq ?? 0); return; }
    const news = v.changes.filter(c => c.seq > seen);
    const told = load("version-told", 0);
    if (news.length && (v.latest_seq ?? 0) > told) {
      Magnus.notify("The agent changed", news.map(describeChange).join("; "));
      save("version-told", v.latest_seq);
    }
  } catch { /* shown on Home */ }
}

function describeChange(c) {
  const what = { harness: "harness", instructions: "instructions", model: "model", playbook: "playbook" }[c.what] || c.what.replace("policy:", "policy for ");
  return `${what} is now ${c.value}`;
}

// ---------- Home ----------

route(/^#\/home$/, async main => {
  const draw = async () => {
    const [health, { tasks }, model] = await Promise.all([api("GET", "/v1/health"), api("GET", "/v1/tasks"), api("GET", "/v1/model")]);
    const v = App.versions;
    const seen = load("version", 0);
    const news = v ? v.changes.filter(c => c.seq > seen) : [];
    const open = App.inbox.filter(i => i.status === "open" && i.can_respond);
    const blocking = open.filter(i => i.blocking);
    tasks.sort((a, b) => (b.created_at || "").localeCompare(a.created_at || ""));
    const active = tasks.filter(t => t.status === "running" || t.status === "suspended");
    const recent = tasks.filter(t => !(t.status === "running" || t.status === "suspended")).slice(0, 8);
    const row = t => `<tr class="click" data-href="#/task/${enc(t.task_id)}"><td>${esc(t.title)}</td><td>${chip(t.status)}</td><td class="small muted">${esc(WAITING[t.waiting_on] || "")}</td><td class="small faint n">${esc(when(t.created_at))}</td></tr>`;
    paint(main, `
      ${news.length ? `<div class="banner" role="status"><strong>The agent changed since you last looked.</strong>
        <ul class="small" style="margin:6px 0">${news.map(c => `<li>${esc(describeChange(c))} <span class="faint">(${esc(when(c.at))})</span></li>`).join("")}</ul>
        <button data-act="seen-versions">Got it</button></div>` : ""}
      ${health.kill_switch ? `<div class="banner bad"><strong>The kill switch is on.</strong> No task acts until the owner turns it off.</div>` : ""}
      <h1>Home</h1>
      <p class="muted">New tasks run on <span class="mono">${esc(model.adapter)} · ${esc(model.model)}</span>.</p>
      <div class="grid" style="margin-top:18px">
        <a class="card" href="#/inbox"><div class="small muted">Needs you now</div><div class="num" style="font-size:28px;font-weight:650">${blocking.length}</div></a>
        <a class="card" href="#/inbox"><div class="small muted">Waiting in the inbox</div><div class="num" style="font-size:28px;font-weight:650">${open.length - blocking.length}</div></a>
        <a class="card" href="#/tasks"><div class="small muted">Running</div><div class="num" style="font-size:28px;font-weight:650">${active.length}</div></a>
      </div>
      <h2>Running</h2>
      ${active.length ? `<table class="t"><tbody>${active.map(row).join("")}</tbody></table>` : `<p class="empty">Nothing is running. <a href="#/new">Start a task</a>.</p>`}
      <h2>Recent</h2>
      ${recent.length ? `<table class="t"><tbody>${recent.map(row).join("")}</tbody></table>` : `<p class="empty">No finished tasks yet.</p>`}`);
  };
  main.onclick = e => {
    const r = e.target.closest("[data-href]"); if (r) location.hash = r.dataset.href;
    if (e.target.closest("[data-act=seen-versions]")) { save("version", App.versions?.latest_seq ?? 0); main._html = null; draw(); }
  };
  await draw();
  every(4000, draw);
});

route(/^#\/tasks$/, async main => {
  const draw = async () => {
    const { tasks } = await api("GET", "/v1/tasks");
    tasks.sort((a, b) => (b.created_at || "").localeCompare(a.created_at || ""));
    paint(main, `<h1>Tasks</h1>${tasks.length ? `<table class="t"><thead><tr><th>Task</th><th>Status</th><th></th><th class="n">Started</th></tr></thead><tbody>${tasks.map(t => `<tr class="click" data-href="#/task/${enc(t.task_id)}"><td>${esc(t.title)}</td><td>${chip(t.status)}</td><td class="small muted">${esc(WAITING[t.waiting_on] || t.summary || "")}</td><td class="small faint n">${esc(when(t.created_at))}</td></tr>`).join("")}</tbody></table>` : `<p class="empty">No tasks yet.</p>`}`);
  };
  main.onclick = e => { const r = e.target.closest("[data-href]"); if (r) location.hash = r.dataset.href; };
  await draw();
  every(5000, draw);
});

// ---------- New task ----------

route(/^#\/new$/, async main => {
  paint(main, `<h1>New task</h1><p class="muted">Say what you want done, the way you would to a colleague.</p>
    <form id="f"><label for="text" class="sr">Task</label><textarea id="text" rows="6" data-draft="new-task" placeholder="For example: Make a one-page summary of last month's sales from the attached sheet." required></textarea>
    <div class="row" style="margin-top:12px"><button class="primary" id="go">Start</button></div></form>`);
  $("#text").focus();
  $("#f").onsubmit = async e => {
    e.preventDefault();
    const text = $("#text").value.trim();
    if (!text) return;
    $("#go").disabled = true;
    try {
      const r = await api("POST", "/v1/tasks", { text });
      delete drafts["new-task"];
      location.hash = `#/task/${enc(r.task_id)}`;
    } catch (err) { toast(err.message); $("#go").disabled = false; }
  };
});

// ---------- Inbox ----------

const KIND = {
  approval: "Approval", question: "Question", review: "Review", notice: "Stopped", message: "Note from the agent",
  policy_proposal: "Policy proposal", schedule_proposal: "Recurring task proposal", tool_change: "Changed tool", tool_review: "New app's tools",
};
const TIER = { read: "reads", sandbox: "workspace only", reversible_write: "can be undone", irreversible: "cannot be undone", financial: "money", prohibited: "prohibited" };
const AUDIENCE = { private: "only you", org: "people in your organization", public: "anyone" };

function fieldValue(v) {
  if (v === null || v === undefined) return `<span class="faint">none</span>`;
  if (typeof v === "string") return `<pre>${esc(v)}</pre>`;
  if (typeof v === "number" || typeof v === "boolean") return `<pre>${esc(String(v))}</pre>`;
  return `<pre>${esc(JSON.stringify(v, null, 2))}</pre>`;
}

// Exactly what will happen: every field of the call, nothing shortened (U6).
function whatHappens(d) {
  const input = d.input && typeof d.input === "object" && !Array.isArray(d.input) ? d.input : { input: d.input };
  return `<div class="what" aria-label="What will happen">
    <div class="field"><span>Action</span><span class="mono">${esc(d.tool)}</span></div>
    ${d.app ? `<div class="field"><span>App</span><span>${esc(d.app)}</span></div>` : ""}
    ${d.audience ? `<div class="field"><span>Who can see it</span><span>${esc(AUDIENCE[d.audience] || d.audience)}</span></div>` : ""}
    ${Object.entries(input).map(([k, v]) => `<div class="field"><span>${esc(k)}</span>${fieldValue(v)}</div>`).join("")}
  </div>`;
}

function taintNote(t) {
  if (!t || (!t.untrusted && !t.private)) return "";
  const parts = [];
  if (t.untrusted) parts.push("has read content from outside sources, which could be steering it");
  if (t.private) parts.push("has read private data");
  return `<p class="small muted">This task ${parts.join(" and ")}.</p>`;
}

function buttons(i, list) {
  if (!i.can_respond) return `<p class="small faint">Waiting for someone with the right role to decide.</p>`;
  return `<div class="row" style="margin-top:12px">${list.map(([d, label, cls, extra]) => `<button class="${cls || ""}" data-item="${esc(i.id)}" data-decision="${d}" ${extra || ""}>${esc(label)}</button>`).join("")}</div>`;
}

function card(i, composed) {
  const d = i.details || {};
  const tier = i.kind === "approval" ? (i.tier || d.risk) : "";
  const head = `<div class="spread"><span class="kind">${esc(KIND[i.kind] || i.kind)}${tier ? ` · ${esc(TIER[tier] || tier)}` : ""}</span>
    <span class="small faint">${i.blocking ? `<strong style="color:var(--fg)">A task is waiting</strong> · ` : ""}${esc(when(i.created_at))}${i.task_id ? ` · <a href="#/task/${enc(i.task_id)}">task</a>` : ""}</span></div>`;
  let body = "";
  if (i.status !== "open") {
    const r = i.resolution || {};
    return `<div class="card flat item ${esc(i.kind)} ${esc(tier)}">${head}<h4>${esc(i.summary)}</h4><p class="small muted">${esc(r.decision)}${r.scope ? ` (${esc(r.scope)})` : ""} by ${esc(String(r.by || "").replace(/^person:/, ""))}${r.text ? `: ${esc(r.text)}` : ""}</p></div>`;
  }
  switch (i.kind) {
    case "approval": {
      const onceOnly = String(d.rule || "").startsWith("kernel:") || d.reason === "unknown_outcome";
      const list = [["approve:once", "Approve once", "primary"]];
      if (!onceOnly) {
        list.push(["approve:task", "Approve for this task"]);
        list.push(["approve:window", "Approve for an hour"]);
        if (isOwner()) list.push(["approve:standing", "Always allow"]);
      }
      list.push(["reject", "Deny"]);
      body = `${i.consequence ? `<div class="consequence" role="note">${esc(i.consequence)}</div>` : ""}
        ${whatHappens(d)}
        ${d.why ? `<p class="small"><span class="muted">Why you are asked:</span> ${esc(d.why)}${d.rule && !String(d.rule).startsWith("kernel:") ? ` <span class="faint mono">(${esc(d.rule)})</span>` : ""}</p>` : ""}
        ${taintNote(d.taint)}${buttons(i, list)}`;
      break;
    }
    case "question": {
      const opts = Array.isArray(d.options) ? d.options : [];
      body = `${taintNote(d.taint)}${i.can_respond ? `${opts.length ? `<div class="row">${opts.map(o => `<button data-item="${esc(i.id)}" data-decision="answer" data-text="${esc(o)}">${esc(o)}</button>`).join("")}</div>` : ""}
        <textarea rows="2" data-draft="ans-${esc(i.id)}" placeholder="Your answer" style="margin-top:10px"></textarea>
        <div class="row" style="margin-top:8px"><button class="primary" data-item="${esc(i.id)}" data-decision="answer">Send answer</button></div>` : buttons(i, [])}`;
      break;
    }
    case "review": {
      const crit = (d.criteria || []).map(c => `<li>${esc(c.description || c.id || c)}</li>`).join("");
      body = `<p>${esc(d.summary)}</p>
        <div class="evidence none"><div class="small muted">These have no check the appliance could run. Check them yourself:</div><ul>${crit}</ul></div>
        ${d.view ? `<iframe class="composed" sandbox="allow-same-origin" title="The outcome" data-view="${esc(i.id)}"></iframe>` : ""}
        ${taintNote(d.taint)}
        ${i.can_respond ? `<textarea rows="2" data-draft="rev-${esc(i.id)}" placeholder="If you send it back, say what is wrong" style="margin-top:10px"></textarea>` : ""}
        ${buttons(i, [["accept", "Accept", "primary"], ["reject", "Send back"]])}`;
      if (d.view) composed.push([i.id, d.view, i.run_id, i.task_id]);
      break;
    }
    case "notice": {
      const ev = d.evidence;
      const list = [];
      if (typeof d.reason === "string") list.push(["resume", "Continue", "primary"]);
      if (d.reason === "budget") list.push(["extend", "Double the budget and continue"]);
      list.push(["dismiss", "Dismiss"]);
      body = `<p class="small"><span class="muted">Why it stopped:</span> ${esc(d.reason)}</p>
        ${ev ? `<div class="evidence fail"><div class="small muted">Evidence</div>${fieldValue(ev)}</div>` : ""}${buttons(i, list)}`;
      break;
    }
    case "policy_proposal":
      body = `<p class="small"><span class="muted">Layer:</span> <span class="mono">${esc(d.layer)}</span></p>
        ${d.rationale ? `<p>${esc(d.rationale)}</p>` : ""}<div class="what"><pre>${esc(d.cedar)}</pre></div>${taintNote(d.taint)}
        ${buttons(i, [["accept", "Add this rule", "primary"], ["reject", "Reject"]])}`;
      break;
    case "schedule_proposal":
      body = `<div class="what"><div class="field"><span>Task</span><span>${esc(d.text)}</span></div>
        <div class="field"><span>When</span><span>${d.every_minutes ? `every ${esc(d.every_minutes)} minutes` : `daily at ${esc(d.daily_at)}`}</span></div></div>${taintNote(d.taint)}
        ${buttons(i, [["accept", "Schedule it", "primary"], ["reject", "Reject"]])}`;
      break;
    case "message":
      body = `${taintNote(d.taint)}${buttons(i, [["dismiss", "Dismiss"]])}`;
      break;
    case "tool_change":
      body = `<div class="what"><div class="field"><span>Tool</span><span class="mono">${esc(d.tool)}</span></div><div class="field"><span>New description</span>${fieldValue(d.description)}</div><div class="field"><span>Inputs</span>${fieldValue(d.input_schema)}</div></div>
        ${buttons(i, [["approve", "Accept the change", "primary"], ["reject", "Keep it blocked"]])}`;
      break;
    case "tool_review":
      body = `<div class="what">${(d.tools || []).map(t => `<div class="field"><span class="mono">${esc(t.tool)}</span>${fieldValue(t.description)}</div>`).join("")}</div>
        ${buttons(i, [["approve", "Let the agent read these", "primary"], ["reject", "Reject"]])}`;
      break;
    default:
      body = `<div class="what">${fieldValue(d)}</div>`;
  }
  return `<article class="card item ${esc(i.kind)} ${esc(tier)}" data-id="${esc(i.id)}">${head}<h4>${esc(i.summary)}</h4>${body}</article>`;
}

// Draw composed outcomes into their frames, from trusted code only.
const fontCss = () => {
  const u = f => new URL(`fonts/${f}`, location.href).href;
  return `@font-face{font-family:Inter;font-weight:100 900;src:url(${u("inter-latin-wght-normal.woff2")}) format("woff2")}`;
};
async function images(view, taskId) {
  const out = {};
  const files = (view || []).filter(c => c.type === "image").map(c => c.file);
  if (!files.length || !taskId) return out;
  const t = await api("GET", `/v1/tasks/${enc(taskId)}`);
  for (const f of files) {
    const o = (t.outputs || []).find(x => x.name === f);
    if (o && /^image\/(png|jpeg|gif|webp)$/.test(o.media_type)) {
      try { out[f] = await Magnus.dataUrl(`/v1/tasks/${enc(taskId)}/outputs/${enc(o.id)}`, o.media_type); } catch { /* shown as missing */ }
    }
  }
  return out;
}
async function fill(frame, view, taskId) {
  if (!frame || frame._drawn) return;
  frame._drawn = true;
  frame.srcdoc = Compose.documentFor(view, await images(view, taskId), fontCss());
  frame.onload = () => { try { frame.style.height = frame.contentDocument.documentElement.scrollHeight + 4 + "px"; } catch { /* sized by CSS */ } };
}

async function respond(btn, root) {
  const id = btn.dataset.item;
  let [decision, scope] = btn.dataset.decision.split(":");
  const body = { decision };
  if (scope) body.scope = scope;
  if (scope === "window") body.minutes = 60;
  const box = $(`[data-draft="ans-${CSS.escape(id)}"]`, root) || $(`[data-draft="rev-${CSS.escape(id)}"]`, root);
  if (btn.dataset.text) body.text = btn.dataset.text;
  else if (box && box.value.trim()) body.text = box.value.trim();
  if (decision === "answer" && !body.text) { toast("Write an answer first."); return; }
  btn.disabled = true;
  try {
    await api("POST", `/v1/inbox/${enc(id)}`, body);
    delete drafts[`ans-${id}`]; delete drafts[`rev-${id}`];
    toast({ approve: "Approved.", reject: decision === "reject" ? "Done." : "Done.", accept: "Accepted.", answer: "Sent.", dismiss: "Dismissed.", resume: "Continuing.", extend: "Budget doubled." }[decision] || "Done.");
    await pollInbox();
  } catch (e) { toast(e.message); btn.disabled = false; }
}

route(/^#\/inbox$/, async main => {
  const draw = async () => {
    await pollInbox();
    const mine = App.inbox;
    const open = mine.filter(i => i.status === "open");
    const now = open.filter(i => i.blocking), later = open.filter(i => !i.blocking);
    const done = mine.filter(i => i.status !== "open").sort((a, b) => (b.created_at || "").localeCompare(a.created_at || "")).slice(0, 15);
    const composed = [];
    const html = `<h1>Inbox</h1><p class="muted">Every decision the agent needs from people is here, and only here.</p>
      <h2>Needs you now ${now.length ? `<span class="badge">${now.length}</span>` : ""}</h2>
      ${now.length ? now.map(i => card(i, composed)).join("") : `<p class="empty">No task is waiting on you.</p>`}
      <h2>When you have a moment</h2>
      ${later.length ? later.map(i => card(i, composed)).join("") : `<p class="empty">Nothing else.</p>`}
      ${done.length ? `<h2>Decided</h2>${done.map(i => card(i, [])).join("")}` : ""}`;
    paint(main, html);
    for (const [id, view, , task] of composed) fill($(`iframe[data-view="${CSS.escape(id)}"]`, main), view, task);
  };
  main.onclick = e => { const b = e.target.closest("button[data-item]"); if (b) respond(b, main).then(draw); };
  await draw();
  every(4000, draw);
});

// ---------- Task detail ----------

function criteria(t) {
  if (!t.criteria?.length) return `<p class="empty">No contract yet.</p>`;
  return t.criteria.map(c => {
    const cls = c.verified === true ? "pass" : c.verified === false ? "fail" : "none";
    const person = String(c.verified_by || "").startsWith("person:") ? String(c.verified_by).slice(7) : null;
    const state = c.verified === true ? (person ? `<span class="chip ok">confirmed by ${esc(person)}</span>` : `<span class="chip ok">holds</span>`)
      : c.verified === false ? `<span class="chip bad">does not hold</span>` : c.check ? `<span class="chip">not checked yet</span>` : `<span class="chip warn">no check: needs a person</span>`;
    const by = c.verified_by && !person ? `<span class="small faint"> · checked by the appliance</span>` : "";
    return `<div class="evidence ${cls}"><div class="spread"><span>${esc(c.description)}</span><span>${state}${by}</span></div>
      ${c.check ? `<div class="small muted" style="margin-top:4px">Check: <span class="mono">${esc(c.check.tool)}</span> ${esc(JSON.stringify(c.check.input || {}))}</div>` : ""}
      ${c.evidence !== undefined && c.evidence !== null ? `<details><summary class="small">Evidence</summary>${fieldValue(c.evidence)}</details>` : ""}</div>`;
  }).join("");
}

function glance(t) {
  const cs = t.criteria || [];
  const byPerson = c => String(c.verified_by || "").startsWith("person:");
  const world = cs.filter(c => c.verified === true && !byPerson(c)).length;
  const people = cs.filter(c => c.verified === true && byPerson(c)).length;
  const failed = cs.filter(c => c.verified === false).length;
  const open = cs.length - world - people - failed;
  const parts = [];
  if (world) parts.push(`${world} of ${cs.length} criteria checked against the world`);
  if (people) parts.push(`${people} confirmed by a person`);
  if (failed) parts.push(`${failed} do not hold`);
  if (open) parts.push(`${open} not yet checked`);
  return parts.join(", ");
}

function plan(t) {
  const units = t.plan?.units || [];
  if (!units.length) return `<p class="empty">The agent has not written a plan.</p>`;
  const mark = { done: "ok", active: "run", dropped: "", pending: "" };
  return `<p class="small faint">Version ${esc(t.plan_version)}. Statuses are the agent's own account; the contract's checks are the evidence.</p>
    <table class="t"><tbody>${units.map(u => `<tr><td class="mono small">${esc(u.id)}</td><td>${esc(u.goal)}<div class="small muted">Done when ${esc(u.done_when)}</div></td><td><span class="chip ${mark[u.status] || ""}">${esc(u.status || "pending")}</span></td></tr>`).join("")}</tbody></table>`;
}

function trace(tl) {
  const steps = tl.steps.map(s => `<div class="step"><div class="small faint">Step ${esc(s.step)}</div>
    ${s.text ? `<p>${esc(s.text)}</p>` : ""}
    ${s.calls.map(c => `<div class="call"><div><span class="mono">${esc(c.tool)}</span> <span class="small faint">issued by ${esc(c.issuer)}</span>
      ${(c.decisions || []).map(d => `<span class="chip ${d.outcome === "allow" || d.outcome === "dedupe" ? "ok" : d.outcome === "ask" ? "warn" : "bad"}">${esc(d.stage)}: ${esc(d.outcome)}</span>`).join(" ")}</div>
      <details><summary class="small">Input${c.result !== null && c.result !== undefined ? " and result" : ""}</summary>${fieldValue(c.input)}${c.result !== null && c.result !== undefined ? `<div class="small muted" style="margin-top:6px">Result${c.is_error ? " (error)" : ""}</div><pre>${esc(c.result)}</pre>` : ""}</details></div>`).join("")}
  </div>`).join("");
  const notices = (tl.notices || []).map(n => `<div class="evidence none small"><span class="muted">Kernel, step ${esc(n.step)}:</span> ${esc(n.text)}</div>`).join("");
  return (steps || `<p class="empty">No steps yet.</p>`) + (notices ? `<h3>Kernel notices</h3>${notices}` : "");
}

route(/^#\/task\/([^/]+)$/, async (main, id) => {
  let layer = "outcome", liveText = "", stopLive = null;
  const draw = async () => {
    const [t, tl] = await Promise.all([api("GET", `/v1/tasks/${enc(id)}`), api("GET", `/v1/tasks/${enc(id)}/timeline`)]);
    const running = t.status === "running";
    if (running && !stopLive) stopLive = Magnus.live(id, txt => { liveText += txt; const el = $("#live", main); if (el) el.textContent = liveText; });
    if (!running && stopLive) { stopLive(); stopLive = null; liveText = ""; }
    const open = App.inbox.filter(i => i.task_id === id && i.status === "open");
    const outcome = `${t.summary && t.status !== "running" ? `<p style="font-size:16px">${esc(t.summary)}</p>` : ""}
      ${t.outcome_view ? `<iframe class="composed" sandbox="allow-same-origin" title="The outcome" id="view"></iframe>` : ""}
      ${t.spoken ? `<div class="row" style="margin:10px 0"><button data-act="speak">Read aloud</button><button data-act="hush">Stop</button></div>` : ""}
      ${t.outputs?.length ? `<h3>Files</h3>${t.outputs.map(o => `<div class="row"><button class="link" data-act="download" data-out="${esc(o.id)}" data-name="${esc(o.name)}">${esc(o.name)}</button><span class="small faint">${esc(o.media_type)} · ${Math.max(1, Math.round(o.size / 1024))} KB</span></div>`).join("")}` : ""}
      <h3>Evidence</h3>${criteria(t)}`;
    const html = `<p class="small"><a href="#/tasks">Tasks</a></p>
      <div class="spread"><h1 style="max-width:44ch">${esc(t.text)}</h1><div class="row">${chip(t.status)}</div></div>
      <p class="muted">${esc(WAITING[t.waiting_on] || "")}${t.waiting_on && glance(t) ? " · " : ""}${esc(glance(t))}</p>
      ${open.length ? `<div class="banner">${open.length === 1 ? "A decision for this task is" : `${open.length} decisions for this task are`} in your <a href="#/inbox">inbox</a>.</div>` : ""}
      ${running ? `<div class="live" id="live" aria-live="polite">${esc(liveText) || `<span class="faint">Working…</span>`}</div>` : ""}
      <div class="layers" role="tablist">${[["outcome", "Outcome"], ["plan", "Contract and plan"], ["trace", "Full trace"]].map(([k, l]) => `<button role="tab" aria-selected="${layer === k}" class="${layer === k ? "on" : ""}" data-layer="${k}">${l}</button>`).join("")}</div>
      <section>${layer === "outcome" ? outcome : layer === "plan" ? `<h3>Task</h3><p>${esc(t.text)}</p><h3>Goal</h3><p>${esc(t.contract?.goal || "")}</p><h3>Plan</h3>${plan(t)}` : trace(tl)}</section>
      <hr><p class="small faint">${esc(t.model?.adapter)} · ${esc(t.model?.name)} · ${t.usage?.model_calls ?? 0} model calls · ${t.usage?.actions ?? 0} actions · $${(t.usage?.cost_usd ?? 0).toFixed(2)}</p>
      <div class="row">${!["completed", "failed", "cancelled"].includes(t.status) ? `<button data-act="cancel">Cancel task</button>` : ""}${t.status === "suspended" && !t.waiting_item ? `<button data-act="resume">Resume</button>` : ""}</div>`;
    if (paint(main, html) && t.outcome_view) fill($("#view", main), t.outcome_view, id);
    main._task = t;
  };
  main.onclick = async e => {
    const l = e.target.closest("[data-layer]");
    if (l) { layer = l.dataset.layer; return draw(); }
    const a = e.target.closest("[data-act]");
    if (!a) return;
    const t = main._task;
    try {
      if (a.dataset.act === "speak") await Magnus.speak(t.spoken);
      if (a.dataset.act === "hush") await Magnus.stopSpeaking();
      if (a.dataset.act === "download") { const where = await Magnus.download(`/v1/tasks/${enc(id)}/outputs/${enc(a.dataset.out)}`, a.dataset.name); toast(`Saved ${where}`); }
      if (a.dataset.act === "cancel" && confirm("Cancel this task? What it already did stays done.")) { await api("POST", `/v1/tasks/${enc(id)}/cancel`, {}); draw(); }
      if (a.dataset.act === "resume") { await api("POST", `/v1/tasks/${enc(id)}/resume`, {}); draw(); }
    } catch (err) { toast(err.message); }
  };
  onLeave(() => { if (stopLive) stopLive(); Magnus.stopSpeaking(); });
  await draw();
  every(2500, draw);
});

// ---------- Models ----------

const signins = {};
route(/^#\/models$/, async main => {
  if (!isOwner()) { paint(main, `<h1>Models</h1><p class="empty">Only the owner connects model providers.</p>`); return; }
  let pick = null;
  const catalogs = {};
  const catalog = async a => {
    if (!catalogs[a]) catalogs[a] = await api("GET", `/v1/adapters/${enc(a)}/catalog`).then(c => c.models ? { ids: c.models.map(m => m.id) } : { error: c.message || "the adapter sent no list" }).catch(e => ({ error: e.message }));
    return catalogs[a];
  };
  const draw = async () => {
    const [{ adapters }, cur] = await Promise.all([api("GET", "/v1/adapters"), api("GET", "/v1/model")]);
    const p = pick || cur.adapter, models = await catalog(p);
    const chosen = p === cur.adapter ? cur.model : null;
    paint(main, `<h1>Models</h1><p class="muted">Connect providers here. Keys and sign-ins go to the appliance's vault; this app keeps none.</p>
      <div class="card"><div class="small muted">New tasks run on</div><div class="mono" style="font-size:16px;margin:2px 0 10px">${esc(cur.adapter)} · ${esc(cur.model)}</div>
        <div class="row"><select id="madapter" aria-label="Provider">${adapters.map(a => `<option ${a.id === p ? "selected" : ""}>${esc(a.id)}</option>`).join("")}</select>
        ${models.ids ? `<select id="mname" aria-label="Model">${chosen && !models.ids.includes(chosen) ? `<option selected>${esc(chosen)}</option>` : ""}${models.ids.map(m => `<option ${m === chosen ? "selected" : ""}>${esc(m)}</option>`).join("")}</select><button class="primary" data-act="use">Use this model</button>` : ""}</div>
        ${models.error ? `<p class="small muted" style="margin-top:8px">No model list from ${esc(p)} yet: ${esc(models.error)}</p>` : ""}</div>
      ${adapters.map(a => `<div class="card" data-adapter="${esc(a.id)}"><div class="spread"><strong>${esc(a.id)}</strong>
          <span class="row">${a.conformance ? `<span class="chip ${a.conformance.passed ? "ok" : "bad"}">suite ${a.conformance.passed ? "passed" : "failed"} · ${esc(a.conformance.model)}</span>` : `<span class="chip warn">suite not run</span>`}</span></div>
        <div class="row" style="margin-top:10px"><button data-act="signin" data-adapter="${esc(a.id)}">Connect</button><button data-act="suite" data-adapter="${esc(a.id)}">Run the conformance suite</button></div>
        <div id="si-${esc(a.id)}">${signins[a.id] || ""}</div><div id="suite-${esc(a.id)}"></div></div>`).join("")}`);
    const ma = $("#madapter", main);
    if (ma) ma.onchange = () => { pick = ma.value; draw(); };
  };
  const show = (a, html) => { signins[a] = html; const el = $(`#si-${CSS.escape(a)}`, main); if (el) el.innerHTML = html; };
  const finish = async (a, step, input) => {
    try {
      await api("POST", `/v1/adapters/${enc(a)}/sign-in/complete`, { flow: step.flow, input });
      delete catalogs[a];
      show(a, `<p class="small" style="color:var(--ok)">Connected.</p>`);
      draw();
    } catch (e) { show(a, `<p class="small" style="color:var(--bad)">${esc(e.message)}</p>`); }
  };
  main.onclick = async e => {
    const b = e.target.closest("button[data-act]"); if (!b) return;
    const a = b.dataset.adapter;
    try {
      if (b.dataset.act === "use") { await api("PUT", "/v1/model", { adapter: $("#madapter").value, model: $("#mname").value }); toast("New tasks will use this model."); draw(); }
      if (b.dataset.act === "signin") {
        b.disabled = true;
        const step = await api("POST", `/v1/adapters/${enc(a)}/sign-in`, {});
        b.disabled = false;
        if (step.event === "error") { show(a, `<p class="small" style="color:var(--bad)">${esc(step.message)}</p>`); return; }
        const head = `<div style="margin-top:12px"><p class="small">${esc(step.instructions)}</p>`;
        if (step.flow?.kind === "api_key") {
          show(a, `${head}<label for="key-${esc(a)}">API key</label><input type="password" id="key-${esc(a)}" style="width:100%" autocomplete="off">
            <div class="row" style="margin-top:8px"><button class="primary" data-act="key" data-adapter="${esc(a)}">Save to the vault</button></div></div>`);
          signins[`${a}:step`] = step;
        } else {
          show(a, `${head}${step.user_code ? `<p>Code: <span class="mono" style="font-size:18px">${esc(step.user_code)}</span></p>` : ""}
            ${step.url ? `<div class="row"><button class="primary" data-act="open" data-url="${esc(step.url)}">Open the sign-in page</button></div>` : ""}
            <p class="small muted">Waiting for you to approve on the provider's site…</p></div>`);
          finish(a, step, {});
        }
      }
      if (b.dataset.act === "open") await Magnus.openExternal(b.dataset.url);
      if (b.dataset.act === "key") {
        const v = $(`#key-${CSS.escape(a)}`, main).value.trim();
        if (!v) return toast("Paste the key first.");
        b.disabled = true;
        await finish(a, signins[`${a}:step`], { api_key: v });
      }
      if (b.dataset.act === "suite") {
        const el = $(`#suite-${CSS.escape(a)}`, main);
        b.disabled = true; el.innerHTML = `<p class="small muted">Running the suite against the live provider…</p>`;
        const r = await api("POST", `/v1/adapters/${enc(a)}/conformance`, {});
        el.innerHTML = `<table class="t small" style="margin-top:8px"><tbody>${r.checks.map(c => `<tr><td>${c.passed ? `<span class="chip ok">pass</span>` : `<span class="chip bad">fail</span>`}</td><td>${esc(c.name)}${c.simulated ? " <span class='faint'>(simulated)</span>" : ""}</td><td class="muted">${esc(c.detail)}</td></tr>`).join("")}</tbody></table>`;
        b.disabled = false; main._html = null;
      }
    } catch (err) { toast(err.message); b.disabled = false; }
  };
  await draw();
});

// ---------- start ----------

let background = [];
async function boot() {
  shell();
  window.onhashchange = go;
  await Promise.all([pollInbox(), pollVersions()]);
  background = [setInterval(pollInbox, 5000), setInterval(pollVersions, 30000)];
  go();
}

function stop() {
  background.forEach(clearInterval); background = [];
  App.timers.forEach(clearInterval); App.timers = [];
  App.leave.forEach(f => { try { f(); } catch { /* ignore */ } }); App.leave = [];
  window.onhashchange = null;
  App.me = null; App.inbox = [];
}

async function start() {
  try {
    const r = await Magnus.restore();
    if (r && r.person) { App.me = r.person; App.appliance = r.appliance; return boot(); }
    signInScreen(r && r.note);
  } catch (e) { signInScreen(e.message || String(e)); }
}

document.addEventListener("DOMContentLoaded", start);
