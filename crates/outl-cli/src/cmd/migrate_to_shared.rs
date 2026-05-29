//! `outl migrate-to-shared <path>` — copy ops from a workspace's local
//! SQLite log into a shared JSONL log so peers (mobile, future desktop)
//! can read them via iCloud / Syncthing / shared folder.
//!
//! When a workspace started life as a local-only TUI session and later
//! gets sym-linked or moved into a synced directory (e.g. the iCloud
//! Ubiquity Container of the mobile app), only the new ops written after
//! the move land in `.ops/`. Anything that was committed earlier still
//! lives in `.outl/log.db` and is invisible to peers.
//!
//! This subcommand reads every op out of the SQLite log, opens (or
//! creates) the `.ops/` directory, and appends every op authored by the
//! local actor to `ops-<actor>.jsonl`. Ops from other actors (rare —
//! only present if you ever ran `outl import`) are reported and skipped
//! because each actor owns exactly one JSONL file.
//!
//! The migration is **idempotent**: it skips ops that already exist in
//! the JSONL log (matched by HLC timestamp), so it's safe to re-run.
//! It also leaves `.outl/log.db` intact — delete it yourself once you've
//! verified the mobile / desktop peers see what they should.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use outl_core::hlc::Hlc;
use outl_core::id::ActorId;
use outl_core::storage::{JsonlStorage, SqliteStorage, Storage};

use crate::workspace_layout::Paths;

/// Run the `migrate-to-shared` subcommand.
pub fn run(path: &Path) -> Result<()> {
    let paths = Paths::at(path.to_path_buf());

    if !paths.db.is_file() {
        anyhow::bail!(
            "no local op log to migrate at {} — workspace already uses the shared backend?",
            paths.db.display()
        );
    }

    let actor = read_actor_id(&paths)?;

    let sqlite = SqliteStorage::open(&paths.db)
        .with_context(|| format!("opening sqlite at {}", paths.db.display()))?;
    let ops = sqlite
        .all_ops()
        .with_context(|| format!("reading ops from {}", paths.db.display()))?;

    let ops_dir = paths.root.join("ops");
    let mut jsonl = JsonlStorage::open(ops_dir.clone(), actor)
        .with_context(|| format!("opening jsonl at {}", ops_dir.display()))?;

    let existing: HashSet<Hlc> = jsonl
        .all_ops()
        .with_context(|| format!("listing existing ops in {}", ops_dir.display()))?
        .into_iter()
        .map(|o| o.ts)
        .collect();

    let mut migrated = 0usize;
    let mut skipped_dup = 0usize;
    let mut skipped_foreign = 0usize;

    for op in ops {
        if op.actor != actor {
            skipped_foreign += 1;
            continue;
        }
        if existing.contains(&op.ts) {
            skipped_dup += 1;
            continue;
        }
        jsonl
            .append_op(&op)
            .with_context(|| format!("appending op at ts={:?} to {}", op.ts, ops_dir.display()))?;
        migrated += 1;
    }

    println!(
        "Migrated {migrated} op{} from {} to {}",
        plural(migrated),
        paths.db.display(),
        ops_dir.display(),
    );
    if skipped_dup > 0 {
        println!(
            "  skipped {skipped_dup} duplicate{} (already present in .ops/)",
            plural(skipped_dup)
        );
    }
    if skipped_foreign > 0 {
        println!(
            "  skipped {skipped_foreign} op{} from other actors — they own separate jsonl files",
            plural(skipped_foreign)
        );
    }
    println!(
        "\nLeft {} intact. Delete it yourself once you've verified peers see what they should.",
        paths.db.display()
    );

    Ok(())
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn read_actor_id(paths: &Paths) -> Result<ActorId> {
    let cfg_raw = std::fs::read_to_string(&paths.config)
        .with_context(|| format!("reading config at {}", paths.config.display()))?;
    let cfg: toml::Value = toml::from_str(&cfg_raw).context("parsing config.toml")?;
    let actor_str = cfg
        .get("workspace")
        .and_then(|w| w.get("actor_id"))
        .and_then(|a| a.as_str())
        .context("workspace.actor_id missing from config.toml")?;
    let actor_ulid = ulid::Ulid::from_string(actor_str).context("actor_id is not a valid ULID")?;
    Ok(ActorId(actor_ulid))
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_core::fractional::Fractional;
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::NodeId;
    use outl_core::op::{LogOp, Op};
    use tempfile::TempDir;

    fn seed_workspace(root: &Path, actor: ActorId) -> Paths {
        let paths = Paths::at(root.to_path_buf());
        crate::workspace_layout::init(&paths).unwrap();
        // Overwrite the auto-generated actor_id so the test controls it.
        let cfg = format!("[workspace]\nactor_id = \"{}\"\n", actor.0.to_string());
        std::fs::write(&paths.config, cfg).unwrap();
        paths
    }

    #[test]
    fn migrates_local_actor_ops_into_jsonl() {
        let tmp = TempDir::new().unwrap();
        let actor = ActorId::new();
        let paths = seed_workspace(tmp.path(), actor);

        // Seed a few ops in the SQLite log.
        let g = HlcGenerator::new(actor);
        let mut sqlite = SqliteStorage::open(&paths.db).unwrap();
        for _ in 0..3 {
            let ts = g.next();
            sqlite
                .append_op(&LogOp {
                    ts,
                    actor,
                    op: Op::Create {
                        node: NodeId::new(),
                        parent: NodeId::root(),
                        position: Fractional::first(),
                    },
                })
                .unwrap();
        }
        drop(sqlite);

        run(&paths.root).unwrap();

        let jsonl = JsonlStorage::open(paths.root.join("ops"), actor).unwrap();
        assert_eq!(jsonl.all_ops().unwrap().len(), 3);
    }

    #[test]
    fn rerun_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let actor = ActorId::new();
        let paths = seed_workspace(tmp.path(), actor);

        let g = HlcGenerator::new(actor);
        let mut sqlite = SqliteStorage::open(&paths.db).unwrap();
        let ts = g.next();
        sqlite
            .append_op(&LogOp {
                ts,
                actor,
                op: Op::Create {
                    node: NodeId::new(),
                    parent: NodeId::root(),
                    position: Fractional::first(),
                },
            })
            .unwrap();
        drop(sqlite);

        run(&paths.root).unwrap();
        run(&paths.root).unwrap();

        let jsonl = JsonlStorage::open(paths.root.join("ops"), actor).unwrap();
        assert_eq!(jsonl.all_ops().unwrap().len(), 1);
    }

    #[test]
    fn skips_ops_from_foreign_actors() {
        let tmp = TempDir::new().unwrap();
        let me = ActorId::new();
        let them = ActorId::new();
        let paths = seed_workspace(tmp.path(), me);

        let g_me = HlcGenerator::new(me);
        let g_them = HlcGenerator::new(them);
        let mut sqlite = SqliteStorage::open(&paths.db).unwrap();
        for actor in [me, them, me] {
            let ts = if actor == me {
                g_me.next()
            } else {
                g_them.next()
            };
            sqlite
                .append_op(&LogOp {
                    ts,
                    actor,
                    op: Op::Create {
                        node: NodeId::new(),
                        parent: NodeId::root(),
                        position: Fractional::first(),
                    },
                })
                .unwrap();
        }
        drop(sqlite);

        run(&paths.root).unwrap();

        let jsonl = JsonlStorage::open(paths.root.join("ops"), me).unwrap();
        // Only our two ops survive — the foreign one stays in sqlite.
        assert_eq!(jsonl.all_ops().unwrap().len(), 2);
    }
}
