//! End-to-end tests for `WorkspaceIndex` (page + block index).
//!
//! Moved out of `src/index.rs` to keep that module under the
//! file-size-guard limit. Every test here exercises only the public
//! API surface (build / patch_page / remove_page / lookups), which is
//! the contract any UI surface — TUI, Tauri, mobile — talks to.
//!
//! **Backlink tests live in `outl-actions::backlinks`.** This index
//! used to carry a parallel backlinks cache; it was removed because
//! it duplicated `outl_actions::backlinks_for_page` and drifted in
//! policy (self-references behaved differently on TUI vs mobile).

use outl_core::id::NodeId;
use outl_md::index::WorkspaceIndex;
use outl_md::parse::parse;
use outl_md::sidecar::{
    self, content_hash, derive_ref_handle, file_hash, write as write_sidecar, Sidecar, SidecarBlock,
};
use std::fs;
use tempfile::TempDir;

fn write_workspace(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().unwrap();
    for (rel, content) in files {
        let full = dir.path().join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, content).unwrap();
    }
    dir
}

#[test]
fn patch_page_updates_title_and_icon() {
    let dir = write_workspace(&[("pages/x.md", "title:: Old Title\nicon:: 🦀\n\n- body\n")]);
    let mut idx = WorkspaceIndex::build(dir.path());
    assert_eq!(idx.by_slug("x").unwrap().title, "Old Title");
    assert_eq!(idx.by_slug("x").unwrap().icon.as_deref(), Some("🦀"));

    let new_page = parse("title:: New Title\nicon:: 🚀\n\n- body\n");
    idx.patch_page(&dir.path().join("pages/x.md"), &new_page);

    let entry = idx.by_slug("x").unwrap();
    assert_eq!(entry.title, "New Title");
    assert_eq!(entry.icon.as_deref(), Some("🚀"));
    assert!(idx.by_title("Old Title").is_none());
    assert_eq!(idx.by_title("New Title").unwrap().slug, "x");
}

#[test]
fn remove_page_drops_entry_and_title_alias() {
    let dir = write_workspace(&[
        ("pages/avelino.md", "title:: Avelino\n\n- author\n"),
        (
            "pages/projeto.md",
            "title:: Projeto\n\n- led by [[Avelino]]\n",
        ),
    ]);
    let mut idx = WorkspaceIndex::build(dir.path());
    assert!(idx.by_slug("projeto").is_some());
    assert_eq!(idx.by_title("Projeto").unwrap().slug, "projeto");

    idx.remove_page("projeto");
    assert!(idx.by_slug("projeto").is_none());
    assert!(idx.by_title("Projeto").is_none());
    // `avelino` page entry is independent — it survives.
    assert!(idx.by_slug("avelino").is_some());
}

#[test]
fn empty_workspace_indexes_to_nothing() {
    let dir = TempDir::new().unwrap();
    let idx = WorkspaceIndex::build(dir.path());
    assert_eq!(idx.page_count(), 0);
    assert_eq!(idx.block_count(), 0);
}

#[test]
fn build_populates_block_index_from_sidecar() {
    let dir = write_workspace(&[("pages/p.md", "- decide backend\n")]);
    let page_path = dir.path().join("pages/p.md");

    let page_id = NodeId::new();
    let block_id = NodeId::new();
    let mut sc = Sidecar::new_for_page(page_id, &file_hash("- decide backend\n"));
    sc.blocks.push(SidecarBlock {
        id: block_id,
        line: 1,
        indent: 0,
        content_hash: content_hash("decide backend"),
        ref_handle: derive_ref_handle(block_id),
    });
    write_sidecar(&sidecar::sidecar_path_for(&page_path), &sc).unwrap();

    let idx = WorkspaceIndex::build(dir.path());
    assert_eq!(idx.block_count(), 1);

    let handle = derive_ref_handle(block_id);
    let entry = idx
        .resolve_block_ref(&handle)
        .expect("block ref must resolve");
    assert_eq!(entry.id, block_id);
    assert_eq!(entry.text, "decide backend");
    assert_eq!(entry.source_slug, "p");
}

