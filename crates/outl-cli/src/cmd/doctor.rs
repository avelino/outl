//! `outl doctor` — workspace integrity check.
//!
//! Reports problems without fixing them — `outl reconcile` and editor
//! workflows are the canonical fix paths. Doctor is read-only.
//!
//! The check pipeline is exposed as [`collect`], returning a
//! serializable [`DoctorReport`]. The CLI surface ([`run`] for human
//! output, [`run_json`] for `--json`) wraps it, and the MCP shim
//! routes `outl_workspace_doctor` straight into [`collect_json`].

use crate::output::{emit, ApiError};
use crate::workspace_layout::{read_config, Paths};
use anyhow::{Context, Result};
use outl_core::storage::{JsonlStorage, Storage};
use outl_md::index::WorkspaceIndex;
use outl_md::inline::{tokenize, InlineTok};
use outl_md::sidecar::{self, sidecar_path_for};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

/// Severity of a single finding.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Healthy state — no action needed.
    Ok,
    /// Informational, not a problem.
    Info,
    /// Possible problem, user should look.
    Warn,
    /// Definite problem, blocks "integrity OK".
    Error,
}

/// One workspace check result.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// Severity bucket.
    pub severity: Severity,
    /// Human-readable description.
    pub message: String,
}

/// Aggregate of every finding from a doctor run.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    /// Workspace root path (display form).
    pub workspace: String,
    /// Actor id from `config.toml`.
    pub actor: String,
    /// Number of ops in the persisted log.
    pub op_count: usize,
    /// Every check emitted, in execution order.
    pub findings: Vec<Finding>,
    /// Convenience counts so callers don't have to count.
    pub error_count: usize,
    /// Number of warning findings.
    pub warn_count: usize,
}

struct Builder {
    workspace: String,
    actor: String,
    op_count: usize,
    findings: Vec<Finding>,
    errors: usize,
    warnings: usize,
}

impl Builder {
    fn new(workspace: String, actor: String) -> Self {
        Self {
            workspace,
            actor,
            op_count: 0,
            findings: Vec::new(),
            errors: 0,
            warnings: 0,
        }
    }
    fn push(&mut self, severity: Severity, message: impl Into<String>) {
        if matches!(severity, Severity::Error) {
            self.errors += 1;
        }
        if matches!(severity, Severity::Warn) {
            self.warnings += 1;
        }
        self.findings.push(Finding {
            severity,
            message: message.into(),
        });
    }
    fn ok(&mut self, msg: impl Into<String>) {
        self.push(Severity::Ok, msg);
    }
    fn info(&mut self, msg: impl Into<String>) {
        self.push(Severity::Info, msg);
    }
    fn warn(&mut self, msg: impl Into<String>) {
        self.push(Severity::Warn, msg);
    }
    fn err(&mut self, msg: impl Into<String>) {
        self.push(Severity::Error, msg);
    }
    fn into_report(self) -> DoctorReport {
        DoctorReport {
            workspace: self.workspace,
            actor: self.actor,
            op_count: self.op_count,
            findings: self.findings,
            error_count: self.errors,
            warn_count: self.warnings,
        }
    }
}

/// Run every doctor check and return a structured report. Used by the
/// CLI human path and the `--json` flag — those acquire the workspace
/// lock fresh, so the lock probe at the end can tell apart "free" /
/// "held by another outl process". The MCP shim uses [`collect_in_session`]
/// because it is already holding the lock through its cached `WsCtx`,
/// so the probe would always return `AlreadyHeld` and lie.
pub fn collect(path: &Path) -> Result<DoctorReport, ApiError> {
    collect_internal(path, true)
}

/// Same as [`collect`] but skips the workspace-lock probe.
///
/// The MCP server holds the lock for its whole session through the
/// cached `WsCtx`, so a second `WorkspaceLock::acquire` from inside
/// the same process would always return `AlreadyHeld` and the probe
/// would always report a non-existent contention. Skipping the probe
/// keeps the doctor signal honest when invoked from within a long-
/// running process.
pub fn collect_in_session(path: &Path) -> Result<DoctorReport, ApiError> {
    collect_internal(path, false)
}

