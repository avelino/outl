import { describe, expect, it } from "vitest";
import { MAX_PEEK, damp } from "./rubber";

describe("damp", () => {
  it("returns zero when there is no travel", () => {
    expect(damp(0)).toBe(0);
  });

  it("never exceeds MAX_PEEK in absolute value", () => {
    for (const dx of [50, 100, 200, 500, 1000, 5000]) {
      expect(Math.abs(damp(dx))).toBeLessThanOrEqual(MAX_PEEK + 0.001);
      expect(Math.abs(damp(-dx))).toBeLessThanOrEqual(MAX_PEEK + 0.001);
    }
  });

  it("preserves the sign of the input", () => {
    expect(damp(20)).toBeGreaterThan(0);
    expect(damp(-20)).toBeLessThan(0);
  });

  it("is monotonically non-decreasing in magnitude", () => {
    let last = 0;
    for (const dx of [4, 16, 36, 64, 100, 144, 196, 400]) {
      const v = Math.abs(damp(dx));
      expect(v).toBeGreaterThanOrEqual(last - 0.001);
      last = v;
    }
  });

  it("damps small movements visibly less than the input", () => {
    // 80px finger travel should peek noticeably less than 80px.
    expect(Math.abs(damp(80))).toBeLessThan(80);
  });
});
