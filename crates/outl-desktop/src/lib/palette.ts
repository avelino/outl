/**
 * Install a [`Palette`] returned by the `get_theme` Tauri command
 * as CSS custom properties on `<html>`.
 *
 * Two namespaces are written so existing code keeps working:
 *
 * - **`--color-outl-*`** — the canonical token set (`accent`,
 *   `ref-link-fg`, `selected-bullet-bg`, …). New code references
 *   these.
 * - **`--color-ios-*` / `--color-iosd-*`** — legacy iOS-themed
 *   tokens consumed by `@outl/shared/markdown::MarkdownInline`
 *   today. We map them to the palette so the renderer stays
 *   client-agnostic until the shared lib migrates.
 *
 * Writing both means a single `applyPalette()` call swaps every
 * theme-driven color on the page without remounting the component
 * tree.
 */

import type { Palette } from "./api";

/**
 * Convert `selected_bullet_bg` → `selected-bullet-bg`. Vite / Tailwind
 * surface custom properties hyphen-delimited; the backend uses snake
 * because Rust + Serde do.
 */
function kebab(snake: string): string {
  return snake.replace(/_/g, "-");
}

export function applyPaletteToRoot(palette: Palette) {
  const root = document.documentElement;
  const set = (prop: string, value: string) => root.style.setProperty(prop, value);

  // Canonical --color-outl-* tokens. Walk every field so new keys
  // added to Palette propagate without extra wiring here.
  for (const [field, value] of Object.entries(palette)) {
    if (field === "name") continue;
    if (typeof value !== "string") continue;
    set(`--color-outl-${kebab(field)}`, value);
  }

  // Legacy --color-ios-* / --color-iosd-* tokens consumed by
  // @outl/shared/markdown::MarkdownInline. Until the shared
  // renderer migrates, we route them at the palette they map to.
  set("--color-ios-bg", palette.bg);
  set("--color-ios-text-primary", palette.fg);
  set("--color-ios-text-secondary", palette.fg_dim);
  set("--color-ios-accent", palette.accent);
  set("--color-ios-divider", palette.border);
  set("--color-iosd-bg", palette.bg_elev);
  set("--color-iosd-text-primary", palette.fg);
  set("--color-iosd-text-secondary", palette.fg_dim);
  set("--color-iosd-accent", palette.accent);
  set("--color-iosd-divider", palette.border);

  // Body background + foreground — Tailwind utilities like
  // `bg-(--color-outl-bg)` reference the canonical tokens, but the
  // bare `<body>` should pick the palette up too so the boot frame
  // matches the theme before any class hydrates.
  document.body.style.backgroundColor = palette.bg;
  document.body.style.color = palette.fg;
}
