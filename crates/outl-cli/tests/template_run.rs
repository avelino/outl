//! Integration tests for `outl template run` — the CLI/MCP execution
//! path for callable templates (issue: callable templates could only be
//! *resolved*, never *run*, outside the TUI).
//!
//! Each test drives the real `outl` binary in a tempdir so it exercises
//! the same code path a user (or the MCP shim) would. The template uses
//! a `lisp` code block — the Steel runtime is always in the default
//! `outl-exec` feature set, so the assertion is deterministic regardless
//! of which optional language runtimes the build unifies in.

use serde_json::Value;
use std::process::Command;
use tempfile::TempDir;

fn outl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_outl"))
}

fn ok(out: std::process::Output) -> Value {
    if !out.status.success() {
        panic!(
            "command failed:\nstatus: {:?}\nstdout: {}\nstderr: {}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
    }
    serde_json::from_slice(&out.stdout).expect("non-JSON stdout")
}

fn init_workspace() -> TempDir {
    let dir = TempDir::new().unwrap();
    let status = outl()
        .arg("init")
        .arg(dir.path())
        .status()
        .expect("init failed");
    assert!(status.success(), "outl init must succeed");
    dir
}

fn create_page(ws: &TempDir, slug: &str) {
    let _ = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "create", slug, "--json"])
        .output()
        .unwrap());
}

fn append_block(ws: &TempDir, page: &str, text: &str) -> String {
    let v = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["block", "append", "--page", page, "--text", text, "--json"])
        .output()
        .unwrap());
    v["data"]["id"].as_str().unwrap().to_string()
}

fn set_prop(ws: &TempDir, page: &str, assignment: &str) {
    let _ = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "prop", "set", page, assignment, "--json"])
        .output()
        .unwrap());
}

/// Define a callable `echo` template (a page with `template:: echo` and a
/// `lisp` code block) and run it against an anchor block on another page.
#[test]
fn template_run_writes_result_subtree() {
    let ws = init_workspace();

    // Callable template page: `template:: echo` + a lisp code block.
    create_page(&ws, "tpl-echo");
    set_prop(&ws, "tpl-echo", "template=echo");
    append_block(
        &ws,
        "tpl-echo",
        "```lisp\n(displayln \"hello from template\")\n```",
    );

    // Target page with an anchor block the result lands under.
    create_page(&ws, "notes");
    let anchor = append_block(&ws, "notes", "run it here");

    let run = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["template", "run", "echo", "--page", "notes", "--block"])
        .arg(&anchor)
        .args(["--json"])
        .output()
        .unwrap());
    assert_eq!(run["ok"], true, "template run must succeed: {run}");
    assert_eq!(run["data"]["template"], "echo");
    assert_eq!(run["data"]["page"], "notes");
    let stdout = run["data"]["result"]["stdout"].as_str().unwrap_or("");
    assert!(
        stdout.contains("hello from template"),
        "runtime stdout should carry the template output, got: {stdout:?}"
    );

    // The `> **result:**` subtree must be projected to disk under the
    // anchor block on the `notes` page.
    let md = std::fs::read_to_string(ws.path().join("pages").join("notes.md")).unwrap();
    assert!(
        md.contains("**result:**"),
        "result header must appear in notes.md, got:\n{md}"
    );
    assert!(
        md.contains("hello from template"),
        "result content must appear in notes.md, got:\n{md}"
    );
}

/// Audit fix #4: `--block` on a page different from `--page` must be
/// rejected with `INVALID_ARG` (instantiating there then reprojecting
/// only `--page` would silently drop the new blocks from disk).
#[test]
fn template_run_rejects_block_on_other_page() {
    let ws = init_workspace();

    create_page(&ws, "tpl-echo");
    set_prop(&ws, "tpl-echo", "template=echo");
    append_block(&ws, "tpl-echo", "```lisp\n(displayln \"hi\")\n```");

    create_page(&ws, "notes");
    create_page(&ws, "other");
    // Anchor lives on `other`, but we pass `--page notes`.
    let foreign = append_block(&ws, "other", "not on notes");

    let out = outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["template", "run", "echo", "--page", "notes", "--block"])
        .arg(&foreign)
        .args(["--json"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "cross-page --block must error, got success"
    );
    let env: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        env["error"]["code"], "INVALID_ARG",
        "cross-page --block must map to INVALID_ARG, got {env}"
    );
}

/// The same cross-page guard applies to `template apply` (the original
/// audit finding: it accepted any `--block` ULID and instantiated under
/// it even on a foreign page).
#[test]
fn template_apply_rejects_block_on_other_page() {
    let ws = init_workspace();

    // Structural template page.
    create_page(&ws, "tpl-struct");
    set_prop(&ws, "tpl-struct", "template=struct");
    append_block(&ws, "tpl-struct", "seed block");

    create_page(&ws, "notes");
    create_page(&ws, "other");
    let foreign = append_block(&ws, "other", "not on notes");

    let out = outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["template", "apply", "struct", "--page", "notes", "--block"])
        .arg(&foreign)
        .args(["--json"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "cross-page --block must error on apply, got success"
    );
    let env: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(env["error"]["code"], "INVALID_ARG");
}
