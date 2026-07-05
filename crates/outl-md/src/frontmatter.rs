//! External-markdown page metadata extraction: YAML frontmatter and
//! leading-H1 titles.
//!
//! Markdown produced by other tools (Obsidian, Bear, Jekyll/Hugo
//! exports, …) carries page metadata in a leading `---` fenced YAML
//! block and/or a leading `# H1` heading. Outl represents the same
//! facts as `key:: value` properties, so importers need to split the
//! fenced block off the body, parse the YAML into flat properties, and
//! optionally lift a leading H1 into the page title.
//!
//! This module owns the **generic** parsing/rewriting half of that
//! job. Source-specific policy — which keys a given tool considers
//! app-only metadata, how a `date` value should be normalized — stays
//! with the caller: [`parse_frontmatter`] takes the drop-list as a
//! parameter and returns property values verbatim.

use serde_yaml_ng::Value as YamlValue;

/// Split a leading `---\n...\n---\n` block from the file. Returns
/// `(Some(yaml_text), body)` when present and well-formed; otherwise
/// `(None, original_text)`. YAML's `...` document-end marker is also
/// honoured as a closing fence.
pub fn split_frontmatter(text: &str) -> (Option<String>, String) {
    let normalized: &str = if text.starts_with("---\r\n") {
        return split_frontmatter(&text.replace("\r\n", "\n"));
    } else if text.starts_with("---\n") {
        text
    } else {
        return (None, text.to_string());
    };
    let after_open = &normalized["---\n".len()..];

    let mut cursor = 0usize;
    for line in after_open.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "---" || trimmed == "..." {
            let yaml = after_open[..cursor].to_string();
            let yaml = yaml.strip_suffix('\n').unwrap_or(&yaml).to_string();
            let body_offset = "---\n".len() + cursor + line.len();
            let body = if body_offset >= normalized.len() {
                String::new()
            } else {
                normalized[body_offset..].to_string()
            };
            return (Some(yaml), body);
        }
        cursor += line.len();
    }
    // No closing fence — treat whole file as body so we don't drop
    // user content.
    (None, normalized.to_string())
}

/// Parsed frontmatter, ready to re-emit as outl `key:: value`
/// properties.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Frontmatter {
    /// The `title` key, when present as a scalar.
    pub title: Option<String>,
    /// Remaining scalar properties in source order. Values are
    /// verbatim — any source-specific normalization (dates, …) is the
    /// caller's job.
    pub props: Vec<(String, String)>,
    /// Count of keys that were dropped: either listed in `drop_keys`
    /// or carrying a non-scalar value we can't represent as
    /// `key:: value`.
    pub dropped: usize,
}

/// Parse a YAML frontmatter block into a flat [`Frontmatter`].
///
/// - `title` (scalar) is lifted into [`Frontmatter::title`].
/// - `tags` accepts a scalar (comma / space separated), an inline
///   list, or a block list; the result is normalized to `#name` form
///   and joined with spaces under a single `tags` property.
/// - Keys listed in `drop_keys`, and any key whose value is a
///   sequence / mapping we can't flatten, are counted in
///   [`Frontmatter::dropped`].
/// - Every other scalar key passes through verbatim.
///
/// Returns `None` when the YAML itself fails to parse. Callers should
/// restore the original fenced block verbatim into the body so the
/// user's content isn't silently lost.
pub fn parse_frontmatter(yaml: &str, drop_keys: &[&str]) -> Option<Frontmatter> {
    let parsed: YamlValue = serde_yaml_ng::from_str(yaml).ok()?;
    let map = match parsed {
        YamlValue::Mapping(m) => m,
        _ => return Some(Frontmatter::default()),
    };

    let mut fm = Frontmatter::default();
    for (k, v) in map.into_iter() {
        let YamlValue::String(key) = k else {
            continue;
        };
        if drop_keys.contains(&key.as_str()) {
            fm.dropped += 1;
            continue;
        }
        match key.as_str() {
            "title" => {
                if let Some(s) = scalar_string(&v) {
                    fm.title = Some(s);
                } else {
                    fm.dropped += 1;
                }
            }
            "tags" => {
                let tags = tags_from_yaml(&v);
                if !tags.is_empty() {
                    fm.props.push(("tags".to_string(), tags.join(" ")));
                } else {
                    fm.dropped += 1;
                }
            }
            _ => {
                if let Some(s) = scalar_string(&v) {
                    fm.props.push((key, s));
                } else {
                    // Non-scalar value we can't represent as `key:: v`.
                    fm.dropped += 1;
                }
            }
        }
    }
    Some(fm)
}

