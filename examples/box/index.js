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
  var TOP_LEFT = "\u250C";
  var TOP_RIGHT = "\u2510";
  var BOTTOM_LEFT = "\u2514";
  var BOTTOM_RIGHT = "\u2518";
  var HORIZONTAL = "\u2500";
  var VERTICAL = "\u2502";
  function boxify(body) {
    const lines = body.replace(/\n$/, "").split("\n");
    const width = lines.reduce((max, line) => Math.max(max, line.length), 0);
    const top = TOP_LEFT + HORIZONTAL.repeat(width + 2) + TOP_RIGHT;
    const bottom = BOTTOM_LEFT + HORIZONTAL.repeat(width + 2) + BOTTOM_RIGHT;
    const middle = lines.map(
      (line) => VERTICAL + " " + line.padEnd(width, " ") + " " + VERTICAL
    );
    return [top, ...middle, bottom].join("\n");
  }
  var index_default = definePlugin({
    activate(ctx) {
      ctx.content.register("box", (body) => ({
        kind: "text",
        content: boxify(body)
      }));
    }
  });
})();
