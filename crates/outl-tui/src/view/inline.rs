//! Inline (span-level) markdown rendering.
//!
//! Two flavours — `render_markdown_inline` strips delimiters and
//! prepends ref icons (read-only blocks), `highlight_inline` keeps the
//! markdown source visible with dim delimiters (cursor-bearing blocks
//! so column-to-byte alignment stays 1:1).

use crate::theme::Theme;
use outl_md::inline::{inline_to_source, tokenize, InlineTok};
use ratatui::text::Span;

/// Strip an optional `TODO`/`DONE` prefix off a block's text, returning
/// both the stripped body and a marker describing what was present.
///
/// `true` means `DONE`, `false` means `TODO`, `None` means no prefix.
/// Delegates to [`outl_actions::split_todo`] so every surface (TUI,
/// mobile, future desktop) agrees on the wire format.
pub(crate) fn split_todo_prefix(text: &str) -> (Option<bool>, &str) {
    let (state, body) = outl_actions::split_todo(text);
    match state {
        Some(outl_actions::TodoState::Todo) => (Some(false), body),
        Some(outl_actions::TodoState::Done) => (Some(true), body),
        None => (None, body),
    }
}

/// Render a block's body the way the outline renders it in read-only
/// mode: strip and visualize a `TODO`/`DONE` prefix, then pass the
/// remainder through [`render_markdown_inline`].
///
/// Used by the outline (for single-line bullets) **and** by the embed
/// expansion (which needs the same affordance — a `TODO` inside an
/// embedded block should still render as `☐` so the reader sees
/// state, not the raw word).
pub(crate) fn render_pretty_block_text(
    text: &str,
    theme: &Theme,
    index: &outl_md::index::WorkspaceIndex,
) -> Vec<Span<'static>> {
    render_pretty_block_text_impl(text, theme, index, true)
}

/// Internal variant that controls whether `InlineTok::Embed` tokens
/// inside `text` are expanded again. Set `expand_embed = false` when
/// the caller is *already* rendering an embed's body — otherwise
/// `A` embedding `B` while `B` embeds `A` (or even `A` itself) would
/// recurse forever and blow the stack.
fn render_pretty_block_text_impl(
    text: &str,
    theme: &Theme,
    index: &outl_md::index::WorkspaceIndex,
    expand_embed: bool,
) -> Vec<Span<'static>> {
    let (todo_state, body) = split_todo_prefix(text);
    let mut out: Vec<Span<'static>> = Vec::new();
    match todo_state {
        Some(false) => {
            out.push(Span::styled("☐ ", theme.todo_open));
            out.extend(render_markdown_inline_impl(
                body,
                theme,
                index,
                expand_embed,
            ));
        }
        Some(true) => {
            out.push(Span::styled("☑ ", theme.todo_done));
            for sp in render_markdown_inline_impl(body, theme, index, expand_embed) {
                out.push(Span::styled(
                    sp.content.into_owned(),
                    sp.style.patch(theme.todo_done_body),
                ));
            }
        }
        None => out.extend(render_markdown_inline_impl(
            text,
            theme,
            index,
            expand_embed,
        )),
    }
    out
}

/// Render with markdown stripped — bold/italic/code/strike applied as
/// styles, `[[ref]]` / `#tag` / `[text](url)` shown without their
/// delimiters. Used when the block is read-only (not selected, not in
/// Insert mode).
///
/// Looks up `[[ref]]` / `#tag` targets in `index` to prepend the
/// page's `icon::` when one is set. The icon is *display-only* — the
/// underlying `.md` keeps the plain `[[Title]]` / `#tag` text.
///
/// `highlight_inline` (the raw, cursor-bearing render) deliberately
/// does *not* take this path — adding a non-source glyph would
/// break column-to-byte alignment for the visible cursor.
pub(crate) fn render_markdown_inline(
    text: &str,
    theme: &Theme,
    index: &outl_md::index::WorkspaceIndex,
) -> Vec<Span<'static>> {
    render_markdown_inline_impl(text, theme, index, true)
}

