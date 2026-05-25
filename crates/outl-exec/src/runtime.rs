//! The `Runtime` trait and its surrounding value types.
//!
//! Every backend (toy Lisp today, wasmtime-hosted interpreters
//! tomorrow) implements [`Runtime`]. The trait is *deliberately tiny* —
//! a single `execute(source, ctx) -> result` — so that we can swap
//! implementations later without dragging UI code along.

use std::path::PathBuf;
use std::time::Duration;

use thiserror::Error;

/// A language backend.
///
/// The contract is intentionally Unix-y: take a `source` string, return
/// stdout / stderr / exit-status / duration. Errors bubble up through
/// [`ExecError`].
///
/// Implementations **must** honour `ctx.timeout`. If your runtime can't
/// be cancelled cooperatively, wrap the work in
/// [`crate::sandbox::with_timeout`] which spawns the work on a separate
/// thread and drops the channel on overrun.
pub trait Runtime: Send + Sync {
    /// The fence info-string this runtime claims. Matched
    /// case-insensitively against ` ```<lang> `.
    fn language(&self) -> &'static str;

    /// Run `source` and return what happened. Returning `Ok` with a
    /// non-zero [`ExitStatus`] signals a *user-level* error (the script
    /// ran but crashed); returning `Err` signals an infrastructure
    /// error (timeout, OOM, missing toolchain).
    fn execute(&self, source: &str, ctx: &ExecContext) -> Result<ExecOutput, ExecError>;
}

/// Context passed to every execution.
///
/// We deliberately keep this small. Anything runtime-specific (env vars,
/// preopened directories, sandbox tweaks) lives inside the runtime
/// itself.
#[derive(Debug, Clone)]
pub struct ExecContext {
    /// Workspace root — runtimes that resolve relative file references
    /// (`include "./helper.lisp"`) start here.
    pub workspace_root: PathBuf,
    /// Optional content piped to the script as stdin. Future: chain
    /// blocks via `((ref))`.
    pub stdin: Option<String>,
    /// Hard wall-clock limit. Past this we kill the run.
    pub timeout: Duration,
    /// Optional heap cap. Honoured only by runtimes that can enforce
    /// it (wasmtime can; in-process toy interpreters can't yet).
    pub mem_limit: Option<usize>,
}

impl Default for ExecContext {
    fn default() -> Self {
        Self {
            workspace_root: PathBuf::from("."),
            stdin: None,
            timeout: Duration::from_secs(5),
            mem_limit: None,
        }
    }
}

/// What an execution produced.
#[derive(Debug, Clone)]
pub struct ExecOutput {
    /// Captured stdout.
    pub stdout: String,
    /// Captured stderr.
    pub stderr: String,
    /// Wall-clock duration of the call to `execute`.
    pub duration: Duration,
    /// How it ended.
    pub exit: ExitStatus,
}

/// How a run terminated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitStatus {
    /// Normal completion.
    Ok,
    /// Script returned non-zero — user-level error, not an
    /// infrastructure failure.
    NonZero(i32),
    /// Runtime trapped (panic, division by zero, etc). Message is
    /// runtime-specific.
    Trap(String),
}

/// Infrastructure-level errors.
///
/// User-script errors (a Lisp `(error ...)` form, a Python exception)
/// surface as `Ok(ExecOutput { exit: NonZero | Trap, .. })`. This enum
/// is reserved for "your sandbox didn't even get to run the code".
#[derive(Debug, Error)]
pub enum ExecError {
    /// `ctx.timeout` elapsed before the script finished.
    #[error("execution timed out after {0:?}")]
    Timeout(Duration),
    /// Out of memory — currently only fired by wasmtime-backed runtimes.
    #[error("out of memory")]
    OutOfMemory,
    /// Language-specific parse / compile failure (e.g. malformed Lisp).
    #[error("{0}")]
    Language(String),
    /// Sandbox setup failed (toolchain missing, wasm load error, ...).
    #[error("sandbox: {0}")]
    Sandbox(String),
    /// I/O failure reading source or writing artifacts.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
