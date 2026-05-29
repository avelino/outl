/**
 * Lightweight haptic feedback helper. Falls back silently when the
 * Web Vibration API is missing (desktop, simulator). Real iOS haptic
 * support will require a Tauri plugin; until then we use the
 * Vibration API which WKWebView exposes.
 */

export type HapticStyle = "light" | "medium" | "heavy" | "success" | "warning";

const PATTERNS: Record<HapticStyle, number[]> = {
  light: [8],
  medium: [14],
  heavy: [22],
  success: [10, 30, 10],
  warning: [20, 40, 20],
};

interface VibratingNavigator extends Navigator {
  vibrate?: (pattern: number[]) => boolean;
}

export function haptic(style: HapticStyle = "light") {
  if (typeof navigator === "undefined") return;
  const v = (navigator as VibratingNavigator).vibrate;
  if (typeof v !== "function") return;
  try {
    v.call(navigator, PATTERNS[style]);
  } catch {
    // best effort
  }
}
