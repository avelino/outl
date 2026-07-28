//! End-to-end: Roam JSON backup → workspace with real `((blk-XXXXXX))`
//! handles, embeds, collapsed ops, and a faithful report.

mod common;

use common::{import_with, import_with_opts, open_test_ws, read, TestWs};
use outl_import::adapters::RoamAdapter;
use outl_import::{dry_run, ImportOptions, ImportReport};
use std::fs;

const FIXTURE: &str = r#"[
    {
        "title": "Source",
        "children": [
            {"string": "the original decision", "uid": "src-uid", "children": [
                {"string": "supporting detail", "uid": "child-uid", "children": [], "open": false}
            ]}
        ]
    },
    {
        "title": "Referrer",
        "children": [
            {"string": "see ((src-uid)) please", "uid": "r1", "children": []},
            {"string": "context: {{[[embed]]: ((src-uid))}}", "uid": "r2", "children": []},
            {"string": "dangling ((nope-uid)) ref", "uid": "r3", "children": []},
            {"string": "Done!((src-uid))", "uid": "r4", "children": []},
            {"string": "negociar: {{embed: ((src-uid))}}\ncollapsed:: true\nid:: 6908fc01-aaaa", "uid": "r5", "children": []}
        ]
    },
    {
        "title": "May 25th, 2026",
        "children": [
            {"string": "{{[[TODO]]}} review #[[My Project]] __soon__", "uid": "j1", "children": []}
        ]
    }
]"#;

fn import_fixture(json: &str) -> (TestWs, ImportReport) {
    let src_dir = tempfile::tempdir().expect("src tempdir");
    let src = src_dir.path().join("backup.json");
    fs::write(&src, json).expect("write fixture");
    import_with(&RoamAdapter, &src)
}

#[test]
fn refs_and_embeds_resolve_to_real_handles() {
    let (ws, report) = import_fixture(FIXTURE);

    let referrer = read(&ws.root.join("pages/referrer.md"));
    assert!(
        !referrer.contains("outl-import:"),
        "placeholders must not survive:\n{referrer}"
    );
    assert!(
        referrer.contains("see ((blk-"),
        "block ref not resolved to a handle:\n{referrer}"
    );
    assert!(
        referrer.contains("context: !((blk-"),
        "embed not resolved to a handle:\n{referrer}"
    );
    assert!(
        referrer.contains("((unresolved:nope-uid))"),
        "unknown uid must stay greppable:\n{referrer}"
    );

    // The handle written in referrer.md is exactly the source block's
    // sidecar handle.
    let sc = outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(
        &ws.root.join("pages/source.md"),
    ))
    .expect("source sidecar");
    let handle = &sc.blocks[0].ref_handle;
    assert!(
        referrer.contains(&format!("(({handle}))")),
        "referrer should point at {handle}:\n{referrer}"
    );

    // Regression: user text ending in `!` glued to a ref must stay a
    // REFERENCE — `!((blk-…))` is outl's embed syntax, so the resolve
    // pass separates them with a space instead of misclassifying.
    assert!(
        referrer.contains("Done! ((blk-"),
        "`!`-adjacent ref must not become an embed:\n{referrer}"
    );

    // Regression: a Roam block whose text carries embedded `key:: value`
    // lines (Logseq residue pasted into Roam) has those lines lifted
    // into block PROPERTIES by outl's parser — the stored text differs
    // from the rendered continuation lines. The resolve pass must
    // still hash-match (texts come from the same parser now) and
    // rewrite the embed instead of leaving the placeholder behind.
    assert!(
        referrer.contains("negociar: !((blk-"),
        "placeholder must resolve even when prop-like lines were lifted:\n{referrer}"
    );

    assert_eq!(report.refs_resolved, 2);
    assert_eq!(report.embeds_resolved, 2);
    assert_eq!(report.refs_unresolved, 1);
}

#[test]
fn collapsed_state_lands_in_the_op_log() {
    let (ws, report) = import_fixture(FIXTURE);
    let sc = outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(
        &ws.root.join("pages/source.md"),
    ))
    .expect("source sidecar");
    // Depth-first: [0] = parent, [1] = "supporting detail" (open: false).
    assert!(ws.workspace.tree().is_collapsed(sc.blocks[1].id));
    assert_eq!(report.collapsed_applied, 1);
}

#[test]
fn journals_and_dialect_translations_land() {
    let (ws, report) = import_fixture(FIXTURE);
    let journal = read(&ws.root.join("journals/2026-05-25.md"));
    assert!(journal.contains("- TODO review [[My Project]] *soon*"));
    assert!(!journal.starts_with("title::"));
    assert_eq!(report.journals, 1);
    assert_eq!(report.pages, 2);
    assert_eq!(report.tasks.get("TODO"), Some(&1));

    let source = read(&ws.root.join("pages/source.md"));
    assert!(source.contains("title:: Source"));
}

