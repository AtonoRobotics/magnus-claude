// What a task started from chat delivers, drawn in the conversation, in a
// real browser (tests/scratch/tests/chat_results.rs sets it up): the trip's
// result in the thread shows its timeline and its table.
//
// MAGNUS_URL, MAGNUS_PERSON, MAGNUS_PASSWORD  where and who
// MAGNUS_THREAD    the conversation; MAGNUS_TASK  the finished task started there
// MAGNUS_CHROMIUM  Chrome to drive; MAGNUS_SHOTS  optional: a screenshot
import { createRequire } from "node:module";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const { chromium } = createRequire(import.meta.url)(join(root, "browser/node_modules/playwright-core"));
const env = k => { const v = process.env[k]; if (!v) { console.error(`${k} is not set`); process.exit(2); } return v; };
const [base, person, password, thread, task] = ["MAGNUS_URL", "MAGNUS_PERSON", "MAGNUS_PASSWORD", "MAGNUS_THREAD", "MAGNUS_TASK"].map(env);
const T = 60_000;

const browser = await chromium.launch({ executablePath: env("MAGNUS_CHROMIUM"), args: ["--no-sandbox", "--disable-dev-shm-usage"] });
const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
const errors = [];
page.on("pageerror", e => errors.push(String(e)));
const fail = async m => { console.error(`FAIL ${m}`); if (process.env.MAGNUS_SHOTS) await page.screenshot({ path: join(process.env.MAGNUS_SHOTS, "chat-results.png") }).catch(() => {}); await browser.close(); process.exit(1); };

await page.goto(base);
await page.getByLabel("Username").fill(person);
await page.getByLabel("Password").fill(password);
await page.getByRole("button", { name: "Sign in" }).click();
await page.getByRole("button", { name: /^Needs you/ }).waitFor({ timeout: T });
// The conversation the trip was asked for in.
await page.locator(`nav.side a[href="#/chat/${thread}"]`).first().click();
const result = page.locator(`#thread [data-result="${task}"]`);
try { await result.waitFor({ timeout: T }); } catch { await fail("the trip's result is not in the conversation"); }

// Its deliverable is drawn there, in its frame.
const frame = result.locator("iframe.composed[data-shown]");
if (!(await frame.count())) await fail("the trip's result in the conversation draws no deliverable");
const drawn = await page.waitForFunction(id => {
  const f = document.querySelector(`#thread [data-result="${id}"] iframe.composed[data-shown]`);
  const d = f?.contentDocument;
  if (!d?.querySelector(".tl") || !d.querySelector("table")) return null;
  return { days: d.querySelectorAll(".tl li").length, total: [...d.querySelectorAll("td")].some(td => td.textContent.trim() === "$912"), height: f.getBoundingClientRect().height };
}, task, { timeout: T }).then(h => h.jsonValue()).catch(() => null);
if (!drawn) await fail("the deliverable's frame in the conversation is empty");
if (drawn.days !== 3) await fail(`the timeline shows ${drawn.days} of its 3 days`);
if (!drawn.total) await fail("the table's total is not shown");
if (drawn.height < 100) await fail(`the deliverable's frame is ${drawn.height}px tall`);
if (process.env.MAGNUS_SHOTS) await page.screenshot({ path: join(process.env.MAGNUS_SHOTS, "chat-results.png") });

if (errors.length) await fail(`page errors: ${errors.join("; ")}`);
await browser.close();
console.log("the deliverable shows in the conversation");
