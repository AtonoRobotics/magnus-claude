// The operator view: Home, Inbox, New Task, Task Detail and Models, all from
// derived views of the log (K45). The developer view is in dev.js.
"use strict";

const $ = (s, el = document) => el.querySelector(s);
const $$ = (s, el = document) => [...el.querySelectorAll(s)];
const esc = s => String(s ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const api = (m, p, b) => Magnus.api(m, p, b);
const enc = encodeURIComponent;

// Read aloud in the appliance's own voice, a sentence at a time, so the first
// is heard while the next is made. With no voice on the appliance, the
// system's own voice reads it.
const voice = { run: 0, audio: null, done: null };
function hush() {
  voice.run++;
  if (voice.audio) voice.audio.pause();
  if (voice.done) voice.done();
  voice.audio = voice.done = null;
  return Magnus.stopSpeaking();
}
async function readAloud(text) {
  await hush();
  // A sentence at a time; a very long one in pieces, as the appliance reads
  // at most 2000 characters at once.
  const pieces = s => { const out = []; let cur = ""; for (const w of s.split(/\s+/)) { if (cur && (cur + " " + w).length > 1500) { out.push(cur); cur = w; } else cur = cur ? `${cur} ${w}` : w; } if (cur) out.push(cur); return out; };
  const run = voice.run, parts = text.split(/(?<=[.!?])\s+/).filter(s => s.trim()).flatMap(pieces);
  // The voice makes a few pieces at a time for everyone: busy, wait and ask again.
  const ask = async (s, tries = 8) => {
    try { return await api("POST", "/v1/speak", { text: s }); } catch (e) {
      if (e.status !== 429 || tries <= 1 || run !== voice.run) throw e;
      await new Promise(r => setTimeout(r, 400));
      return ask(s, tries - 1);
    }
  };
  const make = s => { const p = ask(s); p.catch(() => {}); return p; };
  let next = parts.length ? make(parts[0]) : null;
  for (let i = 0; i < parts.length; i++) {
    let r;
    try { r = await next; } catch (e) { if (e.status === 503 && i === 0) return run === voice.run && Magnus.speak(text); throw e; }
    if (run !== voice.run) return;
    next = i + 1 < parts.length ? make(parts[i + 1]) : null;
    const src = URL.createObjectURL(new Blob([Uint8Array.from(atob(r.audio_b64), c => c.charCodeAt(0))], { type: r.media_type }));
    await new Promise((done, fail) => {
      const a = voice.audio = new Audio(src);
      voice.done = done;
      a.onended = done;
      a.onerror = () => fail(new Error("Could not play the audio."));
      a.play().catch(fail);
    }).finally(() => URL.revokeObjectURL(src));
    if (run !== voice.run) return;
  }
  voice.audio = voice.done = null;
}

const App = {
  me: null,
  appliance: "",
  inbox: [],
  tasks: [],
  schedules: [],
  threads: [],
  projects: [],
  health: null,
  thread: null,
  versions: null,
  update: null,
  timers: [],
  leave: [],
  routes: [],
  seq: 0,
};

// Each resource retains its last good data; only its newest response may reconcile it.
const Resources = (()=>{
 let epoch=0, serial=0;const states=new Map();let announced="";
 function render() {
  const shell=$(".shell");if(!shell)return;
  let el=$("#connection-status");if(!el){el=document.createElement("div");el.id="connection-status";el.className="connection-status";shell.append(el);}
  const stale=[...states].filter(([,s])=>s.failed), text=stale.map(([name,s])=>`${name} may be stale · ${s.at ? `Last refreshed ${new Date(s.at).toLocaleTimeString()}` : "Not yet refreshed"}`).join(". ");
  el.hidden=!stale.length;
  if(el.textContent!==text)el.textContent=text;
  // Announce transitions once, never repeat unchanged polling failures or timestamps.
  const summary=stale.map(([name])=>name).join(", ");
  if(summary!==announced){announced=summary;let live=$("#connection-announcement");if(!live){live=document.createElement("div");live.id="connection-announcement";live.className="sr";live.setAttribute("role","status");document.body.append(live);}live.textContent=summary ? `${summary} may be stale. Cached information remains available.` : "Connection restored.";}
 }
 function reset(){epoch++;states.clear();announced="";render();}
 function forget(name){states.delete(name);render();}
 function good(name){const s=states.get(name)||{};states.set(name,{...s,failed:false,at:Date.now()});render();}
 function bad(name,error){const s=states.get(name)||{};states.set(name,{...s,failed:true});render();if(error?.status===401 && App.me){stop();signInScreen("Your session ended. Sign in again.");}}
 async function read(name,path,key){const scope=epoch,token=++serial,prior=states.get(name)||{};states.set(name,{...prior,token});
  try{const value=await api("GET",path);if(scope!==epoch || states.get(name)?.token!==token)return;good(name);return key ? value[key] : value;}
  catch(error){if(scope===epoch && states.get(name)?.token===token)bad(name,error);}
 }
 return {read,reset,forget,good,bad,render};
})();

// A question before an action, asked in the app itself: a webview may
// ignore window.confirm and answer no without showing it, so a button
// behind one did nothing at all (the Windows app, owner 2026-11-03:
// Disconnect in the Library). Escape, or Back, is no.
function ask(msg, yes) {
  return new Promise(done => {
    const d = document.createElement("dialog");
    d.className = "ask";
    d.innerHTML = `<p></p><div class="row"><button data-ask="no">Back</button><button class="primary" data-ask="yes"></button></div>`;
    $("p", d).textContent = msg;
    $("[data-ask=yes]", d).textContent = yes;
    d.addEventListener("click", e => { const b = e.target.closest("[data-ask]"); if (b) d.close(b.dataset.ask); });
    d.addEventListener("close", () => { d.remove(); done(d.returnValue === "yes"); });
    document.body.append(d);
    d.showModal();
    $("[data-ask=no]", d).focus();
  });
}

function toast(msg, ms = 3200) {
  const t = document.createElement("div");
  t.className = "toast"; t.textContent = msg; t.setAttribute("role", "status");
  // Toasts stack, newest at the bottom, rather than covering each other.
  let box = $("#toasts");
  if (!box) { box = document.createElement("div"); box.id = "toasts"; document.body.append(box); }
  box.append(t);
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
function until(t) {
  if (!t) return "";
  const d = new Date(t), s = (d.getTime() - Date.now()) / 1000;
  if (s < 60) return "now";
  if (s < 3600) return `in ${Math.ceil(s / 60)} min`;
  if (s < 86400) return `in ${Math.round(s / 3600)} h`;
  return d.toLocaleString(undefined, { weekday: "short", month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
}
const firstLine = s => (s || "").split("\n").find(l => l.trim()) || "";

const isOwner = () => App.me?.roles?.includes("owner");
const isDev = () => App.me?.roles?.includes("developer");

// Replace a region's markup only when it changed, keeping what the person
// was typing (anything marked data-draft), where the focus was, and which
// sections they opened (details marked data-key).
const drafts = {};
function paint(el, html) {
  if (!el || el._html === html) return false;
  // A card can be shown in several places at once (its conversation, the
  // panel, Needs you), each copy with the same draft key: the copy being
  // typed in is the draft, and only a region that held the focus takes it.
  const active = document.activeElement;
  $$("[data-draft]", el).forEach(i => { if (!(active !== i && active?.dataset?.draft === i.dataset.draft)) drafts[i.dataset.draft] = i.type === "checkbox" ? i.checked : i.value; });
  const opened = new Set($$("details[data-key]", el).filter(d => d.open).map(d => d.dataset.key));
  const focused = el.contains(active) ? active?.dataset?.draft : undefined;
  el.innerHTML = html;
  el._html = html;
  $$("details[data-key]", el).forEach(d => { if (opened.has(d.dataset.key)) d.open = true; });
  $$("[data-draft]", el).forEach(i => { const v = drafts[i.dataset.draft]; if (v === undefined) return; if (i.type === "checkbox") i.checked = !!v; else i.value = v; });
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
  contract_change: "waiting for you to decide on a change to its done-criteria",
  provider: "paused: the model provider is unavailable", usage_limit: "paused: model usage limit", auth: "paused: sign in to the model again",
  kill_switch: "stopped by the kill switch", timer: "waiting until a set time", invalid_request: "paused: the model rejected the request",
};
const chip = st => { const [c, l] = STATUS[st] || ["", st]; return `<span class="chip ${c}">${esc(l)}</span>`; };

// ---------- shell ----------

// One workspace: the conversation in the middle, what was asked of it beside
// it in a panel, and the sidebar. Everything it shows is read together
// (refresh), so a task, a schedule or a decision is one object wherever it
// appears, and acting on it anywhere redraws every place it is shown.
const needsMe = () => App.inbox.filter(i => i.status === "open" && i.can_respond && !(i.details && i.details.notify_at && Date.parse(i.details.notify_at) > Date.now()));

// The workspace's icons, drawn as in the Magnus Workspace design: 24px
// outlines in the current color.
const UI = {
  plus: '<path d="M12 5v14M5 12h14"/>',
  search: '<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>',
  bell: '<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/>',
  folder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
  tasks: '<path d="M9 6h11M9 12h11M9 18h11M4 6h.01M4 12h.01M4 18h.01"/>',
  calendar: '<rect x="3" y="5" width="18" height="16" rx="2"/><path d="M16 3v4M8 3v4M3 10h18"/>',
  file: '<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/>',
  chat: '<path d="M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.6A8 8 0 1 1 21 12z"/>',
  trash: '<path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3"/>',
  gear: '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>',
  done: '<circle cx="12" cy="12" r="9"/><path d="m8.5 12 2.5 2.5 4.5-5"/>',
  need: '<circle cx="12" cy="12" r="9"/><path d="M12 8v5M12 16h.01"/>',
  working: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
  stopped: '<circle cx="12" cy="12" r="9"/><path d="M15 9l-6 6M9 9l6 6"/>',
  globe: '<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18"/>',
  attach: '<path d="m21 11-8.6 8.6a5 5 0 0 1-7-7l8.5-8.6a3.3 3.3 0 0 1 4.7 4.7l-8.6 8.6a1.7 1.7 0 0 1-2.3-2.3l8-8"/>',
  send: '<path d="M12 19V5M5 12l7-7 7 7"/>',
  shield: '<path d="M12 3 4 6v6c0 5 3.5 8 8 9 4.5-1 8-4 8-9V6z"/><path d="m9 12 2 2 4-4"/>',
  close: '<path d="M18 6 6 18M6 6l12 12"/>',
  download: '<path d="M12 4v11M7 10l5 5 5-5M5 20h14"/>',
  chev: '<path d="m9 6 6 6-6 6"/>',
  grid: '<rect x="4" y="4" width="7" height="7" rx="1.5"/><rect x="13" y="4" width="7" height="7" rx="1.5"/><rect x="4" y="13" width="7" height="7" rx="1.5"/><rect x="13" y="13" width="7" height="7" rx="1.5"/>',
};
const icon = (k, cls = "") => `<svg class="ic${cls ? ` ${cls}` : ""}" viewBox="0 0 24 24" aria-hidden="true">${UI[k]}</svg>`;

function nav() {
  const route = location.hash || "#/home";
  const link = (href, label, ic) => `<a href="${href}" class="${route === href || route.startsWith(href + "/") ? "on" : ""}">${ic ? icon(ic) : ""}<span class="ttl">${label}</span></a>`;
  const on = what => Panel.what === what ? " on" : "";
  const waiting = needsMe(), blocking = waiting.filter(i => i.blocking).length;
  const ready = App.update?.ready;
  // A project's conversations sit under it; the rest, each with its icon.
  const conv = (t, nested = true) => `<a href="#/chat/${enc(t.thread)}" class="${nested ? "conv" : ""}${t.thread === App.thread ? " on" : ""}"${t.thread === App.thread ? ` aria-current="page"` : ""} title="${esc(t.title)}">${nested ? "" : icon("chat")}<span class="ttl">${esc(t.title)}</span></a>`;
  const loose = App.threads.filter(t => !t.project);
  const here = App.threads.find(t => t.thread === App.thread);
  const running = App.tasks.filter(t => t.status === "running").length;
  const count = n => n ? `<span class="count">${n}</span>` : "";
  return `
    ${ready ? `<div class="update-ready" title="${esc(ready.notes || "")}"><div class="small">Magnus ${esc(ready.version)} is ready.</div>
      <button class="primary small" data-act="restart-update">Restart to update</button></div>` : ""}
    <button class="needs${waiting.length ? " some" : ""}${on("needs")}" data-open="needs" title="${blocking ? `${blocking} waiting on you now` : "Approvals, questions and reviews"}">${icon("bell")}<span class="ttl">Needs you</span><span class="pill${blocking ? "" : " quiet"}" data-needs-count>${waiting.length}</span></button>
    <div class="sect"><div class="group"><span>Projects</span><a href="#/projects" data-open="projects" class="more${on("projects")}" title="All projects, and a new one">All</a></div>
    ${App.projects.map(p => `<a href="#/project/${enc(p.id)}" data-open="project:${esc(p.id)}" class="${[on(`project:${p.id}`).trim(), here?.project?.id === p.id ? "cur" : ""].filter(Boolean).join(" ")}">${icon("folder")}<span class="ttl">${esc(p.name)}</span></a>
      ${App.threads.filter(t => t.project?.id === p.id).map(t => conv(t)).join("")}`).join("") || `<p class="small faint pad">None yet.</p>`}</div>
    ${loose.length ? `<div class="sect"><div class="group"><span>Conversations</span></div>${loose.map(t => conv(t, false)).join("")}</div>` : ""}
    <div class="sect"><div class="group"><span>Across everything</span></div>
    <div class="views">
      <a href="#/tasks" data-open="tasks" class="${on("tasks").trim()}">${icon("tasks")}<span class="ttl">Tasks</span>${running ? `<span class="count">${running} running</span>` : ""}</a>
      <a href="#/schedules" data-open="schedules" class="${on("schedules").trim()}">${icon("calendar")}<span class="ttl">Schedules</span>${count(App.schedules.length)}</a>
      <a href="#/files" data-open="files" class="${on("files").trim()}">${icon("file")}<span class="ttl">Files</span></a>
      <a href="#/library" class="${route.startsWith("#/library") ? "on" : ""}">${icon("grid")}<span class="ttl">Library</span></a>
      ${link("#/trash", "Trash", "trash")}</div></div>
    <div class="foot">
    ${isDev() ? `<div class="group"><span>Developer</span></div>
      ${link("#/dev/runs", "Runs")}
      ${link("#/dev/experiments", "Experiments")}
      ${link("#/dev/system", "System")}
      ${link("#/dev/policy", "Policy")}
      ${link("#/dev/playbook", "Playbook")}` : ""}
    <div class="who"><span class="avatar" aria-hidden="true">${esc((App.me?.name || "?").trim().charAt(0).toUpperCase())}</span>
      <div class="whoami" data-appliance="${esc(App.appliance)}"><div class="ttl" title="${esc(App.me?.id)}">${esc(App.me?.name)}</div>
      <button class="link" data-act="sign-out">Sign out</button></div>
      <a href="#/settings" class="icon-btn${route.startsWith("#/settings") ? " on" : ""}" title="Settings">${icon("gear")}<span class="sr">Settings</span></a></div></div>`;
}
const sidebar = () => paint($("#side"), nav());

function shell() {
  document.body.innerHTML = `<div class="shell"><nav class="side" aria-label="Sections"><div class="brand"><span class="logo" aria-hidden="true">M</span><span>Magnus</span>
    <a href="#/chat/new" class="icon-btn new-chat" title="New conversation">${icon("plus")}<span class="sr">New conversation</span></a></div>
    <form id="find" role="search">${icon("search")}<label class="sr" for="q">Search conversations, tasks and files</label><input id="q" type="search" placeholder="Search chats, tasks, files" autocomplete="off"></form>
    <div id="side"></div></nav>
    <main id="main" tabindex="-1"></main><aside id="detail" class="detail" aria-label="Details" hidden></aside></div>`;
  $("nav.side").addEventListener("click", async e => {
    if (e.target.closest("[data-act=restart-update]")) return restartToUpdate();
    if (e.target.closest("[data-act=sign-out]")) {
      stop();
      await Magnus.signOut();
      location.hash = "";
      start();
    }
  });
  $("#find").onsubmit = e => { e.preventDefault(); const q = $("#q").value.trim(); if (q) openPanel(`search:${q}`); };
  sidebar();
  wirePanel();
  if (!App.wired) { App.wired = true; wireDocument(); }
}

// What works wherever it is shown, set once for the page.
function wireDocument() {
  // Anything that names an item opens it beside the conversation, without
  // leaving it.
  document.addEventListener("click", e => {
    const o = e.target.closest("[data-open]");
    if (!o || e.button !== 0 || e.ctrlKey || e.metaKey) return;
    e.preventDefault();
    // From Settings, Trash or a developer view: back to the conversation,
    // with it open beside.
    if (!App.workspace) { App.openAfter = o.dataset.open; location.hash = "#/home"; return; }
    openPanel(o.dataset.open);
  });
  // A decision is made from its card wherever the card is shown: in its
  // conversation, beside it, or under Needs you.
  document.addEventListener("click", e => {
    const b = e.target.closest("button[data-item]");
    if (b) respond(b, b.closest("article") || document);
  });
  // A library file downloads from wherever it is listed; a move control
  // moves its item into a project or out, wherever it is shown.
  document.addEventListener("click", async e => {
    const b = e.target.closest("[data-file-download]");
    if (!b) return;
    e.preventDefault();
    try { toast(`Saved ${await Magnus.download(`/v1/files/${enc(b.dataset.fileDownload)}`, b.dataset.name)}`); } catch (err) { toast(err.message); }
  });
  document.addEventListener("click", async e => {
    const b = e.target.closest("[data-verdict], [data-verdict-save]");
    if (!b) return;
    const bar = b.closest("[data-verdict-for]"), id = bar.dataset.verdictFor;
    const verdict = b.dataset.verdict || $("[data-verdict].on", bar)?.dataset.verdict;
    if (!verdict) { toast("Pick right, partly or wrong first."); return; }
    const box = $("textarea", bar);
    try {
      await api("POST", `/v1/tasks/${enc(id)}/verdict`, { verdict, note: box.value });
      delete drafts[box.dataset.draft];
      $$("[data-verdict]", bar).forEach(x => { const on = x.dataset.verdict === verdict; x.classList.toggle("on", on); x.setAttribute("aria-pressed", on); });
      toast(b.dataset.verdictSave !== undefined ? "Note saved." : "Recorded.");
      await refresh();
    } catch (err) { toast(err.message); }
  });
  // Delete, wherever an item offers it: to the trash, where it can be
  // restored until it is gone for good.
  document.addEventListener("click", async e => {
    const b = e.target.closest("[data-trash]");
    if (!b) return;
    if (!await ask(`${b.textContent}? It moves to the trash, where you can restore it until it is gone for good.`, b.textContent)) return;
    b.disabled = true;
    try {
      await api("POST", "/v1/trash", { item: b.dataset.trash });
      toast("Moved to the trash.");
      if (b.closest("#detail")) closePanel();
      if (b.dataset.then) location.hash = b.dataset.then; else await refresh();
    } catch (err) { toast(err.message); b.disabled = false; }
  });
  document.addEventListener("change", async e => {
    const sel = e.target.closest("select[data-move]");
    if (!sel) return;
    try {
      await api("POST", "/v1/moves", { item: sel.dataset.move, project: sel.value || null });
      toast(sel.value ? `Moved to ${sel.selectedOptions[0].textContent}.` : "Taken out of the project.");
      await refresh();
    } catch (err) { toast(err.message); }
  });
}

// A view of the workspace (a conversation) keeps the panel beside it; any
// other view (Settings, Trash, the developer's) has the page to itself.
function route(re, view, workspace = false) { App.routes.push([re, view, workspace]); }

// A Delete button for an item: to the trash, then to `then` (or redrawn).
const trashButton = (item, label, then = "") => `<button data-trash="${esc(item)}"${then ? ` data-then="${esc(then)}"` : ""}>${esc(label)}</button>`;

// ---------- Files and projects ----------

const kb = n => `${Math.max(1, Math.round(n / 1024))} KB`;
// A file wherever it is listed: its name opens it, Download saves it.
function fileChip(f) {
  return `<span class="fchip"><a href="#/file/${enc(f.id)}" data-open="file:${esc(f.id)}">${esc(f.name)}</a> <span class="small faint">${esc(kb(f.size))}</span> <button class="link small" data-file-download="${esc(f.id)}" data-name="${esc(f.name)}" aria-label="Download ${esc(f.name)}">Download</button></span>`;
}
// One tap records the person's verdict on an outcome; a note is optional.
const VERDICTS = [["right", "Right"], ["partly", "Partly"], ["wrong", "Wrong"]];
const ENDED = ["completed", "failed", "cancelled"];
function verdictBar(taskId, v) {
  return `<div class="verdict" data-verdict-for="${esc(taskId)}"><span class="vq">How did Magnus do?</span>
    ${VERDICTS.map(([k, label]) => `<button class="${v?.verdict === k ? "on" : ""}" data-verdict="${k}" aria-pressed="${v?.verdict === k}">${label}</button>`).join("")}
    <details class="vnote" data-key="note:${esc(taskId)}"${v?.note ? " open" : ""}><summary class="small">${v?.note ? "Note" : "Add a note"}</summary>
    <textarea rows="2" aria-label="Note" data-draft="note:${esc(taskId)}">${esc(v?.note || "")}</textarea> <button data-verdict-save>Save note</button></details></div>`;
}
// Where a library file came from, as a link.
const fileFrom = f => f.origin.kind === "task" ? `<a href="#/task/${enc(f.origin.task_id)}" data-open="task:${esc(f.origin.task_id)}">${esc(f.origin.title)}</a>`
  : f.origin.kind === "chat" ? `<a href="#/chat/${enc(f.origin.thread)}">Chat: ${esc(f.origin.title || "conversation")}</a>` : `<span class="faint">Uploaded to Files</span>`;
function readFile(file) {
  return new Promise((ok, no) => {
    const r = new FileReader();
    r.onload = () => ok({ name: file.name, media_type: file.type || "application/octet-stream", data_b64: String(r.result).slice(String(r.result).indexOf(",") + 1) });
    r.onerror = () => no(new Error(`Could not read ${file.name}`));
    r.readAsDataURL(file);
  });
}
// Upload chosen files to the library (into a project, or in a thread).
async function upload(list, where = {}) {
  const out = [];
  for (const file of list) out.push(await api("POST", "/v1/files", { ...(await readFile(file)), ...where }));
  return out;
}
function projectOptions(projects, current, none = "No project") {
  return `<option value="">${esc(none)}</option>${projects.map(p => `<option value="${esc(p.id)}"${p.id === current ? " selected" : ""}>${esc(p.name)}</option>`).join("")}`;
}
// A control that moves an item into one of the person's projects, or out.
function moveControl(item, current, projects, compact = false) {
  return `<label class="small move"><span class="${compact ? "sr" : "muted"}">Project</span> <select data-move="${esc(item)}" aria-label="Project">${projectOptions(projects, current?.id)}</select></label>`;
}
// A file input behind a button; `onFiles` gets the chosen files.
function attachButton(label = "Add files", iconOnly = false) {
  return iconOnly ? `<label class="icon-btn attach" title="${esc(label)}">${icon("attach")}<span class="sr">${esc(label)}</span><input type="file" multiple hidden data-attach></label>`
    : `<label class="button attach">${esc(label)}<input type="file" multiple hidden data-attach></label>`;
}

// A file shown where it is mentioned: a glimpse of it (an image, or the
// first lines of a text file), its name to open it beside the conversation,
// and Download. Glimpses are fetched once, by the app itself.
const GLIMPSE_IMAGE = /^image\/(png|jpeg|gif|webp)$/;
const glimpses = {};
function filePreview(f) {
  const can = (GLIMPSE_IMAGE.test(f.media_type) && f.size <= 4 << 20) || (Compose.TEXT.test(f.media_type) && f.size <= 256 << 10);
  const g = !can ? `<span class="small faint">${esc(f.media_type)}</span>` : glimpses[f.id] ?? `<span class="small faint">Loading a preview…</span>`;
  return `<div class="fprev" data-preview="${esc(f.id)}"${can && !(f.id in glimpses) ? ` data-glimpse="${esc(f.media_type)}"` : ""}>
    <a class="glimpse" href="#/file/${enc(f.id)}" data-open="file:${esc(f.id)}" aria-hidden="true" tabindex="-1">${g}</a>${fileChip(f)}</div>`;
}
async function loadGlimpses(root) {
  const want = $$("[data-glimpse]", root).filter(el => !(el.dataset.preview in glimpses));
  if (!want.length) return;
  for (const el of want) glimpses[el.dataset.preview] = undefined;
  await Promise.all(want.map(async el => {
    const id = el.dataset.preview, type = el.dataset.glimpse;
    try {
      const url = await Magnus.dataUrl(`/v1/files/${enc(id)}`, type);
      glimpses[id] = GLIMPSE_IMAGE.test(type) ? `<img alt="" src="${esc(url)}">` : `<pre>${esc(decodeText(url).split("\n").slice(0, 8).join("\n"))}</pre>`;
    } catch { glimpses[id] = `<span class="small faint">No preview</span>`; }
  }));
  await Promise.all([App.redraw?.(), drawPanel()]);
}

async function go() {
  App.timers.forEach(clearInterval); App.timers = [];
  App.leave.forEach(f => { try { f(); } catch { /* ignore */ } }); App.leave = [];
  const h = location.hash || "#/home";
  // A view still loading when the person moves on stops where it is: only
  // the latest navigation draws. Each view gets a main of its own, so one
  // still loading (and its handlers) only ever touches the main it left,
  // never the next view's.
  const seq = ++App.seq;
  const left = $("#main"), main = left.cloneNode(false);
  left.replaceWith(main);
  for (const [re, view, workspace] of App.routes) {
    const m = h.match(re);
    if (m) {
      App.workspace = workspace;
      if (!workspace) { App.thread = null; closePanel(); }
      sidebar();
      try { await view(main, ...m.slice(1).map(x => x === undefined ? x : decodeURIComponent(x))); }
      catch (e) { if (seq === App.seq) paint(main, `<div class="banner bad">${esc(e.message)}</div>`); }
      return;
    }
  }
  location.hash = "#/home";
}

// ---------- sign-in ----------

async function signInScreen(note) {
  document.body.innerHTML = `<div class="signin"><h1>Magnus</h1><p class="muted">Sign in to your appliance.</p><div class="card" id="si"></div></div>`;
  const box = $("#si");
  // Signed in. Pair this computer for the public name once, on the private
  // address; a failure is said and sign-in still stands.
  async function enter(person, data) {
    if (data.person) localStorage.setItem("magnus.person", data.person);
    if (Magnus.pairDevice && data.public_url) {
      try { await Magnus.pairDevice(data.public_url); }
      catch (err) { return ask("paired", { ...data, error: err.message || String(err), who: person }); }
    }
    App.me = person;
    App.appliance = data.url || location.origin;
    boot();
  }
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
          else if (c.status === "needs_device") ask("needs_device", { url });
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
    } else if (stage === "needs_device") {
      paint(box, `<h3>This computer is not paired</h3>
        <p class="small">magnus.pyrallon.ai accepts only a computer that was paired with the appliance. Sign in once on the appliance's private address, the one on your network or Tailscale. This computer is paired then, and this address works after that, from anywhere.</p>
        <div class="row" style="margin-top:14px"><button id="back">Use a private address</button></div>`);
      $("#back").onclick = () => ask("address", {});
    } else if (stage === "paired") {
      paint(box, `<h3>This computer was not paired</h3>
        <p class="small" style="color:var(--bad)" role="alert">${esc(data.error)}</p>
        <p class="small muted">You are signed in on this network. ${esc(data.public_url || "")} will not accept this computer until pairing works.</p>
        <div class="row" style="margin-top:14px"><button class="primary" id="go">Continue</button></div>`);
      $("#go").onclick = () => { App.me = data.who; App.appliance = data.url || location.origin; boot(); };
    } else if (stage === "method") {
      paint(box, `<p class="small muted">${esc(data.url || location.origin)}</p><p class="small muted">Connecting…</p>`);
      try {
        const m = await Magnus.signInMethod();
        ask("password", { ...data, public_url: m.public_url, label: m.method === "oidc" ? m.label : undefined });
      } catch (err) { note = err.message || String(err); ask(Magnus.kind === "app" ? "address" : "password", data); }
    } else if (stage === "browser") {
      // The provider's own page in the person's browser: for a passkey, the
      // first time, or when their account needs finishing there.
      const back = () => ask("password", { ...data, message: undefined });
      if (Magnus.providerSignIn) {
        paint(box, `<p class="small muted">${esc(data.url || location.origin)}</p>
          <p>${esc(data.message || `Sign in on the ${data.label} page that just opened in your browser, then come back here.`)}</p>
          <p class="small muted" id="wait" role="status">Waiting for you to sign in…</p>
          <div class="row" style="margin-top:14px"><button class="link" id="cancel">Cancel</button></div>`);
        // A cancelled sign-in stops waiting; one the browser still finishes is not kept.
        let cancelled = false;
        $("#cancel").onclick = () => { cancelled = true; Magnus.providerSignInCancel?.().catch(() => {}); back(); };
        try {
          const r = await Magnus.providerSignIn(data.person);
          if (cancelled) return;
          return enter(r.person, data);
        } catch (err) { if (!cancelled) ask("password", { ...data, message: undefined, error: err.message || String(err) }); return; }
      }
      let s;
      try { s = await Magnus.deviceStart(); } catch (err) { return ask("password", { ...data, error: err.message || String(err) }); }
      const link = s.verification_uri_complete || s.verification_uri;
      let stop = false;
      paint(box, `<p class="small muted">${esc(data.url || location.origin)}</p>
        <p>${esc(data.message || `Sign in on the ${s.label} page that just opened.`)}</p>
        <p class="small">Check that it shows this code:</p><div class="fp" id="code">${esc(s.user_code)}</div>
        <div class="row" style="margin-top:14px"><button data-url="${esc(link)}" id="open">Open the sign-in page again</button><button class="link" id="cancel">Cancel</button></div>
        <p class="small muted" id="wait" role="status">Waiting for you to approve…</p>`);
      $("#open").onclick = () => Magnus.openExternal(link).catch(() => {});
      $("#cancel").onclick = () => { stop = true; back(); };
      Magnus.openExternal(link).catch(() => {});
      const until = Date.now() + s.expires_in * 1000;
      const every = Math.max(1, s.interval) * 1000;
      while (!stop && Date.now() < until) {
        await new Promise(r => setTimeout(r, every));
        if (stop) return;
        try {
          const r = await Magnus.devicePoll(s.flow);
          if (r.pending) continue;
          return enter(r.person, data);
        } catch (err) { return ask("password", { ...data, message: undefined, error: err.message || String(err) }); }
      }
      if (!stop) ask("password", { ...data, message: undefined, error: "The sign-in code expired. Start again." });
    } else {
      // Username and password, then the code from an authenticator app if
      // the person uses one. The password stays in this closure only.
      const coding = stage === "code" || stage === "machine";
      const machine = stage === "machine";
      paint(box, `<form id="f"><p class="small muted">${esc(data.url || location.origin)}</p>
        ${data.error ? `<p class="small" style="color:var(--bad)" role="alert">${esc(data.error)}</p>` : ""}
        ${machine
          ? `<label for="otp">Enter the code \`magnus sign-in-code\` printed on the appliance</label><input id="otp" style="width:100%" autocomplete="one-time-code" maxlength="16" required autofocus>`
          : coding
          ? `<label for="otp">Enter the 6-digit code from your authenticator app</label><input id="otp" style="width:100%" inputmode="numeric" autocomplete="one-time-code" pattern="[0-9]*" maxlength="6" required autofocus>`
          : `<label for="who">Username</label><input id="who" style="width:100%" autocomplete="username" value="${esc(data.person || localStorage.getItem("magnus.person") || "")}" required>
        <label for="pw">Password</label><input id="pw" type="password" style="width:100%" autocomplete="current-password" required>`}
        <div class="row" style="margin-top:16px"><button class="primary" id="go">${coding ? "Continue" : "Sign in"}</button>${coding ? `<button type="button" class="link" id="again">Back</button>` : data.label ? `<button type="button" class="link" id="passkey">Use a passkey instead</button>` : ""}${Magnus.kind === "app" && !coding ? `<button type="button" class="link" id="back">Other appliance</button>` : ""}</div></form>`);
      $("#back")?.addEventListener("click", () => ask("address", data));
      $("#again")?.addEventListener("click", () => ask("password", { ...data, error: undefined, password: undefined }));
      $("#passkey")?.addEventListener("click", () => ask("browser", { ...data, person: $("#who").value.trim() || undefined, error: undefined }));
      $("#f").onsubmit = async e => {
        e.preventDefault();
        const person = coding ? data.person : $("#who").value.trim();
        const password = coding ? data.password : $("#pw").value;
        $("#go").disabled = true;
        try {
          const typed = coding ? $("#otp").value.trim() : undefined;
          const r = await Magnus.signIn(person, password, machine ? undefined : typed, machine ? typed : undefined);
          if (r?.machine_code_needed) return ask("machine", { ...data, person, password, error: r.error });
          if (r?.code_needed) return ask("code", { ...data, person, password, error: coding ? r.error : undefined });
          if (r?.browser) return ask("browser", { ...data, person, password: undefined, message: r.error });
          return enter(r, { ...data, person });
        } catch (err) { ask("password", { ...data, person, password: undefined, error: err.message || String(err) }); }
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
    const items = await Resources.read("Needs you", "/v1/inbox", "items");
    if (!items) return;
    App.inbox = items;
    // Interrupt only for decisions a task is waiting on; the rest wait in
    // the inbox until the person looks (U3).
    const told = new Set(load("told", []));
    const wants = i => i.blocking ? App.prefs?.notify_blocking !== false : App.prefs?.notify_batched === true;
    // Each item is considered once, so turning a kind on later does not
    // bring back everything already waiting.
    const fresh = items.filter(i => i.status === "open" && i.can_respond && !told.has(i.id));
    for (const i of fresh.filter(wants)) Magnus.notify(i.blocking ? "A task needs you" : "Something for you", i.summary);
    if (fresh.length) save("told", [...told, ...fresh.map(i => i.id)].slice(-500));
    sidebar();
  } catch (e) {
    // A dead session must land on sign-in once. Calling start() again
    // restores the same session and boots forever.
    if (e.status === 401 && App.me) { stop(); signInScreen("Your session ended. Sign in again."); }
  }
}

async function pollVersions() {
  try {
    const v = await Resources.read("Agent changes", "/v1/versions");
    if (!v) return;
    App.versions = v;
    const seen = load("version", null);
    if (seen === null) { save("version", v.latest_seq ?? 0); return; }
    const news = v.changes.filter(c => c.seq > seen);
    const told = load("version-told", 0);
    if (news.length && (v.latest_seq ?? 0) > told) {
      if (App.prefs?.notify_agent_changes !== false) Magnus.notify("The agent changed", news.map(describeChange).join("; "));
      save("version-told", v.latest_seq);
    }
  } catch { /* shown above the conversation */ }
}

function describeChange(c) {
  const what = { harness: "harness", instructions: "instructions", model: "model", playbook: "playbook" }[c.what] || c.what.replace("policy:", "policy for ");
  return `${what} is now ${c.value}`;
}

// ---------- the workspace's objects ----------

// Tasks, schedules, conversations and projects, read together with the
// inbox; then every view that shows them is drawn again from what was read.
async function refresh() {
  const get = (path, k) => Resources.read(({tasks:"Tasks",schedules:"Schedules",threads:"Conversations",projects:"Projects"})[k],path,k);
  const [tasks, schedules, threads, projects, health] = await Promise.all([get("/v1/tasks", "tasks"), get("/v1/schedules", "schedules"), get("/v1/chat", "threads"), get("/v1/projects", "projects"), Resources.read("Appliance", "/v1/health"), pollInbox()]);
  if (!App.me) return;
  // Magnus's own background reviews are plumbing, not the person's tasks
  // (owner, 2026-11-07): developers see them in the run inspector.
  if (tasks) App.tasks = tasks.filter(t => !t.review);
  if (schedules) App.schedules = schedules;
  if (threads) App.threads = threads;
  if (projects) App.projects = projects;
  if (health) App.health = health;
  sidebar();
  banners();
  await Promise.all([App.redraw?.(), drawPanel()]);
}

// Above the conversation: the kill switch when it is on, and how the agent
// changed since the person last looked.
function banners() {
  const el = $("#banners");
  if (!el) return;
  const v = App.versions, news = v ? v.changes.filter(c => c.seq > load("version", 0)) : [];
  paint(el, `${App.health?.kill_switch ? `<div class="banner bad"><strong>The kill switch is on.</strong> No task acts until the owner turns it off.</div>` : ""}
    ${news.length ? `<div class="banner" role="status"><strong>The agent changed since you last looked.</strong>
      <ul class="small" style="margin:6px 0">${news.map(c => `<li>${esc(describeChange(c))} <span class="faint">(${esc(when(c.at))})</span></li>`).join("")}</ul>
      <button data-act="seen-versions">Got it</button></div>` : ""}`);
}

// The owner's kill switch (B11): one control that refuses every action,
// suspends every task and revokes the vault's sign-ins and keys; after it,
// what was revoked and what to connect again.
function killSwitch(ks) {
  const revoked = Object.entries(ks.revoked || {});
  const state = r => r.at_provider === null || r.at_provider === undefined ? "revoking…" : r.at_provider ? "revoked by the provider" : `removed from the vault; the provider did not confirm (${r.detail || "no answer"})`;
  return `<div class="card" data-kill-switch="${ks.engaged ? "on" : "off"}" style="margin-top:24px"><h2 style="margin-top:0">Kill switch</h2>
    ${ks.engaged
      ? `<p><strong>On.</strong> Every action is refused and every task is suspended. Nothing resumes when you turn it off; resume each task yourself.</p>
         <button data-kill="off">Turn the kill switch off</button>`
      : `<p class="small muted">Refuses every action, suspends every task, revokes every sign-in at its provider and seals the vault, at once.</p>
         <button class="danger" data-kill="on">Stop everything</button>`}
    ${(ks.sealed || []).length ? `<h3>Sealed until you turn it off</h3><p class="small muted">API keys and app secrets cannot be revoked from here; if you think one leaked, revoke it in that provider's console.</p>
      <ul class="small" data-sealed>${ks.sealed.map(n => `<li class="mono" data-secret="${esc(n)}">${esc(n)}</li>`).join("")}</ul>` : ""}
    ${revoked.length ? `<h3>Revoked${ks.engaged ? "" : ": connect these again"}</h3><table class="t" data-revoked><tbody>${revoked.map(([n, r]) => `<tr data-secret="${esc(n)}"><td class="mono small">${esc(r.adapter ? `${r.adapter} sign-in` : n)}</td><td class="small">${esc(state(r))}</td></tr>`).join("")}</tbody></table>
      ${ks.engaged ? "" : `<p class="small"><a href="#/settings/models">Models</a>: sign in or enter keys again.</p>`}` : ""}
  </div>`;
}

// Where a task stands for the person looking: waiting on them, working,
// waiting on something else (a set time), done, or failed or cancelled.
const STATE = t => t.needs_you ? "needs" : t.status === "running" ? "running" : t.status === "suspended" ? "waiting" : t.status === "completed" ? "done" : "failed";
const SECTIONS = [["needs", "Needs you"], ["running", "Running"], ["waiting", "Waiting"], ["done", "Done"], ["failed", "Failed"]];

// Tasks grouped by the request that started them: a chat message or a
// schedule; a task started on its own is a group of one. A group sits in the
// section of its most pressing task.
function groupTasks(tasks) {
  const by = new Map();
  for (const t of tasks) {
    const key = t.group ? `${t.group.kind}:${t.group.id}` : `task:${t.task_id}`;
    if (!by.has(key)) by.set(key, { key, group: t.group, tasks: [] });
    by.get(key).tasks.push(t);
  }
  const groups = [...by.values()];
  for (const g of groups) {
    g.tasks.sort((a, b) => (a.created_at || "").localeCompare(b.created_at || ""));
    g.count = Object.fromEntries(SECTIONS.map(([s]) => [s, g.tasks.filter(t => STATE(t) === s).length]));
    g.section = ["needs", "running", "waiting", "failed"].find(s => g.count[s]) || "done";
    g.latest = g.tasks.map(t => t.ended_at || t.created_at || "").sort().pop();
  }
  groups.sort((a, b) => b.latest.localeCompare(a.latest));
  return SECTIONS.map(([state, label]) => ({ state, label, groups: groups.filter(g => g.section === state) })).filter(x => x.groups.length);
}

function progress(g) {
  const c = g.count, cancelled = g.tasks.filter(t => t.status === "cancelled").length;
  const parts = [`${c.done} of ${g.tasks.length} done`];
  if (c.needs) parts.push(`${c.needs} need${c.needs === 1 ? "s" : ""} you`);
  if (c.running) parts.push(`${c.running} running`);
  if (c.waiting) parts.push(`${c.waiting} waiting`);
  if (c.failed - cancelled) parts.push(`${c.failed - cancelled} failed`);
  if (cancelled) parts.push(`${cancelled} cancelled`);
  return parts.join(" · ");
}

function taskRow(t) {
  const why = t.status === "suspended" ? WAITING[t.waiting_on] || (t.needs_you ? "waiting for you" : "") : "";
  return `<li class="trow" data-state="${STATE(t)}" data-task="${esc(t.task_id)}"><a href="#/task/${enc(t.task_id)}" data-open="task:${esc(t.task_id)}"><span class="ttl" title="${esc(t.text)}">${esc(t.title)}</span>${chip(t.status)}<span class="small muted why">${esc(why)}</span><span class="small faint when">${esc(when(t.ended_at || t.created_at))}</span></a></li>`;
}

// A group: its request and progress, the tasks that need the person in
// view, the rest folded away.
function groupBlock(g, section) {
  if (!g.group && g.tasks.length === 1) return `<ul class="tlist">${taskRow(g.tasks[0])}</ul>`;
  const href = g.group.kind === "chat" ? `href="#/chat/${enc(g.group.thread)}"` : `href="#/schedules" data-open="schedule:${esc(g.group.id)}"`;
  const shown = section === "needs" ? g.tasks.filter(t => t.needs_you) : [];
  const rest = g.tasks.filter(t => !shown.includes(t));
  return `<div class="tgroup" data-group="${esc(g.key)}"><div class="ghead"><a class="ttl" ${href} title="${esc(g.group.title)}">${g.group.kind === "schedule" ? "Schedule: " : ""}${esc(g.group.title)}</a><span class="small muted">${esc(progress(g))}</span></div>
    ${shown.length ? `<ul class="tlist">${shown.map(taskRow).join("")}</ul>` : ""}
    ${rest.length ? `<details data-key="${esc(g.key)}"><summary class="small muted">${shown.length ? `The other ${rest.length}` : `${rest.length} task${rest.length === 1 ? "" : "s"}`}</summary><ul class="tlist">${rest.map(taskRow).join("")}</ul></details>` : ""}</div>`;
}

// Tasks, as a view over the same tasks the conversations show: grouped by
// the request that started them, in sections by where they stand.
const MORE = new Set();
function tasksView() {
  const LIMIT = { done: 15, failed: 15 };
  const sections = groupTasks(App.tasks);
  return sections.length ? sections.map(sec => {
    const n = sec.groups.reduce((k, g) => k + g.count[sec.state], 0);
    const cap = LIMIT[sec.state] && !MORE.has(sec.state) ? LIMIT[sec.state] : Infinity;
    const left = sec.groups.length - Math.min(cap, sec.groups.length);
    return `<section class="tsec" data-section="${sec.state}"><h3>${esc(sec.label)} <span class="count">${n}</span></h3>
      ${sec.groups.slice(0, cap).map(g => groupBlock(g, sec.state)).join("")}
      ${left ? `<button class="link small" data-more="${sec.state}">Show ${left} more</button>` : ""}</section>`;
  }).join("") : offered("No tasks yet. Ask for something in a conversation.");
}

// An empty Tasks or Schedules: what Magnus offered to take over when it
// opened a conversation itself (at the first sign-in, or for a newly
// connected app) and the person has not answered, each linking to its
// conversation, in Magnus's voice. With none, the plain line.
function offered(none) {
  const cards = App.threads.filter(t => t.opener).flatMap(t => (t.open_cards || []).map(c => ({ ...c, thread: t.thread })));
  if (!cards.length) return `<p class="empty">${esc(none)}</p>`;
  return `<div class="empty" data-offered><p>Nothing here yet. I had a look around, and these are mine to take off your hands as soon as you say so:</p>
    <ul class="tlist">${cards.map(c => `<li class="trow" data-offer="${esc(c.card)}"><a href="#/chat/${enc(c.thread)}"><span class="ttl">${esc(c.title)}</span></a></li>`).join("")}</ul></div>`;
}

// A schedule's next run and its last result, in one line.
function scheduleState(sc) {
  const last = sc.last;
  return `${(sc.paused || sc.ended) ? "" : `Next run ${esc(until(sc.next))} · `}${sc.last_failed ? `Last firing <span class="chip bad">could not start</span> ${esc(sc.last_failed.error)} <span class="faint">${esc(when(sc.last_failed.at))}</span>`
    : last ? `Last result: ${chip(last.status)} <a href="#/task/${enc(last.task_id)}" data-open="task:${esc(last.task_id)}">${esc(firstLine(last.summary) || last.title)}</a> <span class="faint">${esc(when(last.ended_at || last.created_at))}</span>` : "No results yet"}`;
}

// When a schedule ends, what it may spend a run, and why it is paused or
// skipped a run: the same lines in the list and in the schedule itself.
function scheduleFacts(sc) {
  return `${sc.ended ? `<div class="small" data-ended><span class="chip">Ended</span> ${esc(sc.ended)}.</div>` : ""}
    <div class="small muted" data-ends>${sc.until || sc.max_runs ? `Ends: ${esc(ends(sc))}` : "Runs until you stop it"}${sc.max_runs ? ` · ${esc(sc.started)} of ${esc(sc.max_runs)} runs so far` : ""}${sc.budget ? ` · up to $${esc(sc.budget.max_usd)} a run` : ""}</div>
    ${sc.paused ? `<div class="small" data-paused><span class="chip bad">Paused</span> ${esc(sc.paused)}. ${sc.paused_on ? `It goes on once you answer that run (<a href="#/task/${enc(sc.paused_on)}" data-open="task:${esc(sc.paused_on)}">open it</a>).` : "Nothing runs until you resume it."}</div>` : ""}
    ${sc.skipped ? `<div class="small muted" data-skipped>Skipped a run ${esc(when(sc.skipped.at))}: ${esc(sc.skipped.why)} (<a href="#/task/${enc(sc.skipped.task)}" data-open="task:${esc(sc.skipped.task)}">open it</a>)</div>` : ""}`;
}
// Resume, for a schedule paused until the person says so (one paused on a
// run waiting for them goes on once they answer it), and Stop schedule.
// An ended schedule stays until its person deletes it (#264): Delete
// schedule is its one control (#452).
function scheduleButtons(sc, stop = "Stop schedule") {
  if (sc.ended) return `<button data-act="delete-schedule" data-id="${esc(sc.id)}">Delete schedule</button>`;
  return `${sc.paused && !sc.paused_on ? `<button data-act="resume-schedule" data-id="${esc(sc.id)}">Resume</button>` : ""}<button data-act="stop-schedule" data-id="${esc(sc.id)}">${stop}</button>`;
}

// Schedules, soonest first, ended last: each opens beside the conversation.
function schedulesView() {
  const key = sc => `${sc.ended ? "1" : "0"}${sc.next || ""}`;
  const list = [...App.schedules].sort((a, b) => key(a).localeCompare(key(b)));
  return list.length ? list.map(sc => `<div class="card flat sched" data-schedule="${esc(sc.id)}">
      <div class="spread"><a class="ttl" href="#/schedules" data-open="schedule:${esc(sc.id)}" title="${esc(sc.text)}">${esc(sc.title)}</a><span class="small muted">${esc(rule(sc))} ${scheduleButtons(sc)}</span></div>
      ${scheduleFacts(sc)}
      <div class="small muted" data-last>${scheduleState(sc)}</div>
      ${sc.runs.some(r => r.needs_you) ? `<ul class="tlist">${sc.runs.filter(r => r.needs_you).map(taskRow).join("")}</ul>` : ""}</div>`).join("")
    : offered("No schedules yet. Ask in a conversation for something done every day or every week.");
}

// One schedule: what it does, when, how its runs went, and a way to stop it.
function scheduleView(id) {
  const sc = App.schedules.find(x => x.id === id);
  if (!sc) return `<p class="empty">This schedule was stopped.</p>`;
  return `<h2 class="ptitle">${esc(sc.title)}</h2><p class="small muted">${esc(rule(sc))}</p>
    <div class="card flat sched" data-schedule="${esc(sc.id)}"><p>${esc(sc.text)}</p>${scheduleFacts(sc)}<div class="small" data-last>${scheduleState(sc)}</div></div>
    <div class="row">${moveControl(sc.id, sc.project, App.projects)}${scheduleButtons(sc, "Stop this schedule")}</div>
    <h3>Runs</h3>${sc.runs.length ? `<ul class="tlist">${sc.runs.map(taskRow).join("")}</ul>` : `<p class="empty">None yet.</p>`}
    ${sc.failures?.length ? `<h3>Firings that could not start</h3><ul class="small">${sc.failures.map(f => `<li>${esc(when(f.at))}: ${esc(f.error)}</li>`).join("")}</ul>` : ""}`;
}

// ---------- Model choice ----------

// The models a person may run a task or a message on, as the adapters'
// providers list them now. The choice is kept per viewer; the appliance's
// default is used until they pick another.
async function modelPicker() {
  const { default: dflt, adapters } = await api("GET", "/v1/models").catch(() => ({ default: null, adapters: [] }));
  const key = m => JSON.stringify([m.adapter, m.model]);
  const opts = adapters.flatMap(a => a.models.map(model => ({ adapter: a.adapter, model })));
  if (dflt && !opts.some(o => key(o) === key(dflt))) opts.unshift(dflt);
  const saved = load("model-choice", null);
  const pick = opts.find(o => saved && key(o) === key(saved)) || dflt || opts[0];
  const groups = [...new Set(opts.map(o => o.adapter))];
  return `<label class="model"><span class="sr">Model</span><select data-model aria-label="Model">${groups.map(g => `<optgroup label="${esc(g)}">${opts.filter(o => o.adapter === g).map(o =>
    `<option value="${esc(key(o))}"${pick && key(o) === key(pick) ? " selected" : ""}>${esc(o.model)}${dflt && key(o) === key(dflt) ? " (default)" : ""}</option>`).join("")}</optgroup>`).join("")}</select></label>`;
}
function chosenModel(root) {
  const sel = $("select[data-model]", root);
  if (!sel || !sel.value) return undefined;
  const [adapter, model] = JSON.parse(sel.value);
  save("model-choice", { adapter, model });
  return { adapter, model };
}

// ---------- Chat ----------

// A message is answered, or becomes a task or a schedule to confirm: the
// model decides. A conversation goes on until the person starts a new one;
// opening Magnus returns to the latest. What it starts shows in the thread
// as it goes: tasks with their status, the decisions they wait on (the same
// cards as under Needs you), schedules, results and their files.
function composer(placeholder, picker) {
  return `<form id="say" class="say"><label for="msg" class="sr">Message</label><textarea id="msg" rows="1" data-draft="chat-${esc(location.hash)}" placeholder="${esc(placeholder)}" required></textarea>
    <div class="row small" id="pending"></div>
    <div class="say-bar">${picker}<span class="grow"></span>${attachButton("Attach files", true)}<button class="send" id="send" title="Send">${icon("send")}<span class="sr">Send</span></button></div></form>`;
}
// Files added to the next message: uploaded at once, sent with it.
function wireComposer(thread, project) {
  let files = [];
  const shown = () => { $("#pending").innerHTML = files.map(fileChip).join(" "); };
  $("#say [data-attach]").onchange = async e => {
    try { files.push(...await upload(e.target.files, thread ? { thread } : project ? { project } : {})); } catch (err) { toast(err.message); }
    e.target.value = "";
    shown();
  };
  const box = $("#msg");
  box.focus();
  box.onkeydown = e => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); $("#say").requestSubmit(); } };
  // The box grows with the message, up to a third of the window.
  const fit = () => { box.style.height = "auto"; box.style.height = `${Math.min(box.scrollHeight, innerHeight / 3)}px`; };
  box.oninput = fit;
  fit();
  $("#say").onsubmit = async e => {
    e.preventDefault();
    const text = box.value.trim();
    if (!text) return;
    $("#send").disabled = true;
    try {
      const body = { text, model: chosenModel($("#say")), files: files.map(f => f.id) };
      if (thread) body.thread = thread;
      else if (project) body.project = project;
      const r = await api("POST", "/v1/chat", body);
      delete drafts[`chat-${location.hash}`];
      box.value = "";
      fit();
      files = [];
      shown();
      // A new conversation becomes the one shown, in place.
      if (!thread) { history.replaceState(null, "", `#/chat/${enc(r.thread)}`); go(); }
      else await refresh();
    } catch (err) { toast(err.message); }
    if ($("#send")) $("#send").disabled = false;
  };
}

// Lays out a conversation; false when the person moved on while it loaded.
async function chatLayout(main, thread, project) {
  const seq = App.seq;
  App.thread = thread;
  sidebar();
  paint(main, `<section class="conv"><header class="conv-head" id="conv-head"></header><div id="banners"></div><div id="thread" class="thread" aria-live="polite"></div><div id="composer"></div></section>`);
  convHead(thread, project);
  banners();
  $("#banners", main).onclick = e => { if (e.target.closest("[data-act=seen-versions]")) { save("version", App.versions?.latest_seq ?? 0); banners(); } };
  const picker = await modelPicker();
  if (seq !== App.seq) return false;
  paint($("#composer", main), composer(thread ? "Reply, or give Magnus more to do" : "Ask anything, or give Magnus work to do", picker));
  wireComposer(thread, project);
  return true;
}

// Above the conversation: its project (a link to it) and its title, and
// what can be done to the whole conversation.
function convHead(thread, project) {
  const el = $("#conv-head");
  if (!el) return;
  const here = thread && App.threads.find(t => t.thread === thread);
  const p = here ? here.project : project && App.projects.find(x => x.id === project);
  paint(el, `<div class="conv-title"><div class="small muted">${p ? `<a href="#/project/${enc(p.id)}" data-open="project:${esc(p.id)}">${esc(p.name)}</a>` : thread ? "No project" : ""}</div>
    <h1 title="${esc(here?.title || "")}">${esc(here?.title || "New conversation")}</h1></div>
    ${thread ? `<div class="row">${moveControl(thread, here?.project, App.projects, true)}<button class="icon-btn" data-trash="${esc(thread)}" data-then="#/home" title="Delete conversation">${icon("trash")}<span class="sr">Delete conversation</span></button></div>` : ""}`);
}

async function showNewChat(main, project) {
  if (!await chatLayout(main, null, project)) return;
  paint($("#thread", main), `<p class="muted hello">Ask a question, or say what you want done. Magnus answers, starts a task, or proposes a schedule for you to confirm. What it starts shows here as it goes.</p>`);
}

async function showThread(main, thread) {
  if (!await chatLayout(main, thread)) return;
  // An answer being written shows as it streams; `live` holds its text so far.
  const ui = { editing: null, sharing: null, declining: null, people: [], live: {} };
  const listening = {};
  const listen = entries => {
    const running = new Set(entries.filter(e => e.kind === "turn" && e.status === "running").map(e => e.task_id));
    for (const id of running) if (!listening[id]) listening[id] = Magnus.live(id, t => {
      ui.live[id] = (ui.live[id] || "") + t;
      const el = $(`[data-live="${CSS.escape(id)}"]`, main);
      if (el) el.textContent = ui.live[id];
    });
    for (const id of Object.keys(listening)) if (!running.has(id)) { listening[id](); delete listening[id]; delete ui.live[id]; }
  };
  let shown = [], drawRevision=0;
  onLeave(()=>Resources.forget("Conversation"));
  const draw = async () => {
    const revision=++drawRevision;
    const entries = await Resources.read("Conversation", `/v1/chat/${enc(thread)}`, "entries");
    if (!entries || revision!==drawRevision) return;
    if (App.thread !== thread) return;
    shown = entries;
    // A decision the thread now waits on, not read yet: read the inbox first.
    const waits = entries.flatMap(e => e.kind === "tasks" ? e.tasks.map(t => t.waiting_item) : [e.waiting_item]).filter(Boolean);
    if (waits.some(id => !App.inbox.some(i => i.id === id))) await pollInbox();
    convHead(thread);
    ui.composed = [];
    // A result that comes after the person has moved on (a message of theirs
    // since the task started) quotes what they asked.
    ui.movedOn = new Set();
    const startedAt = {};
    let lastMessage = -1;
    entries.forEach((e, i) => {
      if (e.kind === "message") lastMessage = i;
      if (e.kind === "tasks") for (const t of e.tasks) startedAt[t.task_id] = i;
      if (e.kind === "result" && startedAt[e.task_id] !== undefined && lastMessage > startedAt[e.task_id]) ui.movedOn.add(e.task_id);
    });
    const body = entries.map(e => `<div class="entry ${e.kind === "message" ? "me" : "them"}" data-entry="${esc(e.id || "")}" data-kind="${esc(e.kind)}">${entry(e, ui)}${msgActions(e, ui)}</div>`).join("");
    const el = $("#thread", main);
    if (!el) return;
    const grew = paint(el, body);
    bindConnectorActs(el);
    keepHeights(el);
    motion(el, ui);
    for (const e of entries.filter(e => e.view)) fill($(`iframe[data-shown="${CSS.escape(e.id)}"]`, main), e.view, e.task_id);
    for (const [id, view, , task] of ui.composed) fill($(`iframe[data-view="${CSS.escape(id)}"]`, el), view, task);
    listen(entries);
    loadGlimpses(el);
    if (grew && !ui.editing && !ui.sharing && !el.contains(document.activeElement)) el.lastElementChild?.scrollIntoView({ block: "end" });
  };
  const path = (id, act) => `/v1/chat/${enc(thread)}/entries/${enc(id)}/${act}`;
  const msgAct = async (b, el) => {
    const id = el.dataset.entry, e = shown.find(x => x.id === id), act = b.dataset.msgAct;
    if (!e) return;
    switch (act) {
      case "copy": await copyText(entryText(e)); return toast("Copied.");
      case "edit": drafts[`edit-${id}`] = e.text; ui.editing = id; return draw();
      case "edit-cancel": ui.editing = null; return draw();
      case "edit-send": {
        const text = $(`#edit-${CSS.escape(id)}`, main).value.trim();
        if (!text) return toast("Write the message first.");
        b.disabled = true; await api("POST", path(id, "resend"), { text }); ui.editing = null; delete drafts[`edit-${id}`]; return draw();
      }
      case "retry": b.disabled = true; await api("POST", path(id, "resend"), {}); return draw();
      case "branch": { const r = await api("POST", path(id, "branch"), {}); toast("Branched into a new chat."); location.hash = `#/chat/${enc(r.thread)}`; return; }
      case "share": ui.people = (await api("GET", "/v1/directory")).people; if (!ui.people.length) return toast("Nobody else is on this appliance."); ui.sharing = id; return draw();
      case "share-cancel": ui.sharing = null; return draw();
      case "share-send": {
        const to = $("[data-share-to]", el).value;
        b.disabled = true; await api("POST", path(id, "share"), { person: to }); ui.sharing = null;
        toast(`Shared with ${ui.people.find(p => p.id === to)?.name || to}.`); return draw();
      }
      case "export": return toast(`Saved ${await Magnus.download(path(id, "export"), safeName(e))}`);
      case "save": await api("POST", path(id, "save"), {}); return toast("Saved to Files.");
      case "listen": { b.disabled = true; try { await readAloud((await api("POST", path(id, "spoken"), {})).script); } finally { b.disabled = false; } return; }
      case "hush": return hush();
      case "great": case "annoying": b.disabled = true; await api("POST", path(id, "reaction"), { reaction: e.reaction === act ? null : act }); return draw();
      case "task": { const r = await api("POST", path(id, "task"), {}); toast("Started a task."); await refresh(); return r; }
      case "delete":
        if (!await ask("Delete this message? It leaves this chat and what Magnus reads next, and waits in the trash until it is gone for good.", "Delete")) return;
        await api("POST", path(id, "delete"), {}); return draw();
    }
  };
  App.redraw = draw;
  onLeave(() => { App.redraw = null; listen([]); hush(); });
  $("#thread", main).onclick = async e => {
    const m = e.target.closest("[data-msg-act]");
    if (m) { try { await msgAct(m, m.closest("[data-entry]")); } catch (err) { toast(err.message); m.disabled = false; } return; }
    if (e.target.closest("[data-act=change-schedule]")) {
      const box = $("#msg");
      box.value = "Change the schedule: ";
      box.focus();
      box.setSelectionRange(box.value.length, box.value.length);
      return;
    }
    // The live cards' buttons act directly, not through Magnus.
    const ta = e.target.closest("[data-task-act]"), sa = e.target.closest("[data-sched-act]");
    if (ta || sa) {
      e.preventDefault();
      const el = ta || sa;
      el.disabled = true;
      try {
        if (ta) {
          if (!await ask("Cancel this task? What it already did stays done.", "Cancel task")) { el.disabled = false; return; }
          await api("POST", `/v1/tasks/${enc(ta.dataset.task)}/cancel`, {}); toast("Cancelled.");
        } else {
          const id = enc(sa.dataset.scheduleId), k = sa.dataset.schedAct;
          if (k === "cancel") {
            const ended = App.schedules.find(x => x.id === sa.dataset.scheduleId)?.ended;
            if (!await ask(ended ? "Delete this schedule? It has ended, so nothing stops; its runs stay in Tasks." : "Cancel this schedule? No more of its runs start, and a run still going stops.", ended ? "Delete schedule" : "Cancel schedule")) { el.disabled = false; return; }
            await api("DELETE", `/v1/schedules/${id}`); toast(ended ? "The schedule is deleted." : "The schedule is cancelled.");
          } else { await api("POST", `/v1/schedules/${id}/${k}`, {}); toast(k === "pause" ? "Paused." : "Resumed."); }
        }
        await refresh();
      } catch (err) { toast(err.message); el.disabled = false; }
      return;
    }
    const b = e.target.closest("[data-decide]");
    if (!b) return;
    const card = b.closest("[data-card]").dataset.card, decision = b.dataset.decide;
    // Decline asks for the reason first; Magnus reads it before suggesting again.
    if (decision === "decline-open" || decision === "decline-cancel") { ui.declining = decision === "decline-open" ? card : null; return draw(); }
    if (decision === "handle-open" || decision === "handle-cancel") { ui.handling = decision === "handle-open" ? card : null; return draw(); }
    const body = { decision };
    // "Handle things like this", or a grant card accepted: the grant as the person set it in the form.
    const box = b.closest("[data-card]");
    if (decision === "handle" || (box.matches(".grant") && decision === "confirm")) {
      Object.assign(body, readGrantForm(box));
      if (!body.apps.length) return toast("Choose at least one app.");
    }
    const no = decision === "decline" || decision === "dismiss";
    if (no) {
      body.reason = ($(`#why-${CSS.escape(card)}`, main)?.value || "").trim();
      if (!body.reason) return toast("Say why, so Magnus knows.");
    }
    b.disabled = true;
    try {
      const r = await api("POST", `/v1/chat/cards/${enc(card)}`, body);
      toast(no ? (decision === "dismiss" ? "Dismissed." : "Declined.") : decision === "later" ? "Not now." : decision === "handle" ? "Started. Things like this are handled from now on." : r.grant ? "Granted." : r.task ? "Started." : "Scheduled.");
      ui.declining = null; ui.handling = null; delete drafts[`why-${card}`];
      for (const k of Object.keys(drafts)) if (k.startsWith(`g-${card}-`)) delete drafts[k];
      await refresh();
    } catch (err) { toast(err.message); b.disabled = false; }
  };
  await draw();
}

// Open a conversation in place, with `panel` beside it. Replacing the
// window address reloads this app on every entry.
function showChat(main, hash, view, panel) {
  if (location.hash !== hash) history.replaceState(null, "", hash);
  if (panel) openPanel(panel);
  return view(main);
}

// Home is the latest conversation (a new one when there is none), with
// `panel` open beside it.
async function home(main, panel = App.openAfter) {
  App.openAfter = null;
  const seq = App.seq;
  const threads = (await api("GET", "/v1/chat")).threads;
  App.threads = threads;
  if (seq !== App.seq) return;
  if (threads.length) {
    const id = threads[0].thread;
    return showChat(main, `#/chat/${enc(id)}`, m => showThread(m, id), panel);
  }
  return showChat(main, "#/chat/new", m => showNewChat(m), panel);
}

route(/^#\/(?:home|chat)?$/, main => home(main), true);
route(/^#\/chat\/new$/, m => showNewChat(m), true);
route(/^#\/chat\/new\/([^/]+)$/, (m, project) => showNewChat(m, project), true);
// Where the views over tasks, schedules, files and decisions used to be
// pages: each opens beside the latest conversation.
route(/^#\/inbox$/, main => home(main, "needs"), true);
route(/^#\/(tasks|schedules|files|projects)$/, (main, k) => home(main, k), true);
// The Library is the connector catalog, a page of its own (connectors spec,
// Library); a connector's page is under it.
route(/^#\/library(?:\/([^/]+))?$/, (main, name) => showLibrary(main, name));
route(/^#\/new$/, main => home(main), true);
route(/^#\/file\/([^/]+)$/, (main, id) => home(main, `file:${id}`), true);
route(/^#\/project\/([^/]+)$/, (main, id) => home(main, `project:${id}`), true);
// A task opens beside the conversation it came from.
route(/^#\/task\/([^/]+)$/, async (main, id) => {
  const seq = App.seq;
  const t = await api("GET", `/v1/tasks/${enc(id)}`);
  if (seq !== App.seq) return;
  if (!t.thread) return home(main, `task:${id}`);
  return showChat(main, `#/chat/${enc(t.thread)}`, m => showThread(m, t.thread), `task:${id}`);
}, true);

function rule(e) {
  const m = e.every_minutes;
  if (m) {
    for (const [size, one, many] of [[10080, "week", "weeks"], [1440, "day", "days"], [60, "hour", "hours"]]) {
      if (m % size === 0) return m === size ? `Every ${one}` : `Every ${m / size} ${many}`;
    }
    return m === 1 ? "Every minute" : `Every ${m} minutes`;
  }
  if (e.daily_at) return `Every day at ${e.daily_at}`;
  return "";
}
// The decision an entry waits on, as its card, where the person can make it:
// the same card as under Needs you and beside the conversation.
function waitingCard(id, ui) {
  const i = id && App.inbox.find(x => x.id === id && x.status === "open");
  return i ? card(i, ui.composed || []) : "";
}
// When a schedule stops: a time, a number of runs, both, or never.
function ends(e) {
  const parts = [];
  if (e.until) parts.push(`no run after ${new Date(e.until).toLocaleString()}`);
  if (e.max_runs) parts.push(`at most ${e.max_runs} run${e.max_runs === 1 ? "" : "s"}`);
  return parts.length ? parts.join(", ") : "until you stop it";
}
// Where a task stands, as the design draws it: an icon and a pill. A task
// waiting on a decision the person can make needs them.
const needsYou = t => !!t.waiting_item && App.inbox.some(i => i.id === t.waiting_item && i.status === "open" && i.can_respond);
function standing(t) {
  if (needsYou(t)) return ["need", `<span class="chip warn">Needs you</span>`];
  return [{ completed: "done", running: "working", failed: "stopped", cancelled: "stopped" }[t.status] || "working", chip(t.status)];
}
// A task in its conversation: one row that opens it beside the thread, its
// status live; what it waits on from the person follows the request's rows.
function taskCard(t) {
  const why = t.status === "suspended" ? WAITING[t.waiting_on] || "" : t.status === "running" ? "working on it" : "";
  const [ic, pill] = standing(t);
  return `<li class="tcard" data-chat-task="${esc(t.task_id)}" data-status="${esc(t.status)}"><a class="rg-row${Panel.what === `task:${t.task_id}` ? " open" : ""}" href="#/task/${enc(t.task_id)}" data-open="task:${esc(t.task_id)}" title="${esc(t.text)}">
    <span class="st ${ic}">${icon(ic)}</span><span class="rg-main"><span class="ttl">${esc(t.title)}</span>${why ? `<span class="sub why">${esc(why)}</span>` : `<span class="sub">${esc(firstLine(t.text))}</span>`}</span>${pill}${icon("chev", "chev")}</a>
    ${ENDED.includes(t.status) ? "" : `<div class="card-acts"><button class="link small" data-task-act="cancel" data-task="${esc(t.task_id)}">Cancel</button></div>`}</li>`;
}
// A schedule where it was confirmed: a row that opens it, with when it runs
// next and how its last run went.
function scheduleText(sc) {
  const last = sc.last;
  return `${sc.ended ? `Ended: ${sc.ended}` : sc.paused ? `Paused: ${sc.paused}` : `Next run ${until(sc.next)}`} · ${sc.last_failed ? `Last firing could not start: ${sc.last_failed.error}` : last ? `Last result: ${(STATUS[last.status] || ["", last.status])[1]}, ${when(last.ended_at || last.created_at)}` : "No results yet"}`;
}
// A card's answers: accept, Not now, or Decline, which asks for the reason.
// A task proposal is approved or dismissed (with the reason), not put off;
// neither is a grant card (`later` false).
function cardActs(e, ui, accept, more = "", proposal = false, later = !proposal) {
  const no = proposal ? "Dismiss" : "Decline";
  if (ui.declining === e.card) return `<div class="row cc-act decline-why"><label class="sr" for="why-${esc(e.card)}">Why not?</label>
    <input id="why-${esc(e.card)}" data-draft="why-${esc(e.card)}" placeholder="Why not? Magnus reads this before ${proposal ? "proposing" : "suggesting"} again">
    <button class="primary" data-decide="${no.toLowerCase()}">${no}</button><button class="link" data-decide="decline-cancel">Cancel</button></div>`;
  return `<div class="row cc-act"><button class="primary" data-decide="${proposal ? "approve" : "confirm"}">${esc(accept)}</button>${more}${later ? `<button data-decide="later">Not now</button>` : ""}<button class="link" data-decide="decline-open">${no}</button></div>`;
}

// A grant's fields as a form (#235): the kind of work, the apps it covers,
// what may go ahead without asking besides, and its budget of model cost
// per period. `k` keys its drafts; `f` holds the values it starts from;
// `apps` are the person's apps to choose from.
function grantForm(k, f, apps) {
  const id = n => `g-${esc(k)}-${n}`, d = n => `data-draft="${id(n)}"`;
  const lifted = x => (f.allow || []).includes(x) ? "checked" : "";
  const choices = apps?.length ? apps : (f.apps || []).map(name => ({ name, title: name }));
  return `<div class="grant-form" data-grant-form>
    <label for="${id("title")}" class="small muted">What kind of work</label><input id="${id("title")}" ${d("title")} data-g="title" value="${esc(f.title || "")}" style="width:100%">
    <div class="sugg-part"><span class="small muted">In these apps</span>${choices.map(a => `<label class="row toggle small"><input type="checkbox" data-g-app="${esc(a.name)}" ${d(`app-${a.name}`)} ${(f.apps || []).includes(a.name) ? "checked" : ""}><span>${esc(a.title || a.name)}</span></label>`).join("")}</div>
    <div class="sugg-part"><span class="small muted">Without asking, Magnus may also</span>
      <label class="row toggle small"><input type="checkbox" data-g-allow="send" ${d("send")} ${lifted("send")}><span>Send on my behalf</span></label>
      <label class="row toggle small"><input type="checkbox" data-g-allow="irreversible" ${d("irreversible")} ${lifted("irreversible")}><span>Delete or commit me to anything</span></label>
      <p class="small faint">Spending money always asks.</p></div>
    <div class="row"><label for="${id("max")}" class="small muted">Budget</label><span class="small">$</span><input id="${id("max")}" type="number" min="0" step="0.5" data-g="max_usd" ${d("max")} value="${esc(f.max_usd ?? "")}" style="width:6em">
      <select aria-label="Per" data-g="period" ${d("period")}>${["day", "week", "month"].map(p => `<option value="${p}" ${(f.period || "week") === p ? "selected" : ""}>a ${p}</option>`).join("")}</select><span class="small muted">of model cost</span></div>
  </div>`;
}

// The grant a form holds, as the appliance takes it.
function readGrantForm(root) {
  const f = $("[data-grant-form]", root);
  return {
    title: $("[data-g=title]", f).value.trim(), period: $("[data-g=period]", f).value, max_usd: Number($("[data-g=max_usd]", f).value),
    apps: $$("[data-g-app]", f).filter(i => i.checked).map(i => i.dataset.gApp), allow: $$("[data-g-allow]", f).filter(i => i.checked).map(i => i.dataset.gAllow),
  };
}

// What a card rests on: each item, and the tool or task it points at.
function evidenceList(e, label) {
  return `<div class="sugg-part"><span class="small muted">${label}</span><ul class="sugg-list">${(e.evidence || []).map(x => `<li>${esc(x.shows)} <span class="small muted">(${x.call ? `from ${esc(x.tool)}` : `task: ${esc(x.title || "your request")}`})</span></li>`).join("")}</ul></div>`;
}

// What became of a card that is no longer open, other than a schedule it made.
function cardOutcome(e) {
  const p = e.kind === "proposal_card";
  if (e.state === "declined") return `<p class="small"><span class="chip">${p ? "Dismissed" : "Declined"}</span> ${esc(e.title || "")}${e.reason ? ` <span class="muted">· ${esc(e.reason)}</span>` : ""}</p>`;
  if (e.state === "later") return `<p class="small"><span class="chip">Not now</span> ${esc(e.title || "")}</p>`;
  const handled = e.granted ? ` <span class="chip ok">Handled from now on</span> <a class="small" href="#/settings/data">Your grants</a>` : "";
  if (e.started) return `<p class="small"><span class="chip ok">${p ? "Approved" : "Accepted"}</span> ${esc(e.title || "")} <a href="#/task/${enc(e.started)}" data-open="task:${esc(e.started)}">See the task</a>${handled}</p>`;
  if (e.kind === "grant_card" && e.granted) return `<p class="small"><span class="chip ok">Granted</span> ${esc(e.title || "")} <a href="#/settings/data">See it in Settings</a></p>`;
  return "";
}

function scheduleLine(e) {
  if (e.state !== "confirmed") return cardOutcome(e);
  const sc = App.schedules.find(x => x.id === e.schedule);
  if (!sc) return `<p class="small" data-schedule-gone="${esc(e.schedule || "")}"><span class="chip">Cancelled</span> ${esc(e.title || "This schedule")} no longer runs.</p>`;
  const act = (k, label) => `<button class="link small" data-sched-act="${k}" data-schedule-id="${esc(sc.id)}">${label}</button>`;
  const chip = sc.ended ? `<span class="chip">Ended</span>` : `<span class="chip sch">${sc.paused ? "Paused" : "Scheduled"}</span>`;
  const acts = sc.ended ? act("cancel", "Delete") : `${sc.paused ? act("resume", "Resume") : act("pause", "Pause")}${act("cancel", "Cancel")}`;
  return `<div class="reqgroup sched-state" data-schedule="${esc(sc.id)}"><a class="rg-row" href="#/schedules" data-open="schedule:${esc(sc.id)}" title="Open the schedule">
    <span class="st sch">${icon("calendar")}</span><span class="rg-main"><span class="ttl">${esc(sc.title)}</span><span class="sub" data-last>${esc(scheduleText(sc))}</span></span>${chip}${icon("chev", "chev")}</a>
    <div class="card-acts">${acts}<a class="small" href="#/schedules" data-open="schedule:${esc(sc.id)}">Open</a>${sc.started ? `<a class="small" href="#/chat/${enc(`schedule-${sc.id}`)}" data-schedule-runs>Its runs</a>` : ""}</div></div>`;
}

// Motion (U7): what is new in the thread rises into place, a task whose
// status changed shows its new status gently, and finished work gets a quiet
// moment. A repaint draws every node afresh, so only what changed is marked;
// the stylesheet decides, by the motion setting, how each mark moves, if at
// all.
function motion(el, ui) {
  const first = !ui.seen;
  ui.seen ||= new Set();
  ui.status ||= {};
  for (const n of $$(".entry[data-entry]", el)) {
    const k = n.dataset.entry;
    if (!first && k && !ui.seen.has(k)) n.classList.add("enter");
    ui.seen.add(k);
  }
  for (const n of $$("[data-chat-task]", el)) {
    const k = n.dataset.chatTask, st = n.dataset.status, was = ui.status[k];
    if (was && was !== st) n.classList.add(st === "completed" ? "finished" : "changed");
    ui.status[k] = st;
  }
}

// The clipboard where the page may use it; the old way where it may not.
async function copyText(t) {
  try { await navigator.clipboard.writeText(t); return; } catch { /* fall back */ }
  const a = document.createElement("textarea");
  a.value = t; a.style.position = "fixed"; a.style.opacity = "0"; document.body.append(a); a.select();
  const ok = document.execCommand("copy"); a.remove();
  if (!ok) throw new Error("This app may not use the clipboard.");
}

// What a message says, as text to copy.
const entryText = e => e.kind === "result" ? `${e.title} (${e.status})\n\n${e.summary || ""}`
  : e.kind === "tasks" ? e.tasks.map(t => `- ${t.title}: ${t.text}`).join("\n") : e.text || "";
const safeName = e => `${(entryText(e).split(/\s+/).slice(0, 6).join(" ").replace(/[^\p{L}\p{N} -]/gu, " ").trim().replace(/\s+/g, "-") || "message")}.md`;

// Icons for what can be done to a chat entry: the shapes inside each 24px
// outline icon from Lucide (lucide-static 0.563.0), under this licence:
// ISC License
//
// Copyright (c) for portions of Lucide are held by Cole Bemis 2013-2026 as part of Feather (MIT). All other copyright (c) for Lucide are held by Lucide Contributors 2026.
//
// Permission to use, copy, modify, and/or distribute this software for any
// purpose with or without fee is hereby granted, provided that the above
// copyright notice and this permission notice appear in all copies.
//
// THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
// WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
// MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
// ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
// WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
// ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
// OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
//
// ---
//
// The MIT License (MIT) (for portions derived from Feather)
//
// Copyright (c) 2013-2026 Cole Bemis
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.
const ICONS = {
  copy: '<rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>', // copy
  edit: '<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/>', // pencil
  retry: '<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/>', // rotate-ccw
  listen: '<path d="M11 4.702a.705.705 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.705.705 0 0 0 11 19.298z"/><path d="M16 9a5 5 0 0 1 0 6"/><path d="M19.364 18.364a9 9 0 0 0 0-12.728"/>', // volume-2
  hush: '<circle cx="12" cy="12" r="10"/><rect x="9" y="9" width="6" height="6" rx="1"/>', // circle-stop
  branch: '<line x1="6" x2="6" y1="3" y2="15"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M18 9a9 9 0 0 1-9 9"/>', // git-branch
  share: '<circle cx="18" cy="5" r="3"/><circle cx="6" cy="12" r="3"/><circle cx="18" cy="19" r="3"/><line x1="8.59" x2="15.42" y1="13.51" y2="17.49"/><line x1="15.41" x2="8.59" y1="6.51" y2="10.49"/>', // share-2
  export: '<path d="M12 15V3"/><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m7 10 5 5 5-5"/>', // download
  save: '<path d="M12 10v6"/><path d="M9 13h6"/><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>', // folder-plus
  task: '<path d="M16 5H3"/><path d="M11 12H3"/><path d="M16 19H3"/><path d="M18 9v6"/><path d="M21 12h-6"/>', // list-plus
  great: '<circle cx="12" cy="12" r="10"/><path d="M8 14s1.5 2 4 2 4-2 4-2"/><line x1="9" x2="9.01" y1="9" y2="9"/><line x1="15" x2="15.01" y1="9" y2="9"/>', // smile
  annoying: '<circle cx="12" cy="12" r="10"/><path d="M16 16s-1.5-2-4-2-4 2-4 2"/><line x1="9" x2="9.01" y1="9" y2="9"/><line x1="15" x2="15.01" y1="9" y2="9"/>', // frown
  delete: '<path d="M10 11v6"/><path d="M14 11v6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/><path d="M3 6h18"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>', // trash-2
};

// What a person can do with each entry. Edit is for their own messages,
// Retry for Magnus's answers, and Read aloud for an answer, from a script
// written for listening; a reaction (great or annoying, pressed again to
// take it back) for each of Magnus's messages; the rest for every entry. Each is an icon, named
// by its tooltip and, for screen readers, by hidden text.
function msgActions(e, ui) {
  if (!e.id) return "";
  const mine = e.kind === "message", magnus = e.kind === "answer" || e.kind === "turn";
  // Every kind the server takes reactions on (reactions.rs MAGNUS, #359): its answers, turns,
  // results, started tasks, proposed cards, and its own openings (#360).
  const said = ["answer", "turn", "result", "tasks", "proposal_card", "grant_card", "suggestion_card", "schedule_card", "connector_card", "opener"].includes(e.kind);
  const acts = [["copy", "Copy"], mine && ["edit", "Edit"], magnus && ["retry", "Retry"], e.kind === "answer" && ["listen", "Read aloud"], e.kind === "answer" && ["hush", "Stop"],
    said && ["great", "That was great"], said && ["annoying", "That was annoying"], ["branch", "Branch"], ["share", "Share"],
    ["export", "Export"], ["save", "Save to Files"], ["task", "Make a task"], ["delete", "Delete"]].filter(Boolean);
  const share = ui.sharing === e.id ? `<div class="row small share-to"><label class="sr" for="share-${esc(e.id)}">Share with</label>
    <select id="share-${esc(e.id)}" data-share-to>${ui.people.map(p => `<option value="${esc(p.id)}">${esc(p.name)}</option>`).join("")}</select>
    <button class="primary" data-msg-act="share-send">Share</button><button class="link" data-msg-act="share-cancel">Cancel</button></div>` : "";
  const pressed = a => a === "great" || a === "annoying" ? ` aria-pressed="${e.reaction === a}"` : "";
  return `<div class="msg-actions small" role="toolbar" aria-label="Actions">${acts.map(([a, l]) => `<button class="icon${e.reaction === a ? " on" : ""}" data-msg-act="${a}" title="${l}"${pressed(a)}><svg viewBox="0 0 24 24" aria-hidden="true">${ICONS[a]}</svg><span class="sr">${l}</span></button>`).join("")}</div>${share}`;
}

function entry(e, ui = {}) {
  switch (e.kind) {
    case "message": return ui.editing === e.id
      ? `<div class="msg me editing"><label class="sr" for="edit-${esc(e.id)}">Edit and resend</label><textarea id="edit-${esc(e.id)}" rows="3" data-draft="edit-${esc(e.id)}"></textarea>
        <div class="row small"><button class="primary" data-msg-act="edit-send">Send</button><button class="link" data-msg-act="edit-cancel">Cancel</button></div></div>`
      : `<div class="msg me"><div class="small faint">You · ${esc(when(e.at))}</div><div>${esc(e.text)}</div>${e.files?.length ? `<div class="previews">${e.files.map(filePreview).join("")}</div>` : ""}</div>`;
    case "opener": return `<div class="msg magnus" data-opener><div class="small faint">Magnus · ${esc(when(e.at))}</div><div class="small muted">${e.app_title ? `You connected ${esc(e.app_title)}, so I had a look at what I could take over there.` : "You are new here, so I had a look at what I could take over for you."}</div></div>`;
    case "answer": return `<div class="msg magnus" data-answer><div class="small faint">Magnus${e.model ? ` · ${esc(e.model.model)}` : ""} · ${esc(when(e.at))}</div><div class="pre">${esc(e.text || "")}</div>${e.view ? shownFrame(e.id, "What Magnus showed") : ""}${verdictBar(e.task_id, e.verdict)}</div>`;
    case "turn": return e.status === "failed" || e.status === "cancelled"
      ? `<div class="msg magnus"><div class="small bad">Could not answer: ${esc(e.text || e.status)}</div></div>`
      : `<div class="msg magnus"><div class="small faint">${e.status === "running" ? "Working on an answer…" : esc(WAITING[e.waiting_on] || "paused")}</div>${e.status === "running" ? `<div class="pre" data-live="${esc(e.task_id)}">${esc(ui.live?.[e.task_id] || "")}</div>` : ""}</div>${waitingCard(e.waiting_item, ui)}`;
    case "tasks": {
      const done = e.tasks.filter(t => t.status === "completed").length, need = e.tasks.filter(needsYou).length;
      return `<div class="reqgroup chatcard" data-chat-tasks="${esc(e.from_run)}"><div class="rg-head"><span>From this request</span><span class="small muted num">${done} of ${e.tasks.length} done${need ? ` · ${need} need${need === 1 ? "s" : ""} you` : ""}</span></div>
        <ul class="rg-rows">${e.tasks.map(taskCard).join("")}</ul></div>
        ${e.tasks.map(t => t.waiting_item && waitingCard(t.waiting_item, ui) ? `<div class="decide" data-for-task="${esc(t.task_id)}">${waitingCard(t.waiting_item, ui)}</div>` : "").join("")}`;
    }
    case "result": return `<div class="result" data-result="${esc(e.task_id)}">${ui.movedOn?.has(e.task_id) ? `<blockquote class="asked small"><span class="faint">You asked:</span> ${esc(firstLine(e.text))}</blockquote>` : ""}<details data-key="result:${esc(e.task_id)}"><summary>${chip(e.status)} <span class="ttl">${esc(e.title)}</span> <span class="muted">${esc(firstLine(e.summary))}</span></summary>
      <div class="pre">${esc(e.summary || "")}</div>${ENDED.includes(e.status) ? verdictBar(e.task_id, e.verdict) : ""}</details>
      ${e.view ? shownFrame(e.id, "The deliverable") : ""}
      ${e.files?.length ? `<div class="previews">${e.files.map(filePreview).join("")}</div>` : ""}<a class="small see" href="#/task/${enc(e.task_id)}" data-open="task:${esc(e.task_id)}">See the outcome</a></div>`;
    case "connector_card": return appOfferCard(e);
    case "suggestion_card": {
      if (e.state !== "open") return `<div data-card="${esc(e.card)}">${e.schedule ? scheduleLine(e) : cardOutcome(e)}</div>`;
      const timed = e.every_minutes || e.daily_at;
      return `<div class="confirm chatcard suggestion" data-card="${esc(e.card)}"><div class="cc-head"><span class="st sch">${icon(timed ? "calendar" : "tasks")}</span><strong>${esc(e.title)}</strong></div>
        <p class="cc-text">${esc(e.plan)}</p>
        ${e.changed ? `<p class="small"><strong>What changed:</strong> ${esc(e.changed)}</p>` : ""}
        ${evidenceList(e, "Why")}
        <div class="sugg-part"><span class="small muted">What it may do</span><ul class="sugg-list">${(e.permissions || []).map(x => `<li>${esc(x)}</li>`).join("")}</ul></div>
        ${timed ? `<div class="tiles"><div><span>When</span><strong>${esc(rule(e))}</strong></div><div><span>Ends</span><strong>${esc(ends(e))}</strong></div></div>` : ""}
        <p class="small muted">It acts for you (${esc(e.acts_for)}), with your access; what it needs from you shows under Needs you.</p>${taintNote(e.taint)}
        ${cardActs(e, ui, "Accept")}</div>`;
    }
    case "proposal_card": {
      if (e.state !== "open") return `<div data-card="${esc(e.card)}">${cardOutcome(e)}</div>`;
      const n = e.needs || {}, part = (label, html) => `<div class="sugg-part"><span class="small muted">${label}</span>${html}</div>`;
      return `<div class="confirm chatcard suggestion proposal" data-card="${esc(e.card)}"><div class="cc-head"><span class="st sch">${icon("tasks")}</span><strong>${esc(e.title)}</strong></div>
        ${part("Noticed", `<p class="cc-text">${esc(e.noticed?.what)} <span class="small muted">(${esc(e.noticed?.where)})</span></p>`)}
        ${part("Outcome", `<p class="cc-text">${esc(e.outcome)}</p>`)}
        ${part("Why now", `<p class="cc-text">${esc(e.why_now)}</p>`)}
        ${e.changed ? `<p class="small"><strong>What changed:</strong> ${esc(e.changed)}</p>` : ""}
        ${part("Needs", `<ul class="sugg-list">${(n.permissions || []).map(x => `<li>${esc(x)}</li>`).join("")}<li class="num">Up to $${esc(n.estimated_cost_usd)}</li>${n.input ? `<li>From you: ${esc(n.input)}</li>` : ""}</ul>`)}
        ${evidenceList(e, "Seen in")}
        <p class="small muted">It acts for you (${esc(e.acts_for)}), with your access, and spends no more than this; what it needs from you shows under Needs you.</p>${taintNote(e.taint)}
        ${ui.handling === e.card ? `<div class="sugg-part" data-handle>${grantForm(e.card, e.handle || {}, e.your_apps)}
          <p class="small muted">This task starts now, and from now on Magnus handles work like it without asking, within these limits and your organisation's policy. Change or revoke it in Settings.</p>
          <div class="row cc-act"><button class="primary" data-decide="handle">Handle from now on</button><button class="link" data-decide="handle-cancel">Cancel</button></div></div>`
          : cardActs(e, ui, "Approve", `<button data-decide="handle-open">Handle things like this</button>`, true)}</div>`;
    }
    case "grant_card": {
      if (e.state !== "open") return `<div data-card="${esc(e.card)}">${cardOutcome(e)}</div>`;
      return `<div class="confirm chatcard suggestion grant" data-card="${esc(e.card)}"><div class="cc-head"><span class="st sch">${icon("tasks")}</span><strong>Let Magnus handle this on its own?</strong></div>
        <p class="cc-text">${esc(e.why)}</p>
        ${grantForm(e.card, e, e.your_apps)}
        <p class="small muted">Tasks Magnus starts for this go ahead without asking, as you (${esc(e.acts_for)}), within these limits and your organisation's policy; it reports every action. Change or revoke it in Settings.</p>${taintNote(e.taint)}
        ${cardActs(e, ui, "Accept", "", false, false)}</div>`;
    }
    case "schedule_card": {
      const b = e.budget || {};
      if (e.state !== "open") return `<div data-card="${esc(e.card)}">${e.state === "confirmed" ? scheduleLine(e) : cardOutcome(e)}</div>`;
      return `<div class="confirm chatcard" data-card="${esc(e.card)}"><div class="cc-head"><span class="st sch">${icon("calendar")}</span><strong>Start this schedule?</strong></div>
        <p class="cc-text">${esc(e.text)}</p>
        <div class="tiles"><div><span>When</span><strong>${esc(rule(e))}</strong></div><div><span>What</span><strong>${esc(e.scope || "")}</strong></div><div><span>Ends</span><strong>${esc(ends(e))}</strong></div>
          <div><span>Budget</span><strong class="num">Up to $${esc(b.max_usd)} a run</strong><span class="small">${esc(b.max_model_calls)} model calls, ${esc(b.max_actions)} actions</span></div></div>
        <p class="small muted">It acts for you (${esc(e.acts_for)}), with your access; what it needs from you shows under Needs you.</p>${taintNote(e.taint)}
        ${cardActs(e, ui, "Start schedule", `<button data-act="change-schedule">Change</button>`)}</div>`;
    }
    default: return "";
  }
}

route(/^#\/chat\/([^/]+)$/, async (main, thread) => {
  if (thread === "new") return showNewChat(main);
  return showThread(main, thread);
}, true);

// ---------- Inbox ----------

const KIND = {
  approval: "Approval", question: "Question", review: "Review", notice: "Stopped", message: "Note from the agent", observation: "Observation", interrupt: "Interruption",
  policy_proposal: "Policy proposal", schedule_proposal: "Recurring task proposal", task_proposal: "Task proposal", tool_change: "Changed tool", tool_review: "New app's tools",
  contract_change: "Contract change", deletion_request: "Deletion request", shared: "Shared with you", endorsement: "Company fact?",
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
    ${destinationFields(d.destinations)}
    ${Object.entries(input).map(([k, v]) => `<div class="field"><span>${esc(k)}</span>${fieldValue(v)}</div>`).join("")}
  </div>`;
}

// Who it reaches and what it says, as the app has it (a draft's To, Cc,
// Bcc, subject, text and attachments), read by the appliance, not taken
// from what Magnus wrote.
function destinationFields(ds) {
  if (!ds) return "";
  const label = k => esc(k[0].toUpperCase() + k.slice(1));
  const to = Object.entries(ds.recipients || {}).filter(([, a]) => a.length)
    .map(([k, a]) => `<div class="field"><span>${label(k)}</span><span>${a.map(esc).join(", ")}</span></div>`);
  const content = Object.entries(ds.content || {}).filter(([, v]) => v !== null && v !== "" && !(Array.isArray(v) && !v.length))
    .map(([k, v]) => `<div class="field"><span>${label(k)}</span>${Array.isArray(v) ? `<span>${v.map(f => esc(f.filename || f.name || JSON.stringify(f))).join(", ")}</span>` : fieldValue(v)}</div>`);
  return to.concat(content).join("");
}

function taintNote(t) {
  if (!t || (!t.untrusted && !t.private)) return "";
  const parts = [];
  if (t.untrusted) parts.push("has read content from outside sources, which could be steering it");
  if (t.private) parts.push("has read private data");
  return `<p class="small muted">This task ${parts.join(" and ")}.</p>`;
}

// A contract as the person reads it: the goal, then each criterion with the
// check the appliance will run for it, in full.
function checkText(ch) {
  if (!ch) return `<span class="faint">no check: you review it</span>`;
  const exp = (ch.expect || []).map(e => `${e.pointer || "(whole result)"} ${"equals" in e ? `equals ${JSON.stringify(e.equals)}` : `contains ${JSON.stringify(e.contains)}`}`).join("; ");
  return `<span class="mono">${esc(ch.tool)}</span> ${fieldValue(ch.input)}<div class="small">expects ${esc(exp)}</div>`;
}
// Where a check's verdict comes from, as the kernel labelled it when the
// contract was recorded.
function assuranceChip(cr) {
  if (cr.assurance === "appliance-verified") return ` <span class="chip ok" title="The sandbox image's own tools, or the appliance's comparison against the expected values in this contract, decide it">appliance-verified</span>`;
  if (cr.assurance === "self-checked") return ` <span class="chip warn" title="${esc((cr.assurance_why || []).join("; "))}">self-checked</span><div class="small faint">Its check runs code the agent wrote: ${esc((cr.assurance_why || []).join("; "))}</div>`;
  return "";
}
function contractView(c, label) {
  if (!c) return `<div class="small faint">${esc(label)}: none</div>`;
  return `<div aria-label="${esc(label)}"><div class="small muted">${esc(label)}</div><p><strong>${esc(c.goal || "")}</strong></p>
    ${c.conversation?.length ? `<details class="small"><summary>From the chat</summary>${c.conversation.map(m => `<div><span class="muted">${esc(m.role)}:</span> ${esc(m.text)}</div>`).join("")}</details>` : ""}
    <ol class="small">${(c.criteria || []).map(cr => `<li><div>${esc(cr.description || cr.id)} <span class="faint mono">${esc(cr.id)}</span>${assuranceChip(cr)}</div>${checkText(cr.check)}</li>`).join("")}</ol></div>`;
}

function buttons(i, list) {
  if (!i.can_respond) return `<p class="small faint">Waiting for someone with the right role to decide.</p>`;
  return `<div class="row" style="margin-top:12px">${list.map(([d, label, cls, extra]) => `<button class="${cls || ""}" data-item="${esc(i.id)}" data-decision="${d}" ${extra || ""}>${esc(label)}</button>`).join("")}</div>`;
}

function card(i, composed) {
  const d = i.details || {};
  const tier = i.kind === "approval" ? (i.tier || d.risk) : "";
  const head = `<div class="spread"><span class="kind">${esc(KIND[i.kind] || i.kind)}${tier ? ` · ${esc(TIER[tier] || tier)}` : ""}</span>
    <span class="small faint">${i.blocking ? `<strong style="color:var(--fg)">A task is waiting</strong> · ` : ""}${esc(when(i.created_at))}${i.task_id && i.person === App.me?.id ? ` · <a href="#/task/${enc(i.task_id)}" data-open="task:${esc(i.task_id)}">task</a>` : ""}</span></div>`;
  let body = "";
  if (i.status !== "open") {
    const r = i.resolution || {};
    return `<div class="card flat item ${esc(i.kind)} ${esc(tier)}">${head}<h4>${esc(i.summary)}</h4><p class="small muted">${esc(r.decision)}${r.scope ? ` (${esc(r.scope)})` : ""} by ${esc(String(r.by || "").replace(/^person:/, ""))}${r.text ? `: ${esc(r.text)}` : ""}</p></div>`;
  }
  switch (i.kind) {
    case "approval": {
      const onceOnly = String(d.rule || "").startsWith("kernel:") || d.reason === "unknown_outcome";
      // Web lookups after reading private data: one answer for the whole
      // task, or the whole conversation.
      const list = d.web_lookups ? [["approve:task", `Allow web lookups for this ${d.web_lookups}`, "primary"], ["approve:once", "Allow this lookup only"]] : [["approve:once", "Approve once", "primary"]];
      if (!onceOnly) {
        list.push(["approve:task", "Approve for this task"]);
        list.push(["approve:window", "Approve for an hour"]);
        if (isOwner()) list.push(["approve:standing", "Always allow"]);
      }
      list.push(["reject", "Deny"]);
      body = `${i.app ? appOfferCard(i.app, i.person === App.me?.id) : ""}${i.consequence ? `<div class="consequence" role="note">${esc(i.consequence)}</div>` : ""}
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
    case "contract_change":
      body = `<p class="small">The work has started, so the criteria are frozen. The agent wants to replace them. Only criteria you approve decide when the task is done.</p>
        <div class="sidebyside">${contractView(d.old, `Now (version ${d.version})`)}${contractView(d.new, "Proposed")}</div>
        ${taintNote(d.taint)}
        ${i.can_respond ? `<textarea rows="2" data-draft="cc-${esc(i.id)}" placeholder="If you keep the current contract, say why (the agent sees this)" style="margin-top:10px"></textarea>` : ""}
        ${buttons(i, [["approve", "Approve the change", "primary"], ["reject", "Keep the current contract"]])}`;
      break;
    case "review": {
      const crit = (d.criteria || []).map(c => `<li>${esc(c.description || c.id || c)}${c.unchecked_reason ? `<div class="small faint">Its check could not run: ${esc(c.unchecked_reason)}</div>` : ""}</li>`).join("");
      body = `<p>${esc(d.summary)}</p>
        <div class="evidence none"><div class="small muted">The appliance could not check these. Check them yourself:</div><ul>${crit}</ul></div>
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
      // A schedule's own notice stands for no run to continue: its Resume does that.
      if (typeof d.reason === "string" && !d.reason.startsWith("schedule_") && i.run_id) list.push(["resume", "Continue", "primary"]);
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
        <div class="field"><span>When</span><span>${d.every_minutes ? `every ${esc(d.every_minutes)} minutes` : `daily at ${esc(d.daily_at)}`}</span></div>
        <div class="field"><span>Ends</span><span>${esc(ends(d))}</span></div></div>${taintNote(d.taint)}
        ${buttons(i, [["accept", "Schedule it", "primary"], ["reject", "Reject"]])}`;
      break;
    case "task_proposal":
      body = `<div class="what"><div class="field"><span>Task</span><span>${esc(d.title)}</span></div>
        <div class="field"><span>Does</span><span>${esc(d.text)}</span></div>
        <div class="field"><span>Noticed</span><span>${esc(d.noticed?.what)} (${esc(d.noticed?.where)})</span></div>
        <div class="field"><span>Outcome</span><span>${esc(d.outcome)}</span></div>
        <div class="field"><span>Why now</span><span>${esc(d.why_now)}</span></div>
        <div class="field"><span>Needs</span><span>${esc((d.needs?.permissions || []).join("; "))}</span></div>
        <div class="field"><span>May spend</span><span>$${esc(d.budget?.max_usd)}</span></div></div>
        ${(d.evidence || []).length ? `<div class="evidence"><div class="small muted">Evidence</div><ul class="small">${(d.evidence || []).map(c => `<li>${esc(c.title || c.call || "")}: “${esc(c.shows)}”</li>`).join("")}</ul></div>` : ""}${taintNote(d.taint)}
        ${buttons(i, [["accept", "Start this task", "primary"], ["reject", "Dismiss"]])}`;
      break;
    case "message":
      body = `${taintNote(d.taint)}${buttons(i, [["dismiss", "Dismiss"]])}`;
      break;
    case "observation":
    case "interrupt":
      body = `${d.why ? `<p>${esc(d.why)}</p>` : ""}${d.evidence ? `<div class="evidence"><div class="small muted">Evidence</div>${fieldValue(d.evidence)}</div>` : ""}${d.notify_at && Date.parse(d.notify_at) > Date.now() ? `<p class="small faint">Pings ${esc(when(d.notify_at))}.</p>` : ""}${taintNote(d.taint)}${buttons(i, [["dismiss", "Dismiss"]])}`;
      break;
    case "tool_change":
      body = `<div class="what"><div class="field"><span>Tool</span><span class="mono">${esc(d.tool)}</span></div><div class="field"><span>New description</span>${fieldValue(d.description)}</div><div class="field"><span>Inputs</span>${fieldValue(d.input_schema)}</div></div>
        ${buttons(i, [["approve", "Accept the change", "primary"], ["reject", "Keep it blocked"]])}`;
      break;
    case "shared":
      body = `<div class="msg magnus"><div class="small faint">${esc(d.speaker)} · ${esc(when(d.at))}</div><div class="pre">${esc(d.text)}</div></div>${taintNote(d.taint)}
        ${buttons(i, [["dismiss", "Dismiss"]])}`;
      break;
    case "deletion_request":
      body = `<div class="what"><div class="field"><span>From</span><span>${esc(d.requested_by)}</span></div><div class="field"><span>What</span><span>${esc(d.what)}</span></div>
        <div class="field"><span>Due</span><span>${esc(new Date(d.due).toLocaleDateString())}</span></div></div>
        ${i.can_respond ? `<textarea rows="2" data-draft="ans-${esc(i.id)}" placeholder="A note for them: what was deleted, or why not" style="margin-top:10px"></textarea>` : ""}
        ${buttons(i, [["done", "Deleted", "primary"], ["decline", "Decline"]])}`;
      break;
    // An entry to make a company fact: what it says, where it came from, and
    // what the appliance found when it checked its citations.
    case "endorsement": {
      const ev = d.verification?.evidence || [];
      const how = { corroborated: "found in independent sources", system_of_record: "found in a system of record" }[d.verification?.how];
      body = `<div class="msg magnus"><div class="pre">${esc(d.text)}</div></div>
        <div class="what"><div class="field"><span>Now</span><span>${esc(d.integrity)}${how ? ` (${esc(how)})` : ""}</span></div>
        <div class="field"><span>Readable by</span><span>${esc((d.readers || []).length ? d.readers.map(c => c.join(" or ")).join(", and ") : "everyone")}</span></div>
        <div class="field"><span>Asked by</span><span>${esc(d.requested_by)}</span></div>
        <div class="field"><span>Source</span>${ev.length ? `<ul class="small">${ev.map(c => `<li>${c.found ? "✓" : "✗ not found in"} ${esc(c.source)}: “${esc(c.quote)}”</li>`).join("")}</ul>` : fieldValue(d.source)}</div></div>
        ${i.can_respond ? `<textarea rows="2" data-draft="ans-${esc(i.id)}" placeholder="A note: how you checked it (optional)" style="margin-top:10px"></textarea>` : ""}
        ${buttons(i, [["endorse", "Make it a company fact", "primary"], ["decline", "Decline"]])}`;
      break;
    }
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
// A task's delivered files, the latest of each name.
const latestOutputs = t => Object.values(Object.fromEntries((t?.outputs || []).map(o => [o.name, o])));
const decodeText = u => new TextDecoder().decode(Uint8Array.from(atob(u.slice(u.indexOf(",") + 1)), c => c.charCodeAt(0)));
// The delivered files a composition shows, fetched by the app itself:
// images as data: URLs, text files decoded for drawing.
async function fileData(view, t) {
  const out = {};
  const names = new Set((view || []).filter(c => c.type === "image" || c.type === "file").map(c => c.file));
  for (const o of latestOutputs(t).filter(o => names.has(o.name))) {
    const path = `/v1/tasks/${enc(t.task_id)}/outputs/${enc(o.id)}`;
    try {
      if (/^image\/(png|jpeg|gif|webp)$/.test(o.media_type)) out[o.name] = { image: await Magnus.dataUrl(path, o.media_type) };
      else if (Compose.TEXT.test(o.media_type)) out[o.name] = { text: decodeText(await Magnus.dataUrl(path, o.media_type)), media_type: o.media_type };
    } catch { /* shown as missing */ }
  }
  return out;
}
// The look in effect, for what a frame draws: its tokens, and light or dark.
const look = () => { const l = window.Theme?.look; return l ? [Theme.tokens(l), l.mode] : []; };

// The height each composed frame in a conversation last had, by its entry:
// a redraw of the thread makes the frame anew, and drawn at its known height
// the thread keeps its layout (and its scroll) while the frame loads again.
const frameHeights = {};
// (Set after the thread is drawn, not in its markup, so the markup stays the
// same and a loaded frame does not make the thread draw itself again.)
const shownFrame = (id, title) => `<iframe class="composed" sandbox="allow-same-origin" title="${title}" data-shown="${esc(id)}"></iframe>`;
const keepHeights = el => $$("iframe[data-shown]", el).forEach(f => { if (frameHeights[f.dataset.shown]) f.style.height = frameHeights[f.dataset.shown] + "px"; });
async function fill(frame, view, taskId, task) {
  if (!frame || frame._drawn) return;
  frame._drawn = true;
  const t = task || (view?.some(c => c.type === "image" || c.type === "file") ? await api("GET", `/v1/tasks/${enc(taskId)}`) : null);
  frame.srcdoc = Compose.documentFor(view, await fileData(view, t), fontCss(), t?.preview, ...look());
  frame.onload = () => {
    try {
      const h = frame.contentDocument.documentElement.scrollHeight + 4;
      frame.style.height = h + "px";
      if (frame.dataset.shown) frameHeights[frame.dataset.shown] = h;
    } catch { /* sized by CSS */ }
    // At its height: what is below it has stopped moving (the installed-app
    // driver clicks once its card's frames are, #207).
    frame.dataset.sized = "1";
  };
}

async function respond(btn, root) {
  // Sent once, however many handlers see the click.
  if (btn.disabled) return;
  const id = btn.dataset.item;
  let [decision, scope] = btn.dataset.decision.split(":");
  const body = { decision };
  if (scope) body.scope = scope;
  if (scope === "window") body.minutes = 60;
  const box = $(`[data-draft="ans-${CSS.escape(id)}"]`, root) || $(`[data-draft="rev-${CSS.escape(id)}"]`, root) || $(`[data-draft="cc-${CSS.escape(id)}"]`, root);
  if (btn.dataset.text) body.text = btn.dataset.text;
  else if (box && box.value.trim()) body.text = box.value.trim();
  if (decision === "answer" && !body.text) { toast("Write an answer first."); return; }
  btn.disabled = true;
  try {
    await api("POST", `/v1/inbox/${enc(id)}`, body);
    delete drafts[`ans-${id}`]; delete drafts[`rev-${id}`]; delete drafts[`cc-${id}`];
    toast({ endorse: "It is now a company fact.", approve: "Approved.", reject: decision === "reject" ? "Done." : "Done.", accept: "Accepted.", answer: "Sent.", dismiss: "Dismissed.", resume: "Continuing.", extend: "Budget doubled." }[decision] || "Done.");
    await refresh();
  } catch (e) { toast(e.message); btn.disabled = false; }
}

// Needs you: every decision waiting on the person (U1), most pressing
// first, each the same card as in its conversation, with a way there.
// Compact rows for the home feed: ended tasks link to their detail,
// items to the inbox below, where the open ones already wait (#367).
function feedRows(feed) {
  const rows = [];
  for (const t of feed.ended) rows.push(`<div class="feed-row"><a href="#/task/${enc(t.task_id)}">${esc(t.title || "A task")}</a> <span class="chip ${t.status === "failed" ? "bad" : "ok"}">${esc(t.status)}</span></div>`);
  for (const i of feed.arrived) rows.push(`<div class="feed-row"><a href="#/inbox">${esc(i.summary || i.kind)}</a> <span class="small muted">new ${esc(i.kind)}</span></div>`);
  for (const i of feed.decided) rows.push(`<div class="feed-row"><a href="#/inbox">${esc(i.summary || i.kind)}</a> <span class="small muted">${esc(i.resolution?.decision || "decided")}</span></div>`);
  return rows.join("");
}

function needsView(composed) {
  const mine = App.inbox, open = mine.filter(i => i.status === "open");
  // Presented observations and interruptions are recorded opened, once
  // each: the panel draws again on every refresh (#367). An id counts
  // as reported only after the request succeeds, so a transient
  // failure is retried on the next refresh instead of losing first
  // presentation for the session (#399); in-flight ids are held aside
  // so overlapping refreshes do not report twice.
  App.reported ??= new Set();
  App.reporting ??= new Set();
  const present = open.filter(i => (i.kind === "observation" || i.kind === "interrupt") && !App.reported.has(i.id) && !App.reporting.has(i.id));
  if (present.length) {
    for (const i of present) App.reporting.add(i.id);
    api("POST", "/v1/inbox/opened", { ids: present.map(i => i.id) }).then(
      () => { for (const i of present) { App.reporting.delete(i.id); App.reported.add(i.id); } },
      () => { for (const i of present) App.reporting.delete(i.id); },
    );
  }
  const now = open.filter(i => i.can_respond && i.blocking), later = open.filter(i => i.can_respond && !i.blocking), others = open.filter(i => !i.can_respond);
  const done = mine.filter(i => i.status !== "open").sort((a, b) => (b.created_at || "").localeCompare(a.created_at || "")).slice(0, 15);
  const feed = App.feed && (App.feed.ended.length || App.feed.arrived.length || App.feed.decided.length) ? `<h3>Since you were last here</h3>${feedRows(App.feed)}` : "";
  const where = i => {
    const t = i.thread && App.threads.find(x => x.thread === i.thread);
    return t ? `<div class="small where">In <a href="#/chat/${enc(t.thread)}">${esc(t.title)}</a></div>` : "";
  };
  const list = items => items.map(i => `<div class="needs-item" data-needs="${esc(i.id)}">${where(i)}${card(i, composed)}</div>`).join("");
  return `${feed}<h3>A task is waiting ${now.length ? `<span class="badge">${now.length}</span>` : ""}</h3>
    ${now.length ? list(now) : `<p class="empty">No task is waiting on you.</p>`}
    <h3>When you have a moment</h3>
    ${later.length ? list(later) : `<p class="empty">Nothing else.</p>`}
    ${others.length ? `<h3>Waiting on someone else</h3>${list(others)}` : ""}
    ${done.length ? `<details data-key="decided"><summary class="small muted">Decided (${done.length})</summary>${done.map(i => card(i, [])).join("")}</details>` : ""}`;
}

// ---------- Task detail ----------

function criteria(t) {
  if (!t.criteria?.length) return `<p class="empty">No contract yet.</p>`;
  return t.criteria.map(c => {
    const cls = c.verified === true ? "pass" : c.verified === false ? "fail" : "none";
    const person = String(c.verified_by || "").startsWith("person:") ? String(c.verified_by).slice(7) : null;
    const state = c.verified === true ? (person ? `<span class="chip ok">confirmed by ${esc(person)}</span>` : `<span class="chip ok">holds</span>`)
      : c.verified === false ? `<span class="chip bad">does not hold</span>` : c.check ? `<span class="chip">not checked yet</span>` : `<span class="chip warn">no check: needs a person</span>`;
    const by = c.verified_by && !person ? `<span class="small faint"> · checked by the appliance</span>` : "";
    const label = c.assurance ? ` <span class="chip ${c.assurance === "self-checked" ? "warn" : ""}" title="${esc((c.assurance_why || []).join("; "))}">${esc(c.assurance)}</span>` : "";
    return `<div class="evidence ${cls}"><div class="spread"><span>${esc(c.description)}</span><span>${state}${by}${label}</span></div>
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

// "Done: 3 appliance-verified, 1 self-checked" for a finished task.
function doneLine(t) {
  const a = t.assurance;
  if (t.status !== "completed" || !a) return "";
  const parts = [`${a.appliance_verified} appliance-verified`, `${a.self_checked} self-checked`];
  if (a.by_person) parts.push(`${a.by_person} confirmed by a person`);
  return `<p class="small" data-done-line>Done: ${esc(parts.join(", "))}</p>`;
}

function plan(t) {
  const units = t.plan?.units || [];
  if (!units.length) return `<p class="empty">The agent has not written a plan.</p>`;
  const mark = { done: "ok", active: "run", dropped: "", pending: "" };
  // Once the task has ended nothing is in progress: a unit the agent left
  // active or pending was simply not marked done.
  const ended = ["completed", "failed", "cancelled"].includes(t.status);
  const status = u => (ended && (!u.status || u.status === "active" || u.status === "pending")) ? ["", "not marked done"] : [mark[u.status] || "", u.status || "pending"];
  return `<p class="small faint">Version ${esc(t.plan_version)}. Statuses are the agent's own account; the contract's checks are the evidence.</p>
    <table class="t"><tbody>${units.map(u => { const [cls, label] = status(u); return `<tr><td class="mono small">${esc(u.id)}</td><td>${esc(u.goal)}<div class="small muted">Done when ${esc(u.done_when)}</div></td><td><span class="chip ${cls}" data-unit-status="${esc(label)}">${esc(label)}</span></td></tr>`; }).join("")}</tbody></table>`;
}

function contractSection(t) {
  if (!t.contract) return `<p class="empty">The agent has not written a contract.</p>`;
  const state = t.contract_approved_by ? `approved by ${String(t.contract_approved_by).replace(/^person:/, "")}` : t.contract_frozen ? "frozen when the work started" : "not frozen yet: the work has not started";
  return `<p class="small faint">Version ${esc(t.contract_version)} · ${esc(state)}</p>${contractView(t.contract, "Done-criteria and their checks")}`;
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

// What the outcome page draws first: the deliverable the agent composed
// (the summary when it gave none), then each delivered file it does not
// show, in full.
function deliverable(t) {
  const view = t.outcome_view ? [...t.outcome_view] : t.summary && t.status !== "running" ? [{ type: "text", text: t.summary }] : [];
  const shown = new Set(view.filter(c => c.type === "file" || c.type === "image").map(c => c.file));
  for (const o of latestOutputs(t)) if (!shown.has(o.name)) view.push({ type: "file", file: o.name });
  return view;
}

// ---------- The panel beside the conversation ----------

// Whatever the person opens (a task, a file, a schedule, a decision, or a
// view over them) shows here, beside the conversation, never instead of it.
// `what` is "kind:id" or a view's name.
const Panel = { what: null, layer: "outcome", seq: 0, live: null, liveFor: null, liveText: "", lib: { q: "", project: "" } };

function openPanel(what) {
  if (Panel.what !== what) { Panel.layer = "outcome"; stopPanelLive(); }
  Panel.what = what;
  const el = $("#detail");
  if (!el) return;
  // The panel glides in when it opens (as the motion setting allows).
  if (el.hidden) { el.classList.add("opening"); el.addEventListener("animationend", () => el.classList.remove("opening"), { once: true }); }
  el.hidden = false;
  document.body.classList.add("with-detail");
  sidebar();
  markOpen();
  return drawPanel();
}
// The conversation's row for what the panel shows is marked, without
// redrawing the thread.
function markOpen() {
  for (const r of document.querySelectorAll(".rg-row[data-open]")) r.classList.toggle("open", r.dataset.open === Panel.what);
}
function closePanel() {
  stopPanelLive();
  hush();
  Panel.what = null;
  Panel.rendered=null;Resources.forget("Work");
  const el = $("#detail");
  if (!el) return;
  el.hidden = true;
  document.body.classList.remove("with-detail");
  paint(el, "");
  markOpen();
  if ($("#side")) sidebar();
}
function stopPanelLive() {
  if (Panel.live) Panel.live();
  Panel.live = Panel.liveFor = null;
  Panel.liveText = "";
}

const PANEL_TITLES = { needs: "Needs you", tasks: "Tasks", schedules: "Schedules", files: "Files", projects: "Projects", task: "Task", file: "File", schedule: "Schedule", project: "Project", item: "Decision", search: "Search", connector: "Connector" };

async function drawPanel() {
  const el = $("#detail");
  if (!el || !Panel.what) return;
  const n = ++Panel.seq;
  const at = Panel.what.indexOf(":");
  const [kind, id] = at < 0 ? [Panel.what, ""] : [Panel.what.slice(0, at), Panel.what.slice(at + 1)];
  const composed = [];
  let body, after = null, head = null, failed=false;
  try {
    ({ body, after, head } = await PANELS[kind](id, composed));
  } catch (e) {
    failed=true;
    if(n===Panel.seq){Resources.bad("Work",e);if(Panel.rendered===Panel.what)return;}
    body = `<div class="banner bad">${esc(e.status === 404 ? "This is gone: it was deleted, or it is not yours." : e.message)}</div>`;
  }
  if (n !== Panel.seq || !Panel.what) return;
  if(!failed){Panel.rendered=Panel.what;Resources.good("Work");}
  const title = kind === "search" ? `Search: ${id}` : PANEL_TITLES[kind] || kind;
  // A view names itself in the head (a task: its title and where it
  // stands); the others are named by what they are.
  const changed = paint(el, `<div class="detail-head"><div class="dh-main">${head || `<h2>${esc(title)}</h2>`}</div><button class="icon-btn close" data-act="close-panel" title="Close">${icon("close")}<span class="sr">Close</span></button></div><div class="detail-body" data-panel="${esc(kind)}">${body}</div>`);
  if (!changed) return;
  for (const [iid, view, , task] of composed) fill($(`iframe[data-view="${CSS.escape(iid)}"]`, el), view, task);
  if (after) await after(el);
  loadGlimpses(el);
}

// A view whose content is already read (App's objects) or read here.
const plain = body => ({ body });
const PANELS = {
  needs: (_, composed) => plain(needsView(composed)),
  tasks: () => plain(tasksView()),
  schedules: () => plain(schedulesView()),
  schedule: id => plain(scheduleView(id)),
  item: (id, composed) => {
    const i = App.inbox.find(x => x.id === id);
    return plain(i ? card(i, composed) : `<p class="empty">This decision is gone.</p>`);
  },
  task: taskView,
  file: fileView,
  files: filesView,
  connector: connectorView,
  projects: projectsView,
  project: projectView,
  search: searchView,
};

// An app Magnus offered, with why: the same card as in the Library, as the
// person stands with it now, in a conversation, a task or an approval
// (connectors spec, Inside work). Only the person whose app it is gets its
// button; anyone else deciding sees where they stand.
function appOfferCard(e, theirs = true) {
  const app = { title: e.title, listing: { letters: e.letters, tint: e.tint } };
  const STATE = { needs_attention: "Needs signing in again", needs_more_access: "Needs more access", available: "Not connected" };
  const act = e.state === "connected" ? `<span class="chip ok">Connected</span>`
    : !theirs ? `<span class="chip warn">${esc(STATE[e.state] || "Not in their Library")}</span>`
    : e.state === "unavailable" ? `<span class="small muted">No longer in your Library.</span>`
    : e.ready === false ? `<span class="small muted">Not ready on this appliance yet.</span>`
    : e.state === "needs_attention" ? `<button class="primary" data-act="connect" data-app="${esc(e.connector)}">Sign in to ${esc(e.title || "")} again</button>`
    : e.state === "needs_more_access" ? `<button class="primary" data-act="connect" data-app="${esc(e.connector)}">Approve more access</button>`
    : `<button class="primary" data-act="connect" data-app="${esc(e.connector)}">Connect ${esc(e.title || "")}</button>`;
  return `<div class="confirm chatcard conncard" data-connector="${esc(e.connector)}" data-state="${esc(e.state)}"><div class="cc-head">${tile(app)}<div><strong>${esc(e.title || e.connector)}</strong>${e.publisher ? ` <span class="small muted">by ${esc(e.publisher)}</span>` : ""}</div></div>
    ${e.why ? `<p class="cc-text">${esc(e.why)}</p>` : ""}${e.summary ? `<p class="small muted">${esc(e.summary)}</p>` : ""}
    <div class="row cc-act">${act}</div></div>`;
}

// A task: its request, the decisions it waits on (the same cards as in its
// conversation), then its outcome, its contract and plan, or its trace.
async function taskView(id, composed) {
  const [t, tl] = await Promise.all([api("GET", `/v1/tasks/${enc(id)}`), Panel.layer === "trace" ? api("GET", `/v1/tasks/${enc(id)}/timeline`) : null]);
  const running = t.status === "running";
  if (running && Panel.liveFor !== id) {
    stopPanelLive();
    Panel.liveFor = id;
    Panel.live = Magnus.live(id, txt => { Panel.liveText += txt; const el = $("#plive"); if (el) el.textContent = Panel.liveText; });
  }
  if (!running && Panel.liveFor === id) stopPanelLive();
  const items = App.inbox.filter(i => i.task_id === id && i.status === "open");
  const view = deliverable(t);
  const files = latestOutputs(t);
  const cs = t.criteria || [];
  const checked = cs.filter(c => c.verified === true && !String(c.verified_by || "").startsWith("person:")).length;
  const cost = Number.isFinite(t.usage?.cost_usd) ? `$${t.usage.cost_usd.toFixed(2)}` : null;
  // The result leads with the deliverable, then its files, the person's
  // verdict, and the evidence; what it cost closes it.
  const outcome = `${view.length ? `<iframe class="composed" sandbox="allow-same-origin" title="The deliverable" id="pview"></iframe>` : running ? "" : `<p class="empty">No result yet.</p>`}
    ${t.spoken ? `<div class="row" style="margin:10px 0"><button data-act="speak">Read aloud</button><button data-act="hush">Stop</button></div>` : ""}
    ${files.length ? `<div class="frows">${files.map(f => fileRow(f, t.project)).join("")}</div>` : ""}
    ${t.outcome_view && t.summary ? `<h3>Summary</h3><p data-summary>${esc(t.summary)}</p>` : ""}
    <h3>Verification</h3>${doneLine(t)}<details class="verification-detail" data-key="checks:${esc(id)}" ${cs.some(c=>c.verified===false) ? "open" : ""}><summary>${esc(glance(t) || "No checks available")}</summary><div class="checklist">${criteria(t)}</div></details>
    ${ENDED.includes(t.status) ? verdictBar(t.task_id, t.verdict) : ""}
    ${cost ? `<p class="foot-note num" data-cost>${cost}</p>` : ""}
    ${t.model?.name || Number.isFinite(t.usage?.model_calls) || Number.isFinite(t.usage?.actions) ? `<details data-key="usage:${esc(id)}"><summary>Execution details</summary><p class="small muted">${[t.model?.name,Number.isFinite(t.usage?.model_calls) ? `${t.usage.model_calls} model calls` : null,Number.isFinite(t.usage?.actions) ? `${t.usage.actions} actions` : null].filter(Boolean).map(esc).join(" · ")}</p></details>` : ""}`;
  const contract = `<h3>Request</h3><p class="request">${esc(t.text)}</p>
    ${t.files?.length ? `<p class="small"><span class="muted">Your files:</span> ${t.files.map(fileChip).join(" ")}</p>` : ""}
    <h3>Contract</h3>${contractSection(t)}<h3>Plan</h3>${plan(t)}`;
  const layers = { outcome, contract, trace: tl ? trace(tl) : "" };
  const [, pill] = standing({ ...t, waiting_item: items.find(i => i.can_respond)?.id });
  const head = `<h2 class="ptitle" data-task="${esc(t.task_id)}">${esc(t.title || t.text)}</h2>
    <div class="row small dh-sub">${pill}${cs.length ? `<span class="checks">${icon("shield")}<span class="num">${checked} of ${cs.length} checks verified by the appliance</span></span>` : ""}${t.waiting_on ? `<span class="muted">${esc(WAITING[t.waiting_on] || "")}</span>` : ""}</div>`;
  const body = `${items.map(i => card(i, composed)).join("")}
    ${(t.app_offers || []).filter(o => o.state !== "connected").map(appOfferCard).join("")}
    ${running ? `<div class="live" id="plive" aria-live="polite">${esc(Panel.liveText) || `<span class="faint">Working…</span>`}</div>` : ""}
    <div class="seg layers" role="tablist" aria-label="What to show">${[["outcome", "Result"], ["contract", "Plan"], ["trace", "Steps"]].map(([k, l]) => `<button role="tab" aria-selected="${Panel.layer === k}" class="${Panel.layer === k ? "on" : ""}" data-layer="${k}">${l}</button>`).join("")}</div>
    <section class="layer">${layers[Panel.layer]}</section>
    <div class="row pactions">${moveControl(t.task_id, t.project, App.projects)}${!ENDED.includes(t.status) ? `<button data-act="cancel" data-id="${esc(id)}">Cancel task</button>` : trashButton(t.task_id, "Delete task")}${t.status === "suspended" && !t.waiting_item ? `<button data-act="resume" data-id="${esc(id)}">Resume</button>` : ""}</div>`;
  Panel.task = t;
  return { head, body, after: el => Panel.layer === "outcome" && view.length && fill($("#pview", el), view, id, t) };
}

// A delivered file as a row: its type, its name (opens it), where it is
// kept, and Download.
const FILE_TYPES = { "application/pdf": "PDF", "text/csv": "CSV", "text/markdown": "MD", "text/plain": "TXT", "text/html": "HTML", "application/json": "JSON" };
function fileRow(f, project) {
  const type = FILE_TYPES[f.media_type] || (f.media_type || "").split("/").pop().slice(0, 4).toUpperCase() || "FILE";
  const p = project && (typeof project === "object" ? project : App.projects.find(x => x.id === project));
  return `<div class="frow" data-file-row="${esc(f.id)}"><span class="ftype ${type === "PDF" ? "pdf" : ""}">${esc(type)}</span>
    <div class="rg-main"><a class="ttl" href="#/file/${enc(f.id)}" data-open="file:${esc(f.id)}">${esc(f.name)}</a><span class="sub">Saved to Files${p ? ` · ${esc(p.name)}` : ""} · ${esc(kb(f.size))}</span></div>
    <button class="icon-btn" data-file-download="${esc(f.id)}" data-name="${esc(f.name)}" title="Download ${esc(f.name)}">${icon("download")}<span class="sr">Download</span></button></div>`;
}

// One file, drawn the way an outcome draws it, with where it came from and
// a way to save it.
async function fileView(id) {
  const f = await api("GET", `/v1/files/${enc(id)}/info`);
  const path = `/v1/files/${enc(id)}`;
  const body = `<h2 class="ptitle">${esc(f.name)}</h2>
    <p class="small muted">${f.source === "delivered" ? "Delivered by Magnus" : "Uploaded by you"} · ${fileFrom(f)} · ${esc(kb(f.size))} · ${esc(when(f.at))}</p>
    <div class="row"><button class="primary" data-file-download="${esc(f.id)}" data-name="${esc(f.name)}">Download</button> ${moveControl(f.id, f.project, App.projects)} ${trashButton(f.id, "Delete file")}</div>
    <iframe class="composed" sandbox="allow-same-origin" title="${esc(f.name)}" id="fview"></iframe>`;
  return {
    body,
    after: async el => {
      const files = {};
      try {
        if (GLIMPSE_IMAGE.test(f.media_type)) files[f.name] = { image: await Magnus.dataUrl(path, f.media_type) };
        else if (Compose.TEXT.test(f.media_type)) files[f.name] = { text: decodeText(await Magnus.dataUrl(path, f.media_type)), media_type: f.media_type };
      } catch (e) { toast(e.message); }
      const frame = $("#fview", el);
      frame.srcdoc = Compose.documentFor([{ type: "file", file: f.name }], files, fontCss(), f.preview, ...look());
      frame.onload = () => { try { frame.style.height = frame.contentDocument.documentElement.scrollHeight + 4 + "px"; } catch { /* sized by CSS */ } };
    },
  };
}

// ---------- The Library: the connector catalog ----------

// The Library is a catalog of apps, laid out as the owner's catalog mockup
// draws it in every look (connectors spec rev 48, Library): Discover shows
// the whole catalog, Yours what this person has connected or needs to fix;
// one search box and category chips; cards in sections, each with its tile,
// the review mark, one plain sentence, who makes it, and one action at its
// corner. Connecting is a click and the person's own login on the
// provider's page: nothing is typed in Magnus but the search.
const Lib = { tab: "discover", cat: "all", q: "" };
const CATEGORIES = [["all", "All"], ["email", "Email & chat"], ["files", "Files"], ["code", "Code"], ["work", "Work"], ["design", "Design & CAD"]];
const Connecting = {};
const SVG = {
  plus: `<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M12 5v14M5 12h14"/></svg>`,
  check: `<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 12.5l4.5 4.5L19 7.5"/></svg>`,
  again: `<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 12a8 8 0 1 1-2.4-5.7"/><path d="M20 4v5h-5"/></svg>`,
  lock: `<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/></svg>`,
  search: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><circle cx="11" cy="11" r="7"/><path d="M20 20l-3.5-3.5"/></svg>`,
};
const reviewed = `<svg class="lc-rev" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" role="img" aria-label="Reviewed by Alpha Vector"><path d="M12 2.5l2.3 1.7 2.8-.2 1 2.6 2.4 1.5-.7 2.8 1 2.6-2.3 1.7-.5 2.8-2.8.4-1.9 2.1-2.6-1.1-2.6 1.1-1.9-2.1-2.8-.4-.5-2.8L2.6 15.2l1-2.6-.7-2.8 2.4-1.5 1-2.6 2.8.2z"/><path d="M8.5 12.2l2.3 2.3 4.7-4.8"/></svg>`;
const stateOf = c => (Connecting[c.name] ? "connecting" : c.state);
const tintOf = c => (/^#[0-9a-f]{6}$/i.test(c.listing?.tint || "") ? c.listing.tint : "#3A3F48");
const tile = (c, big) => `<span class="ctile${big ? " big" : ""}" aria-hidden="true" style="background:${tintOf(c)}">${esc(c.listing?.letters || (c.title || "?").slice(0, 2))}</span>`;
const byline = c => (c.listing?.publisher ? `by ${c.listing.publisher}` : "Added by your organisation");
const provider = c => c.listing?.publisher || c.title;

function connectorAction(c) {
  const name = esc(c.title);
  switch (stateOf(c)) {
    case "connecting": return `<span class="lc-act busy" role="status" aria-label="Signing in to ${name}"><span class="spin"></span></span>`;
    case "connected": return c.more_access
      ? `<button class="lc-act warn" data-act="connect" data-app="${esc(c.name)}" aria-label="Approve more access for ${name}">${SVG.again}</button>`
      : `<a class="lc-act ok" href="#/library/${enc(c.name)}" aria-label="${name} is connected. Open details">${SVG.check}</a>`;
    case "needs_attention": return `<button class="lc-act warn" data-act="connect" data-app="${esc(c.name)}" aria-label="Sign in to ${name} again">${SVG.again}</button>`;
    case "local": return `<button class="primary lc-enable" data-act="connect" data-app="${esc(c.name)}">Enable</button>`;
    default: return c.ready === false
      ? `<span class="lc-act off" title="Not ready on this appliance yet" aria-label="${name} is not ready on this appliance yet">${SVG.plus}</span>`
      : `<button class="lc-act" data-act="connect" data-app="${esc(c.name)}" aria-label="Connect ${name}">${SVG.plus}</button>`;
  }
}

function connectorCard(c) {
  const st = stateOf(c);
  const line = {
    connecting: `<p class="lc-note">Finish signing in on ${esc(provider(c))}'s page… <button class="link" data-act="connect-page" data-url="${esc(Connecting[c.name] || "")}">Open it again</button></p>`,
    needs_attention: `<p class="lc-note warn">Sign in again to keep using ${esc(c.title)}.</p>`,
    connected: c.account ? `<p class="lc-note ok">Connected as ${esc(c.account)}</p>` : "",
  }[st] || "";
  return `<article class="lcard" data-connector="${esc(c.name)}" data-state="${esc(st)}">${tile(c)}
    <div class="lc-main"><div class="lc-name"><h3><a href="#/library/${enc(c.name)}">${esc(c.title)}</a></h3>${c.listing?.reviewed ? reviewed : ""}</div>
    <p class="lc-sum">${esc(c.summary)}</p>${line}<p class="lc-by">${esc(byline(c))}</p></div>${connectorAction(c)}</article>`;
}

// What a search or a chip leaves: a service's name, or what the person
// wants done, against its name, sentence, publisher and abilities.
function libMatches(c) {
  if (Lib.cat !== "all" && (c.listing?.category || "work") !== Lib.cat) return false;
  const words = Lib.q.toLowerCase().split(/\s+/).filter(Boolean);
  const hay = [c.title, c.summary, c.listing?.publisher, ...(c.listing?.abilities || [])].join(" ").toLowerCase();
  return words.every(w => hay.includes(w));
}

function librarySections(all) {
  const shown = all.filter(libMatches);
  const of = (...states) => shown.filter(c => states.includes(stateOf(c)));
  const sections = Lib.tab === "discover"
    ? [["local", "On this computer", "Programs Magnus found here. Enable one and Magnus can work in it.", of("local")],
       ["attention", "Needs attention", "Sign in again and Magnus picks up where it left off.", of("needs_attention")],
       ["popular", "Popular", "Connect any of these by signing in to your own account.", of("available", "connected", "connecting")]]
    : [["connected", "Connected", "What Magnus can work in for you right now.", of("connected", "connecting")],
       ["attention", "Needs attention", "Sign in again and Magnus picks up where it left off.", of("needs_attention")],
       ["local", "Ready to enable", "Programs on this computer you have not turned on yet.", of("local")]];
  const html = sections.filter(x => x[3].length).map(([k, title, sub, items]) => `<section class="lsect" data-section="${k}">
    <div class="lsect-head"><div><h2>${title}</h2><p>${sub}</p></div><span class="lsect-n">${items.length === 1 ? "1 connector" : `${items.length} connectors`}</span></div>
    <div class="lgrid">${items.map(connectorCard).join("")}</div></section>`).join("");
  if (html) return html;
  return `<div class="lempty">${Lib.q || Lib.cat !== "all" ? "No connectors match. Try another word, or ask Magnus what you want done."
    : Lib.tab === "yours" ? "Nothing connected yet. Discover shows every app Magnus can work in." : "Your organisation's catalog is empty."}</div>`;
}

async function showLibrary(main, name) {
  Lib.main = main;
  if (name) return showConnectorPage(main, name);
  const { connectors } = await api("GET", "/v1/connectors");
  // An app blocked for your groups is not in your Library; the owner still
  // manages it in Settings.
  Lib.all = connectors.filter(c => c.for_you !== false);
  const seg = (k, label) => `<button data-lib-tab="${k}" aria-pressed="${Lib.tab === k}">${label}</button>`;
  main.innerHTML = `<div class="libpage"><header class="lhead"><div><h1>Library</h1><p>Connect the apps and programs you want Magnus to work in. You only ever sign in.</p></div>
    <a class="icon-btn lclose" href="#/home" aria-label="Close Library">${icon("close")}</a></header>
    <div class="lbar"><div class="lseg" role="group" aria-label="Show">${seg("yours", "Yours")}${seg("discover", "Discover")}</div>
    <div class="lsearch">${SVG.search}<label class="sr" for="libq">Search connectors</label><input id="libq" type="search" value="${esc(Lib.q)}" placeholder="Search connectors, or describe what you want done" autocomplete="off"></div></div>
    <div class="lchips" role="group" aria-label="Category">${CATEGORIES.map(([k, l]) => `<button data-lib-cat="${k}" aria-pressed="${Lib.cat === k}">${esc(l)}</button>`).join("")}</div>
    <div id="lib-sections">${librarySections(connectors)}</div></div>`;
  bindConnectorActs(main);
  let t;
  $("#libq", main).addEventListener("input", e => { clearTimeout(t); t = setTimeout(() => { Lib.q = e.target.value.trim(); $("#lib-sections", main).innerHTML = librarySections(Lib.all); }, 150); });
}

// One connector's page, in the two states the mockup draws: before
// connecting, what Magnus can do with it and what it asks for; connected,
// as whom, what it can and cannot do, when, what Magnus did, Disconnect.
function connectorBody(c) {
  const st = stateOf(c), L = c.listing || {};
  const head = `<div class="cp-head">${tile(c, true)}<div><div class="cp-name"><h2 class="ptitle">${esc(c.title)}</h2>${st === "connected" ? `<span class="chip ok">${SVG.check}Connected</span>` : ""}</div>
    <p class="muted">${esc(byline(c))}${L.reviewed ? " · Reviewed by Alpha Vector" : ""}</p></div></div>`;
  const recent = `<h3>Recent actions</h3>${c.recent?.length ? `<ol class="cp-recent">${c.recent.map(u => `<li><a href="#/task/${enc(u.task)}" data-open="task:${esc(u.task)}">${esc(u.action.charAt(0).toUpperCase() + u.action.slice(1))}</a><span class="small faint">${esc(when(u.at))}${u.worked === false ? " · did not work" : ""}</span></li>`).join("")}</ol>` : `<p class="empty">Magnus has not used it for you yet.</p>`}`;
  if (st === "connected" || (st === "needs_attention" && c.connected_at)) {
    return `${head}${st === "needs_attention" ? `<p class="lc-note warn">Your sign-in to ${esc(c.title)} stopped working. Sign in again to keep using it.</p>` : ""}
      <dl class="kv cp-kv"><dt>Account</dt><dd>${esc(c.account || "Your own account")}</dd><dt>It can</dt><dd>${esc(L.can || c.summary)}</dd>${L.cannot ? `<dt>It cannot</dt><dd>${esc(L.cannot)}</dd>` : ""}
      ${c.connected_at ? `<dt>Connected</dt><dd>${esc(when(c.connected_at))}</dd>` : ""}<dt>Last used</dt><dd>${c.last_used ? esc(when(c.last_used)) : "Not yet"}</dd></dl>
      ${recent}<div class="row cp-actions">${st === "needs_attention" ? `<button class="primary" data-act="connect" data-app="${esc(c.name)}">Sign in to ${esc(c.title)} again</button>` : ""}${c.signs_in ? `<button data-act="disconnect" data-app="${esc(c.name)}">Disconnect ${esc(c.title)}</button>` : ""}</div>`;
  }
  const can = L.abilities?.length ? L.abilities : c.allows || [];
  const go = st === "connecting"
    ? `<p class="lc-note">Finish signing in on ${esc(provider(c))}'s page, then come back here. <button class="link" data-act="connect-page" data-url="${esc(Connecting[c.name] || "")}">Open it again</button></p>`
    : c.ready === false ? `<p class="small muted">${esc(c.title)} is not ready on this appliance yet.</p>`
    : `<button class="primary cp-connect" data-act="connect" data-app="${esc(c.name)}">${st === "needs_attention" ? `Sign in to ${esc(c.title)} again` : `Connect ${esc(c.title)}`}</button>`;
  return `${head}<h3>What Magnus can do with ${esc(c.title)}</h3>${can.length ? `<ul class="cp-list">${can.map(l => `<li>${esc(l)}</li>`).join("")}</ul>` : `<p class="empty">Nothing yet: your organisation has not chosen what Magnus may do with it.</p>`}
    ${L.asks?.length ? `<h3>What it asks for</h3><ul class="cp-list">${L.asks.map(l => `<li>${esc(l)}</li>`).join("")}</ul>` : ""}
    <p class="cp-lock">${SVG.lock}<span>You sign in on ${esc(provider(c))}'s own page. Magnus never sees your password, and you can disconnect at any time.</span></p>${go}`;
}

async function showConnectorPage(main, name) {
  const c = await api("GET", `/v1/connectors/${enc(name)}`);
  main.innerHTML = `<div class="libpage cpage"><p class="small"><a href="#/library">Library</a></p><section class="cp" aria-label="${esc(c.title)}">${connectorBody(c)}</section></div>`;
  bindConnectorActs(main);
}

// The same connector in the panel, from a card in chat or a task.
async function connectorView(name) {
  return plain(`<section class="cp">${connectorBody(await api("GET", `/v1/connectors/${enc(name)}`))}</section>`);
}

// What Disconnect did: ended at the app, or, where the app could not end it
// itself, ended here, with the app's own page where the person removes
// Magnus (connectors spec, Lifecycle).
function disconnected(r) {
  if (!r || r.at_provider !== false) { toast("Disconnected. Your sign-in is ended at the app too."); return; }
  toast(r.remove_at ? `Disconnected here. The app could not end your sign-in itself: remove Magnus at ${r.remove_at}` : "Disconnected here. The app could not end your sign-in itself: remove Magnus in its own settings.", 10000);
}

// Connect, again or for the first time, and Disconnect, wherever a card is.
function bindConnectorActs(root) {
  if (root.dataset.connActs) return;
  root.dataset.connActs = "1";
  root.addEventListener("click", async e => {
    const tab = e.target.closest("[data-lib-tab]"), cat = e.target.closest("[data-lib-cat]");
    if (tab || cat) {
      if (tab) Lib.tab = tab.dataset.libTab; else Lib.cat = cat.dataset.libCat;
      return showLibrary(root);
    }
    const a = e.target.closest("[data-act]");
    if (!a || !root.contains(a)) return;
    if (a.dataset.act === "connect") { e.preventDefault(); connectApp(a.dataset.app).catch(err => toast(err.message)); }
    if (a.dataset.act === "connect-page") { e.preventDefault(); Magnus.openExternal(a.dataset.url).catch(() => {}); }
    if (a.dataset.act === "disconnect") {
      e.preventDefault();
      if (!await ask("Disconnect? Your sign-in is ended at the app, and Magnus can no longer use it for you until you connect again.", "Disconnect")) return;
      try { disconnected(await api("DELETE", `/v1/connectors/${enc(a.dataset.app)}/connection`)); connectorsChanged(); } catch (err) { toast(err.message); }
    }
  });
}

// Wherever connectors are shown, they are drawn again.
function connectorsChanged() {
  const m = /^#\/library(?:\/([^/]+))?$/.exec(location.hash);
  if (m && Lib.main?.isConnected) showLibrary(Lib.main, m[1]);
  drawPanel();
}

// Connect: the system browser opens at the app's own sign-in page; the
// answer comes back to this computer, and the card turns Connected.
function onThisComputer(u) {
  try {
    const x = new URL(u);
    return x.protocol === "http:" && x.hostname === "127.0.0.1" && x.pathname === "/callback" && x.port;
  } catch { return false; }
}

async function connectApp(name) {
  if (!Magnus.loopbackOpen) { toast("Connect apps from the Magnus desktop app."); return; }
  // Slack's registered address is https. The desktop listens on the handoff,
  // and the sign-in tells Slack the https address, which the exchange repeats.
  let card = (Lib.all || []).find(c => c.name === name);
  if (!card) { try { card = await api("GET", `/v1/connectors/${enc(name)}`); } catch { card = null; } }
  const listenAt = card && (card.handoff || (onThisComputer(card.redirect_uri) ? card.redirect_uri : null));
  const caught = await Magnus.loopbackOpen(listenAt || null);
  const redirect = card && card.redirect_uri && !onThisComputer(card.redirect_uri) ? card.redirect_uri : caught;
  const r = await api("POST", `/v1/connectors/${enc(name)}/sign-in`, { redirect_uri: redirect });
  Connecting[name] = r.authorize_url;
  connectorsChanged();
  Magnus.openExternal(r.authorize_url).catch(() => {});
  try {
    const got = await Magnus.loopbackWait();
    if (got.state !== r.state) throw new Error("That sign-in came back for another request. Try again.");
    await api("POST", `/v1/connectors/${enc(name)}/sign-in/complete`, { state: got.state, code: got.code, iss: got.iss });
    toast("Connected.");
  } catch (e) { toast(e.message); }
  finally { delete Connecting[name]; connectorsChanged(); }
}

// Files: every file uploaded or delivered, as a view over the same files
// the conversations show.
async function filesView() {
  const { q, project } = Panel.lib;
  const qs = new URLSearchParams();
  if (q) qs.set("q", q);
  if (project) qs.set("project", project);
  const { files } = await api("GET", `/v1/files${qs.size ? `?${qs}` : ""}`);
  return plain(`<div class="row lib-bar"><label class="sr" for="lq">Search files</label><input id="lq" type="search" data-draft="lib-q" placeholder="Files, tasks, projects or text" autocomplete="off">
    <label class="sr" for="lp">Project</label><select id="lp" aria-label="Filter by project"><option value="">All files</option><option value="none"${project === "none" ? " selected" : ""}>In no project</option>${App.projects.map(p => `<option value="${esc(p.id)}"${p.id === project ? " selected" : ""}>${esc(p.name)}</option>`).join("")}</select>
    ${attachButton("Upload")}</div>
    ${files.length ? `<ul class="flist">${files.map(f => `<li data-lib-file="${esc(f.id)}"><div class="rg-main">${fileChip(f)}<span class="small muted">${f.source === "delivered" ? "Delivered by Magnus" : "Uploaded by you"} · ${esc(when(f.at))} · ${fileFrom(f)}</span></div>${moveControl(f.id, f.project, App.projects, true)}</li>`).join("")}</ul>`
      : `<p class="empty">${q || project ? "No file matches." : "No files yet. Upload one, or ask Magnus for a document."}</p>`}`);
}

function projectsView() {
  return plain(`${App.projects.length ? `<ul class="plist">${App.projects.map(p => `<li><a href="#/project/${enc(p.id)}" data-open="project:${esc(p.id)}"><strong>${esc(p.name)}</strong><span class="small muted">${p.counts.tasks} tasks · ${p.counts.files} files · ${p.counts.schedules} schedules</span></a></li>`).join("")}</ul>` : `<p class="empty">No projects yet.</p>`}
    <form id="np" class="card flat"><h3>New project</h3><p class="small muted">A project groups conversations, tasks, schedules and files. Its instructions and files apply to everything in it.</p><label>Name <input id="pn" required></label>
    <label>Instructions for everything in it <span class="faint small">(optional)</span><textarea id="pi" rows="3" data-draft="new-project" placeholder="For example: Prices in euros. Keep every document to one page."></textarea></label>
    <div class="row"><button class="primary">Create</button></div></form>`);
}

async function projectView(id) {
  const p = await api("GET", `/v1/projects/${enc(id)}`);
  return plain(`<div class="spread"><h2 class="ptitle">${esc(p.name)}</h2>${trashButton(id, "Delete project")}</div>
    <div class="row"><a class="button primary" href="#/chat/new/${enc(id)}">New conversation in this project</a></div>
    <h3>Instructions</h3><p class="small muted">Applied to every task and conversation in this project. Only you change them; Magnus reads them.</p>
      <textarea id="pin" rows="4" data-draft="pin-${esc(id)}">${esc(p.instructions || "")}</textarea><div class="row"><button data-act="save-ins" data-id="${esc(id)}">Save instructions</button></div>
    <div class="spread"><h3>Files</h3>${attachButton("Upload to this project")}</div>
      ${p.files.length ? `<ul class="flist">${p.files.map(f => `<li data-project-file="${esc(f.id)}">${fileChip(f)} <span class="small faint">${f.source === "delivered" ? "delivered" : "uploaded"} ${esc(when(f.at))}</span></li>`).join("")}</ul>` : `<p class="empty">No files. Tasks here read the project's files when they need them.</p>`}
    <h3>Conversations</h3>${p.threads.length ? `<ul class="tlist">${p.threads.map(t => `<li class="trow"><a href="#/chat/${enc(t.thread)}"><span class="ttl">${esc(t.title)}</span><span class="small faint when">${esc(when(t.updated_at))}</span></a></li>`).join("")}</ul>` : `<p class="empty">No conversations.</p>`}
    <h3>Tasks</h3>${p.tasks.length ? `<ul class="tlist">${p.tasks.map(taskRow).join("")}</ul>` : `<p class="empty">No tasks.</p>`}
    <h3>Schedules</h3>${p.schedules.length ? `<ul class="tlist">${p.schedules.map(x => `<li class="trow"><a href="#/schedules" data-open="schedule:${esc(x.id)}"><span class="ttl">${esc(x.title)}</span><span class="small faint when">Next ${esc(until(x.next))}</span></a></li>`).join("")}</ul>` : `<p class="empty">No schedules.</p>`}`);
}

// One search across conversations, tasks and files.
async function searchView(q) {
  const r = await api("GET", `/v1/search?q=${enc(q)}`);
  const none = !r.conversations.length && !r.tasks.length && !r.files.length;
  return plain(none ? `<p class="empty">Nothing matches “${esc(q)}”.</p>` : `
    ${r.conversations.length ? `<h3>Conversations</h3><ul class="tlist">${r.conversations.map(c => `<li class="trow found" data-found-thread="${esc(c.thread)}"><a href="#/chat/${enc(c.thread)}"><span class="ttl">${esc(c.title)}</span><span class="small faint when">${esc(when(c.updated_at))}</span></a>${c.snippet ? `<div class="small muted snippet">${esc(c.snippet)}</div>` : ""}</li>`).join("")}</ul>` : ""}
    ${r.tasks.length ? `<h3>Tasks</h3><ul class="tlist">${r.tasks.map(taskRow).join("")}</ul>` : ""}
    ${r.files.length ? `<h3>Files</h3><ul class="flist">${r.files.map(f => `<li data-lib-file="${esc(f.id)}">${fileChip(f)}<span class="small">${fileFrom(f)}</span></li>`).join("")}</ul>` : ""}`);
}

// What can be done in the panel, whichever view it shows.
function wirePanel() {
  const el = $("#detail");
  el.addEventListener("click", async e => {
    if (e.target.closest("[data-act=close-panel]")) return closePanel();
    const l = e.target.closest("[data-layer]");
    if (l) { Panel.layer = l.dataset.layer; return drawPanel(); }
    const m = e.target.closest("[data-more]");
    if (m) { MORE.add(m.dataset.more); return drawPanel(); }
    const a = e.target.closest("button[data-act]");
    if (!a) return;
    const id = a.dataset.id;
    try {
      switch (a.dataset.act) {
        case "speak": await readAloud(Panel.task.spoken); break;
        case "hush": await hush(); break;
        case "cancel":
          if (!await ask("Cancel this task? What it already did stays done.", "Cancel task")) return;
          await api("POST", `/v1/tasks/${enc(id)}/cancel`, {}); break;
        case "resume": await api("POST", `/v1/tasks/${enc(id)}/resume`, {}); break;
        case "stop-schedule":
          if (!await ask("Stop this schedule? No more of its runs start. Active runs stop too. Work already done stays done, and the runs stay in Tasks.", "Stop schedule")) return;
          await api("DELETE", `/v1/schedules/${enc(id)}`); toast("The schedule is stopped."); break;
        case "delete-schedule":
          if (!await ask("Delete this schedule? It has ended, so nothing stops; its runs stay in Tasks.", "Delete schedule")) return;
          await api("DELETE", `/v1/schedules/${enc(id)}`); toast("The schedule is deleted."); break;
        case "resume-schedule": await api("POST", `/v1/schedules/${enc(id)}/resume`, {}); toast("Schedule resumed."); break;
        case "connect": connectApp(a.dataset.app).catch(err => toast(err.message)); return;
        case "connect-page": await Magnus.openExternal(a.dataset.url); return;
        case "disconnect":
          if (!await ask("Disconnect? Your sign-in is ended at the app, and Magnus can no longer use it for you until you connect again.", "Disconnect")) return;
          disconnected(await api("DELETE", `/v1/connectors/${enc(a.dataset.app)}/connection`)); break;
        case "save-ins":
          await api("PUT", `/v1/projects/${enc(id)}`, { instructions: $("#pin", el).value }); delete drafts[`pin-${id}`];
          toast("Instructions saved. New tasks here use them."); break;
        default: return;
      }
      await refresh();
    } catch (err) { toast(err.message); }
  });
  let t;
  el.addEventListener("input", e => {
    if (e.target.id !== "lq") return;
    clearTimeout(t);
    t = setTimeout(() => { Panel.lib.q = e.target.value.trim(); drawPanel(); }, 200);
  });
  el.addEventListener("change", async e => {
    if (e.target.id === "lp") { Panel.lib.project = e.target.value; return drawPanel(); }
    if (!e.target.matches("[data-attach]")) return;
    const project = Panel.what.startsWith("project:") ? Panel.what.slice(8) : Panel.lib.project && Panel.lib.project !== "none" ? Panel.lib.project : null;
    try { const up = await upload(e.target.files, project ? { project } : {}); toast(`Uploaded ${up.map(f => f.name).join(", ")}.`); } catch (err) { toast(err.message); }
    e.target.value = "";
    refresh();
  });
  el.addEventListener("submit", async e => {
    if (e.target.id !== "np") return;
    e.preventDefault();
    try {
      const r = await api("POST", "/v1/projects", { name: $("#pn", el).value, instructions: $("#pi", el).value });
      delete drafts["new-project"];
      await refresh();
      openPanel(`project:${r.id}`);
    } catch (err) { toast(err.message); }
  });
}

// ---------- Trash ----------

const TRASHED = { task: "Task", conversation: "Conversation", message: "Chat message", file: "File", project: "Project", memory: "Memory", knowledge: "Knowledge entry" };
route(/^#\/trash$/, async main => {
  const draw = async () => {
    const { items, keep_days } = await api("GET", "/v1/trash");
    paint(main, `<h1>Trash</h1><p class="muted">What you delete waits here for ${esc(keep_days)} days, then it is gone for good. Restore puts it back where it was.</p>
      ${items.length ? `<div class="row"><button data-act="empty">Empty trash now</button></div>
      <table class="t trash"><thead><tr><th>Deleted</th><th>When</th><th>Gone for good</th><th></th></tr></thead><tbody>${items.map(i => `<tr data-trashed="${esc(i.item)}">
        <td><div class="small faint">${esc(TRASHED[i.kind] || i.kind)}</div><div data-title>${esc(i.title) || `<span class="faint">Untitled</span>`}</div></td>
        <td class="small faint n">${esc(when(i.trashed_at))}</td><td class="small faint n">${esc(new Date(i.removed_at).toLocaleDateString())}</td>
        <td><div class="row"><button data-act="restore">Restore</button><button class="link" data-act="remove">Delete now</button></div></td></tr>`).join("")}</tbody></table>`
      : `<p class="empty">The trash is empty.</p>`}`);
  };
  main.onclick = async e => {
    const b = e.target.closest("button[data-act]"); if (!b) return;
    const item = b.closest("[data-trashed]")?.dataset.trashed;
    try {
      if (b.dataset.act === "restore") { b.disabled = true; await api("POST", `/v1/trash/${enc(item)}/restore`, {}); toast("Restored."); }
      if (b.dataset.act === "remove") {
        if (!await ask("Delete this for good? It cannot be restored.", "Delete for good")) return;
        b.disabled = true; await api("DELETE", `/v1/trash/${enc(item)}`); toast("Deleted for good.");
      }
      if (b.dataset.act === "empty") {
        if (!await ask("Delete everything in the trash for good? None of it can be restored.", "Delete everything")) return;
        b.disabled = true; const r = await api("POST", "/v1/trash/empty", {}); toast(`${r.removed} deleted for good.`);
      }
      await draw();
    } catch (err) { toast(err.message); b.disabled = false; }
  };
  App.redraw = draw;
  onLeave(() => { App.redraw = null; });
  await draw();
});

// ---------- Models ----------

const signins = {};
async function modelsSection(main) {
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
    paint(main, `<h2>Models</h2><p class="muted">Connect providers here. Keys and sign-ins go to the appliance's vault; this app keeps none.</p>
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
}

// ---------- Connectors: the organisation's apps (owner) ----------

// The organisation's apps: the whole catalog, already set up, and any
// server the admin added (in the admin console, never here). Nothing here
// asks for a client ID, secret or address (connectors spec rev 46, "What
// the user sees"); each person connects from their Library. The owner
// decides what an added server's tools may do; a tool left "not offered"
// is never shown to the worker.
const RISKS = [["", "Not offered"], ["read", "Reads only"], ["reversible_write", "Changes things (undoable)"], ["irreversible", "Changes things (cannot undo)"], ["financial", "Moves money"], ["prohibited", "Never"]];
// A tool whose definition changed is blocked until the owner accepts it;
// the decision is the inbox item's, shown again on its connector.
const changesFor = name => App.inbox.filter(i => i.status === "open" && (i.kind === "tool_change" && String(i.details?.tool).startsWith(`${name}__`)
  || i.kind === "tool_review" && (i.details?.tools || []).some(t => String(t.tool).startsWith(`${name}__`))));
async function connectorsSection(main) {
  const draw = async () => {
    const { connectors } = await api("GET", "/v1/connectors");
    paint(main, `<h2>Connectors</h2><p class="muted">The apps in Magnus's catalog, already set up for your organisation. Each is in everyone's Library, and each person connects it with their own account; no one acts through anyone else's.</p>
      ${connectors.map(c => `<div class="card" data-app="${esc(c.name)}"><div class="spread"><strong>${esc(c.title)}</strong>
          <span class="row">${c.source === "config" ? `<span class="chip">set in the config file</span>` : c.source === "catalog" ? `<span class="chip">from the catalog</span>` : `<button class="link" data-act="remove" data-app="${esc(c.name)}">Remove for everyone</button>`}</span></div>
        <div class="small muted">${esc(c.summary)}</div>
        ${changesFor(c.name).map(i => card(i, [])).join("")}
        ${c.tools.length ? `<table class="t small" style="margin-top:10px"><thead><tr><th>Tool</th><th>What the app says it does</th><th>May</th></tr></thead><tbody>
          ${c.tools.map(t => `<tr><td class="mono">${esc(t.name)}</td><td class="muted">${esc(t.description)}</td>
            <td><select data-tool="${esc(t.name)}" data-app="${esc(c.name)}" aria-label="What ${esc(t.name)} may do" ${c.source === "app" ? "" : "disabled"}>
              ${RISKS.map(([v, l]) => `<option value="${v}" ${(t.risk || "") === v ? "selected" : ""}>${esc(l)}${v && v === t.suggested && !t.risk ? " (the app's hint)" : ""}</option>`).join("")}</select></td></tr>`).join("")}</tbody></table>`
          : c.signed_in ? "" : `<p class="small">Connect it in your Library to see its tools and choose what each may do.</p>`}
      </div>`).join("") || `<p class="empty">No apps yet.</p>`}`);
  };
  main.onchange = async e => {
    const s = e.target.closest("select[data-tool]"); if (!s) return;
    try {
      await api("PUT", `/v1/connectors/${enc(s.dataset.app)}/tools/${enc(s.dataset.tool)}`, { risk: s.value || null });
      toast(s.value ? `${s.dataset.tool}: ${RISKS.find(r => r[0] === s.value)[1].toLowerCase()}.` : `${s.dataset.tool} is not offered to the worker.`);
    } catch (err) { toast(err.message); draw(); }
  };
  main.onclick = async e => {
    const it = e.target.closest("button[data-item]");
    if (it) { await respond(it, main); return draw(); }
    const b = e.target.closest("button[data-act]"); if (!b) return;
    const app = b.dataset.app;
    try {
      if (b.dataset.act === "remove") {
        if (!await ask(`Remove ${app} for everyone? Every person's sign-in to it is ended, and the worker can no longer use it.`, "Remove")) return;
        await api("DELETE", `/v1/connectors/${enc(app)}`);
        await draw();
      }
    } catch (err) { toast(err.message); b.disabled = false; }
  };
  await draw();
  every(5000, draw);
}

// ---------- Settings ----------

// Everything a person can set, in one place; developer views stay in their
// own group. Sections that need the owner role are shown only to owners.
const SETTINGS = [
  ["models", "Models", modelsSection, true], ["connectors", "Connectors", connectorsSection, true], ["account", "Account", accountSection],
  ["appearance", "Appearance", appearanceSection], ["notifications", "Notifications", notificationsSection], ["updates", "Updates", updatesSection], ["data", "Data and privacy", dataSection],
  ["safety", "Kill switch", killSwitchSection, true],
];
route(/^#\/settings(?:\/([a-z]+))?$/, async (main, which) => {
  const shown = SETTINGS.filter(x => !x[3] || isOwner());
  const sec = shown.find(x => x[0] === which) || shown[0];
  main.innerHTML = `<h1>Settings</h1><nav class="tabs" aria-label="Settings">${shown.map(([k, label]) =>
    `<a href="#/settings/${k}" class="${k === sec[0] ? "on" : ""}"${k === sec[0] ? ` aria-current="page"` : ""}>${esc(label)}</a>`).join("")}</nav><section id="setting" data-section="${sec[0]}"></section>`;
  await sec[2]($("#setting", main));
});
// Where these lived before Settings.
route(/^#\/models$/, () => { location.hash = "#/settings/models"; });
route(/^#\/apps$/, () => { location.hash = "#/settings/connectors"; });

// How Magnus looks on this device (U7, amended): light, dark or the
// computer's; solid or glass; an accent; and how much it moves. The choice
// takes effect at once; the computer's own settings for less motion and less
// transparency still apply on top.
function appearanceSection(el) {
  const radio = (look, value, label, inner = label, cls = "", style = "") => {
    const on = (look === "theme" && Theme.prefs().theme !== "custom" ? Theme.look.theme : Theme.prefs()[look]) === value;
    return `<button class="${cls}${on ? " on" : ""}" role="radio" aria-checked="${on}" data-look="${look}" data-value="${value}" title="${esc(label)}"${style ? ` style="${style}"` : ""}>${inner}</button>`;
  };
  const draw = () => {
    const p = Theme.prefs(), os = Theme.os(), look = Theme.look;
    paint(el, `<h2>Appearance</h2><div class="appearance">
      <h3 id="theme-h">Theme</h3><div class="theme-presets" role="radiogroup" aria-labelledby="theme-h">${Theme.THEMES.map(([k, label]) => radio("theme", k, label, `<span class="preset-preview" aria-hidden="true" style="--preview-chrome:#${Theme.PRESETS[k][1]};--preview-canvas:#${Theme.PRESETS[k][2]};--preview-text:#${Theme.PRESETS[k][4]}"><span></span><span></span><span></span></span>${esc(label)}`)).join("")}</div>
      <label class="follow-computer"><input type="checkbox" data-follow-computer ${p.followComputer ? "checked" : ""}>Follow computer</label>
      ${p.theme === "custom" ? `<p class="small muted">Custom appearance${Theme.tokens(look)["contrast-adjusted"] === "true" ? " — contrast adjustments are active" : ""}.</p>` : ""}
      ${Theme.saved === false ? `<p class="small" role="status">Appearance changed for this session but was not saved on this device.</p>` : ""}
      <h3 id="mode-h">Custom mode</h3><div class="seg" role="radiogroup" aria-labelledby="mode-h">${Theme.MODES.map(([k, l]) => radio("mode", k, l)).join("")}</div>
      <h3 id="mat-h">Material</h3><div class="materials" role="radiogroup" aria-labelledby="mat-h">${Theme.MATERIALS.map(([k, l, hint]) => radio("material", k, l,
        `<span class="mprev ${k}" aria-hidden="true"><span class="ms"></span><span class="mm"></span></span><span class="mt"><span class="ml">${esc(l)}</span><span class="small muted">${esc(hint)}</span></span>`, "material")).join("")}</div>
      ${p.material === "glass" && os.reduceTransparency ? `<p class="small muted" data-look-note>Your computer asks for less transparency, so Magnus stays solid.</p>` : ""}
      <h3 id="acc-h">Accent color</h3><div class="swatches" role="radiogroup" aria-labelledby="acc-h">${Theme.ACCENTS.map(a => radio("accent", a.id, a.label, `<span class="sr">${esc(a.label)}</span>`, "swatch", `--swatch:${a.swatch};background:${a.swatch}`)).join("")}</div>
      <h3 id="mot-h">Motion</h3><div class="seg" role="radiogroup" aria-labelledby="mot-h">${Theme.MOTIONS.map(([k, l]) => radio("motion", k, l)).join("")}</div>
      <p class="small muted" data-motion-hint>${esc(Theme.MOTION_HINTS[p.motion])}${look.motion !== p.motion ? " Your computer asks for reduced motion, so Magnus uses quick fades." : ""}</p>
      <p class="small faint">Magnus also follows your computer's settings for reduced motion and reduced transparency.</p></div>`);
  };
  el.onclick = e => {
    const b = e.target.closest("[data-look]");
    if (!b) return;
    const field = b.dataset.look, value = b.dataset.value;
    Theme.set({ [field]: value });
    draw();
    $(`[data-look="${field}"][data-value="${value}"]`, el)?.focus({ preventScroll: true });
  };
  const changed = () => {
    if (!el.isConnected) return;
    const active = document.activeElement;
    const selector = active?.dataset?.look ? `[data-look="${active.dataset.look}"][data-value="${active.dataset.value}"]` : active?.matches("[data-follow-computer]") ? "[data-follow-computer]" : null;
    draw();
    if (selector) $(selector, el)?.focus({ preventScroll: true });
  };
  window.addEventListener("magnus-theme", changed);
  onLeave(() => window.removeEventListener("magnus-theme", changed));
  el.onchange = e => { if (e.target.matches("[data-follow-computer]")) { Theme.set({ followComputer: e.target.checked }); draw(); $("[data-follow-computer]", el)?.focus({ preventScroll: true }); } };
  draw();
}

// The owner's kill switch, and what it revoked.
async function killSwitchSection(el) {
  const draw = async () => paint(el, `<h2>Kill switch</h2>${killSwitch(await api("GET", "/v1/kill-switch"))}`);
  el.onclick = async e => {
    const kill = e.target.closest("[data-kill]");
    if (!kill) return;
    const on = kill.dataset.kill === "on";
    const sure = on ? await ask("Stop everything? Every action is refused, every task is suspended, every sign-in in the vault is revoked at its provider, and every key in the vault is sealed. You will sign in to providers again afterwards.", "Stop everything")
      : await ask("Turn the kill switch off? Suspended tasks stay suspended until you resume each one.", "Turn it off");
    if (sure) api("POST", "/v1/kill-switch", { engaged: on }).then(() => { toast(on ? "Everything is stopped." : "The kill switch is off."); draw(); refresh(); }).catch(err => toast(err.message));
  };
  App.redraw = draw;
  onLeave(() => { App.redraw = null; });
  await draw();
}

async function accountSection(el) {
  const draw = async () => {
    const [{ sessions }, people] = await Promise.all([api("GET", "/v1/sessions"), isOwner() ? api("GET", "/v1/people") : null]);
    const FROM = { provider: "from the identity provider", settings: "set here", config: "from the config file" };
    paint(el, `<h2>Account</h2>
      <div class="card"><div class="spread"><div><strong>${esc(App.me.name)}</strong> <span class="faint">(${esc(App.me.id)})</span>
        <div class="small muted">Signed in to ${esc(App.appliance)}${App.me.roles?.length ? ` · roles: ${esc(App.me.roles.join(", "))}` : ""}${App.me.groups?.length ? ` · groups: ${esc(App.me.groups.join(", "))}` : ""}</div></div>
        <button data-act="sign-out">Sign out</button></div></div>
      <h3>Devices</h3>
      ${sessions.length ? `<table class="t" data-devices><tbody>${sessions.map(x => `<tr data-session="${esc(x.id)}"><td>${x.client === "app" ? "Magnus app" : x.client === "browser" ? "Browser" : "Device"}${x.current ? ` <span class="chip ok">this device</span>` : ""}</td>
        <td class="small muted">signed in ${esc(when(x.created))} · until ${esc(new Date(x.expires).toLocaleDateString())}</td>
        <td>${x.current ? "" : `<button class="link" data-act="end" data-id="${esc(x.id)}">Sign out</button>`}</td></tr>`).join("")}</tbody></table>`
        : `<p class="empty">Signed in with an access key; no devices to list.</p>`}
      ${people ? `<h3>People and roles</h3><table class="t" data-people><thead><tr><th>Person</th><th>Roles</th><th></th></tr></thead><tbody>
        ${people.people.map(p => `<tr data-person="${esc(p.id)}"><td>${esc(p.name)} <span class="faint">(${esc(p.id)})</span><div class="small faint">${esc(FROM[p.roles_from])}</div></td>
          <td>${p.roles_from === "provider" ? esc(p.roles.join(", ") || "none")
            : `<input data-roles="${esc(p.id)}" aria-label="Roles of ${esc(p.name)}" value="${esc(p.roles.join(", "))}" placeholder="none">`}</td>
          <td>${p.roles_from === "provider" ? "" : `<button data-act="roles" data-id="${esc(p.id)}">Save</button>`}</td></tr>`).join("")}</tbody></table>
        <p class="small muted">Roles: owner (everything), developer (the developer views), approver:&lt;risk&gt; (approves actions of that risk). Separate several with commas.</p>` : ""}`);
  };
  el.onclick = async e => {
    const b = e.target.closest("button[data-act]"); if (!b) return;
    try {
      if (b.dataset.act === "sign-out") return $("nav.side [data-act=sign-out]")?.click();
      if (b.dataset.act === "end") { b.disabled = true; await api("DELETE", `/v1/sessions/${enc(b.dataset.id)}`); toast("That device is signed out."); }
      if (b.dataset.act === "roles") {
        const roles = $(`[data-roles="${CSS.escape(b.dataset.id)}"]`, el).value.split(",").map(x => x.trim()).filter(Boolean);
        await api("PUT", `/v1/people/${enc(b.dataset.id)}/roles`, { roles });
        toast("Roles saved.");
        if (b.dataset.id === App.me.id) { App.me = await api("GET", "/v1/me"); sidebar(); }
      }
      el._html = null; await draw();
    } catch (err) { toast(err.message); b.disabled = false; }
  };
  await draw();
}

// Notifications follow the person to every device: the choices live on the appliance.
const NOTIFY = [["notify_blocking", "A task needs me now", "An approval, a question, or a review a task is waiting on."],
  ["notify_batched", "Something can wait for me", "Results to look at and proposals: decisions no task is waiting on."],
  ["notify_agent_changes", "The agent changes", "A new model, instructions, harness or policy."]];
async function notificationsSection(el) {
  const draw = async () => {
    App.prefs = await api("GET", "/v1/preferences");
    paint(el, `<h2>Notifications</h2><p class="muted">Magnus tells this computer when…</p>
      <div class="card">${NOTIFY.map(([k, label, hint]) => `<label class="row toggle"><input type="checkbox" data-pref="${k}" ${App.prefs[k] ? "checked" : ""}>
        <span><span>${esc(label)}</span><span class="small muted" style="display:block">${esc(hint)}</span></span></label>`).join("")}</div>`);
  };
  el.onchange = async e => {
    const c = e.target.closest("input[data-pref]"); if (!c) return;
    try { App.prefs = await api("PUT", "/v1/preferences", { [c.dataset.pref]: c.checked }); toast("Saved."); } catch (err) { toast(err.message); c.checked = !c.checked; }
  };
  await draw();
}

async function updatesSection(el) {
  const draw = async () => {
    const [st, notes, ver, appl] = await Promise.all([Magnus.updateState().catch(() => null), Magnus.whatsNew(true).catch(() => null),
      api("GET", "/v1/version"), isOwner() ? api("GET", "/v1/updates").catch(() => null) : null]);
    const para = t => esc(t || "Fixes and improvements.").split(/\n+/).map(l => `<p>${l}</p>`).join("");
    paint(el, `<h2>Updates</h2>
      ${st ? `<div class="card" data-app-version><div class="spread"><div><strong>This app</strong> <span class="mono">${esc(st.current)}</span></div>
          ${st.ready ? `<button class="primary" data-act="restart-update">Restart to update</button>` : `<button data-act="look">Check now</button>`}</div>
        <div class="small muted">${st.ready ? `Magnus ${esc(st.ready.version)} is downloaded and checked.` : st.error ? `The last look failed: ${esc(st.error)}` : st.can_update ? esc(st.can_update) : "Up to date."}</div>
        ${st.ready ? `<h3>What's new in ${esc(st.ready.version)}</h3><div class="notes">${para(st.ready.notes)}</div>` : ""}
        ${notes ? `<h3>What's new in ${esc(notes.version)}</h3><div class="notes">${para(notes.notes)}</div>` : ""}</div>` : ""}
      <div class="card" data-appliance-version><div class="spread"><div><strong>The appliance</strong> <span class="mono">${esc(ver.version)}</span></div>
          ${appl?.latest && appl.latest.version !== ver.version ? `<button class="primary" data-act="install">Install ${esc(appl.latest.version)} now</button>` : ""}</div>
        ${appl ? `<div class="small muted">${appl.automatic ? "Updates install on their own." : "Updates install when you choose."}${appl.previous ? ` Rolls back to ${esc(appl.previous)} if one fails.` : ""}</div>
          ${appl.latest ? `<h3>What's new in ${esc(appl.latest.version)}</h3><div class="notes">${para(appl.latest.notes)}</div>` : ""}` : ""}</div>`);
  };
  // A look that ends while this is showing (a download finishing) shows here
  // too, not only in the sidebar.
  App.redrawUpdates = () => el.isConnected ? draw() : (App.redrawUpdates = null);
  el.onclick = async e => {
    const b = e.target.closest("button[data-act]"); if (!b) return;
    try {
      if (b.dataset.act === "restart-update") return restartToUpdate();
      if (b.dataset.act === "look") { b.disabled = true; await Magnus.updateLook(); await refreshUpdate(); }
      if (b.dataset.act === "install") { b.disabled = true; await api("POST", "/v1/updates/check", {}); toast("Installing; the appliance restarts in a moment."); }
      el._html = null; await draw();
    } catch (err) { toast(err.message); b.disabled = false; }
  };
  await draw();
}

// One grant in Settings: what it covers, what still asks, its spend this
// period, the actions taken under it and its policy, with Edit and Revoke.
function grantRow(g, apps, editing) {
  const acts = g.actions || [];
  return `<div class="grant-row" data-grant="${esc(g.id)}" style="margin-top:10px">
    <div class="spread"><strong>${esc(g.title)}</strong>${g.revoked ? `<span class="chip">Revoked ${esc(when(g.revoked))}</span>` : `<span class="row"><button data-act="grant-edit">Edit</button><button data-act="grant-revoke">Revoke</button></span>`}</div>
    <p class="small muted">In ${esc(g.apps.join(", "))} · still asks before ${esc(g.still_asks.join(", "))} · <span class="num">$${esc(Number(g.spent).toFixed(2))} of $${esc(g.max_usd)} this ${esc(g.period)}</span></p>
    ${editing === g.id ? `${grantForm(g.id, g, apps)}<div class="row"><button class="primary" data-act="grant-save">Save</button><button class="link" data-act="grant-cancel">Cancel</button></div>` : ""}
    ${acts.length ? `<details data-key="grant-actions:${esc(g.id)}"><summary class="small">${acts.length} action${acts.length === 1 ? "" : "s"} taken under it</summary><ul class="small">${acts.slice(0, 20).map(a => `<li><span class="faint">${esc(when(a.at))}</span> ${esc(a.tool)} in <a href="#/task/${enc(a.task)}" data-open="task:${esc(a.task)}">${esc(a.task_title || "a task")}</a></li>`).join("")}</ul></details>` : `<p class="small faint">Nothing done under it yet.</p>`}
    <details data-key="grant-policy:${esc(g.id)}"><summary class="small">Its policy</summary><pre class="mono small">${esc(g.cedar)}</pre></details>
  </div>`;
}

// One commitment in Settings (#236): who said they would do what, for
// whom, by when, overdue shown, where it was said; Done, Release, Edit and
// Remove.
function commitmentRow(c, editing) {
  const STATE = { done: ["ok", "Done"], released: ["", "Released"] };
  const who = c.who === "magnus" ? "Magnus" : "You";
  const from = c.source?.kind === "app" ? `read in ${esc(c.source.app)}` : "said in a conversation";
  const status = c.status === "open" ? (c.overdue ? `<span class="chip bad">Overdue</span>` : "") : `<span class="chip ${STATE[c.status]?.[0] || ""}">${esc(STATE[c.status]?.[1] || c.status)}</span>`;
  const acts = c.status === "open" ? `<button data-act="cmt-done">Done</button><button data-act="cmt-release">Release</button>` : `<button data-act="cmt-reopen">Reopen</button>`;
  return `<div class="cmt-row" data-commitment="${esc(c.id)}" style="margin-top:10px">
    <div class="spread"><span><strong>${who}:</strong> ${esc(c.what)} ${status}</span><span class="row">${acts}<button data-act="cmt-edit">Edit</button><button data-act="cmt-remove">Remove</button></span></div>
    <p class="small muted">${c.to ? `For ${esc(c.to)} · ` : ""}${c.by ? `By ${esc(c.by)} · ` : ""}${from}, ${esc(when(c.at))}${c.quote ? ` · <span class="faint">“${esc(c.quote)}”</span>` : ""}</p>
    ${editing === c.id ? `<div class="row" style="margin-top:6px"><input data-f="what" aria-label="What" value="${esc(c.what)}" style="flex:2"><input data-f="to" aria-label="For whom" placeholder="For whom" value="${esc(c.to || "")}" style="flex:1"><input data-f="by" aria-label="By when" placeholder="YYYY-MM-DD" value="${esc(c.by || "")}" style="flex:1"></div>
      <div class="row"><button class="primary" data-act="cmt-save">Save</button><button class="link" data-act="cmt-cancel">Cancel</button></div>` : ""}
  </div>`;
}

async function dataSection(el) {
  // The person's relationship notes as last drawn: each row names its note by them.
  let notes = [];
  // The grant being edited, if any.
  let editing = null;
  // The commitment being edited, if any.
  let editingC = null;
  const draw = async () => {
    const [prefs, { requests }, backups, rel, { disagreements }, granted, { commitments }] = await Promise.all([api("GET", "/v1/preferences"), api("GET", "/v1/deletion-requests"), isOwner() ? api("GET", "/v1/backups").catch(() => null) : null, api("GET", "/v1/relationship"), api("GET", "/v1/disagreements"), api("GET", "/v1/grants"), api("GET", "/v1/commitments")]);
    App.prefs = prefs;
    notes = rel.notes;
    const STATE = { done: ["ok", "done"], decline: ["bad", "declined"] };
    paint(el, `<h2>Data and privacy</h2>
      <div class="card"><label class="row toggle"><input type="checkbox" data-pref="feedback_consent" ${prefs.feedback_consent ? "checked" : ""}>
        <span><span>Send problem reports</span><span class="small muted" style="display:block">When you report a problem, what you wrote and the task's record go to the people who build Magnus. Off: nothing leaves the appliance.</span></span></label></div>
      <h3>How Magnus works with you</h3>
      <div class="card" data-relationship><p class="small muted">What you told Magnus about working with you: tone, what you delegated, standing decisions. Every task and chat of yours carries it; no one else sees it.</p>
        ${notes.map((n, i) => `<div class="row" data-note="${i}" style="margin-top:6px"><input aria-label="Note" value="${esc(n)}" style="flex:1"><button data-act="note-save">Save</button><button data-act="note-remove">Remove</button></div>`).join("") || `<p class="small faint">Nothing yet.</p>`}
        <div class="row" style="margin-top:8px"><input id="newnote" aria-label="New note" style="flex:1" placeholder="For example: fewer jokes"><button data-act="note-add">Add</button></div></div>
      <h3>What Magnus may do on its own</h3>
      <div class="card" data-grants><p class="small muted">Kinds of your work you let Magnus handle without asking, within your organisation's policy, each with what was done under it. They are yours alone; spending money always asks.</p>
        ${granted.grants.map(g => grantRow(g, granted.apps, editing)).join("") || `<p class="small faint">None yet. Choose "Handle things like this" on a task Magnus proposes, or ask Magnus.</p>`}</div>
      <h3>What you and Magnus said you'd do</h3>
      <div class="card" data-commitments><p class="small muted">What you or Magnus said you would do, recognised from your conversations and what Magnus read for you, with for whom and by when. Open ones are in every task and chat of yours; no one else sees them.</p>
        ${commitments.map(c => commitmentRow(c, editingC)).join("") || `<p class="small faint">None yet.</p>`}</div>
      <h3>Where you decided otherwise</h3>
      <div class="card" data-disagreements><p class="small muted">When Magnus advised one thing and you decided another, it notes both, and later how it turned out. Only your own tasks and chats see these.</p>
        ${disagreements.map(d => `<p class="small"><span class="faint">${esc(when(d.at))}</span> Magnus advised: ${esc(d.position)} You decided: ${esc(d.decision)} ${d.outcome ? `Outcome: ${esc(d.outcome)}` : `<span class="faint">Outcome not known yet.</span>`}</p>`).join("") || `<p class="small faint">None.</p>`}</div>
      <h3>Delete my data</h3>
      <div class="card"><p class="small muted">The owner is asked, and has 30 days to act. What is deleted leaves every backup within that time.</p>
        <label for="delwhat">What to delete</label><input id="delwhat" data-draft="delwhat" style="width:100%" placeholder="all of my data">
        <div class="row" style="margin-top:8px"><button data-act="delete-request">Ask for deletion</button></div>
        ${requests.length ? `<table class="t small" data-deletions style="margin-top:10px"><tbody>${requests.map(q => `<tr><td>${esc(q.what)}${isOwner() && q.person !== App.me.id ? ` <span class="faint">(${esc(q.person)})</span>` : ""}</td>
          <td>${q.decision ? `<span class="chip ${STATE[q.decision]?.[0] || ""}">${esc(STATE[q.decision]?.[1] || q.decision)}</span>${q.note ? ` <span class="muted">${esc(q.note)}</span>` : ""}` : `<span class="chip warn">asked</span> due ${esc(new Date(q.due).toLocaleDateString())}`}</td></tr>`).join("")}</tbody></table>` : ""}</div>
      ${backups ? `<h3>Backups</h3><div class="card"><div class="spread"><span class="small muted">${backups.files.length ? `${backups.files.length} kept for ${esc(backups.keep_days)} days` : "None yet."}${backups.latest ? ` · last ${esc(when(backups.latest.at))}${backups.latest.restored ? ", restored to check it" : `, <span style="color:var(--bad)">did not restore: ${esc(backups.latest.detail)}</span>`}` : ""}</span>
        <button data-act="backup">Back up now</button></div>
        ${backups.scrub_pending ? `<p class="small" data-backup-scrub style="color:var(--bad)">${backups.scrub_pending.erasures ? `${esc(backups.scrub_pending.erasures)} deleted ${backups.scrub_pending.erasures === 1 ? "person's" : "people's"} data is not fully erased: ` : ""}${esc(backups.scrub_pending.files.length)} ${backups.scrub_pending.files.length === 1 ? "backup still holds" : "backups still hold"} deleted data, since ${esc(when(backups.scrub_pending.since))}. Magnus tries again every hour and at start: ${esc(backups.scrub_pending.detail)}</p>` : ""}
        ${backups.on_appliance_disk ? `<p class="small" data-backup-disk style="color:var(--warn)">They are on the appliance's own disk, so a lost disk loses them too: set <span class="mono">[backup] dir</span> to another disk, and keep a copy of the backup key off the appliance.</p>` : ""}</div>` : ""}`);
  };
  el.onchange = async e => {
    const c = e.target.closest("input[data-pref]"); if (!c) return;
    try { App.prefs = await api("PUT", "/v1/preferences", { [c.dataset.pref]: c.checked }); toast("Saved."); } catch (err) { toast(err.message); c.checked = !c.checked; }
  };
  el.onclick = async e => {
    const b = e.target.closest("button[data-act]"); if (!b) return;
    try {
      if (b.dataset.act === "delete-request") {
        const what = $("#delwhat", el).value.trim();
        if (!await ask(`Ask the owner to delete ${what || "all of your data"}?`, "Ask the owner")) return;
        b.disabled = true; await api("POST", "/v1/deletion-requests", { what }); delete drafts.delwhat; toast("Asked. The owner has 30 days to act.");
      }
      if (b.dataset.act === "backup") { b.disabled = true; await api("POST", "/v1/backups", {}); toast("Backed up."); }
      const row = b.closest("[data-note]"), note = row && notes[+row.dataset.note];
      if (b.dataset.act === "note-add") { b.disabled = true; await api("POST", "/v1/relationship", { action: "add", text: $("#newnote", el).value }); toast("Saved."); }
      if (b.dataset.act === "note-save") { b.disabled = true; await api("POST", "/v1/relationship", { action: "change", note, text: $("input", row).value }); toast("Saved."); }
      if (b.dataset.act === "note-remove") { b.disabled = true; await api("POST", "/v1/relationship", { action: "remove", note }); toast("Removed."); }
      const g = b.closest("[data-grant]")?.dataset.grant;
      if (b.dataset.act === "grant-edit") editing = g;
      if (b.dataset.act === "grant-cancel") editing = null;
      if (b.dataset.act === "grant-save") {
        const f = readGrantForm(b.closest("[data-grant]"));
        if (!f.apps.length) return toast("Choose at least one app.");
        b.disabled = true; await api("PUT", `/v1/grants/${enc(g)}`, f); editing = null; toast("Saved.");
        for (const k of Object.keys(drafts)) if (k.startsWith(`g-${g}-`)) delete drafts[k];
      }
      if (b.dataset.act === "grant-revoke") {
        if (!await ask("Revoke this? From now on, what it covered asks you again.", "Revoke")) return;
        b.disabled = true; await api("DELETE", `/v1/grants/${enc(g)}`); toast("Revoked.");
      }
      const cr = b.closest("[data-commitment]"), c = cr?.dataset.commitment;
      if (b.dataset.act === "cmt-edit") editingC = c;
      if (b.dataset.act === "cmt-cancel") editingC = null;
      const status = { "cmt-done": "done", "cmt-release": "released", "cmt-reopen": "open" }[b.dataset.act];
      if (status) { b.disabled = true; await api("PUT", `/v1/commitments/${enc(c)}`, { status }); toast(status === "done" ? "Marked done." : status === "released" ? "Released." : "Reopened."); }
      if (b.dataset.act === "cmt-save") {
        const f = k => $(`[data-f="${k}"]`, cr).value.trim();
        if (!f("what")) return toast("Say what will be done.");
        b.disabled = true; await api("PUT", `/v1/commitments/${enc(c)}`, { what: f("what"), to: f("to"), by: f("by") }); editingC = null; toast("Saved.");
      }
      if (b.dataset.act === "cmt-remove") {
        if (!await ask("Remove this? It leaves your list and Magnus's context, and its words are deleted.", "Remove")) return;
        b.disabled = true; await api("DELETE", `/v1/commitments/${enc(c)}`); toast("Removed.");
      }
      el._html = null; await draw();
    } catch (err) { toast(err.message); b.disabled = false; }
  };
  await draw();
}

// ---------- start ----------

let background = [];
// ---------- the app's own updates ----------

// `look`: ask the appliance now, as when a session begins (the app's hourly
// look may have run before it was connected).
async function refreshUpdate(look = false) {
  try { App.update = await (look ? Magnus.updateLook() : Magnus.updateState()); } catch { App.update = await Magnus.updateState().catch(() => null); }
  if ($("#side")) sidebar();
  await App.redrawUpdates?.();
}

async function restartToUpdate() {
  toast("Installing the update; Magnus restarts in a moment.", 10000);
  try { await Magnus.updateInstall(); } catch (e) { toast(`The update did not install: ${e.message}`); refreshUpdate(); }
}

// What changed, in the release's own plain words, once after an update.
async function showWhatsNew() {
  let n = null;
  try { n = await Magnus.whatsNew(); } catch { return; }
  if (!n) return;
  const box = document.createElement("div");
  box.className = "whats-new card"; box.setAttribute("role", "dialog"); box.setAttribute("aria-label", "What's new");
  box.innerHTML = `<h3>What's new in Magnus ${esc(n.version)}</h3>
    <div class="notes">${esc(n.notes || "Fixes and improvements.").split(/\n+/).map(l => `<p>${l}</p>`).join("")}</div>
    <div class="row"><button class="primary" data-act="close">Got it</button></div>`;
  box.querySelector("[data-act=close]").onclick = () => box.remove();
  document.body.append(box);
}

// The appliance no longer accepts this version of the app: only updating helps.
let updateScreen = null;
async function updateRequiredScreen(info) {
  if (updateScreen) return;
  stop();
  document.body.innerHTML = `<div class="signin"><h1>Magnus</h1><h3>Update required</h3>
    <p class="muted">${esc(info?.error || "The appliance needs a newer version of this app.")}</p><div class="card" id="ur"></div></div>`;
  const draw = async () => {
    let st = null;
    try { st = await Magnus.updateState(); } catch { /* shown below */ }
    const box = $("#ur");
    if (!box) return;
    if (!st) paint(box, `<p>Open the appliance's address in a browser to use it until the app is updated.</p>`);
    else if (st.ready) paint(box, `<p>Magnus ${esc(st.ready.version)} is downloaded and checked.</p><div class="row"><button class="primary" data-act="restart-update">Restart to update</button></div>`);
    else if (st.can_update) paint(box, `<p>${esc(st.can_update)}</p>`);
    else paint(box, `<p>${st.error ? `The update could not be fetched: ${esc(st.error)}` : "Downloading the update…"}</p><div class="row"><button data-act="look">Try again</button></div>`);
  };
  document.body.addEventListener("click", async e => {
    if (e.target.closest("[data-act=restart-update]")) restartToUpdate();
    if (e.target.closest("[data-act=look]")) { try { await Magnus.updateLook(); } catch { /* drawn */ } draw(); }
  });
  Magnus.onUpdateReady(draw);
  updateScreen = setInterval(draw, 5000);
  draw();
}
window.addEventListener("magnus-update-required", e => updateRequiredScreen(e.detail));

async function boot() {
  Resources.reset();
  Theme.scope(App.appliance, App.me.id);
  shell();
  window.onhashchange = go;
  Magnus.onUpdateReady(() => refreshUpdate());
  refreshUpdate(true);
  showWhatsNew();
  App.prefs = await api("GET", "/v1/preferences").catch(() => null);
  await Promise.all([refresh(), pollVersions()]);
  // Opening the app shows what changed since the last session: with
  // news, and landing on home, the needs panel opens beside it (#367).
  App.reported = new Set();
  App.reporting = new Set();
  App.feed = await api("POST", "/v1/home/opened", {}).catch(() => null);
  if (App.feed && (App.feed.ended.length || App.feed.arrived.length || App.feed.decided.length) && (!location.hash || location.hash === "#/home")) App.openAfter = "needs";
  background = [setInterval(refresh, 3000), setInterval(pollVersions, 30000)];
  go();
}

function stop() {
  Resources.reset();
  background.forEach(clearInterval); background = [];
  App.timers.forEach(clearInterval); App.timers = [];
  App.leave.forEach(f => { try { f(); } catch { /* ignore */ } }); App.leave = [];
  window.onhashchange = null;
  Panel.what = null;
  Panel.rendered=null;Resources.forget("Work");
  Theme.scope(null, null);
  App.me = null; App.inbox = []; App.tasks = []; App.schedules = []; App.threads = []; App.projects = []; App.thread = null; App.feed = null; App.reported = new Set(); App.reporting = new Set();
  // What one person was typing or filtering by is not the next one's here.
  Panel.lib = { q: "", project: "" };
  for (const k of Object.keys(drafts)) delete drafts[k];
}

async function start() {
  try {
    const r = await Magnus.restore();
    if (r && r.update_required) return updateRequiredScreen(r.update_required);
    if (r && r.person) { App.me = r.person; App.appliance = r.appliance; return boot(); }
    signInScreen(r && r.note);
  } catch (e) { signInScreen(e.message || String(e)); }
}

document.addEventListener("DOMContentLoaded", start);
