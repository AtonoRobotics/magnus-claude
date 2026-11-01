// Themes (U7, amended): one component set, its tokens swapped by mode
// (light, dark, match computer) × material (solid, glass) + accent, from
// the Magnus Workspace design. Glass is for chrome only (sidebar, panel,
// floating cards); reading surfaces (the conversation, messages, results,
// decision cards) stay solid in every look. The tint of each glass surface
// is chosen so its text reaches 4.5:1 over any backdrop, black or white;
// `desktop/tests/theme-check.mjs` checks every look.
"use strict";

(function (root) {
  const LIGHT = {
    bg: "#F3F3F0", side: "#F3F3F0", panel: "#FAFAF8", main: "#FFFFFF", surface: "#FFFFFF", raised: "#F6F6F3",
    bubble: "#F1F1EE", seg: "#EDEDEA", "seg-on": "#FFFFFF",
    fg: "#1C1C1E", fg2: "#3F3F44", muted: "#5E5E63", faint: "#5E5E63",
    line: "#E7E6E2", hair: "#EFEEEA", "line-strong": "#C9C8C2", "btn-border": "#E2E1DC", "button-bg": "#FFFFFF",
    "row-sel": "rgba(0,0,0,0.06)", search: "rgba(118,118,128,0.12)", "group-head": "#FAFAF8", "row-hi": "#F4F6FE",
    ok: "#1A6B3A", "ok-soft": "#E3F4E9", "ok-icon": "#1E7A44", warn: "#8A4200", "warn-soft": "#FFEBD2", "warn-icon": "#9A4A00",
    sch: "#2C43AD", "sch-soft": "#E9EDFC", "approval-bg": "#FFFAF3", "approval-border": "#F2D9B8", bad: "#B42318", "bad-soft": "#FBE7E5",
    shadow: "0 1px 2px rgba(0,0,0,0.04), 0 6px 20px rgba(0,0,0,0.04)", avatar: "#D9D8D2", chev: "#8A8A8F", "pdf-bg": "#FBE7E4", "pdf-fg": "#A23A2C", "panel-edge": "#E7E6E2", "main-edge": "transparent",
    "t-read": "#5E5E63", "t-reversible": "#2C43AD", "t-irreversible": "#9A4A00", "t-financial": "#B42318",
    "k-question": "#6D28D9", "k-review": "#0F766E", "k-notice": "#8A4200", "k-proposal": "#4338CA", "k-message": "#5E5E63", "k-tool": "#7C2D12",
  };
  const DARK = {
    bg: "#161618", side: "#161618", panel: "#19191C", main: "#1C1C1F", surface: "#242428", raised: "#2A2A2F",
    bubble: "#2C2C31", seg: "#2A2A2F", "seg-on": "#3A3A40",
    fg: "#F2F2F4", fg2: "#D1D1D6", muted: "#A1A1A8", faint: "#A1A1A8",
    line: "#2E2E33", hair: "#2A2A2F", "line-strong": "#4A4A52", "btn-border": "#3A3A40", "button-bg": "#2A2A2F",
    "row-sel": "rgba(255,255,255,0.08)", search: "rgba(255,255,255,0.08)", "group-head": "#202024", "row-hi": "#20263A",
    ok: "#86DBA0", "ok-soft": "#16331F", "ok-icon": "#6CCB8A", warn: "#FFBE78", "warn-soft": "#3A2410", "warn-icon": "#FFAA55",
    sch: "#A9BAFF", "sch-soft": "#1D2547", "approval-bg": "#241B10", "approval-border": "#4A3418", bad: "#FF8A80", "bad-soft": "#3A1717",
    shadow: "0 1px 2px rgba(0,0,0,0.3)", avatar: "#3A3A40", chev: "#7A7A80", "pdf-bg": "#3A1E1A", "pdf-fg": "#FF9D8C", "panel-edge": "#2E2E33", "main-edge": "transparent",
    "t-read": "#A1A1A8", "t-reversible": "#A9BAFF", "t-irreversible": "#FFAA55", "t-financial": "#FF8A80",
    "k-question": "#C4B5FD", "k-review": "#5EEAD4", "k-notice": "#FFBE78", "k-proposal": "#A5B4FC", "k-message": "#A1A1A8", "k-tool": "#FDBA74",
  };
  // Glass: the chrome's tint (the design canvas's), its blur, the drawn
  // backdrop for systems with no window material, and `system`, the tint
  // Magnus gives a material it chooses itself (Acrylic on Windows 10;
  // Windows 11's Mica keeps its own, within that tint's range). Text on
  // glass is a step stronger than on solid.
  const GLASS = {
    light: {
      system: "rgba(248,248,246,0.86)",
      side: "rgba(248,248,246,0.62)", panel: "rgba(250,250,248,0.70)", search: "rgba(255,255,255,0.55)", seg: "rgba(0,0,0,0.06)",
      muted: "#55555A", faint: "#55555A", "panel-edge": "rgba(255,255,255,0.6)", "main-edge": "rgba(0,0,0,0.05)",
      "main-shadow": "0 10px 40px rgba(40,50,90,0.12)",
      backdrop: "radial-gradient(60% 50% at 12% 18%, #F7C8A6 0%, rgba(247,200,166,0) 70%), radial-gradient(55% 55% at 88% 12%, #AFC2F7 0%, rgba(175,194,247,0) 70%), radial-gradient(60% 60% at 72% 92%, #BFE5D2 0%, rgba(191,229,210,0) 70%), #E4E6EE",
    },
    dark: {
      system: "rgba(24,24,28,0.86)",
      side: "rgba(26,26,30,0.58)", panel: "rgba(28,28,32,0.66)", search: "rgba(255,255,255,0.10)", seg: "rgba(255,255,255,0.08)",
      "panel-edge": "rgba(255,255,255,0.08)", "main-edge": "rgba(255,255,255,0.06)",
      "main-shadow": "0 10px 40px rgba(0,0,0,0.45)",
      // The design's muted text over its 10% search field falls to 4.2:1
      // over the wallpaper's darkest color; the placeholder alone is lifted.
      "search-muted": "#B4B4BB",
      backdrop: "radial-gradient(60% 50% at 12% 18%, #4B2A5E 0%, rgba(75,42,94,0) 70%), radial-gradient(55% 55% at 88% 12%, #173F66 0%, rgba(23,63,102,0) 70%), radial-gradient(60% 60% at 72% 92%, #5A2F1C 0%, rgba(90,47,28,0) 70%), #0D0F1A",
    },
  };
  // Each accent: the fill of primary buttons with the text on it, and the
  // color of accent text (links, selected labels) on each mode's surfaces.
  const ACCENTS = [
    { id: "blue", label: "Blue", swatch: "#3557D9", light: { accent: "#3557D9", "accent-fg": "#FFFFFF", "accent-text": "#3557D9", "accent-soft": "#E9EDFC" }, dark: { accent: "#3557D9", "accent-fg": "#FFFFFF", "accent-text": "#9DB1FF", "accent-soft": "#1D2547" } },
    { id: "teal", label: "Teal", swatch: "#1F7A6D", light: { accent: "#1F7A6D", "accent-fg": "#FFFFFF", "accent-text": "#1F7A6D", "accent-soft": "#EAF6F3" }, dark: { accent: "#1F7A6D", "accent-fg": "#FFFFFF", "accent-text": "#6FD3C1", "accent-soft": "#13302B" } },
    { id: "clay", label: "Clay", swatch: "#B4532A", light: { accent: "#B4532A", "accent-fg": "#FFFFFF", "accent-text": "#B4532A", "accent-soft": "#FDF2EC" }, dark: { accent: "#B4532A", "accent-fg": "#FFFFFF", "accent-text": "#F7B394", "accent-soft": "#3A2016" } },
    { id: "graphite", label: "Graphite", swatch: "#1C1C1E", light: { accent: "#1C1C1E", "accent-fg": "#FFFFFF", "accent-text": "#1C1C1E", "accent-soft": "#EDEDEA" }, dark: { accent: "#E5E5EA", "accent-fg": "#1C1C1E", "accent-text": "#E5E5EA", "accent-soft": "#3A3A40" } },
  ];
  const MODES = [["light", "Light"], ["dark", "Dark"], ["system", "Match computer"]];
  const MATERIALS = [["solid", "Solid", "Clean, opaque surfaces"], ["glass", "Glass", "Frosted panels over your wallpaper"]];
  const MOTIONS = [["full", "Full"], ["reduced", "Reduced"], ["off", "Off"]];
  const MOTION_HINTS = {
    full: "Panels glide, cards update smoothly, and finished work gets a quiet moment.",
    reduced: "Quick fades instead of movement.",
    off: "No animation. Everything changes instantly.",
  };
  const DEFAULTS = { mode: "system", material: "solid", accent: "blue", motion: "full" };
  // Which chrome surfaces may be glass; every other surface is solid.
  const GLASS_SURFACES = ["side", "panel"];

  // The look in effect: the person's choices, as far as the system allows.
  // `os`: { dark, reduceMotion, reduceTransparency }.
  function effective(prefs, os) {
    const p = { ...DEFAULTS, ...(prefs || {}) };
    const dark = p.mode === "dark" || (p.mode === "system" && !!os.dark);
    const glass = p.material === "glass" && !os.reduceTransparency;
    const order = ["off", "reduced", "full"];
    const motion = os.reduceMotion && order.indexOf(p.motion) > 1 ? "reduced" : p.motion;
    const accent = ACCENTS.find(a => a.id === p.accent) || ACCENTS[0];
    return { mode: dark ? "dark" : "light", material: glass ? "glass" : "solid", motion, accent: accent.id };
  }

  // Every token of a look, by CSS variable name (without the dashes).
  function tokens(look) {
    const dark = look.mode === "dark";
    const t = { ...(dark ? DARK : LIGHT) };
    // Solid: the conversation fills its column. Glass: it floats, inset,
    // over the backdrop the chrome is frosted from.
    t.blur = "none"; t["main-gap"] = "0px"; t["main-radius"] = "0px"; t["main-shadow"] = "none"; t.backdrop = t.bg;
    // What glass looks like in this mode, for the preview in Appearance.
    t["glass-backdrop"] = GLASS[look.mode].backdrop; t["glass-side"] = GLASS[look.mode].side; t["glass-edge"] = GLASS[look.mode]["panel-edge"];
    if (look.material === "glass") {
      Object.assign(t, GLASS[look.mode]);
      t.blur = "blur(40px) saturate(170%)"; t["main-gap"] = "10px"; t["main-radius"] = "18px";
    }
    Object.assign(t, (ACCENTS.find(a => a.id === look.accent) || ACCENTS[0])[look.mode]);
    // The search field's placeholder and icon: muted, unless the look lifts it.
    t["search-muted"] = (look.material === "glass" && GLASS[look.mode]["search-muted"]) || t.muted;
    return t;
  }

  root.Theme = { LIGHT, DARK, GLASS, ACCENTS, MODES, MATERIALS, MOTIONS, MOTION_HINTS, DEFAULTS, GLASS_SURFACES, effective, tokens };
  if (typeof document === "undefined") return;

  // In the page: the person's choices are kept on this device (the look
  // suits the screen), applied before anything is drawn, and again when
  // the system's settings change.
  const KEY = "magnus.appearance";
  const media = q => (typeof matchMedia === "function" ? matchMedia(q) : null);
  const darkQ = media("(prefers-color-scheme: dark)"), motionQ = media("(prefers-reduced-motion: reduce)"), clearQ = media("(prefers-reduced-transparency: reduce)");
  let system = { reduceTransparency: false };
  Theme.prefs = () => { try { return { ...DEFAULTS, ...JSON.parse(localStorage.getItem(KEY) || "{}") }; } catch { return { ...DEFAULTS }; } };
  Theme.os = () => ({ dark: !!darkQ?.matches, reduceMotion: !!motionQ?.matches, reduceTransparency: !!clearQ?.matches || system.reduceTransparency });
  Theme.apply = () => {
    const look = effective(Theme.prefs(), Theme.os());
    const el = document.documentElement, t = tokens(look);
    for (const [k, v] of Object.entries(t)) el.style.setProperty(`--${k}`, v);
    el.style.colorScheme = look.mode;
    Object.assign(el.dataset, { mode: look.mode, material: look.material, motion: look.motion, accent: look.accent });
    Theme.look = look;
    // The system's own material behind the window where it has one; the
    // page's drawn backdrop where it has none.
    const glass = look.material === "glass";
    const tint = GLASS[look.mode].system.match(/[\d.]+/g).map(Number);
    if (root.Magnus?.setMaterial) root.Magnus.setMaterial(glass, look.mode === "dark", [...tint.slice(0, 3), Math.round(tint[3] * 255)]).then(m => { el.dataset.backdrop = glass && m.native ? "native" : "drawn"; });
    else el.dataset.backdrop = "drawn";
    return look;
  };
  Theme.set = change => {
    try { localStorage.setItem(KEY, JSON.stringify({ ...Theme.prefs(), ...change })); } catch { /* private window: this session only */ }
    return Theme.apply();
  };
  for (const q of [darkQ, motionQ, clearQ]) q?.addEventListener?.("change", Theme.apply);
  Theme.apply();
  // What the page cannot read itself (Windows' transparency setting).
  addEventListener("DOMContentLoaded", async () => {
    if (!root.Magnus?.systemLook) return;
    const s = await root.Magnus.systemLook();
    system = { reduceTransparency: !!s.reduce_transparency };
    Theme.apply();
  });
})(typeof window !== "undefined" ? window : globalThis);
