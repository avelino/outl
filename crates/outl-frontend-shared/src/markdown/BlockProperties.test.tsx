/**
 * `<BlockProperties />`'s commit path — specifically that a failed
 * commit reaches the host.
 *
 * The chip repaints with the new value the moment the input closes,
 * regardless of whether the write landed. So a failure that doesn't
 * surface reads to the user as a successful edit, with the backend
 * still holding the old value.
 */

import { render } from "solid-js/web";
import { describe, expect, it, vi } from "vitest";

import { BlockProperties } from "./BlockProperties";

function mount(node: () => unknown) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(node as () => any, host);
  return {
    host,
    dispose: () => {
      dispose();
      host.remove();
    },
  };
}

/**
 * Open the first chip's editor, type `value`, commit with Enter.
 *
 * The explicit `focus()` matters: Enter's handler commits by calling
 * `blur()`, and an unfocused element doesn't emit one.
 */
function editFirstChip(host: HTMLElement, value: string) {
  (host.querySelector("button") as HTMLButtonElement).click();
  const input = host.querySelector("input") as HTMLInputElement;
  input.focus();
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  input.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
  );
}

const PROPS = [["priority", "high"]] as ReadonlyArray<
  readonly [string, string]
>;

describe("BlockProperties commit failures", () => {
  it("routes a rejected onCommit to onError", async () => {
    const onError = vi.fn();
    const m = mount(() => (
      <BlockProperties
        properties={PROPS}
        onCommit={() => Promise.reject(new Error("backend said no"))}
        onError={onError}
      />
    ));

    editFirstChip(m.host, "low");
    await vi.waitFor(() => expect(onError).toHaveBeenCalledOnce());
    expect(onError).toHaveBeenCalledWith("backend said no");
    m.dispose();
  });

  it("routes a synchronously thrown onCommit to onError too", async () => {
    // The failure mode `Promise.resolve(onCommit(...))` could not
    // catch: the call is evaluated as an argument, before the promise
    // exists, so the throw escapes past `.catch` and out of the
    // keydown handler entirely.
    const onError = vi.fn();
    const m = mount(() => (
      <BlockProperties
        properties={PROPS}
        onCommit={() => {
          throw new Error("threw before awaiting");
        }}
        onError={onError}
      />
    ));

    expect(() => editFirstChip(m.host, "low")).not.toThrow();
    await vi.waitFor(() => expect(onError).toHaveBeenCalledOnce());
    expect(onError).toHaveBeenCalledWith("threw before awaiting");
    m.dispose();
  });

  it("commits without an onError wired", async () => {
    // Both clients pass handlers that already catch internally, so
    // `onError` is optional and a successful commit must not depend
    // on it being there.
    const onCommit = vi.fn(() => Promise.resolve());
    const m = mount(() => (
      <BlockProperties properties={PROPS} onCommit={onCommit} />
    ));

    editFirstChip(m.host, "low");
    await vi.waitFor(() => expect(onCommit).toHaveBeenCalledOnce());
    expect(onCommit).toHaveBeenCalledWith("priority", "low");
    m.dispose();
  });
});
