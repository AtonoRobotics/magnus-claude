// The owner adds one of the person's apps in the installed app and signs in
// to it: the shell opens a loopback address on this computer, the app's
// sign-in page sends its code there, and the shell hands it to the
// appliance. Then the owner lets the worker read the app's events.
//
// MAGNUS_WEBDRIVER  tauri-driver's address      MAGNUS_APP  the app binary
// MAGNUS_URL        the appliance (https)       MAGNUS_FINGERPRINT  what it printed
// MAGNUS_PROVIDER   the identity provider's name on the sign-in button
// MAGNUS_PERSON     the owner                   MAGNUS_APP_URL  the app's MCP address
// MAGNUS_SHOTS      optional: where to keep screenshots
import { env, open, pinAndSignIn } from "./appdriver.mjs";

const [wd, app, url, fingerprint, provider, person, appUrl] =
  ["MAGNUS_WEBDRIVER", "MAGNUS_APP", "MAGNUS_URL", "MAGNUS_FINGERPRINT", "MAGNUS_PROVIDER", "MAGNUS_PERSON", "MAGNUS_APP_URL"].map(env);
const d = await open(wd, app, process.env.MAGNUS_SHOTS, "apps");

await pinAndSignIn(d, { url, fingerprint, provider, person });

await d.step("add the calendar by its address", async () => {
  await d.click(d.xpath("//nav//a[normalize-space()='Apps']"));
  await d.type(d.css("#aname"), "calendar");
  await d.type(d.css("#aurl"), appUrl);
  await d.click(d.button("Add"));
});

await d.step("sign in on the app's page; its code comes back to this computer", async () => {
  const link = await d.attr(d.css("#app-open"), "data-url");
  // The person's browser: the app's page approves and redirects to the shell's loopback address.
  const r = await fetch(link);
  const page = await r.text();
  if (!r.url.startsWith("http://127.0.0.1:") || !page.includes("Signed in")) throw new Error(`the sign-in ended at ${r.url}: ${page.slice(0, 200)}`);
  await d.css('[data-app="calendar"] .chip.ok');
});

await d.step("let the worker read events; the rest stays not offered", async () => {
  await d.css('select[data-tool="list_events"]');
  await d.run(`const s = document.querySelector('select[data-tool="list_events"]'); s.value = "read"; s.dispatchEvent(new Event("change", { bubbles: true }));`);
  await d.seen("list_events: reads only.");
});

await d.close();
console.log("apps passed");
