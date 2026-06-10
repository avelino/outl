# Theming

outl ships seven built-in palettes (`outl`, `default-dark`, `light`, `dracula`, `solarized-dark`, `nord`, `monokai`).
The hex values live in the shared **`outl-theme`** crate — every styled surface (`ref_link_fg`, `cursor_block_bg`, `bold_fg`, `status_normal_bg`, …) is a named field on a `Palette` struct, and the TUI / desktop / mobile clients each turn those hex strings into whatever their renderer expects.

This means a color change in `outl-theme/src/presets.rs` propagates to every client without a coordinated edit.

## Picking a theme

Four ways to pick, in precedence order:

1. **CLI flag** (this run only):
   ```bash
   outl --workspace ~/notes --theme dracula
   ```
2. **Per-workspace config** — overrides only this workspace:
   ```toml
   # ~/notes/.outl/config.toml
   [theme]
   preset = "dracula"
   ```
3. **Global config** — read by every client (TUI + desktop) via the shared **`outl-config`** crate:
   ```toml
   # ~/.config/outl/config.toml
   [theme]
   preset = "outl"
   ```
   The desktop's Settings modal writes here, so changing the theme there propagates to the next `outl-tui` launch automatically.
4. **Default** — `outl` (brand palette) when nothing is configured.

Names are case- and separator-insensitive.
`dracula`, `Dracula`, `DRACULA`, `Solarized Dark`, `solarized_dark`, `solarized-dark` all resolve to the same theme.

You can also swap themes at runtime from the command palette:

```
:theme nord
```

The status line confirms the switch (`theme: nord`).

## Built-in presets

| Name | Vibe |
|------|------|
| `default-dark` | The original outl-tui palette. Cyan refs, magenta tags, green code. |
| `light` | High-brightness terminals. Blue refs, red tags. |
| `dracula` | Iconic dark palette — pink, purple, cyan, yellow. |
| `solarized-dark` | Ethan Schoonover's classic. Muted base03 background. |
| `nord` | Arctic blue-greys. Cool, low-contrast. |
| `monokai` | Wimer Hazenberg's high-contrast. Hot pink for highlights. |

`outl theme list` prints them on a terminal.
`outl theme show <name>` dumps every style in that preset (`ref_link = Style { fg: ..., ... }`).

## Semantic surfaces

Every preset fills every field.
If you add a new field to `Theme`, **every preset must set it** — the compiler enforces this.

### Outline

| Field | Used for |
|-------|----------|
| `bullet` | The `- ` glyph on a regular block |
| `selected_bullet` | The `- ` glyph on the focused block |
| `cursor_block` | Vim-style block cursor (char under cursor in Normal) |
| `cursor_caret` | Thin caret (`▏`) at end-of-line or in Insert |
| `property_key` / `property_value` | `key:: value` lines |
| `heading` | Page title in the header |

### Inline tokens

| Field | Used for |
|-------|----------|
| `ref_link` | `[[page]]` references |
| `tag_link` | `#tag` references |
| `md_link` | `[text](url)` markdown links |
| `bold` / `italic` / `strike` / `code` | Standard markdown emphasis |
| `todo_open` / `todo_done` / `todo_done_body` | TODO / DONE prefix + DONE body |
| `dim` | Delimiters in raw render mode (`**`, `~~`, etc.) |

### Chrome

| Field | Used for |
|-------|----------|
| `border` | Panel borders |
| `hint` | Footer hint text |
| `status_normal` / `status_insert` / `status_visual` | Mode badges |
| `status_message` | Transient status messages |
| `help_title` | Section titles in the help popup, overlay titles |
| `popup_bg` | Background color for overlays |
| `list_selected` | Highlighted entry in popups (quick switcher, search) |

## Defining a new preset

Presets are constructors in `crates/outl-theme/src/presets.rs`:

```rust
pub fn my_theme() -> Palette {
    Palette {
        name: "my-theme".into(),
        bg: "#141420".into(),
        // ... fill every field with a #rrggbb hex string
    }
}
```

