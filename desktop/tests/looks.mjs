// The looks as the Magnus Workspace references draw them, in a real browser
// (tests/scratch/tests/looks.rs sets the scene up): each look the references
// draw, at their size, with the trip open beside its conversation, and
// Settings > Appearance.
//
// MAGNUS_URL, MAGNUS_PERSON, MAGNUS_PASSWORD  where and who
// MAGNUS_TASK      the finished trip, opened in the panel; each look's colors
//                  and sizes are checked against the references' values
// MAGNUS_CHROMIUM  Chrome to drive; MAGNUS_SHOTS  where to keep screenshots
import { createRequire } from "node:module";
import { mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const { chromium } = createRequire(import.meta.url)(join(root, "browser/node_modules/playwright-core"));
const env = k => { const v = process.env[k]; if (!v) { console.error(`${k} is not set`); process.exit(2); } return v; };
const [base, person, password, task] = ["MAGNUS_URL", "MAGNUS_PERSON", "MAGNUS_PASSWORD", "MAGNUS_TASK"].map(env);
const shots = process.env.MAGNUS_SHOTS;
if (shots) mkdirSync(shots, { recursive: true });
const T = 60_000;
// A screenshot is evidence, never a check.
const shoot = (page, name) => shots ? page.screenshot({ path: join(shots, name) }).catch(e => console.error(`screenshot not kept: ${e.message.split("\n")[0]}`)) : null;

const browser = await chromium.launch({ executablePath: env("MAGNUS_CHROMIUM"), args: ["--no-sandbox", "--disable-dev-shm-usage"] });
const failures = [];
// The references' values (Magnus Workspace, renderVals and inline styles),
// copied here apart from theme.js so a drift in either shows.
const LIGHT = { side: "#F3F3F0", panel: "#FAFAF8", main: "#FFFFFF", card: "#FFFFFF", border: "#E7E6E2", seg: "#EDEDEA", segOn: "#FFFFFF",
  text: "#1C1C1E", text3: "#5E5E63", search: "rgba(118,118,128,0.12)", groupHead: "#FAFAF8", accentText: "#3557D9", rowHi: "#F4F6FE" };
const NIGHT = { side: "#161618", panel: "#19191C", main: "#1C1C1F", card: "#242428", border: "#2E2E33", seg: "#2A2A2F", segOn: "#3A3A40",
  text: "#F2F2F4", text3: "#A1A1A8", search: "rgba(255,255,255,0.08)", groupHead: "#202024", accentText: "#9DB1FF", rowHi: "#20263A" };
const REF = {
  "light-solid": LIGHT,
  "light-glass": { ...LIGHT, side: "rgba(248,248,246,0.62)", panel: "rgba(250,250,248,0.70)", search: "rgba(255,255,255,0.55)", seg: "rgba(0,0,0,0.06)", text3: "#55555A" },
  "dark-glass": { ...NIGHT, side: "rgba(26,26,30,0.58)", panel: "rgba(28,28,32,0.66)", search: "rgba(255,255,255,0.10)", seg: "rgba(255,255,255,0.08)" },
};

// A color as [r, g, b, a], from #RRGGBB or rgb()/rgba().
function rgba(c) {
  const h = /^#([0-9a-f]{6})$/i.exec(c);
  if (h) return [0, 2, 4].map(i => parseInt(h[1].slice(i, i + 2), 16)).concat(1);
  const m = /rgba?\(([^)]+)\)/.exec(c);
  if (!m) return null;
  const v = m[1].split(/[ ,\/]+/).filter(Boolean).map(Number);
  return [v[0], v[1], v[2], v.length > 3 ? v[3] : 1];
}
const same = (a, b) => { const [x, y] = [rgba(a), rgba(b)]; return x && y && x.every((v, i) => Math.abs(v - y[i]) <= (i === 3 ? 0.01 : 1)); };
function expect(look, what, got, want, eq = same) {
  if (got == null) failures.push(`${look}: ${what} is not drawn`);
  else if (!eq(got, want)) failures.push(`${look}: ${what} is ${got}, the reference's is ${want}`);
}
const px = (a, b) => Math.abs(parseFloat(a) - parseFloat(b)) < 0.5;

const LOOKS = [
  ["light-solid", { mode: "light", material: "solid", accent: "blue", motion: "off" }],
  ["light-glass", { mode: "light", material: "glass", accent: "blue", motion: "off" }],
  ["dark-glass", { mode: "dark", material: "glass", accent: "blue", motion: "off" }],
];

async function open(look) {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1060 } });
  await page.addInitScript(l => { try { localStorage.setItem("magnus.appearance", JSON.stringify(l)); } catch { /* none */ } }, look);
  page.on("pageerror", e => failures.push(`page error: ${e}`));
  await page.goto(base);
  await page.getByLabel("Username").fill(person);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await page.getByRole("button", { name: /^Needs you/ }).waitFor({ timeout: T });
  return page;
}

