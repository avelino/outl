import { fileURLToPath, URL } from "node:url";

import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import tailwindcss from "@tailwindcss/vite";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [solid(), tailwindcss()],

  resolve: {
    alias: {
      "@outl/shared": fileURLToPath(
        new URL("../outl-frontend-shared/src", import.meta.url),
      ),
    },
  },

  clearScreen: false,
  // Desktop uses port 1421 so it can coexist with `outl-mobile` (1420)
  // when both are running side by side during development.
  server: {
    port: 1421,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1422,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));
