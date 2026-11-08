// The four looks, in the installed app (U7, amended): light and dark, each
// solid and glass, chosen in Settings > Appearance. In each look the chrome
// (the sidebar, the panel beside the conversation) is glass only when glass
// is chosen; the conversation and what is read in it stay solid; and every
// text on screen reaches 4.5:1 against what is under it, glass measured over
// the worst case behind it: the system's material over a black or white
// desktop, or the backdrop the page draws where there is no material. Then: the accent, the
// motion setting, and the choice kept on this device across a restart of the
// page. Runs the same on Windows, where glass is the system's Mica or Acrylic.
//
// MAGNUS_WEBDRIVER  tauri-driver's address      MAGNUS_APP  the app binary
// MAGNUS_URL        the appliance (https)       MAGNUS_FINGERPRINT  what it printed
// MAGNUS_PROVIDER   the identity provider's name on the sign-in button
// MAGNUS_PERSON     who signs in
// MAGNUS_SHOTS      optional: screenshots, one per look
import { env, open, pinAndSignIn } from "./appdriver.mjs";

const [wd, app, url, fingerprint, provider, person] =
  ["MAGNUS_WEBDRIVER", "MAGNUS_APP", "MAGNUS_URL", "MAGNUS_FINGERPRINT", "MAGNUS_PROVIDER", "MAGNUS_PERSON"].map(env);
const d = await open(wd, app, process.env.MAGNUS_SHOTS, "looks");
const { step, css, xpath, click, type, run, until } = d;

await pinAndSignIn(d, { url, fingerprint, provider, person });

const appearance = async () => {
  await click(xpath("//nav//a[normalize-space()='Settings']"));
  await click(xpath("//nav[@aria-label='Settings']//a[normalize-space()='Appearance']"));
  await xpath("//section[@data-section='appearance']//h2[normalize-space()='Appearance']");
};
const choose = async (label, value) => {
  await click(xpath(`//section[@data-section='appearance']//button[@role='radio' and @data-look='${label}' and @data-value='${value}']`));
  await until(`${label} ${value} chosen`, () => run("return document.querySelector(`[data-look='${arguments[0]}'][data-value='${arguments[1]}']`)?.getAttribute('aria-checked') === 'true'", label, value));
};

// Everything the page shows, measured as it is drawn: each surface's
// background (glass is any surface not opaque), and each text's contrast
// against the surfaces under it, laid over every backdrop that can be
// behind them wherever they let it through.
const MEASURE = `
  const rgba = c => { const p = (c.match(/[\\d.]+/g) || []).map(Number); return p.length === 3 ? [...p, 1] : p; };
  const over = (t, u) => [0, 1, 2].map(i => t[i] * t[3] + u[i] * (1 - t[3])).concat(1);
  const lum = c => { const [r, g, b] = c.slice(0, 3).map(v => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }); return 0.2126 * r + 0.7152 * g + 0.0722 * b; };
  const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
  // The layers under an element, top first, down to the first opaque one.
  const under = el => { const ls = []; for (let e = el; e && e !== document.documentElement; e = e.parentElement) { const c = rgba(getComputedStyle(e).backgroundColor); if (c[3] === 0) continue; ls.push(c); if (c[3] === 1) return ls; } return ls; };
  // Behind glass: the system's material over anything on the desktop (its
  // tint over black, grey and white), or the drawn wallpaper's colors where
  // the page draws its own backdrop.
  const behind = [[0,0,0,1],[255,255,255,1]];
  const backs = el => { const ls = under(el); const opaque = ls.length && ls[ls.length - 1][3] === 1; return (opaque ? [ls.pop()] : behind).map(b => ls.reduceRight((acc, l) => over(l, acc), b)); };
  const shown = el => { const r = el.getBoundingClientRect(); const s = getComputedStyle(el); return r.width > 0 && r.height > 0 && s.visibility !== "hidden" && s.opacity !== "0" && !el.closest(".sr, [hidden], option"); };
  const texts = root => [...root.querySelectorAll("*")].filter(el => [...el.childNodes].some(n => n.nodeType === 3 && n.textContent.trim()) && shown(el));
  const low = [];
  for (const root of document.querySelectorAll("nav.side, #detail .detail-head, main .conv-head, #thread, #composer")) for (const el of texts(root)) {
    const fg = rgba(getComputedStyle(el).color);
    for (const b of backs(el)) { const r = ratio(fg, b); if (r < 4.5) low.push(\`\${el.tagName.toLowerCase()}.\${el.className} "\${el.textContent.trim().slice(0, 30)}" \${r.toFixed(2)}:1\`); }
  }
  const alpha = q => { const el = document.querySelector(q); return el ? rgba(getComputedStyle(el).backgroundColor)[3] : null; };
  const filter = q => { const s = getComputedStyle(document.querySelector(q)); return s.backdropFilter || s.webkitBackdropFilter || "none"; };
  return JSON.stringify({ low, side: alpha("nav.side"), panel: alpha("#detail"), sideBlur: filter("nav.side"), panelBlur: filter("#detail"),
    reading: ["main", "#composer .say", "#thread .msg.me", "#detail iframe.composed, #detail .card"].map(q => [q, alpha(q)]),
    look: { ...document.documentElement.dataset } });
`;

