// Drives the installed desktop app through tauri-driver (W3C WebDriver), for
// the installed-app scripts: open a session on the app, act on its page,
// and the steps every script begins with (pin the appliance's certificate,
// sign in through the identity provider).
import { writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

export const env = k => { const v = process.env[k]; if (!v) { console.error(`${k} is not set`); process.exit(2); } return v; };
const E = "element-6066-11e4-a52e-4f735466cecf";
const T = 60_000;

// A session on the app at `app`, through the driver at `wd`. Screenshots go
// to `shots` (optional), named with `prefix`.
export async function open(wd, app, shots, prefix) {
  if (shots) mkdirSync(shots, { recursive: true });
  async function call(method, path, body) {
    const r = await fetch(`${wd}${path}`, { method, headers: { "content-type": "application/json" }, body: body === undefined ? undefined : JSON.stringify(body) });
    const j = await r.json();
    if (!r.ok || (j.value && j.value.error)) throw new Error(`${method} ${path}: ${JSON.stringify(j.value)}`);
    return j.value;
  }
  const session = (await call("POST", "/session", { capabilities: { alwaysMatch: { "tauri:options": { application: app } } } })).sessionId;
  const s = p => `/session/${session}${p}`;
  const d = { call, s };
  d.run = (script, ...args) => call("POST", s("/execute/sync"), { script, args });
  d.until = async (what, f) => {
    const end = Date.now() + T;
    let last;
    while (Date.now() < end) {
      try { const v = await f(); if (v) return v; } catch (e) { last = e; }
      await new Promise(r => setTimeout(r, 250));
    }
    throw new Error(`timed out waiting for ${what}${last ? `: ${last.message}` : ""}`);
  };
  const find = (using, value) => d.until(value, async () => {
    const els = await call("POST", s("/elements"), { using, value });
    return els.length ? els[0][E] : null;
  });
  d.css = v => find("css selector", v);
  d.xpath = v => find("xpath", v);
  d.button = (label, within = "") => d.xpath(`${within}//button[normalize-space()='${label}']`);
  d.click = async el => call("POST", s(`/element/${await el}/click`), {});
  d.type = async (el, text) => call("POST", s(`/element/${await el}/value`), { text });
  d.text = async el => call("GET", s(`/element/${await el}/text`));
  d.attr = async (el, name) => call("GET", s(`/element/${await el}/attribute/${name}`));
  d.seen = t => d.until(`"${t}"`, () => d.run("return document.body.innerText.includes(arguments[0])", t));
  let n = 0;
  const shot = async name => { if (shots) writeFileSync(join(shots, `${prefix}-${String(++n).padStart(2, "0")}-${name}.png`), Buffer.from(await call("GET", s("/screenshot")), "base64")); };
  d.close = () => call("DELETE", s("")).catch(() => {});
  d.step = async (name, f) => {
    try { await f(); await shot(name); console.log(`ok   ${name}`); }
    catch (e) { await shot(`FAILED-${name}`).catch(() => {}); console.error(`FAIL ${name}: ${e.message}`); await d.close(); process.exit(1); }
  };
  return d;
}

// Do `act` (a click that downloads), then wait for the app's next toast and
// return the path of the file it saved, or fail with what it said instead.
// Toasts already showing are from earlier steps and are passed over.
export async function savedPath(d, act) {
  await d.run("document.querySelectorAll('.toast').forEach(t => t.dataset.seen = '1')");
  await act();
  const said = await d.until("the app to say what happened", () => d.run("const t = [...document.querySelectorAll('.toast:not([data-seen])')]; return t.length ? JSON.stringify(t.map(e => e.innerText)) : null"));
  const saved = JSON.parse(said).find(t => t.startsWith("Saved "));
  if (!saved) throw new Error(`the app did not save the file; it said ${said}`);
  return saved.slice("Saved ".length).trim();
}

// Connect to the appliance over HTTPS, pinning the certificate it printed,
// and sign in through the identity provider; the test is the person's
// browser and approves the code the app shows.
export async function pinAndSignIn(d, { url, fingerprint, provider, person }) {
  await d.step("pin the appliance's certificate", async () => {
    await d.type(d.css("#addr"), url);
    await d.click(d.button("Continue"));
    const shown = (await d.text(d.css("#fp"))).trim();
    if (shown !== fingerprint) throw new Error(`the app shows ${shown}, the appliance printed ${fingerprint}`);
    await d.click(d.button("They match"));
  });
  await signIn(d, { provider, person });
}

// Sign in through the identity provider on an app already pinned to its
// appliance; the test is the person's browser and approves the code the app
// shows, as `person`.
export async function signIn(d, { provider, person, as }) {
  await d.step(`sign in with ${provider} as ${as || person}`, async () => {
    await d.click(d.button(`Sign in with ${provider}`));
    const code = (await d.text(d.css("#code"))).trim();
    const link = await d.attr(d.css("#open"), "data-url");
    if (!link.includes(code)) throw new Error(`the provider's page ${link} is not for code ${code}`);
    const approved = await (await fetch(as ? `${link}&as=${encodeURIComponent(as)}` : link)).text();
    if (!approved.includes(`Signed in as ${as || person}`)) throw new Error(`the provider did not approve: ${approved}`);
    await d.xpath("//h1[normalize-space()='Home']");
    const page = await d.run("return JSON.stringify(localStorage) + JSON.stringify(sessionStorage) + document.cookie");
    if (page.includes("mgs_")) throw new Error("the session token reached the page");
  });
}
