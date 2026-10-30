// One workspace, in the installed app: research is started in a
// conversation, approved there, its result read and its file downloaded
// there, and a schedule created and checked there, without leaving the
// conversation: no page loads, no change of address, no navigation at all.
// The decisions are the same cards under Needs you, and acting on one in the
// thread updates Needs you.
//
// MAGNUS_WEBDRIVER  tauri-driver's address      MAGNUS_APP  the app binary
// MAGNUS_URL        the appliance (https)       MAGNUS_FINGERPRINT  what it printed
// MAGNUS_PROVIDER   the identity provider's name on the sign-in button
// MAGNUS_PERSON     who signs in
// MAGNUS_RESEARCH   the message that starts the research
// MAGNUS_SCHEDULE   the message that asks for a schedule
// MAGNUS_SHOTS      optional: screenshots
import { readFileSync } from "node:fs";
import { env, open, pinAndSignIn, savedPath } from "./appdriver.mjs";

const [wd, app, url, fingerprint, provider, person, research, schedule] =
  ["MAGNUS_WEBDRIVER", "MAGNUS_APP", "MAGNUS_URL", "MAGNUS_FINGERPRINT", "MAGNUS_PROVIDER", "MAGNUS_PERSON", "MAGNUS_RESEARCH", "MAGNUS_SCHEDULE"].map(env);
const d = await open(wd, app, process.env.MAGNUS_SHOTS, "workspace");
const { step, css, xpath, button, click, type, text, run, until } = d;

const thread = "//div[@id='thread']";
const panel = "//aside[@id='detail']";
const task = `${thread}//li[@data-chat-task]`;
const count = () => run("return document.querySelector('[data-needs-count]')?.innerText.trim()");
const present = xp => run("return document.evaluate(arguments[0], document, null, XPathResult.FIRST_ORDERED_NODE_TYPE, null).singleNodeValue !== null", xp);
const gone = (what, xp) => until(what, async () => !(await present(xp)) || null);
const say = async words => {
  await type(css("#msg"), words);
  await click(css("#send"));
};

await pinAndSignIn(d, { url, fingerprint, provider, person });

// From here on, any navigation is counted: a change of address, a history
// step, or a page load (which would lose the mark set on the window).
let address;
await step("start research in a conversation", async () => {
  await run(`window.__workspace = "kept"; window.__navigations = [];
    addEventListener("hashchange", e => __navigations.push("hashchange " + e.newURL));
    addEventListener("popstate", () => __navigations.push("popstate " + location.href));`);
  await say(research);
  await xpath(`${thread}//div[@data-kind='message']//div[normalize-space()='${research}']`);
  address = await until("the conversation's address", async () => { const h = await run("return location.hash"); return h.startsWith("#/chat/") && h !== "#/chat/new" ? h : null; });
  await xpath(task);
});

let approval;
await step("the task card shows its live status and the approval it waits on", async () => {
  await xpath(`${task}[@data-status='suspended']//span[contains(@class,'why') and normalize-space()='waiting for an approval']`);
  const card = `${task}//article[contains(@class,'approval')]`;
  approval = await d.attr(xpath(card), "data-id");
  const what = await text(xpath(`${card}//div[@aria-label='What will happen']`));
  if (!what.includes("deliver_file")) throw new Error(`the card does not name the action: ${what}`);
});

await step("Needs you counts it and holds the same card", async () => {
  await until("Needs you to count 1", async () => (await count()) === "1" || null);
  await click(xpath("//nav//button[contains(@class,'needs')]"));
  await xpath(`${panel}//article[@data-id='${approval}']`);
});

await step("approve in the conversation; Needs you follows", async () => {
  await click(button("Approve once", `${task}//article[@data-id='${approval}']`));
  await d.seen("Approved.");
  await gone("the approval in the thread", `${thread}//article[@data-id='${approval}']`);
  await gone("the approval under Needs you", `${panel}//article[@data-id='${approval}']`);
});

await step("accept the outcome's review in the conversation", async () => {
  const review = `${task}//article[contains(@class,'review')]`;
  const id = await d.attr(xpath(review), "data-id");
  await xpath(`${panel}//article[@data-id='${id}']`);
  await click(button("Accept", review));
  await d.seen("Accepted.");
  await gone("the review under Needs you", `${panel}//article[@data-id='${id}']`);
  await until("Needs you to count 0", async () => (await count()) === "0" || null);
  await xpath(`${task}[@data-status='completed']`);
});

await step("read the result beside the conversation", async () => {
  const result = `${thread}//div[@data-result]`;
  await xpath(`${result}//span[contains(@class,'chip') and normalize-space()='Done']`);
  await until("the file's preview in the thread", () => run("const p = document.querySelector('#thread [data-result] [data-preview] .glimpse pre'); return p && p.innerText.includes('41.7M')"));
  await click(xpath(`${result}//a[normalize-space()='See the outcome']`));
  await xpath(`${panel}//div[contains(@class,'evidence') and contains(@class,'pass')]//span[normalize-space()='holds']`);
  await xpath(`${panel}//div[contains(@class,'evidence') and contains(@class,'pass')]//span[normalize-space()='confirmed by ${person}']`);
  await until("the composed outcome", () => run("const f = document.querySelector('#detail iframe.composed'); return f && f.contentDocument && f.contentDocument.body.innerText.includes('41.7M')"));
});

await step("download the file from the conversation", async () => {
  const body = readFileSync(await savedPath(d, () => click(xpath(`${thread}//div[@data-result]//button[normalize-space()='Download']`))), "utf8");
  if (!body.includes("41.7M")) throw new Error(`the downloaded file holds ${JSON.stringify(body)}`);
});

await step("create a schedule in the conversation", async () => {
  await say(schedule);
  const card = `${thread}//div[@data-card]`;
  await click(button("Confirm", card));
  await xpath(`${card}//span[contains(@class,'chip') and normalize-space()='Scheduled']`);
});

await step("check the schedule's status in the conversation and beside it", async () => {
  const card = `${thread}//div[@data-card]`;
  const said = await until("the schedule's next run", async () => { const t = await text(xpath(`${card}//span[@data-last]`)); return t.startsWith("Next run") ? t : null; });
  if (!said.includes("No results yet")) throw new Error(`the new schedule says ${said}`);
  await click(xpath(`${card}//a[normalize-space()='Open the schedule']`));
  await xpath(`${panel}//div[@data-schedule]//div[@data-last][starts-with(normalize-space(),'Next run')]`);
  await xpath(`${panel}//button[normalize-space()='Stop this schedule']`);
});

await step("no navigation happened", async () => {
  const [kept, navigations, now] = await run("return [window.__workspace, window.__navigations, location.hash]");
  if (kept !== "kept") throw new Error("the page was loaded again");
  if (navigations.length) throw new Error(`the app navigated: ${navigations.join("; ")}`);
  if (now !== address) throw new Error(`the address changed from ${address} to ${now}`);
  await xpath(`${thread}//div[@data-kind='message']//div[normalize-space()='${research}']`);
});

await d.close();
console.log("workspace passed");
