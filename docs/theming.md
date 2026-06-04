# Theming

The TUI uses a structured palette: every styled surface (`ref_link`, `cursor_block`, `bold`, `status_normal`, ...) is a named field on a `Theme` struct.
Renderers never reference hard-coded colors, so swapping themes is one assignment.

## Picking a theme

Three ways to pick, in precedence order:

1. **CLI flag** (this run only):
   ```bash
   outl --workspace ~/notes --theme dracula
   ```
2. **Workspace config** — persists for the workspace:
   ```toml
   # ~/notes/.outl/config.toml
   [theme]
   preset = "dracula"
   ```
3. **Default** — `default-dark` when nothing is configured.

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

Themes are just constructors in `crates/outl-tui/src/theme.rs`:

```rust
pub fn my_theme() -> Theme {
    Theme {
        name: "my-theme",
        background: Color::Rgb(20, 20, 30),
        bullet: Style::default().fg(Color::Rgb(100, 100, 120)),
        // ... fill every field
    }
}
```

Then:

1. Add the name to the `PRESETS` slice.
2. Add the `"my-theme" => Some(my_theme())` arm to `by_name`.
3. The compiler tells you if you missed a field.
   Done.

The `every_listed_preset_resolves` test ensures every name in `PRESETS` has a working constructor.
The `theme_name_matches_preset_id` test catches typos where you say `name: "monokay"` but listed `"monokai"`.

## Tips

- **Don't overlap modifiers on the same field across themes.** Solarized's `bold` is `fg(orange) + BOLD`; Dracula's is similar but on orange too.
  Keep modifiers semantic (BOLD for bold, etc.) and let the color carry the personality.
- **Backgrounds**: only `popup_bg` and `background` matter today.
  The alternate screen inherits the terminal's background; we don't fight it.
- **Underline on `ref_link` and `tag_link` is intentional.** They're the only "clickable" things in pretty-render mode, and the underline is the visual affordance.
- **Contrast matters more than tone.** Test your theme against a workspace with lots of refs, tags, code, and TODOs.

## Future

- **User TOML overrides** — `[theme.colors]` table letting you tweak fields without rebuilding the binary.
- **Theme hot-reload on config change** — listen on `.outl/config.toml` via `notify`.
- **`outl theme preview`** — render every preset side-by-side on the same fixture so you can pick by feel.

None of these are wired today; they're tracked when there's user demand.
