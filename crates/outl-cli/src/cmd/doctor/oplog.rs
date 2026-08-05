//! Op-log-side integrity checks.
//!
//! Three things live here, all of them about bytes on disk rather than
//! about the materialized tree:
//!
//! 1. [`check_jsonl_lines`] — a raw, line-by-line sweep of every
//!    `ops-*.jsonl`. `JsonlStorage::open` deliberately *skips* malformed
//!    records so one torn tail line can't lock a user out of the
//!    workspace, and it reports them only through `tracing::warn!`. That
//!    is the right boot behaviour and the wrong migration behaviour: a
//!    user moving a 66k-block graph in needs to know **which line** of
//!    **which file** was lost. The record framing here mirrors
//!    `storage::jsonl::read_log_record` (read to `\n`, tolerate non-UTF8,
//!    stream concatenated JSON values off a glued line) because that is
//!    the framing whose defects we must name; the private helper isn't
//!    reachable from this crate and the doctor needs per-line
//!    positions the storage layer never surfaces.
//! 2. [`check_snapshots`] — `.outl/snapshots/snap-<actor>.bin` decode +
//!    hash verification via [`SnapshotBody::decode`].
//! 3. [`check_offset_indexes`] — every offset in `.ops-<actor>.idx` must
//!    point at the first byte of a real record in its `.jsonl`.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use outl_core::op::LogOp;
use outl_core::snapshot::SnapshotBody;
use outl_core::storage::{ActorIndex, OffsetIndex};

use super::Builder;

/// Cap on how many individual bad lines we name per file. A file that
/// is corrupt end-to-end would otherwise print thousands of rows.
const MAX_REPORTED_LINES: usize = 20;

/// Whether the op log can still be treated as the **complete** source of
/// truth for this workspace.
///
/// Everything downstream of the log is a projection of it: the
/// materialized tree, and every `.md` rendered from that tree. When a
/// record is unreadable, `JsonlStorage` skips it *by design* — one torn
/// tail line must never lock a user out of their workspace — so the tree
/// boots **truncated** and then looks perfectly healthy from the
/// inside. Nothing in the tree remembers what was lost.
///
/// That is why the reasons are collected rather than counted. A repair
/// that writes a projection out of a truncated tree overwrites good user
/// content with an incomplete render, so the doctor has to be able to
/// say *what* to recover before it is allowed to write anything.
#[derive(Debug, Default)]
pub(super) struct OpLogHealth {
    /// One entry per distinct reason the replayed tree may be missing
    /// ops. Empty means the log read back whole.
    pub compromised_by: Vec<String>,
}

impl OpLogHealth {
    /// True when at least one op may be missing from the replayed tree.
    pub fn is_compromised(&self) -> bool {
        !self.compromised_by.is_empty()
    }

    /// Record a reason, deduplicated so a directory full of the same
    /// defect reads as one line.
    pub fn compromise(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        if !self.compromised_by.contains(&reason) {
            self.compromised_by.push(reason);
        }
    }
}

/// One physical record's verdict during the raw sweep.
enum LineVerdict {
    /// Parsed cleanly into exactly one op.
    Clean,
    /// Blank line — benign, the appender leaves them on some crashes.
    Blank,
    /// Several ops recovered from one physical line (two writers'
    /// `write_all`s interleaved). Recoverable, but a real defect.
    Glued(usize),
    /// Nothing usable on this line. `String` is the human reason.
    Bad(String),
}

/// Result of sweeping one `.jsonl`: the byte offset of every record we
/// could frame, so the offset-index check can verify its pointers.
pub(super) struct FileScan {
    /// Byte offset of the first byte of every physical record.
    pub record_offsets: BTreeSet<u64>,
    /// Total bytes in the file.
    pub size: u64,
    /// Ops successfully parsed out of the file.
    pub ops: usize,
}

/// Every `*.jsonl` under `ops_dir`, recursively.
///
/// Top-level `ops-<actor>.jsonl` is the Global scope layout; the
/// per-page layout nests `<actor>/<slug>.jsonl`, so the walk is
/// recursive rather than `read_dir`.
pub(super) fn jsonl_files(ops_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(ops_dir).max_depth(3) {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        let p = entry.path();
        if p.extension().and_then(|x| x.to_str()) == Some("jsonl") {
            out.push(p.to_path_buf());
        }
    }
    out.sort();
    out
}

