import { describe, expect, it } from "vitest";
import {
  EDGE_GUTTER,
  classifyOrigin,
  decideCapture,
} from "./edge-swipe";

const WIDTH = 393; // iPhone 17 Pro logical width

describe("classifyOrigin", () => {
  it("tags taps in the left gutter as 'left'", () => {
    expect(classifyOrigin(0, WIDTH)).toBe("left");
    expect(classifyOrigin(EDGE_GUTTER, WIDTH)).toBe("left");
  });

  it("tags taps in the right gutter as 'right'", () => {
    expect(classifyOrigin(WIDTH, WIDTH)).toBe("right");
    expect(classifyOrigin(WIDTH - EDGE_GUTTER, WIDTH)).toBe("right");
  });

  it("returns null for taps in the middle", () => {
    expect(classifyOrigin(EDGE_GUTTER + 1, WIDTH)).toBeNull();
    expect(classifyOrigin(WIDTH / 2, WIDTH)).toBeNull();
    expect(classifyOrigin(WIDTH - EDGE_GUTTER - 1, WIDTH)).toBeNull();
  });
});

describe("decideCapture", () => {
  it("aborts when the gesture started off-edge", () => {
    const d = decideCapture({ origin: null, dx: 50, dy: 0 });
    expect(d).toEqual({ capture: false, abort: true });
  });

  it("waits while the gesture is still small", () => {
    const d = decideCapture({ origin: "left", dx: 4, dy: 2 });
    expect(d).toEqual({ capture: false, abort: false });
  });

  it("aborts on a clearly vertical drag", () => {
    const d = decideCapture({ origin: "left", dx: 6, dy: 40 });
    expect(d).toEqual({ capture: false, abort: true });
  });

  it("captures a rightward pull from the left edge", () => {
    const d = decideCapture({ origin: "left", dx: 30, dy: 4 });
    expect(d).toEqual({ capture: true, abort: false });
  });

  it("aborts a leftward pull from the left edge", () => {
    const d = decideCapture({ origin: "left", dx: -30, dy: 4 });
    expect(d).toEqual({ capture: false, abort: true });
  });

  it("captures a leftward pull from the right edge", () => {
    const d = decideCapture({ origin: "right", dx: -30, dy: 4 });
    expect(d).toEqual({ capture: true, abort: false });
  });

  it("aborts a rightward pull from the right edge", () => {
    const d = decideCapture({ origin: "right", dx: 30, dy: 4 });
    expect(d).toEqual({ capture: false, abort: true });
  });

  it("does not capture when the drag is not horizontal-dominant", () => {
    // 15px horizontal but vertical is bigger
    const d = decideCapture({ origin: "left", dx: 15, dy: 30 });
    expect(d).toEqual({ capture: false, abort: true });
  });
});
