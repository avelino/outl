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
///
/// Version history:
/// - **1** — initial format (page_id, last_synced_hash, blocks with id /
///   line / indent / content_hash).
/// - **2** — added `ref_handle` on every block to power `((blk-XXXXXX))`
///   inline references. Backward-compatible read: v1 sidecars load fine
///   and their handles are derived on the fly from the block id.
pub const SIDECAR_VERSION: u32 = 2;

/// Lowest sidecar version this crate is willing to read.
///
/// Older versions return [`SidecarError::UnsupportedVersion`]. Keeping
/// this explicit (rather than a magic number in `read`) so the contract
/// is greppable when a v3 ever needs to drop v1 support.
pub const MIN_READABLE_SIDECAR_VERSION: u32 = 1;

/// Prefix every block ref handle carries in the `.md` file.
///
/// `((blk-r6s4a1))` is what users see. The prefix lets a reader (human
/// or parser) tell a block ref apart from page refs / tags at a glance.
pub const REF_HANDLE_PREFIX: &str = "blk-";

/// Number of base32 (Crockford, lowercased) characters taken from the
/// **tail** of the block's ULID to form its ref handle.
///
/// ULIDs are 26 chars total, split as 10 chars of timestamp + 16 chars
/// of random tail. Pulling 6 chars from the tail gives ~30 bits of
/// entropy (~1B values). Birthday-collision probability at 100k blocks
/// is ~5e-6 — effectively zero. Lazy expansion to 7+ chars happens at
/// index-build time if a collision is ever observed (see
/// `WorkspaceIndex`); the sidecar itself always stores whatever handle
/// resolved a given block at write time.
pub const REF_HANDLE_TAIL_LEN: usize = 6;

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
    /// Short, stable, human-typeable handle for `((blk-XXXXXX))` inline
    /// references and `!((blk-XXXXXX))` embeds.
    ///
    /// Default-derived from [`derive_ref_handle`] using the block id.
    /// The handle is stable as long as the block keeps the same id —
    /// editing the block's text does **not** change it. Persisted so
    /// that a future change to the derivation scheme cannot invalidate
    /// existing references already living in `.md` files.
    ///
    /// `#[serde(default)]` is what makes v1 sidecars load cleanly:
    /// missing handles are backfilled by [`read`] from the id.
    #[serde(default)]
    pub ref_handle: String,
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
/// `pages/foo.md` → `pages/foo.outl`. The `.md` is dropped on purpose —
/// the sidecar always pairs with a markdown file, so encoding the
/// extension twice (`.foo.md.outl`) is noise.
///
/// **The sidecar is not hidden.** Earlier releases stored it as
/// `.foo.outl` to keep it out of casual `ls` output, but that confused
/// iCloud Drive (it would still sync, but Files.app on iOS hides
/// dotted entries entirely, leaving users unable to confirm a
/// peer-side write had landed). Sitting next to its `.md` makes the
/// relationship visible to the user and any other tool walking the
/// directory.
pub fn sidecar_path_for(md_path: &Path) -> PathBuf {
    let parent = md_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = md_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "untitled".to_string());
    parent.join(format!("{stem}.outl"))
}

/// Legacy sidecar path (dotted) used by builds before v0. Kept so the
/// reader can transparently pick up old sidecars and rename them to
/// the modern un-hidden form on first read.
fn legacy_sidecar_path_for(md_path: &Path) -> PathBuf {
    let parent = md_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = md_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "untitled".to_string());
    parent.join(format!(".{stem}.outl"))
}

/// Find the path the caller should use right now to read or write the
/// sidecar for `md_path`.
///
/// In the common case this is the canonical (non-dotted) `<stem>.outl`
/// next to the `.md`. Two transitional cases also return the legacy
/// dotted form so the caller still sees a sidecar where there is one:
///
/// - The modern path doesn't exist yet but a legacy `.<stem>.outl`
///   does and the migration rename to the modern name succeeds — we
///   return the modern path.
/// - Same setup, but the rename fails (read-only filesystem, race with
///   another writer) — we return the legacy dotted path so the caller
///   can still read it. The next successful call moves it.
///
/// Returning the legacy path on rename failure is intentional: callers
/// `read()` and `write()` against whatever we return. If we always
/// returned the modern path while the file was still at the legacy one,
/// `read()` would fail with `NotFound` and the sidecar would appear to
/// be missing.
pub fn resolve_sidecar_path(md_path: &Path) -> PathBuf {
    let modern = sidecar_path_for(md_path);
    if modern.exists() {
        return modern;
    }
    let legacy = legacy_sidecar_path_for(md_path);
    if legacy.exists() {
        if std::fs::rename(&legacy, &modern).is_ok() {
            return modern;
        }
        return legacy;
    }
    modern
}

