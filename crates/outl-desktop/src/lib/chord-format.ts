/**
 * Pretty-print a `Chord` / `ChordSequence` for the help overlay.
 *
 * Format mirrors what most macOS apps surface in menu items:
 *
 *   Cmd+Shift+E
 *   Cmd+B
 *   Ctrl+P
 *   g j        (vim-style chord sequence)
 *   ?          (plain symbol)
 *
 * Order of modifiers: Cmd → Ctrl → Alt → Shift → Key. Key names
 * follow ratatui / TUI conventions (Esc, Enter, Tab, Backspace)
 * with arrow glyphs (↑↓←→) for compactness on screen.
 */

import {
  type Chord,
  type Key,
  MOD_ALT,
  MOD_CTRL,
  MOD_META,
  MOD_SHIFT,
} from "./api";

function keyLabel(k: Key): string {
  switch (k.kind) {
    case "Char":
      // Letters → uppercase for menu look (Cmd+B not Cmd+b).
      return /^[a-z]$/.test(k.value) ? k.value.toUpperCase() : k.value;
    case "Enter":
      return "Enter";
    case "Esc":
      return "Esc";
    case "Tab":
      return "Tab";
    case "Backspace":
      return "Backspace";
    case "Delete":
      return "Delete";
    case "Up":
      return "↑";
    case "Down":
      return "↓";
    case "Left":
      return "←";
    case "Right":
      return "→";
    case "Home":
      return "Home";
    case "End":
      return "End";
    case "PageUp":
      return "PgUp";
    case "PageDown":
      return "PgDn";
    case "Space":
      return "Space";
    case "Function":
      return `F${k.value}`;
  }
}

/** Format one chord. */
export function formatChord(chord: Chord): string {
  const parts: string[] = [];
  if (chord.mods & MOD_META) parts.push("Cmd");
  if (chord.mods & MOD_CTRL) parts.push("Ctrl");
  if (chord.mods & MOD_ALT) parts.push("Alt");
  if (chord.mods & MOD_SHIFT) parts.push("Shift");
  parts.push(keyLabel(chord.key));
  return parts.join("+");
}

/**
 * Format a sequence. Single-chord sequences print as the chord;
 * multi-chord sequences (`g j`, `q q`, `d d`) print space-separated.
 */
export function formatSequence(seq: Chord[]): string {
  return seq.map(formatChord).join(" ");
}
