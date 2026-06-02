//! Paste external markdown as a tree of blocks.
//!
//! When a user copies a chunk of bullet-list markdown from another app
//! (Roam, Logseq, GitHub issue, Notion export, Notes.app draft) and
//! pastes it into outl, we want the hierarchy to come across — not a
//! single block with literal `\n- ` characters in it. This module is
//! the **single** entry point both clients (TUI, mobile) call to make
//! that happen, so the semantics stay identical between surfaces.
//!
//! ## What gets converted to outl syntax on the way in
//!
//! `paste_markdown` runs the raw clipboard text through
//! [`normalize::normalize_external_syntax`] before parsing it. The
//! conversions cover the most common syntaxes the user might copy
//! from:
//!
//! | Input (external) | Output (outl) | Origin |
//! |------------------|---------------|--------|
//! | `{{[[TODO]]}} foo` | `TODO foo` | Roam |
//! | `{{[[DONE]]}} foo` | `DONE foo` | Roam |
//! | `- [ ] foo` | `- TODO foo` | GitHub / CommonMark task list |
//! | `- [x] foo` / `- [X] foo` | `- DONE foo` | GitHub / CommonMark |
//! | `{{embed: ((blk-XXXXXX))}}` | `!((blk-XXXXXX))` | Roam |
//! | `{{[[query]]: foo}}` | `{{query: foo}}` | Roam |
//! | `^^highlight^^` | (stripped) | Roam |
//! | `{{video: url}}` and other unknown `{{…}}` | (stripped) | various |
//! | `id:: 01HXY…` (alone on a line) | (line dropped) | Logseq |
//! | 4-space indent | 2-space indent | Roam/Notion export |
//!
//! The unknown-token strip is deliberate: blocks come into outl clean.
//! We never invent information; we only delete tokens that aren't
//! part of our syntax.
//!
//! ## Properties
//!
//! Lines of the form `key:: value` that the parser attaches to a block
//! are re-applied to the freshly minted node as `Op::SetProp` via
//! [`crate::page::set_property`]. They converge across devices the
//! same way every other op does.

mod anchors;
mod normalize;

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

use crate::block::BlockTreeOutcome;
use crate::error::ActionError;

pub use normalize::normalize_external_syntax;

/// Where in the workspace the pasted markdown should be grafted.
#[derive(Debug, Clone)]
pub enum PasteAnchor {
    /// Append the pasted blocks as new last children of `parent`.
    AsLastChildOf(NodeId),
    /// Insert the pasted blocks as siblings immediately after `after`.
    AfterBlock(NodeId),
    /// The user is editing `block` and the caret sits at char offset
    /// `caret` inside its text.
    ///
    /// The first parsed bullet (if any) is appended to the text on the
    /// left of the caret and becomes the new text of `block`; any
    /// children of that first bullet land as children of `block`.
    /// Subsequent root-level bullets become siblings after `block`.
    /// Finally, whatever text was on the right of the caret is added
    /// as one more sibling so nothing the user typed gets lost.
    AtCaret {
        /// Block currently being edited.
        block: NodeId,
        /// Caret position, measured in `char` offsets into the block's
        /// text (not byte offsets).
        caret: usize,
    },
}

/// What `paste_markdown` did, so the caller can update UI state.
#[derive(Debug, Clone, Default)]
pub struct PasteOutcome {
    /// Ids of newly created blocks, in DFS / sibling order.
    pub new_blocks: Vec<NodeId>,
    /// New text of the host block when `AtCaret` was used and the
    /// host's text changed. `None` for the other anchors.
    pub host_text: Option<String>,
    /// Number of root-level blocks the caller should report to the
    /// user (e.g. status line `"pasted N blocks"`). Zero when the
    /// paste was treated as plain text.
    pub root_count: usize,
}

/// Apply pasted markdown to the workspace at `anchor`.
///
/// `raw` is the clipboard contents verbatim. The function:
///
/// 1. Normalises external syntax to outl (see module docs).
/// 2. Detects whether the result is an outline (any line starting
///    with `- `). If not, falls back to "plain text" behaviour
///    appropriate for the anchor.
/// 3. Parses the normalised text via `outl_md::parse::parse`.
/// 4. Materialises blocks through [`crate::block::append_tree`] /
///    [`crate::block::create_after`].
/// 5. Re-applies block properties via `Op::SetProp`.
pub fn paste_markdown(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    anchor: PasteAnchor,
    raw: &str,
) -> Result<PasteOutcome, ActionError> {
    let normalized = normalize_external_syntax(raw);
    let trimmed = normalized.trim_end_matches('\n');

    if !looks_like_outline(trimmed) {
        return anchors::paste_plain_text(workspace, hlc, anchor, trimmed);
    }

    let parsed = outl_md::parse::parse(trimmed);
    if parsed.blocks.is_empty() {
        // Heuristic said outline but parser disagreed (mangled input).
        // Fall back to plain-text behaviour so we never lose user input.
        return anchors::paste_plain_text(workspace, hlc, anchor, trimmed);
    }

    match anchor {
        PasteAnchor::AsLastChildOf(parent) => {
            anchors::paste_as_children(workspace, hlc, parent, &parsed.blocks)
        }
        PasteAnchor::AfterBlock(after) => {
            anchors::paste_after(workspace, hlc, after, &parsed.blocks)
        }
        PasteAnchor::AtCaret { block, caret } => {
            anchors::paste_at_caret(workspace, hlc, block, caret, &parsed.blocks)
        }
    }
}

