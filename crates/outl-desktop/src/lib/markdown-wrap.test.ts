import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { insertLink, wrapSelection } from "./markdown-wrap";

function mountTextarea(value: string, selStart: number, selEnd: number) {
  const ta = document.createElement("textarea");
  document.body.appendChild(ta);
  ta.value = value;
  ta.focus();
  ta.setSelectionRange(selStart, selEnd);
  return ta;
}

describe("wrapSelection", () => {
  let inputCount: number;

  beforeEach(() => {
    document.body.innerHTML = "";
    inputCount = 0;
  });

  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("wraps a selection with the symmetric delimiter", () => {
    const ta = mountTextarea("hello world", 6, 11);
    ta.addEventListener("input", () => {
      inputCount += 1;
    });
    wrapSelection("**");
    expect(ta.value).toBe("hello **world**");
    expect(inputCount).toBe(1);
    // Selection now spans just the wrapped word, not the delimiters.
    expect(ta.selectionStart).toBe(8);
    expect(ta.selectionEnd).toBe(13);
  });

  it("supports asymmetric delimiter pairs (strikethrough)", () => {
    const ta = mountTextarea("nope", 0, 4);
    wrapSelection("~~", "~~");
    expect(ta.value).toBe("~~nope~~");
  });

  it("inserts the pair and parks caret between them on empty selection", () => {
    const ta = mountTextarea("ab", 1, 1);
    wrapSelection("_");
    expect(ta.value).toBe("a__b");
    expect(ta.selectionStart).toBe(2);
    expect(ta.selectionEnd).toBe(2);
  });

  it("fires an input event so Solid signals stay in sync", () => {
    const ta = mountTextarea("x", 0, 1);
    ta.addEventListener("input", () => {
      inputCount += 1;
    });
    wrapSelection("`");
    expect(inputCount).toBe(1);
    expect(ta.value).toBe("`x`");
  });

  it("is a no-op when no textarea is focused", () => {
    const ta = mountTextarea("hello", 0, 5);
    ta.blur();
    document.body.removeChild(ta);
    // No element is focused now — nothing should throw, nothing should change.
    expect(() => wrapSelection("**")).not.toThrow();
    expect(ta.value).toBe("hello");
  });
});

describe("insertLink", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
  });

  it("uses selection as label and selects 'url' for typing", () => {
    const ta = mountTextarea("see google here", 4, 10);
    insertLink();
    expect(ta.value).toBe("see [google](url) here");
    // The `url` slug is selected so the user can type the destination.
    const urlStart = ta.value.indexOf("url");
    expect(ta.selectionStart).toBe(urlStart);
    expect(ta.selectionEnd).toBe(urlStart + 3);
  });

  it("falls back to [text](url) and selects 'text' on empty selection", () => {
    const ta = mountTextarea("ab", 1, 1);
    insertLink();
    expect(ta.value).toBe("a[text](url)b");
    expect(ta.selectionStart).toBe(2); // after "["
    expect(ta.selectionEnd).toBe(6); // end of "text"
  });

  it("is a no-op when no textarea is focused", () => {
    expect(() => insertLink()).not.toThrow();
  });
});