fn collect_internal(path: &Path, probe_lock: bool) -> Result<DoctorReport, ApiError> {
    let paths = Paths::at(path.to_path_buf());
    let cfg = read_config(&paths).map_err(|e| {
        ApiError::new(
            crate::output::codes::NO_WORKSPACE,
            format!("workspace config missing — run `outl init` first ({e})"),
        )
    })?;
    let actor = cfg.actor().map_err(|e| {
        ApiError::new(
            crate::output::codes::INTERNAL,
            format!("invalid actor in config: {e}"),
        )
    })?;
    let mut b = Builder::new(paths.root.display().to_string(), cfg.workspace.actor_id);

    // 1. Op log readability. `JsonlStorage::open` already skips
    //    malformed lines and emits them through `tracing::warn!`
    //    (intentional, so a single bad tail line in `ops-*.jsonl`
    //    can't lock the user out of the workspace). We only surface
    //    here the harder failures the open itself returns: missing
    //    `ops/` directory, permission denied, unreadable header,
    //    storage backend errors. Parse warnings live in the
    //    `tracing` sink the caller chose. Future enhancement: have
    //    `JsonlStorage::open` expose a `parse_warnings()` accessor
    //    so doctor can attach them to the report.
    let known_node_ids: HashSet<outl_core::id::NodeId> =
        match JsonlStorage::open(paths.ops.clone(), actor) {
            Ok(storage) => match storage.all_ops() {
                Ok(ops) => {
                    b.op_count = ops.len();
                    b.ok(format!("op log has {} ops", ops.len()));
                    let mut ids = HashSet::new();
                    for op in &ops {
                        let node = match &op.op {
                            outl_core::op::Op::Move { node, .. }
                            | outl_core::op::Op::Edit { node, .. }
                            | outl_core::op::Op::SetProp { node, .. }
                            | outl_core::op::Op::Create { node, .. } => *node,
                        };
                        ids.insert(node);
                    }
                    ids
                }
                Err(e) => {
                    b.err(format!("could not read op log: {e}"));
                    HashSet::new()
                }
            },
            Err(e) => {
                b.err(format!(
                    "could not open ops dir at {}: {e}",
                    paths.ops.display()
                ));
                HashSet::new()
            }
        };

    // 3. Pages and journals: `.md` ↔ sidecar pairing.
    for dir in [&paths.pages, &paths.journals] {
        if !dir.is_dir() {
            continue;
        }
        let mut md_files = Vec::new();
        let mut sidecar_files = Vec::new();
        for entry in walkdir::WalkDir::new(dir).max_depth(1) {
            let Ok(entry) = entry else { continue };
            let p = entry.path();
            if !entry.file_type().is_file() {
                continue;
            }
            let name = match p.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if name.starts_with('.') {
                if name.ends_with(".outl") {
                    sidecar_files.push(p.to_path_buf());
                }
                continue;
            }
            if p.extension().and_then(|x| x.to_str()) == Some("md") {
                md_files.push(p.to_path_buf());
            }
        }
        check_md_files(&mut b, &md_files, &known_node_ids);
        check_orphan_sidecars(&mut b, &sidecar_files, &md_files);
    }

    // 4. Block ref integrity — every `((blk-XXXXXX))` mentioned must
    //    resolve to an indexed block. Build the index once.
    let workspace_index = WorkspaceIndex::build(&paths.root);
    check_orphan_block_refs(&mut b, &workspace_index);

    // 5. Orphan log presence (informational).
    if paths.orphans.exists() {
        let bytes = std::fs::metadata(&paths.orphans)
            .map(|m| m.len())
            .unwrap_or(0);
        if bytes == 0 {
            b.ok("orphans.log is empty");
        } else {
            b.info(format!(
                "orphans.log has {bytes} bytes — run `outl reconcile` to triage"
            ));
        }
    }

    // 6. Lock file: warn if held by another process.
    //    Skipped when running inside an outl process that already
    //    holds the lock (e.g. MCP server) — `AlreadyHeld` would just
    //    report itself.
    if probe_lock {
        match outl_core::WorkspaceLock::acquire(&paths.root) {
            Ok(_lock) => b.ok("workspace lock is free (no other outl process attached)"),
            Err(outl_core::LockError::AlreadyHeld(_)) => {
                b.warn("another outl process is holding the workspace lock");
            }
            Err(e) => b.warn(format!("could not test workspace lock: {e}")),
        }
    } else {
        b.info("workspace lock probe skipped (running inside an outl session)");
    }

    Ok(b.into_report())
}

/// CLI `--json` entry point — returns the report as JSON `data` with
/// the full lock probe enabled, matching the human `outl doctor`
/// behaviour. The MCP tool uses [`collect_in_session_json`] instead.
pub fn collect_json(path: &Path) -> Result<Value, ApiError> {
    let report = collect(path)?;
    serde_json::to_value(&report).map_err(ApiError::internal)
}

/// MCP entry point — returns the report as JSON `data` without the
/// workspace-lock probe. The MCP shim already owns the lock for the
/// session, so a fresh `acquire` would always report contention
/// against itself.
pub fn collect_in_session_json(path: &Path) -> Result<Value, ApiError> {
    let report = collect_in_session(path)?;
    serde_json::to_value(&report).map_err(ApiError::internal)
}

