import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import { PluginViewOverlay } from "./PluginViewOverlay";

/**
 * Security-critical guard: a plugin `ctx.ui.render` payload runs as
 * untrusted author JS, so the iframe MUST be `sandbox="allow-scripts"`
 * WITHOUT `allow-same-origin` (the combination defeats the sandbox). These
 * tests pin that, plus the ephemeral auto-removal, so a refactor can't
 * silently widen the sandbox or leak a pinned overlay.
 *
 * The overlay renders through a `<Portal>` to `document.body`, so the
 * iframes are queried off `document`, not the mount host.
 */

let dispose: (() => void) | undefined;

/** Mount the overlay and return its imperative `push(html)` fn. */
function mount(): (html: string) => void {
  const host = document.createElement("div");
  document.body.appendChild(host);
  let push!: (html: string) => void;
  dispose = render(
    () => PluginViewOverlay({ bind: (p) => (push = p) }),
    host,
  );
  return push;
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

describe("PluginViewOverlay", () => {
  it("renders a sandboxed iframe with the pushed html, no same-origin", () => {
    const push = mount();
    push("<script>1</script>");

    const frame = document.querySelector("iframe");
    expect(frame).not.toBeNull();
    const sandbox = frame!.getAttribute("sandbox") ?? "";
    expect(sandbox).toContain("allow-scripts");
    // The whole point of the sandbox — never co-present with allow-scripts.
    expect(sandbox).not.toContain("allow-same-origin");
    // Content goes in via srcdoc (no network), not a plugin-controlled src.
    expect(frame!.getAttribute("srcdoc")).toBe("<script>1</script>");
    expect(frame!.hasAttribute("src")).toBe(false);
  });

  it("does not intercept taps (pointer-events: none)", () => {
    const push = mount();
    push("<p>hi</p>");
    const frame = document.querySelector("iframe") as HTMLIFrameElement;
    expect(frame.style.pointerEvents).toBe("none");
  });

  it("stacks multiple pushed views", () => {
    const push = mount();
    push("<p>a</p>");
    push("<p>b</p>");
    expect(document.querySelectorAll("iframe").length).toBe(2);
  });

  it("auto-removes a view after the TTL", () => {
    vi.useFakeTimers();
    try {
      const push = mount();
      push("<p>gone</p>");
      expect(document.querySelectorAll("iframe").length).toBe(1);
      vi.advanceTimersByTime(6000);
      expect(document.querySelectorAll("iframe").length).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });
});
