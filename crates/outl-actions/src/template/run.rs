//! Callable-template execution — the shared owner every client wraps.
//!
//! A ` ```call:<name> ` fence resolves a callable template, injects its
//! params, runs it through a client-supplied [`RuntimeRegistry`], and
//! writes the output as a `> **result:**` subtree. The TUI (`gx`) and
//! the desktop exec command both call [`run_callable_block`] so the two
//! surfaces can't drift on param injection or result shape — the exact
//! duplication the repo's reuse-first policy exists to prevent.

use std::collections::HashSet;

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_exec::{ExecContext, ExecOutput, RuntimeRegistry};
use sha2::{Digest, Sha256};

use crate::block::{create_with_explicit_id, delete, edit_text, move_under};
use crate::error::ActionError;
use crate::template::{inject_call_params, parse_call_params, resolve_call};
use crate::tree::{children_of, position_for_new_last_child};

/// Parse a ` ```call:<name> ` block into its template name and params.
///
/// Returns `None` when `text` is not a call block. Fence parsing goes
/// through [`outl_exec::extract_fence`] (the same parser the runtime
/// uses), so "what executes" and "what is detected" can't drift.
pub fn parse_call_invocation(text: &str) -> Option<(String, Vec<(String, String)>)> {
    let parts = outl_exec::extract_fence(text)?;
    let name = parts
        .language
        .strip_prefix("call:")
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    Some((name.to_string(), parse_call_params(&parts.body)))
}

/// Resolve callable template `name`, inject `params`, execute it via
/// `registry`, and write stdout as a `> **result:**` subtree under
/// `anchor` (replacing any prior result on re-run).
///
/// Returns the runtime's [`ExecOutput`] (stdout/duration/exit) so the
/// caller can surface it. The caller owns the reload/reproject after
/// this returns (each client projects `.md` differently).
pub fn run_callable_block(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    registry: &RuntimeRegistry,
    name: &str,
    params: &[(String, String)],
    anchor: NodeId,
) -> Result<ExecOutput, ActionError> {
    let resolution = resolve_call(workspace, name)?;
    let lang = outl_md::lang::canonical(&resolution.language)
        .unwrap_or(&resolution.language)
        .to_string();
    let runtime = registry
        .get(&lang)
        .ok_or_else(|| ActionError::Exec(format!("no runtime for `{lang}`")))?;
    let source = inject_call_params(&resolution.language, &resolution.source, params);
    let ctx = ExecContext {
        workspace_root: workspace.root.clone().unwrap_or_default(),
        ..Default::default()
    };
    let out = runtime
        .execute(&source, &ctx)
        .map_err(|e| ActionError::Exec(e.to_string()))?;

    write_result_subtree(workspace, hlc, anchor, out.stdout.trim())?;
    Ok(out)
}

/// Replace any `> **result:**` subtree under `anchor` with a fresh one
/// whose children are `stdout`'s lines. Empty `stdout` clears the
/// previous result.
const RESULT_HEADER: &str = "> **result:**";

/// Whether `text` is a result-block header (current or legacy form).
fn is_result_header(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with(RESULT_HEADER) || t.starts_with("> result:")
}

/// Deterministic `NodeId` for a call block's result subtree.
///
/// Both devices derive the **same** id from the call block, so a
/// re-run (local or on a peer) converges on one node instead of each
/// device creating its own result and deleting the other's — a
/// delete/recreate war that bloated the op log and made the tree
/// oscillate under P2P sync.
fn result_node_id(anchor: NodeId) -> NodeId {
    derive_result_id(&format!("{anchor}:result"))
}

/// Deterministic `NodeId` for the `index`-th output line under a result
/// node.
fn result_child_id(result: NodeId, index: usize) -> NodeId {
    derive_result_id(&format!("{result}:{index}"))
}

fn derive_result_id(seed: &str) -> NodeId {
    let mut h = Sha256::new();
    h.update(b"outl-call-result:");
    h.update(seed.as_bytes());
    let digest = h.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    NodeId(ulid::Ulid::from_bytes(bytes))
}