#[test]
fn dry_run_writes_nothing_and_predicts_resolution() {
    let src_dir = tempfile::tempdir().expect("tempdir");
    let src = src_dir.path().join("backup.json");
    fs::write(&src, FIXTURE).expect("write fixture");

    let report = dry_run(&RoamAdapter, &src, &ImportOptions::default()).expect("dry run");
    assert_eq!(report.refs_resolved, 2);
    assert_eq!(report.embeds_resolved, 2);
    assert_eq!(report.refs_unresolved, 1);
    assert_eq!(report.pages, 2);
    assert_eq!(report.journals, 1);
    assert_eq!(report.blocks, 8);
}

#[test]
fn unmappable_page_placeholders_degrade_via_file_fallback() {
    // The second block's string embeds a `- ` line, which outl's parser
    // SPLITS into a child block — the page's block count diverges from
    // the renderer's walk, so the whole page is unmappable for handle
    // wiring. The ref on it must still degrade to a `[[Title]]` link
    // (file fallback), never survive as a literal placeholder.
    let json = r#"[
        {"title": "Source", "children": [
            {"string": "the original", "uid": "src-uid", "children": []}
        ]},
        {"title": "Tricky", "children": [
            {"string": "see ((src-uid)) here", "uid": "t1", "children": []},
            {"string": "pasted text\n- embedded bullet line", "uid": "t2", "children": []}
        ]}
    ]"#;
    let (ws, report) = import_fixture(json);
    let tricky = read(&ws.root.join("pages/tricky.md"));
    assert!(
        !tricky.contains("outl-import"),
        "no placeholder marker may survive on disk:\n{tricky}"
    );
    assert!(
        tricky.contains("see [[Source]] here"),
        "unmappable ref must degrade to a page link:\n{tricky}"
    );
    assert!(report.refs_page_fallback >= 1);
}

#[test]
fn markers_in_prop_values_and_post_prop_lines_never_survive() {
    // The Omnivore-integration shape: a multiline quote block whose
    // embedded lines the parser lifts into block PROPERTIES. The
    // `((uid))` then lives in a prop VALUE (never in any block text),
    // and a bare `((uid))` line after a prop line isn't in the AST at
    // all — both invisible to the block-level resolve path. The
    // file-fallback sweep must still erase every marker.
    let json = r#"[
        {"title": "Target", "children": [
            {"string": "the referenced post", "uid": "LX2n3H5HX", "children": []}
        ]},
        {"title": "omnivore-saved", "children": [
            {"string": "> quote one [link](https://x.com) \n\nnote:: ((missing-uid))\ndate-highlighted:: [[2024-04-25]]", "uid": "g1", "children": []},
            {"string": "> quote two [link](https://y.com) \n\nnote:: esse post esta linkado com\n((LX2n3H5HX))", "uid": "u1", "children": []}
        ]}
    ]"#;
    let (ws, report) = import_fixture(json);
    let page = read(&ws.root.join("pages/omnivore-saved.md"));
    assert!(
        !page.contains("outl-import"),
        "no marker may survive, prop values included:\n{page}"
    );
    assert!(
        page.contains("note:: ((unresolved:missing-uid))"),
        "unknown uid in a prop value stays greppable:\n{page}"
    );
    assert!(
        page.contains("[[Target]]"),
        "known uid degrades to a page link:\n{page}"
    );
    assert_eq!(report.refs_unresolved, 1);
    assert!(report.refs_page_fallback >= 1);
}

#[test]
fn slug_collision_gets_suffixed() {
    let json = r#"[
        {"title": "Foo Bar", "children": [{"string": "a", "uid": "a1", "children": []}]},
        {"title": "foo bar", "children": [{"string": "b", "uid": "b1", "children": []}]}
    ]"#;
    let (ws, report) = import_fixture(json);
    assert!(ws.root.join("pages/foo-bar.md").exists());
    assert!(ws.root.join("pages/foo-bar-2.md").exists());
    assert_eq!(report.slug_collisions, 1);
}

#[test]
fn timestamps_dropped_by_default_kept_on_opt_in() {
    let json = r#"[
        {"title": "Stamped", "children": [
            {"string": "x", "uid": "s1", "children": [], "create-time": 1700000000000}
        ]}
    ]"#;
    let (_ws, report) = import_fixture(json);
    assert_eq!(report.timestamps_dropped, 1);

    let src_dir = tempfile::tempdir().expect("tempdir");
    let src = src_dir.path().join("backup.json");
    fs::write(&src, json).expect("write fixture");
    let opts = ImportOptions {
        preserve_timestamps: true,
    };
    let (ws, report) = import_with_opts(&RoamAdapter, &src, &opts);
    assert_eq!(report.timestamps_dropped, 0);
    let page = read(&ws.root.join("pages/stamped.md"));
    assert!(page.contains("created:: 2023-11-14T"), "page:\n{page}");

    let _ = open_test_ws(); // keep harness helpers exercised
}
