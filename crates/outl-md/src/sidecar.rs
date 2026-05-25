//! `.outl` sidecar file — JSON dotfile next to each `.md`.
//!
//! Holds the IDs and content hashes the clean `.md` cannot. See
//! `docs/markdown-format.md` §sidecar for the format spec.

use chrono::{DateTime, FixedOffset, Local};
use outl_core::id::NodeId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Current sidecar format version. Bump when introducing breaking changes.
pub const SIDECAR_VERSION: u32 = 1;

/// One block entry in the sidecar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidecarBlock {
    /// Block id.
    pub id: NodeId,
    /// 1-indexed line number in the `.md` at last sync.
    pub line: usize,
    /// Indent level (0 for top-level outline items).
    pub indent: u32,
    /// SHA-256 of the block's textual content, formatted `sha256:<hex>`.
    pub content_hash: String,
}

/// Full sidecar payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sidecar {
    /// Format version. Always present.
    pub version: u32,
    /// Page id (also the root block id).
    pub page_id: NodeId,
    /// SHA-256 of the full `.md` at last sync (`sha256:<hex>`).
    pub last_synced_hash: String,
    /// When the sidecar was last written. ISO 8601 with timezone.
    pub last_synced_at: DateTime<FixedOffset>,
    /// Block entries in tree (depth-first preorder) order.
    pub blocks: Vec<SidecarBlock>,
}

impl Sidecar {
    /// Build an empty sidecar for a new page.
    pub fn new_for_page(page_id: NodeId, md_hash: &str) -> Self {
        Self {
            version: SIDECAR_VERSION,
            page_id,
            last_synced_hash: md_hash.to_string(),
            last_synced_at: now_local(),
            blocks: Vec::new(),
        }
    }
}

/// Errors loading or storing a sidecar.
#[derive(Debug, thiserror::Error)]
pub enum SidecarError {
    /// JSON parse failure.
    #[error("invalid sidecar JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    /// I/O failure reading/writing the sidecar file.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// Unsupported sidecar version.
    #[error("unsupported sidecar version: {0}")]
    UnsupportedVersion(u32),
}

/// Compute the sidecar path for a given `.md` path.
///
/// `pages/foo.md` → `pages/.foo.outl`. The `.md` is dropped on purpose —
/// the sidecar always pairs with a markdown file, so encoding the
/// extension twice (`.foo.md.outl`) is noise. The leading dot keeps it
/// hidden in `ls`.
pub fn sidecar_path_for(md_path: &Path) -> PathBuf {
    let parent = md_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = md_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "untitled".to_string());
    parent.join(format!(".{stem}.outl"))
}

/// Read and validate a sidecar from disk.
pub fn read(path: &Path) -> Result<Sidecar, SidecarError> {
    let s = std::fs::read_to_string(path)?;
    let sc: Sidecar = serde_json::from_str(&s)?;
    if sc.version != SIDECAR_VERSION {
        return Err(SidecarError::UnsupportedVersion(sc.version));
    }
    Ok(sc)
}

/// Write a sidecar to disk as pretty-printed JSON.
///
/// Uses [`crate::atomic::write_atomic`] so a crash mid-write can never
/// leave a half-written sidecar that would fail to parse on next open.
pub fn write(path: &Path, sidecar: &Sidecar) -> Result<(), SidecarError> {
    let s = serde_json::to_string_pretty(sidecar)?;
    crate::atomic::write_atomic(path, s.as_bytes())?;
    Ok(())
}

/// Compute the canonical content hash of a block's text.
///
/// The text is whitespace-normalized (internal whitespace collapsed to a
/// single space, leading/trailing trimmed) before hashing. The result is
/// `sha256:<lowercase-hex>`. Same function used on read and write.
pub fn content_hash(text: &str) -> String {
    let normalized = normalize(text);
    let mut h = Sha256::new();
    h.update(normalized.as_bytes());
    format!("sha256:{}", hex::encode(h.finalize()))
}

/// Compute the hash of the full `.md` file content.
pub fn file_hash(md: &str) -> String {
    let mut h = Sha256::new();
    h.update(md.as_bytes());
    format!("sha256:{}", hex::encode(h.finalize()))
}

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn now_local() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn sidecar_path_strategy_is_dotfile() {
        let p = sidecar_path_for(Path::new("/notes/pages/foo.md"));
        assert_eq!(p, PathBuf::from("/notes/pages/.foo.outl"));
    }

    #[test]
    fn sidecar_path_drops_md_extension() {
        // Regression: we used to emit `.foo.md.outl`. The `.md` is
        // redundant (sidecars always pair with `.md`) and confusing.
        let p = sidecar_path_for(Path::new("/notes/journals/2026-05-22.md"));
        assert_eq!(
            p,
            PathBuf::from("/notes/journals/.2026-05-22.outl"),
            "sidecar must drop the .md extension"
        );
    }

    #[test]
    fn roundtrip_through_disk() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".foo.outl");
        let sc = Sidecar::new_for_page(NodeId::new(), &file_hash("- hello\n"));
        write(&path, &sc).unwrap();
        let loaded = read(&path).unwrap();
        assert_eq!(loaded.version, SIDECAR_VERSION);
        assert_eq!(loaded.page_id, sc.page_id);
        assert_eq!(loaded.last_synced_hash, sc.last_synced_hash);
    }

    #[test]
    fn unsupported_version_fails_loudly() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".bad.outl");
        std::fs::write(
            &path,
            r#"{"version":99,"page_id":"01HXY","last_synced_hash":"x","last_synced_at":"2026-05-24T10:00:00-03:00","blocks":[]}"#,
        )
        .unwrap();
        match read(&path) {
            Err(SidecarError::InvalidJson(_)) | Err(SidecarError::UnsupportedVersion(99)) => {}
            other => panic!("expected version/json error, got {other:?}"),
        }
    }

    #[test]
    fn content_hash_normalizes_whitespace() {
        let a = content_hash("hello world");
        let b = content_hash("  hello   world  ");
        let c = content_hash("hello\tworld\n");
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    #[test]
    fn content_hash_differs_on_real_content_changes() {
        assert_ne!(content_hash("hello world"), content_hash("hello worlds"));
    }
}
