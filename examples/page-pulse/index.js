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
      ctx.commands.register("pulse", async () => {
        const all = await ctx.blocks.query({});
        const open = all.filter((b) => b.todo === "TODO").length;
        const done = all.filter((b) => b.todo === "DONE").length;
        ctx.ui.notify(`\u{1F493} ${all.length} blocks \xB7 ${open} open \xB7 ${done} done`);
      });
    }
  });
})();
