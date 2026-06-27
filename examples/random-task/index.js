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
  var index_default = definePlugin({
    activate(ctx) {
      ctx.commands.register("pick", async () => {
        const open = await ctx.blocks.query({ todo: "TODO" });
        if (open.length === 0) {
          ctx.ui.notify("\u{1F389} No open tasks!");
          return;
        }
        const chosen = open[Math.floor(Math.random() * open.length)];
        ctx.ui.notify(`\u{1F449} Focus on: ${chosen.text}`);
      });
    }
  });
})();
