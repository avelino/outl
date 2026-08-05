//! Atomic file writes — `write-temp + rename` pattern.
//!
//! `fs::write` is two operations under the hood: truncate + write. A
//! crash between them leaves a partial file. For outl this could mean
//! a `.md` with half a page or a sidecar with a broken JSON. Either
//! way you'd need [`crate::reconcile`] to clean up.
//!
//! The fix is universal in POSIX: write to a sibling `*.tmp` then
//! `rename(tmp, final)`. `rename` is atomic — readers see either the
//! old file or the new file, never a half-written one.
//!
//! On Windows the same `std::fs::rename` works on the same volume
//! (which `.outl/` always is).
//!
//! The read counterpart is [`read_for_rewrite`] — see its docs for why
//! `read_to_string(..).unwrap_or_default()` is never acceptable on a
//! path you are about to write back.

use std::fs;
use std::io;
use std::path::Path;

/// Write `contents` to `path` atomically.
///
/// Steps:
/// 1. Compute a sibling temp path: `path` with `.tmp` appended to the
///    filename (so `pages/foo.md` becomes `pages/foo.md.tmp`).
/// 2. Write the temp file fully and sync it to disk.
/// 3. `rename(tmp, path)` — atomic on a single filesystem.
///
/// If anything fails, the temp file is cleaned up so we don't leave
/// `*.tmp` litter behind.
pub fn write_atomic<P: AsRef<Path>>(path: P, contents: &[u8]) -> io::Result<()> {
    let path = path.as_ref();
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "atomic write requires a path with a parent directory",
        )
    })?;
    // Make sure the parent exists — `outl init` creates these but a
    // user moving files around could delete `pages/` and we'd hit a
    // raw IO error otherwise.
    if !parent.as_os_str().is_empty() && !parent.exists() {
        fs::create_dir_all(parent)?;
    }

    let tmp = tmp_path(path);

    // Write + fsync the temp.
    {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)?;
        use std::io::Write;
        file.write_all(contents)?;
        // Flush kernel buffers to disk so a power loss between write
        // and rename can't replay garbage.
        file.sync_all()?;
    }

    // Atomic swap. On error, clean up the temp.
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }

    // `rename` is atomic with respect to readers, but the *directory
    // entry* it creates is itself only durable once the directory is
    // fsynced. Without this, a power loss right after the rename can
    // leave the old file (or, on some filesystems, an empty one) even
    // though `sync_all` on the temp succeeded. APFS and ext4's default
    // `data=ordered` usually paper over it; other filesystems don't.
    //
    // Best-effort on purpose: some platforms (notably Windows) refuse
    // to open a directory as a file, and failing the whole write there
    // would be worse than the durability gap we're closing.
    if let Ok(dir) = fs::File::open(parent) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Read a file that is about to be **parsed, mutated, and written back**.
///
/// A missing file is the one legitimate "empty" case — a page that does
/// not exist yet renders from an empty AST. Every other error
/// (`PermissionDenied`, `Interrupted`, a raw `EIO`, an iCloud placeholder
/// whose contents have not been materialised locally yet) is propagated.
///
/// This exists because `fs::read_to_string(p).unwrap_or_default()` on a
/// rewrite path is a silent-data-loss bug: the read fails, the caller
/// parses `""` into an empty AST, renders it, and [`write_atomic`]
/// faithfully replaces a full page with nothing. The sidecar is then
/// rebuilt to match, so the hashes agree and no later scan can tell that
/// the page was ever populated. Under iCloud — where a not-yet-downloaded
/// file is exactly this kind of read failure — it is not a rare edge case.
///
/// Callers that genuinely want "absent or unreadable both mean empty"
/// must say so explicitly at the call site, and must not be on a path
/// that writes the result back.
pub fn read_for_rewrite<P: AsRef<Path>>(path: P) -> io::Result<String> {
    match fs::read_to_string(path.as_ref()) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e),
    }
}

/// `path` plus a `.tmp` suffix on the filename.
fn tmp_path(path: &Path) -> std::path::PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn writes_and_renames() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("foo.md");
        write_atomic(&path, b"hello").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn no_temp_file_left_behind() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("foo.md");
        write_atomic(&path, b"hello").unwrap();
        let tmp = dir.path().join("foo.md.tmp");
        assert!(!tmp.exists(), "temp file should be gone after rename");
    }

    #[test]
    fn overwrites_existing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("foo.md");
        fs::write(&path, "old").unwrap();
        write_atomic(&path, b"new").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
    }

    #[test]
    fn creates_missing_parent_dir() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("deep").join("nested").join("x.md");
        write_atomic(&path, b"hi").unwrap();
        assert!(path.exists());
    }

    #[test]
    fn read_for_rewrite_returns_contents() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("foo.md");
        fs::write(&path, "- a\n").unwrap();
        assert_eq!(read_for_rewrite(&path).unwrap(), "- a\n");
    }

    #[test]
    fn read_for_rewrite_missing_file_is_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nope.md");
        assert_eq!(read_for_rewrite(&path).unwrap(), "");
    }

    /// The whole point: a read that fails for any reason *other* than
    /// "file isn't there" must not be reported as an empty page, because
    /// the caller is about to render that emptiness back over the file.
    #[test]
    fn read_for_rewrite_propagates_non_notfound_errors() {
        let dir = TempDir::new().unwrap();
        // A directory is readable metadata-wise but `read_to_string`
        // fails on it — a stand-in for any non-NotFound I/O failure
        // that doesn't need root or a fault-injection layer to trigger.
        let err = read_for_rewrite(dir.path()).expect_err("reading a directory must fail");
        assert_ne!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn read_for_rewrite_rejects_invalid_utf8() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bad.md");
        fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        let err = read_for_rewrite(&path).expect_err("invalid UTF-8 must not read as empty");
        assert_ne!(err.kind(), io::ErrorKind::NotFound);
    }
}
