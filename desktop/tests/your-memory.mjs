// "Your memory" in a real browser (tests/scratch/tests/your_memory.rs sets
// it up): Settings lists what Magnus keeps about the person in its groups;
// a note is edited in place, and a fact forgotten.
//
// MAGNUS_URL, MAGNUS_PERSON, MAGNUS_PASSWORD  where and who
// MAGNUS_CHROMIUM  Chrome to drive; MAGNUS_SHOTS  optional: screenshots
import { createRequire } from "node:module";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const { chromium } = createRequire(import.meta.url)(join(root, "browser/node_modules/playwright-core"));
const env = k => { const v = process.env[k]; if (!v) { console.error(`${k} is not set`); process.exit(2); } return v; };
const [base, person, password] = ["MAGNUS_URL", "MAGNUS_PERSON", "MAGNUS_PASSWORD"].map(env);
const T = 60_000;

const browser = await chromium.launch({ executablePath: env("MAGNUS_CHROMIUM"), args: ["--no-sandbox", "--disable-dev-shm-usage"] });
const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
const errors = [];
page.on("pageerror", e => errors.push(String(e)));
page.on("dialog", d => d.accept());
let n = 0;
const shoot = name => process.env.MAGNUS_SHOTS ? page.screenshot({ path: join(process.env.MAGNUS_SHOTS, `your-memory-${String(++n).padStart(2, "0")}-${name}.png`), fullPage: true }).catch(() => {}) : null;
const step = async (name, f) => {
  try { await f(); await shoot(name); console.log(`ok   ${name}`); }
  catch (e) { await shoot(`FAILED-${name}`); console.error(`FAIL ${name}: ${e.message}`); await browser.close(); process.exit(1); }
};

await step("sign in and open Your memory", async () => {
  await page.goto(base);
  await page.getByLabel("Username").fill(person);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await page.getByRole("button", { name: /^Needs you/ }).waitFor({ timeout: T });
  await page.goto(`${base}/#/settings/memory`);
  await page.getByRole("heading", { name: "Your memory" }).waitFor({ timeout: T });
});

await step("what is kept is in its groups, one marked no longer true", async () => {
  const how = page.locator('ul[data-kind="relationship"]');
  await how.getByText("Fewer jokes, please.").waitFor({ timeout: T });
  await how.locator(".chip", { hasText: "No longer true" }).waitFor({ timeout: T });
  await page.locator('ul[data-kind="fact"]').getByText("Alice's desk is now by the door.").waitFor({ timeout: T });
  await page.locator('ul[data-kind="note"]').getByText("Reports as two pages.").waitFor({ timeout: T });
});

await step("a note is edited in place", async () => {
  const note = page.locator('ul[data-kind="note"] li').first();
  await note.getByRole("button", { name: "Edit" }).click();
  await note.getByLabel("Edit").fill("Reports as three pages.");
  await note.getByRole("button", { name: "Save" }).click();
  await page.locator('ul[data-kind="note"]').getByText("Reports as three pages.").waitFor({ timeout: T });
});

await step("a fact is forgotten", async () => {
  const fact = page.locator('ul[data-kind="fact"] li', { hasText: "parking committee" });
  await fact.getByRole("button", { name: "Forget" }).click();
  await fact.waitFor({ state: "detached", timeout: T });
});

if (errors.length) { console.error(`FAIL page errors: ${errors.join("; ")}`); process.exit(1); }
await browser.close();