/// True when at least one non-blank line starts with `- ` or is just `-`.
fn looks_like_outline(s: &str) -> bool {
    s.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed == "-" || trimmed.starts_with("- ")
    })
}

/// Flatten a `BlockTreeOutcome` forest into the ids it minted, in
/// DFS / sibling order.
pub(crate) fn collect_ids(outcomes: &[BlockTreeOutcome]) -> Vec<NodeId> {
    let mut out = Vec::new();
    for o in outcomes {
        push_ids(&mut out, o);
    }
    out
}

pub(crate) fn push_ids(out: &mut Vec<NodeId>, o: &BlockTreeOutcome) {
    out.push(o.id);
    for c in &o.children {
        push_ids(out, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::append_block;
    use outl_core::id::ActorId;

    fn ws() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    #[test]
    fn outline_detector_true_on_bullet_lines() {
        assert!(looks_like_outline("- foo"));
        assert!(looks_like_outline("  - nested"));
        assert!(looks_like_outline("preface\n- bullet"));
    }

    #[test]
    fn outline_detector_false_on_plain_text() {
        assert!(!looks_like_outline("just words"));
        assert!(!looks_like_outline("multi\nline\ntext"));
        assert!(!looks_like_outline(""));
    }

    #[test]
    fn paste_as_last_child_of_root() {
        let (mut workspace, hlc) = ws();
        let parent = append_block(&mut workspace, &hlc, None, Some("host")).unwrap();
        let out = paste_markdown(
            &mut workspace,
            &hlc,
            PasteAnchor::AsLastChildOf(parent),
            "- one\n- two\n- three",
        )
        .unwrap();
        assert_eq!(out.root_count, 3);
        let kids: Vec<String> = crate::tree::children_of(&workspace, parent)
            .into_iter()
            .map(|(id, _)| workspace.block_text(id).unwrap_or_default())
            .collect();
        assert_eq!(kids, vec!["one", "two", "three"]);
    }

    #[test]
    fn paste_after_root_block() {
        let (mut workspace, hlc) = ws();
        let a = append_block(&mut workspace, &hlc, None, Some("a")).unwrap();
        let _z = append_block(&mut workspace, &hlc, None, Some("z")).unwrap();
        let _ = paste_markdown(
            &mut workspace,
            &hlc,
            PasteAnchor::AfterBlock(a),
            "- one\n- two",
        )
        .unwrap();
        let order: Vec<String> = crate::tree::children_of(&workspace, NodeId::root())
            .into_iter()
            .map(|(id, _)| workspace.block_text(id).unwrap_or_default())
            .collect();
        assert_eq!(order, vec!["a", "one", "two", "z"]);
    }

    #[test]
    fn paste_at_caret_splits_and_appends_tail() {
        let (mut workspace, hlc) = ws();
        let host = append_block(&mut workspace, &hlc, None, Some("olá mundo")).unwrap();
        // caret = 4 → after "olá " (4 chars including the space).
        let out = paste_markdown(
            &mut workspace,
            &hlc,
            PasteAnchor::AtCaret {
                block: host,
                caret: 4,
            },
            "- um\n- dois",
        )
        .unwrap();
        assert_eq!(workspace.block_text(host).as_deref(), Some("olá um"));
        let order: Vec<String> = crate::tree::children_of(&workspace, NodeId::root())
            .into_iter()
            .map(|(id, _)| workspace.block_text(id).unwrap_or_default())
            .collect();
        assert_eq!(order, vec!["olá um", "dois", "mundo"]);
        assert_eq!(out.host_text.as_deref(), Some("olá um"));
        // 2 new sibling blocks created ("dois", "mundo"). "um" was
        // merged into the host so it isn't in new_blocks.
        assert_eq!(out.new_blocks.len(), 2);
    }

    #[test]
    fn paste_at_caret_with_plain_text_is_a_splice() {
        let (mut workspace, hlc) = ws();
        let host = append_block(&mut workspace, &hlc, None, Some("hello world")).unwrap();
        let out = paste_markdown(
            &mut workspace,
            &hlc,
            PasteAnchor::AtCaret {
                block: host,
                caret: 6,
            },
            "BRAVE ",
        )
        .unwrap();
        assert_eq!(
            workspace.block_text(host).as_deref(),
            Some("hello BRAVE world")
        );
        assert!(out.new_blocks.is_empty());
    }

    #[test]
    fn paste_empty_input_is_noop() {
        let (mut workspace, hlc) = ws();
        let host = append_block(&mut workspace, &hlc, None, Some("hi")).unwrap();
        let out = paste_markdown(
            &mut workspace,
            &hlc,
            PasteAnchor::AtCaret {
                block: host,
                caret: 2,
            },
            "",
        )
        .unwrap();
        assert!(out.new_blocks.is_empty());
        // Splice of empty into "hi" is still "hi".
        assert_eq!(workspace.block_text(host).as_deref(), Some("hi"));
    }

    #[test]
    fn paste_preserves_nested_children() {
        let (mut workspace, hlc) = ws();
        let parent = append_block(&mut workspace, &hlc, None, Some("p")).unwrap();
        let _ = paste_markdown(
            &mut workspace,
            &hlc,
            PasteAnchor::AsLastChildOf(parent),
            "- a\n  - a1\n  - a2\n- b",
        )
        .unwrap();
        let kids: Vec<(String, Vec<String>)> = crate::tree::children_of(&workspace, parent)
            .into_iter()
            .map(|(id, _)| {
                let grand: Vec<String> = crate::tree::children_of(&workspace, id)
                    .into_iter()
                    .map(|(gid, _)| workspace.block_text(gid).unwrap_or_default())
                    .collect();
                (workspace.block_text(id).unwrap_or_default(), grand)
            })
            .collect();
        assert_eq!(
            kids,
            vec![
                ("a".to_string(), vec!["a1".to_string(), "a2".to_string()]),
                ("b".to_string(), Vec::new()),
            ]
        );
    }

    #[test]
    fn paste_applies_block_properties() {
        use outl_core::property::PropValue;
        let (mut workspace, hlc) = ws();
        let parent = append_block(&mut workspace, &hlc, None, Some("p")).unwrap();
        let _ = paste_markdown(
            &mut workspace,
            &hlc,
            PasteAnchor::AsLastChildOf(parent),
            "- header\n  priority:: high\n  - child",
        )
        .unwrap();
        let kids = crate::tree::children_of(&workspace, parent);
        assert_eq!(kids.len(), 1);
        let header_id = kids[0].0;
        let prop = workspace.tree().property(header_id, "priority");
        match prop {
            Some(PropValue::Text(v)) => assert_eq!(v, "high"),
            other => panic!("expected Text(\"high\"), got {other:?}"),
        }
    }

    #[test]
    fn paste_user_prompt_fixture() {
        // The literal markdown the user pasted in the prompt that
        // motivated this feature. The exact same string must produce
        // the expected tree on both clients.
        let raw = "- #LinkedIn #hot-take. Draft\n    - **Tema:** Can the stockmarket swallow Anthropic, SpaceX and OpenAI? ([hackernews](https://www.economist.com/finance-and-economics/2026/06/01/can-the-stockmarket-swallow-anthropic-spacex-and-openai))\n    - **Score:** 368 pontos, 641 comentários (hackernews)\n    - No mesmo dia, The Economist pergunta se o mercado público consegue engolir Anthropic, OpenAI e SpaceX juntas, Alphabet anuncia equity raise de $80 bi pra AI infra, e Groq abre rodada nova antes da última fechar.\n    - As três privadas somam mais de $1 trilhão em valuation. Capex de AI cresce mais rápido que revenue de qualquer um dos players.\n    - {{[[TODO]]}} revisar antes de postar\n";

        let (mut workspace, hlc) = ws();
        let _ = paste_markdown(
            &mut workspace,
            &hlc,
            PasteAnchor::AsLastChildOf(NodeId::root()),
            raw,
        )
        .unwrap();

        // Exactly one root block — the LinkedIn draft header.
        let roots = crate::tree::children_of(&workspace, NodeId::root());
        assert_eq!(roots.len(), 1, "expected 1 root block, got {}", roots.len());
        let header_id = roots[0].0;
        assert_eq!(
            workspace.block_text(header_id).as_deref(),
            Some("#LinkedIn #hot-take. Draft")
        );

        // Five children, the last one converted from {{[[TODO]]}}.
        let kids = crate::tree::children_of(&workspace, header_id);
        assert_eq!(kids.len(), 5);
        let last_text = workspace.block_text(kids[4].0).unwrap_or_default();
        assert_eq!(last_text, "TODO revisar antes de postar");
    }
}
