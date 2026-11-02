// The admin console's shell (admin console spec, Web UI). It shows the
// appliance's status; signed in, the five sections and the admin's menu,
// else the sign-in. Each section arrives with its own part of the console.
"use strict";

const $ = (s, el = document) => el.querySelector(s);
const esc = s => String(s ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const Admin = { me: null };

async function status() {
  const el = $("#status");
  try {
    const r = await fetch("/status", { credentials: "same-origin" });
    const v = await r.json();
    el.dataset.state = v.ok ? "ok" : "down";
    $(".small", el).textContent = v.ok ? "Running" : "Not answering";
  } catch {
    el.dataset.state = "down";
    $(".small", el).textContent = "Not answering";
  }
}

// Every change carries the session's CSRF token.
async function api(method, path, body) {
  const r = await fetch(`/api/v1${path}`, {
    method, credentials: "same-origin",
    headers: { ...(body ? { "content-type": "application/json" } : {}), ...(Admin.me ? { "x-csrf-token": Admin.me.csrf } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  });
  if (r.status === 401) {
    const v = await r.json().catch(() => ({}));
    // A step-up goes to the provider and comes back here.
    if (v.error === "step_up_required") { location.href = `/auth/step-up?then=${encodeURIComponent(location.pathname + location.hash)}`; return null; }
    signedOut();
    return null;
  }
  if (!r.ok) throw new Error((await r.json().catch(() => ({}))).message || r.statusText);
  return r.status === 204 ? null : r.json();
}

function signedOut() {
  Admin.me = null;
  document.body.classList.remove("signed-in");
  $("#nav").hidden = true;
  $("#me").innerHTML = "";
  $("#sign-in").hidden = false;
}

// Security › Audit (admin console spec): the full log as a table, filtered
// by actor, action, target and time, with CSV export.
async function auditView(view) {
  view.innerHTML = `<h1>Security</h1><h2>Audit</h2>
    <form class="row" id="audit-filter" role="search">
      <label>Who <input name="actor" autocomplete="off"></label>
      <label>Action <input name="action" autocomplete="off"></label>
      <label>On <input name="target" autocomplete="off"></label>
      <label>From <input name="since" type="datetime-local"></label>
      <label>To <input name="until" type="datetime-local"></label>
      <button class="primary">Filter</button>
      <a class="button" id="audit-csv" href="/api/v1/audit.csv" download>Export CSV</a>
    </form>
    <div id="audit-rows"><p class="muted">Loading the audit log…</p></div>`;
  const form = $("#audit-filter", view);
  const query = () => {
    const q = new URLSearchParams();
    for (const [k, v] of new FormData(form)) if (v) q.set(k, k === "since" || k === "until" ? new Date(v).toISOString() : v);
    return q.toString();
  };
  const draw = async () => {
    const q = query();
    $("#audit-csv", view).href = `/api/v1/audit.csv${q ? `?${q}` : ""}`;
    try {
      const v = await api("GET", `/audit${q ? `?${q}` : ""}`);
      if (!v) return;
      $("#audit-rows", view).innerHTML = v.entries.length
        ? `<table class="t small"><thead><tr><th>When</th><th>Who</th><th>What</th><th>On</th><th>From</th><th>Change</th></tr></thead><tbody>${v.entries.map(e => `<tr>
            <td>${esc(new Date(e.at).toLocaleString())}</td><td>${esc(e.name || e.admin)}</td><td>${esc(e.action)}</td><td>${esc(e.target || "")}</td>
            <td class="mono">${esc(e.source || "")}</td><td class="mono">${e.before || e.after ? `${esc(JSON.stringify(e.before ?? null))} → ${esc(JSON.stringify(e.after ?? null))}` : ""}</td></tr>`).join("")}</tbody></table>`
        : `<p class="empty">Nothing in the audit log matches.</p>`;
    } catch (err) {
      $("#audit-rows", view).innerHTML = `<p class="error">The audit log could not be read: ${esc(err.message)}</p>`;
    }
  };
  form.onsubmit = e => { e.preventDefault(); draw(); };
  draw();
}

const pct = x => (typeof x === "number" ? `${Math.round(x * 100)}%` : "–");
const who = by => String(by || "").replace(/^admin:/, "an admin ").replace(/^person:/, "");

// The kill switch, on every screen (admin console spec, Shell): "Stop all
// agents", pressed and held for a second; while it is on, a red banner says
// who engaged it and when, with Release (which needs a fresh password).
async function killSwitch() {
  const el = $("#kill");
  let v;
  try { v = await api("GET", "/kill-switch"); } catch { return; }
  if (!v) return;
  el.hidden = false;
  let banner = $("#kill-banner");
  if (v.engaged) {
    if (!banner) { banner = document.createElement("div"); banner.id = "kill-banner"; banner.setAttribute("role", "alert"); document.body.prepend(banner); }
    banner.innerHTML = `<strong>All agents are stopped.</strong> <span>Engaged by ${esc(who(v.by))}${v.at ? ` at ${esc(new Date(v.at).toLocaleString())}` : ""}.</span> <button id="kill-release">Release</button>`;
    $("#kill-release").onclick = async () => { try { await api("POST", "/kill-switch/release"); killSwitch(); } catch (e) { alert(e.message); } };
    el.innerHTML = `<span class="small ad-kill-on">Stopped</span>`;
    return;
  }
  banner?.remove();
  if (el.querySelector("#kill-hold")) return;
  el.innerHTML = `<button id="kill-hold" class="ad-kill-btn" aria-describedby="kill-how">Stop all agents</button><span id="kill-how" class="vh">Press and hold for one second</span>`;
  const b = $("#kill-hold");
  let t = null;
  const start = e => { if (t) return; e.preventDefault(); b.classList.add("holding"); t = setTimeout(async () => { t = null; b.classList.remove("holding"); try { await api("POST", "/kill-switch/engage"); } catch (err) { alert(err.message); } el.innerHTML = ""; killSwitch(); }, 1000); };
  const stop = () => { clearTimeout(t); t = null; b.classList.remove("holding"); };
  b.addEventListener("pointerdown", start);
  for (const ev of ["pointerup", "pointerleave", "pointercancel"]) b.addEventListener(ev, stop);
  b.addEventListener("keydown", e => { if ((e.key === " " || e.key === "Enter") && !e.repeat) start(e); });
  b.addEventListener("keyup", e => { if (e.key === " " || e.key === "Enter") stop(); });
}

async function overviewView(view) {
  view.innerHTML = `<h1>Overview</h1><div id="ov"><p class="muted">Loading…</p></div>`;
  try {
    const v = await api("GET", "/overview");
    if (!v) return;
    const c = v.cards;
    $("#ov", view).innerHTML = `
      <p class="small"><span class="ad-dot ${v.status.healthy ? "ok" : "down"}"></span> ${v.status.healthy ? "Everything is answering." : "Something needs you."} · Kill switch ${v.status.kill_switch ? "<strong>on</strong>" : "off"} · Magnus ${esc(v.status.version || "")}</p>
      <div class="ad-cards">
        <div class="card"><div class="small muted">Tasks running</div><div class="ad-big">${esc(c.running_tasks ?? "–")}</div><div class="small muted">${esc(c.waiting_tasks ?? 0)} waiting</div></div>
        <div class="card"><div class="small muted">People signed in</div><div class="ad-big">${esc(c.signed_in ?? "–")}</div></div>
        <div class="card"><div class="small muted">Knowledge changes, last 24 hours</div><div class="ad-big">${esc(c.knowledge_changes_24h ?? "–")}</div></div>
      </div>
      <h2>Alerts</h2>${v.alerts.length ? `<ul class="ad-alerts">${v.alerts.map(a => `<li><span>${esc(a.what)}</span> <a href="${esc(a.fix)}">Fix</a></li>`).join("")}</ul>` : `<p class="empty">No alerts.</p>`}`;
  } catch (e) { $("#ov", view).innerHTML = `<p class="error">${esc(e.message)}</p>`; }
}

async function operationsView(view) {
  view.innerHTML = `<h1>Operations</h1><h2>Kill switch</h2><div id="ops-kill"></div><h2 id="health">Health</h2><div id="ops-health"><p class="muted">Loading…</p></div>`;
  try {
    const [k, h] = await Promise.all([api("GET", "/kill-switch"), api("GET", "/health")]);
    if (!k || !h) return;
    $("#ops-kill", view).innerHTML = `<p>${k.engaged ? `<strong>On.</strong> Engaged by ${esc(who(k.by))}${k.at ? ` at ${esc(new Date(k.at).toLocaleString())}` : ""}.` : `Off${k.by ? `. Last released by ${esc(who(k.by))}${k.at ? ` at ${esc(new Date(k.at).toLocaleString())}` : ""}` : ""}.`}</p>
      <dl class="kv small"><dt>Tasks halted</dt><dd>${esc(k.tasks_halted)}</dd><dt>Schedules held</dt><dd>${esc(k.schedules_held)}</dd><dt>Sign-ins revoked</dt><dd>${esc(k.sign_ins_revoked)}</dd></dl>`;
    $("#ops-health", view).innerHTML = `<ul class="ad-services">${h.services.map(s => `<li><span class="ad-dot ${s.up ? "ok" : "down"}"></span> ${esc(s.name)} <span class="small muted">${s.up ? "answering" : "not answering"}</span></li>`).join("")}</ul>
      <dl class="kv small"><dt>Load</dt><dd>${esc(h.host.load ?? "–")} on ${esc(h.host.cpus)} processors</dd><dt>Memory used</dt><dd>${pct(h.host.memory_used)}</dd><dt>Disk used</dt><dd>${pct(h.host.disk_used)}</dd><dt>Tasks running</dt><dd>${esc(h.running_tasks ?? "–")}</dd></dl>
      <h3>Recent errors</h3>${(h.recent_errors || []).length ? `<ul class="small">${h.recent_errors.map(e => `<li>${esc(e.reason || "failed")} <span class="muted">· ${esc(e.person)} · ${e.at ? esc(new Date(e.at).toLocaleString()) : ""}</span></li>`).join("")}</ul>` : `<p class="empty">No recent errors.</p>`}`;
  } catch (e) { $("#ops-health", view).innerHTML = `<p class="error">${esc(e.message)}</p>`; }
}

function section() {
  const name = (location.hash.match(/^#\/(\w+)/) || [, "overview"])[1];
  for (const a of document.querySelectorAll("#nav a")) {
    if (a.dataset.section === name) a.setAttribute("aria-current", "page"); else a.removeAttribute("aria-current");
  }
  const title = $(`#nav a[data-section="${name}"]`)?.textContent || "Overview";
  let view = $("#view");
  if (!view) { view = document.createElement("section"); view.id = "view"; $("#main").append(view); }
  if (name === "security") return auditView(view);
  if (name === "overview") return overviewView(view);
  if (name === "operations") return operationsView(view);
  view.innerHTML = `<h1>${esc(title)}</h1><p class="muted">This section arrives with its part of the console.</p>`;
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
  $("#me").innerHTML = `<span class="small">${esc(Admin.me.name)}</span> <button class="link small" id="sign-out">Sign out</button>`;
  $("#sign-out").onclick = async () => { await api("DELETE", "/session"); signedOut(); };
  section();
  killSwitch();
  setInterval(() => { if (Admin.me) killSwitch(); }, 5000);
}

addEventListener("hashchange", () => { if (Admin.me) section(); });
addEventListener("DOMContentLoaded", () => {
  status();
  session();
  setInterval(status, 15000);
});
