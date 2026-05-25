//! Build and place the `> **result:**` subblock under a code block.
//!
//! Pure functions on [`OutlineNode`] — no parsing, no I/O. Persisting
//! the mutated AST is the caller's job (see [`crate::orchestrate`]).
//!
//! The result block is a *child* of the code block, identified by a
//! literal `> **result:**` marker on its first line. Single-line
//! outputs become an inline code span; multi-line outputs use a fenced
//! code block so newlines render cleanly in any markdown viewer.

use outl_md::parse::OutlineNode;
use sha2::{Digest, Sha256};

use crate::runtime::{ExecError, ExecOutput, ExitStatus};

/// Marker that identifies a result block. The very first non-whitespace
/// text on the line must equal this.
pub const RESULT_MARKER: &str = "> **result:**";

/// Property key written into the result subblock so we can detect
/// whether the parent's source has changed since the last run.
/// Used by [`crate::orchestrate::run_block_at_index_if_source_changed`]
/// and by the TUI's auto-run loop.
pub const SOURCE_HASH_KEY: &str = "source-hash";

/// SHA-256 of `source`, formatted `sha256:<hex>`. Same shape as
/// [`outl_md::sidecar::content_hash`] but without whitespace
/// normalisation — for code, whitespace IS the semantics.
pub fn source_hash(source: &str) -> String {
    let mut h = Sha256::new();
    h.update(source.as_bytes());
    format!("sha256:{}", hex_encode(&h.finalize()))
}

/// Tiny hex encoder so we don't pull in `hex` for one call site.
fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Render an [`ExecOutput`] (or a top-level [`ExecError`]) as the
/// markdown body of a result subblock.
///
/// Format:
///
/// - Single-line stdout → `> **result:** \`output\``
/// - Multi-line stdout  → `> **result:**\n```\noutput\n````
/// - Non-zero / trap    → `> **result (exit N):** ...` or `> **result (trap):** ...`
/// - Infrastructure err → `> **result (error):** \`message\``
pub fn render_result_body(out: Result<&ExecOutput, &ExecError>) -> String {
    match out {
        Err(e) => format!("{RESULT_MARKER} `error: {e}`"),
        Ok(o) => {
            let header = match &o.exit {
                ExitStatus::Ok => RESULT_MARKER.to_string(),
                ExitStatus::NonZero(code) => format!("> **result (exit {code}):**"),
                ExitStatus::Trap(msg) => format!("> **result (trap: {msg}):**"),
            };
            let payload = if o.stdout.is_empty() && !o.stderr.is_empty() {
                // Nothing on stdout but the script wrote to stderr —
                // show that instead so the user sees the message.
                o.stderr.trim_end()
            } else {
                o.stdout.trim_end()
            };

            if payload.is_empty() {
                format!("{header} `(no output)`")
            } else if payload.contains('\n') {
                format!("{header}\n```\n{payload}\n```")
            } else {
                format!("{header} `{payload}`")
            }
        }
    }
}

/// Insert or replace the result child under `parent`.
///
/// If `parent` already has a child whose text starts with
/// [`RESULT_MARKER`], its text is overwritten in place (preserving
/// position, properties, descendant children). Otherwise a fresh child
/// is appended at the end of `parent.children`.
///
/// The "in place" replace is what makes re-runs idempotent: the
/// reconcile pass downstream sees one block edit, not a delete +
/// create, so the block keeps the same ID in the sidecar.
pub fn upsert_result_child(parent: &mut OutlineNode, body: String) {
    if let Some(idx) = parent.children.iter().position(is_result_block) {
        parent.children[idx].text = body;
    } else {
        parent.children.push(OutlineNode {
            text: body,
            properties: Vec::new(),
            children: Vec::new(),
        });
    }
}

/// Same as [`upsert_result_child`] but also stamps the result subblock
/// with a `source-hash::` property. The auto-run loop reads that
/// property to short-circuit re-runs when the parent source hasn't
/// changed.
pub fn upsert_result_child_with_hash(parent: &mut OutlineNode, body: String, source_hash: &str) {
    let idx = match parent.children.iter().position(is_result_block) {
        Some(i) => {
            parent.children[i].text = body;
            i
        }
        None => {
            parent.children.push(OutlineNode {
                text: body,
                properties: Vec::new(),
                children: Vec::new(),
            });
            parent.children.len() - 1
        }
    };
    let props = &mut parent.children[idx].properties;
    if let Some(p) = props.iter_mut().find(|(k, _)| k == SOURCE_HASH_KEY) {
        p.1 = source_hash.to_string();
    } else {
        props.push((SOURCE_HASH_KEY.to_string(), source_hash.to_string()));
    }
}

/// Read the `source-hash::` recorded in the result subblock under
/// `parent`, if any. Returns `None` when no result subblock exists or
/// when it was written without the hash (pre-auto-run runs).
pub fn result_source_hash(parent: &OutlineNode) -> Option<&str> {
    let result = parent.children.iter().find(|c| is_result_block(c))?;
    result
        .properties
        .iter()
        .find(|(k, _)| k == SOURCE_HASH_KEY)
        .map(|(_, v)| v.as_str())
}

