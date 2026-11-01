// Connectors in the Library, as a person sees them (connectors spec, The
// connector card and Library): grouped by where they stand, each card its
// name, one plain sentence and one button, with no protocol words; a
// connected one opens to as whom, since when, what it allows, what Magnus
// did through it, and Disconnect. The owner adds an app by its address, the
// one field. The test arranges: calendar connected for the person and used
// once, notes not connected.
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

await step("the Library groups the organisation's apps by where the person stands", async () => {
  await page.getByRole("link", { name: "Library", exact: true }).click();
  const connected = page.locator("#detail .cgroup[data-group=connected]");
  const available = page.locator("#detail .cgroup[data-group=available]");
  await connected.getByText("Connected", { exact: true }).first().waitFor({ timeout: T });
  await connected.locator("[data-connector=calendar]").waitFor({ timeout: T });
  await available.locator("[data-connector=notes]").getByRole("button", { name: "Connect" }).waitFor({ timeout: T });
  const said = await page.locator("#detail .clist").allInnerTexts();
  if (PROTOCOL.test(said.join(" "))) throw new Error(`a card shows a protocol word: ${said.join(" | ")}`);
  if (!said.join(" ").includes("Magnus can read and make changes that can be undone in Calendar.")) throw new Error(`the calendar card does not say what Magnus can do: ${said}`);
});

await step("a search finds connectors too", async () => {
  await page.getByLabel(/Search files and connectors/).fill("notes");
  await page.locator("#detail [data-connector=calendar]").waitFor({ state: "detached", timeout: T });
  await page.locator("#detail [data-connector=notes]").waitFor({ timeout: T });
  await page.getByLabel(/Search files and connectors/).fill("");
  await page.locator("#detail [data-connector=calendar]").waitFor({ timeout: T });
});

await step("connecting happens in the desktop app, on the app's own page", async () => {
  await page.locator("#detail [data-connector=notes]").getByRole("button", { name: "Connect" }).click();
  await page.getByText("Connect apps from the Magnus desktop app.").waitFor({ timeout: T });
});

await step("a connected app opens to as whom, since when, what it allows and what Magnus did", async () => {
  await page.locator("#detail [data-connector=calendar]").getByRole("link", { name: "Calendar" }).click();
  const d = page.locator("#detail");
  await d.getByRole("heading", { name: "Calendar" }).waitFor({ timeout: T });
  for (const t of ["Connected as", "Last used", "What it allows", "Reads: list events", "Changes that can be undone: create event", "Recent actions", "Create event"]) await d.getByText(t, { exact: true }).first().waitFor({ timeout: T });
  const text = await d.innerText();
  if (PROTOCOL.test(text)) throw new Error(`the connector's page shows a protocol word: ${text}`);
});

await step("Disconnect ends the person's sign-in; the app is available to them again", async () => {
  await page.locator("#detail").getByRole("button", { name: "Disconnect" }).click();
  await page.getByText("Disconnected.").waitFor({ timeout: T });
  if (!dialogs.some(m => m.includes("Your sign-in is ended at the app"))) throw new Error(`no confirmation said what it does: ${dialogs}`);
  await page.getByRole("link", { name: "Library", exact: true }).click();
  await page.locator("#detail .cgroup[data-group=available] [data-connector=calendar]").waitFor({ timeout: T });
});

await step("the owner adds an app for everyone by its address alone", async () => {
  await page.getByRole("link", { name: "Settings" }).click();
  await page.getByRole("link", { name: "Connectors" }).click();
  const form = page.locator(".card", { hasText: "Add an app for everyone" });
  await form.waitFor({ timeout: T });
  const fields = await form.locator("input, select, textarea").count();
  if (fields !== 1) throw new Error(`adding an app asks for ${fields} fields, not its address alone`);
});

if (errors.length) { console.error(`FAIL page errors: ${errors.join("; ")}`); await browser.close(); process.exit(1); }
await browser.close();
console.log("connectors passed");
