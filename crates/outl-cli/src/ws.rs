//! Shared workspace bootstrap used by every machine-shaped subcommand.
//!
//! The open logic lives in [`outl_ws`] (shared with embedders); this
//! module keeps the CLI-facing signature, mapping every boot failure to
//! an [`ApiError`] with a stable code.

use std::path::Path;

pub use outl_ws::WsCtx;
use outl_ws::WsError;

use crate::output::{codes, ApiError};

/// Open the workspace at `path`. Returns a stable [`ApiError`] on any
/// boot failure (missing workspace, bad config, filesystem error).
///
/// See [`outl_ws::open`] for the lock/actor policy.
pub fn open(path: &Path) -> Result<WsCtx, ApiError> {
    outl_ws::open(path).map_err(|e| match e {
        WsError::NoWorkspace(_) => ApiError::new(codes::NO_WORKSPACE, e.to_string()),
        WsError::Config(_) => ApiError::new(codes::NO_WORKSPACE, e.to_string()),
        WsError::InvalidActor(_) => ApiError::new(codes::INTERNAL, e.to_string()),
        WsError::Internal(msg) => ApiError::internal(msg),
    })
}
