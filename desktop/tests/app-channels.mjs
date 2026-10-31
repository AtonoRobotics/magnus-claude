// The owner chooses the release channel their apps update from, in Settings
// > Updates (DECISIONS.md, "Release channels"): a pull request's build is
// offered once they choose its channel, and they release it to testers;
// choosing stable again drops it. Nothing is offered before they choose.
//
// MAGNUS_WEBDRIVER  tauri-driver's address      MAGNUS_APP  the app (an AppImage)
// MAGNUS_URL        the appliance (https)       MAGNUS_FINGERPRINT  what it printed
// MAGNUS_PROVIDER   the identity provider's name
// MAGNUS_PERSON     the owner                   MAGNUS_NEW  the build pull request 7's channel has
// MAGNUS_SHOTS      optional: where to keep screenshots
import { env, open, pinAndSignIn } from "./appdriver.mjs";

const [wd, app, url, fingerprint, provider, person, fresh] =
  ["MAGNUS_WEBDRIVER", "MAGNUS_APP", "MAGNUS_URL", "MAGNUS_FINGERPRINT", "MAGNUS_PROVIDER", "MAGNUS_PERSON", "MAGNUS_NEW"].map(env);
const d = await open(wd, app, process.env.MAGNUS_SHOTS, "channels");
const state = () => d.run("return window.__TAURI__.core.invoke('update_state')");
const chosen = () => d.run("return window.__TAURI__.core.invoke('api', { method: 'GET', path: '/v1/updates/channels' }).then(r => r.body.chosen)");
const choose = async (channel, said) => {
  await d.click(d.xpath(`//select[@id='channel']/option[@value='${channel}']`));
  await d.seen(said);
};

await pinAndSignIn(d, { url, fingerprint, provider, person });

await d.step("Updates shows the channels, none chosen yet", async () => {
  await d.click(d.xpath("//nav//a[normalize-space()='Settings']"));
  await d.click(d.xpath("//nav[@aria-label='Settings']//a[normalize-space()='Updates']"));
  await d.xpath("//h2[normalize-space()='Updates']");
  await d.xpath(`//select[@id='channel']/option[@value='pr-7'][starts-with(normalize-space(), 'Review: pull request #7, Sources in every answer (${fresh})')]`);
  const [st, ch] = [await state(), await chosen()];
  if (ch || st.ready) throw new Error(`before any choice: ${ch} ${JSON.stringify(st)}`);
});

await d.step("choosing pull request 7 offers its build", async () => {
  await choose("pr-7", "Your apps now follow Review.");
  await d.seen(`Magnus ${fresh} is downloaded and checked.`);
  const [st, ch] = [await state(), await chosen()];
  if (ch !== "pr-7" || st.ready?.version !== fresh) throw new Error(`after choosing pull request 7: ${ch} ${JSON.stringify(st)}`);
});

await d.step("the owner releases that build to testers", async () => {
  await d.xpath("//div[@data-testers]//span[contains(normalize-space(), 'have no test release yet')]");
  await d.click(d.button(`Release ${fresh} to testers`));
  const asked = await d.until("the confirmation", () => d.call("GET", d.s("/alert/text")));
  if (!asked.includes(`Release ${fresh} to testers?`)) throw new Error(`the confirmation says: ${asked}`);
  await d.call("POST", d.s("/alert/accept"), {});
  await d.seen(`Testers get ${fresh}.`);
  await d.xpath(`//div[@data-testers]//span[@class='mono'][normalize-space()='${fresh}']`);
});

await d.step("choosing stable again drops it", async () => {
  await choose("stable", "Your apps now follow Stable.");
  await d.seen("Up to date.");
  const [st, ch] = [await state(), await chosen()];
  if (ch !== "stable" || st.ready) throw new Error(`after choosing stable: ${ch} ${JSON.stringify(st)}`);
});

await d.close();
console.log("channels chosen");
