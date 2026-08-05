//! On-disk format of the device store's files.
//!
//! Every file here is a handful of `key=value` lines. A bare line with no
//! `=` is the legacy single-value form (a plain ULID) and binds to *both*
//! primary-key names — `actor` for `<dir>/actor` and `actors/*`, `id` for
//! `machine-id`. Those two never coexist in one file, so the alias cannot
//! shadow anything, and the Tauri clients' pre-existing `actor` file keeps
//! resolving to the same actor.
//!
//! Writes go through a temp file + rename. A torn record makes the next
//! open mint a fresh actor for a workspace that already had one — cheap,
//! but noisy, and there is no reason to risk it.

use std::path::{Path, PathBuf};

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
    let tmp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("record"),
        std::process::id()
    ));
    std::fs::write(&tmp, render(pairs)).map_err(|source| DeviceError::Io {
        path: tmp.clone(),
        source,
    })?;
    std::fs::rename(&tmp, path).map_err(|source| DeviceError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Create `path` only if it does not exist yet (`O_EXCL`), so a caller
/// can tell "I bound this" from "someone else got here first".
pub(super) fn create_new_record<K: AsRef<str>, V: AsRef<str>>(
    path: &Path,
    pairs: &[(K, V)],
) -> std::io::Result<()> {
    use std::io::Write;
    ensure_parent(path).map_err(|DeviceError::Io { source, .. }| source)?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(render(pairs).as_bytes())
}

fn ensure_parent(path: &Path) -> Result<PathBuf, DeviceError> {
    let parent = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    std::fs::create_dir_all(&parent).map_err(|source| DeviceError::Io {
        path: parent.clone(),
        source,
    })?;
    Ok(parent)
}
