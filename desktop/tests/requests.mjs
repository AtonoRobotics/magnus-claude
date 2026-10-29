// A request set compared across models, in a real browser: the Experiments
// view lists each request against each model with its choice, stops, cost
// and time, and a judgment the person clicks is kept.
//
// MAGNUS_URL, MAGNUS_PERSON, MAGNUS_PASSWORD  where and who (a developer)
// MAGNUS_EXPERIMENT  the experiment; MAGNUS_MODELS  how many models ran it
// MAGNUS_CHROMIUM  Chrome to drive; MAGNUS_SHOTS  where to keep screenshots
import { createRequire } from "node:module";
import { mkdirSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const { chromium } = createRequire(import.meta.url)(join(root, "browser/node_modules/playwright-core"));
const env = k => { const v = process.env[k]; if (!v) { console.error(`${k} is not set`); process.exit(2); } return v; };
const [base, person, password, experiment, models] = ["MAGNUS_URL", "MAGNUS_PERSON", "MAGNUS_PASSWORD", "MAGNUS_EXPERIMENT", "MAGNUS_MODELS"].map(env);
const shots = process.env.MAGNUS_SHOTS;
if (shots) mkdirSync(shots, { recursive: true });
const win = JSON.parse(readFileSync(join(root, "desktop/src-tauri/tauri.conf.json"), "utf8")).app.windows[0];

const browser = await chromium.launch({ executablePath: env("MAGNUS_CHROMIUM"), args: ["--no-sandbox", "--disable-dev-shm-usage"] });
const page = await browser.newPage({ viewport: { width: win.width, height: win.height } });
const errors = [];
page.on("pageerror", e => errors.push(String(e)));
let n = 0;
const shot = async name => { if (shots) await page.screenshot({ path: join(shots, `requests-${String(++n).padStart(2, "0")}-${name}.png`), fullPage: true }); };
const step = async (name, f) => {
  try { await f(); await shot(name); console.log(`ok   ${name}`); }
  catch (e) { await shot(`FAILED-${name}`); console.error(`FAIL ${name}: ${e.message}`); await browser.close(); process.exit(1); }
};
const T = 60_000;

await step("sign in", async () => {
  await page.goto(base);
  await page.getByLabel("Your name on the appliance").fill(person);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await page.getByRole("heading", { name: "Home" }).waitFor({ timeout: T });
});

await step("each request against each model", async () => {
  await page.goto(`${base}/#/dev/experiment/${experiment}`);
  await page.locator("table[data-summary] tbody tr").first().waitFor({ timeout: T });
  const summary = await page.locator("table[data-summary] tbody tr").count();
  if (summary !== Number(models)) throw new Error(`${summary} models in the summary, not ${models}`);
  const cards = page.locator("[data-case]");
  for (let i = 0; i < await cards.count(); i++) {
    const rows = await cards.nth(i).locator("tr[data-request]").count();
    if (rows !== Number(models)) throw new Error(`request ${i + 1} shows ${rows} models`);
  }
  await page.locator("[data-case]", { hasText: "Email Bob the agenda." }).getByText("Started tasks").first().waitFor({ timeout: T });
  await page.locator("[data-case]", { hasText: "Every Monday" }).getByText("Proposed a schedule").first().waitFor({ timeout: T });
});

await step("a judgment is kept", async () => {
  const row = () => page.locator("[data-case]", { hasText: "How many legs" }).locator("tr[data-request]").first();
  const model = (await row().locator("td").first().innerText()).trim();
  const judged = async () => {
    const t = await page.locator("table[data-summary] tr", { hasText: model }).locator("td").nth(1).innerText();
    const m = t.match(/(\d+) of (\d+)/);
    return m ? Number(m[2]) : 0;
  };
  const before = await judged();
  await row().getByRole("button", { name: "Correct choice" }).click();
  await row().getByRole("button", { name: "Correct choice" }).and(page.locator(".primary")).waitFor({ timeout: T });
  await page.reload();
  await row().getByRole("button", { name: "Correct choice" }).and(page.locator(".primary")).waitFor({ timeout: T });
  const after = await judged();
  if (after !== before + 1) throw new Error(`${model} shows ${after} judged after the click, ${before} before`);
});

if (errors.length) { console.error(`FAIL page errors:\n${errors.join("\n")}`); await browser.close(); process.exit(1); }
await browser.close();
console.log("requests passed");