/// Sweep every op-log file line by line and report each defective
/// record with its line number and reason.
///
/// Returns the per-file scan so [`check_offset_indexes`] can reuse the
/// record offsets instead of reading the files a second time.
///
/// Every defect that costs an op — an unreadable file, an I/O error
/// mid-scan, a line carrying no usable op — is also recorded on
/// `health`, because those are exactly the cases where the tree replays
/// short and no repair may write a projection from it.
pub(super) fn check_jsonl_lines(
    b: &mut Builder,
    ops_dir: &Path,
    health: &mut OpLogHealth,
) -> Vec<(PathBuf, FileScan)> {
    let files = jsonl_files(ops_dir);
    if files.is_empty() {
        b.info(format!("no `*.jsonl` op log under {}", ops_dir.display()));
        return Vec::new();
    }

    let mut scans = Vec::new();
    let mut total_bad = 0usize;
    let mut total_glued = 0usize;

    for path in files {
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                b.err(format!("{}: unreadable op log: {e}", path.display()));
                health.compromise(format!(
                    "{} could not be opened, so none of its ops were replayed",
                    path.display()
                ));
                continue;
            }
        };
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        let mut reader = BufReader::new(file);
        let mut buf: Vec<u8> = Vec::new();
        let mut offset = 0u64;
        let mut line_no = 0usize;
        let mut record_offsets = BTreeSet::new();
        let mut ops = 0usize;
        let mut bad: Vec<String> = Vec::new();
        let mut glued = 0usize;
        let mut io_error: Option<String> = None;

        loop {
            buf.clear();
            let n = match reader.read_until(b'\n', &mut buf) {
                Ok(n) => n,
                Err(e) => {
                    io_error = Some(e.to_string());
                    break;
                }
            };
            if n == 0 {
                break;
            }
            line_no += 1;
            record_offsets.insert(offset);
            match classify_record(&buf) {
                LineVerdict::Clean => ops += 1,
                LineVerdict::Blank => {}
                LineVerdict::Glued(count) => {
                    ops += count;
                    glued += 1;
                    if bad.len() < MAX_REPORTED_LINES {
                        bad.push(format!(
                            "line {line_no} (byte {offset}): {count} ops glued onto one line \
                             — concurrent unsynchronized append; recovered"
                        ));
                    }
                }
                LineVerdict::Bad(reason) => {
                    if bad.len() < MAX_REPORTED_LINES {
                        bad.push(format!("line {line_no} (byte {offset}): {reason}"));
                    }
                    total_bad += 1;
                }
            }
            offset += n as u64;
        }

        if let Some(e) = io_error {
            b.err(format!(
                "{}: I/O error while scanning at byte {offset}: {e}",
                path.display()
            ));
            health.compromise(format!(
                "{} stopped reading at byte {offset}, so every op past it is unaccounted for",
                path.display()
            ));
        }

        total_glued += glued;
        // Count the unrecoverable lines for THIS file separately from
        // the glued ones, which are only warnings.
        let hard_bad = bad
            .iter()
            .filter(|l| !l.contains("glued onto one line"))
            .count();
        for line in &bad {
            if line.contains("glued onto one line") {
                b.warn(format!("{}: {line}", path.display()));
            } else {
                b.err(format!("{}: {line}", path.display()));
            }
        }
        if hard_bad + glued > MAX_REPORTED_LINES {
            b.warn(format!(
                "{}: … and more defective lines beyond the first {MAX_REPORTED_LINES}",
                path.display()
            ));
        }
        scans.push((
            path,
            FileScan {
                record_offsets,
                size,
                ops,
            },
        ));
    }

    match (total_bad, total_glued) {
        (0, 0) => b.ok("every op-log line parses — no corrupt records"),
        (0, g) => b.warn(format!(
            "{g} glued op-log line(s) — every op was recovered, but two writers raced on one file"
        )),
        (bad, _) => {
            b.err(format!(
                "{bad} op-log line(s) carry no usable op — those mutations are lost"
            ));
            health.compromise(format!(
                "{bad} op-log line(s) carry no usable op, so the replayed tree is missing them"
            ));
        }
    }
    scans
}

/// Frame one physical record the way the storage layer does.
fn classify_record(raw: &[u8]) -> LineVerdict {
    let Ok(text) = std::str::from_utf8(raw) else {
        return LineVerdict::Bad("non-UTF8 bytes (partial sync or bit-rot)".to_string());
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return LineVerdict::Blank;
    }
    let mut count = 0usize;
    for item in serde_json::Deserializer::from_str(trimmed).into_iter::<LogOp>() {
        match item {
            Ok(_) => count += 1,
            Err(e) => {
                return LineVerdict::Bad(if count == 0 {
                    format!("invalid JSON: {e}")
                } else {
                    format!("{count} op(s) recovered then invalid JSON: {e}")
                })
            }
        }
    }
    match count {
        0 => LineVerdict::Bad("no JSON value on a non-empty line".to_string()),
        1 => LineVerdict::Clean,
        n => LineVerdict::Glued(n),
    }
}

