import { render } from "solid-js/web";
import { describe, expect, it } from "vitest";

import { QuoteWrap, isBlockQuoted } from "./QuoteWrap";

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

describe("QuoteWrap", () => {
  it("applies only baseClass when not quoted", () => {
    const m = mount(() => (
      <QuoteWrap
        quoted={false}
        baseClass="flex base"
        chromeClass="border-l-2 chrome"
      >
        <span>child</span>
      </QuoteWrap>
    ));
    const wrap = m.host.firstChild as HTMLElement;
    expect(wrap.className).toBe("flex base");
    expect(wrap.textContent).toBe("child");
    m.dispose();
  });

  it("concatenates baseClass + chromeClass when quoted", () => {
    const m = mount(() => (
      <QuoteWrap
        quoted={true}
        baseClass="flex base"
        chromeClass="border-l-2 chrome"
      >
        <span>child</span>
      </QuoteWrap>
    ));
    const wrap = m.host.firstChild as HTMLElement;
    expect(wrap.className).toBe("flex base border-l-2 chrome");
    expect(wrap.textContent).toBe("child");
    m.dispose();
  });

  it("preserves multiple children verbatim", () => {
    const m = mount(() => (
      <QuoteWrap quoted={true} baseClass="b" chromeClass="c">
        <span>bullet</span>
        <span>body</span>
      </QuoteWrap>
    ));
    const wrap = m.host.firstChild as HTMLElement;
    expect(wrap.children.length).toBe(2);
    expect(wrap.children[0].textContent).toBe("bullet");
    expect(wrap.children[1].textContent).toBe("body");
    m.dispose();
  });
});

describe("isBlockQuoted", () => {
  it("returns true only for text starting with `> `", () => {
    expect(isBlockQuoted("> hello")).toBe(true);
    expect(isBlockQuoted("> ")).toBe(true);
    expect(isBlockQuoted("hello")).toBe(false);
    expect(isBlockQuoted(">hello")).toBe(false);
  });
});