await step("a conversation to look at, with Needs you open beside it", async () => {
  await type(css("#msg"), "Hello");
  await click(css("#send"));
  await xpath("//div[@id='thread']//div[@data-answer]");
  await click(xpath("//nav//a[contains(@class,'needs')]"));
  await xpath("//aside[@id='detail']//h2[normalize-space()='Needs you']");
});

await step("Settings > Appearance offers mode, material, accent and motion", async () => {
  await appearance();
  const offered = JSON.parse(await run(`return JSON.stringify([...document.querySelectorAll("[data-look]")].map(b => b.dataset.look + ":" + (b.getAttribute("title") || b.innerText.trim())))`));
  for (const want of ["mode:Light", "mode:Dark", "mode:Match computer", "material:Solid", "material:Glass", "accent:Blue", "accent:Teal", "accent:Clay", "accent:Graphite", "motion:Full", "motion:Reduced", "motion:Off"]) {
    if (!offered.includes(want)) throw new Error(`Appearance does not offer ${want}: ${offered.join(", ")}`);
  }
  await d.seen("Magnus also follows your computer's settings for reduced motion and reduced transparency.");
});

for (const [theme, material] of ["light", "dark", "ember", "corporate"].flatMap(t => ["solid", "glass"].map(m => [t,m]))) {
  const mode = ["dark", "ember"].includes(theme) ? "dark" : "light";
  await step(`${mode} ${material}`, async () => {
    await appearance();
    await choose("theme", theme);
    await choose("material", material);
    await click(xpath("//nav//a[contains(@title,'Hello')]"));
    await click(xpath("//nav//a[contains(@class,'needs')]"));
    await xpath("//aside[@id='detail']//h2[normalize-space()='Needs you']");
    // Let the panel settle before measuring what is under each text.
    await until("the panel to settle", () => run("return document.getElementById('detail').getAnimations().length === 0"));
    const m = JSON.parse(await run(MEASURE));
    if (m.look.theme !== theme || m.look.mode !== mode || m.look.material !== material) throw new Error(`the page is ${m.look.mode} ${m.look.material}`);
    const glass = material === "glass";
    for (const [what, a, blur] of [["sidebar", m.side, m.sideBlur], ["panel", m.panel, m.panelBlur]]) {
      if (glass && what === "sidebar" && !(a >= .9 && a < 1 && blur.includes("blur"))) throw new Error(`the ${what} is not glass: alpha ${a}, ${blur}`);
      if ((!glass || what === "panel") && a !== 1) throw new Error(`the ${what} is not solid: alpha ${a}`);
    }
    for (const [what, a] of m.reading) if (a !== null && a !== 1) throw new Error(`the reading surface ${what} is not solid: alpha ${a}`);
    if (m.low.length) throw new Error(`text below 4.5:1: ${m.low.join("; ")}`);
  });
}

await step("the accent colors what acts", async () => {
  await appearance();
  await choose("mode", "dark");
  await choose("accent", "teal");
  await click(xpath("//nav//a[contains(@title,'Hello')]"));
  const send = await until("Send in teal", () => run("const b = getComputedStyle(document.getElementById('send')).backgroundColor; return b === 'rgb(31, 122, 109)' && b"));
  if (!send) throw new Error("Send is not teal");
});

await step("motion: full glides the panel in, off moves nothing", async () => {
  // The panel is closed (Settings has the page to itself); it opens now.
  await click(xpath("//nav//a[contains(@class,'needs')]"));
  const moving = await run("return document.getElementById('detail').getAnimations().length");
  if (!moving) throw new Error("the panel did not glide in with motion on Full");
  await appearance();
  await choose("motion", "off");
  await click(xpath("//nav//a[contains(@title,'Hello')]"));
  await click(xpath("//nav//a[contains(@class,'needs')]"));
  await xpath("//aside[@id='detail']//h2[normalize-space()='Needs you']");
  const still = await run("return document.getAnimations().length");
  if (still) throw new Error(`${still} animations ran with motion Off`);
});

await step("the look is kept on this device", async () => {
  await run("location.reload()");
  await xpath("//nav//button[starts-with(normalize-space(),'Needs you')]");
  const look = await run("return { ...document.documentElement.dataset }");
  if (look.mode !== "dark" || look.material !== "glass" || look.accent !== "teal" || look.motion !== "off") throw new Error(`after a restart the look is ${JSON.stringify(look)}`);
});

await d.close();
console.log("looks passed");