/// Verify `.outl/snapshots/snap-*.bin`.
///
/// A snapshot is a pure boot cache: a bad one is never fatal, it just
/// means the next boot pays a full op-log replay. Reported as a warning
/// and offered to `--repair` for deletion.
///
/// **"I could not read it" is not "I read it and it is garbage."** A
/// permission error, a busy file, a half-arrived sync, an `EIO` off a
/// flaky disk — none of those say anything about the bytes, and
/// `--repair` deletes for real. Only a snapshot we read end-to-end and
/// then failed to decode has earned that. An unreadable one is reported
/// and left exactly where it is; the next boot ignores it anyway, so
/// leaving it costs nothing but a replay.
///
/// Returns the paths of the snapshots that decoded incorrectly — never
/// the ones that merely could not be read.
pub(super) fn check_snapshots(b: &mut Builder, root: &Path) -> Vec<PathBuf> {
    let dir = root.join(".outl").join("snapshots");
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            b.info("no snapshot yet — boot does a full op-log replay (correct, just slower)");
            return Vec::new();
        }
        Err(e) => {
            b.warn(format!("could not read {}: {e}", dir.display()));
            return Vec::new();
        }
    };

    let mut good = 0usize;
    let mut corrupt = Vec::new();
    let mut unreadable = 0usize;
    let mut stale_tmp = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.ends_with(".bin.tmp") {
            stale_tmp += 1;
            continue;
        }
        if !name.starts_with("snap-") || !name.ends_with(".bin") {
            continue;
        }
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                // Deliberately NOT pushed onto `corrupt`: we never saw
                // the bytes, so "delete it" would be a guess with a
                // `remove_file` behind it.
                unreadable += 1;
                b.warn(format!(
                    "{}: snapshot could not be read ({e}) — left alone, because an unreadable \
                     file is not a proven-bad one. Boot ignores it and does a full op-log \
                     replay; fix the permissions or the disk and re-run",
                    path.display()
                ));
                continue;
            }
        };
        match SnapshotBody::decode(&bytes) {
            Ok(body) => {
                good += 1;
                b.ok(format!(
                    "{}: snapshot decodes, hash verified ({} nodes, {} bytes)",
                    path.display(),
                    body.nodes.len(),
                    bytes.len()
                ));
            }
            Err(e) => {
                b.warn(format!(
                    "{}: snapshot unusable ({e}) — boot falls back to a full op-log replay. \
                     No data is at risk; `--repair` deletes it so it is rebuilt on next boot",
                    path.display()
                ));
                corrupt.push(path);
            }
        }
    }

    if stale_tmp > 0 {
        b.info(format!(
            "{stale_tmp} leftover `snap-*.bin.tmp` in {} — a snapshot write was interrupted; harmless",
            dir.display()
        ));
    }
    if good == 0 && corrupt.is_empty() && unreadable == 0 {
        b.info("no snapshot yet — boot does a full op-log replay (correct, just slower)");
    }
    corrupt
}

/// Cross-check every `.ops-<actor>.idx` against the `.jsonl` it indexes.
///
/// The index is a local cache: a wrong offset is recoverable (delete it
/// and the next boot rebuilds), but a *silently* wrong one makes the
/// index-driven cold reads return the wrong op, so it must be named.
pub(super) fn check_offset_indexes(b: &mut Builder, ops_dir: &Path, scans: &[(PathBuf, FileScan)]) {
    let mut checked = 0usize;
    let mut problems = 0usize;

    for (jsonl, scan) in scans {
        let Some(name) = jsonl.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // Only the Global-scope `ops-<actor>.jsonl` layout has a
        // matching `.idx`; a per-page shard doesn't.
        let Some(actor) = name
            .strip_prefix("ops-")
            .and_then(|s| s.strip_suffix(".jsonl"))
            .and_then(|s| ulid::Ulid::from_string(s).ok())
            .map(outl_core::id::ActorId)
        else {
            continue;
        };
        let idx_path = ActorIndex::sidecar_path(ops_dir, actor);
        let index = match OffsetIndex::load(&idx_path) {
            // `load` folds a malformed file into `Ok(None)` on purpose
            // (the caller rebuilds). Absent and malformed are the same
            // signal from here: nothing to cross-check.
            Ok(None) => continue,
            Ok(Some(index)) => index,
            Err(e) => {
                b.warn(format!(
                    "{}: offset index unreadable: {e}",
                    idx_path.display()
                ));
                problems += 1;
                continue;
            }
        };
        checked += 1;

        let timestamps: Vec<_> = index.timestamps().copied().collect();
        let mut off_record = 0usize;
        let mut past_eof = 0usize;
        let mut first_bad: Option<u64> = None;
        for ts in &timestamps {
            let Some(offset) = index.get(ts) else {
                continue;
            };
            if offset >= scan.size {
                past_eof += 1;
                first_bad.get_or_insert(offset);
            } else if !scan.record_offsets.contains(&offset) {
                off_record += 1;
                first_bad.get_or_insert(offset);
            }
        }

        if past_eof > 0 || off_record > 0 {
            problems += 1;
            b.warn(format!(
                "{}: {} offset(s) past EOF and {} not on a record boundary (first bad byte {}) \
                 — stale index; delete it and the next boot rebuilds from the `.jsonl`",
                idx_path.display(),
                past_eof,
                off_record,
                first_bad.unwrap_or(0),
            ));
        } else if index.len() > scan.ops {
            problems += 1;
            b.warn(format!(
                "{}: indexes {} ops but the `.jsonl` only yields {} — stale index",
                idx_path.display(),
                index.len(),
                scan.ops,
            ));
        }
    }

    if checked > 0 && problems == 0 {
        b.ok(format!(
            "{checked} offset index file(s) agree with their `.jsonl`"
        ));
    }
}
