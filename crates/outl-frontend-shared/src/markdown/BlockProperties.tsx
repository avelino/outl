import { For, Show, createMemo, createSignal, type JSX } from "solid-js";

import { propertyChips } from "./properties";

export interface BlockPropertiesProps {
  /** The block's `(key, value)` pairs, alpha-sorted by the backend. */
  properties?: ReadonlyArray<readonly [string, string]>;
  /**
   * Commit a new value for `key`. An empty string clears the property.
   *
   * Omit to render inert chips. Wiring it turns each chip into an
   * inline editor — the same interaction on both clients, because an
   * `<input>` is an input whether it's driven by a keyboard or by the
   * iOS one.
   */
  onCommit?: (key: string, value: string) => void | Promise<void>;
  /**
   * Surface a failed commit. Omit and a rejection is dropped, which is
   * why this exists: the chip repaints with the new value either way,
   * so a silent failure reads as a successful edit.
   */
  onError?: (message: string) => void;
  /** Theme tokens for a chip. Tailwind literals so JIT sees them. */
  chipClass?: string;
  /** Theme tokens for the editing input. */
  inputClass?: string;
}

/**
 * A block's `key:: value` properties, rendered as chips under its text
 * and editable in place.
 *
 * Both GUI clients showed nothing here, so a `remind::` written by a
 * chord or the long-press menu left the block looking untouched — the
 * rule existed, fired on schedule, and was invisible. Read-only chips
 * fixed half of that; without editing you could see `priority:: high`
 * and still have to open the `.md` to change it.
 *
 * Presentational: no store, no command. The client passes `onCommit`
 * and decides what writing means.
 */
export function BlockProperties(props: BlockPropertiesProps): JSX.Element {
  // Which key is being edited, and its in-flight text. One at a time:
  // a row of simultaneously-open inputs is noise, and committing on
  // blur means opening a second would commit the first anyway.
  const [editing, setEditing] = createSignal<string | null>(null);
  const [draft, setDraft] = createSignal("");

  // Memoised: `<Show>` and `<For>` below both read it, so a plain
  // function allocated the projection twice per render of every block.
  const chips = createMemo(() => propertyChips(props.properties));

  function open(key: string, value: string) {
    if (!props.onCommit) return;
    setDraft(value);
    setEditing(key);
  }

  function commit(key: string) {
    // Read before clearing: `setEditing` re-renders and the input is
    // gone by the time an async `onCommit` resolves.
    const value = draft();
    setEditing(null);
    // A rejected commit has to reach the host. Swallowing it left the
    // chip showing the new value while the backend still held the old
    // one, and the only trace was an unhandled rejection in a console
    // the user never opens.
    void Promise.resolve(props.onCommit?.(key, value)).catch((e) => {
      props.onError?.(e instanceof Error ? e.message : String(e));
    });
  }

  return (
    <Show when={chips().length > 0}>
      <div class="mt-0.5 flex flex-wrap items-center gap-1">
        <For each={chips()}>
          {(chip) => (
            <Show
              when={editing() === chip.key}
              fallback={
                <Show
                  when={props.onCommit}
                  fallback={
                    <span class={props.chipClass} title={`${chip.key}:: ${chip.value}`}>
                      {chip.icon ? `${chip.icon} ${chip.value}` : `${chip.key}: ${chip.value}`}
                    </span>
                  }
                >
                  <button
                    type="button"
                    class={props.chipClass}
                    title={`${chip.key}:: ${chip.value} — click to edit`}
                    onClick={(e) => {
                      // The row underneath selects / edits the block; a
                      // chip click means "edit this property", not both.
                      e.stopPropagation();
                      open(chip.key, chip.value);
                    }}
                  >
                    {chip.icon ? `${chip.icon} ${chip.value}` : `${chip.key}: ${chip.value}`}
                  </button>
                </Show>
              }
            >
              <input
                class={props.inputClass ?? props.chipClass}
                value={draft()}
                autofocus
                aria-label={`${chip.key} value`}
                placeholder={`${chip.key}…`}
                onClick={(e) => e.stopPropagation()}
                onInput={(e) => setDraft(e.currentTarget.value)}
                onBlur={() => commit(chip.key)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    // Blur would fire commit a second time.
                    e.currentTarget.blur();
                  } else if (e.key === "Escape") {
                    e.preventDefault();
                    // Drop the draft, keep the stored value.
                    setEditing(null);
                  }
                  // Otherwise let the key reach the input; the outline's
                  // vim bindings must not eat characters being typed.
                  e.stopPropagation();
                }}
              />
            </Show>
          )}
        </For>
      </div>
    </Show>
  );
}
