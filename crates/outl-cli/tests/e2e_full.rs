//! End-to-end smoke: init → write .md → serve --once → ensure sidecar
//! has IDs, op log persisted, reopened workspace shows same blocks.

use std::fs;
use std::process::Command;

fn cargo_run(args: &[&str]) -> std::process::Output {
    let bin = env!("CARGO_BIN_EXE_outl");
    Command::new(bin)
        .args(args)
        .output()
        .expect("failed to run outl binary")
}

fn must_ok(out: &std::process::Output, ctx: &str) {
    if !out.status.success() {
        panic!(
            "{ctx} failed:\nstatus: {:?}\nstdout:\n{}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn full_workspace_lifecycle() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("ws");
    let root_str = root.to_str().unwrap();

    // 1. init
    must_ok(&cargo_run(&["init", root_str]), "init");
    assert!(root.join(".outl/log.db").exists());
    assert!(root.join(".outl/config.toml").exists());

    // 2. write a markdown page in pages/
    let md_path = root.join("pages").join("hello.md");
    fs::write(
        &md_path,
        "title:: hello\nstatus:: active\n\n- first block\n  priority:: high\n  - nested child\n- second block with [[link]] and #tag\n",
    )
    .unwrap();

    // 3. serve --once: reconcile and produce sidecar
    must_ok(&cargo_run(&["serve", root_str, "--once"]), "serve --once");

    // 4. assertions on sidecar
    let sidecar_path = root.join("pages").join(".hello.outl");
    assert!(sidecar_path.exists(), "sidecar must exist after serve");
    let sidecar_text = fs::read_to_string(&sidecar_path).unwrap();
    let sidecar: serde_json::Value = serde_json::from_str(&sidecar_text).unwrap();
    assert_eq!(sidecar["version"], 2);
    let blocks = sidecar["blocks"].as_array().expect("blocks array");
    // Two top-level + one nested. `priority:: high` is a property, not a block.
    assert_eq!(blocks.len(), 3, "expected 3 blocks in sidecar");
    // Each block must have a ULID-looking id, a content hash, and a v2
    // `ref_handle` of the form `blk-XXXXXX` (lowercase tail).
    for b in blocks {
        let id = b["id"].as_str().unwrap();
        let hash = b["content_hash"].as_str().unwrap();
        let handle = b["ref_handle"].as_str().expect("ref_handle on v2 block");
        assert_eq!(id.len(), 26, "ULID should be 26 chars, got {id}");
        assert!(hash.starts_with("sha256:"));
        assert!(handle.starts_with("blk-"), "handle prefix: {handle}");
        assert_eq!(handle.len(), "blk-".len() + 6, "handle length: {handle}");
        assert_eq!(
            handle,
            handle.to_lowercase(),
            "handle must be lowercase: {handle}"
        );
    }

    // 5. assertions on the .md — must stay CLEAN (no id::, no UUIDs)
    let md_after = fs::read_to_string(&md_path).unwrap();
    assert!(
        !md_after.contains("id::"),
        "markdown must remain free of `id::` lines"
    );
    assert!(
        !md_after.contains("01H") && !md_after.contains("01J") && !md_after.contains("01K"),
        "markdown must not contain visible ULIDs"
    );

    // 6. doctor reports OK and counts ops
    let out = cargo_run(&["doctor", root_str]);
    must_ok(&out, "doctor");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ok:") || stdout.contains("integrity OK"),
        "doctor stdout should mention `ok:` lines:\n{stdout}"
    );

    // 7. idempotency: running serve --once again should produce no new ops
    let out2 = cargo_run(&["serve", root_str, "--once"]);
    must_ok(&out2, "serve --once (second pass)");

    // 8. reload from disk — sidecar still intact, IDs preserved
    let sidecar_text2 = fs::read_to_string(&sidecar_path).unwrap();
    let sidecar2: serde_json::Value = serde_json::from_str(&sidecar_text2).unwrap();
    let blocks2 = sidecar2["blocks"].as_array().unwrap();
    let ids_before: Vec<&str> = blocks.iter().map(|b| b["id"].as_str().unwrap()).collect();
    let ids_after: Vec<&str> = blocks2.iter().map(|b| b["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids_before, ids_after,
        "block IDs must be preserved across reconcile passes"
    );

    // 9. external edit: delete one block; sidecar shrinks and orphan logged
    fs::write(
        &md_path,
        "title:: hello\nstatus:: active\n\n- first block\n  priority:: high\n  - nested child\n",
    )
    .unwrap();
    must_ok(
        &cargo_run(&["serve", root_str, "--once"]),
        "serve after delete",
    );
    let sidecar_text3 = fs::read_to_string(&sidecar_path).unwrap();
    let sidecar3: serde_json::Value = serde_json::from_str(&sidecar_text3).unwrap();
    let blocks3 = sidecar3["blocks"].as_array().unwrap();
    assert_eq!(
        blocks3.len(),
        2,
        "deletion of `second block` leaves `first block` + `nested child` (2 entries)"
    );

    // Orphan log gained an entry.
    let orphans_log = fs::read_to_string(root.join(".outl/orphans.log")).unwrap();
    assert!(
        orphans_log.contains("id="),
        "orphans.log should contain at least one orphan entry:\n{orphans_log}"
    );
}
