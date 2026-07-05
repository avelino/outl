//! Pipeline tests for the Obsidian importer: full `import()` runs
//! against fixture vaults, asserting the produced workspace files.
//!
//! Function-level specs for the generic markdown primitives (YAML
//! frontmatter parsing, wiki-link variant collapse, image-link
//! conversion) live as unit tests next to their owners in
//! `outl-md/src/frontmatter.rs` and `outl-md/src/wikilink.rs`; this
//! file covers the importer's wiring and policy (title resolution,
//! journal routing, `path::`, collision suffixes, idempotency).

use super::import;
use crate::cmd::import::ImportReport;
use crate::workspace_layout::Paths;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Run an import against `vault`, returning the workspace TempDir
/// (kept alive so the caller can read the produced files), the
/// workspace [`Paths`], and the import report.
fn run_import(vault: &Path) -> (TempDir, Paths, ImportReport) {
    let dst_dir = TempDir::new().unwrap();
    let dst = dst_dir.path().join("ws");
    crate::cmd::init::run(&dst).unwrap();
    let paths = Paths::at(dst);
    let report = import(vault, &paths).unwrap();
    (dst_dir, paths, report)
}

fn vault_with(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().unwrap();
    for (rel, content) in files {
        let p = dir.path().join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, content).unwrap();
    }
    dir
}

// --- pipeline-level tests -------------------------------------------

#[test]
fn basic_page_with_bullets_round_trips() {
    let vault = vault_with(&[("Project.md", "- goal A\n- goal B\n")]);
    let (_hold, paths, report) = run_import(vault.path());
    assert_eq!(report.pages, 1);
    assert_eq!(report.journals, 0);

    let out = fs::read_to_string(paths.pages.join("project.md")).unwrap();
    assert!(
        out.starts_with("title:: Project\n"),
        "title missing:\n{out}"
    );
    assert!(out.contains("- goal A"));
    assert!(out.contains("- goal B"));
}

#[test]
fn iso_filename_routes_to_journals() {
    let vault = vault_with(&[("2026-05-25.md", "- morning note\n")]);
    let (_hold, paths, report) = run_import(vault.path());
    assert_eq!(report.journals, 1);
    assert_eq!(report.pages, 0);

    let p = paths.journals.join("2026-05-25.md");
    assert!(p.exists(), "journal file missing at {}", p.display());
    let out = fs::read_to_string(p).unwrap();
    // Journals don't get title:: — the filename is the date.
    assert!(!out.starts_with("title::"));
    assert!(out.contains("- morning note"));
}