#[test]
fn patch_page_refreshes_block_index() {
    let dir = write_workspace(&[("pages/p.md", "- one\n")]);
    let page_path = dir.path().join("pages/p.md");
    let page_id = NodeId::new();
    let id_one = NodeId::new();

    let mut sc = Sidecar::new_for_page(page_id, &file_hash("- one\n"));
    sc.blocks.push(SidecarBlock {
        id: id_one,
        line: 1,
        indent: 0,
        content_hash: content_hash("one"),
        ref_handle: derive_ref_handle(id_one),
    });
    write_sidecar(&sidecar::sidecar_path_for(&page_path), &sc).unwrap();

    let mut idx = WorkspaceIndex::build(dir.path());
    assert_eq!(idx.block_count(), 1);

    fs::write(&page_path, "- one\n- two\n").unwrap();
    let id_two = NodeId::new();
    sc.blocks.push(SidecarBlock {
        id: id_two,
        line: 2,
        indent: 0,
        content_hash: content_hash("two"),
        ref_handle: derive_ref_handle(id_two),
    });
    sc.last_synced_hash = file_hash("- one\n- two\n");
    write_sidecar(&sidecar::sidecar_path_for(&page_path), &sc).unwrap();

    let new_page = parse("- one\n- two\n");
    idx.patch_page(&page_path, &new_page);

    assert_eq!(idx.block_count(), 2);
    assert!(idx.resolve_block_ref(&derive_ref_handle(id_two)).is_some());
}

#[test]
fn patch_page_preserves_cross_page_reverse_refs() {
    // Regression guard from the code review: in-process edits to a
    // citing page (B) used to invalidate the reverse-ref bookkeeping
    // for the target page (A). After patch_page, A's reverse-ref
    // list must still include B's new citing block.
    let dir = write_workspace(&[
        ("pages/a.md", "- target block\n"),
        ("pages/b.md", "- placeholder\n"),
    ]);
    let a_path = dir.path().join("pages/a.md");
    let b_path = dir.path().join("pages/b.md");

    // Sidecar A: one target block.
    let id_target = NodeId::new();
    let mut sc_a = Sidecar::new_for_page(NodeId::new(), &file_hash("- target block\n"));
    sc_a.blocks.push(SidecarBlock {
        id: id_target,
        line: 1,
        indent: 0,
        content_hash: content_hash("target block"),
        ref_handle: derive_ref_handle(id_target),
    });
    write_sidecar(&sidecar::sidecar_path_for(&a_path), &sc_a).unwrap();

    // Sidecar B: one placeholder (no ref yet).
    let id_placeholder = NodeId::new();
    let mut sc_b = Sidecar::new_for_page(NodeId::new(), &file_hash("- placeholder\n"));
    sc_b.blocks.push(SidecarBlock {
        id: id_placeholder,
        line: 1,
        indent: 0,
        content_hash: content_hash("placeholder"),
        ref_handle: derive_ref_handle(id_placeholder),
    });
    write_sidecar(&sidecar::sidecar_path_for(&b_path), &sc_b).unwrap();

    let mut idx = WorkspaceIndex::build(dir.path());
    assert!(idx.block_refs_to(id_target).is_empty(), "no refs yet");

    // Simulate B saving with a new citing block.
    let target_handle = derive_ref_handle(id_target);
    let b_md_after = format!("- placeholder\n- see (({target_handle})) here\n");
    fs::write(&b_path, &b_md_after).unwrap();
    let id_cite = NodeId::new();
    sc_b.blocks.push(SidecarBlock {
        id: id_cite,
        line: 2,
        indent: 0,
        content_hash: content_hash(&format!("see (({target_handle})) here")),
        ref_handle: derive_ref_handle(id_cite),
    });
    sc_b.last_synced_hash = file_hash(&b_md_after);
    write_sidecar(&sidecar::sidecar_path_for(&b_path), &sc_b).unwrap();
    idx.patch_page(&b_path, &parse(&b_md_after));

    let refs = idx.block_refs_to(id_target);
    assert_eq!(
        refs.len(),
        1,
        "B's new citing block must show in A's reverse refs"
    );
    assert_eq!(refs[0].source_slug, "b");
}

