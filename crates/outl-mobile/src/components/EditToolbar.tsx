import { JSX, Show } from "solid-js";

interface EditToolbarProps {
  visible: boolean;
  /** Pixels the iOS keyboard currently consumes at the bottom. */
  keyboardInset: number;
  onIndent: () => void;
  onOutdent: () => void;
  onMoveUp: () => void;
  onMoveDown: () => void;
  onToggleTodo: () => void;
  onDelete: () => void;
  onNewLine: () => void;
  onDone: () => void;
  onWrap: (style: "bold" | "italic" | "code") => void;
}

/**
 * Toolbar that floats above the iOS keyboard while a block is being
 * edited. Outline ops (indent / outdent / new line / todo / delete)
 * plus markdown affordances (bold / italic / code).
 *
 * Positioning relies on `keyboardInset` measured from
 * `window.visualViewport` — `env(keyboard-inset-height)` is not
 * reliably supported in WKWebView yet.
 */
export function EditToolbar(props: EditToolbarProps): JSX.Element {
  return (
    <Show when={props.visible}>
      <div
        class="fixed inset-x-0 z-50 flex items-center gap-1 overflow-x-auto border-t border-(--color-ios-divider)/40 bg-(--color-ios-tabbar) px-2 py-1.5 backdrop-blur-xl dark:border-(--color-iosd-divider)/40 dark:bg-(--color-iosd-tabbar)"
        style={{
          // The iOS form input accessory bar is suppressed natively
          // (see `gen/apple/Sources/outl-mobile/main.mm`). With it
          // gone, our toolbar sits at `bottom: 0` of the resized
          // viewport — right above the keyboard.
          bottom: "0px",
          "padding-bottom":
            props.keyboardInset > 0
              ? "6px"
              : "max(env(safe-area-inset-bottom), 6px)",
        }}
      >
        <ToolbarButton onClick={props.onOutdent} aria-label="Outdent">
          <Icon path="M3 5h12M3 12h8M3 19h12M21 9l-3 3 3 3" />
        </ToolbarButton>
        <ToolbarButton onClick={props.onIndent} aria-label="Indent">
          <Icon path="M3 5h12M3 12h8M3 19h12M15 9l3 3-3 3" />
        </ToolbarButton>
        <ToolbarButton onClick={props.onMoveUp} aria-label="Move up">
          <Icon path="M12 19V5M5 12l7-7 7 7" />
        </ToolbarButton>
        <ToolbarButton onClick={props.onMoveDown} aria-label="Move down">
          <Icon path="M12 5v14M19 12l-7 7-7-7" />
        </ToolbarButton>
        <Divider />
        <ToolbarButton onClick={() => props.onWrap("bold")} aria-label="Bold">
          <span class="text-[17px] font-bold">B</span>
        </ToolbarButton>
        <ToolbarButton
          onClick={() => props.onWrap("italic")}
          aria-label="Italic"
        >
          <span class="text-[17px] italic">I</span>
        </ToolbarButton>
        <ToolbarButton onClick={() => props.onWrap("code")} aria-label="Code">
          <span class="font-mono text-[14px]">{`</>`}</span>
        </ToolbarButton>
        <Divider />
        <ToolbarButton onClick={props.onToggleTodo} aria-label="Toggle TODO">
          <Icon path="M5 12l4 4 10-10" />
        </ToolbarButton>
        <ToolbarButton onClick={props.onNewLine} aria-label="New line">
          <Icon path="M5 12h14M12 5v14" />
        </ToolbarButton>
        <ToolbarButton
          onClick={props.onDelete}
          aria-label="Delete"
          tone="destructive"
        >
          <Icon path="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m-9 0v14a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2V6" />
        </ToolbarButton>
        <div class="ml-auto" />
        <button
          type="button"
          onClick={props.onDone}
          onPointerDown={(e) => e.preventDefault()}
          class="rounded-lg px-3 py-1.5 text-[15px] font-semibold text-(--color-ios-accent) active:opacity-60 dark:text-(--color-iosd-accent)"
        >
          Done
        </button>
      </div>
    </Show>
  );
}

function ToolbarButton(props: {
  children: JSX.Element;
  onClick: () => void;
  "aria-label": string;
  tone?: "default" | "destructive";
}) {
  return (
    <button
      type="button"
      aria-label={props["aria-label"]}
      onPointerDown={(e) => e.preventDefault()}
      onClick={props.onClick}
      class="flex h-10 w-10 items-center justify-center rounded-lg active:bg-(--color-ios-divider)/40 dark:active:bg-(--color-iosd-divider)/40"
      style={{
        color:
          props.tone === "destructive"
            ? "var(--color-ios-destructive)"
            : undefined,
      }}
    >
      {props.children}
    </button>
  );
}

function Divider() {
  return (
    <span class="mx-0.5 h-5 w-px bg-(--color-ios-divider)/60 dark:bg-(--color-iosd-divider)/60" />
  );
}

function Icon(props: { path: string }) {
  return (
    <svg
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d={props.path} />
    </svg>
  );
}
