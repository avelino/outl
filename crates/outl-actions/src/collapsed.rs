//! Block fold (collapsed) state — Op-driven sync.
//!
//! Folding a block is a user-meaningful state mutation that must
//! converge between devices. Every flip goes through the op log
//! (`Op::SetCollapsed` in `outl-core`), exactly like Move / Edit /
//! SetProp. Each device writes to its own `ops-<actor>.jsonl`; iCloud
//! / Syncthing / shared FS sync those per-actor files; the CRDT
//! merges concurrent flips by HLC ordering.
//!
//! **Do not bring back the "write a flag to the sidecar" path.** That
//! was the previous design and it lost flips under iCloud's
//! last-write-wins-per-file semantics. The sidecar is for structural
//! `.md` ↔ tree matching only; any cross-device state belongs in the
//! op log. The root `CLAUDE.md` invariant codifies this.

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::op::{LogOp, Op};
use outl_core::workspace::Workspace;

use crate::error::ActionError;

/// Set `node`'s collapsed flag to `value`. Generates `Op::SetCollapsed`
/// with a fresh HLC and applies it through `Workspace::apply`.
///
/// Returns `true` when the materialised state actually changed (the
/// flag flipped), `false` when the op was a no-op against the current
/// state. Either way the op enters the log — the CRDT needs every
/// flip to converge across devices, and idempotent re-apply is cheap.
pub fn set_block_collapsed(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
    value: bool,
) -> Result<bool, ActionError> {
    let was = workspace.tree().is_collapsed(node);
    let ts = hlc.next();
    let op = LogOp {
        ts,
        actor: ts.actor,
        op: Op::SetCollapsed {
            node,
            value,
            // Filled by `do_op`. The value we pass here is irrelevant
            // beyond the type contract.
            old_value: false,
        },
    };
    workspace.apply(op)?;
    Ok(was != value)
}

/// Flip `node`'s collapsed flag. Returns the new value.
pub fn toggle_block_collapsed(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
) -> Result<bool, ActionError> {
    let new_value = !workspace.tree().is_collapsed(node);
    set_block_collapsed(workspace, hlc, node, new_value)?;
    Ok(new_value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_core::id::ActorId;

    fn make_ws() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        let ws = Workspace::open_in_memory(actor).unwrap();
        let hlc = HlcGenerator::new(actor);
        (ws, hlc)
    }

    #[test]
    fn set_then_read_round_trip() {
        let (mut ws, hlc) = make_ws();
        let node = NodeId::new();
        assert!(!ws.tree().is_collapsed(node));
        let changed = set_block_collapsed(&mut ws, &hlc, node, true).unwrap();
        assert!(changed);
        assert!(ws.tree().is_collapsed(node));
    }

    #[test]
    fn set_block_collapsed_reports_no_change_when_already_equal() {
        // Idempotency at the action level: flipping `true` twice
        // reports `false` on the second call even though the op was
        // still appended to the log (the CRDT needs it for cross-
        // device convergence).
        let (mut ws, hlc) = make_ws();
        let node = NodeId::new();
        assert!(set_block_collapsed(&mut ws, &hlc, node, true).unwrap());
        assert!(!set_block_collapsed(&mut ws, &hlc, node, true).unwrap());
        assert!(ws.tree().is_collapsed(node));
    }

    #[test]
    fn toggle_flips_value() {
        let (mut ws, hlc) = make_ws();
        let node = NodeId::new();
        assert!(toggle_block_collapsed(&mut ws, &hlc, node).unwrap());
        assert!(ws.tree().is_collapsed(node));
        assert!(!toggle_block_collapsed(&mut ws, &hlc, node).unwrap());
        assert!(!ws.tree().is_collapsed(node));
    }
}
