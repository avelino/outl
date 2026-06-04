import { describe, expect, it } from "vitest";

import { looksLikeOutline, utf16OffsetToCharOffset } from "@outl/shared/paste";

/**
 * Smoke tests for the desktop client scaffold. They prove that:
 *
 * 1. Vitest is wired up (vite-plugin-solid, happy-dom).
 * 2. The `@outl/shared` workspace dependency resolves through the
 *    `paths` alias in `tsconfig.json` and `resolve.alias` in
 *    `vitest.config.ts`.
 *
 * Once the first real desktop component lands these can be deleted —
 * any new shared-import test elsewhere will cover the same ground.
 */
describe("outl-desktop scaffold", () => {
  it("resolves @outl/shared/paste at runtime", () => {
    expect(looksLikeOutline("- bullet")).toBe(true);
    expect(looksLikeOutline("plain text")).toBe(false);
  });

  it("uses the shared utf16 helper unchanged", () => {
    expect(utf16OffsetToCharOffset("abc", 999)).toBe(3);
  });
});
