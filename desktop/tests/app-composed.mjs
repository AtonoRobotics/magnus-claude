// Composed answers in the installed app (U8): a data question's text streams
// first and its chart and table are drawn after it; a plain question gets
// text only; and what the agent composed can make no network request, run
// nothing, and hold no control (approvals stay on their cards).
//
// MAGNUS_WEBDRIVER  tauri-driver's address      MAGNUS_APP  the app binary
// MAGNUS_URL        the appliance (https)       MAGNUS_FINGERPRINT  what it printed
// MAGNUS_PROVIDER   the identity provider's name on the sign-in button
// MAGNUS_PERSON     who signs in                MAGNUS_SHOTS  optional: screenshots
// MAGNUS_DATA_QUESTION, MAGNUS_DATA_ANSWER     the data question and its text
// MAGNUS_HOSTILE_QUESTION                      a question whose answer shows hostile components
// MAGNUS_BEACON     an address that records every request made to it
import { env, open, pinAndSignIn } from "./appdriver.mjs";

const [wd, app, url, fingerprint, provider, person, question, answer, hostile, beacon] =
  ["MAGNUS_WEBDRIVER", "MAGNUS_APP", "MAGNUS_URL", "MAGNUS_FINGERPRINT", "MAGNUS_PROVIDER", "MAGNUS_PERSON",
    "MAGNUS_DATA_QUESTION", "MAGNUS_DATA_ANSWER", "MAGNUS_HOSTILE_QUESTION", "MAGNUS_BEACON"].map(env);
const d = await open(wd, app, process.env.MAGNUS_SHOTS, "composed");

const answerTo = text => `//div[@data-entry][@data-kind='answer'][.//div[normalize-space()='${text}']]`;
const count = xp => d.run(`return document.evaluate(arguments[0], document, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null).snapshotLength`, xp);
const send = async text => {
  await d.type(d.css("#msg"), text);
  await d.click(d.css("#send"));
};
// What the frame below an answer holds, read by the app's own page.
const inFrame = (text, js, ...args) => d.run(`
  const e = document.evaluate(arguments[0], document, null, XPathResult.FIRST_ORDERED_NODE_TYPE, null).singleNodeValue;
  const f = e && e.querySelector("iframe[data-shown]");
  const doc = f && f.contentDocument;
  if (!doc || !doc.body || !doc.body.children.length) return null;
  ${js}`, answerTo(text), ...args);

await pinAndSignIn(d, { url, fingerprint, provider, person });

await d.step("a data question: the text streams first, then a chart and a table", async () => {
  await d.click(d.xpath("//nav//a[normalize-space()='Chat']"));
  await send(question);
  // While the model is still writing, the text shows and nothing else.
  await d.until("the answer's text streaming", () => d.run(
    "const l = document.querySelector('[data-live]'); return l && l.textContent.includes(arguments[0]) && !document.querySelector('iframe[data-shown]') || null", answer));
  const drawn = await d.until("the chart and table below the answer", () => inFrame(answer,
    "return JSON.stringify({ chart: doc.querySelectorAll('svg rect.bar').length, rows: doc.querySelectorAll('table tbody tr').length })"));
  const { chart, rows } = JSON.parse(drawn);
  if (chart !== 3 || rows !== 3) throw new Error(`the frame holds ${drawn}`);
  const order = await d.run(`const e = document.evaluate(arguments[0], document, null, XPathResult.FIRST_ORDERED_NODE_TYPE, null).singleNodeValue;
    return e.querySelector(".pre").compareDocumentPosition(e.querySelector("iframe[data-shown]")) & Node.DOCUMENT_POSITION_FOLLOWING ? "after" : "before"`, answerTo(answer));
  if (order !== "after") throw new Error("the components are drawn before the text");
});

await d.step("a plain question: text only", async () => {
  await send("Hello, who are you?");
  await d.xpath(answerTo("Answer to: Hello, who are you?"));
  if (await count(`${answerTo("Answer to: Hello, who are you?")}//iframe`)) throw new Error("a plain answer drew components");
});

await d.step("composed content makes no request, runs nothing and holds no control", async () => {
  await send(hostile);
  const said = "Here is what you asked for.";
  await d.until("the hostile components drawn", () => inFrame(said, "return doc.querySelectorAll('table, pre, svg').length >= 3 || null"));
  const sandbox = await inFrame(said, "return f.getAttribute('sandbox')");
  if (/allow-scripts|allow-forms|allow-popups|allow-top-navigation/.test(sandbox)) throw new Error(`the frame may ${sandbox}`);
  const controls = await inFrame(said, "return doc.querySelectorAll('button, a, form, input, select, textarea, iframe, img, link, script, object, embed').length");
  if (controls !== 0) throw new Error(`the composed content holds ${controls} elements that act or load`);
  // As if drawing had let markup through: the frame itself must refuse to load or run it.
  await inFrame(said, `
    const add = (tag, attrs, text) => { const el = doc.createElement(tag); for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v); if (text) el.textContent = text; doc.body.append(el); };
    add("img", { src: arguments[1] + "/injected-img" });
    add("link", { rel: "stylesheet", href: arguments[1] + "/injected-css" });
    add("iframe", { src: arguments[1] + "/injected-frame" });
    add("script", { src: arguments[1] + "/injected-script" });
    add("script", {}, "document.title = 'ran'; new Image().src = '" + arguments[1] + "/injected-inline';");
    add("div", { style: "background: url(" + arguments[1] + "/injected-style)" }, "x");
    add("img", { src: "x", onerror: "document.title = 'ran'" });
    return true;`, beacon);
  await new Promise(r => setTimeout(r, 3000));
  const ran = await inFrame(said, "return doc.title");
  if (ran === "ran") throw new Error("code inside the composed content ran");
});
