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

/// A remote image link in a Roam block is scanned into a remote asset
/// ref and, with `--no-assets` (which the harness sets so no download
/// hits the network), the original `![](url)` is kept verbatim.
///
/// The real remote-download path needs network access and is exercised
/// only manually; the ext-derivation + remote-scan units cover its
/// pure pieces (`emit::assets` + `adapters::asset_scan` tests).
#[test]
fn remote_image_without_download_keeps_original_link() {
    let json = r#"[
        {
            "title": "Gallery",
            "children": [
                {"string": "shot ![cover](https://firebasestorage.example/o/img.png?alt=media)", "uid": "g1", "children": []}
            ]
        }
    ]"#;
    let src_dir = tempfile::tempdir().expect("src tempdir");
    let src = src_dir.path().join("backup.json");
    fs::write(&src, json).expect("write fixture");

    let opts = ImportOptions {
        import_assets: false,
        ..Default::default()
    };
    let (ws, report) = import_with_opts(&RoamAdapter, &src, &opts);

    let gallery = read(&ws.root.join("pages/gallery.md"));
    assert!(
        gallery.contains("![cover](https://firebasestorage.example/o/img.png?alt=media)"),
        "original remote link kept with --no-assets:\n{gallery}"
    );
    assert!(
        !gallery.contains("outl-import-asset:"),
        "placeholder must not survive:\n{gallery}"
    );
    // Nothing was pulled (assets off) — neither copied nor missing.
    assert_eq!(report.assets_copied, 0);
    assert_eq!(report.assets_missing, 0);
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
    // Two folds: `child-uid`'s `open: false`, plus `r5`'s inline
    // `collapsed:: true` line — a structural attribute the adapter now
    // maps to the fold flag instead of leaving it as a literal property.
    assert_eq!(report.collapsed_applied, 2);
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
fn roam_page_attributes_land_as_page_properties() {
    // The dominant real-world shape: a contact/company page whose head
    // is Roam attribute blocks (`icon::`, `page-type::`, `work::`).
    // These must reach the `.md` as page properties in the header — the
    // form outl's index reads for the sidebar icon, `page-type` filter,
    // and `@` mention autocomplete — never as stray text bullets.
    let json = r#"[
        {"title": "@Tonico", "children": [
            {"string": "icon:: 👤\npage-type:: contact\nwork:: [[buser]]", "uid": "a1", "children": []},
            {"string": "related:: [[triathlon]]", "uid": "a2", "children": []},
            {"string": "met at the conference", "uid": "n1", "children": []}
        ]}
    ]"#;
    let (ws, report) = import_fixture(json);
    let page = read(&ws.root.join("pages/tonico.md"));
    for line in [
        "icon:: 👤",
        "page-type:: contact",
        "work:: [[buser]]",
        "related:: [[triathlon]]",
    ] {
        assert!(
            page.contains(line),
            "missing page property `{line}`:\n{page}"
        );
    }
    assert!(
        !page.contains("- icon::") && !page.contains("- page-type::"),
        "attribute lines must be page properties, not text bullets:\n{page}"
    );
    assert!(
        page.contains("- met at the conference"),
        "real note dropped:\n{page}"
    );
    assert!(
        report.props_pages >= 4,
        "page props not counted: {}",
        report.props_pages
    );
}

#[test]
fn namespaced_page_keeps_title_and_flattens_slug() {
    // A Roam page named `buser/tech/data` (1000+ of these in a real
    // graph): the title must survive verbatim with its slashes, while
    // the on-disk slug flattens to a single filesystem-safe component.
    // A ref to it keeps the namespaced form and resolves via the slug.
    let json = r#"[
        {"title": "buser/tech/data", "children": [{"string": "the note", "uid": "n1", "children": []}]},
        {"title": "elsewhere", "children": [{"string": "see [[buser/tech/data]] here", "uid": "r1", "children": []}]}
    ]"#;
    let (ws, _report) = import_fixture(json);
    let page = read(&ws.root.join("pages/buser-tech-data.md"));
    assert!(
        page.contains("title:: buser/tech/data"),
        "namespaced title must survive verbatim:\n{page}"
    );
    let referrer = read(&ws.root.join("pages/elsewhere.md"));
    assert!(
        referrer.contains("[[buser/tech/data]]"),
        "namespaced ref must stay verbatim (resolves via slug):\n{referrer}"
    );
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
        ..Default::default()
    };
    let (ws, report) = import_with_opts(&RoamAdapter, &src, &opts);
    assert_eq!(report.timestamps_dropped, 0);
    let page = read(&ws.root.join("pages/stamped.md"));
    assert!(page.contains("created:: 2023-11-14T"), "page:\n{page}");

    let _ = open_test_ws(); // keep harness helpers exercised
}
