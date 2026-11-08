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

  // Version 2 keeps legacy values for rollback and separates navigation's
  // colors from reading surfaces. The array order is the baseline token table.
  const PRESETS = {
    light: ["F2F4F7", "F2F4F7", "FFFFFF", "F4F6F9", "17212E", "526173", "17212E", "526173", "3459D4", "FFFFFF", "3459D4", "D6DDE7", "E7EDFD", "E7EDFD"],
    dark: ["0E1117", "131923", "171C24", "232C38", "F2F5FA", "AAB6C5", "F2F5FA", "AAB6C5", "9CB4FF", "101827", "9CB4FF", "394451", "29364E", "29364E"],
    ember: ["111013", "171418", "1B181B", "272126", "F5EEE8", "BFB0A9", "F5EEE8", "BFB0A9", "E5A176", "201611", "E5A176", "494047", "3A2924", "3A2924"],
    corporate: ["EDF1F6", "14253D", "FFFFFF", "F4F6F9", "18283D", "53657B", "F2F5FA", "B6C4D7", "245CC3", "FFFFFF", "A9C6FF", "CBD5E1", "E7EEFA", "213B60"],
  };
  const THEMES = [["light", "Light"], ["dark", "Dark"], ["ember", "Ember"], ["corporate", "Corporate"]];
  const V2_DEFAULTS = { version: 2, theme: "light", followComputer: true, material: "solid", motion: "full", navigation: "expanded", conversationWidthPx: 420 };
  const member = (v, choices, fallback) => choices.includes(v) ? v : fallback;
  function preferences(value, legacy) {
    const v = value?.version === 2 ? value : legacy ? { ...legacy, theme: "custom", followComputer: legacy.mode === "system" } : {};
    return { ...V2_DEFAULTS, version: 2,
      theme: member(v.theme, [...Object.keys(PRESETS), "custom"], "light"),
      followComputer: typeof v.followComputer === "boolean" ? v.followComputer : true,
      material: member(v.material, MATERIALS.map(x => x[0]), "solid"), motion: member(v.motion, MOTIONS.map(x => x[0]), "full"),
      navigation: member(v.navigation, ["expanded", "compact"], "expanded"),
      conversationWidthPx: Number.isFinite(v.conversationWidthPx) ? Math.max(360, Math.min(560, v.conversationWidthPx)) : 420,
      mode: member(v.mode, MODES.map(x => x[0]), "system"), accent: member(v.accent, ACCENTS.map(x => x.id), "blue"),
    };
  }
  const legacyEffective = effective, legacyTokens = tokens;
  function effectiveV2(prefs, os = {}) {
    // The pure legacy interface remains available to existing callers/tests.
    if (!prefs?.version && !prefs?.theme) return legacyEffective(prefs, os);
    const p = preferences(prefs), custom = p.theme === "custom";
    const theme = p.followComputer ? (os.dark ? "dark" : "light") : p.theme;
    const base = legacyEffective(p, os);
    const mode = custom ? base.mode : ["dark", "ember"].includes(theme) ? "dark" : "light";
    const material = p.material === "glass" && !os.reduceTransparency && os.blur !== false && !os.forcedColors ? "glass" : "solid";
    return { ...base, theme: custom ? "custom" : theme, mode, material, followComputer: p.followComputer };
  }
  function contrast(a, b) {
    const l = h => [1, 3, 5].map(i => parseInt(h.slice(i, i + 2), 16) / 255).map(v => v <= .04045 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4).reduce((s, v, i) => s + v * [.2126, .7152, .0722][i], 0);
    const x = l(a), y = l(b); return (Math.max(x, y) + .05) / (Math.min(x, y) + .05);
  }
  const tint = (hex, alpha = .92) => `rgba(${[1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16)).join(",")},${alpha})`;
  function tokensV2(look) {
    if (!look.theme) return legacyTokens(look);
    const dark = look.mode === "dark", custom = look.theme === "custom";
    const v = PRESETS[custom ? look.mode : look.theme].map(h => `#${h}`);
    const [shell, chrome, canvas, raised, text, muted, chromeText, chromeMuted, accent, accentFg, chromeFocus, border, selected, chromeSelected] = v;
    const t = { ...legacyTokens({ ...look, material: "solid" }),
      shell, chrome, canvas, bg: shell, side: chrome, panel: canvas, main: canvas, surface: canvas, raised,
      bubble: raised, seg: raised, "seg-on": selected, fg: text, fg2: text, muted, faint: muted,
      line: border, hair: border, "line-strong": muted, "btn-border": muted, "button-bg": canvas,
      "row-sel": selected, search: raised, "group-head": raised, "row-hi": selected,
      accent, "accent-fg": accentFg, "accent-text": accent, "accent-soft": selected, focus: accent,
      "chrome-text": chromeText, "chrome-muted": chromeMuted, "chrome-focus": chromeFocus, "chrome-selected": chromeSelected,
      "chrome-surface": chrome, "chrome-accent-fg": chrome, "chrome-line": chromeMuted,
      "search-muted": muted, "panel-edge": border, "main-gap": "0px", "main-radius": "0px", "main-shadow": "none",
      blur: "none", backdrop: shell,
    };
    if (custom) {
      Object.assign(t, legacyTokens({ ...look, material: "solid" }));
      t.focus = t["accent-text"];
      // Preserve the chosen values; only effective rendering is adjusted.
      t["contrast-adjusted"] = "false";
      for (const fg of ["fg", "fg2", "muted", "faint", "accent-text"]) {
        if (["main", "surface", "raised", "row-sel", "accent-soft"].some(bg => /^#[\da-f]{6}$/i.test(t[bg]) && contrast(t[fg], t[bg]) < 4.5)) {
          t[fg] = fg === "accent-text" ? accent : fg === "muted" || fg === "faint" ? muted : text;
          t["contrast-adjusted"] = "true";
        }
      }
      if (contrast(t["accent-fg"], t.accent) < 4.5) { t.accent = accent; t["accent-fg"] = accentFg; t["contrast-adjusted"] = "true"; }
      t["chrome-text"] = t.fg; t["chrome-muted"] = t.muted; t["chrome-focus"] = t["accent-text"];
      t["chrome-surface"] = t.side; t["chrome-selected"] = t["row-hi"]; t["chrome-line"] = t.muted;
    }
    const success = dark ? "#86DBA0" : "#176B3A", warning = dark ? "#FFBE78" : "#8A4200", error = dark ? "#FF8A80" : "#B42318";
    Object.assign(t, { ok: success, "ok-icon": success, "ok-soft": t.surface, warn: warning, "warn-icon": warning, "warn-soft": t.surface, bad: error, "bad-soft": t.surface, "approval-bg": t.surface, "approval-border": warning });
    if (look.theme === "ember") t.backdrop = "radial-gradient(at top left,rgba(160,75,38,.12),transparent 65%),radial-gradient(at bottom right,rgba(94,35,55,.10),transparent 65%),#111013";
    else if (look.theme === "dark") t.backdrop = `radial-gradient(at top left,rgba(80,100,130,.10),transparent 65%),${shell}`;
    else if (look.theme === "light") t.backdrop = `radial-gradient(at top left,rgba(100,130,170,.08),transparent 65%),${shell}`;
    if (look.material === "glass") { t.side = tint(t["chrome-surface"]); t.blur = "blur(20px)"; }
    return t;
  }
  root.Theme = { LIGHT, DARK, GLASS, ACCENTS, MODES, MATERIALS, MOTIONS, MOTION_HINTS, DEFAULTS, V2_DEFAULTS, PRESETS, THEMES, GLASS_SURFACES, preferences, contrast, effective: effectiveV2, tokens: tokensV2 };
  if (typeof document === "undefined") return;

  // In the page: the person's choices are kept on this device (the look
  // suits the screen), applied before anything is drawn, and again when
  // the system's settings change.
  const LEGACY_KEY = "magnus.appearance";
  const media = q => typeof matchMedia === "function" ? matchMedia(q) : null;
  const darkQ = media("(prefers-color-scheme: dark)"), motionQ = media("(prefers-reduced-motion: reduce)"), clearQ = media("(prefers-reduced-transparency: reduce)"), forcedQ = media("(forced-colors: active)");
  let system = {}, key = null, current = preferences(), revision = 0;
  const parse = k => { try { return JSON.parse(localStorage.getItem(k)); } catch { return null; } };
  Theme.prefs = () => ({ ...current });
  Theme.scope = (appliance, person) => {
    key = person ? `magnus.ui.v2.${appliance}.${person}` : null;
    const saved = key && parse(key);
    current = preferences(saved, key && !saved ? parse(LEGACY_KEY) : null);
    Theme.saved = true;
    if (key && !saved) { try { localStorage.setItem(key, JSON.stringify(current)); } catch { Theme.saved = false; } }
    return Theme.apply();
  };
  Theme.os = () => ({ dark: !!darkQ?.matches, reduceMotion: !!motionQ?.matches, reduceTransparency: !!clearQ?.matches || !!system.reduce_transparency,
    forcedColors: !!forcedQ?.matches, blur: !root.CSS?.supports || CSS.supports("backdrop-filter", "blur(20px)") || CSS.supports("-webkit-backdrop-filter", "blur(20px)") });
  Theme.apply = () => {
    const rev = ++revision, look = effectiveV2(current, Theme.os()), el = document.documentElement, t = tokensV2(look);
    for (const [k, v] of Object.entries(t)) el.style.setProperty(`--${k}`, v);
    el.style.colorScheme = look.mode;
    Object.assign(el.dataset, { mode: look.mode, theme: look.theme, material: look.material, motion: look.motion, accent: look.accent });
    Theme.look = look;
    // Sandboxed same-origin frames contain no scripts. Update their variables
    // in place: preserve data, selection and scroll without fetching again.
    for (const f of document.querySelectorAll("iframe.composed")) {
      try { const r = f.contentDocument?.documentElement; if (r) { for (const [k, v] of Object.entries(t)) r.style.setProperty(`--${k}`, v); r.style.colorScheme = look.mode; } } catch { /* frame still loading */ }
    }
    const glass = look.material === "glass", color = t["chrome-surface"];
    if (root.Magnus?.setMaterial) root.Magnus.setMaterial(glass, look.mode === "dark", [...[1, 3, 5].map(i => parseInt(color.slice(i, i + 2), 16)), 235])
      .then(m => { if (rev === revision) el.dataset.backdrop = glass && m.native ? "native" : "drawn"; }).catch(() => { if (rev === revision) el.dataset.backdrop = "drawn"; });
    else el.dataset.backdrop = "drawn";
    root.dispatchEvent(new CustomEvent("magnus-theme", { detail: { look, saved: Theme.saved } }));
    return look;
  };
  Theme.set = change => {
    const next = { ...current, ...change };
    if (change.theme && change.theme !== "custom") next.followComputer = false;
    if (change.followComputer === true) next.theme = Theme.os().dark ? "dark" : "light";
    if (change.followComputer === false && next.theme === "custom" && next.mode === "system") next.mode = Theme.look.mode;
    if (change.mode || change.accent) { next.theme = "custom"; next.followComputer = change.mode === "system"; }
    current = preferences(next);
    Theme.saved = true;
    if (key) { try { localStorage.setItem(key, JSON.stringify(current)); } catch { Theme.saved = false; } }
    return Theme.apply();
  };
  for (const q of [darkQ, motionQ, clearQ, forcedQ]) q?.addEventListener?.("change", Theme.apply);
  Theme.apply();
  addEventListener("DOMContentLoaded", async () => {
    if (root.Magnus?.systemLook) { try { system = await root.Magnus.systemLook(); Theme.apply(); } catch { /* CSS fallback remains usable */ } }
  });
})(typeof window !== "undefined" ? window : globalThis);
