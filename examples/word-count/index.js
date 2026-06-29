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
  var MILESTONES = [50, 100, 250, 500];
  var index_default = definePlugin({
    activate(ctx) {
      const announced = /* @__PURE__ */ new Map();
      ctx.ops.onOp((op) => {
        if (op.kind !== "Edit" || typeof op.text !== "string") {
          return;
        }
        const words = countWords(op.text);
        const reached = highestMilestone(words);
        if (reached === null) {
          return;
        }
        const previous = announced.get(op.node) ?? 0;
        if (reached > previous) {
          announced.set(op.node, reached);
          ctx.ui.notify(`\u{1F4DD} ${reached} words in this block`);
        }
      });
    }
  });
  function countWords(text) {
    const trimmed = text.trim();
    if (trimmed.length === 0) {
      return 0;
    }
    return trimmed.split(/\s+/).length;
  }
  function highestMilestone(words) {
    let reached = null;
    for (const milestone of MILESTONES) {
      if (words >= milestone) {
        reached = milestone;
      }
    }
    return reached;
  }
})();
