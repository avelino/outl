//! `outl migrate-to-per-page-ops <path>` — Phase B of RFC #137.
//!
//! Reads the legacy `ops/ops-<actor>.jsonl` and splits it into
//! `ops/<actor>/<slug>.jsonl` per page. The original file is preserved
//! as `ops/ops-<actor>.jsonl.v0.bak` so the migration is reversible.
//!
//! Idempotent: a second run is a no-op once the legacy file is gone.
//!
//! Routing rule: each op carries a `node: NodeId`. We resolve that
//! node to its page's slug by walking the materialized tree (the
//! Workspace is opened on the legacy storage to do this) and looking
//! up the root id in the `WorkspaceIndex` (which knows the slug of
//! every `.md` file under `pages/` and `journals/`). Ops whose node
//! can't be resolved (orphan, root, deleted-before-boot) land in the
//! `_default` bucket so nothing is lost.

use crate::workspace_layout::{read_config, Paths};
use anyhow::{bail, Context, Result};
use outl_core::id::NodeId;
use outl_core::op::LogOp;
use outl_core::storage::JsonlStorage;
use outl_core::Workspace;
use outl_md::sidecar;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

/// Run the `migrate-to-per-page-ops` subcommand.
pub fn run(path: &std::path::Path) -> Result<()> {
    let paths = Paths::at(path.to_path_buf());
    let cfg = read_config(&paths)?;
    let actor = cfg.actor()?;

    let legacy_path = paths.ops.join(format!("ops-{actor}.jsonl"));
    if !legacy_path.exists() {
        bail!(
            "no legacy op log at {} — already migrated or never initialised",
            legacy_path.display()
        );
    }

    // Build a `NodeId → slug` map by reading every `.outl` sidecar
    // under `pages/` and `journals/`. The sidecar's `page_id` is the
    // root block id of that page; the slug is the filename without
    // extension.
    let root_to_slug =
        build_root_to_slug_map(&paths).with_context(|| "building page-root → slug map")?;

    println!(
        "Loaded {} page+journal roots from sidecars.",
        root_to_slug.len()
    );

    // Open the legacy storage so the materialised tree is in RAM. We
    // need it to walk parent chains from any node up to its page root.
    let ws = Workspace::open_with_storage(
        actor,
        Box::new(
            JsonlStorage::open(paths.ops.clone(), actor)
                .with_context(|| format!("opening legacy storage at {}", paths.ops.display()))?,
        ),
        Some(paths.root.clone()),
    )
    .with_context(|| "opening workspace for migration")?;
    let tree = ws.tree().clone();
    drop(ws);

    // Stream the legacy file once, bucket ops by slug, write each
    // bucket into its `ops/<actor>/<slug>.jsonl`.
    let buckets = bucket_ops_by_slug(&legacy_path, &tree, &root_to_slug)?;
    if buckets.is_empty() {
        println!("No ops to migrate at {}", legacy_path.display());
        return Ok(());
    }

    let actor_dir = paths.ops.join(actor.to_string());
    fs::create_dir_all(&actor_dir).with_context(|| format!("creating {}", actor_dir.display()))?;

    let mut total_written = 0usize;
    let mut per_bucket: Vec<(String, usize)> = buckets
        .iter()
        .map(|(slug, ops)| (slug.clone(), ops.len()))
        .collect();
    per_bucket.sort_by_key(|b| std::cmp::Reverse(b.1));
    for (slug, ops) in &buckets {
        let target_path = actor_dir.join(format!("{slug}.jsonl"));
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&target_path)
            .with_context(|| format!("opening {}", target_path.display()))?;
        for op in ops {
            let line =
                serde_json::to_string(op).with_context(|| format!("serialising op {:?}", op.ts))?;
            use std::io::Write;
            writeln!(file, "{line}")
                .with_context(|| format!("writing {}", target_path.display()))?;
        }
        total_written += ops.len();
    }

    println!("\nBuckets written:");
    for (slug, count) in &per_bucket {
        println!("  {slug:<40} {count:>6} ops");
    }

    // Backup the legacy file. Renamed, not deleted, so the migration
    // is reversible: rename back, delete the per-actor dir, you're
    // back on Global.
    let backup_path = legacy_path.with_extension("jsonl.v0.bak");
    fs::rename(&legacy_path, &backup_path).with_context(|| {
        format!(
            "backing up {} → {}",
            legacy_path.display(),
            backup_path.display()
        )
    })?;

    println!(
        "\nMigrated {} ops across {} pages. Backup at {}.",
        total_written,
        buckets.len(),
        backup_path.display()
    );
    println!(
        "Run `outl serve --once --workspace {}` to verify the new layout.",
        paths.root.display()
    );
    Ok(())
}