fn node_is_child(workspace: &Workspace, parent: NodeId, node: NodeId) -> bool {
    children_of(workspace, parent)
        .iter()
        .any(|(id, _)| *id == node)
}

/// Set `node`'s text only when it differs — a no-op edit still emits an
/// `Op::Edit`, so guarding it keeps re-runs of an unchanged result from
/// churning the op log.
fn set_text_if_changed(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
    text: &str,
) -> Result<(), ActionError> {
    if workspace.block_text(node).as_deref() != Some(text) {
        edit_text(workspace, hlc, node, text)?;
    }
    Ok(())
}

/// Upsert the `> **result:**` subtree under `anchor` **in place**.
///
/// The result parent and each output line get a deterministic `NodeId`
/// (derived from the call block), so re-runs update the same nodes
/// instead of delete+recreate, and two devices converge on one result
/// (last write wins per line via HLC) instead of oscillating between
/// competing subtrees. An empty `stdout` clears the result.
fn write_result_subtree(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    anchor: NodeId,
    stdout: &str,
) -> Result<(), ActionError> {
    let result_id = result_node_id(anchor);

    // One-time cleanup: drop any legacy result blocks (random ids from
    // the old delete+recreate code, or a peer's) so only the stable node
    // survives. A no-op once converged (nothing left matches).
    let legacy: Vec<NodeId> = children_of(workspace, anchor)
        .into_iter()
        .filter(|(id, _)| *id != result_id)
        .filter(|(id, _)| {
            workspace
                .block_text(*id)
                .is_some_and(|t| is_result_header(&t))
        })
        .map(|(id, _)| id)
        .collect();
    for id in legacy {
        delete(workspace, hlc, id)?;
    }

    if stdout.is_empty() {
        if node_is_child(workspace, anchor, result_id) {
            delete(workspace, hlc, result_id)?;
        }
        return Ok(());
    }

    // Ensure the result parent exists (idempotent on the stable id) and
    // carries the header.
    let pos = position_for_new_last_child(workspace, anchor);
    create_with_explicit_id(workspace, hlc, result_id, anchor, pos, None)?;
    if !node_is_child(workspace, anchor, result_id) {
        // Was trashed by a prior empty-result run — bring it back.
        move_under(workspace, hlc, result_id, anchor)?;
    }
    set_text_if_changed(workspace, hlc, result_id, RESULT_HEADER)?;

    // Upsert one child per output line, keyed by index.
    let lines: Vec<&str> = stdout.lines().collect();
    let keep: HashSet<NodeId> = (0..lines.len())
        .map(|i| result_child_id(result_id, i))
        .collect();
    for (i, line) in lines.iter().enumerate() {
        let child = result_child_id(result_id, i);
        let cpos = position_for_new_last_child(workspace, result_id);
        create_with_explicit_id(workspace, hlc, child, result_id, cpos, None)?;
        if !node_is_child(workspace, result_id, child) {
            // Existed but trashed (a prior shorter output) — resurrect.
            move_under(workspace, hlc, child, result_id)?;
        }
        set_text_if_changed(workspace, hlc, child, line)?;
    }

    // Trim any children not in the current output (shrunk output, or a
    // stray from a legacy run that landed under the stable node).
    let extra: Vec<NodeId> = children_of(workspace, result_id)
        .into_iter()
        .map(|(id, _)| id)
        .filter(|id| !keep.contains(id))
        .collect();
    for id in extra {
        delete(workspace, hlc, id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::append_block;
    use crate::page::{open_or_create, set_property, PageKind};
    use crate::template::TEMPLATE_KEY;
    use outl_core::id::ActorId;
    use outl_core::property::PropValue;

    fn ws() -> (Workspace, HlcGenerator) {
        let a = ActorId::new();
        (Workspace::open_in_memory(a).unwrap(), HlcGenerator::new(a))
    }

    #[test]
    fn parse_call_invocation_extracts_name_and_params() {
        let (name, params) = parse_call_invocation("```call:calc\nx: 1\ny: 2\n```").unwrap();
        assert_eq!(name, "calc");
        assert_eq!(
            params,
            vec![("x".into(), "1".into()), ("y".into(), "2".into())]
        );
        // A plain code fence is not a call invocation.
        assert!(parse_call_invocation("```python\nprint(1)\n```").is_none());
        assert!(parse_call_invocation("just text").is_none());
    }

    #[test]
    fn run_callable_block_resolves_template_language_not_call_prefix() {
        // Regression: a `call:calc` block must resolve to the template's
        // OWN code-block language, not be treated as a language literally
        // named `call:calc`. The template uses `fortran` — never a linked
        // runtime in any build — so the error deterministically names
        // `fortran` (proving resolution happened), regardless of which
        // real runtimes feature-unification pulls into the test binary.
        let (mut w, hlc) = ws();
        let tpl = open_or_create(&mut w, &hlc, "template-calc", "calc", PageKind::Page).unwrap();
        set_property(
            &mut w,
            &hlc,
            tpl,
            TEMPLATE_KEY,
            Some(PropValue::Text("calc".into())),
        )
        .unwrap();
        append_block(&mut w, &hlc, Some(tpl), Some("```fortran\nprint *, 1\n```")).unwrap();
        let anchor = append_block(&mut w, &hlc, None, Some("host")).unwrap();

        let registry = outl_exec::RuntimeRegistry::with_builtins();
        let err = run_callable_block(&mut w, &hlc, &registry, "calc", &[], anchor).unwrap_err();
        match err {
            ActionError::Exec(m) => assert!(
                m.contains("fortran"),
                "error should name the resolved language, not `call:calc`; got: {m}"
            ),
            other => panic!("expected Exec error, got {other:?}"),
        }
    }

    fn result_lines(w: &Workspace, rid: NodeId) -> Vec<String> {
        children_of(w, rid)
            .into_iter()
            .filter_map(|(id, _)| w.block_text(id))
            .collect()
    }

    #[test]
    fn result_subtree_is_idempotent_and_convergent() {
        let (mut w, hlc) = ws();
        let anchor = append_block(&mut w, &hlc, None, Some("host")).unwrap();
        let rid = result_node_id(anchor);

        // First write: stable parent id + one child per line.
        write_result_subtree(&mut w, &hlc, anchor, "a\nb\nc").unwrap();
        assert!(node_is_child(&w, anchor, rid), "result parent at stable id");
        assert_eq!(result_lines(&w, rid), vec!["a", "b", "c"]);

        // Re-run same output: no duplicate children, same node.
        write_result_subtree(&mut w, &hlc, anchor, "a\nb\nc").unwrap();
        assert_eq!(children_of(&w, rid).len(), 3, "no dup children on rerun");
        assert_eq!(result_lines(&w, rid), vec!["a", "b", "c"]);

        // Shrink + change text: updated in place under the same node.
        write_result_subtree(&mut w, &hlc, anchor, "x\ny").unwrap();
        assert!(
            node_is_child(&w, anchor, rid),
            "same result node after shrink"
        );
        assert_eq!(result_lines(&w, rid), vec!["x", "y"]);

        // Grow past the prior max: previously-trashed children resurrect.
        write_result_subtree(&mut w, &hlc, anchor, "p\nq\nr\ns").unwrap();
        assert_eq!(result_lines(&w, rid), vec!["p", "q", "r", "s"]);

        // Empty output clears the result entirely.
        write_result_subtree(&mut w, &hlc, anchor, "").unwrap();
        assert!(
            !node_is_child(&w, anchor, rid),
            "empty output clears result"
        );
    }

    #[test]
    fn result_node_id_is_deterministic_per_anchor() {
        let (mut w, hlc) = ws();
        let a = append_block(&mut w, &hlc, None, Some("a")).unwrap();
        let b = append_block(&mut w, &hlc, None, Some("b")).unwrap();
        // Same anchor → same id (both devices converge); different
        // anchors → different ids (no cross-block collision).
        assert_eq!(result_node_id(a), result_node_id(a));
        assert_ne!(result_node_id(a), result_node_id(b));
        assert_ne!(
            result_child_id(result_node_id(a), 0),
            result_child_id(result_node_id(a), 1)
        );
    }
}
