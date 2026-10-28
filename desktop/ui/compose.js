// Draws an outcome the agent composed from the fixed components (U8). The
// agent chooses which components and what they say; this code decides how
// they look. The result is a document for a sandboxed frame that may not run
// scripts, so nothing the agent wrote can execute.
"use strict";

const Compose = (() => {
  const esc = s => String(s ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
  const fmt = v => typeof v === "number" ? (Math.abs(v) >= 1000 ? v.toLocaleString() : String(Math.round(v * 1000) / 1000)) : esc(v);

  function frame(c, inner) {
    const title = c.title ? `<h3>${esc(c.title)}</h3>` : "";
    const cap = c.caption ? `<p class="cap">${esc(c.caption)}</p>` : "";
    return `<figure>${title}${inner}${cap}</figure>`;
  }

  function bar(c) {
    const vals = c.values.map(Number), max = Math.max(0, ...vals), min = Math.min(0, ...vals);
    const span = max - min || 1, rowH = 26, labelW = 120, w = 560, plotW = w - labelW - 70;
    const zero = labelW + ((0 - min) / span) * plotW;
    const rows = c.labels.map((l, i) => {
      const v = vals[i], x0 = labelW + ((Math.min(0, v) - min) / span) * plotW, bw = (Math.abs(v) / span) * plotW, y = i * rowH;
      return `<text x="${labelW - 8}" y="${y + 17}" text-anchor="end" class="lbl">${esc(l)}</text>
        <rect x="${x0}" y="${y + 5}" width="${Math.max(bw, 1)}" height="${rowH - 10}" class="bar"/>
        <text x="${x0 + Math.max(bw, 1) + 6}" y="${y + 17}" class="val">${fmt(v)}${c.unit ? " " + esc(c.unit) : ""}</text>`;
    }).join("");
    const h = c.labels.length * rowH;
    return frame(c, `<svg viewBox="0 0 ${w} ${h + 4}" role="img" aria-label="${esc(c.title || "bar chart")}"><line x1="${zero}" y1="0" x2="${zero}" y2="${h}" class="axis"/>${rows}</svg>`);
  }

  function line(c) {
    const all = c.series.flatMap(s => s.values.map(Number)), max = Math.max(...all), min = Math.min(...all);
    const span = max - min || 1, w = 560, h = 220, l = 48, r = 12, t = 10, b = 30;
    const x = i => l + (c.labels.length === 1 ? 0 : (i / (c.labels.length - 1)) * (w - l - r));
    const y = v => t + (1 - (v - min) / span) * (h - t - b);
    const ticks = [min, (min + max) / 2, max].map(v => `<text x="${l - 6}" y="${y(v) + 4}" text-anchor="end" class="val">${fmt(v)}</text><line x1="${l}" x2="${w - r}" y1="${y(v)}" y2="${y(v)}" class="grid"/>`).join("");
    const step = Math.ceil(c.labels.length / 8);
    const xl = c.labels.map((lb, i) => i % step === 0 ? `<text x="${x(i)}" y="${h - 10}" text-anchor="middle" class="lbl">${esc(lb)}</text>` : "").join("");
    const paths = c.series.map((s, k) => `<polyline class="ln s${k}" points="${s.values.map((v, i) => `${x(i)},${y(Number(v))}`).join(" ")}"/>`).join("");
    const legend = c.series.length > 1 ? `<p class="legend">${c.series.map((s, k) => `<span class="key s${k}"></span>${esc(s.name)}`).join(" ")}</p>` : "";
    return frame(c, `<svg viewBox="0 0 ${w} ${h}" role="img" aria-label="${esc(c.title || "line chart")}">${ticks}${xl}${paths}</svg>${legend}${c.unit ? `<p class="cap">Values in ${esc(c.unit)}</p>` : ""}`);
  }

  function part(c, images) {
    switch (c.type) {
      case "heading": return `<h2>${esc(c.text)}</h2>`;
      case "text": return `<p>${esc(c.text)}</p>`;
      case "list": return frame(c, `<ul>${c.items.map(i => `<li>${esc(i)}</li>`).join("")}</ul>`);
      case "table": return frame(c, `<div class="tw"><table><thead><tr>${c.columns.map(h => `<th>${esc(h)}</th>`).join("")}</tr></thead><tbody>${c.rows.map(r => `<tr>${r.map(v => `<td class="${typeof v === "number" ? "num" : ""}">${fmt(v)}</td>`).join("")}</tr>`).join("")}</tbody></table></div>`);
      case "stat": return `<div class="stat"><div class="sv">${fmt(c.value)}${c.unit ? `<span class="su"> ${esc(c.unit)}</span>` : ""}</div><div class="sl">${esc(c.label)}</div>${c.caption ? `<p class="cap">${esc(c.caption)}</p>` : ""}</div>`;
      case "bar_chart": return bar(c);
      case "line_chart": return line(c);
      case "image": return images[c.file] ? frame(c, `<img src="${images[c.file]}" alt="${esc(c.caption || c.file)}">`) : frame(c, `<p class="cap">${esc(c.file)} (not an image this app can show)</p>`);
      default: return "";
    }
  }

  const CSS = `
    :root { color-scheme: light dark; --fg:#1b1d21; --muted:#5c6370; --line:#d9dce1; --bar:#2f5bd3; --bg:#ffffff; --s0:#2f5bd3; --s1:#c2410c; --s2:#0f766e; --s3:#7c3aed; --s4:#a16207; --s5:#be123c; }
    @media (prefers-color-scheme: dark) { :root { --fg:#e7e9ec; --muted:#a3a9b3; --line:#3a3f47; --bar:#7aa2ff; --bg:#16181c; --s0:#7aa2ff; --s1:#fb923c; --s2:#2dd4bf; --s3:#a78bfa; --s4:#facc15; --s5:#fb7185; } }
    html,body { margin:0; background:var(--bg); color:var(--fg); font: 15px/1.55 Inter, system-ui, sans-serif; }
    body { padding: 4px 2px 8px; }
    h2 { font-size: 18px; margin: 18px 0 6px; letter-spacing: -0.01em; }
    h3 { font-size: 13px; text-transform: uppercase; letter-spacing: .06em; color: var(--muted); margin: 0 0 8px; font-weight: 600; }
    p { margin: 6px 0 10px; max-width: 70ch; }
    figure { margin: 14px 0; }
    .cap { color: var(--muted); font-size: 13px; }
    ul { margin: 6px 0; padding-left: 20px; }
    .tw { overflow-x: auto; }
    table { border-collapse: collapse; font-size: 14px; }
    th, td { text-align: left; padding: 6px 12px 6px 0; border-bottom: 1px solid var(--line); }
    th { color: var(--muted); font-weight: 600; font-size: 12px; text-transform: uppercase; letter-spacing: .05em; }
    td.num { text-align: right; font-variant-numeric: tabular-nums; }
    .stat { display: inline-block; margin: 10px 32px 10px 0; vertical-align: top; }
    .sv { font-size: 30px; font-weight: 650; letter-spacing: -0.02em; font-variant-numeric: tabular-nums; }
    .su { font-size: 15px; color: var(--muted); font-weight: 500; }
    .sl { color: var(--muted); font-size: 13px; }
    svg { width: 100%; max-width: 560px; height: auto; display: block; }
    svg text { fill: var(--fg); font-size: 12px; font-family: Inter, system-ui, sans-serif; }
    svg .val { fill: var(--muted); font-variant-numeric: tabular-nums; }
    .bar { fill: var(--bar); }
    .axis, .grid { stroke: var(--line); stroke-width: 1; }
    .ln { fill: none; stroke-width: 2.25; }
    ${[0, 1, 2, 3, 4, 5].map(k => `.ln.s${k}{stroke:var(--s${k})} .key.s${k}{background:var(--s${k})}`).join(" ")}
    .legend { font-size: 13px; color: var(--muted); }
    .key { display: inline-block; width: 10px; height: 10px; border-radius: 2px; margin: 0 6px 0 10px; }
    img { max-width: 100%; border: 1px solid var(--line); border-radius: 4px; }
  `;

  /// A whole document for the frame's srcdoc. `images` maps a delivered
  /// file's name to a data: URL the app fetched itself.
  function documentFor(view, images, fontCss) {
    const body = (view || []).map(c => part(c, images || {})).join("\n");
    return `<!doctype html><html><head><meta charset="utf-8">
      <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data: ${location.origin}">
      <style>${fontCss || ""}${CSS}</style></head><body>${body}</body></html>`;
  }

  return { documentFor };
})();
