import { render } from "solid-js/web";
import { describe, expect, it } from "vitest";

import type { InlineToken } from "../api/types";
import { MarkdownInline } from "./MarkdownInline";

function mount(
  tokens: InlineToken[],
  variant?: "inline",
  onLinkClick?: (href: string) => void,
) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(
    () => (
      <MarkdownInline
        tokens={tokens}
        variant={variant}
        onLinkClick={onLinkClick}
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

describe("MarkdownInline — external link rendering", () => {
  const linkTok: InlineToken[] = [
    { kind: "link", value: "site", href: "https://example.com" },
  ];

  it("fires onLinkClick with the href when a link is clicked", () => {
    const opened: string[] = [];
    const m = mount(linkTok, "inline", (href) => opened.push(href));
    const span = m.host.querySelector('[role="button"]') as HTMLElement;
    expect(span).not.toBeNull();
    expect(span.textContent).toBe("site");
    expect(span.getAttribute("tabindex")).toBe("0");
    span.click();
    expect(opened).toEqual(["https://example.com"]);
    m.dispose();
  });

  it("fires onLinkClick on Enter / Space for keyboard users", () => {
    const opened: string[] = [];
    const m = mount(linkTok, "inline", (href) => opened.push(href));
    const span = m.host.querySelector('[role="button"]') as HTMLElement;
    span.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    );
    span.dispatchEvent(
      new KeyboardEvent("keydown", { key: " ", bubbles: true }),
    );
    expect(opened).toEqual([
      "https://example.com",
      "https://example.com",
    ]);
    m.dispose();
  });

  it("renders the label as inert text (no fake button) when no onLinkClick", () => {
    const m = mount(linkTok, "inline");
    expect(m.host.textContent).toBe("site");
    // a11y: an inert link must NOT masquerade as a button or be a tab stop.
    expect(m.host.querySelector('[role="button"]')).toBeNull();
    const span = m.host.querySelector("span") as HTMLElement;
    expect(span.getAttribute("tabindex")).toBeNull();
    expect(() => span.click()).not.toThrow();
    expect(span.className).not.toContain("cursor-pointer");
    m.dispose();
  });
});