/// CLI entry point with human output. Exits with status 1 when the
/// report has errors so scripts can detect failure.
pub fn run(path: &Path) -> Result<()> {
    let report = collect(path).with_context(|| format!("running doctor on {}", path.display()))?;
    println!("workspace: {}", report.workspace);
    println!("actor:     {}", report.actor);
    println!();
    for finding in &report.findings {
        let tag = match finding.severity {
            Severity::Ok => "ok:  ",
            Severity::Info => "info:",
            Severity::Warn => "warn:",
            Severity::Error => "err: ",
        };
        println!("{tag} {}", finding.message);
    }
    println!();
    match (report.error_count, report.warn_count) {
        (0, 0) => println!("integrity OK"),
        (0, w) => println!("integrity OK with {w} warning(s)"),
        (e, w) => {
            println!("{e} error(s), {w} warning(s) — see lines above");
            std::process::exit(1);
        }
    }
    Ok(())
}

/// `outl doctor --json` shape — emits the envelope and exits 1 when
/// the report has errors.
pub fn run_json(path: &Path) -> i32 {
    let result = collect_json(path);
    let exit = emit(true, result.clone(), |_| {});
    // `emit` already used the JSON branch; force an error exit when
    // the report itself carried errors even though the call succeeded.
    if exit == 0
        && result
            .ok()
            .and_then(|v| v.get("error_count").and_then(|n| n.as_u64()))
            .unwrap_or(0)
            > 0
    {
        return 1;
    }
    exit
}

fn check_md_files(
    b: &mut Builder,
    md_files: &[std::path::PathBuf],
    known_node_ids: &HashSet<outl_core::id::NodeId>,
) {
    for md in md_files {
        let scp = sidecar_path_for(md);
        if !scp.exists() {
            b.warn(format!(
                "{}: no sidecar (next `outl serve` or TUI commit will create one)",
                md.display()
            ));
            continue;
        }
        match sidecar::read(&scp) {
            Ok(sc) if sc.version == sidecar::SIDECAR_VERSION => {
                let mut unknown = 0;
                for sb in &sc.blocks {
                    if !known_node_ids.is_empty() && !known_node_ids.contains(&sb.id) {
                        unknown += 1;
                    }
                }
                if unknown == 0 {
                    b.ok(format!(
                        "{} (sidecar v{}, {} blocks, all IDs known)",
                        md.display(),
                        sc.version,
                        sc.blocks.len()
                    ));
                } else {
                    b.warn(format!(
                        "{}: {} block id(s) in sidecar not present in op log (workspace partially de-synced)",
                        md.display(),
                        unknown
                    ));
                }
            }
            Ok(sc) => {
                b.warn(format!(
                    "{}: sidecar version {} unsupported by this build",
                    md.display(),
                    sc.version
                ));
            }
            Err(e) => {
                b.err(format!("{}: sidecar unreadable: {e}", md.display()));
            }
        }
    }
}

/// Walk every indexed block, tokenize its text, and warn for every
/// `((blk-XXXXXX))` or `!((blk-XXXXXX))` whose handle does not resolve
/// to an indexed block.
fn check_orphan_block_refs(b: &mut Builder, idx: &WorkspaceIndex) {
    let mut orphans = 0usize;
    for block in idx.iter_blocks() {
        for tok in tokenize(&block.text) {
            let (handle, literal) = match tok {
                InlineTok::BlockRef { handle } => (handle, format!("(({handle}))")),
                InlineTok::Embed { handle } => (handle, format!("!(({handle}))")),
                _ => continue,
            };
            if idx.resolve_block_ref(handle).is_none() {
                orphans += 1;
                b.warn(format!(
                    "{}: orphan block ref {} — source block missing or not indexed",
                    block.source_path.display(),
                    literal,
                ));
            }
        }
    }
    if orphans == 0 {
        b.ok("no orphan ((blk-XXXXXX)) / !((blk-XXXXXX)) references");
    }
}

fn check_orphan_sidecars(
    b: &mut Builder,
    sidecar_files: &[std::path::PathBuf],
    md_files: &[std::path::PathBuf],
) {
    let md_names: HashSet<String> = md_files
        .iter()
        .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(String::from))
        .collect();
    for scp in sidecar_files {
        let Some(name) = scp.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(stripped) = name.strip_prefix('.').and_then(|s| s.strip_suffix(".outl")) else {
            continue;
        };
        if !md_names.contains(stripped) {
            b.warn(format!(
                "{}: orphaned sidecar (no matching {} on disk)",
                scp.display(),
                stripped
            ));
        }
    }
}
