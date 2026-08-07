//! On-disk format of the device store's files.
//!
//! Every file here is a handful of `key=value` lines. A bare line with no
//! `=` is the legacy single-value form (a plain ULID) and binds to *both*
//! primary-key names — `actor` for `<dir>/actor` and `actors/*`, `id` for
//! `machine-id`. Those two never coexist in one file, so the alias cannot
//! shadow anything, and the Tauri clients' pre-existing `actor` file keeps
//! resolving to the same actor.
//!
//! Every write composes the whole record in a sibling temp file and then
//! publishes it in one step — `rename` to replace, `link` to create-if-absent.
//! A torn record makes the next open mint a fresh actor for a workspace
//! that already had one — cheap, but noisy, and there is no reason to risk
//! it. A *blank* one is worse than torn: readers here map an empty file to
//! "absent", which is the answer that licenses a caller to overwrite it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::DeviceError;

/// Parsed key/value pairs of one device-store file.
#[derive(Debug, Default)]
pub(super) struct Record(Vec<(String, String)>);

impl Record {
    /// First value recorded under `key`, if any.
    pub(super) fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

fn parse(raw: &str) -> Record {
    let mut out = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match line.split_once('=') {
            Some((key, value)) => out.push((key.trim().to_string(), value.trim().to_string())),
            None => {
                out.push(("actor".to_string(), line.to_string()));
                out.push(("id".to_string(), line.to_string()));
            }
        }
    }
    Record(out)
}

/// Read and parse `path`, treating a missing or blank file as absent.
pub(super) fn read_record(path: &Path) -> Result<Option<Record>, DeviceError> {
    match std::fs::read_to_string(path) {
        Ok(contents) if contents.trim().is_empty() => Ok(None),
        Ok(contents) => Ok(Some(parse(&contents))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(DeviceError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn render<K: AsRef<str>, V: AsRef<str>>(pairs: &[(K, V)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}\n", k.as_ref(), v.as_ref()))
        .collect()
}

/// Replace `path` atomically (temp file + rename).
pub(super) fn write_record<K: AsRef<str>, V: AsRef<str>>(
    path: &Path,
    pairs: &[(K, V)],
) -> Result<(), DeviceError> {
    let parent = ensure_parent(path)?;
    let tmp = scratch_path(&parent, path, "tmp");
    std::fs::write(&tmp, render(pairs)).map_err(|source| DeviceError::Io {
        path: tmp.clone(),
        source,
    })?;
    std::fs::rename(&tmp, path).map_err(|source| DeviceError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Create `path` only if it does not exist yet, so a caller can tell "I
/// bound this" from "someone else got here first" — and publish it with
/// its content already in place.
///
/// The content half is not a nicety. A bare `O_EXCL` open creates an
/// **empty** file and fills it a moment later, and every reader here maps
/// a blank record to `None` — "absent", which is precisely the answer that
/// licenses a caller to write its own value. A concurrent first open that
/// lands in that window therefore overwrites the winner instead of
/// adopting it, which is the compare-and-swap failing open. Writing a
/// temp file and `link(2)`-ing it into place makes the file appear
/// complete or not at all.
///
/// `hard_link` is the exclusive-create here; it reports
/// [`std::io::ErrorKind::AlreadyExists`] exactly like `O_EXCL` did. On a
/// filesystem that cannot link (FAT-family removable media), it fails for
/// some other reason and we fall back to the plain `O_EXCL` open — the
/// narrow race is still better than a device store that cannot be written
/// at all.
pub(super) fn create_new_record<K: AsRef<str>, V: AsRef<str>>(
    path: &Path,
    pairs: &[(K, V)],
) -> std::io::Result<()> {
    let parent = ensure_parent(path).map_err(|DeviceError::Io { source, .. }| source)?;
    let tmp = scratch_path(&parent, path, "new");
    std::fs::write(&tmp, render(pairs))?;
    let linked = std::fs::hard_link(&tmp, path);
    let _ = std::fs::remove_file(&tmp);
    match linked {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(e),
        // Never expose an empty destination: the atomic publication method
        // is unavailable on this filesystem, so fail rather than reintroduce
        // the O_EXCL-then-write visibility race.
        Err(e) => Err(e),
    }
}

fn exclusive_create<K: AsRef<str>, V: AsRef<str>>(
    path: &Path,
    pairs: &[(K, V)],
) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(render(pairs).as_bytes())
}

/// A sibling scratch name for `path`, unique to this **call**.
///
/// The pid alone is not enough, and that is not a theoretical concern: two
/// threads of one process resolving their actors concurrently would derive
/// the same scratch name, so one's `fs::write` truncates and interleaves
/// with the other's, and whichever wins the publish (`rename` or
/// `hard_link`) publishes a record neither writer composed. A device store
/// that answers with a torn machine id hands every workspace on the
/// machine a fresh actor.
fn scratch_path(parent: &Path, path: &Path, suffix: &str) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    parent.join(format!(
        ".{}.{}.{}.{suffix}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("record"),
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed),
    ))
}

fn ensure_parent(path: &Path) -> Result<PathBuf, DeviceError> {
    let parent = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    std::fs::create_dir_all(&parent).map_err(|source| DeviceError::Io {
        path: parent.clone(),
        source,
    })?;
    Ok(parent)
}