for (const [name, look] of LOOKS) {
  const page = await open(look);
  // The conversation is home; the trip opens beside it.
  await page.locator(`[data-open="task:${task}"]`).first().click();
  await page.locator("aside.detail").waitFor({ timeout: T });
  await page.locator("iframe.composed").first().waitFor({ timeout: T });
  await page.waitForTimeout(800);
  await shoot(page, `app-${name}.png`);
  const got = await page.evaluate(() => {
    const cs = (sel, prop, root = document) => { const e = root.querySelector(sel); return e ? getComputedStyle(e)[prop] : null; };
    const frame = document.querySelector("aside.detail iframe.composed")?.contentDocument;
    return {
      body: cs("body", "color"),
      side: cs("nav.side", "backgroundColor"),
      panel: cs("aside.detail", "backgroundColor"),
      panelWidth: cs("aside.detail", "width"),
      main: cs("main", "backgroundColor"),
      search: cs("#find input", "backgroundColor"),
      searchHeight: cs("#find input", "height"),
      needsHeight: cs("nav.side button.needs", "height"),
      needsRadius: cs("nav.side button.needs", "borderTopLeftRadius"),
      seg: cs("aside.detail .seg.layers", "backgroundColor"),
      segOn: cs("aside.detail .seg.layers button.on", "backgroundColor"),
      tabHeight: cs("aside.detail .seg.layers button", "height"),
      groupHead: cs(".rg-head", "backgroundColor"),
      rowHi: cs(".rg-row.open", "backgroundColor"),
      h3: cs("aside.detail .layer h3", "color"),
      h3Case: cs("aside.detail .layer h3", "textTransform"),
      h3Size: cs("aside.detail .layer h3", "fontSize"),
      verdictHeight: cs("aside.detail .verdict button[data-verdict]", "height"),
      card: frame && cs("figure", "backgroundColor", frame),
      cardEdge: frame && cs("figure", "borderTopColor", frame),
      cardRadius: frame && cs("figure", "borderTopLeftRadius", frame),
      when: frame && cs(".tl .when", "color", frame),
      cardText: frame && cs(".tl .lbl", "color", frame),
      frameBg: frame && cs("body", "backgroundColor", frame),
    };
  });
  const r = REF[name];
  for (const [what, key, want] of [["the text", "body", r.text], ["the sidebar", "side", r.side], ["the task panel", "panel", r.panel],
    ["the conversation", "main", r.main], ["the search field", "search", r.search], ["the tabs' track", "seg", r.seg],
    ["the chosen tab", "segOn", r.segOn], ["a group's head", "groupHead", r.groupHead], ["the open task's row", "rowHi", r.rowHi],
    ["a result's heading", "h3", r.text3], ["a result card", "card", r.card], ["a result card's edge", "cardEdge", r.border],
    ["a timeline's day", "when", r.accentText], ["a result card's text", "cardText", r.text]]) expect(name, what, got[key], want);
  for (const [what, key, want] of [["the task panel's width", "panelWidth", "420px"], ["the search field's height", "searchHeight", "40px"],
    ["Needs you's height", "needsHeight", "44px"], ["Needs you's corners", "needsRadius", "12px"], ["a tab's height", "tabHeight", "34px"],
    ["a result heading's size", "h3Size", "12px"], ["a verdict button's height", "verdictHeight", "44px"], ["a result card's corners", "cardRadius", "16px"]])
    expect(name, what, got[key], want, px);
  expect(name, "a result heading's case", got.h3Case, "none", (a, b) => a === b);
  expect(name, "behind the result cards", got.frameBg, "rgba(0, 0, 0, 0)");
  console.log(`checked ${name}`);
  await page.close();
}

{
  const page = await open(LOOKS[0][1]);
  await page.getByRole("link", { name: "Settings" }).click();
  await page.getByRole("navigation", { name: "Settings" }).getByRole("link", { name: "Appearance" }).click();
  await page.waitForTimeout(800);
  await shoot(page, "app-appearance.png");
  const got = await page.evaluate(() => {
    const cs = (sel, prop) => { const e = document.querySelector(sel); return e ? getComputedStyle(e)[prop] : null; };
    return {
      labels: [...document.querySelectorAll(".appearance h3")].map(h => h.textContent),
      track: cs(".appearance .seg", "backgroundColor"),
      option: cs(".appearance .seg button", "height"),
      swatch: cs(".appearance .swatch", "width"),
    };
  });
  const want = ["Mode", "Material", "Accent color", "Motion"];
  if (want.join("|") !== got.labels.join("|")) failures.push(`appearance: the sections are ${got.labels.join(", ")}, the reference's are ${want.join(", ")}`);
  expect("appearance", "the choices' track", got.track, LIGHT.seg);
  expect("appearance", "a choice's height", got.option, "40px", px);
  expect("appearance", "a swatch's size", got.swatch, "44px", px);
  console.log("checked appearance");
  await page.close();
}

// A closed panel is gone: in a window narrow enough that the panel lies over
// the conversation, nothing covers the conversation while it is closed.
{
  const page = await open(LOOKS[0][1]);
  await page.setViewportSize({ width: 1000, height: 800 });
  await page.waitForTimeout(500);
  const hit = await page.evaluate(() => {
    const d = document.querySelector("#detail");
    const m = document.querySelector("main").getBoundingClientRect();
    const e = document.elementFromPoint(m.right - 40, m.top + m.height / 2);
    return { hidden: d.hidden, over: !!e?.closest("#detail"), what: e ? (e.id || e.className || e.tagName) : null };
  });
  if (!hit.hidden) failures.push("narrow: the panel is open on arrival");
  else if (hit.over) failures.push(`narrow: the closed panel covers the conversation (${hit.what})`);
  console.log("checked the closed panel");
  await page.close();
}

await browser.close();
if (failures.length) { console.error(failures.map(f => `FAIL ${f}`).join("\n")); process.exit(1); }
console.log("looks done");
