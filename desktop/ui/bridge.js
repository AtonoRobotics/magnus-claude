// One frontend, two hosts. In the desktop app the Rust side holds the
// session token, pins the appliance's certificate and makes every request;
// this page never sees the token. In a browser served by the appliance the
// session is an HttpOnly cookie, and changes carry a header another site's
// page cannot set.
"use strict";

class ApiError extends Error {
  constructor(status, message, body) { super(message); this.status = status; this.body = body; }
}

function errorOf(status, body) {
  const msg = (body && (body.error || body.message)) || (typeof body === "string" && body) || `request failed (${status})`;
  return new ApiError(status, msg, body);
}

const tauri = window.__TAURI__;

const appBridge = tauri && {
  kind: "app",
  invoke: (cmd, args) => tauri.core.invoke(cmd, args || {}),
  async restore() { return this.invoke("restore"); },
  async connect(url) { return this.invoke("connect", { url }); },
  async trust(url, fingerprint) { return this.invoke("trust", { url, fingerprint }); },
  async signInMethod() { return this.invoke("sign_in_method"); },
  async signIn(person, password) { return this.invoke("sign_in", { person, password }); },
  async deviceStart() { return this.invoke("device_start"); },
  async devicePoll(flow) { return this.invoke("device_poll", { flow }); },
  async signOut() { return this.invoke("sign_out"); },
  async api(method, path, body) {
    const r = await this.invoke("api", { method, path, body: body === undefined ? null : body });
    if (r.status >= 400) throw errorOf(r.status, r.body);
    return r.body;
  },
  live(task, onText) {
    let stop = false, unlisten = null;
    tauri.event.listen("live", e => { if (!stop && e.payload.task === task) onText(e.payload.text); }).then(u => { unlisten = u; if (stop) u(); });
    this.invoke("live_start", { task }).catch(() => {});
    return () => { stop = true; if (unlisten) unlisten(); this.invoke("live_stop", { task }).catch(() => {}); };
  },
  async notify(title, body) { return this.invoke("notify", { title, body }).catch(() => {}); },
  async speak(text) { return this.invoke("speak", { text }); },
  async stopSpeaking() { return this.invoke("stop_speaking").catch(() => {}); },
  async download(path, name) { return this.invoke("download", { path, name }); },
  async openExternal(url) { return this.invoke("open_url", { url }); },
  // A browser sign-in to an app returns to a listener on this machine.
  async loopbackListen() { return this.invoke("loopback_listen"); },
  async loopbackWait(id) { return this.invoke("loopback_wait", { id }); },
  async dataUrl(path, mediaType) { const b64 = await this.invoke("fetch_bytes", { path }); return `data:${mediaType};base64,${b64}`; },
};

const webBridge = {
  kind: "web",
  async restore() {
    const r = await fetch("/v1/me", { credentials: "same-origin" });
    if (!r.ok) return null;
    return { person: await r.json(), appliance: location.origin };
  },
  async connect() { return { status: "trusted", url: location.origin }; },
  async trust() {},
  async signInMethod() { return this.api("GET", "/v1/sign-in"); },
  async signIn(person, password) {
    const body = await this.api("POST", "/v1/sign-in", { person, password });
    return body.person;
  },
  async deviceStart() { return this.api("POST", "/v1/sign-in/device", { client: "browser" }); },
  async devicePoll(flow) {
    const r = await fetch(`/v1/sign-in/device/${encodeURIComponent(flow)}`, { method: "POST", credentials: "same-origin", headers: { "x-magnus-client": "web" } });
    const body = await r.json().catch(() => null);
    if (r.status === 202) return { pending: true };
    if (!r.ok) throw errorOf(r.status, body);
    return { person: body.person };
  },
  async signOut() { await this.api("POST", "/v1/sign-out", {}).catch(() => {}); },
  async api(method, path, body) {
    const opts = { method, credentials: "same-origin", headers: { "x-magnus-client": "web" } };
    if (body !== undefined && body !== null) { opts.headers["content-type"] = "application/json"; opts.body = JSON.stringify(body); }
    const r = await fetch(path, opts);
    const text = await r.text();
    let parsed = text;
    try { parsed = text ? JSON.parse(text) : null; } catch { /* plain text */ }
    if (!r.ok) throw errorOf(r.status, parsed);
    return parsed;
  },
  live(task, onText) {
    const es = new EventSource(`/v1/tasks/${encodeURIComponent(task)}/live`);
    es.onmessage = e => { try { onText(JSON.parse(e.data).text); } catch { /* ignore */ } };
    return () => es.close();
  },
  async notify(title, body) {
    if (!("Notification" in window)) return;
    if (Notification.permission === "default") await Notification.requestPermission();
    if (Notification.permission === "granted") new Notification(title, { body });
  },
  async speak(text) {
    if (!("speechSynthesis" in window)) throw new Error("This browser cannot read aloud.");
    speechSynthesis.cancel();
    speechSynthesis.speak(new SpeechSynthesisUtterance(text));
  },
  async stopSpeaking() { if ("speechSynthesis" in window) speechSynthesis.cancel(); },
  async download(path, name) {
    const a = document.createElement("a");
    a.href = path; a.download = name; document.body.append(a); a.click(); a.remove();
    return name;
  },
  async openExternal(url) { window.open(url, "_blank", "noopener"); },
  // A page cannot listen: the person pastes the address their browser opened.
  loopbackListen: null,
  async dataUrl(path) {
    const r = await fetch(path, { credentials: "same-origin" });
    if (!r.ok) throw errorOf(r.status, await r.text());
    const blob = await r.blob();
    return new Promise((ok, no) => { const f = new FileReader(); f.onload = () => ok(f.result); f.onerror = no; f.readAsDataURL(blob); });
  },
};

window.Magnus = appBridge || webBridge;
window.ApiError = ApiError;