Then:

1. Add the name to the `PRESETS` slice in `crates/outl-theme/src/lib.rs`.
2. Add the `"my-theme" => Some(presets::my_theme())` arm to `by_name`.
3. The compiler tells you if you missed a field on `Palette`.
4. Add a TUI delegate in `crates/outl-tui/src/theme.rs`:
   ```rust
   pub fn my_theme() -> Theme {
       theme_from_palette("my-theme", &outl_theme::presets::my_theme())
   }
   ```
   `default-dark` and `light` are the two TUI presets that *don't* go through `theme_from_palette` — they build on top of ANSI named colors (`Color::Reset`, `Color::DarkGray`, …) so the user's terminal palette shows through, which is intentional for ANSI-only environments.
5. Desktop and mobile clients pick the preset up automatically through `list_themes` / `get_theme` Tauri commands.

The `every_listed_preset_resolves` test ensures every name in `PRESETS` has a working constructor.
The `every_palette_field_is_hex` test catches a typo like `"#xyz123"` or a missed `#` prefix before it hits the renderers.

## Tips

- **Don't overlap modifiers on the same field across themes.** Solarized's `bold` is `fg(orange) + BOLD`; Dracula's is similar but on orange too.
  Keep modifiers semantic (BOLD for bold, etc.) and let the color carry the personality.
- **Backgrounds**: the RGB presets (`outl`, `dracula`, `solarized-dark`, `nord`, `monokai`) paint `bg` across the whole TUI canvas and use `fg` as the base text color, so a light theme stays readable on a dark terminal (and vice versa).
  Only the two ANSI presets (`default-dark`, `light`) keep `Color::Reset` and inherit the terminal's own background/foreground — that's their point.
- **Underline on `ref_link` and `tag_link` is intentional.** They're the only "clickable" things in pretty-render mode, and the underline is the visual affordance.
- **Contrast matters more than tone.** Test your theme against a workspace with lots of refs, tags, code, and TODOs.

## How each client consumes the palette

| Client | What it does with the hex |
|---|---|
| **`outl-tui`** | `crates/outl-tui/src/theme.rs::theme_from_palette` converts each `#rrggbb` to `ratatui::Color::Rgb(r, g, b)` and re-applies the consistent modifiers (`BOLD` on `bold`, `UNDERLINED` on links, `ITALIC` on `italic`, `CROSSED_OUT` on `strike`). The five RGB presets (`outl`, `dracula`, `solarized-dark`, `nord`, `monokai`) are one-line delegates; `default-dark` and `light` stay manual on ANSI named colors. |
| **`outl-desktop`** | The Tauri commands `list_themes()` and `get_theme(name)` return the `Palette` as JSON. The frontend writes each field as a CSS custom property on `<html>` (`--color-outl-accent`, `--color-outl-ref-link-fg`, …) so Tailwind class utilities like `text-(--color-outl-accent)` resolve at runtime, and flips `color-scheme` (light/dark) from the palette's `bg` luminance so native controls and scrollbars follow. Settings modal exposes the dropdown. Chrome surfaces never hardcode a hue — translucent layers derive from `--color-outl-fg` (`bg-(--color-outl-fg)/10`) so they adapt to light and dark presets alike. |
| **`outl-mobile`** | Today the mobile client uses its iOS-specific tokens (`--color-ios-*`). When desktop ships, it mirrors those names so `<MarkdownInline />` from `@outl/shared` stays portable. The migration to neutral `--color-outl-*` tokens lands when mobile picks up the theme picker. |

## Future

- **User TOML overrides** — `[theme.colors]` table letting you tweak fields without rebuilding the binary.
- **Theme hot-reload on config change** — listen on `~/.config/outl/config.toml` and per-workspace `.outl/config.toml` via `notify`.
- **`outl theme preview`** — render every preset side-by-side on the same fixture so you can pick by feel.

None of these are wired today; they're tracked when there's user demand.
