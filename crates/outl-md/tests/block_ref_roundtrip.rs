//! End-to-end roundtrip for `((blk-XXXXXX))` block references.
//!
//! Three contracts under test:
//!
//! 1. **Markdown stability** — `((blk-xxx))` survives parse + render
//!    without normalization. The renderer is text-based, so the only
//!    risk is a future parser change that mangles inline tokens. This
//!    test pins it.
//! 2. **Index resolution** — a real workspace with two pages where one
//!    cites a block of the other resolves through `WorkspaceIndex`.
//! 3. **Edit preservation** — editing the *citing* page (not the
//!    target) leaves the cited handle valid. Catches a regression
//!    where `patch_page` would forget a foreign block by mistake.

use outl_core::id::NodeId;
use outl_md::index::WorkspaceIndex;
use outl_md::parse::parse;
use outl_md::render::render;
use outl_md::sidecar::{
    self, content_hash, derive_ref_handle, file_hash, write as write_sidecar, Sidecar, SidecarBlock,
};
use std::fs;
use tempfile::TempDir;

fn write_page_with_sidecar(
    dir: &TempDir,
    rel: &str,
    md: &str,
    blocks: &[(NodeId, &str, usize, u32)],
) {
    let full = dir.path().join(rel);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&full, md).unwrap();

    let page_id = NodeId::new();
    let mut sc = Sidecar::new_for_page(page_id, &file_hash(md));
    for (id, text, line, indent) in blocks {
        sc.blocks.push(SidecarBlock {
            id: *id,
            line: *line,
            indent: *indent,
            content_hash: content_hash(text),
            ref_handle: derive_ref_handle(*id),
        });
    }
    write_sidecar(&sidecar::sidecar_path_for(&full), &sc).unwrap();
}

#[test]
fn block_ref_roundtrips_through_parse_and_render() {
    let md = "- decide backend\n- see ((blk-r6s4a1)) for context\n";
    let ast = parse(md);
    let rendered = render(&ast);
    assert_eq!(
        rendered, md,
        "((blk-...)) must survive parse + render byte-for-byte"
    );
}

#[test]
fn block_ref_resolves_across_pages_after_index_build() {
    let dir = TempDir::new().unwrap();
    let id_target = NodeId::new();
    let target_handle = derive_ref_handle(id_target);

    // Page A defines the target block.
    write_page_with_sidecar(
        &dir,
        "pages/a.md",
        "- decide backend\n",
        &[(id_target, "decide backend", 1, 0)],
    );

    // Page B cites it inline.
    let b_md = format!("- see (({target_handle})) for context\n");
    let id_cite = NodeId::new();
    write_page_with_sidecar(
        &dir,
        "pages/b.md",
        &b_md,
        &[(
            id_cite,
            &format!("see (({target_handle})) for context"),
            1,
            0,
        )],
    );

    let idx = WorkspaceIndex::build(dir.path());
    let resolved = idx
        .resolve_block_ref(&target_handle)
        .expect("citing-page reference must resolve to a's target");
    assert_eq!(resolved.id, id_target);
    assert_eq!(resolved.text, "decide backend");

    // And the reverse edge is registered.
    let reverse = idx.block_refs_to(id_target);
    assert_eq!(reverse.len(), 1);
    assert_eq!(reverse[0].source_slug, "b");
}

#[test]
fn editing_citing_page_leaves_cited_handle_valid() {
    // Regression guard: patch_page used to drop *all* blocks for the
    // patched slug, including those whose ids belonged to other pages
    // that happened to share a handle path. forget_page must scope
    // strictly by source_slug.
    let dir = TempDir::new().unwrap();
    let id_target = NodeId::new();
    let target_handle = derive_ref_handle(id_target);

    write_page_with_sidecar(
        &dir,
        "pages/a.md",
        "- decide backend\n",
        &[(id_target, "decide backend", 1, 0)],
    );

    let b_md = format!("- see (({target_handle})) for context\n");
    let id_cite = NodeId::new();
    write_page_with_sidecar(
        &dir,
        "pages/b.md",
        &b_md,
        &[(
            id_cite,
            &format!("see (({target_handle})) for context"),
            1,
            0,
        )],
    );

    let mut idx = WorkspaceIndex::build(dir.path());
    assert!(idx.resolve_block_ref(&target_handle).is_some());

    // Re-render b with an additional, unrelated block.
    let b_md2 = format!("- see (({target_handle})) for context\n- and another thought entirely\n");
    let id_extra = NodeId::new();
    let b_path = dir.path().join("pages/b.md");
    fs::write(&b_path, &b_md2).unwrap();
    let mut sc = sidecar::read(&sidecar::sidecar_path_for(&b_path)).unwrap();
    sc.blocks.push(SidecarBlock {
        id: id_extra,
        line: 2,
        indent: 0,
        content_hash: content_hash("and another thought entirely"),
        ref_handle: derive_ref_handle(id_extra),
    });
    sc.last_synced_hash = file_hash(&b_md2);
    write_sidecar(&sidecar::sidecar_path_for(&b_path), &sc).unwrap();

    let new_ast = parse(&b_md2);
    idx.patch_page(&b_path, &new_ast);

    assert!(
        idx.resolve_block_ref(&target_handle).is_some(),
        "patching page b must NOT invalidate page a's block ref"
    );
}

#[test]
fn block_ref_handle_is_stable_across_repeated_index_builds() {
    // Property-ish test: the handle of a given block id is the same
    // regardless of how many times we build the workspace index from
    // scratch. derive_ref_handle is pure, so the entry must agree
    // every run.
    let dir = TempDir::new().unwrap();
    let id = NodeId::new();
    write_page_with_sidecar(&dir, "pages/p.md", "- alpha\n", &[(id, "alpha", 1, 0)]);

    let expected = derive_ref_handle(id);
    for _ in 0..5 {
        let idx = WorkspaceIndex::build(dir.path());
        let e = idx.resolve_block_ref(&expected).unwrap();
        assert_eq!(e.ref_handle, expected);
    }
}
