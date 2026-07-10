//! Workspace lifecycle commands: reload + stats.

use outl_actions::open_today;
use tauri::State;

use crate::state::{AppState, WorkspaceSummary};

#[tauri::command]
pub(crate) fn workspace_stats(state: State<'_, AppState>) -> WorkspaceSummary {
    let guard = state.workspace.lock();
    let storage_root = state.storage_root.to_string_lossy().into_owned();
    match guard.as_ref() {
        Some(ws) => WorkspaceSummary {
            blocks: ws.tree().node_count(),
            ops: ws.log().len(),
            actor: ws.actor.to_string(),
            storage_root,
            ready: true,
        },
        None => WorkspaceSummary {
            blocks: 0,
            ops: 0,
            actor: String::new(),
            storage_root,
            ready: false,
        },
    }
}

#[tauri::command]
pub(crate) fn reload_workspace(state: State<'_, AppState>) -> Result<(), String> {
    let engine = outl_actions::SyncEngine::new(state.storage_root.clone(), state.hlc.actor());
    let mut fresh = engine
        .reload_workspace()
        .map_err(|e| format!("reload workspace: {e}"))?;
    // NOTE: orphan-`.md` reconcile is a BOOT/recovery concern (it runs
    // md → ops and desync recovery, both of which MUTATE the op log). It
    // used to run here inline on every 3s poll, which — on a page being
    // edited concurrently on two devices while sync ingests peer ops —
    // turned the routine reload into a projection↔op-log feedback loop
    // and made the page flip-flop between the two devices' states. iroh
    // peers ship OPS (not `.md`), so a routine reload only needs to
    // re-materialize the op log; orphan `.md` recovery already runs once
    // at boot (`workspace_open`). Keep the reload a pure re-read.
    // Resolve today's journal *in the fresh workspace* so the page
    // id reflects the merged op log. `open_today` is idempotent —
    // when the page already exists it just returns the id; when it
    // doesn't, it creates one with the deterministic slug-derived
    // id, which both peers will agree on.
    let today_id = open_today(&mut fresh, &state.hlc).map_err(|e| e.to_string())?;
    let _ = engine.reproject_page(&fresh, today_id);
    *state.workspace.lock() = Some(fresh);
    Ok(())
}
