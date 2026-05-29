//! Resolve the iCloud Ubiquity Container path for this app.
//!
//! On iOS we call `NSFileManager.URLForUbiquityContainerIdentifier(_:)`,
//! which Apple defines as the only correct way to discover the container.
//! On other targets we fall back to the well-known macOS path so the same
//! storage backend can be exercised during development on a Mac.

use std::path::PathBuf;

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
