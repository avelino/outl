//! External wiki-link rewriting: collapse `[[…]]` variants from other
//! tools into outl's canonical `[[Note]]` form, and convert image
//! wiki-links into standard CommonMark links.
//!
//! Tools like Obsidian extend the wiki-link syntax with aliases
//! (`[[Note|alias]]`), heading refs (`[[Note#heading]]`), block refs
//! (`[[Note^block-id]]`), folder prefixes (`[[folder/Note]]`), and
//! image attachments (`![[image.png]]`). Outl supports none of those
//! variants — pages are flat, block refs use `((blk-XXXXXX))`, and
//! images are plain CommonMark — so importers (and any future
//! paste-coercion path) collapse them with the helpers here.
//!
//! Everything in this module is pure text → text; no vault layout or
//! routing policy.

/// File extensions treated as image attachments. When a wiki-link
/// target ends in one of these, the link is rewritten as a standard
/// CommonMark link / image rather than a `[[ref]]`, because outl has
/// no notion of image-as-page and a bare `[[bar.jpeg]]` would be a
/// dangling ref.
const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "avif", "ico", "tiff", "tif",
];

/// True when a wiki-link target points at an image attachment.
///
/// Source tools allow `#heading` and `^block-id` suffixes on any wiki
/// link, including images (e.g. `![[image.png#crop]]` for image
/// cropping) — those are stripped before checking the extension.
pub fn is_image_target(target: &str) -> bool {
    let stripped = target.split_once('#').map(|(t, _)| t).unwrap_or(target);
    let stripped = stripped.split_once('^').map(|(t, _)| t).unwrap_or(stripped);
    let lower = stripped.to_ascii_lowercase();
    IMAGE_EXTENSIONS
        .iter()
        .any(|ext| lower.ends_with(&format!(".{ext}")))
}

