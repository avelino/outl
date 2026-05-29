import { afterEach, describe, expect, it } from "vitest";
import {
  HIDE_MESSAGE,
  buildShowMessage,
  getNativeSuggesterState,
  setNativeSuggesterState,
} from "./native-suggester";
import { PageMeta } from "./api";

afterEach(() => {
  setNativeSuggesterState(null);
});

describe("buildShowMessage", () => {
  it("strips PageMeta down to the wire shape", () => {
    const pages: PageMeta[] = [
      { id: "a", slug: "avelino", title: "Avelino", kind: "page" },
      { id: "b", slug: "2026-05-28", title: "2026-05-28", kind: "journal" },
    ];
    expect(buildShowMessage(pages)).toEqual({
      action: "show",
      items: [
        { slug: "avelino", title: "Avelino", kind: "page" },
        { slug: "2026-05-28", title: "2026-05-28", kind: "journal" },
      ],
    });
  });

  it("replaces a journal's human title with its ISO slug", () => {
    const pages: PageMeta[] = [
      {
        id: "j1",
        slug: "2026-05-28",
        title: "Thursday, May 28, 2026",
        kind: "journal",
      },
    ];
    expect(buildShowMessage(pages).items[0].title).toBe("2026-05-28");
  });

  it("emits an empty items array when no pages match", () => {
    expect(buildShowMessage([])).toEqual({ action: "show", items: [] });
  });
});

describe("HIDE_MESSAGE", () => {
  it("is the literal hide payload", () => {
    expect(HIDE_MESSAGE).toEqual({ action: "hide" });
  });
});

describe("setNativeSuggesterState / getNativeSuggesterState", () => {
  it("starts null", () => {
    expect(getNativeSuggesterState()).toBeNull();
  });

  it("round-trips a show message through the global", () => {
    const msg = buildShowMessage([
      { id: "a", slug: "avelino", title: "Avelino", kind: "page" },
    ]);
    setNativeSuggesterState(msg);
    expect(getNativeSuggesterState()).toEqual(msg);
  });

  it("clears with HIDE_MESSAGE", () => {
    setNativeSuggesterState(buildShowMessage([]));
    setNativeSuggesterState(HIDE_MESSAGE);
    expect(getNativeSuggesterState()).toEqual({ action: "hide" });
  });

  it("clears entirely with null", () => {
    setNativeSuggesterState(buildShowMessage([]));
    setNativeSuggesterState(null);
    expect(getNativeSuggesterState()).toBeNull();
  });
});
