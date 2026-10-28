// The operator flow, through the page a person uses: sign in, connect
// Claude, type a plain-language task, approve in the inbox, and see the
// outcome with its evidence. It checks behavior, not appearance (U7).
//
// MAGNUS_URL       the appliance (its own page, or a page that loads ../ui)
// MAGNUS_PERSON    the person signing in, with either
//   MAGNUS_PASSWORD  their password, or
//   MAGNUS_PROVIDER  the identity provider's name on the sign-in button
//                    (the person approves on the provider's page)
// MAGNUS_API_KEY   the key to paste when connecting Claude
// MAGNUS_TASK      the task to type
// MAGNUS_CHROMIUM  Chrome to drive; MAGNUS_SHOTS  where to keep screenshots
import { createRequire } from "node:module";
import { mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const { chromium } = createRequire(import.meta.url)(join(root, "browser/node_modules/playwright-core"));
const env = k => { const v = process.env[k]; if (!v) { console.error(`${k} is not set`); process.exit(2); } return v; };
const [base, person, apiKey, task] = ["MAGNUS_URL", "MAGNUS_PERSON", "MAGNUS_API_KEY", "MAGNUS_TASK"].map(env);
const provider = process.env.MAGNUS_PROVIDER;
const password = provider ? null : env("MAGNUS_PASSWORD");
const shots = process.env.MAGNUS_SHOTS;
if (shots) mkdirSync(shots, { recursive: true });

const browser = await chromium.launch({ executablePath: env("MAGNUS_CHROMIUM"), args: ["--no-sandbox", "--disable-dev-shm-usage"] });
const page = await browser.newPage({ viewport: { width: 1200, height: 900 } });
const errors = [];
page.on("pageerror", e => errors.push(String(e)));
let n = 0;
const shot = async name => { if (shots) await page.screenshot({ path: join(shots, `${String(++n).padStart(2, "0")}-${name}.png`), fullPage: true }); };
const step = async (name, f) => {
  try { await f(); await shot(name); console.log(`ok   ${name}`); }
  catch (e) { await shot(`FAILED-${name}`); console.error(`FAIL ${name}: ${e.message}`); await browser.close(); process.exit(1); }
};
const T = 60_000;

if (provider) await step(`sign in with ${provider}`, async () => {
  await page.goto(base);
  const popup = page.waitForEvent("popup", { timeout: T });
  await page.getByRole("button", { name: `Sign in with ${provider}` }).click();
  // The provider's page opens with the code; the person approves there.
  const idp = await popup;
  await idp.waitForLoadState();
  const code = (await page.locator("#code").innerText()).trim();
  if (!idp.url().includes(code)) throw new Error(`the provider's page ${idp.url()} is not for code ${code}`);
  await idp.getByText(`Signed in as ${person}`).waitFor({ timeout: T });
  await page.getByRole("heading", { name: "Home" }).waitFor({ timeout: T });
  const page_kept = await page.evaluate(() => JSON.stringify(localStorage) + JSON.stringify(sessionStorage) + document.cookie);
  if (page_kept.includes("mgs_")) throw new Error("the session token reached the page");
});

if (!provider) await step("sign in", async () => {
  await page.goto(base);
  await page.getByLabel("Your name on the appliance").fill(person);
  await page.getByLabel("Password").fill("not the password");
  await page.getByRole("button", { name: "Sign in" }).click();
  await page.getByRole("alert").filter({ hasText: "wrong" }).waitFor({ timeout: T });
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await page.getByRole("heading", { name: "Home" }).waitFor({ timeout: T });
});

await step("connect Claude", async () => {
  await page.getByRole("link", { name: "Models" }).click();
  const card = page.locator("[data-adapter=anthropic]");
  await card.getByRole("button", { name: "Connect" }).click();
  await card.getByLabel("API key").fill(apiKey);
  await card.getByRole("button", { name: "Save to the vault" }).click();
  await card.getByText("Connected.").waitFor({ timeout: T });
  // The key went to the appliance; the page does not keep it.
  const kept = await page.evaluate(k => JSON.stringify(localStorage).includes(k) || JSON.stringify(sessionStorage).includes(k) || document.cookie.includes(k), apiKey);
  if (kept) throw new Error("the page kept the API key");
  const pick = page.getByLabel("Model");
  await pick.waitFor({ timeout: T });
  await page.getByLabel("Provider").selectOption("anthropic");
  await pick.selectOption({ index: 0 });
  await page.getByRole("button", { name: "Use this model" }).click();
  await page.getByText("New tasks will use this model.").waitFor({ timeout: T });
});

let taskUrl;
await step("type a task", async () => {
  await page.getByRole("link", { name: "New task" }).click();
  await page.getByRole("textbox").fill(task);
  await page.getByRole("button", { name: "Start" }).click();
  await page.waitForURL(/#\/task\//, { timeout: T });
  taskUrl = page.url();
  await page.getByRole("heading", { name: task.slice(0, 40) }).waitFor({ timeout: T }).catch(() => {});
});

await step("approve in the inbox", async () => {
  await page.getByRole("link", { name: /Inbox/ }).click();
  const card = page.locator("article.item.approval").first();
  await card.waitFor({ timeout: T });
  // The card says exactly what will happen, and that a task waits on it.
  await card.getByText("A task is waiting").waitFor({ timeout: T });
  const what = await card.getByLabel("What will happen").innerText();
  if (!/deliver_file/.test(what)) throw new Error(`the card does not name the action: ${what}`);
  await shot("approval card");
  await card.getByRole("button", { name: "Approve once" }).click();
  await page.getByText("Approved.").waitFor({ timeout: T });
  // A criterion no check can settle comes back as a review card.
  const review = page.locator("article.item.review").first();
  await review.waitFor({ timeout: T });
  await shot("review card");
  await review.getByRole("button", { name: "Accept" }).click();
  await page.getByText("Accepted.").waitFor({ timeout: T });
});

await step("see the outcome with its evidence", async () => {
  await page.goto(taskUrl);
  await page.locator(".chip", { hasText: "Done" }).waitFor({ timeout: T });
  await page.locator(".evidence.pass").first().waitFor({ timeout: T });
  const holds = await page.locator(".evidence.pass .chip.ok", { hasText: "holds" }).count();
  if (!holds) throw new Error("no criterion shows that it holds in the world");
  const confirmed = await page.locator(".evidence.pass .chip.ok", { hasText: `confirmed by ${person}` }).count();
  if (!confirmed) throw new Error("the reviewed criterion does not say who confirmed it");
  await page.getByText(/1 of 2 criteria checked against the world, 1 confirmed by a person/).waitFor({ timeout: T });
  await page.locator(".evidence.pass details").first().click();
  await page.locator(".evidence.pass pre").first().waitFor({ timeout: T });
  const frame = page.frameLocator("iframe.composed");
  await frame.locator("body").getByText(/\S/).first().waitFor({ timeout: T });
  // The composed outcome cannot run scripts.
  const sandbox = await page.locator("iframe.composed").getAttribute("sandbox");
  if (sandbox === null || /allow-scripts/.test(sandbox)) throw new Error(`the outcome frame may run scripts: ${sandbox}`);
  const files = await page.locator("button[data-act=download]").count();
  if (!files) throw new Error("the delivered file is not offered");
  await shot("outcome");
  // The plan left u1 active; once the task is done, nothing shows as active.
  await page.getByRole("tab", { name: "Contract and plan" }).click();
  await page.locator("[data-unit-status]").first().waitFor({ timeout: T });
  const states = await page.locator("[data-unit-status]").evaluateAll(els => els.map(e => e.dataset.unitStatus));
  if (states.some(s => s === "active" || s === "pending")) throw new Error(`a finished task shows plan units as ${states.join(", ")}`);
  await page.getByRole("tab", { name: "Full trace" }).click();
  await page.locator(".call").first().waitFor({ timeout: T });
});

if (errors.length) { console.error(`FAIL page errors:\n${errors.join("\n")}`); await browser.close(); process.exit(1); }
await browser.close();
console.log("flow passed");