/// Stream the legacy `.jsonl` once and bucket each op by the slug of
/// its node's owning page.
///
/// Walks `tree.parent(node)` up to the page root, then looks up the
/// root id in `root_to_slug`. Ops that can't be resolved (orphan,
/// root, deleted-before-boot) land in `_default`.
fn bucket_ops_by_slug(
    legacy_path: &PathBuf,
    tree: &outl_core::tree::Tree,
    root_to_slug: &HashMap<NodeId, String>,
) -> Result<HashMap<String, Vec<LogOp>>> {
    let file = fs::File::open(legacy_path)
        .with_context(|| format!("opening {}", legacy_path.display()))?;
    let reader = BufReader::new(file);
    let mut buckets: HashMap<String, Vec<LogOp>> = HashMap::new();
    let mut unresolved = 0usize;
    let mut total = 0usize;
    for (lineno, line) in reader.lines().enumerate() {
        let raw = match line {
            Ok(l) if !l.is_empty() => l,
            Ok(_) => continue,
            Err(e) => {
                bail!("read {}:{}: {e}", legacy_path.display(), lineno + 1);
            }
        };
        let stream = serde_json::Deserializer::from_str(&raw).into_iter::<LogOp>();
        for op in stream {
            let op =
                op.with_context(|| format!("parse {}:{}", legacy_path.display(), lineno + 1))?;
            total += 1;
            let node = outl_core::op::op_node(&op.op).unwrap_or(NodeId::root());
            let slug = resolve_page_slug(node, tree, root_to_slug);
            if slug.is_none() {
                unresolved += 1;
            }
            buckets
                .entry(slug.unwrap_or_else(|| "_default".to_string()))
                .or_default()
                .push(op);
        }
    }
    if unresolved > 0 {
        println!(
            "  {unresolved}/{total} ops couldn't be resolved to a page — landing in _default."
        );
    }
    Ok(buckets)
}

/// Walk `tree.parent(node)` up until we hit a node whose id is in
/// `root_to_slug`. Returns the slug of that page, or `None` if the
/// chain dead-ends at the workspace root without finding a page.
fn resolve_page_slug(
    mut node: NodeId,
    tree: &outl_core::tree::Tree,
    root_to_slug: &HashMap<NodeId, String>,
) -> Option<String> {
    loop {
        if let Some(slug) = root_to_slug.get(&node) {
            return Some(slug.clone());
        }
        node = tree.parent(node)?;
    }
}

/// Read every `.outl` sidecar under `pages/` and `journals/` and build
/// a `NodeId → slug` map. The sidecar's `page_id` field is the root
/// block id of the page; the slug is the filename without extension.
///
/// Sidecars that fail to parse are skipped (logged) — they'll be
/// rebuilt on the next `outl serve --once` after migration.
fn build_root_to_slug_map(paths: &Paths) -> Result<HashMap<NodeId, String>> {
    let mut map = HashMap::new();
    for (dir, _is_journal) in [(paths.pages.clone(), false), (paths.journals.clone(), true)] {
        if !dir.is_dir() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&dir).max_depth(1) {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let sidecar_path = path.with_extension("outl");
            let slug = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            match sidecar::read(&sidecar_path) {
                Ok(sc) => {
                    map.insert(sc.page_id, slug);
                }
                Err(e) => {
                    println!("  skip sidecar {}: {e}", sidecar_path.display());
                }
            }
        }
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_core::fractional::Fractional;
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::ActorId;
    use outl_core::op::{LogOp, Op};
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    fn mk_create(g: &HlcGenerator) -> LogOp {
        let ts = g.next();
        LogOp {
            ts,
            actor: ts.actor,
            op: Op::Create {
                node: NodeId::new(),
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        }
    }

    #[test]
    fn bucket_ops_returns_one_per_line() {
        let tmp = TempDir::new().unwrap();
        let actor = ActorId::new();
        let g = HlcGenerator::new(actor);
        let path = tmp.path().join(format!("ops-{actor}.jsonl"));
        let ops: Vec<LogOp> = (0..3).map(|_| mk_create(&g)).collect();
        let mut file = fs::File::create(&path).unwrap();
        for op in &ops {
            writeln!(file, "{}", serde_json::to_string(op).unwrap()).unwrap();
        }
        drop(file);

        let tree = outl_core::tree::Tree::new();
        let empty_roots: HashMap<NodeId, String> = HashMap::new();
        let buckets = bucket_ops_by_slug(&path, &tree, &empty_roots).unwrap();
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets["_default"].len(), 3);
    }
}
