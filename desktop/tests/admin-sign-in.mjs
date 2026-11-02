// Signing in to the admin console in a real browser (admin console spec,
// Authentication and session; no passkeys, owner's rulings 1 and 2): the
// console's Sign in goes to the identity provider's own page, the person
// types their password there, and comes back to the console signed in, or
// told that only admins can enter. Then, if asked, a step-up: the provider
// asks for the password again, and the console comes back to where it was.
//
// MAGNUS_ADMIN_URL   the console (https://<hostname>:<port>)
// MAGNUS_ADMIN_AT    the console's address (ip:port), for the browser to find its hostname
// MAGNUS_PERSON, MAGNUS_PASSWORD  who signs in
// MAGNUS_EXPECT      "admin" (signed in) or "refused" (not an admin)
// MAGNUS_STEP_UP     optional: "1" to step up after signing in
// MAGNUS_VISIT       optional: a screen (e.g. "#/users") to open signed in, which
//                    must render in light, dark and glass, and at phone width
//                    with no sideways scroll (admin console spec, invariant tests, UI)
// MAGNUS_CHROMIUM    Chrome to drive; MAGNUS_SHOTS  where to keep screenshots
//
// Prints the console's session cookie (signed in) as its last line.
import { createRequire } from "node:module";
import { mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const { chromium } = createRequire(import.meta.url)(join(root, "browser/node_modules/playwright-core"));
const env = k => { const v = process.env[k]; if (!v) { console.error(`${k} is not set`); process.exit(2); } return v; };
const [base, at, person, password, expect] = ["MAGNUS_ADMIN_URL", "MAGNUS_ADMIN_AT", "MAGNUS_PERSON", "MAGNUS_PASSWORD", "MAGNUS_EXPECT"].map(env);
const host = new URL(base).hostname;
const shots = process.env.MAGNUS_SHOTS;
if (shots) mkdirSync(shots, { recursive: true });

// The organisation's DNS names the console; its certificate is the test's own.
const browser = await chromium.launch({ executablePath: env("MAGNUS_CHROMIUM"), args: ["--no-sandbox", "--disable-dev-shm-usage", `--host-resolver-rules=MAP ${host} ${at.split(":")[0]}`] });
const ctx = await browser.newContext({ ignoreHTTPSErrors: true, viewport: { width: 1280, height: 800 } });
const page = await ctx.newPage();
const errors = [];
page.on("pageerror", e => errors.push(String(e)));
let n = 0;
const shot = async name => { if (shots) await page.screenshot({ path: join(shots, `admin-sign-in-${String(++n).padStart(2, "0")}-${name}.png`), fullPage: true }).catch(() => {}); };
const step = async (name, f) => {
  try { await f(); await shot(name); console.log(`ok   ${name}`); }
  catch (e) { await shot(`FAILED-${name}`); console.error(`FAIL ${name}: ${e.message}`); await browser.close(); process.exit(1); }
};
const T = 60_000;
// On a re-authentication (a step-up), Keycloak shows who is signing in as
// fixed text and asks only for the password.
const login = async () => {
  if (await page.locator("#username").count()) await page.locator("#username").fill(person);
  await page.locator("#password").fill(password);
  await Promise.all([page.waitForEvent("load"), page.locator("#kc-login").click()]);
};

await step("the console asks for a sign-in", async () => {
  await page.goto(`${base}/`);
  await page.getByRole("link", { name: "Sign in" }).click();
  await page.locator("#username").waitFor({ timeout: T });
  if (new URL(page.url()).host === new URL(base).host) throw new Error("the password is asked on the console's page, not the provider's");
});

await step("the password on the provider's own page", login);

if (expect === "refused") {
  await step("only admins can enter", async () => {
    await page.getByRole("heading", { name: "Only admins can enter" }).waitFor({ timeout: T });
  });
} else {
  await step("back on the console, signed in", async () => {
    await page.waitForURL(u => new URL(u).host === new URL(base).host, { timeout: T });
    await page.locator("#nav").waitFor({ state: "visible", timeout: T });
    await page.locator("#me").getByText(/./).first().waitFor({ timeout: T });
  });
  if (process.env.MAGNUS_STEP_UP === "1") {
    await step("a step-up asks for the password again, and comes back", async () => {
      await page.goto(`${base}/auth/step-up?then=/%23/security`);
      await page.locator("#password").waitFor({ timeout: T });
      await login();
      await page.waitForURL(u => new URL(u).host === new URL(base).host && u.toString().endsWith("/#/security"), { timeout: T });
    });
  }
}

const visit = process.env.MAGNUS_VISIT;
if (visit && expect === "admin") {
  await step(`${visit} renders in every look and at phone width`, async () => {
    await page.goto(`${base}/${visit}`);
    await page.locator("#view h1").waitFor({ timeout: T });
    await page.locator("#view table, #view .empty").first().waitFor({ timeout: T });
    for (const [mode, material] of [["light", "solid"], ["dark", "solid"], ["light", "glass"], ["dark", "glass"]]) {
      for (const width of [1280, 390]) {
        await page.setViewportSize({ width, height: 800 });
        await page.evaluate(([mode, material]) => Theme.set({ mode, material }), [mode, material]);
        const over = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
        if (over > 0) throw new Error(`${visit} in ${mode} ${material} at ${width}px scrolls sideways by ${over}px`);
        await shot(`${visit.replace(/\W/g, "")}-${mode}-${material}-${width}`);
      }
    }
  });
}

if (errors.length) { console.error(`FAIL page errors: ${errors.join("; ")}`); await browser.close(); process.exit(1); }
const cookie = (await ctx.cookies()).find(c => c.name === "__Host-magnus_admin");
await browser.close();
console.log(cookie ? cookie.value : "");