#[test]
fn block_at_location_returns_none_for_unknown_path() {
    // Sanity guard for the O(1) location lookup: an out-of-range
    // DFS path (deeper than the AST, or pointing at a sibling that
    // doesn't exist) returns None rather than panicking.
    let dir = write_workspace(&[("pages/p.md", "- alpha\n")]);
    let page_path = dir.path().join("pages/p.md");
    let id = NodeId::new();
    let mut sc = Sidecar::new_for_page(NodeId::new(), &file_hash("- alpha\n"));
    sc.blocks.push(SidecarBlock {
        id,
        line: 1,
        indent: 0,
        content_hash: content_hash("alpha"),
        ref_handle: derive_ref_handle(id),
    });
    write_sidecar(&sidecar::sidecar_path_for(&page_path), &sc).unwrap();

    let idx = WorkspaceIndex::build(dir.path());
    assert!(
        idx.block_at_location("p", &[0]).is_some(),
        "real path resolves"
    );
    assert!(
        idx.block_at_location("p", &[0, 7]).is_none(),
        "deep path → None"
    );
    assert!(
        idx.block_at_location("nope", &[0]).is_none(),
        "unknown slug → None"
    );
}

#[test]
fn remove_page_drops_block_entries_too() {
    let dir = write_workspace(&[("pages/p.md", "- alpha\n")]);
    let page_path = dir.path().join("pages/p.md");
    let page_id = NodeId::new();
    let block_id = NodeId::new();

    let mut sc = Sidecar::new_for_page(page_id, &file_hash("- alpha\n"));
    sc.blocks.push(SidecarBlock {
        id: block_id,
        line: 1,
        indent: 0,
        content_hash: content_hash("alpha"),
        ref_handle: derive_ref_handle(block_id),
    });
    write_sidecar(&sidecar::sidecar_path_for(&page_path), &sc).unwrap();

    let mut idx = WorkspaceIndex::build(dir.path());
    assert_eq!(idx.block_count(), 1);
    idx.remove_page("p");
    assert_eq!(idx.block_count(), 0);
    assert!(idx
        .resolve_block_ref(&derive_ref_handle(block_id))
        .is_none());
}

#[test]
fn pages_get_indexed_by_slug_and_title() {
    let dir = write_workspace(&[
        (
            "pages/avelino.md",
            "title:: Avelino\n\n- some note about me\n",
        ),
        ("pages/projeto.md", "title:: Meu Projeto\n\n- objetivo\n"),
    ]);
    let idx = WorkspaceIndex::build(dir.path());
    assert_eq!(idx.page_count(), 2);
    assert_eq!(idx.by_slug("avelino").unwrap().title, "Avelino");
    assert_eq!(idx.by_title("Meu Projeto").unwrap().slug, "projeto");
}

#[test]
fn missing_title_falls_back_to_slug() {
    let dir = write_workspace(&[("pages/no-title.md", "- bare bullet\n")]);
    let idx = WorkspaceIndex::build(dir.path());
    assert_eq!(idx.by_slug("no-title").unwrap().title, "no-title");
}

#[test]
fn icon_property_lands_on_the_page_entry() {
    let dir = write_workspace(&[
        (
            "pages/avelino.md",
            "title:: Avelino\nicon:: 🦀\n\n- author\n",
        ),
        (
            "pages/projeto.md",
            "title:: Projeto\nicon:: 🚀\n\n- led by [[Avelino]]\n",
        ),
        ("pages/bare.md", "title:: Bare\n\n- nothing fancy\n"),
    ]);
    let idx = WorkspaceIndex::build(dir.path());

    assert_eq!(idx.by_slug("avelino").unwrap().icon.as_deref(), Some("🦀"));
    assert_eq!(idx.by_slug("projeto").unwrap().icon.as_deref(), Some("🚀"));
    assert_eq!(idx.by_slug("bare").unwrap().icon, None);
}

#[test]
fn empty_icon_is_treated_as_none() {
    let dir = write_workspace(&[("pages/x.md", "title:: X\nicon::\n\n- body\n")]);
    let idx = WorkspaceIndex::build(dir.path());
    assert_eq!(idx.by_slug("x").unwrap().icon, None);
}

#[test]
fn journals_are_treated_as_pages_for_lookup() {
    let dir = write_workspace(&[("journals/2026-05-24.md", "- entry\n")]);
    let idx = WorkspaceIndex::build(dir.path());
    let entry = idx.by_slug("2026-05-24").unwrap();
    assert!(entry.is_journal);
}

