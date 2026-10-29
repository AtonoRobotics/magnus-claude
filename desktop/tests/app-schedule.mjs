// A schedule from chat, in the installed app: the person asks in Chat,
// confirms the card, and watches the Schedules view as runs arrive, then
// deletes the schedule there. The test restarts the appliance under the open
// app after the first run.
//
// MAGNUS_WEBDRIVER  tauri-driver's address      MAGNUS_APP  the app binary
// MAGNUS_URL        the appliance (https)       MAGNUS_FINGERPRINT  what it printed
// MAGNUS_PROVIDER   the identity provider's name on the sign-in button
// MAGNUS_PERSON
// MAGNUS_ASK        what the person says        MAGNUS_RUNS  completed runs to wait for
// MAGNUS_SHOTS      optional: where to keep screenshots
import { env, open, pinAndSignIn } from "./appdriver.mjs";

const [wd, app, url, fingerprint, provider, person, ask, runs] =
  ["MAGNUS_WEBDRIVER", "MAGNUS_APP", "MAGNUS_URL", "MAGNUS_FINGERPRINT", "MAGNUS_PROVIDER", "MAGNUS_PERSON", "MAGNUS_ASK", "MAGNUS_RUNS"].map(env);
const d = await open(wd, app, process.env.MAGNUS_SHOTS, "schedule");

await pinAndSignIn(d, { url, fingerprint, provider, person });

await d.step("ask in chat and confirm the schedule's card", async () => {
  await d.click(d.xpath("//nav//a[normalize-space()='Chat']"));
  await d.type(d.css("#msg"), ask);
  await d.click(d.css("#send"));
  await d.click(d.xpath("//div[@data-card]//button[normalize-space()='Confirm']"));
  await d.xpath("//div[@data-card]//span[normalize-space()='Scheduled']");
});

// The view redraws every five seconds; a run counts once it has completed.
await d.step(`the Schedules view shows ${runs} completed runs`, async () => {
  await d.click(d.xpath("//nav//a[normalize-space()='Schedules']"));
  const end = Date.now() + 18 * 60_000;
  let seen = "";
  while (Date.now() < end) {
    seen = await d.run(`const c = document.querySelector("[data-schedule]"); if (!c) return "no schedule";
      c.querySelector("details")?.setAttribute("open", "");
      const done = c.querySelectorAll("details .tlist li .chip.ok").length;
      return done + " completed · " + c.innerText.replace(/\\s+/g, " ");`);
    if (parseInt(seen) >= Number(runs)) return;
    await new Promise(r => setTimeout(r, 2000));
  }
  throw new Error(`the view shows: ${seen}`);
});

// Deleting asks first; the runs it made stay in Tasks.
await d.step("delete the schedule from the view", async () => {
  await d.click(d.xpath("//div[@data-schedule]//button[normalize-space()='Delete']"));
  const asked = await d.until("the confirmation", () => d.call("GET", d.s("/alert/text")));
  if (!asked.includes("starts no more runs")) throw new Error(`the confirmation says: ${asked}`);
  await d.call("POST", d.s("/alert/accept"), {});
  await d.seen("No schedules yet.");
});

await d.close();
console.log("schedule from chat passed");
