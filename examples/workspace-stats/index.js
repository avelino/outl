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
      ctx.commands.register("stats", async () => {
        const blocks = await ctx.blocks.query({});
        const pages = await ctx.page.list();
        const todos = blocks.filter((b) => b.todo === "TODO").length;
        const dones = blocks.filter((b) => b.todo === "DONE").length;
        ctx.ui.notify(
          `\u{1F4CA} ${blocks.length} blocks \xB7 ${todos} TODO \xB7 ${dones} DONE \xB7 ${pages.length} pages`
        );
      });
    }
  });
})();
