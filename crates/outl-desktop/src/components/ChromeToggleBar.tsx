import { appState, setAppState } from "../lib/store";

/**
 * Small chrome toggle button (sidebar / shortcuts help).
 *
 * These surface keyboard chords that are otherwise invisible
 * (`Cmd/Ctrl+Shift+E` for the sidebar, `?` / `Cmd/Ctrl+/` for the
 * help overlay). They carry no business logic — clicking flips the
 * same store signal the `outl-shortcuts` dispatcher flips, so the
 * button and the keyboard stay in sync automatically.
 */
function ChromeToggle(props: {
  glyph: string;
  active: boolean;
  label: string;
  title: string;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={props.label}
      aria-pressed={props.active}
      title={props.title}
      onClick={props.onToggle}
      class={`flex h-7 w-7 items-center justify-center rounded-md text-[14px] leading-none transition-colors ${
        props.active
          ? "bg-(--color-outl-accent) text-(--color-outl-bg)"
          : "text-(--color-outl-fg-dim) hover:bg-(--color-outl-bg-elev) hover:text-(--color-outl-fg)"
      }`}
    >
      <span aria-hidden="true">{props.glyph}</span>
    </button>
  );
}

/**
 * Bottom-left chrome cluster — sidebar + shortcuts-help toggles.
 *
 * Pinned to the lower-left corner of the window (VS Code's activity-bar
 * convention) so the affordances are always in the same place, independent
 * of which page or pane is open. The sidebar toggle stays reachable here
 * even after the left pane is hidden because the cluster floats over the
 * main pane, not inside the sidebar.
 *
 * The cluster sits on an elevated, bordered surface so it reads with clear
 * contrast against the page content behind it; the active toggle inverts to
 * the accent color for an unmistakable on/off state.
 */
export function ChromeToggleBar() {
  return (
    <div class="fixed bottom-3 left-3 z-20 flex items-center gap-1 rounded-lg border border-(--color-outl-border) bg-(--color-outl-bg-elev) p-1 shadow-lg">
      <ChromeToggle
        glyph="◫"
        active={appState.sidebarOpen}
        label={appState.sidebarOpen ? "Hide sidebar" : "Show sidebar"}
        title="Toggle sidebar (⌘⇧E)"
        onToggle={() => setAppState("sidebarOpen", !appState.sidebarOpen)}
      />
      <ChromeToggle
        glyph="?"
        active={appState.helpOpen}
        label={
          appState.helpOpen
            ? "Hide keyboard shortcuts"
            : "Show keyboard shortcuts"
        }
        title="Keyboard shortcuts (?)"
        onToggle={() => setAppState("helpOpen", !appState.helpOpen)}
      />
    </div>
  );
}
