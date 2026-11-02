// Every message action in the installed app: copy and delete, edit and
// resend a message, retry an answer, save to the library, export as a file,
// make a task, share with a colleague, and branch into a new chat.
//
// MAGNUS_WEBDRIVER  tauri-driver's address      MAGNUS_APP  the app binary
// MAGNUS_URL        the appliance (https)       MAGNUS_FINGERPRINT  what it printed
// MAGNUS_PROVIDER   the identity provider's name on the sign-in button
// MAGNUS_PERSON     who signs in                MAGNUS_OTHER  a colleague (id)
// MAGNUS_OTHER_NAME the colleague's name        MAGNUS_SHOTS  optional: screenshots
// MAGNUS_DISPLAY    the app's X display (Linux), to read the clipboard; MAGNUS_XAUTHORITY opens it
import { execFileSync } from "node:child_process";
import { env, open, pinAndSignIn, savedPath } from "./appdriver.mjs";

// The system clipboard, read the way a person's other programs would.
const clipboard = () => process.platform === "win32"
  ? execFileSync("powershell", ["-NoProfile", "-Command", "Get-Clipboard"], { encoding: "utf8" }).trimEnd()
  : execFileSync("xclip", ["-o", "-selection", "clipboard"], { encoding: "utf8", env: { ...process.env, DISPLAY: env("MAGNUS_DISPLAY"), XAUTHORITY: process.env.MAGNUS_XAUTHORITY || "" } });

const [wd, app, url, fingerprint, provider, person, otherName] =
  ["MAGNUS_WEBDRIVER", "MAGNUS_APP", "MAGNUS_URL", "MAGNUS_FINGERPRINT", "MAGNUS_PROVIDER", "MAGNUS_PERSON", "MAGNUS_OTHER_NAME"].map(env);
const d = await open(wd, app, process.env.MAGNUS_SHOTS, "chat-actions");

// The entry whose text is `text`, of `kind` (message or answer).
const entry = (kind, text) => `//div[@data-entry][@data-kind='${kind}'][.//div[normalize-space()='${text}']]`;
const act = (kind, text, label) => d.click(d.xpath(`${entry(kind, text)}//div[@role='toolbar']//button[normalize-space()='${label}']`));
const count = xp => d.run(`return document.evaluate(arguments[0], document, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null).snapshotLength`, xp);
const gone = async (what, xp) => d.until(what, async () => (await count(xp)) === 0 || null);
const accept = async words => {
  const asked = await d.until("the confirmation", () => d.call("GET", d.s("/alert/text")));
  if (!asked.includes(words)) throw new Error(`the confirmation says: ${asked}`);
  await d.call("POST", d.s("/alert/accept"), {});
};
const ask = async text => {
  await d.type(d.css("#msg"), text);
  await d.click(d.css("#send"));
  await d.xpath(entry("answer", `Answer to: ${text}`));
};

await pinAndSignIn(d, { url, fingerprint, provider, person });

await d.step("two questions, two answers", async () => {
  // The conversation is home: signing in opens a new one.
  await ask("First question");
  await ask("Second question");
});

await d.step("copy an answer", async () => {
  await act("answer", "Answer to: First question", "Copy");
  await d.seen("Copied.");
  const got = clipboard();
  if (got !== "Answer to: First question") throw new Error(`the clipboard holds ${got}`);
});

await d.step("delete an answer", async () => {
  await act("answer", "Answer to: First question", "Delete");
  await accept("waits in the trash");
  await gone("the deleted answer", entry("answer", "Answer to: First question"));
});

await d.step("edit and resend a message", async () => {
  await act("message", "Second question", "Edit");
  await d.css(".msg.editing textarea");
  await d.run(`const t = document.querySelector('.msg.editing textarea'); t.value = "Second, edited";`);
  await d.click(d.xpath("//div[contains(@class,'editing')]//button[normalize-space()='Send']"));
  await d.xpath(entry("answer", "Answer to: Second, edited"));
  await gone("the replaced message", entry("message", "Second question"));
});

await d.step("retry an answer", async () => {
  const before = await d.attr(d.xpath(entry("answer", "Answer to: Second, edited")), "data-entry");
  await act("answer", "Answer to: Second, edited", "Retry");
  await d.until("a new answer", async () => (await d.attr(d.xpath(entry("answer", "Answer to: Second, edited")), "data-entry")) !== before || null);
  if (await count(entry("message", "Second, edited")) !== 1) throw new Error("the retried message shows twice");
});

await d.step("save to Files and export as a file", async () => {
  await act("answer", "Answer to: Second, edited", "Save to Files");
  await d.seen("Saved to Files.");
  await savedPath(d, () => act("answer", "Answer to: Second, edited", "Export"));
});

await d.step("make a task", async () => {
  await act("message", "Second, edited", "Make a task");
  await d.seen("Started a task.");
  await d.xpath("//div[@data-kind='tasks']");
});

await d.step("share with a colleague", async () => {
  await act("answer", "Answer to: Second, edited", "Share");
  await d.css("[data-share-to]");
  await d.run(`const s = document.querySelector("[data-share-to]"); s.value = [...s.options].find(o => o.text === arguments[0]).value;`, otherName);
  await d.click(d.xpath("//div[contains(@class,'share-to')]//button[normalize-space()='Share']"));
  await d.seen(`Shared with ${otherName}.`);
});

await d.step("branch into a new chat", async () => {
  const from = await d.run("return location.hash");
  await act("message", "First question", "Branch");
  await d.until("the new chat", async () => (await d.run("return location.hash")) !== from || null);
  await d.xpath(entry("message", "First question"));
  await gone("what came after the branch point", entry("message", "Second, edited"));
});

await d.close();
console.log("chat actions passed");