#[test]
fn title_prefix_lookup() {
    let dir = write_workspace(&[
        ("pages/a.md", "title:: Apple\n\n- a\n"),
        ("pages/b.md", "title:: Apricot\n\n- a\n"),
        ("pages/c.md", "title:: Banana\n\n- a\n"),
    ]);
    let idx = WorkspaceIndex::build(dir.path());
    let hits = idx.pages_by_title_prefix("Ap", 10);
    assert_eq!(hits.len(), 2);
    let names: Vec<_> = hits.iter().map(|p| p.title.as_str()).collect();
    assert!(names.contains(&"Apple"));
    assert!(names.contains(&"Apricot"));
}

#[test]
fn page_type_property_lands_on_the_page_entry() {
    let dir = write_workspace(&[
        (
            "pages/avelino.md",
            "title:: Avelino\ntype:: person\n\n- author\n",
        ),
        (
            "pages/projeto.md",
            "title:: Projeto\ntype:: project\n\n- shipped\n",
        ),
        ("pages/bare.md", "title:: Bare\n\n- nothing fancy\n"),
    ]);
    let idx = WorkspaceIndex::build(dir.path());

    assert_eq!(
        idx.by_slug("avelino").unwrap().page_type.as_deref(),
        Some("person")
    );
    assert_eq!(
        idx.by_slug("projeto").unwrap().page_type.as_deref(),
        Some("project")
    );
    assert_eq!(idx.by_slug("bare").unwrap().page_type, None);
}

#[test]
fn page_type_is_lowercased_on_index() {
    let dir = write_workspace(&[(
        "pages/avelino.md",
        "title:: Avelino\ntype::   Person\n\n- author\n",
    )]);
    let idx = WorkspaceIndex::build(dir.path());
    assert_eq!(
        idx.by_slug("avelino").unwrap().page_type.as_deref(),
        Some("person")
    );
}

#[test]
fn empty_page_type_is_treated_as_none() {
    let dir = write_workspace(&[("pages/x.md", "title:: X\ntype::\n\n- body\n")]);
    let idx = WorkspaceIndex::build(dir.path());
    assert_eq!(idx.by_slug("x").unwrap().page_type, None);
}

#[test]
fn pages_by_type_filters_to_matching_pages() {
    let dir = write_workspace(&[
        (
            "pages/avelino.md",
            "title:: Avelino\ntype:: person\n\n- a\n",
        ),
        ("pages/maria.md", "title:: Maria\ntype:: PERSON\n\n- a\n"),
        (
            "pages/projeto.md",
            "title:: Projeto\ntype:: project\n\n- a\n",
        ),
        ("pages/bare.md", "title:: Bare\n\n- a\n"),
    ]);
    let idx = WorkspaceIndex::build(dir.path());

    let persons: Vec<&str> = idx
        .pages_by_type("person")
        .map(|p| p.title.as_str())
        .collect();
    assert_eq!(
        persons.len(),
        2,
        "expected Avelino + Maria, got {persons:?}"
    );
    assert!(persons.contains(&"Avelino"));
    assert!(persons.contains(&"Maria"));

    // Case-insensitive needle.
    let persons_upper: Vec<&str> = idx
        .pages_by_type("PERSON")
        .map(|p| p.title.as_str())
        .collect();
    assert_eq!(persons_upper.len(), 2);

    // Different type doesn't match.
    let projects: Vec<&str> = idx
        .pages_by_type("project")
        .map(|p| p.title.as_str())
        .collect();
    assert_eq!(projects, vec!["Projeto"]);

    // Unknown type returns nothing.
    assert_eq!(idx.pages_by_type("ghost").count(), 0);
}

#[test]
fn patch_page_refreshes_page_type() {
    let dir = write_workspace(&[("pages/x.md", "title:: X\n\n- body\n")]);
    let mut idx = WorkspaceIndex::build(dir.path());
    assert_eq!(idx.by_slug("x").unwrap().page_type, None);

    let path = dir.path().join("pages/x.md");
    fs::write(&path, "title:: X\ntype:: person\n\n- body\n").unwrap();
    let parsed = parse(&fs::read_to_string(&path).unwrap());
    idx.patch_page(&path, &parsed);

    assert_eq!(
        idx.by_slug("x").unwrap().page_type.as_deref(),
        Some("person")
    );
}
