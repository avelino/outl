//! Integration tests for the machine-shaped CLI surface (page / block /
//! daily / search / etc.). Each test runs the real `outl` binary in a
//! tempdir so the assertions exercise the same code paths a user
//! would.
//!
//! Pairs with `e2e_full.rs` (TUI-style smoke) and `mcp_smoke.rs` (MCP
//! over stdio). Together they keep the CLI + MCP surface honest end to
//! end.

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

#[test]
fn page_create_and_get() {
    let ws = init_workspace();
    let env = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "create", "ideas", "--title", "Ideas", "--json"])
        .output()
        .unwrap());
    assert_eq!(env["ok"], true);
    assert_eq!(env["data"]["meta"]["slug"], "ideas");

    let got = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "get", "ideas", "--json"])
        .output()
        .unwrap());
    assert_eq!(got["data"]["meta"]["title"], "Ideas");
    assert!(got["data"]["outline"].is_array());
}

#[test]
fn block_append_then_toggle_todo() {
    let ws = init_workspace();
    let _ = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "create", "ideas", "--json"])
        .output()
        .unwrap());
    let append = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args([
            "block", "append", "--page", "ideas", "--text", "ship it", "--json",
        ])
        .output()
        .unwrap());
    let id = append["data"]["id"].as_str().unwrap().to_string();

    let toggle = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["block", "toggle-todo", &id, "--json"])
        .output()
        .unwrap());
    assert_eq!(toggle["data"]["todo"], "TODO");

    let toggle = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["block", "toggle-todo", &id, "--json"])
        .output()
        .unwrap());
    assert_eq!(toggle["data"]["todo"], "DONE");
}

#[test]
fn search_finds_appended_block() {
    let ws = init_workspace();
    let _ = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "create", "ideas", "--json"])
        .output()
        .unwrap());
    let _ = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args([
            "block",
            "append",
            "--page",
            "ideas",
            "--text",
            "shipping #shipping today",
            "--json",
        ])
        .output()
        .unwrap());
    let search = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["search", "shipping", "--in", "blocks", "--json"])
        .output()
        .unwrap());
    let blocks = search["data"]["blocks"].as_array().unwrap();
    assert!(
        !blocks.is_empty(),
        "search should find at least one block, got {search}"
    );
}

#[test]
fn page_delete_requires_confirm() {
    let ws = init_workspace();
    let _ = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "create", "throwaway", "--json"])
        .output()
        .unwrap());

    let no_confirm = outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "delete", "throwaway", "--json"])
        .output()
        .unwrap();
    assert!(
        !no_confirm.status.success(),
        "delete without --confirm must error"
    );
    let env: Value = serde_json::from_slice(&no_confirm.stdout).unwrap();
    assert_eq!(env["error"]["code"], "CONFIRM_REQUIRED");

    let yes = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "delete", "throwaway", "--confirm", "--json"])
        .output()
        .unwrap());
    assert_eq!(yes["data"]["slug"], "throwaway");
}

#[test]
fn workspace_info_returns_summary() {
    let ws = init_workspace();
    let info = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["workspace", "info", "--json"])
        .output()
        .unwrap());
    assert!(info["data"]["root"].is_string());
    assert!(info["data"]["actor"].is_string());
    assert!(info["data"]["ops"].is_number());
}
