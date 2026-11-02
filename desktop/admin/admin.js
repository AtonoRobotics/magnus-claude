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

// Users (admin console spec): everyone in the organisation's realm, with
// Invite; each person's Overview (role, on or off, end every session) and
// Sessions. A role change, or inviting an admin, asks for a fresh password.
const when = t => (t ? new Date(t).toLocaleString() : "–");
const act = async (method, path, body, then) => {
  try { const v = await api(method, path, body); then?.(v); } catch (e) { alert(e.message); }
};

async function usersView(view) {
  const id = (location.hash.match(/^#\/users\/([\w-]+)/) || [])[1];
  if (id) return userView(view, id);
  view.innerHTML = `<h1>Users</h1>
    <div class="row ad-bar"><input id="users-q" type="search" placeholder="Find by name or email" aria-label="Find people" autocomplete="off"><button class="primary" id="invite-open">Invite</button></div>
    <div id="users-rows"><p class="muted">Loading…</p></div>
    <dialog id="invite" aria-labelledby="invite-title"><form method="dialog" id="invite-form">
      <h2 id="invite-title">Invite someone</h2>
      <p class="small muted">They get an email from ${esc(document.title.split(" · ")[0])}'s sign-in with a link to choose their password.</p>
      <label>Email <input name="email" type="email" required autocomplete="off"></label>
      <label>Name <input name="name" required autocomplete="off"></label>
      <label>Role <select name="role"><option value="member">Member</option><option value="admin">Admin (asks for your password again)</option></select></label>
      <p class="error" id="invite-error" hidden></p>
      <div class="row"><button value="cancel" formnovalidate>Cancel</button><button class="primary" value="send">Send invitation</button></div>
    </form></dialog>`;
  let all = [];
  const draw = () => {
    const q = $("#users-q", view).value.trim().toLowerCase();
    const rows = all.filter(u => !q || `${u.name} ${u.email}`.toLowerCase().includes(q));
    $("#users-rows", view).innerHTML = rows.length ? `<div class="ad-scroll"><table class="t small"><thead><tr><th>Name</th><th>Email</th><th>Role</th><th>Status</th><th>Last sign-in</th><th>Sessions</th><th></th></tr></thead><tbody>${rows.map(u => `<tr>
        <td><a href="#/users/${esc(u.id)}">${esc(u.name)}</a></td><td>${esc(u.email || "")}</td><td>${u.role === "admin" ? "Admin" : "Member"}</td>
        <td><span class="ad-pill ${esc(u.status)}">${esc(u.status)}</span></td><td>${esc(when(u.last_sign_in))}</td><td>${esc(u.sessions)}</td>
        <td>${u.status === "invited" ? `<button class="link small" data-resend="${esc(u.id)}">Resend</button> <button class="link small" data-cancel="${esc(u.id)}">Cancel</button>` : ""}</td></tr>`).join("")}</tbody></table></div>`
      : `<p class="empty">${all.length ? "Nobody matches." : "Nobody is in the organisation yet."}</p>`;
    for (const b of view.querySelectorAll("[data-resend]")) b.onclick = () => act("POST", `/users/${b.dataset.resend}/invite`, null, () => alert("Sent again."));
    for (const b of view.querySelectorAll("[data-cancel]")) b.onclick = () => confirm("Cancel this invitation? The account goes, and the link stops working.") && act("DELETE", `/users/${b.dataset.cancel}/invite`, null, load);
  };
  const load = async () => {
    try { const v = await api("GET", "/users"); if (!v) return; all = v.users; draw(); }
    catch (e) { $("#users-rows", view).innerHTML = `<p class="error">The people could not be read: ${esc(e.message)}</p>`; }
  };
  $("#users-q", view).oninput = draw;
  const dlg = $("#invite", view);
  $("#invite-open", view).onclick = () => { $("#invite-form", view).reset(); $("#invite-error", view).hidden = true; dlg.showModal(); };
  $("#invite-form", view).onsubmit = async e => {
    if (e.submitter?.value !== "send") return;
    e.preventDefault();
    const body = Object.fromEntries(new FormData(e.target));
    try {
      const v = await api("POST", "/users", body);
      if (!v) return;
      dlg.close();
      if (!v.email_sent) alert(`The account is made, but the email could not be sent (${v.email_error}). Use Resend once email works.`);
      load();
    } catch (err) { const el = $("#invite-error", view); el.textContent = err.message; el.hidden = false; }
  };
  load();
}

async function userView(view, id) {
  const tab = (location.hash.match(/^#\/users\/[\w-]+\/(\w+)/) || [, "overview"])[1];
  view.innerHTML = `<p class="small"><a href="#/users">Users</a></p><div id="user"><p class="muted">Loading…</p></div>`;
  let u;
  try { u = await api("GET", `/users/${id}`); } catch (e) { $("#user", view).innerHTML = `<p class="error">${esc(e.message)}</p>`; return; }
  if (!u) return;
  const tabs = [["overview", "Overview"], ["sessions", "Sessions"]];
  const sessions = Array.isArray(u.sessions) ? u.sessions : [];
  const again = () => userView(view, id);
  $("#user", view).innerHTML = `<h1>${esc(u.name)}</h1><p class="muted">${esc(u.email || "")} · ${u.role === "admin" ? "Admin" : "Member"} · <span class="ad-pill ${esc(u.status)}">${esc(u.status)}</span></p>
    <nav class="ad-tabs" aria-label="${esc(u.name)}">${tabs.map(([k, l]) => `<a href="#/users/${esc(id)}/${k}" ${k === tab ? 'aria-current="page"' : ""}>${l}</a>`).join("")}</nav>
    <div id="user-tab"></div>`;
  const pane = $("#user-tab", view);
  if (tab === "sessions") {
    pane.innerHTML = sessions.length ? `<table class="t small"><thead><tr><th>Signed in</th><th>From</th><th>Ends</th><th></th></tr></thead><tbody>${sessions.map(s => `<tr><td>${esc(when(s.created))}</td><td>${esc(s.client === "browser" ? "A browser" : "The app")}</td><td>${esc(when(s.expires))}</td><td><button class="link small" data-end="${esc(s.id)}">End</button></td></tr>`).join("")}</tbody></table>`
      : `<p class="empty">No sessions open on the appliance.</p>`;
    for (const b of pane.querySelectorAll("[data-end]")) b.onclick = () => act("DELETE", `/users/${id}/sessions/${b.dataset.end}`, null, again);
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
  $("#u-role", view).onclick = () => confirm(`Make ${u.name} ${other === "admin" ? "an admin" : "a member"}?`) && act("PATCH", `/users/${id}`, { role: other }, again);
  const dis = $("#u-disable", view);
  if (dis) dis.onclick = () => confirm(`Turn ${u.name} off? Their sessions end and their schedules pause.`) && act("POST", `/users/${id}/disable`, null, again);
  const en = $("#u-enable", view);
  if (en) en.onclick = () => act("POST", `/users/${id}/enable`, null, again);
  $("#u-end", view).onclick = () => confirm(`End every session ${u.name} has open?`) && act("DELETE", `/users/${id}/sessions`, null, again);
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
  if (name === "users") return usersView(view);
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
