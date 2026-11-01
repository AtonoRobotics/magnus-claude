// Connectors in the Library, as a person sees them (connectors spec rev 46,
// What the user sees, The connector card and Library), with the real
// catalog: Google's Gmail, Calendar, Drive and Chat, already set up. Each
// card is its name, one plain sentence and one button, with no protocol
// words; a connector opens to what it allows. Connecting is the desktop
// app's, at Google's own page. Nothing on the connect path, and nothing in
// the owner's Connectors settings, asks anyone to type anything: no client
// ID, secret or address. No fakes (owner, 2026-11-01).
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
// Text a person could type in, other than the Library's own search box.
const typed = scope => page.locator(`${scope} input:not([type=search]):not([type=checkbox]):not([type=radio]):not([type=file]):not([type=hidden]), ${scope} textarea`).count();

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

await step("connecting happens in the desktop app, on the app's own page, and asks nothing here", async () => {
  if (await typed("#detail")) throw new Error("the Library asks for something to be typed");
  await page.locator("#detail [data-connector=google-gmail]").getByRole("button", { name: "Connect" }).click();
  await page.getByText("Connect apps from the Magnus desktop app.").waitFor({ timeout: T });
  for (const scope of ["#detail", "[role=dialog]", ".sheet", ".toast"]) if (await typed(scope)) throw new Error(`connecting asks for something to be typed in Magnus (${scope})`);
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

await step("the owner's Connectors settings are the catalog's apps, and ask for nothing", async () => {
  await page.getByRole("link", { name: "Settings" }).click();
  await page.getByRole("link", { name: "Connectors" }).click();
  await page.locator('[data-app="google-gmail"]').getByText("from the catalog").waitFor({ timeout: T });
  const said = await page.locator("main").innerText();
  if (/client (id|secret)|registration|its address/i.test(said)) throw new Error(`the settings ask for setup: ${said}`);
  if (await typed("main")) throw new Error("the owner's Connectors settings ask for something to be typed");
});

await step("from Settings, the Library's address opens the Library", async () => {
  await page.evaluate(() => { location.hash = "#/library"; });
  await page.locator("#detail [data-connector=google-gmail]").waitFor({ timeout: T });
});

if (errors.length) { console.error(`FAIL page errors: ${errors.join("; ")}`); await browser.close(); process.exit(1); }
await browser.close();
console.log("connectors passed");
