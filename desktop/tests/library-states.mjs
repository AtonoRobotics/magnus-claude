// The Library and an app's page as a connection changes (connectors spec,
// Library and The connector card). MAGNUS_PHASE "connected": in Yours the
// card shows a check and the account; the app's page shows Connected, the
// account, when connected and last used, the recent action linking to its
// task, and Disconnect. "needs_attention": the card asks to sign in again,
// Discover lists it under Needs attention, and the page says so with one
// button. Both in light, dark and glass, and at phone width with no
// sideways scroll, and no protocol words.
//
// MAGNUS_URL, MAGNUS_PERSON, MAGNUS_PASSWORD  where and who
// MAGNUS_APP, MAGNUS_TASK  the app, and the task that used it
// MAGNUS_CHROMIUM  Chrome to drive; MAGNUS_SHOTS  where to keep screenshots
import { createRequire } from "node:module";
import { mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const { chromium } = createRequire(import.meta.url)(join(root, "browser/node_modules/playwright-core"));
const env = k => { const v = process.env[k]; if (!v) { console.error(`${k} is not set`); process.exit(2); } return v; };
const [base, person, password, app, task, phase] = ["MAGNUS_URL", "MAGNUS_PERSON", "MAGNUS_PASSWORD", "MAGNUS_APP", "MAGNUS_TASK", "MAGNUS_PHASE"].map(env);
const shots = process.env.MAGNUS_SHOTS;
if (shots) mkdirSync(shots, { recursive: true });

const browser = await chromium.launch({ executablePath: env("MAGNUS_CHROMIUM"), args: ["--no-sandbox", "--disable-dev-shm-usage"] });
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
const errors = [];
page.on("pageerror", e => errors.push(String(e)));
let n = 0;
const shot = async name => { if (shots) await page.screenshot({ path: join(shots, `library-${phase}-${String(++n).padStart(2, "0")}-${name}.png`), fullPage: true }).catch(() => {}); };
const step = async (name, f) => {
  try { await f(); await shot(name); console.log(`ok   ${name}`); }
  catch (e) { await shot(`FAILED-${name}`); console.error(`FAIL ${name}: ${e.message}`); await browser.close(); process.exit(1); }
};
const T = 60_000;
const PROTOCOL = /\b(oauth|scopes?|tokens?|mcp|port|https?:)/i;
const go = async hash => { await page.evaluate(h => { location.hash = h; }, hash); };
const noProtocol = async loc => { const said = await loc.innerText(); if (PROTOCOL.test(said)) throw new Error(`a protocol word is shown: ${said}`); };
const everyLook = async check => {
  for (const [mode, material] of [["light", "solid"], ["dark", "solid"], ["light", "glass"], ["dark", "glass"]]) {
    for (const width of [1280, 390]) {
      await page.setViewportSize({ width, height: 800 });
      await page.evaluate(([mode, material]) => Theme.set({ mode, material }), [mode, material]);
      await check();
      const over = await page.evaluate(() => Math.max(document.documentElement.scrollWidth - document.documentElement.clientWidth, ...[...document.querySelectorAll("main")].map(m => m.scrollWidth - m.clientWidth)));
      if (over > 0) throw new Error(`${mode} ${material} at ${width}px scrolls sideways by ${over}px`);
    }
  }
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.evaluate(() => Theme.set({ mode: "system", material: "solid" }));
};

await step("sign in", async () => {
  await page.goto(base);
  await page.getByLabel("Username").fill(person);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await page.getByRole("button", { name: /^Needs you/ }).waitFor({ timeout: T });
});

const card = page.locator(`.lcard[data-connector="${app}"]`);
const cp = page.locator("section.cp");

if (phase === "connected") {
  await step("Yours: the card shows a check and the account", async () => {
    await go("#/library");
    await page.locator('[data-lib-tab="yours"]').click();
    await page.locator('[data-section="connected"]').locator(`.lcard[data-connector="${app}"]`).waitFor({ timeout: T });
    if ((await card.getAttribute("data-state")) !== "connected") throw new Error(`the card is ${await card.getAttribute("data-state")}`);
    await card.getByRole("link", { name: /is connected\. Open details$/ }).waitFor({ timeout: T });
    await card.getByText(/^Connected as carol@calendar\.test$/).waitFor({ timeout: T });
    await noProtocol(card);
  });
  await step("the app's page: account, when, recent actions and Disconnect", async () => {
    await card.getByRole("link", { name: /is connected\. Open details$/ }).click();
    await cp.getByText("Connected", { exact: true }).first().waitFor({ timeout: T });
    const kv = await cp.locator(".cp-kv").innerText();
    for (const want of ["carol@calendar.test", "Connected", "Last used"]) if (!kv.includes(want)) throw new Error(`the page lacks ${want}: ${kv}`);
    if (/Last used\s*Not yet/.test(kv)) throw new Error(`the page says it was never used: ${kv}`);
    const recent = cp.locator(`.cp-recent a[href="#/task/${task}"]`);
    await recent.waitFor({ timeout: T });
    if ((await recent.innerText()).trim() !== "List events") throw new Error(`the recent action reads ${await recent.innerText()}`);
    await cp.getByRole("button", { name: /^Disconnect / }).waitFor({ timeout: T });
    await noProtocol(cp);
  });
  await step("in every look and at phone width", async () => everyLook(() => cp.waitFor({ timeout: T })));
} else {
  await step("Yours: the card asks to sign in again", async () => {
    await go("#/library");
    await page.locator('[data-lib-tab="yours"]').click();
    await page.locator('[data-section="attention"]').locator(`.lcard[data-connector="${app}"]`).waitFor({ timeout: T });
    if ((await card.getAttribute("data-state")) !== "needs_attention") throw new Error(`the card is ${await card.getAttribute("data-state")}`);
    await card.getByRole("button", { name: /^Sign in to .+ again$/ }).waitFor({ timeout: T });
    await card.getByText(/^Sign in again to keep using/).waitFor({ timeout: T });
  });
  await step("Discover lists it under Needs attention", async () => {
    await page.locator('[data-lib-tab="discover"]').click();
    await page.locator('[data-section="attention"]').locator(`.lcard[data-connector="${app}"]`).waitFor({ timeout: T });
  });
  await step("the app's page says so, with one button", async () => {
    await go(`#/library/${app}`);
    await cp.getByText(/stopped working\. Sign in again/).waitFor({ timeout: T });
    await cp.getByRole("button", { name: /^Sign in to .+ again$/ }).waitFor({ timeout: T });
    await noProtocol(cp);
  });
  await step("in every look and at phone width", async () => everyLook(() => cp.waitFor({ timeout: T })));
}

if (errors.length) { console.error(`FAIL page errors: ${errors.join("; ")}`); process.exit(1); }
await browser.close();
