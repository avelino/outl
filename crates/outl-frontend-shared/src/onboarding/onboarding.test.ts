import { describe, expect, it } from "vitest";

import {
  FINISH_CTA,
  STORAGE_STEP,
  SYNC_STEP,
} from "./index";

describe("onboarding copy", () => {
  it("storage step has a title and an honest, lock-in-free body", () => {
    expect(STORAGE_STEP.title.length).toBeGreaterThan(0);
    expect(STORAGE_STEP.body.toLowerCase()).toContain("markdown");
    // The promise we must not break: no account.
    expect(STORAGE_STEP.body.toLowerCase()).toContain("no account");
  });

  it("sync step frames pairing as optional and account-free", () => {
    expect(SYNC_STEP.title.length).toBeGreaterThan(0);
    expect(SYNC_STEP.body.toLowerCase()).toContain("peer-to-peer");
    expect(SYNC_STEP.body.toLowerCase()).toContain("no account");
    // Must stay skippable — a single device is a first-class setup.
    expect(SYNC_STEP.skipCta.length).toBeGreaterThan(0);
    expect(SYNC_STEP.pairCta.length).toBeGreaterThan(0);
  });

  it("sync step reassures that one device is fine", () => {
    const joined = SYNC_STEP.bullets.join(" ").toLowerCase();
    expect(SYNC_STEP.bullets.length).toBeGreaterThan(0);
    expect(joined).toContain("one device");
  });

  it("exposes a finish CTA", () => {
    expect(FINISH_CTA.length).toBeGreaterThan(0);
  });
});