/// Internal variant: when `expand_embed = false`, `InlineTok::Embed`
/// renders as its raw `!((handle))` form (dim) instead of recursing
/// into the source block. The Embed arm passes `false` to its own
/// recursive call so an A → B → A cycle terminates after a single
/// expansion. Doctor still surfaces the citation in either direction.
fn render_markdown_inline_impl(
    text: &str,
    theme: &Theme,
    index: &outl_md::index::WorkspaceIndex,
    expand_embed: bool,
) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    for tok in tokenize(text) {
        match tok {
            InlineTok::Plain(s) => out.push(Span::raw(s.to_string())),
            InlineTok::PageRef { name } => {
                if let Some(icon) = index.by_title(name).and_then(|p| p.icon.as_deref()) {
                    out.push(Span::styled(format!("{icon} "), theme.dim));
                }
                out.push(Span::styled(name.to_string(), theme.ref_link));
            }
            InlineTok::Tag { name } => {
                if let Some(icon) = index.by_slug(name).and_then(|p| p.icon.as_deref()) {
                    out.push(Span::styled(format!("{icon} "), theme.dim));
                }
                out.push(Span::styled(format!("#{name}"), theme.tag_link));
            }
            InlineTok::Bold { inner } => {
                out.push(Span::styled(inline_to_source(&inner), theme.bold))
            }
            InlineTok::Italic { inner, .. } => {
                out.push(Span::styled(inline_to_source(&inner), theme.italic))
            }
            InlineTok::Strike { inner } => {
                out.push(Span::styled(inline_to_source(&inner), theme.strike))
            }
            InlineTok::Code { inner } => out.push(Span::styled(inner.to_string(), theme.code)),
            InlineTok::Link { text, .. } => out.push(Span::styled(text.to_string(), theme.md_link)),
            InlineTok::BlockRef { handle } => {
                // Resolve the handle to the source block's text and
                // surface it inline, Roam-style. Citing-block readers
                // see content, not a UUID-ish handle.
                //
                // Unresolved handles (orphan reference: source block
                // deleted or never indexed) render dimmed with the
                // handle visible so `outl doctor` (#10) has something
                // to point at and the user understands what broke.
                match index.resolve_block_ref(handle) {
                    Some(entry) => {
                        // Source page icon — same affordance as PageRef.
                        if let Some(icon) = index
                            .by_slug(&entry.source_slug)
                            .and_then(|p| p.icon.as_deref())
                        {
                            out.push(Span::styled(format!("{icon} "), theme.dim));
                        }
                        out.push(Span::styled(entry.text.clone(), theme.ref_link));
                    }
                    None => {
                        out.push(Span::styled(format!("(({handle}))"), theme.dim));
                    }
                }
            }
            InlineTok::Embed { handle } => {
                // Cycle / recursion guard: when this call is itself
                // rendering an embedded block's body, treat the inner
                // embed as raw text. That breaks `A → B → A` cycles
                // and bounds rendering work at one level of expansion.
                if !expand_embed {
                    out.push(Span::styled(format!("!(({handle}))"), theme.dim));
                    continue;
                }
                // Inline read-only render: `↳ ` prefix marks "this row
                // belongs to an embed", and the source block's text is
                // pushed through `render_pretty_block_text_impl` (with
                // `expand_embed = false`) so TODO / DONE prefixes,
                // `[[refs]]`, `#tags`, bold etc. render but a nested
                // `!((blk-Y))` inside `entry.text` doesn't recurse.
                // The subtree below this row (rendered by
                // `emit_embedded_children`) uses the same `↳ ` prefix
                // so the whole embed reads as one visual block.
                match index.resolve_block_ref(handle) {
                    Some(entry) => {
                        if let Some(icon) = index
                            .by_slug(&entry.source_slug)
                            .and_then(|p| p.icon.as_deref())
                        {
                            out.push(Span::styled(format!("{icon} "), theme.dim));
                        }
                        out.push(Span::styled("↳ ".to_string(), theme.dim));
                        // `expand_embed = false`: stop recursive embed
                        // expansion at this depth so a nested embed in
                        // `entry.text` shows up raw and doesn't cycle.
                        out.extend(render_pretty_block_text_impl(
                            &entry.text,
                            theme,
                            index,
                            false,
                        ));
                    }
                    None => {
                        out.push(Span::styled(format!("!(({handle}))"), theme.dim));
                    }
                }
            }
        }
    }
    out
}

/// Render with markdown markers visible (dimmed) and inner text styled.
/// Used when the block is selected in Normal mode (so the visible cursor
/// columns match the underlying source bytes) or in Insert mode. The
/// delimiters themselves use a dim style so the formatting markers
/// don't distract.
pub(crate) fn highlight_inline(text: &str, theme: &Theme) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let dim = theme.dim;

    for tok in tokenize(text) {
        match tok {
            InlineTok::Plain(s) => out.push(Span::raw(s.to_string())),
            InlineTok::PageRef { name } => {
                out.push(Span::styled(format!("[[{name}]]"), theme.ref_link))
            }
            InlineTok::Tag { name } => out.push(Span::styled(format!("#{name}"), theme.tag_link)),
            InlineTok::Bold { inner } => {
                out.push(Span::styled("**".to_string(), dim));
                out.push(Span::styled(inline_to_source(&inner), theme.bold));
                out.push(Span::styled("**".to_string(), dim));
            }
            InlineTok::Italic { inner, marker } => {
                let m = marker.to_string();
                out.push(Span::styled(m.clone(), dim));
                out.push(Span::styled(inline_to_source(&inner), theme.italic));
                out.push(Span::styled(m, dim));
            }
            InlineTok::Strike { inner } => {
                out.push(Span::styled("~~".to_string(), dim));
                out.push(Span::styled(inline_to_source(&inner), theme.strike));
                out.push(Span::styled("~~".to_string(), dim));
            }
            InlineTok::Code { inner } => {
                out.push(Span::styled("`".to_string(), dim));
                out.push(Span::styled(inner.to_string(), theme.code));
                out.push(Span::styled("`".to_string(), dim));
            }
            InlineTok::Link { text, url } => {
                out.push(Span::styled("[".to_string(), dim));
                out.push(Span::styled(text.to_string(), theme.md_link));
                out.push(Span::styled(format!("]({url})"), dim));
            }
            InlineTok::BlockRef { handle } => {
                // Cursor-bearing render keeps the `((...))` delimiters
                // dimmed so column-to-byte alignment for the visible
                // cursor stays 1:1 with the source bytes.
                out.push(Span::styled("((".to_string(), dim));
                out.push(Span::styled(handle.to_string(), theme.ref_link));
                out.push(Span::styled("))".to_string(), dim));
            }
            InlineTok::Embed { handle } => {
                // Cursor-bearing render: full raw source so column
                // accounting stays exact while the user is editing
                // the embed token itself.
                out.push(Span::styled("!((".to_string(), dim));
                out.push(Span::styled(handle.to_string(), theme.ref_link));
                out.push(Span::styled("))".to_string(), dim));
            }
        }
    }
    out
}
