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
pub(crate) async fn reload_workspace(state: State<'_, AppState>) -> Result<(), String> {
    // A reload replays the WHOLE op log (`Workspace::open_with_storage`) —
    // O(all ops), which on a freshly-synced workspace is 200k+ ops. This is
    // CPU-bound and runs for seconds. A synchronous `#[tauri::command]`
    // executes on the Tauri IPC/main worker, so doing the replay inline
    // holds that thread through the whole rebuild and iOS fires the
    // scene-update watchdog (>10s → SIGKILL) — the "app freezes forever
    // after pairing" bug. Offload the replay to a blocking pool thread
    // (mirrors the background boot opener, which is why boot never trips
    // the watchdog while this path did) so the WebView keeps painting.
    let storage_root = state.storage_root.clone();
    let hlc = state.hlc.clone();
    let workspace = state.workspace.clone();
    let backlink_index = state.backlink_index.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let engine = outl_actions::SyncEngine::new(storage_root, hlc.actor());
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
        let today_id = open_today(&mut fresh, &hlc).map_err(|e| e.to_string())?;
        let _ = engine.reproject_page(&fresh, today_id);
        *workspace.lock() = Some(fresh);
        // Peer ops replaced the workspace, so the cached backlinks index
        // is stale — drop it; the next `page_backlinks` rebuilds it.
        *backlink_index.lock() = None;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| format!("reload task join: {e}"))?
}
