import { JSX, Show } from "solid-js";

export type SyncStatus = "synced" | "syncing" | "offline";

interface SyncDotProps {
  status: SyncStatus;
}

/**
 * Small status indicator next to the refresh button. Green dot when
 * the workspace is in sync, blue spinner while a sync is in flight,
 * orange dot when offline.
 */
export function SyncDot(props: SyncDotProps): JSX.Element {
  return (
    <span
      class="inline-flex h-2.5 w-2.5 items-center justify-center"
      title={
        props.status === "synced"
          ? "Synced"
          : props.status === "syncing"
            ? "Syncing…"
            : "Offline"
      }
    >
      <Show
        when={props.status === "syncing"}
        fallback={
          <span
            aria-hidden="true"
            class="h-2 w-2 rounded-full"
            style={{
              background:
                props.status === "synced"
                  ? "#34c759"
                  : "#ff9500",
            }}
          />
        }
      >
        <span
          aria-hidden="true"
          class="h-2.5 w-2.5 animate-spin rounded-full border-2 border-(--color-ios-accent) border-t-transparent"
        />
      </Show>
    </span>
  );
}
