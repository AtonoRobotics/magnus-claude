// The real desktop app, driven through tauri-driver (W3C WebDriver): connect
// to the appliance over HTTPS and pin its certificate, sign in through the
// identity provider, connect
// Claude, type a task, approve in the inbox, and see the outcome with its
// evidence. Behavior, not appearance (U7).
//
// MAGNUS_WEBDRIVER  tauri-driver's address      MAGNUS_APP  the app binary
// MAGNUS_URL        the appliance (https)       MAGNUS_FINGERPRINT  what it printed
// MAGNUS_PROVIDER   the identity provider's name on the sign-in button
// MAGNUS_PERSON  MAGNUS_API_KEY  MAGNUS_TASK
// MAGNUS_SHOTS      optional: where to keep screenshots
import { readFileSync } from "node:fs";
import { env, open, pinAndSignIn, savedPath } from "./appdriver.mjs";

const [wd, app, url, fingerprint, provider, person, apiKey, task] =
  ["MAGNUS_WEBDRIVER", "MAGNUS_APP", "MAGNUS_URL", "MAGNUS_FINGERPRINT", "MAGNUS_PROVIDER", "MAGNUS_PERSON", "MAGNUS_API_KEY", "MAGNUS_TASK"].map(env);
const d = await open(wd, app, process.env.MAGNUS_SHOTS, "app");
const { step, css, xpath, button, click, type, text, seen, run, until } = d;

await pinAndSignIn(d, { url, fingerprint, provider, person });

await step("connect Claude", async () => {
  await click(xpath("//nav//a[normalize-space()='Settings']"));
  await click(xpath("//nav[@aria-label='Settings']//a[normalize-space()='Models']"));
  const card = "//div[@data-adapter='anthropic']";
  await click(button("Connect", card));
  await type(css("#key-anthropic"), apiKey);
  await click(button("Save to the vault", card));
  await seen("Connected.");
  await css("#mname");
  await click(button("Use this model"));
  await seen("New tasks will use this model.");
});

let taskHash;
await step("type a task", async () => {
  await click(xpath("//a[normalize-space()='New task']"));
  await type(css("#text"), task);
  await click(button("Start"));
  taskHash = await until("the task page", async () => { const h = await run("return location.hash"); return h.startsWith("#/task/") ? h : null; });
});

await step("approve in the inbox", async () => {
  await click(xpath("//nav//a[starts-with(normalize-space(), 'Inbox')]"));
  const card = "//article[contains(@class,'approval')]";
  await xpath(`${card}//*[contains(., 'A task is waiting')]`);
  const what = await text(xpath(`${card}//div[@aria-label='What will happen']`));
  if (!what.includes("deliver_file")) throw new Error(`the card does not name the action: ${what}`);
  await click(button("Approve once", card));
  await seen("Approved.");
  await click(button("Accept", "//article[contains(@class,'review')]"));
  await seen("Accepted.");
});

await step("see the outcome with its evidence", async () => {
  await run("location.hash = arguments[0]", taskHash);
  await xpath("//span[contains(@class,'chip') and normalize-space()='Done']");
  await xpath("//div[contains(@class,'evidence') and contains(@class,'pass')]//span[normalize-space()='holds']");
  await xpath(`//div[contains(@class,'evidence') and contains(@class,'pass')]//span[normalize-space()='confirmed by ${person}']`);
  const sandbox = await run("return document.querySelector('iframe.composed')?.getAttribute('sandbox')");
  if (sandbox === null || /allow-scripts/.test(sandbox)) throw new Error(`the outcome frame may run scripts: ${sandbox}`);
  await until("the composed outcome", () => run("const f = document.querySelector('iframe.composed'); return f && f.contentDocument && f.contentDocument.body.innerText.includes('41.7M')"));
});

await step("download the delivered file", async () => {
  const body = readFileSync(await savedPath(d, () => click(css("button[data-act=download]"))), "utf8");
  if (!body.includes("41.7M")) throw new Error(`the downloaded file holds ${JSON.stringify(body)}`);
});

await d.close();
console.log("app flow passed");
