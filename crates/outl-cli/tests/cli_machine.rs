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
fn malicious_slug_is_rejected() {
    let ws = init_workspace();
    // Path traversal attempt — slug must not be accepted because it
    // would end up joined into a filesystem path on export.
    for bad in ["../escape", "with/slash", "with\\backslash", "..", "."] {
        let out = outl()
            .args(["--workspace"])
            .arg(ws.path())
            .args(["page", "create", bad, "--json"])
            .output()
            .unwrap();
        assert!(
            !out.status.success(),
            "slug `{bad}` should be rejected, got success"
        );
        let env: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(
            env["error"]["code"], "INVALID_ARG",
            "slug `{bad}` should map to INVALID_ARG"
        );
    }
}

#[test]
fn doctor_json_runs_cleanly() {
    let ws = init_workspace();
    let out = outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["doctor", "--json"])
        .output()
        .unwrap();
    let env: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(env["ok"], true, "doctor --json must succeed on a fresh ws");
    assert!(env["data"]["findings"].is_array());
    // Fresh workspace has no errors (warnings are allowed: missing
    // sidecar for the seed journal is expected on first init).
    let errors = env["data"]["error_count"].as_u64().unwrap_or(0);
    assert_eq!(errors, 0, "fresh workspace should have zero errors");
}

#[test]
fn delete_is_idempotent_when_md_is_missing() {
    let ws = init_workspace();
    let _ = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "create", "throwaway", "--json"])
        .output()
        .unwrap());
    // Drop the on-disk .md before deleting. The op log must still
    // succeed and the missing file must not crash anything.
    let md = ws.path().join("pages").join("throwaway.md");
    std::fs::remove_file(&md).ok();
    let v = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "delete", "throwaway", "--confirm", "--json"])
        .output()
        .unwrap());
    assert_eq!(v["data"]["slug"], "throwaway");
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

// --- OUTL_WORKSPACE env var resolution -------------------------------
//
// Precedence: positional > `--workspace` flag > `OUTL_WORKSPACE` env >
// cwd. `init` is the one exception — it ignores the env var so it never
// scaffolds a workspace at a directory inherited from the shell.

/// With no flag and no positional, the env var alone targets the
/// workspace.
#[test]
fn workspace_from_env() {
    let ws = init_workspace();
    let env = ok(outl()
        .env("OUTL_WORKSPACE", ws.path())
        .args(["page", "create", "from-env", "--title", "FromEnv", "--json"])
        .output()
        .unwrap());
    assert_eq!(env["ok"], true);
    assert_eq!(env["data"]["meta"]["slug"], "from-env");

    // And the page is readable from the same env-resolved workspace.
    let got = ok(outl()
        .env("OUTL_WORKSPACE", ws.path())
        .args(["page", "get", "from-env", "--json"])
        .output()
        .unwrap());
    assert_eq!(got["data"]["meta"]["title"], "FromEnv");
}

/// The `--workspace` flag wins over `OUTL_WORKSPACE`: the page lands in
/// the flag's workspace, and the env's workspace stays empty.
#[test]
fn flag_beats_env() {
    let flag_ws = init_workspace();
    let env_ws = init_workspace();

    let created = ok(outl()
        .env("OUTL_WORKSPACE", env_ws.path())
        .args(["--workspace"])
        .arg(flag_ws.path())
        .args([
            "page", "create", "flagwins", "--title", "FlagWins", "--json",
        ])
        .output()
        .unwrap());
    assert_eq!(created["ok"], true);

    // Present in the flag workspace...
    let in_flag = ok(outl()
        .args(["--workspace"])
        .arg(flag_ws.path())
        .args(["page", "get", "flagwins", "--json"])
        .output()
        .unwrap());
    assert_eq!(in_flag["data"]["meta"]["slug"], "flagwins");

    // ...and absent from the env workspace.
    let in_env = outl()
        .args(["--workspace"])
        .arg(env_ws.path())
        .args(["page", "get", "flagwins", "--json"])
        .output()
        .unwrap();
    assert!(
        !in_env.status.success(),
        "page must not exist in env workspace"
    );
}

/// `init` deliberately ignores `OUTL_WORKSPACE`: with the env set but no
/// positional/flag, it must refuse rather than scaffold at the env path.
#[test]
fn init_ignores_env() {
    let dir = TempDir::new().unwrap();
    let out = outl()
        .env("OUTL_WORKSPACE", dir.path())
        .arg("init")
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "init must fail when only the env var is set"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("needs an explicit path"),
        "unexpected stderr: {stderr}"
    );
    // No workspace layout was created at the env path.
    assert!(!dir.path().join("pages").exists());
}

/// `init --workspace <dir>` works even with the env set and the flag
/// placed *after* the subcommand — covers `value_source` correctly
/// reporting `CommandLine` for the global arg.
#[test]
fn init_with_flag_after_subcommand_beats_env() {
    let env_dir = TempDir::new().unwrap();
    let target = TempDir::new().unwrap();
    let status = outl()
        .env("OUTL_WORKSPACE", env_dir.path())
        .arg("init")
        .args(["--workspace"])
        .arg(target.path())
        .status()
        .unwrap();
    assert!(
        status.success(),
        "init with explicit --workspace must succeed"
    );
    assert!(
        target.path().join("pages").exists(),
        "init must scaffold the flag path"
    );
    assert!(
        !env_dir.path().join("pages").exists(),
        "env path must stay untouched"
    );
}
