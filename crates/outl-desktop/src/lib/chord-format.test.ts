import { describe, expect, it } from "vitest";

import {
  type Chord,
  MOD_ALT,
  MOD_CTRL,
  MOD_META,
  MOD_SHIFT,
} from "./api";
import { formatChord, formatSequence } from "./chord-format";

function chord(mods: number, key: Chord["key"]): Chord {
  return { mods, key };
}

describe("formatChord", () => {
  it("uppercases single letters for menu look", () => {
    expect(formatChord(chord(MOD_META, { kind: "Char", value: "b" }))).toBe(
      "Cmd+B",
    );
  });

  it("emits modifier order Cmd → Ctrl → Alt → Shift", () => {
    const c = chord(
      MOD_META | MOD_CTRL | MOD_ALT | MOD_SHIFT,
      { kind: "Char", value: "k" },
    );
    expect(formatChord(c)).toBe("Cmd+Ctrl+Alt+Shift+K");
  });

  it("renders arrow keys as glyphs", () => {
    expect(formatChord(chord(0, { kind: "Up" }))).toBe("↑");
    expect(formatChord(chord(0, { kind: "Down" }))).toBe("↓");
    expect(formatChord(chord(0, { kind: "Left" }))).toBe("←");
    expect(formatChord(chord(0, { kind: "Right" }))).toBe("→");
  });

  it("renders Enter / Esc / Tab / Backspace / Delete by name", () => {
    expect(formatChord(chord(0, { kind: "Enter" }))).toBe("Enter");
    expect(formatChord(chord(0, { kind: "Esc" }))).toBe("Esc");
    expect(formatChord(chord(0, { kind: "Tab" }))).toBe("Tab");
    expect(formatChord(chord(0, { kind: "Backspace" }))).toBe("Backspace");
    expect(formatChord(chord(0, { kind: "Delete" }))).toBe("Delete");
  });

  it("compacts PageUp / PageDown to PgUp / PgDn", () => {
    expect(formatChord(chord(0, { kind: "PageUp" }))).toBe("PgUp");
    expect(formatChord(chord(0, { kind: "PageDown" }))).toBe("PgDn");
  });

  it("renders function keys as F<n>", () => {
    expect(formatChord(chord(0, { kind: "Function", value: 5 }))).toBe("F5");
    expect(formatChord(chord(MOD_CTRL, { kind: "Function", value: 12 }))).toBe(
      "Ctrl+F12",
    );
  });

  it("leaves non-letter chars as-is (no uppercase for /, ?, etc.)", () => {
    expect(formatChord(chord(0, { kind: "Char", value: "/" }))).toBe("/");
    expect(formatChord(chord(MOD_META, { kind: "Char", value: "/" }))).toBe(
      "Cmd+/",
    );
    expect(formatChord(chord(MOD_META | MOD_SHIFT, { kind: "Char", value: "?" })))
      .toBe("Cmd+Shift+?");
  });
});

describe("formatSequence", () => {
  it("formats a single-chord sequence as the chord itself", () => {
    expect(formatSequence([chord(MOD_META, { kind: "Char", value: "p" })])).toBe(
      "Cmd+P",
    );
  });

  it("space-separates vim-style chord sequences", () => {
    const seq: Chord[] = [
      chord(0, { kind: "Char", value: "g" }),
      chord(0, { kind: "Char", value: "j" }),
    ];
    expect(formatSequence(seq)).toBe("G J");
  });

  it("handles q q (Quit) and d d (Delete) prefixes", () => {
    expect(
      formatSequence([
        chord(0, { kind: "Char", value: "q" }),
        chord(0, { kind: "Char", value: "q" }),
      ]),
    ).toBe("Q Q");
    expect(
      formatSequence([
        chord(0, { kind: "Char", value: "d" }),
        chord(0, { kind: "Char", value: "d" }),
      ]),
    ).toBe("D D");
  });
});
