import { createSignal, onCleanup, onMount } from "solid-js";

/**
 * Track how much vertical space the iOS keyboard is currently
 * eating. Multiple signals get used because iOS WKWebView is
 * unreliable about which event fires when:
 *
 * 1. `visualViewport.height` — works when iOS actually shrinks the
 *    viewport (most modern WKWebView versions).
 * 2. `window.innerHeight` delta vs. the initial value at first
 *    render — works when `interactive-widget=resizes-content` is set.
 * 3. Heuristic of 290px while a textarea/input is focused — a last
 *    resort so the toolbar never disappears behind the keyboard.
 */
export function useKeyboardInset() {
  const [inset, setInset] = createSignal(0);
  let initialHeight = 0;

  function recompute() {
    if (typeof window === "undefined") return;

    // We use `interactive-widget=resizes-content` in the viewport
    // meta, so iOS already shrinks the visual viewport when the
    // keyboard appears. The only thing the toolbar needs is the
    // remaining `safe-area-inset-bottom`; `bottom: 0` already sits
    // right above the keyboard.
    //
    // We still expose visualViewport-based detection because some
    // surfaces want to know "is the keyboard up at all" — but we no
    // longer apply the 290px heuristic that floated the bar in mid
    // air.
    const vv = window.visualViewport;
    let value = 0;
    if (vv) {
      value = Math.max(
        0,
        Math.round(window.innerHeight - (vv.height + vv.offsetTop)),
      );
    }
    if (value === 0 && initialHeight > 0 && window.innerHeight < initialHeight) {
      value = initialHeight - window.innerHeight;
    }
    setInset(value);
  }

  onMount(() => {
    if (typeof window === "undefined") return;
    initialHeight = window.innerHeight;

    const vv = window.visualViewport;
    if (vv) {
      vv.addEventListener("resize", recompute);
      vv.addEventListener("scroll", recompute);
    }
    window.addEventListener("resize", recompute);

    function onFocusChange() {
      requestAnimationFrame(() => requestAnimationFrame(recompute));
    }
    document.addEventListener("focusin", onFocusChange);
    document.addEventListener("focusout", onFocusChange);

    // Some iOS quirks: the resize event fires *before* the keyboard
    // animation completes, leaving `vv.height` momentarily stale.
    // Re-poll a few times after focus events to catch the settled
    // value.
    let pollTimer: number | null = null;
    function startPoll() {
      let ticks = 0;
      if (pollTimer !== null) window.clearInterval(pollTimer);
      pollTimer = window.setInterval(() => {
        recompute();
        ticks += 1;
        if (ticks > 10 && pollTimer !== null) {
          window.clearInterval(pollTimer);
          pollTimer = null;
        }
      }, 60);
    }
    document.addEventListener("focusin", startPoll);

    recompute();

    onCleanup(() => {
      const vv2 = window.visualViewport;
      if (vv2) {
        vv2.removeEventListener("resize", recompute);
        vv2.removeEventListener("scroll", recompute);
      }
      window.removeEventListener("resize", recompute);
      document.removeEventListener("focusin", onFocusChange);
      document.removeEventListener("focusout", onFocusChange);
      document.removeEventListener("focusin", startPoll);
      if (pollTimer !== null) window.clearInterval(pollTimer);
    });
  });

  return inset;
}
