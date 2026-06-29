"use strict";
(() => {
  // ../../plugin-sdk/src/index.ts
  function definePlugin(def) {
    if (def === null || typeof def !== "object") {
      throw new TypeError("definePlugin: expected a plugin definition object");
    }
    if (typeof def.activate !== "function") {
      throw new TypeError("definePlugin: `activate` must be a function");
    }
    if (def.deactivate !== void 0 && typeof def.deactivate !== "function") {
      throw new TypeError(
        "definePlugin: `deactivate` must be a function when provided"
      );
    }
    const host = globalThis;
    host.__outl_register?.(def);
    return def;
  }

  // src/index.ts
  function parseRows(body) {
    const rows = [];
    for (const raw of body.split("\n")) {
      const line = raw.trim();
      if (line === "") continue;
      const colon = line.lastIndexOf(":");
      if (colon === -1) continue;
      const label = line.slice(0, colon).trim();
      const value = Number(line.slice(colon + 1).trim());
      if (label === "" || !Number.isFinite(value)) continue;
      rows.push({ label, value });
    }
    return rows;
  }
  function escapeHtml(s) {
    return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  }
  var COLORS = [
    "#ff595e",
    "#ffca3a",
    "#8ac926",
    "#1982c4",
    "#6a4c93",
    "#ff6ec7"
  ];
  function renderChart(rows) {
    const max = rows.reduce((m, r) => Math.max(m, r.value), 0);
    const bars = rows.map((r, i) => {
      const pct = max > 0 ? Math.max(0, r.value / max * 100) : 0;
      const color = COLORS[i % COLORS.length];
      return `<div class="row">
        <div class="label" title="${escapeHtml(r.label)}">${escapeHtml(r.label)}</div>
        <div class="track">
          <div class="bar" style="width:${pct.toFixed(1)}%;background:${color}"></div>
        </div>
        <div class="value">${escapeHtml(String(r.value))}</div>
      </div>`;
    }).join("");
    const body = rows.length ? `<div class="chart">${bars}</div>` : `<div class="empty">No <code>label: number</code> lines to plot.</div>`;
    return `<!doctype html><html><head><meta charset="utf-8"><style>
    :root{color-scheme:light dark}
    html,body{margin:0;background:transparent;
      font:13px/1.4 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif}
    .chart{display:flex;flex-direction:column;gap:8px;padding:10px 4px}
    .row{display:grid;grid-template-columns:minmax(60px,22%) 1fr auto;
      align-items:center;gap:10px}
    .label{text-align:right;font-weight:600;white-space:nowrap;
      overflow:hidden;text-overflow:ellipsis;opacity:.85}
    .track{position:relative;height:20px;border-radius:5px;
      background:rgba(128,128,128,.18)}
    .bar{height:100%;border-radius:5px;min-width:2px;
      transition:width .35s ease;box-shadow:inset 0 -2px 4px rgba(0,0,0,.12)}
    .value{font-variant-numeric:tabular-nums;opacity:.7;white-space:nowrap}
    .empty{padding:14px;opacity:.6}
  </style></head><body>${body}<script>
    // Tell the desktop host how tall to make the iframe (optional; default ~240px).
    requestAnimationFrame(function(){
      var h=Math.ceil(document.body.getBoundingClientRect().height);
      parent.postMessage({outlHeight:h},'*');
    });
  </script></body></html>`;
  }
  var index_default = definePlugin({
    activate(ctx) {
      ctx.content.register("bars", (body) => ({
        kind: "rich",
        content: renderChart(parseRows(body))
      }));
    }
  });
})();