#[test]
fn iso_date_filename_routes_to_journals_regardless_of_folder() {
    // File lives in a `daily/` folder with `.obsidian/daily-notes.json`
    // pointing at it — but routing fires because of the ISO filename,
    // not the folder. We keep this test as a sanity check that the
    // common Obsidian setup imports cleanly.
    let vault = vault_with(&[
        (".obsidian/daily-notes.json", r#"{"folder":"daily"}"#),
        ("daily/2026-06-01.md", "- standup\n"),
        ("pages/Project.md", "- work\n"),
    ]);
    let (_hold, paths, report) = run_import(vault.path());
    assert_eq!(report.journals, 1);
    assert_eq!(report.pages, 1);
    assert!(paths.journals.join("2026-06-01.md").exists());
    assert!(paths.pages.join("project.md").exists());
}

#[test]
fn non_date_file_in_daily_folder_stays_a_page() {
    // HIGH regression guard: a file inside the configured daily
    // notes folder whose filename isn't a date must stay a regular
    // page (with `path::` recording the origin), not be force
    // routed to journals/.
    let vault = vault_with(&[
        (".obsidian/daily-notes.json", r#"{"folder":"daily"}"#),
        ("daily/sprint-kickoff.md", "- agenda\n"),
    ]);
    let (_hold, paths, report) = run_import(vault.path());
    assert_eq!(report.pages, 1);
    assert_eq!(report.journals, 0);
    let out = fs::read_to_string(paths.pages.join("sprint-kickoff.md")).unwrap();
    assert!(out.contains("title:: sprint-kickoff"), "title lost:\n{out}");
    assert!(out.contains("path:: daily"), "path missing:\n{out}");
}

#[test]
fn skips_obsidian_and_trash_dirs() {
    let vault = vault_with(&[
        (".obsidian/app.json", "{}"),
        (".obsidian/workspace.json", "{}"),
        (".trash/Deleted.md", "- should not import\n"),
        ("Note.md", "- keep\n"),
    ]);
    let (_hold, paths, report) = run_import(vault.path());
    assert_eq!(report.pages, 1);
    // No file produced for the dotfile entries.
    assert!(!paths.pages.join("app.md").exists());
    assert!(!paths.pages.join("workspace.md").exists());
    assert!(!paths.pages.join("deleted.md").exists());
    assert!(paths.pages.join("note.md").exists());
}

#[test]
fn nested_folder_emits_path_property() {
    let vault = vault_with(&[("projects/work/Q4.md", "- quarter plan\n")]);
    let (_hold, paths, report) = run_import(vault.path());
    assert_eq!(report.pages, 1);

    let out = fs::read_to_string(paths.pages.join("q4.md")).unwrap();
    assert!(out.contains("path:: projects/work"), "path missing:\n{out}");
    assert!(out.contains("- quarter plan"));
}

#[test]
fn vault_root_file_has_no_path_property() {
    let vault = vault_with(&[("Flat.md", "- hi\n")]);
    let (_hold, paths, _report) = run_import(vault.path());
    let out = fs::read_to_string(paths.pages.join("flat.md")).unwrap();
    assert!(!out.contains("path::"), "unexpected path:: in:\n{out}");
}

#[test]
fn journal_in_daily_folder_has_no_path_property() {
    let vault = vault_with(&[
        (".obsidian/daily-notes.json", r#"{"folder":"daily"}"#),
        ("daily/2026-06-01.md", "- standup\n"),
    ]);
    let (_hold, paths, _report) = run_import(vault.path());
    let out = fs::read_to_string(paths.journals.join("2026-06-01.md")).unwrap();
    assert!(!out.contains("path::"), "unexpected path:: in:\n{out}");
}

// --- wiki-link wiring (variant matrix lives in outl-md::wikilink) ----

#[test]
fn wikilink_alias_is_stripped() {
    let vault = vault_with(&[("Note.md", "- see [[Target|the alias]] here\n")]);
    let (_hold, paths, _report) = run_import(vault.path());
    let out = fs::read_to_string(paths.pages.join("note.md")).unwrap();
    assert!(out.contains("[[Target]]"), "alias not stripped:\n{out}");
    assert!(!out.contains("the alias"));
}

#[test]
fn note_embeds_are_preserved_image_embeds_become_md_links() {
    // Outl supports `![[note]]` block-note embeds natively, so a
    // non-image embed round-trips unchanged. Image attachments
    // (`![[foo.jpeg]]`) are converted to standard CommonMark image
    // syntax because outl has no image-as-page.
    let vault = vault_with(&[("Note.md", "- see ![[other-note]] and ![[image.png]]\n")]);
    let (_hold, paths, _) = run_import(vault.path());
    let out = fs::read_to_string(paths.pages.join("note.md")).unwrap();
    assert!(out.contains("![[other-note]]"), "note embed lost:\n{out}");
    assert!(
        out.contains("![image.png](image.png)"),
        "image embed not converted:\n{out}"
    );
}

// --- slug collision disambiguation ----------------------------------

#[test]
fn colliding_titles_get_path_derived_suffix() {
    // Two source files with the same H1 "Ideas" but different
    // folders. The lex-smallest relative path wins the bare slug;
    // the other gets a folder-derived suffix.
    //   "Docs/HL Game Design/Ideas.md"  (H...)
    //   "Docs/Ideas/Ideas.md"           (I...)
    // 'H' < 'I' lexicographically, so HL Game Design wins the bare
    // slug and Docs/Ideas gets the suffix.
    let vault = vault_with(&[
        ("Docs/Ideas/Ideas.md", "# Ideas\n- a\n"),
        ("Docs/HL Game Design/Ideas.md", "# Ideas\n- b\n"),
    ]);
    let (_hold, paths, report) = run_import(vault.path());
    assert_eq!(report.pages, 2);

    let winner = fs::read_to_string(paths.pages.join("ideas.md")).unwrap();
    assert!(
        winner.contains("- b"),
        "wrong winner content (expected HL Game Design's '- b'):\n{winner}"
    );
    let suffixed = fs::read_to_string(paths.pages.join("ideas-ideas.md"))
        .expect("expected suffixed file `ideas-ideas.md`");
    assert!(suffixed.contains("- a"));
    // title:: is unaffected by the disambiguation.
    assert!(suffixed.contains("title:: Ideas"));
}

#[test]
fn same_folder_collision_uses_folder_suffix() {
    // Two files in the same folder with the same H1. The folder
    // suffix alone produces a unique stem (because the winner has
    // no suffix), so the disambiguator stops there. Filename-stem
    // is only tried if folder-suffix also collides.
    let vault = vault_with(&[
        ("Docs/Prompt A.md", "# Same Title\n- a\n"),
        ("Docs/Prompt B.md", "# Same Title\n- b\n"),
    ]);
    let (_hold, paths, report) = run_import(vault.path());
    assert_eq!(report.pages, 2);

    // "Docs/Prompt A.md" < "Docs/Prompt B.md" lex, so A wins.
    let winner = fs::read_to_string(paths.pages.join("same-title.md")).unwrap();
    assert!(winner.contains("- a"), "wrong winner:\n{winner}");
    let suffixed = fs::read_to_string(paths.pages.join("same-title-docs.md"))
        .expect("expected `same-title-docs.md`");
    assert!(suffixed.contains("- b"));
}

// --- frontmatter policy (generic parsing lives in outl-md::frontmatter)

#[test]
fn frontmatter_title_and_tags_become_properties() {
    let vault = vault_with(&[(
        "Note.md",
        "---\ntitle: Real Title\ntags: [foo, bar]\n---\n- body bullet\n",
    )]);
    let (_hold, paths, report) = run_import(vault.path());
    assert_eq!(report.pages, 1);

    // Filename is `Note.md` but frontmatter title is `Real Title`,
    // so the slug comes from `Real Title`.
    let out = fs::read_to_string(paths.pages.join("real-title.md")).unwrap();
    assert!(out.contains("title:: Real Title"), "title wrong:\n{out}");
    assert!(out.contains("tags:: #foo #bar"), "tags wrong:\n{out}");
    assert!(out.contains("- body bullet"));
}

#[test]
fn frontmatter_dropped_keys_are_counted() {
    let vault = vault_with(&[(
        "Note.md",
        "---\naliases: [foo, bar]\ncssclass: wide\npublish: false\n---\n- x\n",
    )]);
    let (_hold, _paths, report) = run_import(vault.path());
    assert!(
        report.artifacts_stripped >= 3,
        "dropped frontmatter keys not counted: {report:?}"
    );
}

#[test]
fn frontmatter_date_is_normalized() {
    let vault = vault_with(&[("Note.md", "---\ndate: 2026/04/22\n---\n- x\n")]);
    let (_hold, paths, _) = run_import(vault.path());
    let out = fs::read_to_string(paths.pages.join("note.md")).unwrap();
    assert!(
        out.contains("date:: 2026-04-22"),
        "date not normalized:\n{out}"
    );
}

#[test]
fn frontmatter_no_closing_fence_passes_through() {
    // Malformed frontmatter should not eat the file.
    let vault = vault_with(&[("Note.md", "---\ntitle: half\n- bullet\n")]);
    let (_hold, paths, _) = run_import(vault.path());
    let out = fs::read_to_string(paths.pages.join("note.md")).unwrap();
    assert!(out.contains("- bullet"), "body lost on bad fm:\n{out}");
}

// --- title fallbacks -------------------------------------------------

#[test]
fn leading_h1_becomes_title_and_is_stripped_from_body() {
    let vault = vault_with(&[("Note.md", "# Real Heading\n- under h1\n")]);
    let (_hold, paths, _) = run_import(vault.path());
    // Slug derived from H1, not from filename.
    let out = fs::read_to_string(paths.pages.join("real-heading.md")).unwrap();
    assert!(out.contains("title:: Real Heading"), "title wrong:\n{out}");
    // H1 line itself is gone.
    assert!(!out.contains("# Real Heading"));
    assert!(out.contains("- under h1"));
}

#[test]
fn frontmatter_title_beats_h1() {
    let vault = vault_with(&[("Note.md", "---\ntitle: FM Title\n---\n# H1 Title\n- body\n")]);
    let (_hold, paths, _) = run_import(vault.path());
    let out = fs::read_to_string(paths.pages.join("fm-title.md")).unwrap();
    assert!(out.contains("title:: FM Title"));
    // H1 stays in body since title came from frontmatter.
    assert!(out.contains("# H1 Title"));
}

// --- idempotency -----------------------------------------------------

#[test]
fn reimport_produces_same_files() {
    let vault = vault_with(&[
        ("Note.md", "- x\n- y\n"),
        ("projects/Sub.md", "- nested\n"),
        ("2026-05-25.md", "- journal\n"),
    ]);

    let dst1 = TempDir::new().unwrap();
    let dst1_path = dst1.path().join("ws");
    crate::cmd::init::run(&dst1_path).unwrap();
    let paths1 = Paths::at(dst1_path);
    import(vault.path(), &paths1).unwrap();

    let dst2 = TempDir::new().unwrap();
    let dst2_path = dst2.path().join("ws");
    crate::cmd::init::run(&dst2_path).unwrap();
    let paths2 = Paths::at(dst2_path);
    import(vault.path(), &paths2).unwrap();

    for name in &["note.md", "sub.md"] {
        let a = fs::read_to_string(paths1.pages.join(name)).unwrap();
        let b = fs::read_to_string(paths2.pages.join(name)).unwrap();
        assert_eq!(a, b, "non-idempotent page {name}:\nA:\n{a}\nB:\n{b}");
    }
    let a = fs::read_to_string(paths1.journals.join("2026-05-25.md")).unwrap();
    let b = fs::read_to_string(paths2.journals.join("2026-05-25.md")).unwrap();
    assert_eq!(a, b, "non-idempotent journal:\nA:\n{a}\nB:\n{b}");
}

#[test]
fn reimport_into_same_destination_is_idempotent() {
    // Same-destination re-import is the more failure-prone case
    // (overwrite semantics, stale sidecars, path collisions).
    let vault = vault_with(&[
        ("Note.md", "- a\n"),
        ("projects/Sub.md", "- b\n"),
        ("2026-05-25.md", "- j\n"),
    ]);

    let dst = TempDir::new().unwrap();
    let dst_path = dst.path().join("ws");
    crate::cmd::init::run(&dst_path).unwrap();
    let paths = Paths::at(dst_path.clone());

    import(vault.path(), &paths).unwrap();
    let snap: Vec<(String, String)> = walkdir::WalkDir::new(&dst_path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
        .map(|e| {
            (
                e.path()
                    .strip_prefix(&dst_path)
                    .unwrap()
                    .display()
                    .to_string(),
                fs::read_to_string(e.path()).unwrap(),
            )
        })
        .collect();

    import(vault.path(), &paths).unwrap();
    for (rel, before) in &snap {
        let after = fs::read_to_string(dst_path.join(rel)).unwrap();
        assert_eq!(before, &after, "non-idempotent re-import of {rel}");
    }
}
