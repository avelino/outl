//! `outl serve` — watch the workspace and reconcile external edits.

use crate::sync_engine::{reconcile_dir, reconcile_md, ReconcileReport};
use crate::workspace_layout::{ensure_ops_dir, is_workspace_md, read_config, Paths};
use anyhow::{Context, Result};
use notify::RecursiveMode;
use notify_debouncer_full::new_debouncer;
use outl_core::hlc::HlcGenerator;
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use std::path::Path;
use std::sync::mpsc::channel;
use std::time::Duration;
use tracing::{error, info};

/// Run the `serve` subcommand.
///
/// When `once` is true, reconciles every `.md` once and returns —
/// useful for smoke tests and scripting. Otherwise installs a 200 ms
/// debounced file watcher and blocks until interrupted.
///
/// Lock policy mirrors `ws::open` and `outl-tui::open_workspace`:
/// shared workspace lock plus a per-actor write lock resolved through
/// [`outl_core::resolve_write_actor`]. A `serve` running alongside a
/// TUI/MCP server gets an ephemeral actor and writes to its own
/// `ops-<ephemeral>.jsonl`. Without this, both processes would race
/// on `ops-<config>.jsonl` — the very bug 0.5.1 was meant to close.
pub fn run(path: &Path, once: bool) -> Result<()> {
    let paths = Paths::at(path.to_path_buf());
    let cfg =
        read_config(&paths).with_context(|| "workspace config missing — run `outl init` first")?;
    let config_actor = cfg.actor()?;
    // Shared workspace lock — coexists with every other well-behaved
    // `outl` process.
    let _lock = outl_core::WorkspaceLock::acquire(&paths.root).with_context(|| {
        format!(
            "could not acquire workspace lock at {}",
            paths.root.display()
        )
    })?;
    ensure_ops_dir(&paths)?;
    // Exclusive per-actor write lock. Falls back to ephemeral when
    // another process already owns the config actor.
    let (_actor_lock, actor) = outl_core::resolve_write_actor(&paths.ops, config_actor)
        .with_context(|| format!("acquiring per-actor write lock at {}", paths.ops.display()))?;
    if actor != config_actor {
        info!(
            "another outl process owns the config actor {config_actor}; serve writes under ephemeral actor {actor}"
        );
    }
    let storage = JsonlStorage::open(paths.ops.clone(), actor)?;
    let mut ws = Workspace::open_with_storage(actor, Box::new(storage), Some(paths.root.clone()))?;
    let hlc = HlcGenerator::new(actor);

    info!("starting outl serve at {}", paths.root.display());

    // Initial scan: reconcile every .md in pages/ and journals/.
    let initial = initial_scan(&mut ws, &hlc, &paths)?;
    summarize(&initial);

    if once {
        return Ok(());
    }

    // File watcher with 200ms debounce.
    let (tx, rx) = channel();
    let mut debouncer = new_debouncer(Duration::from_millis(200), None, move |res| {
        let _ = tx.send(res);
    })
    .with_context(|| "creating file watcher")?;

    debouncer
        .watch(&paths.pages, RecursiveMode::NonRecursive)
        .with_context(|| format!("watching {}", paths.pages.display()))?;
    debouncer
        .watch(&paths.journals, RecursiveMode::NonRecursive)
        .with_context(|| format!("watching {}", paths.journals.display()))?;

    info!("watching pages/ and journals/ (Ctrl-C to stop)");

    for res in rx {
        match res {
            Ok(events) => {
                let mut paths_to_sync: std::collections::BTreeSet<std::path::PathBuf> =
                    Default::default();
                for ev in events {
                    for p in &ev.event.paths {
                        if is_workspace_md(&paths, p) {
                            paths_to_sync.insert(p.clone());
                        }
                    }
                }
                for p in paths_to_sync {
                    match reconcile_md(&mut ws, &hlc, &paths, &p) {
                        Ok(r) if r.ops_applied > 0 || r.orphans > 0 => {
                            info!(
                                "{} → {} ops, {} orphans, sidecar {}",
                                r.md_path.display(),
                                r.ops_applied,
                                r.orphans,
                                if r.created_sidecar {
                                    "created"
                                } else {
                                    "updated"
                                }
                            );
                        }
                        Ok(_) => {}
                        Err(e) => error!("reconcile failed for {}: {e:#}", p.display()),
                    }
                }
            }
            Err(errs) => {
                for e in errs {
                    error!("watcher error: {e}");
                }
            }
        }
    }

    Ok(())
}

fn initial_scan(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    paths: &Paths,
) -> Result<Vec<ReconcileReport>> {
    let mut all = Vec::new();
    for dir in [&paths.pages, &paths.journals] {
        let mut reports = reconcile_dir(ws, hlc, paths, dir)?;
        all.append(&mut reports);
    }
    Ok(all)
}

fn summarize(reports: &[ReconcileReport]) {
    let mut ops = 0usize;
    let mut orphans = 0usize;
    let mut created = 0usize;
    for r in reports {
        ops += r.ops_applied;
        orphans += r.orphans;
        if r.created_sidecar {
            created += 1;
        }
    }
    info!(
        "initial scan: {} files, {} ops applied, {} orphans, {} new sidecars",
        reports.len(),
        ops,
        orphans,
        created,
    );
}
