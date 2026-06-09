import { render } from "solid-js/web";
import { describe, expect, it } from "vitest";

import type { InlineToken } from "../api/types";
import { MarkdownInline } from "./MarkdownInline";

function mount(tokens: InlineToken[], variant?: "inline") {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(
    () => (
      <MarkdownInline
        tokens={tokens}
        variant={variant}
      />
    ),
    host,
  );
  return {
    host,
    dispose: () => {
      dispose();
      host.remove();
    },
  };
}

describe("MarkdownInline — tag rendering", () => {
  /**
   * Regression: the Rust serializer stores the leading `#` in `value`
   * already (see `outl_md::inline::InlineToken::from_borrowed` and the
   * `round_trips_every_variant_into_serializable_form` test in
   * `crates/outl-md/src/inline.rs`). Both renderer variants must
   * therefore emit the `value` verbatim — adding a `#` prefix produces
   * the `##avelino` bug seen on desktop's pretty render.
   */
  it("inline variant does not double-prefix the `#`", () => {
    const tokens: InlineToken[] = [{ kind: "tag", value: "#avelino" }];
    const m = mount(tokens, "inline");
    expect(m.host.textContent).toBe("#avelino");
    expect(m.host.textContent).not.toBe("##avelino");
    m.dispose();
  });

  it("default (mobile) variant does not double-prefix the `#`", () => {
    const tokens: InlineToken[] = [{ kind: "tag", value: "#avelino" }];
    const m = mount(tokens);
    expect(m.host.textContent).toBe("#avelino");
    expect(m.host.textContent).not.toBe("##avelino");
    m.dispose();
  });

  it("renders tags inline with surrounding plain text intact", () => {
    const tokens: InlineToken[] = [
      { kind: "plain", value: "see " },
      { kind: "tag", value: "#code-review" },
      { kind: "plain", value: " for context" },
    ];
    const m = mount(tokens, "inline");
    expect(m.host.textContent).toBe("see #code-review for context");
    m.dispose();
  });
});
