import { fileURLToPath, URL } from "node:url";

import { defineConfig } from "vitest/config";
import solid from "vite-plugin-solid";

export default defineConfig({
  plugins: [solid()],
  resolve: {
    alias: {
      "@outl/shared": fileURLToPath(
        new URL("../outl-frontend-shared/src", import.meta.url),
      ),
    },
  },
  test: {
    environment: "happy-dom",
    include: ["src/**/*.test.{ts,tsx}"],
    globals: false,
  },
});
