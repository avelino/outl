//! Smoke test for the MCP stdio surface.
//!
//! Spawns `outl mcp serve --workspace <tmp>` in a subprocess, sends
//! `initialize`, `tools/list`, and `tools/call outl_workspace_info`
//! through stdin, and asserts the JSON-RPC responses. This is the
//! ground truth — if Claude Desktop / Cursor break, this would too.

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use tempfile::TempDir;

fn outl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_outl"))
}

fn init_workspace() -> TempDir {
    let dir = TempDir::new().unwrap();
    let status = outl()
        .arg("init")
        .arg(dir.path())
        .status()
        .expect("init failed");
    assert!(status.success());
    dir
}

struct McpClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl McpClient {
    fn spawn(workspace: &std::path::Path) -> Self {
        let mut child = outl()
            .args(["--workspace"])
            .arg(workspace)
            .args(["mcp", "serve"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn mcp serve");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn call(&mut self, payload: Value) -> Value {
        let line = payload.to_string();
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
        let mut response = String::new();
        self.stdout.read_line(&mut response).expect("read response");
        serde_json::from_str(response.trim()).expect("response was JSON")
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        // Closing stdin makes the MCP loop exit.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn initialize_then_call_workspace_info() {
    let ws = init_workspace();
    let mut client = McpClient::spawn(ws.path());

    let init = client.call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {}
        }
    }));
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["serverInfo"]["name"], "outl");

    let tools = client.call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list"
    }));
    let list = tools["result"]["tools"]
        .as_array()
        .expect("tools list is an array");
    assert!(
        list.iter().any(|t| t["name"] == "outl_workspace_info"),
        "outl_workspace_info must be registered"
    );

    let call = client.call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "outl_workspace_info",
            "arguments": {}
        }
    }));
    assert_eq!(call["id"], 3);
    let structured = &call["result"]["structuredContent"];
    assert_eq!(structured["ok"], true);
    assert!(structured["data"]["root"].is_string());
}

#[test]
fn doctor_via_mcp_does_not_lie_about_lock() {
    // Regression: doctor used to call `WorkspaceLock::acquire`, which
    // would always fail inside the MCP session (the server already
    // owns the lock) and report a non-existent contention. The fix
    // skips the lock probe and emits an info-level "probe skipped"
    // finding instead.
    let ws = init_workspace();
    let mut client = McpClient::spawn(ws.path());

    let _ = client.call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "protocolVersion": "2024-11-05", "capabilities": {} }
    }));

    let resp = client.call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": { "name": "outl_workspace_doctor", "arguments": {} }
    }));
    let structured = &resp["result"]["structuredContent"];
    assert_eq!(structured["ok"], true, "doctor must succeed inside MCP");
    let findings = structured["data"]["findings"].as_array().unwrap();
    let has_lock_warning = findings.iter().any(|f| {
        f["message"]
            .as_str()
            .unwrap_or("")
            .contains("another outl process is holding the workspace lock")
    });
    assert!(
        !has_lock_warning,
        "doctor must not warn about its own lock when running in MCP session"
    );
}

#[test]
fn page_create_then_get_via_mcp() {
    let ws = init_workspace();
    let mut client = McpClient::spawn(ws.path());

    // The handshake is required by some hosts; harmless if skipped, but
    // we go through it so the test mirrors real usage.
    let _ = client.call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "protocolVersion": "2024-11-05", "capabilities": {} }
    }));

    let create = client.call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "outl_page_create",
            "arguments": { "slug": "ideas", "title": "Ideas" }
        }
    }));
    let structured = &create["result"]["structuredContent"];
    assert_eq!(structured["ok"], true);
    assert_eq!(structured["data"]["meta"]["slug"], "ideas");

    let get = client.call(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "outl_page_get",
            "arguments": { "slug": "ideas" }
        }
    }));
    let s2 = &get["result"]["structuredContent"];
    assert_eq!(s2["ok"], true);
    assert_eq!(s2["data"]["meta"]["title"], "Ideas");
}
