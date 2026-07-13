import { type JSX, Show } from "solid-js";
import type { ToolbarAction } from "@outl/shared/toolbar";
import { useKeyboardInset } from "../lib/viewport";
import { KeyboardToolbar } from "./KeyboardToolbar";
import { SuggesterStrip } from "./SuggesterStrip";

/**
 * Bottom-anchored column that docks the ref-suggestion strip and the edit
 * toolbar above the soft keyboard. This is the web replacement for the
 * iOS-native accessory stack (`OutlSuggestOverlay` + `OutlToolbarView`),
 * rendered in the webview so **iOS and Android share one bar**.
 *
 * Positioning: the column sits at `bottom: keyboardInset`. With
 * `interactive-widget=resizes-content` (+ Android `adjustResize`) the
 * visual viewport shrinks and the inset collapses to ~0, so the column
 * rests at the viewport floor, right above the keys; without a resize the
 * inset equals the keyboard height and floats the column above it. One
 * formula covers both, so no per-platform offset math.
 *
 * The suggester strip stacks *above* the toolbar (matching the native
 * order) and self-hides when there's nothing to suggest, so the toolbar
 * simply rises to fill the gap.
 *
 * Gated by `active` (the caller passes `isAndroid && editingId`): iOS keeps
 * its native bar for now, so this only mounts on Android.
 */
export function KeyboardAccessory(props: {
  active: boolean;
  onAction: (action: ToolbarAction) => void;
}): JSX.Element {
  const inset = useKeyboardInset();
  return (
    <Show when={props.active}>
      <div
        class="pointer-events-none fixed inset-x-0 z-40 flex flex-col items-center gap-1 px-3 pb-1"
        style={{ bottom: `${inset()}px` }}
      >
        <SuggesterStrip />
        <KeyboardToolbar onAction={props.onAction} />
      </div>
    </Show>
  );
}