/// Render a scalar YAML value (string / number / bool) to a String.
/// Returns `None` for sequences and mappings.
fn scalar_string(v: &YamlValue) -> Option<String> {
    match v {
        YamlValue::String(s) => Some(s.clone()),
        YamlValue::Number(n) => Some(n.to_string()),
        YamlValue::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Extract tags from a YAML value. Frontmatter dialects allow three
/// shapes:
/// - scalar: `tags: foo` (also comma / space separated)
/// - inline list: `tags: [foo, bar]`
/// - block list: `tags:\n  - foo\n  - bar`
///
/// Returned tags are normalized to `#name` form (no leading `#` in
/// the YAML, but `#`-prefixed in outl's `tags::` property).
fn tags_from_yaml(v: &YamlValue) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    match v {
        YamlValue::String(s) => {
            for t in s.split([',', ' ']) {
                let t = t.trim();
                if !t.is_empty() {
                    out.push(tag_form(t));
                }
            }
        }
        YamlValue::Sequence(seq) => {
            for item in seq {
                if let Some(s) = scalar_string(item) {
                    let s = s.trim();
                    if !s.is_empty() {
                        out.push(tag_form(s));
                    }
                }
            }
        }
        _ => {}
    }
    out
}

/// Normalize a tag to `#name` form. Strips any leading `#` the user
/// might have written (source tools accept both `foo` and `#foo`).
fn tag_form(raw: &str) -> String {
    let stripped = raw.trim_start_matches('#');
    format!("#{stripped}")
}

/// If `body` opens (after optional blank lines) with a single H1 line
/// (`# Heading`), return `(Some(title), rest_of_body)` with the H1
/// line stripped. Otherwise return `(None, body_unchanged)`. Only the
/// very first non-blank line is considered — a heading buried inside
/// the body stays as content.
pub fn extract_leading_h1(body: &str) -> (Option<String>, String) {
    let lines: Vec<&str> = body.lines().collect();
    let mut idx = 0;
    while idx < lines.len() && lines[idx].trim().is_empty() {
        idx += 1;
    }
    if idx >= lines.len() {
        return (None, body.to_string());
    }
    let trimmed = lines[idx].trim_start();
    let Some(rest) = trimmed.strip_prefix("# ") else {
        return (None, body.to_string());
    };
    let title = rest.trim().to_string();
    if title.is_empty() {
        return (None, body.to_string());
    }
    let remaining = if lines.len() > idx + 1 {
        lines[idx + 1..].join("\n")
    } else {
        String::new()
    };
    (Some(title), remaining)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- split_frontmatter -------------------------------------------------

    #[test]
    fn split_extracts_fenced_block() {
        let (yaml, body) = split_frontmatter("---\ntitle: X\n---\n- body\n");
        assert_eq!(yaml.as_deref(), Some("title: X"));
        assert_eq!(body, "- body\n");
    }

    #[test]
    fn split_honours_document_end_marker() {
        let (yaml, body) = split_frontmatter("---\ntitle: X\n...\n- body\n");
        assert_eq!(yaml.as_deref(), Some("title: X"));
        assert_eq!(body, "- body\n");
    }

    #[test]
    fn split_handles_crlf() {
        let (yaml, body) = split_frontmatter("---\r\ntitle: X\r\n---\r\n- body\r\n");
        assert_eq!(yaml.as_deref(), Some("title: X"));
        assert_eq!(body, "- body\n");
    }

    #[test]
    fn split_without_fence_returns_original() {
        let (yaml, body) = split_frontmatter("- just bullets\n");
        assert!(yaml.is_none());
        assert_eq!(body, "- just bullets\n");
    }

    #[test]
    fn split_without_closing_fence_keeps_whole_file_as_body() {
        // Malformed frontmatter must not eat the file.
        let (yaml, body) = split_frontmatter("---\ntitle: half\n- bullet\n");
        assert!(yaml.is_none());
        assert_eq!(body, "---\ntitle: half\n- bullet\n");
    }

    #[test]
    fn split_with_empty_body_after_fence() {
        let (yaml, body) = split_frontmatter("---\ntitle: X\n---\n");
        assert_eq!(yaml.as_deref(), Some("title: X"));
        assert_eq!(body, "");
    }

    // --- parse_frontmatter ---------------------------------------------------

    #[test]
    fn title_and_tags_are_extracted() {
        let fm = parse_frontmatter("title: Real Title\ntags: [foo, bar]", &[]).unwrap();
        assert_eq!(fm.title.as_deref(), Some("Real Title"));
        assert_eq!(
            fm.props,
            vec![("tags".to_string(), "#foo #bar".to_string())]
        );
        assert_eq!(fm.dropped, 0);
    }

    #[test]
    fn tags_block_list_form() {
        let fm = parse_frontmatter("tags:\n  - alpha\n  - beta", &[]).unwrap();
        assert_eq!(
            fm.props,
            vec![("tags".to_string(), "#alpha #beta".to_string())]
        );
    }

    #[test]
    fn tags_scalar_comma_separated_and_hash_prefixed() {
        let fm = parse_frontmatter("tags: \"#foo, bar\"", &[]).unwrap();
        assert_eq!(
            fm.props,
            vec![("tags".to_string(), "#foo #bar".to_string())]
        );
    }

    #[test]
    fn unknown_scalar_keys_pass_through_in_order() {
        let fm = parse_frontmatter("author: jane\nrating: 7\ndone: true", &[]).unwrap();
        assert_eq!(
            fm.props,
            vec![
                ("author".to_string(), "jane".to_string()),
                ("rating".to_string(), "7".to_string()),
                ("done".to_string(), "true".to_string()),
            ]
        );
    }

    #[test]
    fn drop_keys_are_counted_not_emitted() {
        let fm = parse_frontmatter(
            "aliases: [foo, bar]\ncssclass: wide\npublish: false",
            &["aliases", "cssclass", "publish", "scroll"],
        )
        .unwrap();
        assert!(fm.props.is_empty());
        assert_eq!(fm.dropped, 3);
    }

    #[test]
    fn non_scalar_values_are_dropped_and_counted() {
        let fm = parse_frontmatter("meta:\n  nested: 1\nok: yes", &[]).unwrap();
        assert_eq!(fm.dropped, 1);
        assert_eq!(fm.props.len(), 1);
    }

    #[test]
    fn invalid_yaml_returns_none() {
        assert!(parse_frontmatter("title: [unclosed", &[]).is_none());
    }

    #[test]
    fn non_mapping_yaml_yields_empty_frontmatter() {
        let fm = parse_frontmatter("- a\n- b", &[]).unwrap();
        assert_eq!(fm, Frontmatter::default());
    }

    // --- extract_leading_h1 ------------------------------------------------

    #[test]
    fn leading_h1_is_lifted_and_stripped() {
        let (title, rest) = extract_leading_h1("# Real Heading\n- under h1\n");
        assert_eq!(title.as_deref(), Some("Real Heading"));
        assert_eq!(rest, "- under h1");
    }

    #[test]
    fn blank_lines_before_h1_are_skipped() {
        let (title, _rest) = extract_leading_h1("\n\n# Heading\n- x\n");
        assert_eq!(title.as_deref(), Some("Heading"));
    }

    #[test]
    fn buried_heading_is_not_a_title() {
        let (title, rest) = extract_leading_h1("- first\n# Not Title\n");
        assert!(title.is_none());
        assert_eq!(rest, "- first\n# Not Title\n");
    }

    #[test]
    fn empty_h1_is_ignored() {
        let (title, _) = extract_leading_h1("# \n- x\n");
        assert!(title.is_none());
    }
}
