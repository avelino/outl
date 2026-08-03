import { describe, expect, it } from "vitest";

import { splitQuietHours, withQuietEnd } from "./quiet-hours";

describe("splitQuietHours", () => {
  it("takes a wrapping window apart", () => {
    expect(splitQuietHours("22:00-07:00")).toEqual(["22:00", "07:00"]);
  });

  it("takes a same-day window apart", () => {
    expect(splitQuietHours("13:00-14:00")).toEqual(["13:00", "14:00"]);
  });

  it("tolerates spaces around the separator", () => {
    expect(splitQuietHours(" 22:00 - 07:00 ")).toEqual(["22:00", "07:00"]);
  });

  it("renders blank for anything the backend would reject", () => {
    // Showing half of a malformed value in a picker reads as
    // "configured" when it isn't.
    for (const bad of ["", "22:00", "banana", "25:00-07:00", "22:00-07:70"]) {
      expect(splitQuietHours(bad)).toEqual(["", ""]);
    }
  });
});

describe("withQuietEnd", () => {
  it("replaces one end and keeps the other", () => {
    expect(withQuietEnd("22:00-07:00", 0, "23:00")).toBe("23:00-07:00");
    expect(withQuietEnd("22:00-07:00", 1, "08:00")).toBe("22:00-08:00");
  });

  it("stays empty until both ends are set", () => {
    // A half-filled window is not a window; `"22:00-"` would just be
    // an unparseable value the backend drops on the next read.
    const half = withQuietEnd("", 0, "22:00");
    expect(half).toBe("");
    expect(withQuietEnd(half, 1, "07:00")).toBe("");
  });

  it("builds the window once both ends land", () => {
    // The component holds the in-progress pair, so drive it the way
    // the two pickers do: set one, then the other, against the value
    // the component is tracking.
    let raw = "22:00-07:00";
    raw = withQuietEnd(raw, 0, "23:30");
    raw = withQuietEnd(raw, 1, "06:15");
    expect(raw).toBe("23:30-06:15");
  });

  it("clearing either end turns quiet hours off", () => {
    expect(withQuietEnd("22:00-07:00", 0, "")).toBe("");
    expect(withQuietEnd("22:00-07:00", 1, "")).toBe("");
  });

  it("ignores a value the picker could never produce", () => {
    expect(withQuietEnd("22:00-07:00", 1, "99:99")).toBe("");
  });
});