/// Convert wiki-link / embed syntax for image assets into standard
/// CommonMark links, preserving the original folder path so the link
/// stays resolvable. Shapes handled:
///
/// - `![[assets/foo/bar.jpeg]]`              → `![bar.jpeg](assets/foo/bar.jpeg)`
/// - `![[assets/foo/bar.jpeg|caption]]`      → `![caption](assets/foo/bar.jpeg)`
/// - `[[assets/foo/bar.jpeg|Open: x.png]]`   → `[Open: x.png](assets/foo/bar.jpeg)`
/// - `[[assets/foo/bar.jpeg]]`               → `[bar.jpeg](assets/foo/bar.jpeg)`
///
/// Non-image wiki-links pass through untouched (the regular
/// [`rewrite_wikilinks`] pass handles them afterwards).
pub fn convert_image_links(text: &str) -> String {
    if !text.contains("[[") {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    while let Some(open_rel) = text[cursor..].find("[[") {
        let abs_open = cursor + open_rel;
        // Flush preceding text, including a possible leading '!'.
        out.push_str(&text[cursor..abs_open]);
        let embed = abs_open > 0 && bytes[abs_open - 1] == b'!';
        if embed {
            // Drop the '!' we already flushed; we'll re-emit it inside
            // the rewritten image token.
            out.pop();
        }
        let after_open = abs_open + 2;
        let Some(close_rel) = text[after_open..].find("]]") else {
            // Unbalanced — flush rest verbatim and stop.
            out.push_str(&text[abs_open..]);
            return out;
        };
        let close = after_open + close_rel;
        let inner = &text[after_open..close];

        // Split target / alias on the first '|'.
        let (target, alias) = match inner.split_once('|') {
            Some((t, a)) => (t.trim(), Some(a.trim())),
            None => (inner.trim(), None),
        };

        if is_image_target(target) {
            // Preserve folder path in the URL so the link resolves once
            // the user copies the `assets/` tree alongside the workspace.
            // Heading / block-ref suffixes are kept in the URL (they
            // may carry meaning, e.g. image-crop fragments) but
            // dropped from the caption so the alt text stays readable.
            let link_target = target.trim();
            let caption_target = link_target
                .split_once('#')
                .map(|(t, _)| t)
                .unwrap_or(link_target);
            let caption_target = caption_target
                .split_once('^')
                .map(|(t, _)| t)
                .unwrap_or(caption_target);
            let leaf = caption_target
                .rsplit('/')
                .next()
                .unwrap_or(caption_target)
                .trim();
            let caption = alias.unwrap_or(leaf);
            if embed {
                out.push('!'); // image embed: `![caption](target)`
            }
            out.push('[');
            out.push_str(caption);
            out.push_str("](");
            out.push_str(link_target);
            out.push(')');
        } else {
            // Non-image — re-emit verbatim (including the leading '!'
            // if we popped one) so the downstream wiki-link rewriter
            // sees the original token.
            if embed {
                out.push('!');
            }
            out.push_str("[[");
            out.push_str(inner);
            out.push_str("]]");
        }
        cursor = close + 2;
    }
    out.push_str(&text[cursor..]);
    out
}

/// Rewrite external wiki-link variants to canonical `[[Note]]` form.
///
/// - `[[Note|alias]]` → `[[Note]]`
/// - `[[Note#heading]]` → `[[Note]]`
/// - `[[Note^block-id]]` → `[[Note]]`
/// - `[[folder/Note]]` → `[[Note]]`
/// - `[[Note]]` → unchanged
///
/// Embeds (`![[...]]`) are passed through with the same target
/// cleanup — outl supports block embeds in that shape, so the `!`
/// prefix survives untouched (it sits outside the `[[` token).
pub fn rewrite_wikilinks(text: &str) -> String {
    if !text.contains("[[") {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    while let Some(open_rel) = text[cursor..].find("[[") {
        let abs_open = cursor + open_rel;
        out.push_str(&text[cursor..abs_open]);
        let after_open = abs_open + 2;
        if let Some(close_rel) = text[after_open..].find("]]") {
            let close = after_open + close_rel;
            let inner = &text[after_open..close];
            out.push_str("[[");
            out.push_str(&clean_wikilink_target(inner));
            out.push_str("]]");
            cursor = close + 2;
        } else {
            // Unbalanced — copy the rest verbatim and stop.
            out.push_str(&text[abs_open..]);
            return out;
        }
    }
    out.push_str(&text[cursor..]);
    out
}

/// Strip alias / heading / block-ref markers and folder prefixes from
/// a wiki-link target. The `|` alias marker binds tightest (source
/// tools forbid `|` inside the target itself), then `#` heading, then
/// `^` block ref. Folder prefixes (`folder/Note`) collapse to the
/// last path segment because outl pages are flat.
pub fn clean_wikilink_target(inner: &str) -> String {
    let target = inner.split_once('|').map(|(t, _)| t).unwrap_or(inner);
    let target = target.split_once('#').map(|(t, _)| t).unwrap_or(target);
    let target = target.split_once('^').map(|(t, _)| t).unwrap_or(target);
    let target = target.rsplit_once('/').map(|(_, n)| n).unwrap_or(target);
    target.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- rewrite_wikilinks / clean_wikilink_target -----------------------

    #[test]
    fn alias_is_stripped() {
        assert_eq!(
            rewrite_wikilinks("see [[Target|the alias]] here"),
            "see [[Target]] here"
        );
    }

    #[test]
    fn heading_is_stripped() {
        assert_eq!(
            rewrite_wikilinks("jump to [[Target#section]] now"),
            "jump to [[Target]] now"
        );
    }

    #[test]
    fn block_ref_is_stripped() {
        assert_eq!(rewrite_wikilinks("see [[Target^abc123]]"), "see [[Target]]");
    }

    #[test]
    fn folder_prefix_is_stripped() {
        assert_eq!(
            rewrite_wikilinks("link [[folder/sub/Target]] now"),
            "link [[Target]] now"
        );
    }

    #[test]
    fn combined_variants_collapse_to_target() {
        assert_eq!(
            rewrite_wikilinks("x [[folder/Note|alias#section]] y"),
            "x [[Note]] y"
        );
    }

    #[test]
    fn plain_link_is_unchanged() {
        assert_eq!(rewrite_wikilinks("- [[Note]] stays"), "- [[Note]] stays");
    }

    #[test]
    fn unbalanced_link_passes_through() {
        assert_eq!(rewrite_wikilinks("broken [[Note"), "broken [[Note");
    }

    // --- is_image_target ---------------------------------------------------

    #[test]
    fn image_extensions_are_detected_case_insensitively() {
        assert!(is_image_target("foo.PNG"));
        assert!(is_image_target("a/b/pic.jpeg"));
        assert!(!is_image_target("Spec.v3"));
        assert!(!is_image_target("image-notes"));
    }

    #[test]
    fn image_target_with_suffixes_is_still_recognised() {
        assert!(is_image_target("image.png#crop"));
        assert!(is_image_target("photo.jpeg^meta"));
    }

    // --- convert_image_links -------------------------------------------------

    #[test]
    fn image_embed_becomes_md_image() {
        assert_eq!(
            convert_image_links("see ![[image.png]]"),
            "see ![image.png](image.png)"
        );
    }

    #[test]
    fn note_embed_is_preserved() {
        assert_eq!(
            convert_image_links("see ![[other-note]]"),
            "see ![[other-note]]"
        );
    }

    #[test]
    fn image_link_with_alias_uses_alias_as_caption() {
        assert_eq!(
            convert_image_links("- [[assets/foo/bar.jpeg|Open: pasted.png]] here"),
            "- [Open: pasted.png](assets/foo/bar.jpeg) here"
        );
    }

    #[test]
    fn image_link_without_alias_uses_leaf_name_as_caption() {
        assert_eq!(
            convert_image_links("see [[folder/deep/photo.jpeg]] now"),
            "see [photo.jpeg](folder/deep/photo.jpeg) now"
        );
    }

    #[test]
    fn suffixes_stay_in_url_but_not_in_caption() {
        assert_eq!(
            convert_image_links("![[image.png#crop]]"),
            "![image.png](image.png#crop)"
        );
        assert_eq!(
            convert_image_links("[[photo.jpeg^meta|cap]]"),
            "[cap](photo.jpeg^meta)"
        );
    }

    #[test]
    fn non_image_links_pass_through_for_the_wikilink_pass() {
        assert_eq!(
            convert_image_links("- [[Spec.v3]] and [[image-notes]]"),
            "- [[Spec.v3]] and [[image-notes]]"
        );
    }
}