/// Read and validate a sidecar from disk.
///
/// Accepts any version in `[MIN_READABLE_SIDECAR_VERSION, SIDECAR_VERSION]`.
/// Older payloads are upgraded in-memory: every block missing a
/// `ref_handle` gets one [derived from its id](derive_ref_handle), and
/// the in-memory `version` is bumped to [`SIDECAR_VERSION`]. The next
/// [`write()`] then persists the upgraded shape.
pub fn read(path: &Path) -> Result<Sidecar, SidecarError> {
    let s = std::fs::read_to_string(path)?;
    let mut sc: Sidecar = serde_json::from_str(&s)?;
    if sc.version < MIN_READABLE_SIDECAR_VERSION || sc.version > SIDECAR_VERSION {
        return Err(SidecarError::UnsupportedVersion(sc.version));
    }
    for b in &mut sc.blocks {
        if b.ref_handle.is_empty() {
            b.ref_handle = derive_ref_handle(b.id);
        }
    }
    sc.version = SIDECAR_VERSION;
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

/// Derive the canonical ref handle for a given block id.
///
/// Format: `blk-` followed by the last [`REF_HANDLE_TAIL_LEN`] characters
/// of the ULID's Crockford base32 representation, lowercased. ULID
/// `Display` is always exactly 26 ASCII characters today; iterating
/// by `chars()` keeps the function safe if a future id encoding ever
/// becomes multi-byte UTF-8.
///
/// Determinism matters: the same block id must always yield the same
/// handle so that two devices building the sidecar independently agree
/// on what `((blk-XXXXXX))` means.
pub fn derive_ref_handle(id: NodeId) -> String {
    let s = id.to_string();
    let total = s.chars().count();
    let skip = total.saturating_sub(REF_HANDLE_TAIL_LEN);
    let tail: String = s.chars().skip(skip).collect();
    format!("{REF_HANDLE_PREFIX}{}", tail.to_lowercase())
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
    fn sidecar_path_is_visible_next_to_md() {
        let p = sidecar_path_for(Path::new("/notes/pages/foo.md"));
        assert_eq!(p, PathBuf::from("/notes/pages/foo.outl"));
    }

    #[test]
    fn sidecar_path_drops_md_extension() {
        // Regression: we used to emit `foo.md.outl`. The `.md` is
        // redundant (sidecars always pair with `.md`) and confusing.
        let p = sidecar_path_for(Path::new("/notes/journals/2026-05-22.md"));
        assert_eq!(
            p,
            PathBuf::from("/notes/journals/2026-05-22.outl"),
            "sidecar must drop the .md extension"
        );
    }

    #[test]
    fn resolve_sidecar_migrates_dotted_legacy() {
        let tmp = TempDir::new().unwrap();
        let md = tmp.path().join("foo.md");
        std::fs::write(&md, "- block\n").unwrap();
        let legacy = tmp.path().join(".foo.outl");
        std::fs::write(&legacy, "{\"version\":2}").unwrap();

        let resolved = resolve_sidecar_path(&md);
        assert_eq!(resolved, tmp.path().join("foo.outl"));
        assert!(
            resolved.exists(),
            "modern sidecar must exist after migration"
        );
        assert!(!legacy.exists(), "legacy dotted sidecar must be gone");
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
    fn derive_ref_handle_uses_last_six_chars_lowercased() {
        // The derivation is "take the lowercased tail of the ULID's
        // Display impl". We assert that property holds for an arbitrary
        // id without depending on the `ulid` crate here (outl-md does
        // not have it as a direct dependency).
        let id = NodeId::new();
        let display = id.to_string();
        let expected_tail = display[display.len() - REF_HANDLE_TAIL_LEN..].to_lowercase();
        assert_eq!(
            derive_ref_handle(id),
            format!("{REF_HANDLE_PREFIX}{expected_tail}")
        );
    }

    #[test]
    fn derive_ref_handle_is_deterministic() {
        let id = NodeId::new();
        assert_eq!(derive_ref_handle(id), derive_ref_handle(id));
    }

    #[test]
    fn derive_ref_handle_format_is_blk_prefix_plus_six() {
        let id = NodeId::new();
        let h = derive_ref_handle(id);
        assert!(h.starts_with(REF_HANDLE_PREFIX));
        let tail = &h[REF_HANDLE_PREFIX.len()..];
        assert_eq!(tail.len(), REF_HANDLE_TAIL_LEN);
        assert!(tail.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_eq!(tail, tail.to_lowercase());
    }

    #[test]
    fn v1_sidecar_loads_and_backfills_ref_handle() {
        // Hand-written v1 payload (no `ref_handle` field on the block).
        // We deserialize through `read` and assert it:
        //   1. parses without error,
        //   2. surfaces version == SIDECAR_VERSION on the in-memory
        //      value (upgrade-on-read),
        //   3. populates a non-empty `ref_handle` derived from `id`.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".legacy.outl");
        let id = NodeId::new();
        let v1_json = format!(
            r#"{{
              "version": 1,
              "page_id": "{page}",
              "last_synced_hash": "sha256:abc",
              "last_synced_at": "2026-05-24T10:00:00-03:00",
              "blocks": [
                {{
                  "id": "{block}",
                  "line": 1,
                  "indent": 0,
                  "content_hash": "sha256:def"
                }}
              ]
            }}"#,
            page = NodeId::new(),
            block = id,
        );
        std::fs::write(&path, v1_json).unwrap();
        let sc = read(&path).unwrap();
        assert_eq!(sc.version, SIDECAR_VERSION);
        assert_eq!(sc.blocks.len(), 1);
        assert_eq!(sc.blocks[0].ref_handle, derive_ref_handle(id));
    }

    #[test]
    fn write_then_read_v2_preserves_ref_handle() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".foo.outl");
        let page_id = NodeId::new();
        let block_id = NodeId::new();
        let mut sc = Sidecar::new_for_page(page_id, &file_hash("- hello\n"));
        sc.blocks.push(SidecarBlock {
            id: block_id,
            line: 1,
            indent: 0,
            content_hash: content_hash("hello"),
            ref_handle: derive_ref_handle(block_id),
        });
        write(&path, &sc).unwrap();

        let loaded = read(&path).unwrap();
        assert_eq!(loaded.version, SIDECAR_VERSION);
        assert_eq!(loaded.blocks.len(), 1);
        assert_eq!(loaded.blocks[0].ref_handle, derive_ref_handle(block_id));

        // And the on-disk JSON actually contains the field — guards
        // against a future serde attribute accidentally skipping it.
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(
            on_disk.contains("ref_handle"),
            "v2 sidecar must persist ref_handle on disk; got: {on_disk}"
        );
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
