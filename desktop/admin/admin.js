// The admin console's page (admin console spec, Web UI). The shell: the
// top bar (organisation, status, kill switch, the admin's menu), the five
// sections, and the patterns every screen uses (`UI`): lists that open a
// detail panel, a toast with Undo for what can be undone, a confirm sheet
// for what cannot, the step-up sheet, live data over server-sent events,
// and designed empty, loading and error states.
"use strict";

const $ = (s, el = document) => el.querySelector(s);
const $$ = (s, el = document) => [...el.querySelectorAll(s)];
const esc = s => String(s ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const Admin = { me: null, live: null, status: null };
const when = t => (t ? new Date(t).toLocaleString() : "–");
const pct = x => (typeof x === "number" ? `${Math.round(x * 100)}%` : "–");
const who = by => String(by || "").replace(/^admin:/, "an admin ").replace(/^person:/, "");
const org = () => document.title.split(" · ")[0];
// A person's status as the app's pills draw it.
const pill = st => `<span class="chip ${({ active: "ok", disabled: "bad", invited: "warn" })[st] || ""}">${esc(st)}</span>`;

// ---- The patterns every screen uses ----------------------------------

const UI = {
  loading(el, what) {
    el.innerHTML = `<div class="ad-state" aria-busy="true"><span class="ad-spin" aria-hidden="true"></span><span>Loading ${esc(what)}…</span></div>`;
  },
  empty(text, hint) {
    return `<div class="ad-state ad-empty"><p>${esc(text)}</p>${hint ? `<p class="small muted">${esc(hint)}</p>` : ""}</div>`;
  },
  // An error names what could not be done and why, with Try again.
  error(el, what, err, retry) {
    el.innerHTML = `<div class="ad-state ad-error" role="alert"><p><strong>${esc(what)}</strong></p><p class="small">${esc(err?.message || err)}</p>${retry ? `<button class="ad-retry">Try again</button>` : ""}</div>`;
    if (retry) $(".ad-retry", el).onclick = retry;
  },

  // A toast says what was done; with `undo`, it offers Undo for ten
  // seconds.
  toast(text, undo) {
    let host = $("#toasts");
    if (!host) { host = document.createElement("div"); host.id = "toasts"; host.setAttribute("role", "status"); host.setAttribute("aria-live", "polite"); document.body.append(host); }
    const t = document.createElement("div");
    t.className = "toast ad-toast";
    t.innerHTML = `<span>${esc(text)}</span>${undo ? `<button class="link ad-undo">Undo</button>` : ""}`;
    host.append(t);
    const gone = setTimeout(() => t.remove(), undo ? 10000 : 5000);
    if (undo) $(".ad-undo", t).onclick = async () => { clearTimeout(gone); t.remove(); try { await undo(); } catch (e) { UI.toast(`Could not undo: ${e.message}`); } };
  },

  // The confirm sheet, for what cannot be undone. `typed` asks for the
  // target's name typed before the action is offered.
  confirm({ title, body, verb = "Confirm", danger = false, typed = null }) {
    return new Promise(done => {
      const d = document.createElement("dialog");
      d.className = "ad-sheet";
      d.setAttribute("aria-labelledby", "sheet-title");
      d.innerHTML = `<form method="dialog">
        <h2 id="sheet-title">${esc(title)}</h2>
        ${body ? `<p>${esc(body)}</p>` : ""}
        ${typed ? `<label>Type <strong class="mono">${esc(typed)}</strong> to confirm <input name="typed" autocomplete="off" spellcheck="false"></label>` : ""}
        <div class="row ad-sheet-actions"><button value="no" formnovalidate>Cancel</button><button value="yes" class="${danger ? "danger" : "primary"}" ${typed ? "disabled" : ""}>${esc(verb)}</button></div>
      </form>`;
      document.body.append(d);
      const yes = $('button[value="yes"]', d);
      if (typed) $("input", d).oninput = e => { yes.disabled = e.target.value.trim() !== typed; };
      d.addEventListener("close", () => { done(d.returnValue === "yes"); d.remove(); });
      d.showModal();
    });
  },

  // The detail panel: on the right of its list, full screen at phone
  // width. It closes with its button or Escape, and goes back to `back`.
  panel(title, html, back) {
    UI.closePanel();
    const main = $("#main");
    const p = document.createElement("aside");
    p.id = "panel";
    p.className = "ad-panel";
    p.setAttribute("aria-label", title);
    p.innerHTML = `<div class="ad-panel-head"><h2>${esc(title)}</h2><button class="ad-close" aria-label="Close">×</button></div><div class="ad-panel-body">${html}</div>`;
    main.classList.add("with-panel");
    main.append(p);
    const close = () => { if (back) location.hash = back; else UI.closePanel(); };
    $(".ad-close", p).onclick = close;
    p.addEventListener("keydown", e => { if (e.key === "Escape") close(); });
    $(".ad-close", p).focus();
    return $(".ad-panel-body", p);
  },
  closePanel() {
    $("#panel")?.remove();
    $("#main").classList.remove("with-panel");
  },

  // The step-up sheet: the provider's sign-in in its own small window; the
  // action goes on once the console has the fresh password. Where windows
  // are blocked, the whole page goes and comes back.
  stepUp() {
    return new Promise(done => {
      const d = document.createElement("dialog");
      d.className = "ad-sheet";
      d.setAttribute("aria-labelledby", "step-title");
      d.innerHTML = `<form method="dialog"><h2 id="step-title">Confirm it's you</h2>
        <p>This needs your password again, typed in the last five minutes. It opens ${esc(org())}'s sign-in in a small window and comes back here.</p>
        <p class="small muted" id="step-wait" hidden>Waiting for you to sign in in the other window. Closed it by mistake? Choose Open it again.</p>
        <div class="row ad-sheet-actions"><button value="no" formnovalidate>Cancel</button><button value="go" class="primary" id="step-go">Continue</button></div></form>`;
      document.body.append(d);
      // The window is followed through the session, not its handle: the
      // console's opener policy cuts the link when it goes to the provider.
      let w = null, timer = null, ok = false;
      const finish = v => { ok = v; clearInterval(timer); d.close(); };
      $("#step-go", d).onclick = e => {
        e.preventDefault();
        w = window.open("/auth/step-up?then=/auth/done", "magnus-step-up", "popup,width=520,height=680");
        if (!w) { location.href = `/auth/step-up?then=${encodeURIComponent(location.pathname + location.hash)}`; return; }
        $("#step-wait", d).hidden = false;
        $("#step-go", d).textContent = "Open it again";
        clearInterval(timer);
        timer = setInterval(async () => {
          const r = await fetch("/api/v1/session", { credentials: "same-origin" }).catch(() => null);
          const v = r?.ok ? await r.json() : null;
          if (v?.stepped_up_until && new Date(v.stepped_up_until) > new Date()) { Admin.me = v; try { w.close(); } catch { /* it closes itself */ } finish(true); }
        }, 1000);
      };
      d.addEventListener("close", () => { clearInterval(timer); d.remove(); done(ok); });
      d.showModal();
    });
  },
};

// Every change carries the session's CSRF token. A step-up asked for opens
// the sheet, and the same request is sent once more after it.
async function api(method, path, body, again = true) {
  const r = await fetch(`/api/v1${path}`, {
    method, credentials: "same-origin",
    headers: { ...(body ? { "content-type": "application/json" } : {}), ...(Admin.me ? { "x-csrf-token": Admin.me.csrf } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  });
  if (r.status === 401) {
    const v = await r.json().catch(() => ({}));
    if (v.error === "step_up_required" && again) {
      if (!(await UI.stepUp())) throw new Error("Not confirmed: nothing was changed.");
      return api(method, path, body, false);
    }
    if (v.error !== "step_up_required") signedOut();
    throw new Error(v.message || "Sign in again.");
  }
  if (!r.ok) throw new Error((await r.json().catch(() => ({}))).message || r.statusText);
  return r.status === 204 ? null : r.json();
}

// An action from a button: its error as a toast, its success as `then`.
async function act(method, path, body, then) {
  try { const v = await api(method, path, body); await then?.(v); return v; } catch (e) { UI.toast(e.message); return undefined; }
}

function signedOut() {
  Admin.me = null;
  Admin.live?.close();
  Admin.live = null;
  document.body.classList.remove("signed-in");
  $("#nav").hidden = true;
  $("#kill").hidden = true;
  $("#kill-banner")?.remove();
  $("#me").innerHTML = "";
  $("#view")?.remove();
  UI.closePanel();
  $("#sign-in").hidden = false;
}

// ---- Status, live ------------------------------------------------------

function drawStatus(ok, text) {
  const el = $("#status");
  el.dataset.state = ok ? "ok" : "down";
  $(".small", el).textContent = text;
}

async function status() {
  if (Admin.live) return;
  try {
    const v = await (await fetch("/status", { credentials: "same-origin" })).json();
    drawStatus(v.ok, v.ok ? "Running" : "Not answering");
  } catch { drawStatus(false, "Not answering"); }
}

// The live stream: the status dot, the kill switch and whatever screen
// listens (Overview), as they change.
function live() {
  Admin.live?.close();
  const es = new EventSource("/api/v1/events", { withCredentials: true });
  Admin.live = es;
  es.addEventListener("status", e => {
    const v = JSON.parse(e.data);
    Admin.status = v;
    const ok = v.kernel_up && v.log_up;
    drawStatus(ok, ok ? "Running" : !v.kernel_up ? "Kernel not answering" : "Event log not answering");
    if (v.kill_switch) drawKill(v.kill_switch);
    document.dispatchEvent(new CustomEvent("ad-status", { detail: v }));
  });
  es.onerror = () => {
    if (es.readyState !== EventSource.CLOSED) return;
    // Refused (the session ended) or the stream's time is up: back
    // through the guard, or to the sign-in.
    Admin.live = null;
    setTimeout(async () => {
      const r = await fetch("/api/v1/session", { credentials: "same-origin" }).catch(() => null);
      if (r?.ok) live(); else signedOut();
    }, 2000);
  };
}

// ---- The kill switch, on every screen ---------------------------------
//
// "Stop all agents", pressed and held for a second; while it is on, a red
// banner says who engaged it and when, with Release (which needs a fresh
// password).

function drawKill(v) {
  const el = $("#kill");
  el.hidden = false;
  let banner = $("#kill-banner");
  if (v.engaged) {
    if (!banner) { banner = document.createElement("div"); banner.id = "kill-banner"; banner.setAttribute("role", "alert"); document.body.prepend(banner); }
    banner.innerHTML = `<strong>All agents are stopped.</strong> <span>Engaged by ${esc(who(v.by))}${v.at ? ` at ${esc(when(v.at))}` : ""}.</span> <button id="kill-release">Release</button>`;
    $("#kill-release").onclick = () => act("POST", "/kill-switch/release", null, async () => { UI.toast("Agents may run again."); drawKill(await api("GET", "/kill-switch")); });
    el.innerHTML = `<span class="small ad-kill-on">Stopped</span>`;
    return;
  }
  banner?.remove();
  if ($("#kill-hold", el)) return;
  el.innerHTML = `<button id="kill-hold" class="ad-kill-btn" aria-describedby="kill-how">Stop all agents</button><span id="kill-how" class="vh">Press and hold for one second</span>`;
  const b = $("#kill-hold");
  let t = null;
  const start = e => {
    if (t) return;
    e.preventDefault();
    b.classList.add("holding");
    t = setTimeout(async () => {
      t = null;
      b.classList.remove("holding");
      const v = await act("POST", "/kill-switch/engage");
      el.innerHTML = "";
      drawKill(v?.engaged !== undefined ? v : await api("GET", "/kill-switch"));
    }, 1000);
  };
  const stop = () => { clearTimeout(t); t = null; b.classList.remove("holding"); };
  b.addEventListener("pointerdown", start);
  for (const ev of ["pointerup", "pointerleave", "pointercancel"]) b.addEventListener(ev, stop);
  b.addEventListener("keydown", e => { if ((e.key === " " || e.key === "Enter") && !e.repeat) start(e); });
  b.addEventListener("keyup", e => { if (e.key === " " || e.key === "Enter") stop(); });
}

async function killSwitch() {
  try { const v = await api("GET", "/kill-switch"); if (v) drawKill(v); } catch { /* the live stream redraws it */ }
}

// ---- The admin's menu: theme and sign out -----------------------------

function menu() {
  const m = $("#me");
  m.innerHTML = `<button class="ad-me-btn" id="me-btn" aria-haspopup="true" aria-expanded="false">${esc(Admin.me.name)}</button>
    <div class="ad-menu card" id="me-menu" role="menu" hidden>
      <div class="small muted">Theme</div>
      <div class="ad-seg" role="group" aria-label="Light or dark">${Theme.MODES.map(([k, l]) => `<button role="menuitemradio" data-mode="${k}">${esc(l)}</button>`).join("")}</div>
      <div class="ad-seg" role="group" aria-label="Material">${Theme.MATERIALS.map(([k, l]) => `<button role="menuitemradio" data-material="${k}">${esc(l)}</button>`).join("")}</div>
      <hr><button class="link" role="menuitem" id="sign-out">Sign out</button>
    </div>`;
  const btn = $("#me-btn"), box = $("#me-menu");
  const mark = () => {
    const p = Theme.prefs();
    for (const b of $$("[data-mode]", box)) b.setAttribute("aria-checked", String(b.dataset.mode === p.mode));
    for (const b of $$("[data-material]", box)) b.setAttribute("aria-checked", String(b.dataset.material === p.material));
  };
  const open = v => { box.hidden = !v; btn.setAttribute("aria-expanded", String(v)); if (v) mark(); };
  btn.onclick = e => { e.stopPropagation(); open(box.hidden); };
  document.addEventListener("click", e => { if (!m.contains(e.target)) open(false); });
  document.addEventListener("keydown", e => { if (e.key === "Escape" && !box.hidden) { open(false); btn.focus(); } });
  for (const b of $$("[data-mode]", box)) b.onclick = () => { Theme.set({ mode: b.dataset.mode }); mark(); };
  for (const b of $$("[data-material]", box)) b.onclick = () => { Theme.set({ material: b.dataset.material }); mark(); };
  $("#sign-out").onclick = async () => { try { await api("DELETE", "/session"); } catch { /* signed out either way */ } signedOut(); };
}

// ---- Overview ----------------------------------------------------------

async function overviewView(view) {
  view.innerHTML = `<h1>Overview</h1><div id="ov"></div>`;
  const box = $("#ov", view);
  UI.loading(box, "the overview");
  let v;
  try { v = await api("GET", "/overview"); } catch (e) { return UI.error(box, "The overview could not be read.", e, () => overviewView(view)); }
  const strip = s => `<p class="small ad-strip"><span><span class="ad-dot ${s.healthy ? "ok" : "down"}"></span> ${s.healthy ? "Everything is answering" : "Something needs you"}</span>
      <span>Kill switch ${s.kill_switch?.engaged ?? s.kill_switch ? "<strong>on</strong>" : "off"}</span><span>Magnus ${esc(s.version || "")}</span></p>`;
  const cards = c => `
      <a class="card ad-card" href="#/operations/health"><div class="small muted">Tasks running</div><div class="ad-big">${esc(c.running_tasks ?? "–")}</div><div class="small muted">${esc(c.waiting_tasks ?? 0)} waiting</div></a>
      <a class="card ad-card" href="#/users"><div class="small muted">People signed in</div><div class="ad-big">${esc(c.signed_in ?? "–")}</div></a>
      <a class="card ad-card" href="#/knowledge/activity"><div class="small muted">Agent knowledge changes, last 24 hours</div><div class="ad-big">${esc(c.knowledge_changes_24h ?? "–")}</div><div class="small muted">Open Activity</div></a>`;
  box.innerHTML = `<div id="ov-strip">${strip(v.status)}</div>
    <div class="ad-cards" id="ov-cards">${cards(v.cards)}</div>
    <h2>Alerts</h2>${v.alerts.length ? `<ul class="ad-alerts">${v.alerts.map(a => `<li class="card flat"><span>${esc(a.what)}</span> <a href="${esc(a.fix)}">Fix</a></li>`).join("")}</ul>` : UI.empty("No alerts.", "Services, disk and certificates are fine.")}`;
  // Live: the strip and the cards as they change.
  const on = e => {
    if (!document.body.contains(box)) return document.removeEventListener("ad-status", on);
    const s = e.detail;
    $("#ov-cards", box).innerHTML = cards(s);
    $("#ov-strip", box).innerHTML = strip({ healthy: v.status.healthy && s.kernel_up && s.log_up, kill_switch: s.kill_switch, version: s.version });
  };
  document.addEventListener("ad-status", on);
}

// ---- Operations --------------------------------------------------------

async function operationsView(view) {
  view.innerHTML = `<h1>Operations</h1><h2>Kill switch</h2><div id="ops-kill"></div><h2 id="health">Health</h2><div id="ops-health"></div>`;
  UI.loading($("#ops-kill", view), "the kill switch");
  UI.loading($("#ops-health", view), "the appliance's health");
  let k, h;
  try { [k, h] = await Promise.all([api("GET", "/kill-switch"), api("GET", "/health")]); }
  catch (e) { return UI.error($("#ops-health", view), "Operations could not be read.", e, () => operationsView(view)); }
  $("#ops-kill", view).innerHTML = `<p>${k.engaged ? `<strong>On.</strong> Engaged by ${esc(who(k.by))}${k.at ? ` at ${esc(when(k.at))}` : ""}.` : `Off${k.by ? `. Last moved by ${esc(who(k.by))}${k.at ? ` at ${esc(when(k.at))}` : ""}` : ""}.`}</p>
    <dl class="kv small"><dt>Tasks halted</dt><dd>${esc(k.tasks_halted)}</dd><dt>Schedules held</dt><dd>${esc(k.schedules_held)}</dd><dt>Sign-ins revoked</dt><dd>${esc(k.sign_ins_revoked)}</dd></dl>`;
  $("#ops-health", view).innerHTML = `<ul class="ad-services">${h.services.map(s => `<li><span class="ad-dot ${s.up ? "ok" : "down"}"></span> ${esc(s.name)} <span class="small muted">${s.up ? "answering" : "not answering"}</span></li>`).join("")}</ul>
    <dl class="kv small"><dt>Load</dt><dd>${esc(h.host.load ?? "–")} on ${esc(h.host.cpus)} processors</dd><dt>Memory used</dt><dd>${pct(h.host.memory_used)}</dd><dt>Disk used</dt><dd>${pct(h.host.disk_used)}</dd><dt>Tasks running</dt><dd>${esc(h.running_tasks ?? "–")}</dd></dl>
    <h3>Recent errors</h3>${(h.recent_errors || []).length ? `<ul class="small">${h.recent_errors.map(e => `<li>${esc(e.reason || "failed")} <span class="muted">· ${esc(e.person)} · ${esc(when(e.at))}</span></li>`).join("")}</ul>` : UI.empty("No recent errors.")}`;
}

// ---- Security › Audit --------------------------------------------------
//
// The full log as a table, filtered by actor, action, target and time, with
// CSV export; a row opens the entry with its before and after.

async function auditView(view, seq) {
  if (!$("#audit-filter", view)) {
    view.innerHTML = `<form class="row" id="audit-filter" role="search">
        <label>Who <input name="actor" autocomplete="off"></label>
        <label>Action <input name="action" autocomplete="off"></label>
        <label>On <input name="target" autocomplete="off"></label>
        <label>From <input name="since" type="datetime-local"></label>
        <label>To <input name="until" type="datetime-local"></label>
        <button class="primary">Filter</button>
        <a class="button" id="audit-csv" href="/api/v1/audit.csv" download>Export CSV</a>
      </form>
      <div id="audit-rows"></div>`;
    $("#audit-filter", view).onsubmit = e => { e.preventDefault(); drawAudit(view); };
    await drawAudit(view);
  }
  const e = (view.audit || []).find(x => String(x.seq) === seq);
  if (!e) return UI.closePanel();
  UI.panel(e.action, `<dl class="kv small"><dt>When</dt><dd>${esc(when(e.at))}</dd><dt>Who</dt><dd>${esc(e.name || e.admin)} <span class="muted mono">${esc(e.admin)}</span></dd>
    <dt>On</dt><dd>${esc(e.target || "–")}</dd><dt>From</dt><dd class="mono">${esc(e.source || "–")}</dd></dl>
    ${e.before != null ? `<h3>Before</h3><pre class="ad-json">${esc(JSON.stringify(e.before, null, 2))}</pre>` : ""}
    ${e.after != null ? `<h3>After</h3><pre class="ad-json">${esc(JSON.stringify(e.after, null, 2))}</pre>` : ""}`, "#/security/audit");
}

async function drawAudit(view) {
  const form = $("#audit-filter", view), box = $("#audit-rows", view);
  const q = new URLSearchParams();
  for (const [k, v] of new FormData(form)) if (v) q.set(k, k === "since" || k === "until" ? new Date(v).toISOString() : v);
  const qs = q.toString();
  $("#audit-csv", view).href = `/api/v1/audit.csv${qs ? `?${qs}` : ""}`;
  UI.loading(box, "the audit log");
  try {
    const v = await api("GET", `/audit${qs ? `?${qs}` : ""}`);
    view.audit = v.entries;
    box.innerHTML = v.entries.length
      ? `<div class="ad-scroll"><table class="t small"><thead><tr><th>When</th><th>Who</th><th>What</th><th>On</th><th>From</th></tr></thead><tbody>${v.entries.map(e => `<tr class="click" data-href="#/security/audit/${esc(e.seq)}" tabindex="0">
          <td>${esc(when(e.at))}</td><td>${esc(e.name || e.admin)}</td><td>${esc(e.action)}</td><td>${esc(e.target || "")}</td><td class="mono">${esc(e.source || "")}</td></tr>`).join("")}</tbody></table></div>`
      : UI.empty("Nothing in the audit log matches.", qs ? "Clear a filter to see more." : null);
    rowLinks(box);
  } catch (e) { UI.error(box, "The audit log could not be read.", e, () => drawAudit(view)); }
}

// Rows that open their detail: by click, or Enter.
function rowLinks(el) {
  for (const r of $$("tr[data-href]", el)) {
    r.onclick = e => { if (!e.target.closest("button, a, input")) location.hash = r.dataset.href; };
    r.onkeydown = e => { if (e.key === "Enter") location.hash = r.dataset.href; };
  }
}

// ---- Security ----------------------------------------------------------
//
// Its four parts as tabs: Sign-in, Agent policy, Connectors and
// credentials, Audit.

const SECURITY_TABS = [["sign-in", "Sign-in"], ["policy", "Agent policy"], ["connectors", "Connectors and credentials"], ["audit", "Audit"]];

function securityView(view, rest) {
  const tab = SECURITY_TABS.some(([k]) => k === rest[0]) ? rest[0] : "connectors";
  if (!$("#sec-tabs", view)) {
    view.innerHTML = `<h1>Security</h1><nav class="ad-tabs" id="sec-tabs" aria-label="Security">${SECURITY_TABS.map(([k, l]) => `<a href="#/security/${k}" data-tab="${k}">${l}</a>`).join("")}</nav><div id="sec"></div>`;
  }
  for (const a of $$("#sec-tabs a", view)) { if (a.dataset.tab === tab) a.setAttribute("aria-current", "page"); else a.removeAttribute("aria-current"); }
  const box = $("#sec", view);
  if (box.dataset.tab !== tab) { box.dataset.tab = tab; box.innerHTML = ""; UI.closePanel(); }
  if (tab === "audit") return auditView(box, rest[1]);
  if (tab === "connectors") return connectorsView(box, rest[1]);
  box.innerHTML = UI.empty("Not built yet.", "docs/admin-console.md lists what this holds when it is done.");
}

// Security › Connectors and credentials: the organisation's apps, their
// status, how people reach them, their scopes and last use; a row opens the
// app, where it is turned off or on, who may use it is set, and the
// organisation's credential is set, rotated or revoked. A credential is
// never shown, only whether it is set and when.

const CONN_STATUS = { ready: ["ok", "Ready"], needs_credential: ["warn", "Needs its credential"], needs_review: ["warn", "Tools not reviewed"], off: ["", "Off"] };
const SIGN_IN = { own_account: "Each person's own account", org_key: "The organisation's key", none: "No sign-in" };
const AUDIENCE = [["private", "Only the person"], ["internal", "People in the organisation"], ["public", "Anyone"]];
const LABEL = [["principal", "Trusted, not private"], ["private", "Private"], ["untrusted", "Untrusted"], ["private+untrusted", "Private and untrusted"]];
const credName = c => (c.credential.kind === "client_secret" ? `${c.registration ? c.registration[0].toUpperCase() + c.registration.slice(1) : "The provider"}'s client secret` : "The organisation's key");
const day = t => (t ? new Date(t).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" }) : "");

async function connectorsView(box, name) {
  if (!$("#conn-rows", box)) {
    box.innerHTML = `<div class="row ad-bar">
        <input id="conn-q" type="search" placeholder="Find an app" aria-label="Find an app" autocomplete="off">
        <select id="conn-f" aria-label="Show"><option value="">All apps</option><option value="attention">Needs attention</option><option value="off">Off</option><option value="used">Used by someone</option></select>
        <button class="primary" id="conn-add">Add an app</button>
      </div><div id="conn-rows"></div>`;
    $("#conn-q", box).oninput = () => drawConnectors(box);
    $("#conn-f", box).onchange = () => drawConnectors(box);
    $("#conn-add", box).onclick = () => addConnector(box);
    await loadConnectors(box);
  }
  if (!name) return UI.closePanel();
  connectorPanel(box, name);
}

async function loadConnectors(box) {
  const el = $("#conn-rows", box);
  if (!box.conns) UI.loading(el, "the organisation's apps");
  try { box.conns = (await api("GET", "/connectors")).connectors; drawConnectors(box); }
  catch (e) { UI.error(el, "The organisation's apps could not be read.", e, () => loadConnectors(box)); }
}

function drawConnectors(box) {
  const el = $("#conn-rows", box);
  const q = $("#conn-q", box).value.trim().toLowerCase(), f = $("#conn-f", box).value;
  const all = box.conns || [];
  const rows = all.filter(c => (!q || `${c.title} ${c.name}`.toLowerCase().includes(q))
    && (!f || (f === "attention" && (c.status === "needs_credential" || c.status === "needs_review")) || (f === "off" && !c.enabled) || (f === "used" && c.last_used)));
  el.innerHTML = rows.length ? `<div class="ad-scroll"><table class="t small"><thead><tr><th>App</th><th>Status</th><th>Sign-in</th><th>Scopes</th><th>Credential</th><th class="n">Signed in</th><th>Last used</th></tr></thead><tbody>${rows.map(c => {
      const [cls, label] = CONN_STATUS[c.status] || ["", c.status];
      return `<tr class="click" data-href="#/security/connectors/${esc(c.name)}" tabindex="0">
        <td><strong>${esc(c.title)}</strong> <span class="muted mono">${esc(c.name)}</span></td>
        <td><span class="chip ${cls}">${esc(label)}</span></td>
        <td>${esc(SIGN_IN[c.sign_in] || c.sign_in)}</td>
        <td>${c.scopes.length ? `${c.scopes.length} scope${c.scopes.length === 1 ? "" : "s"}` : "–"}</td>
        <td>${c.credential.kind === "none" ? "–" : c.credential.set ? `Set ${esc(day(c.credential.changed_at))}` : `<span class="chip warn">Not set</span>`}</td>
        <td class="n">${esc(c.signed_in)}</td><td>${esc(c.last_used ? when(c.last_used) : "Never")}</td></tr>`;
    }).join("")}</tbody></table></div>`
    : UI.empty(all.length ? "No app matches." : "The organisation has no apps yet.", all.length ? "Clear the search or the filter." : "Add one by its address.");
  rowLinks(el);
}

async function connectorPanel(box, name) {
  const c = (box.conns || []).find(x => x.name === name);
  const back = "#/security/connectors";
  if (!c) { const b = UI.panel("App", "", back); b.innerHTML = UI.empty("No such app."); return; }
  const [cls, label] = CONN_STATUS[c.status] || ["", c.status];
  const reload = async () => { await loadConnectors(box); if (location.hash === `${back}/${name}`) connectorPanel(box, name); };
  const audience = (AUDIENCE.find(([k]) => k === c.audience) || [, c.audience])[1];
  const returns = (LABEL.find(([k]) => k === c.label) || [, c.label])[1];
  const body = UI.panel(c.title, `
    <p class="small"><span class="chip ${cls}">${esc(label)}</span> <span class="muted">${esc({ catalog: "From the catalog", added: "Added by its address", config: "In the appliance's config file" }[c.source])}</span></p>
    <dl class="kv small">
      <dt>Name</dt><dd class="mono">${esc(c.name)}</dd>
      <dt>Address</dt><dd class="mono ad-wrap">${esc(c.url)}</dd>
      <dt>Sign-in</dt><dd>${esc(SIGN_IN[c.sign_in] || c.sign_in)}</dd>
      <dt>Scopes</dt><dd>${c.scopes.length ? c.scopes.map(x => `<div class="mono ad-wrap">${esc(x)}</div>`).join("") : "–"}</dd>
      <dt>What reaches it</dt><dd>${esc(audience)}</dd>
      <dt>What it returns</dt><dd>${esc(returns)}</dd>
      <dt>Signed in</dt><dd>${esc(c.signed_in)} ${c.signed_in === 1 ? "person" : "people"}</dd>
      <dt>Last used</dt><dd>${esc(c.last_used ? when(c.last_used) : "Never")}</dd>
    </dl>
    <h3>For the organisation</h3>
    <div class="row ad-bar">${c.enabled ? `<button id="cn-off" class="danger">Turn off</button>` : `<button id="cn-on" class="primary">Turn on</button>`}</div>
    <p class="small muted">${c.enabled ? "Turned off, no one is offered it and no task reaches it. People's sign-ins stay until revoked." : "It is off: no one is offered it and no task reaches it."}</p>
    <form id="cn-groups" class="ad-form"><label>Who may use it <input name="groups" value="${esc(c.access_groups.join(", "))}" placeholder="Everyone" autocomplete="off" aria-describedby="cn-groups-how"></label>
      <p class="small muted" id="cn-groups-how">Groups, separated by commas. Empty: everyone in the organisation.</p>
      <div class="row"><button>Save</button></div></form>
    <h3>Credential</h3>
    ${c.credential.kind === "none"
      ? `<p class="small">${c.sign_in === "own_account" ? "Each person signs in with their own account. The organisation holds no credential for it." : "It takes no credential."}</p>
         ${c.signed_in ? `<div class="row ad-bar"><button id="cn-revoke" class="danger">Revoke every sign-in</button></div>` : ""}`
      : `<p class="small">${esc(credName(c))}: ${c.credential.set ? `<strong>set</strong>${c.credential.changed_at ? `, ${esc(when(c.credential.changed_at))}` : ""}` : "<strong>not set</strong>"}. It is never shown again, here or anywhere.</p>
         ${c.credential.kind === "client_secret" ? `<p class="small muted">It serves every ${esc(c.registration)} app: setting it here sets it for all of them.</p>` : ""}
         <form id="cn-cred" class="ad-form"><label>${c.credential.set ? "New value" : "Value"} <input name="value" type="password" autocomplete="new-password" spellcheck="false" required></label>
           <div class="row"><button class="primary">${c.credential.set ? "Rotate" : "Set"}</button>${c.credential.set ? ` <button type="button" id="cn-revoke" class="danger">Revoke</button>` : ""}</div></form>`}
    ${c.source === "added" ? `<h3>Remove</h3><p class="small muted">Removing it revokes every sign-in to it and deletes its key.</p><div class="row"><button id="cn-remove" class="danger">Remove ${esc(c.title)}</button></div>` : ""}`, back);
  const setOn = on => act("PATCH", `/connectors/${name}`, { enabled: on }, async () => {
    UI.toast(on ? `${c.title} is on.` : `${c.title} is off for everyone.`, () => act("PATCH", `/connectors/${name}`, { enabled: !on }, reload));
    await reload();
  });
  $("#cn-off", body)?.addEventListener("click", () => setOn(false));
  $("#cn-on", body)?.addEventListener("click", () => setOn(true));
  $("#cn-groups", body).onsubmit = e => {
    e.preventDefault();
    const groups = new FormData(e.target).get("groups").split(",").map(x => x.trim()).filter(Boolean);
    const was = c.access_groups;
    act("PATCH", `/connectors/${name}`, { access_groups: groups }, async () => {
      UI.toast(groups.length ? `Only ${groups.join(", ")} may use ${c.title}.` : `Everyone may use ${c.title}.`, () => act("PATCH", `/connectors/${name}`, { access_groups: was }, reload));
      await reload();
    });
  };
  const cred = $("#cn-cred", body);
  if (cred) cred.onsubmit = e => {
    e.preventDefault();
    const value = new FormData(e.target).get("value");
    e.target.reset();
    act("PUT", `/connectors/${name}/credential`, { value }, async () => { UI.toast(`${credName(c)} is ${c.credential.set ? "rotated" : "set"}.`); await reload(); });
  };
  const rev = $("#cn-revoke", body);
  if (rev) rev.onclick = async () => {
    const shared = c.credential.kind === "client_secret" ? ` Every person's sign-in to each ${c.registration} app ends.` : c.signed_in ? ` ${c.signed_in} sign-in${c.signed_in === 1 ? "" : "s"} end.` : "";
    if (await UI.confirm({ title: c.credential.kind === "none" ? `Revoke every sign-in to ${c.title}?` : `Revoke ${credName(c).toLowerCase()}?`, body: `It is revoked at the app and deleted here.${shared} People sign in again once it is set.`, verb: "Revoke", danger: true, typed: c.name }))
      act("DELETE", `/connectors/${name}/credential`, null, async v => { UI.toast(`Revoked: ${v?.sign_ins_revoked ?? 0} sign-ins ended.`); await reload(); });
  };
  const rm = $("#cn-remove", body);
  if (rm) rm.onclick = async () => {
    if (await UI.confirm({ title: `Remove ${c.title}?`, body: "Every sign-in to it is revoked, its key deleted, and it leaves the Library.", verb: "Remove", danger: true, typed: c.name }))
      act("DELETE", `/connectors/${name}`, null, async () => { UI.toast(`${c.title} is removed.`); location.hash = back; await loadConnectors(box); });
  };
}

function addConnector(box) {
  const d = document.createElement("dialog");
  d.className = "ad-sheet";
  d.setAttribute("aria-labelledby", "add-title");
  d.innerHTML = `<form id="add-form">
    <h2 id="add-title">Add an app</h2>
    <p class="small muted">By its MCP address. Magnus asks the app whether people sign in to it with their own accounts.</p>
    <label>Address <input name="url" type="url" required placeholder="https://" autocomplete="off"></label>
    <label>What reaches it <select name="audience">${AUDIENCE.map(([k, l]) => `<option value="${k}">${l}</option>`).join("")}</select></label>
    <label>What it returns <select name="label">${LABEL.map(([k, l]) => `<option value="${k}" ${k === "private+untrusted" ? "selected" : ""}>${l}</option>`).join("")}</select></label>
    <label>The organisation's key, if it takes one <input name="token" type="password" autocomplete="new-password" spellcheck="false"></label>
    <label>Who may use it <input name="groups" placeholder="Everyone" autocomplete="off"></label>
    <p class="error" id="add-error" hidden></p>
    <div class="row ad-sheet-actions"><button type="button" id="add-cancel">Cancel</button><button class="primary">Add</button></div>
  </form>`;
  document.body.append(d);
  d.addEventListener("close", () => d.remove());
  $("#add-cancel", d).onclick = () => d.close();
  $("#add-form", d).onsubmit = async e => {
    e.preventDefault();
    const f = Object.fromEntries(new FormData(e.target));
    const body = { url: f.url, audience: f.audience, label: f.label, token: f.token || undefined, access_groups: f.groups.split(",").map(x => x.trim()).filter(Boolean) };
    try {
      const v = await api("POST", "/connectors", body);
      d.close();
      UI.toast(`${v.name} is added.${v.sign_in ? " People sign in to it with their own accounts." : ""}`);
      await loadConnectors(box);
      location.hash = `#/security/connectors/${v.name}`;
    } catch (err) { const el = $("#add-error", d); el.textContent = err.message; el.hidden = false; }
  };
  d.showModal();
}

// ---- Users -------------------------------------------------------------
//
// Everyone in the organisation's realm, with Invite; a row opens the person
// in the detail panel, as tabs.

async function usersView(view, id, tab) {
  if (!$("#users-rows", view)) {
    view.innerHTML = `<h1>Users</h1>
      <div class="row ad-bar"><input id="users-q" type="search" placeholder="Find by name or email" aria-label="Find people" autocomplete="off"><button class="primary" id="invite-open">Invite</button></div>
      <div id="users-rows"></div>`;
    $("#users-q", view).oninput = () => drawUsers(view);
    $("#invite-open", view).onclick = () => invite(view);
    await loadUsers(view);
  }
  if (!id) return UI.closePanel();
  return userPanel(view, id, tab || "overview");
}

async function loadUsers(view) {
  const box = $("#users-rows", view);
  UI.loading(box, "the people");
  try { view.users = (await api("GET", "/users")).users; drawUsers(view); }
  catch (e) { UI.error(box, "The people could not be read.", e, () => loadUsers(view)); }
}

function drawUsers(view) {
  const box = $("#users-rows", view);
  const q = $("#users-q", view).value.trim().toLowerCase();
  const all = view.users || [];
  const rows = all.filter(u => !q || `${u.name} ${u.email}`.toLowerCase().includes(q));
  box.innerHTML = rows.length ? `<div class="ad-scroll"><table class="t small"><thead><tr><th>Name</th><th>Email</th><th>Role</th><th>Status</th><th>Last sign-in</th><th class="n">Sessions</th><th></th></tr></thead><tbody>${rows.map(u => `<tr class="click" data-href="#/users/${esc(u.id)}" tabindex="0">
      <td>${esc(u.name)}</td><td>${esc(u.email || "")}</td><td>${u.role === "admin" ? "Admin" : "Member"}</td>
      <td>${pill(u.status)}</td><td>${esc(when(u.last_sign_in))}</td><td class="n">${esc(u.sessions)}</td>
      <td>${u.status === "invited" ? `<button class="link small" data-resend="${esc(u.id)}">Resend</button> <button class="link small" data-cancel="${esc(u.id)}">Cancel</button>` : ""}</td></tr>`).join("")}</tbody></table></div>`
    : UI.empty(all.length ? "Nobody matches." : "Nobody is in the organisation yet.", all.length ? null : "Invite the first person.");
  rowLinks(box);
  for (const b of $$("[data-resend]", box)) b.onclick = () => act("POST", `/users/${b.dataset.resend}/invite`, null, () => UI.toast("The invitation was sent again."));
  for (const b of $$("[data-cancel]", box)) b.onclick = async () => {
    const u = all.find(x => x.id === b.dataset.cancel);
    if (await UI.confirm({ title: "Cancel this invitation?", body: `${u?.email || "Their"} account goes, and the link in the email stops working.`, verb: "Cancel invitation", danger: true }))
      act("DELETE", `/users/${b.dataset.cancel}/invite`, null, () => { UI.toast("The invitation is cancelled."); loadUsers(view); });
  };
}

function invite(view) {
  const d = document.createElement("dialog");
  d.className = "ad-sheet";
  d.setAttribute("aria-labelledby", "invite-title");
  d.innerHTML = `<form id="invite-form">
    <h2 id="invite-title">Invite someone</h2>
    <p class="small muted">They get an email from ${esc(org())}'s sign-in with a link to choose their password.</p>
    <label>Email <input name="email" type="email" required autocomplete="off"></label>
    <label>Name <input name="name" required autocomplete="off"></label>
    <label>Role <select name="role"><option value="member">Member</option><option value="admin">Admin (asks for your password again)</option></select></label>
    <p class="error" id="invite-error" hidden></p>
    <div class="row ad-sheet-actions"><button type="button" id="invite-cancel">Cancel</button><button class="primary">Send invitation</button></div>
  </form>`;
  document.body.append(d);
  d.addEventListener("close", () => d.remove());
  $("#invite-cancel", d).onclick = () => d.close();
  $("#invite-form", d).onsubmit = async e => {
    e.preventDefault();
    try {
      const v = await api("POST", "/users", Object.fromEntries(new FormData(e.target)));
      d.close();
      UI.toast(v.email_sent ? "Invitation sent." : `The account is made, but the email could not be sent (${v.email_error}). Use Resend once email works.`);
      loadUsers(view);
    } catch (err) { const el = $("#invite-error", d); el.textContent = err.message; el.hidden = false; }
  };
  d.showModal();
}

async function userPanel(view, id, tab) {
  const body = UI.panel("Person", "", "#/users");
  UI.loading(body, "this person");
  let u;
  try { u = await api("GET", `/users/${id}`); } catch (e) { return UI.error(body, "This person could not be read.", e, () => userPanel(view, id, tab)); }
  $("#panel h2").textContent = u.name;
  const tabs = [["overview", "Overview"], ["sessions", "Sessions"], ["schedules", "Schedules"], ["data", "Data"], ["delete", "Delete"]];
  const sessions = Array.isArray(u.sessions) ? u.sessions : [];
  const again = () => { loadUsers(view); userPanel(view, id, tab); };
  body.innerHTML = `<p class="muted small">${esc(u.email || "")} · ${u.role === "admin" ? "Admin" : "Member"} · ${pill(u.status)}</p>
    <nav class="ad-tabs" aria-label="${esc(u.name)}">${tabs.map(([k, l]) => `<a href="#/users/${esc(id)}/${k}" ${k === tab ? 'aria-current="page"' : ""}>${l}</a>`).join("")}</nav>
    <div id="user-tab"></div>`;
  const pane = $("#user-tab", body);
  if (tab === "schedules") return userSchedules(pane, u, id);
  if (tab === "data") return userData(pane, u, id);
  if (tab === "delete") return userDelete(pane, u, id, view);
  if (tab === "sessions") {
    pane.innerHTML = sessions.length ? `<table class="t small"><thead><tr><th>Signed in</th><th>From</th><th>Ends</th><th></th></tr></thead><tbody>${sessions.map(s => `<tr><td>${esc(when(s.created))}</td><td>${esc(s.client === "browser" ? "A browser" : "The app")}</td><td>${esc(when(s.expires))}</td><td><button class="link small" data-end="${esc(s.id)}">End</button></td></tr>`).join("")}</tbody></table>`
      : UI.empty("No sessions open on the appliance.");
    for (const b of $$("[data-end]", pane)) b.onclick = async () => {
      if (await UI.confirm({ title: "End this session?", body: `That device is signed out of ${org()}.`, verb: "End session", danger: true }))
        act("DELETE", `/users/${id}/sessions/${b.dataset.end}`, null, () => { UI.toast("Session ended."); again(); });
    };
    return;
  }
  const other = u.role === "admin" ? "member" : "admin";
  pane.innerHTML = `<dl class="kv small"><dt>Created</dt><dd>${esc(when(u.created))}</dd><dt>Sessions here</dt><dd>${esc(sessions.length)}</dd></dl>
    <div class="row ad-bar">
      <button id="u-role">Make ${other === "admin" ? "an admin" : "a member"}</button>
      ${u.status === "disabled" ? `<button id="u-enable">Turn on</button>` : `<button id="u-disable" class="danger">Turn off</button>`}
      <button id="u-end">End all sessions</button>
    </div>
    <p class="small muted">Turning someone off ends their sessions here and at sign-in, and pauses their schedules until they resume them. A role change asks for your password again.</p>`;
  $("#u-role", pane).onclick = async () => {
    if (await UI.confirm({ title: `Make ${u.name} ${other === "admin" ? "an admin" : "a member"}?`, body: other === "admin" ? "An admin has full authority over everything on the appliance." : "They lose the admin console.", verb: "Change role" }))
      act("PATCH", `/users/${id}`, { role: other }, () => { UI.toast(`${u.name} is now ${other === "admin" ? "an admin" : "a member"}.`); again(); });
  };
  const dis = $("#u-disable", pane);
  if (dis) dis.onclick = () => act("POST", `/users/${id}/disable`, null, v => {
    UI.toast(`${u.name} is turned off: ${v?.sessions_ended ?? 0} sessions ended, ${v?.schedules_paused ?? 0} schedules paused.`, () => act("POST", `/users/${id}/enable`, null, again));
    again();
  });
  const en = $("#u-enable", pane);
  if (en) en.onclick = () => act("POST", `/users/${id}/enable`, null, () => { UI.toast(`${u.name} is turned on. Their paused schedules stay paused until they resume them.`); again(); });
  $("#u-end", pane).onclick = async () => {
    if (await UI.confirm({ title: `End every session ${u.name} has open?`, body: "Every device they use is signed out, here and at sign-in.", verb: "End all sessions", danger: true }))
      act("DELETE", `/users/${id}/sessions`, null, () => { UI.toast("Every session ended."); again(); });
  };
}

// A schedule's rule in words.
const rule = x => (x.daily_at ? `Every day at ${x.daily_at}` : x.every_minutes ? (x.every_minutes % 1440 === 0 ? `Every ${x.every_minutes / 1440} day${x.every_minutes === 1440 ? "" : "s"}` : x.every_minutes % 60 === 0 ? `Every ${x.every_minutes / 60} hour${x.every_minutes === 60 ? "" : "s"}` : `Every ${x.every_minutes} minutes`) : "Once");

// Users › Schedules and triggers: each of their schedules, suspended or
// resumed by an admin. (Magnus starts work on a schedule or on request;
// it has no other triggers.)
async function userSchedules(pane, u, id) {
  UI.loading(pane, "their schedules");
  let v;
  try { v = await api("GET", `/users/${id}/schedules`); } catch (e) { return UI.error(pane, "Their schedules could not be read.", e, () => userSchedules(pane, u, id)); }
  const list = v.schedules;
  pane.innerHTML = list.length ? `<div class="ad-scroll"><table class="t small"><thead><tr><th>Schedule</th><th>When</th><th>Next</th><th class="n">Runs</th><th class="n">Failures</th><th></th></tr></thead><tbody>${list.map(x => `<tr>
      <td>${esc(x.title)}${x.paused ? `<div class="small"><span class="chip warn">Suspended</span> <span class="muted">${esc(x.paused.reason || x.paused)}</span></div>` : ""}</td>
      <td>${esc(rule(x))}${x.until ? `<div class="small muted">until ${esc(when(x.until))}</div>` : ""}</td>
      <td>${x.paused ? "–" : esc(when(x.next))}</td><td class="n">${esc(x.runs)}</td><td class="n">${esc(x.failures)}</td>
      <td>${x.paused ? `<button class="link small" data-resume="${esc(x.id)}">Resume</button>` : `<button class="link small" data-suspend="${esc(x.id)}">Suspend</button>`}</td></tr>`).join("")}</tbody></table></div>`
    : UI.empty(`${u.name} has no schedules.`);
  const again = () => userSchedules(pane, u, id);
  for (const b of $$("[data-suspend]", pane)) b.onclick = () => act("POST", `/users/${id}/schedules/${b.dataset.suspend}/suspend`, null, () => {
    UI.toast("Suspended: it does not run until it is resumed.", () => act("POST", `/users/${id}/schedules/${b.dataset.suspend}/resume`, null, again));
    again();
  });
  for (const b of $$("[data-resume]", pane)) b.onclick = () => act("POST", `/users/${id}/schedules/${b.dataset.resume}/resume`, null, () => { UI.toast("Resumed."); again(); });
}

// Users › Data: root's access to their tasks, conversations, files and
// memory. Nothing is read until asked; each read is in the audit log and
// in their inbox.
const DATA_KINDS = [["tasks", "Tasks"], ["chats", "Conversations"], ["files", "Files"], ["memory", "Memory"]];

function userData(pane, u, id) {
  pane.innerHTML = `<div class="banner" role="note"><strong>Each access is recorded.</strong> Opening ${esc(u.name)}'s tasks, conversations, files or memory writes it to the audit log, and they are told in their inbox.</div>
    <div class="ad-seg" role="group" aria-label="Their data" id="data-kinds">${DATA_KINDS.map(([k, l]) => `<button data-kind="${k}">${l}</button>`).join("")}</div>
    <div id="data-box">${UI.empty("Choose what to open.")}</div>`;
  const box = $("#data-box", pane);
  for (const b of $$("[data-kind]", pane)) b.onclick = () => {
    for (const x of $$("[data-kind]", pane)) x.setAttribute("aria-pressed", String(x === b));
    dataList(box, u, id, b.dataset.kind);
  };
}

async function dataList(box, u, id, kind) {
  UI.loading(box, `their ${kind}`);
  let v;
  try { v = await api("GET", `/users/${id}/data/${kind}`); } catch (e) { return UI.error(box, `Their ${kind} could not be read.`, e, () => dataList(box, u, id, kind)); }
  const rows = v[kind] || [];
  const open = item => dataItem(box, u, id, kind, item);
  const line = {
    tasks: t => [t.task_id, t.title || t.text, `${t.status} · ${when(t.created_at)}`],
    chats: t => [t.id || t.thread, t.title || "Conversation", when(t.updated_at || t.at || t.created_at)],
    files: f => [f.id, f.name, `${f.media_type || ""} · ${when(f.created_at || f.at)}`],
    memory: m => [m.id, m.fact, `${m.topic || ""} · ${when(m.at)}`],
  }[kind];
  box.innerHTML = rows.length ? `<ul class="ad-list">${rows.map(r => { const [key, title, sub] = line(r); return `<li><button class="link" data-open="${esc(key)}">${esc(title || "(untitled)")}</button><div class="small muted">${esc(sub)}</div></li>`; }).join("")}</ul>`
    : UI.empty(`${u.name} has no ${kind === "chats" ? "conversations" : kind}.`);
  for (const b of $$("[data-open]", box)) b.onclick = () => (kind === "memory" ? null : open(b.dataset.open));
}

async function dataItem(box, u, id, kind, item) {
  UI.loading(box, "it");
  let v;
  try { v = (await api("GET", `/users/${id}/data/${kind}/${encodeURIComponent(item)}`))[kind]; } catch (e) { return UI.error(box, "It could not be read.", e, () => dataItem(box, u, id, kind, item)); }
  const back = `<p class="small"><button class="link" id="data-back">← All their ${kind === "chats" ? "conversations" : kind}</button></p>`;
  let html = "";
  if (kind === "tasks") {
    const t = v.task;
    html = `<h3>${esc(t.title || t.text)}</h3><p class="small muted">${esc(t.status)} · ${esc(when(t.created_at))}${t.summary ? ` · ${esc(t.summary)}` : ""}</p>
      ${(v.timeline.steps || []).map(st => `<div class="card flat small"><div class="muted">Step ${esc(st.step)}</div>${st.text ? `<p>${esc(st.text)}</p>` : ""}${st.calls.map(c => `<div><span class="mono">${esc(c.tool)}</span> ${c.is_error ? `<span class="chip bad">failed</span>` : ""}${c.result ? `<pre class="ad-json">${esc(c.result)}</pre>` : ""}</div>`).join("")}</div>`).join("") || UI.empty("No steps.")}`;
  } else if (kind === "chats") {
    html = (v.entries || []).map(e => `<div class="card flat small"><div class="muted">${esc(e.kind === "message" ? u.name : "Magnus")} · ${esc(when(e.at))}</div><p class="ad-wrap">${esc(e.text || e.summary || "")}</p></div>`).join("") || UI.empty("Nothing was said.");
  } else if (kind === "files") {
    const f = v.file;
    html = `<h3>${esc(f.name)}</h3><p class="small muted">${esc(f.media_type)} · ${esc(v.bytes)} bytes · ${esc(when(f.created_at || f.at))}</p>${v.text != null ? `<pre class="ad-json">${esc(v.text)}</pre>` : UI.empty("Not text: nothing to show here.")}`;
  }
  box.innerHTML = back + html;
  $("#data-back", box).onclick = () => dataList(box, u, id, kind);
}

// Users › Delete: the account and everything of theirs, for good. It asks
// for a fresh password, and their email typed.
function userDelete(pane, u, id, view) {
  pane.innerHTML = `<p>Deleting ${esc(u.name)} removes their account at sign-in and everything of theirs on the appliance: their running tasks stop, their schedules end, their sessions and their sign-ins to apps are revoked, and their tasks, conversations, files and memory are deleted, from the backups too. It cannot be undone.</p>
    <div class="row ad-bar"><button class="danger" id="u-delete">Delete ${esc(u.name)}</button></div>
    <p class="small muted">It asks for your password again, and for their email address typed.</p>`;
  $("#u-delete", pane).onclick = async () => {
    if (!(await UI.confirm({ title: `Delete ${u.name} for good?`, body: "Their account and all of their data go, and nothing brings them back.", verb: "Delete", danger: true, typed: u.email }))) return;
    act("DELETE", `/users/${id}`, { email: u.email }, v => {
      UI.toast(`${u.name} is deleted: ${v?.runs_stopped ?? 0} tasks stopped, ${v?.schedules_ended ?? 0} schedules ended, ${v?.content_removed ?? 0} records removed.`);
      location.hash = "#/users";
      loadUsers(view);
    });
  };
}

// ---- Sections ----------------------------------------------------------

const SECTIONS = {
  overview: view => overviewView(view),
  users: (view, rest) => usersView(view, rest[0], rest[1]),
  security: (view, rest) => securityView(view, rest),
  operations: view => operationsView(view),
};

function section() {
  const parts = (location.hash.replace(/^#\/?/, "") || "overview").split("/");
  const name = parts[0] in SECTIONS || parts[0] === "knowledge" ? parts[0] : "overview";
  for (const a of $$("#nav a")) {
    if (a.dataset.section === name) a.setAttribute("aria-current", "page"); else a.removeAttribute("aria-current");
  }
  let view = $("#view");
  // A different section starts its screen afresh; within one, the list
  // stays and only the panel changes.
  const keep = name === "users" || name === "security";
  if (!view || view.dataset.section !== `${name}/${parts[1] && !keep ? parts[1] : ""}`) {
    view?.remove();
    UI.closePanel();
    view = document.createElement("section");
    view.id = "view";
    view.dataset.section = `${name}/${parts[1] && !keep ? parts[1] : ""}`;
    $("#main").prepend(view);
  }
  const draw = SECTIONS[name];
  if (draw) return draw(view, parts.slice(1));
  const title = $(`#nav a[data-section="${name}"]`)?.textContent || "Overview";
  view.innerHTML = `<h1>${esc(title)}</h1>${UI.empty("Not built yet.", "docs/admin-console.md lists what this section holds when it is done.")}`;
}

async function session() {
  try {
    const r = await fetch("/api/v1/session", { credentials: "same-origin" });
    if (!r.ok) return signedOut();
    Admin.me = await r.json();
  } catch { return signedOut(); }
  document.body.classList.add("signed-in");
  $("#sign-in").hidden = true;
  $("#nav").hidden = false;
  menu();
  section();
  killSwitch();
  live();
}

addEventListener("hashchange", () => { if (Admin.me) section(); });
addEventListener("DOMContentLoaded", () => {
  status();
  session();
  setInterval(status, 15000);
});
