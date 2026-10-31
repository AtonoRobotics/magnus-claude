// The Schedules screen shows a paused schedule as paused, with why, its
// end and its budget per run; Resume starts it again, and Stop schedule,
// once confirmed, removes it.
//
// MAGNUS_URL, MAGNUS_PERSON, MAGNUS_PASSWORD  where and who
// MAGNUS_SCHEDULE  a paused schedule
// MAGNUS_CHROMIUM  Chrome to drive; MAGNUS_SHOTS  where to keep screenshots
import { createRequire } from "node:module";
import { mkdirSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const { chromium } = createRequire(import.meta.url)(join(root, "browser/node_modules/playwright-core"));
const env = k => { const v = process.env[k]; if (!v) { console.error(`${k} is not set`); process.exit(2); } return v; };
const [base, person, password, id] = ["MAGNUS_URL", "MAGNUS_PERSON", "MAGNUS_PASSWORD", "MAGNUS_SCHEDULE"].map(env);
const shots = process.env.MAGNUS_SHOTS;
if (shots) mkdirSync(shots, { recursive: true });
const win = JSON.parse(readFileSync(join(root, "desktop/src-tauri/tauri.conf.json"), "utf8")).app.windows[0];

const browser = await chromium.launch({ executablePath: env("MAGNUS_CHROMIUM"), args: ["--no-sandbox", "--disable-dev-shm-usage"] });
const page = await browser.newPage({ viewport: { width: win.width, height: win.height } });
const errors = [];
page.on("pageerror", e => errors.push(String(e)));
const dialogs = [];
page.on("dialog", d => { dialogs.push(d.message()); d.accept(); });
const T = 60_000;
const step = async (name, f) => {
  try { await f(); if (shots) await page.screenshot({ path: join(shots, `schedules-${name.replace(/\W+/g, "-")}.png`), fullPage: true }); console.log(`ok   ${name}`); }
  catch (e) { if (shots) await page.screenshot({ path: join(shots, "schedules-FAILED.png"), fullPage: true }); console.error(`FAIL ${name}: ${e.message}`); await browser.close(); process.exit(1); }
};
const card = () => page.locator(`[data-schedule="${id}"]`);

await step("sign in", async () => {
  await page.goto(base);
  await page.getByLabel("Username").fill(person);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Sign in" }).click();
  // The conversation is home: the workspace shows, with Needs you.
  await page.getByRole("button", { name: /^Needs you/ }).waitFor({ timeout: T });
});

await step("the paused schedule says so, with its end and budget", async () => {
  await page.goto(`${base}/#/schedules`);
  await card().locator("[data-paused]", { hasText: "Paused" }).waitFor({ timeout: T });
  const paused = await card().locator("[data-paused]").innerText();
  if (!paused.includes("failed")) throw new Error(`no reason: ${paused}`);
  const ends = await card().locator("[data-ends]").innerText();
  if (!ends.includes("until you stop it") || !/up to \$\d/.test(ends)) throw new Error(`no end or budget: ${ends}`);
  if (await card().locator("[data-last]").innerText().then(t => t.includes("Next run"))) throw new Error("a paused schedule shows a next run");
});

await step("Resume starts it again", async () => {
  await card().getByRole("button", { name: "Resume" }).click();
  await card().locator("[data-paused]").waitFor({ state: "detached", timeout: T });
  await card().locator("[data-last]", { hasText: "Next run" }).waitFor({ timeout: T });
});

await step("Stop schedule, once confirmed, removes it", async () => {
  await card().getByRole("button", { name: "Stop schedule" }).click();
  await card().waitFor({ state: "detached", timeout: T });
  if (!dialogs.some(m => m.includes("No more of its runs start"))) throw new Error(`no confirmation said what it does: ${dialogs}`);
});

if (errors.length) { console.error(`FAIL page errors: ${errors.join("; ")}`); await browser.close(); process.exit(1); }
await browser.close();
