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
    const r = await fetch("/api/v1/health", { credentials: "same-origin" });
    const v = await r.json();
    el.dataset.state = v.ok ? "ok" : "down";
    $(".small", el).textContent = v.ok ? `Running · ${v.version}` : "Not answering";
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

function section() {
  const name = (location.hash.match(/^#\/(\w+)/) || [, "overview"])[1];
  for (const a of document.querySelectorAll("#nav a")) {
    if (a.dataset.section === name) a.setAttribute("aria-current", "page"); else a.removeAttribute("aria-current");
  }
  const title = $(`#nav a[data-section="${name}"]`)?.textContent || "Overview";
  let view = $("#view");
  if (!view) { view = document.createElement("section"); view.id = "view"; $("#main").append(view); }
  if (name === "security") return auditView(view);
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
}

addEventListener("hashchange", () => { if (Admin.me) section(); });
addEventListener("DOMContentLoaded", () => {
  status();
  session();
  setInterval(status, 15000);
});
