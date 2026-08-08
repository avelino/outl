//! The three properties the `.md` pipeline must hold, checked over a
//! corpus of shapes taken from a real workspace.
//!
//! ## Why a corpus and not more unit tests
//!
//! Issue #210 was found in production, not in CI, and every one of the
//! four regressions introduced while fixing it was found by running a
//! throwaway probe over 2,827 real `.md` files — never by the 237 unit
//! tests that were green the whole time. The unit tests check the shapes
//! someone thought of; the corpus checks the shapes that exist.
//!
//! The files in `tests/corpus/` are those shapes, reduced to the
//! smallest input that reproduces each one. They are ugly on purpose:
//! odd indentation, tabs, Roam leftovers, bullets inside fences, unicode
//! separators pasted from PDFs. Every one of them is real.
//!
//! **Add a file here whenever a `.md`-handling bug is found in the
//! wild.** That is the whole maintenance rule.
//!
//! ## The three properties
//!
//! 1. **No content is lost.** Every non-blank line of the source must
//!    still be findable after `parse → render`.
//! 2. **`render → parse` is a fixpoint.** Not merely lossless: a
//!    document that keeps changing shape on each save mutates the user's
//!    file forever, emits an `Op::Edit` every reconcile, and is worse
//!    than the bug it replaced — which at least converged.
//! 3. **The unlogged-content check does not cry wolf.** A page whose
//!    content the log fully accounts for must not be reported as holding
//!    unlogged lines, because that verdict withholds its
//!    `last_synced_hash` and refuses its re-projection — freezing a page
//!    that has nothing wrong with it.
//!
//! Property 3 is the one that would have caught the last defect closed
//! on #210: 8 pages of the reporting workspace reported unlogged content
//! they did not have, purely because a bullet inside a code fence lives
//! in the block's text with its marker while the disk side strips it.

use outl_core::id::NodeId;
use outl_md::parse::parse;
use outl_md::render::render;
use outl_md::sidecar::SidecarBlock;

/// Sidecar blocks as the log would hold them for a freshly parsed page:
/// one entry per block, text verbatim, DFS preorder.
fn blocks_of(md: &str) -> Vec<SidecarBlock> {
    fn walk(nodes: &[outl_md::OutlineNode], out: &mut Vec<SidecarBlock>) {
        for n in nodes {
            out.push(SidecarBlock::from_text(
                NodeId::new(),
                out.len() + 1,
                0,
                &n.text,
            ));
            walk(&n.children, out);
        }
    }
    let mut out = Vec::new();
    walk(&parse(md).blocks, &mut out);
    out
}

fn corpus() -> Vec<(String, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("corpus");
    let mut out: Vec<(String, String)> = std::fs::read_dir(&dir)
        .expect("tests/corpus must exist")
        .flatten()
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("md"))
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let body = std::fs::read_to_string(e.path()).expect("corpus file readable");
            (name, body)
        })
        .collect();
    out.sort();
    assert!(!out.is_empty(), "the corpus must not be empty");
    out
}

#[test]
fn no_corpus_file_loses_a_line() {
    for (name, src) in corpus() {
        let rendered = render(&parse(&src));
        for line in src.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let body = line.strip_prefix("- ").unwrap_or(line);
            assert!(
                rendered.contains(body),
                "{name}: line {body:?} vanished\n--- rendered ---\n{rendered}"
            );
        }
    }
}

#[test]
fn every_corpus_file_is_a_render_parse_fixpoint() {
    for (name, src) in corpus() {
        let once = render(&parse(&src));
        let twice = render(&parse(&once));
        assert_eq!(
            once, twice,
            "{name}: the document changes on every save\n--- pass 1 ---\n{once}\n--- pass 2 ---\n{twice}"
        );
    }
}

#[test]
fn no_corpus_file_reports_content_the_log_already_has() {
    for (name, src) in corpus() {
        let missing = outl_md::unlogged::content_lines_missing_from(&src, &blocks_of(&src));
        assert!(
            missing.is_empty(),
            "{name}: reported {} line(s) as unlogged that the log does hold — \
             this withholds the page's hash and freezes its re-projection: {missing:?}",
            missing.len()
        );
    }
}
