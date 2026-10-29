// The installed app downloads a file from the library: the shell fetches its
// bytes and saves them in the person's Downloads folder. A browser test cannot
// see this; the shell's own file fetch refused library files until 0.4.1.
//
// MAGNUS_WEBDRIVER  tauri-driver's address      MAGNUS_APP  the app binary
// MAGNUS_URL        the appliance (https)       MAGNUS_FINGERPRINT  what it printed
// MAGNUS_PROVIDER   the identity provider's name on the sign-in button
// MAGNUS_PERSON     MAGNUS_DOWNLOADS
// MAGNUS_FILE       the library file's name     MAGNUS_FILE_TEXT  its content
// MAGNUS_SHOTS      optional: where to keep screenshots
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { env, open, pinAndSignIn } from "./appdriver.mjs";

const [wd, app, url, fingerprint, provider, person, downloads, name, content] =
  ["MAGNUS_WEBDRIVER", "MAGNUS_APP", "MAGNUS_URL", "MAGNUS_FINGERPRINT", "MAGNUS_PROVIDER", "MAGNUS_PERSON", "MAGNUS_DOWNLOADS", "MAGNUS_FILE", "MAGNUS_FILE_TEXT"].map(env);
const d = await open(wd, app, process.env.MAGNUS_SHOTS, "library");

await pinAndSignIn(d, { url, fingerprint, provider, person });

await d.step("download a file from the library", async () => {
  await d.click(d.xpath("//nav//a[normalize-space()='Library']"));
  const row = `//tr[@data-lib-file][.//*[normalize-space()='${name}']]`;
  await d.click(d.xpath(`${row}//button[@data-file-download]`));
  // The shell says where it saved the file, or why it could not; a toast
  // with no words is a failure the person could not read.
  const said = await d.until("the app to say what happened", () => d.run("const t = [...document.querySelectorAll('.toast')]; return t.length ? JSON.stringify(t.map(e => e.innerText)) : null"));
  if (!said.includes("Saved ")) throw new Error(`the app did not save the file; it said ${said}`);
  await d.until(`${name} in Downloads`, () => { try { return readdirSync(downloads).includes(name) || null; } catch { return null; } });
  const body = readFileSync(join(downloads, name), "utf8");
  if (body !== content) throw new Error(`the downloaded file holds ${JSON.stringify(body)}`);
});

await d.close();
console.log("library download passed");