fn is_result_block(node: &OutlineNode) -> bool {
    node.text
        .lines()
        .next()
        .map(|first| first.trim_start().starts_with(RESULT_MARKER))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ok_output(stdout: &str) -> ExecOutput {
        ExecOutput {
            stdout: stdout.to_string(),
            stderr: String::new(),
            duration: Duration::from_millis(1),
            exit: ExitStatus::Ok,
        }
    }

    #[test]
    fn single_line_output_uses_inline_backticks() {
        let body = render_result_body(Ok(&ok_output("3")));
        assert_eq!(body, "> **result:** `3`");
    }

    #[test]
    fn multi_line_output_uses_fenced_block() {
        let body = render_result_body(Ok(&ok_output("a\nb\nc")));
        assert_eq!(body, "> **result:**\n```\na\nb\nc\n```");
    }

    #[test]
    fn empty_output_shows_placeholder() {
        let body = render_result_body(Ok(&ok_output("")));
        assert_eq!(body, "> **result:** `(no output)`");
    }

    #[test]
    fn non_zero_exit_marks_header() {
        let out = ExecOutput {
            stdout: "boom".into(),
            stderr: String::new(),
            duration: Duration::from_millis(1),
            exit: ExitStatus::NonZero(2),
        };
        let body = render_result_body(Ok(&out));
        assert!(body.starts_with("> **result (exit 2):**"));
    }

    #[test]
    fn trap_marks_header_with_message() {
        let out = ExecOutput {
            stdout: String::new(),
            stderr: "divide by zero".into(),
            duration: Duration::from_millis(1),
            exit: ExitStatus::Trap("div-by-zero".into()),
        };
        let body = render_result_body(Ok(&out));
        assert!(body.starts_with("> **result (trap: div-by-zero):**"));
        assert!(body.contains("divide by zero"));
    }

    #[test]
    fn error_path_renders_message() {
        let err = ExecError::Timeout(Duration::from_secs(2));
        let body = render_result_body(Err(&err));
        assert!(body.starts_with("> **result:** `error:"));
        assert!(body.contains("timed out"));
    }

    #[test]
    fn upsert_creates_child_when_absent() {
        let mut parent = OutlineNode {
            text: "```lisp\n(+ 1 2)\n```".into(),
            properties: Vec::new(),
            children: Vec::new(),
        };
        upsert_result_child(&mut parent, "> **result:** `3`".into());
        assert_eq!(parent.children.len(), 1);
        assert_eq!(parent.children[0].text, "> **result:** `3`");
    }

    #[test]
    fn upsert_replaces_existing_result_child_in_place() {
        let mut parent = OutlineNode {
            text: "```lisp\n(+ 1 2)\n```".into(),
            properties: Vec::new(),
            children: vec![OutlineNode {
                text: "> **result:** `old`".into(),
                properties: Vec::new(),
                children: Vec::new(),
            }],
        };
        upsert_result_child(&mut parent, "> **result:** `new`".into());
        assert_eq!(parent.children.len(), 1, "must not create a second child");
        assert_eq!(parent.children[0].text, "> **result:** `new`");
    }

    #[test]
    fn upsert_ignores_non_result_children() {
        let mut parent = OutlineNode {
            text: "code".into(),
            properties: Vec::new(),
            children: vec![OutlineNode {
                text: "some unrelated note".into(),
                properties: Vec::new(),
                children: Vec::new(),
            }],
        };
        upsert_result_child(&mut parent, "> **result:** `42`".into());
        assert_eq!(parent.children.len(), 2);
        // Unrelated note untouched, new result appended at end.
        assert_eq!(parent.children[0].text, "some unrelated note");
        assert_eq!(parent.children[1].text, "> **result:** `42`");
    }

    #[test]
    fn source_hash_is_stable() {
        let a = source_hash("(+ 1 2)");
        let b = source_hash("(+ 1 2)");
        assert_eq!(a, b);
        assert!(a.starts_with("sha256:"));
    }

    #[test]
    fn source_hash_differs_on_whitespace() {
        // Code is whitespace-sensitive (Python!) — don't collapse.
        assert_ne!(source_hash("a b"), source_hash("a  b"));
        assert_ne!(source_hash("a\nb"), source_hash("a b"));
    }

    #[test]
    fn upsert_with_hash_stamps_property_on_existing_child() {
        let mut parent = OutlineNode {
            text: "code".into(),
            properties: Vec::new(),
            children: vec![OutlineNode {
                text: "> **result:** `old`".into(),
                properties: Vec::new(),
                children: Vec::new(),
            }],
        };
        upsert_result_child_with_hash(&mut parent, "> **result:** `3`".into(), "sha256:deadbeef");
        assert_eq!(parent.children.len(), 1);
        assert_eq!(parent.children[0].text, "> **result:** `3`");
        assert_eq!(result_source_hash(&parent), Some("sha256:deadbeef"));
    }

    #[test]
    fn upsert_with_hash_creates_child_when_absent() {
        let mut parent = OutlineNode {
            text: "code".into(),
            properties: Vec::new(),
            children: Vec::new(),
        };
        upsert_result_child_with_hash(&mut parent, "> **result:** `5`".into(), "sha256:abc");
        assert_eq!(parent.children.len(), 1);
        assert_eq!(result_source_hash(&parent), Some("sha256:abc"));
    }

    #[test]
    fn result_source_hash_returns_none_when_unstamped() {
        let parent = OutlineNode {
            text: "code".into(),
            properties: Vec::new(),
            children: vec![OutlineNode {
                text: "> **result:** `legacy`".into(),
                properties: Vec::new(),
                children: Vec::new(),
            }],
        };
        // Legacy result subblock (written before the hash logic
        // shipped) should look like "never ran" so it gets refreshed.
        assert_eq!(result_source_hash(&parent), None);
    }
}
