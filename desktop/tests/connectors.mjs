// Connectors in the Library, as a person sees them (connectors spec, The
// connector card and Library), with the real catalog: Google's Gmail,
// Calendar, Drive and Chat, online through the organisation's registration
// with Google (set by the test from CI's secrets). Each card is its name,
// one plain sentence and one button, with no protocol words; a connector
// opens to what it allows. Connecting is the desktop app's, at Google's own
// page. The owner sees the registration set and adds an app by its
// address, the one field. No fakes (owner, 2026-11-01).
//
// MAGNUS_URL, MAGNUS_PERSON, MAGNUS_PASSWORD  where and who (the owner)
// MAGNUS_CHROMIUM  Chrome to drive; MAGNUS_SHOTS  where to keep screenshots
import { createRequire } from "node:module";
import { mkdirSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const { chromium } = createRequire(import.meta.url)(join(root, "browser/node_modules/playwright-core"));
const env = k => { const v = process.env[k]; if (!v) { console.error(`${k} is not set`); process.exit(2); } return v; };
const [base, person, password] = ["MAGNUS_URL", "MAGNUS_PERSON", "MAGNUS_PASSWORD"].map(env);
const shots = process.env.MAGNUS_SHOTS;
if (shots) mkdirSync(shots, { recursive: true });
const win = JSON.parse(readFileSync(join(root, "desktop/src-tauri/tauri.conf.json"), "utf8")).app.windows[0];

const browser = await chromium.launch({ executablePath: env("MAGNUS_CHROMIUM"), args: ["--no-sandbox", "--disable-dev-shm-usage"] });
const page = await browser.newPage({ viewport: { width: win.width, height: win.height } });
const errors = [];
page.on("pageerror", e => errors.push(String(e)));
const dialogs = [];
page.on("dialog", d => { dialogs.push(d.message()); d.accept(); });
let n = 0;
const shot = async name => { if (shots) await page.screenshot({ path: join(shots, `connectors-${String(++n).padStart(2, "0")}-${name}.png`), fullPage: true }).catch(e => console.error(`screenshot not kept: ${e.message.split("\n")[0]}`)); };
const step = async (name, f) => {
  try { await f(); await shot(name); console.log(`ok   ${name}`); }
  catch (e) { await shot(`FAILED-${name}`); console.error(`FAIL ${name}: ${e.message}`); await browser.close(); process.exit(1); }
};
const T = 60_000;
const PROTOCOL = /\b(oauth|scopes?|tokens?|mcp|port|https?:)/i;

await step("sign in", async () => {
  await page.goto(base);
  await page.getByLabel("Username").fill(person);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await page.getByRole("button", { name: /^Needs you/ }).waitFor({ timeout: T });
});

await step("the Library lists the catalog's Google apps, available to connect", async () => {
  await page.getByRole("link", { name: "Library", exact: true }).click();
  const available = page.locator("#detail .cgroup[data-group=available]");
  for (const name of ["google-gmail", "google-calendar", "google-drive", "google-chat"]) await available.locator(`[data-connector=${name}]`).getByRole("button", { name: "Connect" }).waitFor({ timeout: T });
  const said = (await page.locator("#detail .clist").allInnerTexts()).join(" ");
  if (PROTOCOL.test(said)) throw new Error(`a card shows a protocol word: ${said}`);
  for (const t of ["Gmail", "Google Calendar", "Google Drive", "Google Chat", "Magnus can search and read your email"]) if (!said.includes(t)) throw new Error(`the cards do not say "${t}": ${said}`);
});

await step("a search finds connectors too", async () => {
  await page.getByLabel(/Search files and connectors/).fill("drive");
  await page.locator("#detail [data-connector=google-gmail]").waitFor({ state: "detached", timeout: T });
  await page.locator("#detail [data-connector=google-drive]").waitFor({ timeout: T });
  await page.getByLabel(/Search files and connectors/).fill("");
  await page.locator("#detail [data-connector=google-gmail]").waitFor({ timeout: T });
});

await step("connecting happens in the desktop app, on the app's own page", async () => {
  await page.locator("#detail [data-connector=google-gmail]").getByRole("button", { name: "Connect" }).click();
  await page.getByText("Connect apps from the Magnus desktop app.").waitFor({ timeout: T });
});

await step("a connector opens to what it allows, in plain words", async () => {
  await page.locator("#detail [data-connector=google-gmail]").getByRole("link", { name: "Gmail" }).click();
  const d = page.locator("#detail");
  await d.getByRole("heading", { name: "Gmail" }).waitFor({ timeout: T });
  for (const t of ["What it allows", "Recent actions", "Magnus has not used it for you yet."]) await d.getByText(t, { exact: true }).first().waitFor({ timeout: T });
  const text = await d.innerText();
  if (!/Reads: .*search threads/.test(text)) throw new Error(`it does not say what Magnus may read: ${text}`);
  if (PROTOCOL.test(text)) throw new Error(`the connector's page shows a protocol word: ${text}`);
  if (await d.getByRole("button", { name: "Disconnect" }).count()) throw new Error("Disconnect is offered for an app the person has not connected");
});

await step("the owner sees the Google registration set, and adds an app by its address alone", async () => {
  await page.getByRole("link", { name: "Settings" }).click();
  await page.getByRole("link", { name: "Connectors" }).click();
  const reg = page.locator("[data-registration=google]");
  await reg.getByText("Set", { exact: true }).waitFor({ timeout: T });
  await page.locator('[data-app="google-gmail"]').getByText("from the catalog").waitFor({ timeout: T });
  const form = page.locator(".card", { hasText: "Add an app for everyone" });
  const fields = await form.locator("input, select, textarea").count();
  if (fields !== 1) throw new Error(`adding an app asks for ${fields} fields, not its address alone`);
});

if (errors.length) { console.error(`FAIL page errors: ${errors.join("; ")}`); await browser.close(); process.exit(1); }
await browser.close();
console.log("connectors passed");
