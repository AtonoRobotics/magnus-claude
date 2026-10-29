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

  // Places by latitude and longitude on a plain projection. The frame has no
  // network, so there are no map tiles beneath them.
  function map(c) {
    const ps = c.places.map(p => ({ ...p, lat: Number(p.lat), lon: Number(p.lon) }));
    const lats = ps.map(p => p.lat), lons = ps.map(p => p.lon);
    const mid = (Math.min(...lats) + Math.max(...lats)) / 2, k = Math.max(Math.cos(mid * Math.PI / 180), 0.2);
    const pad = d => Math.max(d * 0.15, 0.5);
    const [x0, x1] = [Math.min(...lons), Math.max(...lons)], [y0, y1] = [Math.min(...lats), Math.max(...lats)];
    const [lo0, lo1, la0, la1] = [x0 - pad(x1 - x0), x1 + pad(x1 - x0), y0 - pad(y1 - y0), y1 + pad(y1 - y0)];
    const w = 560, sx = w / ((lo1 - lo0) * k), h = Math.min(Math.max((la1 - la0) * sx, 160), 420), sy = h / (la1 - la0);
    const x = lon => (lon - lo0) * k * sx, y = lat => (la1 - lat) * sy;
    const dots = ps.map(p => `<circle cx="${x(p.lon)}" cy="${y(p.lat)}" r="5" class="pt"/><text x="${x(p.lon) + 8}" y="${y(p.lat) + 4}" class="lbl">${esc(p.name)}</text>`).join("");
    const corner = (lat, lon) => `${Math.abs(lat).toFixed(1)}°${lat < 0 ? "S" : "N"} ${Math.abs(lon).toFixed(1)}°${lon < 0 ? "W" : "E"}`;
    const notes = ps.filter(p => p.note).map(p => `<li><b>${esc(p.name)}</b>: ${esc(p.note)}</li>`).join("");
    return frame(c, `<svg viewBox="-2 -2 ${w + 4} ${h + 22}" role="img" aria-label="${esc(c.title || "map")}: ${esc(ps.map(p => p.name).join(", "))}">
      <rect x="0" y="0" width="${w}" height="${h}" class="area"/>${dots}
      <text x="0" y="${h + 16}" class="val">${corner(la0, lo0)}</text><text x="${w}" y="${h + 16}" text-anchor="end" class="val">${corner(la1, lo1)}</text></svg>${notes ? `<ul>${notes}</ul>` : ""}`);
  }

  function timeline(c) {
    return frame(c, `<ol class="tl">${c.events.map(e => `<li><span class="when">${esc(e.when)}</span><span class="what">${esc(e.label)}${e.note ? `<span class="cap"> ${esc(e.note)}</span>` : ""}</span></li>`).join("")}</ol>`);
  }

  // Options side by side, one row per aspect; the option that wins an aspect
  // is marked.
  function comparison(c) {
    const head = `<tr><th></th>${c.options.map(o => `<th>${esc(o)}</th>`).join("")}</tr>`;
    const rows = c.aspects.map(a => `<tr><th scope="row">${esc(a.name)}</th>${a.values.map((v, i) => `<td class="${typeof v === "number" ? "num" : ""}${a.best === i ? " best" : ""}">${fmt(v)}${a.best === i ? ` <span class="sr">(best)</span>` : ""}</td>`).join("")}</tr>`).join("");
    return frame(c, `<div class="tw"><table class="cmp"><thead>${head}</thead><tbody>${rows}</tbody></table></div>`);
  }

  // A delivered file shown in full, drawn from the same components: Markdown
  // as headings, paragraphs, lists and tables; CSV as a table; other text as
  // text. `limit` is the appliance's ({chars, rows}); the rest is in the file.
  const inline = s => s
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]+)\]\(([^)\s]+)[^)]*\)/g, (_, t, u) => t === u ? u : `${t} (${u})`)
    .replace(/<(https?:[^>\s]+)>/g, "$1")
    .replace(/\*\*(.+?)\*\*|__(.+?)__/g, (_, a, b) => a ?? b)
    .replace(/`([^`]+)`/g, "$1");

  function table(columns, rows, limit) {
    const t = { type: "table", columns: columns.map(c => c || " "), rows: rows.slice(0, limit.rows).map(r => columns.map((_, k) => r[k] ?? "")) };
    if (rows.length > limit.rows) t.caption = `The first ${limit.rows} of ${rows.length} rows; the rest are in the file.`;
    return t;
  }

  function fromMarkdown(md, limit) {
    const out = [], lines = md.replace(/\r\n?/g, "\n").split("\n");
    let para = [], list = null;
    const endPara = () => { if (para.length) out.push({ type: "text", text: inline(para.join("\n")) }); para = []; };
    const endList = () => { if (list) out.push(list); list = null; };
    const cells = l => l.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map(c => inline(c.trim()));
    const rule = /^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)*\|?\s*$/;
    for (let i = 0; i < lines.length; i++) {
      const l = lines[i];
      let m;
      if (/^\s*```/.test(l)) {
        endPara(); endList();
        const body = [];
        while (++i < lines.length && !/^\s*```/.test(lines[i])) body.push(lines[i]);
        if (body.join("").trim()) out.push({ type: "text", text: body.join("\n") });
      } else if (!l.trim()) { endPara(); endList(); }
      else if ((m = l.match(/^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/))) { endPara(); endList(); out.push({ type: "heading", text: inline(m[2]) || " ", level: m[1].length }); }
      else if (/^\s{0,3}([-*_])(\s*\1){2,}\s*$/.test(l)) { endPara(); endList(); }
      else if (l.includes("|") && rule.test(lines[i + 1] || "")) {
        endPara(); endList();
        const columns = cells(l), rows = [];
        for (i += 1; i + 1 < lines.length && lines[i + 1].includes("|") && lines[i + 1].trim(); i++) rows.push(cells(lines[i + 1]));
        out.push(table(columns, rows, limit));
      } else if ((m = l.match(/^\s*([-*+]|\d+[.)])\s+(.*)$/))) {
        endPara();
        const ordered = /\d/.test(m[1]);
        if (list && list.ordered !== ordered) endList();
        (list ||= { type: "list", items: [], ordered }).items.push(inline(m[2]));
      } else if (list && /^\s{2,}\S/.test(l)) list.items[list.items.length - 1] += " " + inline(l.trim());
      else { endList(); para.push((l.match(/^\s*>\s?(.*)$/) || [, l])[1].trim()); }
    }
    endPara(); endList();
    return out;
  }

  function fromCsv(text, limit) {
    const rows = [];
    let row = [], cell = "", quoted = false;
    for (let i = 0; i < text.length; i++) {
      const c = text[i];
      if (quoted) {
        if (c !== '"') cell += c;
        else if (text[i + 1] === '"') { cell += '"'; i++; }
        else quoted = false;
      } else if (c === '"') quoted = true;
      else if (c === ",") { row.push(cell); cell = ""; }
      else if (c === "\n" || c === "\r") { if (c === "\r" && text[i + 1] === "\n") i++; row.push(cell); rows.push(row); row = []; cell = ""; }
      else cell += c;
    }
    if (cell || row.length) { row.push(cell); rows.push(row); }
    const [columns, ...body] = rows.filter(r => r.some(x => x.trim()));
    return columns ? [table(columns, body, limit)] : [];
  }

  const TEXT = /^(text\/|application\/(json|xml|csv|x-ndjson|yaml))/;
  function fileParts(name, f, limit) {
    if (f?.image) return [{ type: "image", file: name }];
    if (typeof f?.text !== "string") return [{ type: "text", text: `${name} cannot be shown here; download it from Files.` }];
    const cut = f.text.length > limit.chars, text = cut ? f.text.slice(0, limit.chars) : f.text;
    const md = /\.(md|markdown)$/i.test(name) || f.media_type === "text/markdown";
    const csv = /\.csv$/i.test(name) || /csv/.test(f.media_type);
    const parts = md ? fromMarkdown(text, limit) : csv ? fromCsv(text, limit) : text.trim() ? [{ type: "text", text }] : [];
    if (cut) parts.push({ type: "text", text: "The file goes on; the rest is in the file.", muted: true });
    return parts;
  }

  function part(c, files, limit) {
    switch (c.type) {
      case "heading": return c.level > 2 ? `<h4>${esc(c.text)}</h4>` : `<h2>${esc(c.text)}</h2>`;
      case "text": return `<p${c.muted ? ` class="cap"` : ""}>${esc(c.text)}</p>`;
      case "list": return frame(c, c.ordered ? `<ol>${c.items.map(i => `<li>${esc(i)}</li>`).join("")}</ol>` : `<ul>${c.items.map(i => `<li>${esc(i)}</li>`).join("")}</ul>`);
      case "table": return frame(c, `<div class="tw"><table><thead><tr>${c.columns.map(h => `<th>${esc(h)}</th>`).join("")}</tr></thead><tbody>${c.rows.map(r => `<tr>${r.map(v => `<td class="${typeof v === "number" ? "num" : ""}">${fmt(v)}</td>`).join("")}</tr>`).join("")}</tbody></table></div>`);
      case "stat": return `<div class="stat"><div class="sv">${fmt(c.value)}${c.unit ? `<span class="su"> ${esc(c.unit)}</span>` : ""}</div><div class="sl">${esc(c.label)}</div>${c.caption ? `<p class="cap">${esc(c.caption)}</p>` : ""}</div>`;
      case "bar_chart": return bar(c);
      case "line_chart": return line(c);
      case "map": return map(c);
      case "timeline": return timeline(c);
      case "comparison": return comparison(c);
      case "code": return frame(c, `<pre class="code"${c.language ? ` data-language="${esc(c.language)}"` : ""}><code>${esc(c.code)}</code></pre>`);
      case "image": return files[c.file]?.image ? frame(c, `<img src="${files[c.file].image}" alt="${esc(c.caption || c.file)}">`) : frame(c, `<p class="cap">${esc(c.file)} (not an image this app can show)</p>`);
      case "file": return `<section class="file" data-file="${esc(c.file)}"><p class="fname">${esc(c.file)}</p>${fileParts(c.file, files[c.file], limit).map(x => part(x, files, limit)).join("\n")}</section>`;
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
    p { margin: 6px 0 10px; max-width: 70ch; white-space: pre-line; }
    h4 { font-size: 15px; margin: 14px 0 4px; }
    ul, ol { max-width: 70ch; }
    li { margin: 3px 0; }
    .file { border-top: 1px solid var(--line); margin-top: 14px; padding-top: 4px; }
    .fname { color: var(--muted); font-size: 12px; margin: 4px 0 0; }
    figure { margin: 14px 0; }
    .cap { color: var(--muted); font-size: 13px; }
    ul, ol { margin: 6px 0; padding-left: 20px; }
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
    .area { fill: none; stroke: var(--line); }
    .pt { fill: var(--bar); }
    .tl { list-style: none; padding: 0; border-left: 2px solid var(--line); margin-left: 4px; }
    .tl li { display: flex; gap: 14px; padding: 4px 0 4px 14px; position: relative; }
    .tl li::before { content: ""; position: absolute; left: -6px; top: 11px; width: 10px; height: 10px; border-radius: 50%; background: var(--bar); }
    .when { color: var(--muted); font-variant-numeric: tabular-nums; min-width: 9ch; }
    .cmp th[scope=row] { text-transform: none; letter-spacing: 0; font-size: 14px; color: var(--fg); font-weight: 500; }
    .cmp td.best { font-weight: 650; color: var(--bar); }
    .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); }
    pre.code { font: 13px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace; background: color-mix(in srgb, var(--line) 35%, transparent); padding: 10px 12px; border-radius: 4px; overflow-x: auto; white-space: pre; }
  `;

  /// A whole document for the frame's srcdoc. `files` maps a delivered
  /// file's name to what the app fetched itself: {image: a data: URL} or
  /// {text, media_type}. `limit` is how much of a file to show.
  function documentFor(view, files, fontCss, limit) {
    const body = (view || []).map(c => part(c, files || {}, limit || { chars: 160000, rows: 200 })).join("\n");
    return `<!doctype html><html><head><meta charset="utf-8">
      <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data: ${location.origin}">
      <style>${fontCss || ""}${CSS}</style></head><body>${body}</body></html>`;
  }

  return { documentFor, TEXT };
})();
