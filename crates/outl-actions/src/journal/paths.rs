//! Workspace directory layout + atomic `.md` write + projection removal.

use std::path::{Path, PathBuf};

use crate::page::{PageKind, PageMeta};

/// Path for the `journals/` directory inside the workspace.
///
/// `root` is the **workspace root** — the directory whose immediate
/// children are `journals/`, `pages/`, `ops/`. Callers are responsible
/// for picking the right root: on iOS the mobile app derives this from
/// the iCloud Ubiquity Container (`<container>/Documents/outl`); on
/// desktop the TUI receives it as `--path`. We do **not** re-join
/// `Documents/outl` here — doing so silently nested the layout twice
/// when the TUI passed the already-final workspace path.
pub fn journals_dir(root: &Path) -> PathBuf {
    root.join("journals")
}

/// Path for the `pages/` directory inside the workspace.
///
/// See [`journals_dir`] for the contract on `root`.
pub fn pages_dir(root: &Path) -> PathBuf {
    root.join("pages")
}

/// Build the on-disk path for a given page's `.md` projection.
pub fn page_md_path(root: &Path, meta: &PageMeta) -> PathBuf {
    let folder = match meta.kind {
        PageKind::Journal => journals_dir(root),
        PageKind::Page => pages_dir(root),
    };
    folder.join(format!("{}.md", meta.slug))
}

/// Remove a page's `.md` projection and its `.outl` sidecar from disk.
///
/// The inverse of [`super::apply_page_md_with_sidecar`]: after
/// [`crate::page::delete`] moves the page root to the trash, the
/// on-disk projection would otherwise linger. A peer that hasn't
/// received the delete op yet would keep reading the stale `.md`;
/// removing the projection here on the acting device means the next
/// `outl doctor` / orphan scan agrees with the op log, and the page
/// disappears from listings that walk `pages/` directly.
///
/// Idempotent: a missing file is silently OK (the page may never have
/// been projected on this device — common right after a peer-shipped
/// delete). Any other I/O error is returned so the caller can decide
/// whether to swallow (CLI's `remove_or_warn`) or propagate (Tauri
/// command's error envelope).
pub fn remove_page_projection(root: &Path, meta: &PageMeta) -> std::io::Result<()> {
    let md_path = page_md_path(root, meta);
    match std::fs::remove_file(&md_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let sidecar_path = outl_md::resolve_sidecar_path(&md_path);
    match std::fs::remove_file(&sidecar_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    Ok(())
}

/// Atomic write of `contents` to `path`, creating parents as needed.
///
/// Delegates to [`outl_md::write_atomic`] rather than repeating the
/// tmp-then-rename dance. The hand-rolled copy this replaced wrote the
/// temp file and renamed it with **no fsync at all**, on the hottest
/// `.md` write path in the crate (`apply_page_md`,
/// `apply_page_md_with_sidecar`, undo restore), while the primitives
/// catalog told readers this function wrapped the crash-safe one.
/// A rename is only durable once the file's bytes and the parent
/// directory entry are both synced; without that, a crash can leave the
/// rename visible and the contents empty.
pub fn write_md_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    outl_md::write_atomic(path, contents.as_bytes())
}
