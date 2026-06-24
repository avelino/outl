//! iCloud Ubiquity Container helpers for this app.
//!
//! **iCloud is opt-in, not the default.** As of the Fase 2 sync overhaul
//! the workspace root is a folder the user picks (it may live anywhere —
//! local app data, the Files app, or inside an iCloud container) and iroh
//! P2P is the primary sync. This module no longer decides *where* the
//! workspace lives; it only answers two iCloud-specific questions for the
//! callers that still care:
//!
//! - [`resolve_container`] — where *is* the app's iCloud container, for
//!   when the user explicitly wants to store the workspace inside it.
//! - [`is_inside_icloud`] — is a given chosen path inside an iCloud
//!   container? This gates the iCloud-only change detector
//!   (`NSMetadataQuery` watcher in `OutlOpsWatcher.swift`): a local folder
//!   relies on the iroh reload signal instead and must not require the
//!   iCloud daemon.
//!
//! On iOS we call `NSFileManager.URLForUbiquityContainerIdentifier(_:)`,
//! which Apple defines as the only correct way to discover the container.
//! On other targets we fall back to the well-known macOS path so the same
//! storage backend can be exercised during development on a Mac.

use std::path::{Path, PathBuf};

/// Resolve the iCloud container root for `container_id`
/// (e.g. `iCloud.app.outl.mobile`). Returns `None` when:
///
/// - the user is not signed into iCloud,
/// - the entitlement is missing or rejected,
/// - the container is still being provisioned by iOS.
///
/// The call may take a few seconds the first time on a device, since iOS
/// negotiates the container with the iCloud servers.
pub fn resolve_container(container_id: &str) -> Option<PathBuf> {
    resolve_container_impl(container_id)
}

#[cfg(target_os = "ios")]
fn resolve_container_impl(container_id: &str) -> Option<PathBuf> {
    use objc2_foundation::{NSFileManager, NSString};

    let id = NSString::from_str(container_id);
    let manager = NSFileManager::defaultManager();
    let url = manager.URLForUbiquityContainerIdentifier(Some(&id))?;
    let path = url.path()?;
    Some(PathBuf::from(path.to_string()))
}

#[cfg(not(target_os = "ios"))]
fn resolve_container_impl(container_id: &str) -> Option<PathBuf> {
    // macOS layout: ~/Library/Mobile Documents/iCloud~app~outl~mobile/
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(home)
        .join("Library")
        .join("Mobile Documents")
        .join(container_id.replace('.', "~"));
    dir.exists().then_some(dir)
}

/// Whether `path` lives inside *any* iCloud Ubiquity Container.
///
/// Used to gate the iCloud-only change detector: the `NSMetadataQuery`
/// watcher (and the `NSFileCoordinator` materialisation dance) only make
/// sense when the workspace folder is in iCloud. A local folder relies on
/// the iroh reload signal instead, so the watcher must not be required for
/// it.
///
/// The detection is path-shape based rather than a live container lookup
/// so it works the same on the host build (where the container may not be
/// provisioned) and stays cheap to call on every boot:
///
/// - iOS: any `URLForUbiquityContainerIdentifier(nil)`-style path contains
///   the `Mobile Documents` segment (the on-device mount of iCloud Drive),
///   matching Apple's documented layout.
/// - host / macOS dev: the same `Library/Mobile Documents/iCloud~…` layout
///   `resolve_container` returns.
///
/// Matching on the `Mobile Documents` path segment covers both without an
/// entitlement round-trip.
pub fn is_inside_icloud(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .is_some_and(|s| s == "Mobile Documents")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icloud_container_path_is_detected() {
        let p = PathBuf::from(
            "/Users/me/Library/Mobile Documents/iCloud~app~outl~mobile-app/Documents",
        );
        assert!(is_inside_icloud(&p));
    }

    #[test]
    fn local_app_data_path_is_not_icloud() {
        // The new default: a folder in the app's local data dir, no iCloud.
        let p = PathBuf::from("/var/mobile/Containers/Data/Application/ABC/Library/outl");
        assert!(!is_inside_icloud(&p));
    }

    #[test]
    fn arbitrary_files_app_folder_is_not_icloud() {
        let p = PathBuf::from("/private/var/mobile/Documents/my-notes");
        assert!(!is_inside_icloud(&p));
    }
}
