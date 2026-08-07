//! A test run must resolve its actor from an **isolated** device store,
//! never the developer's real `~/.config/outl/`.

use outl_core::device::device_dir;
use outl_core::workspace_id::WorkspaceId;
use outl_ws::layout::{init, Paths};
use tempfile::TempDir;

/// **Do not delete.** Opening a workspace binds this device's actor under
/// [`device_dir`], and keeps no identity of its own inside the workspace.
///
/// Why it exists: every `outl_ws::open` writes a `<device_dir>/actors/*`
/// binding, and the suite opens hundreds of throwaway workspaces. While
/// `device_dir()` resolved to the real `~/.config/outl/`, each of those
/// left a stale binding in the developer's own config directory — dozens
/// of them, naming temp paths long since deleted — and, worse, made every
/// test binary share the one `machine-id` that arbitrates actor claims.
/// That sharing is what made `outl-cli`'s doctor suite fail
/// intermittently: `init` stamped a claim, and the `open` right after it
/// read a machine id a concurrent test binary had just reminted.
///
/// The redirect itself lives in the repo's `.cargo/config.toml`, guarded
/// by `outl_core`'s `the_test_suite_runs_against_an_isolated_device_store`.
/// This is the other half of the pair: it pins that a workspace open
/// really does route its actor through `device_dir()`, so the redirect
/// cannot quietly stop covering the path it was written for.
#[test]
fn opening_a_workspace_binds_its_actor_inside_the_device_dir() {
    let dir = TempDir::new().expect("tempdir");
    let paths = Paths::at(dir.path().to_path_buf());
    init(&paths).expect("init");

    let ctx = outl_ws::open(dir.path()).expect("open");
    let (actor, ephemeral) = (ctx.actor, ctx.ephemeral_actor);
    drop(ctx);

    // Nobody else holds this brand-new workspace, so the open must have
    // landed on the device actor. Without this the binding assertion
    // below could pass while comparing against an ephemeral stand-in.
    assert!(
        !ephemeral,
        "a freshly created workspace has no other holder, so {actor} should be the device actor"
    );

    let id = WorkspaceId::read_or_create(dir.path()).expect("workspace id");
    let binding = device_dir().join("actors").join(id.as_str());
    let recorded = std::fs::read_to_string(&binding)
        .unwrap_or_else(|e| panic!("open must record this device's actor at {binding:?}: {e}"));
    assert!(
        recorded.contains(&actor.to_string()),
        "the binding at {binding:?} must name the actor the open resolved to ({actor}), got \
         {recorded:?}"
    );

    // And the identity is never derivable from the workspace itself: it
    // rides the file-sync surface on every transport but iCloud, and two
    // devices reading one actor id is silent op loss (`outl_ws::actor`).
    for stray in ["machine-id", "actor", "actors"] {
        let leaked = paths.dot_outl.join(stray);
        assert!(
            !leaked.exists(),
            "device identity must not live inside the workspace, found {leaked:?}"
        );
    }
}
