// Mail, calendar and files through the page a person uses: add an app from
// a preset and remove it, sign in to mail with a code and to the calendar
// through the browser, classify each app's tools on its review card, run a
// files task and a calendar task, and approve an inbox reply that is signed
// as Magnus for the person. It checks behavior, not appearance (U7).
//
// MAGNUS_URL       the appliance, with mail, calendar and files connectors
// MAGNUS_PERSON, MAGNUS_PASSWORD  the owner, who signs in and decides
// MAGNUS_OAUTH     the sign-in provider (its /activate page approves a code)
// MAGNUS_REPLY     the reply the task writes
// MAGNUS_CHROMIUM  Chrome to drive; MAGNUS_SHOTS  where to keep screenshots
import { createRequire } from "node:module";
import { mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
// The repository's own playwright-core (browser/), or one installed on the machine.
const load = () => { const req = createRequire(import.meta.url); try { return req(join(root, "browser/node_modules/playwright-core")); } catch { return req(process.env.MAGNUS_PLAYWRIGHT || "playwright"); } };
const { chromium } = load();
const env = k => { const v = process.env[k]; if (!v) { console.error(`${k} is not set`); process.exit(2); } return v; };
const [base, person, password, oauth, reply] = ["MAGNUS_URL", "MAGNUS_PERSON", "MAGNUS_PASSWORD", "MAGNUS_OAUTH", "MAGNUS_REPLY"].map(env);
const shots = process.env.MAGNUS_SHOTS;
if (shots) mkdirSync(shots, { recursive: true });

const browser = await chromium.launch({ executablePath: env("MAGNUS_CHROMIUM"), args: ["--no-sandbox", "--disable-dev-shm-usage"] });
const context = await browser.newContext({ viewport: { width: 1200, height: 900 } });
const page = await context.newPage();
const errors = [];
page.on("pageerror", e => errors.push(String(e)));
let n = 0;
const shot = async name => { if (shots) await page.screenshot({ path: join(shots, `${String(++n).padStart(2, "0")}-${name}.png`), fullPage: true }); };
const step = async (name, f) => {
  try { await f(); await shot(name); console.log(`ok   ${name}`); }
  catch (e) { await shot(`FAILED-${name}`); console.error(`FAIL ${name}: ${e.message}`); await browser.close(); process.exit(1); }
};
const T = 60_000;
const app = name => page.locator(`[data-connector="${name}"]`);

// A browser sign-in returns to this machine, where nothing listens for a
// page: the person copies the address it landed on. Catch that address.
let landed = null;
await context.route(/^http:\/\/127\.0\.0\.1:\d+\/\?(code|error)=/, r => {
  landed = r.request().url();
  return r.fulfill({ status: 200, contentType: "text/html", body: "<p>This address does not load: copy it back into Magnus.</p>" });
});

await step("sign in", async () => {
  await page.goto(base);
  await page.getByLabel("Your name on the appliance").fill(person);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await page.getByRole("heading", { name: "Home" }).waitFor({ timeout: T });
});

await step("see the apps, none signed in", async () => {
  await page.getByRole("link", { name: "Connectors" }).click();
  await page.getByRole("heading", { name: "Connectors" }).waitFor({ timeout: T });
  for (const a of ["mail", "calendar"]) await app(a).getByText("not signed in").waitFor({ timeout: T });
  await app("files").getByText("Runs on the appliance").waitFor({ timeout: T });
});

await step("add Gmail from its preset, then remove it", async () => {
  await page.getByLabel("App", { exact: true }).selectOption("gmail");
  await page.getByText(/Developer Preview/).waitFor({ timeout: T });
  await page.getByRole("button", { name: "Add" }).click();
  await page.getByText(/Enter the client id/i).waitFor({ timeout: T });
  await page.getByLabel(/Client ID/).fill("1234-test.apps.googleusercontent.com");
  await page.getByRole("button", { name: "Add" }).click();
  await app("gmail").waitFor({ timeout: T });
  await app("gmail").getByRole("button", { name: "Sign in to Gmail" }).waitFor({ timeout: T });
  await shot("gmail added");
  await app("gmail").getByRole("button", { name: "Remove" }).click();
  await app("gmail").waitFor({ state: "detached", timeout: T });
});

await step("sign in to mail with a code", async () => {
  await app("mail").getByRole("button", { name: "Sign in to Mail" }).click();
  const code = (await app("mail").locator("[data-code]").innerText()).trim();
  await shot("mail code");
  // The person approves the code on the provider's page.
  const approved = await page.request.get(`${oauth}/activate?user_code=${encodeURIComponent(code)}`);
  if (!approved.ok()) throw new Error(`the provider did not take ${code}`);
  await app("mail").getByText("signed in", { exact: true }).waitFor({ timeout: T });
  const kept = await page.evaluate(() => JSON.stringify(localStorage) + JSON.stringify(sessionStorage) + document.cookie);
  if (/at-\d+-/.test(kept)) throw new Error("a mail token reached the page");
});

await step("sign in to the calendar through the browser", async () => {
  const popup = page.waitForEvent("popup", { timeout: T });
  await app("calendar").getByRole("button", { name: "Sign in to Calendar" }).click();
  await popup;
  for (let i = 0; !landed && i < 300; i++) await new Promise(r => setTimeout(r, 100));
  if (!landed) throw new Error("the provider did not send the browser back");
  await app("calendar").getByRole("textbox").fill(landed);
  await shot("calendar paste");
  await app("calendar").getByRole("button", { name: "Finish signing in" }).click();
  await app("calendar").getByText("signed in", { exact: true }).waitFor({ timeout: T });
});

// The first task is what first lists the apps' tools; their review cards
// then wait in the inbox, and nothing of theirs is offered until decided.
const start = async text => {
  await page.getByRole("link", { name: "New task" }).click();
  await page.getByRole("textbox").fill(text);
  await page.getByRole("button", { name: "Start" }).click();
  await page.waitForURL(/#\/task\//, { timeout: T });
  return page.url();
};
const done = async url => {
  await page.goto(url);
  await page.locator(".chip", { hasText: "Done" }).waitFor({ timeout: T });
};

await step("review and classify each app's tools", async () => {
  await start("Look at my apps.");
  await page.getByRole("link", { name: /Inbox/ }).click();
  for (const a of ["mail", "calendar", "files"]) {
    const card = page.locator("article.item.tool_review", { hasText: `of ${a} (` });
    await card.waitFor({ timeout: T });
    await card.getByText(`${a}__`).first().waitFor({ timeout: T });
  }
  const mail = page.locator("article.item.tool_review", { hasText: "of mail (" });
  const send = mail.locator('[data-class-tool="mail__send"]');
  if (await send.locator('[data-k="risk"]').inputValue() !== "irreversible") throw new Error("send is not proposed as irreversible");
  if (await send.locator('[data-k="sends"]').inputValue() !== "body") throw new Error("send's message field is not proposed");
  // The owner keeps whoami away from tasks.
  await mail.locator('[data-class-tool="mail__whoami"] [data-k="risk"]').selectOption("prohibited");
  await shot("mail review card");
  for (const a of ["mail", "calendar", "files"]) {
    const card = page.locator("article.item.tool_review", { hasText: `of ${a} (` });
    await card.getByRole("button", { name: "Approve with these classes" }).click();
    await card.waitFor({ state: "detached", timeout: T }).catch(() => {});
  }
});

await step("a calendar task", async () => {
  await done(await start("What is on my calendar tomorrow?"));
  await page.getByText("Tomorrow: the board meeting at 15:00.").waitFor({ timeout: T });
});

await step("a files task", async () => {
  await done(await start("What does the plan in my files say?"));
  await page.getByText(/ship on Friday/).first().waitFor({ timeout: T });
});

let replyUrl;
await step("an inbox task asks before it replies", async () => {
  replyUrl = await start("Read my inbox and reply to the CFO's email.");
  await page.getByRole("link", { name: /Inbox/ }).click();
  const card = page.locator("article.item.approval").first();
  await card.waitFor({ timeout: T });
  await card.getByText("A task is waiting").waitFor({ timeout: T });
  const what = await card.getByLabel("What will happen").innerText();
  if (!what.includes("mail__send")) throw new Error(`the card does not name the action: ${what}`);
  if (!what.includes(reply)) throw new Error(`the card does not show the reply: ${what}`);
  if (!what.includes("Sent by Magnus, a digital worker, on behalf of")) throw new Error(`the card does not show who it is from: ${what}`);
  const why = await card.innerText();
  if (!/outside sources/.test(why)) throw new Error(`the card does not say the task read outside content: ${why}`);
  await shot("reply approval card");
  await card.getByRole("button", { name: "Approve once" }).click();
  await page.getByText("Approved.").waitFor({ timeout: T });
});

await step("the reply is sent and the task done", async () => {
  await done(replyUrl);
});

await browser.close();
if (errors.length) { console.error(`page errors:\n${errors.join("\n")}`); process.exit(1); }
console.log("connectors flow ok");
