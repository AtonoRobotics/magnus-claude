// The real desktop app, driven through tauri-driver (W3C WebDriver): connect
// to the appliance over HTTPS and pin its certificate, sign in, connect
// Claude, type a task, approve in the inbox, and see the outcome with its
// evidence. Behavior, not appearance (U7).
//
// MAGNUS_WEBDRIVER  tauri-driver's address      MAGNUS_APP  the app binary
// MAGNUS_URL        the appliance (https)       MAGNUS_FINGERPRINT  what it printed
// MAGNUS_PERSON  MAGNUS_PASSWORD  MAGNUS_API_KEY  MAGNUS_TASK  MAGNUS_DOWNLOADS
// MAGNUS_SHOTS      optional: where to keep screenshots
import { writeFileSync, mkdirSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const env = k => { const v = process.env[k]; if (!v) { console.error(`${k} is not set`); process.exit(2); } return v; };
const [wd, app, url, fingerprint, person, password, apiKey, task, downloads] =
  ["MAGNUS_WEBDRIVER", "MAGNUS_APP", "MAGNUS_URL", "MAGNUS_FINGERPRINT", "MAGNUS_PERSON", "MAGNUS_PASSWORD", "MAGNUS_API_KEY", "MAGNUS_TASK", "MAGNUS_DOWNLOADS"].map(env);
const shots = process.env.MAGNUS_SHOTS;
if (shots) mkdirSync(shots, { recursive: true });
const E = "element-6066-11e4-a52e-4f735466cecf";
const T = 60_000;

async function call(method, path, body) {
  const r = await fetch(`${wd}${path}`, { method, headers: { "content-type": "application/json" }, body: body === undefined ? undefined : JSON.stringify(body) });
  const j = await r.json();
  if (!r.ok || (j.value && j.value.error)) throw new Error(`${method} ${path}: ${JSON.stringify(j.value)}`);
  return j.value;
}

const session = (await call("POST", "/session", { capabilities: { alwaysMatch: { "tauri:options": { application: app } } } })).sessionId;
const s = p => `/session/${session}${p}`;
const run = (script, ...args) => call("POST", s("/execute/sync"), { script, args });
const until = async (what, f) => {
  const end = Date.now() + T;
  let last;
  while (Date.now() < end) {
    try { const v = await f(); if (v) return v; } catch (e) { last = e; }
    await new Promise(r => setTimeout(r, 250));
  }
  throw new Error(`timed out waiting for ${what}${last ? `: ${last.message}` : ""}`);
};
const find = (using, value) => until(value, async () => {
  const els = await call("POST", s("/elements"), { using, value });
  return els.length ? els[0][E] : null;
});
const css = v => find("css selector", v);
const xpath = v => find("xpath", v);
const button = (label, within = "") => xpath(`${within}//button[normalize-space()='${label}']`);
const click = async el => call("POST", s(`/element/${await el}/click`), {});
const type = async (el, text) => call("POST", s(`/element/${await el}/value`), { text });
const text = async el => call("GET", s(`/element/${await el}/text`));
const seen = t => until(`"${t}"`, () => run("return document.body.innerText.includes(arguments[0])", t));
let n = 0;
const shot = async name => { if (shots) writeFileSync(join(shots, `${String(++n).padStart(2, "0")}-${name}.png`), Buffer.from(await call("GET", s("/screenshot")), "base64")); };
const step = async (name, f) => {
  try { await f(); await shot(name); console.log(`ok   ${name}`); }
  catch (e) { await shot(`FAILED-${name}`).catch(() => {}); console.error(`FAIL ${name}: ${e.message}`); await call("DELETE", s("")).catch(() => {}); process.exit(1); }
};

await step("pin the appliance's certificate", async () => {
  await type(css("#addr"), url);
  await click(button("Continue"));
  const shown = (await text(css("#fp"))).trim();
  if (shown !== fingerprint) throw new Error(`the app shows ${shown}, the appliance printed ${fingerprint}`);
  await click(button("They match"));
});

await step("sign in", async () => {
  await type(css("#who"), person);
  await type(css("#pw"), password);
  await click(button("Sign in"));
  await xpath("//h1[normalize-space()='Home']");
  const page = await run("return JSON.stringify(localStorage) + JSON.stringify(sessionStorage) + document.cookie");
  if (page.includes("mgs_")) throw new Error("the session token reached the page");
});

await step("connect Claude", async () => {
  await click(xpath("//a[normalize-space()='Models']"));
  const card = "//div[@data-adapter='anthropic']";
  await click(button("Connect", card));
  await type(css("#key-anthropic"), apiKey);
  await click(button("Save to the vault", card));
  await seen("Connected.");
  await css("#mname");
  await click(button("Use this model"));
  await seen("New tasks will use this model.");
});

let taskHash;
await step("type a task", async () => {
  await click(xpath("//a[normalize-space()='New task']"));
  await type(css("#text"), task);
  await click(button("Start"));
  taskHash = await until("the task page", async () => { const h = await run("return location.hash"); return h.startsWith("#/task/") ? h : null; });
});

await step("approve in the inbox", async () => {
  await click(xpath("//nav//a[starts-with(normalize-space(), 'Inbox')]"));
  const card = "//article[contains(@class,'approval')]";
  await xpath(`${card}//*[contains(., 'A task is waiting')]`);
  const what = await text(xpath(`${card}//div[@aria-label='What will happen']`));
  if (!what.includes("deliver_file")) throw new Error(`the card does not name the action: ${what}`);
  await click(button("Approve once", card));
  await seen("Approved.");
  await click(button("Accept", "//article[contains(@class,'review')]"));
  await seen("Accepted.");
});

await step("see the outcome with its evidence", async () => {
  await run("location.hash = arguments[0]", taskHash);
  await xpath("//span[contains(@class,'chip') and normalize-space()='Done']");
  await xpath("//div[contains(@class,'evidence') and contains(@class,'pass')]//span[normalize-space()='holds']");
  await xpath(`//div[contains(@class,'evidence') and contains(@class,'pass')]//span[normalize-space()='confirmed by ${person}']`);
  const sandbox = await run("return document.querySelector('iframe.composed')?.getAttribute('sandbox')");
  if (sandbox === null || /allow-scripts/.test(sandbox)) throw new Error(`the outcome frame may run scripts: ${sandbox}`);
  await until("the composed outcome", () => run("const f = document.querySelector('iframe.composed'); return f && f.contentDocument && f.contentDocument.body.innerText.includes('41.7M')"));
});

await step("download the delivered file", async () => {
  await click(css("button[data-act=download]"));
  await seen("Saved ");
  const files = await until("the file in Downloads", () => { try { const f = readdirSync(downloads); return f.length ? f : null; } catch { return null; } });
  const body = readFileSync(join(downloads, files[0]), "utf8");
  if (!body.includes("41.7M")) throw new Error(`the downloaded file holds ${JSON.stringify(body)}`);
});

await call("DELETE", s(""));
console.log("app flow passed");
