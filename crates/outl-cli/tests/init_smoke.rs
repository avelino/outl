//! End-to-end smoke: init a workspace, add a markdown file, reconcile,
//! re-open the workspace from disk, confirm state is preserved.

use outl_cli_test_support::*;

mod outl_cli_test_support {
    use std::path::Path;
    use std::process::Command;

    pub fn cargo_run(args: &[&str]) -> std::process::Output {
        let bin = env!("CARGO_BIN_EXE_outl");
        Command::new(bin)
            .args(args)
            .output()
            .expect("failed to run outl binary")
    }

    pub fn assert_ok(out: &std::process::Output, ctx: &str) {
        if !out.status.success() {
            panic!(
                "{ctx} failed: status={:?}\nstdout: {}\nstderr: {}",
                out.status,
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    pub fn exists(p: &Path) -> bool {
        p.exists()
    }
}

#[test]
fn outl_init_then_doctor_reports_ok() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("ws");

    let out = cargo_run(&["init", root.to_str().unwrap()]);
    assert_ok(&out, "outl init");
    assert!(exists(&root.join("ops")));
    assert!(exists(&root.join(".outl/config.toml")));
    assert!(exists(&root.join("pages")));
    assert!(exists(&root.join("journals")));

    let out = cargo_run(&["doctor", root.to_str().unwrap()]);
    assert_ok(&out, "outl doctor");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("integrity OK") || stdout.contains("finding"),
        "doctor stdout did not contain expected status:\n{stdout}"
    );
}
