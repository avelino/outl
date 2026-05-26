//! `wasmtime::Engine` factory + sandbox knobs.
//!
//! We build the engine once per runtime instance (engines are cheap to
//! share — modules are the heavy thing) and clone it into every store.
//! The configuration is the **single source of truth** for what
//! "sandboxed" means in outl:
//!
//! - Fuel on (consume_fuel = true) → caller decides how many
//!   instructions a run gets.
//! - Epoch interruption on → caller bumps the engine's epoch from a
//!   timer thread for wall-clock cancellation. Today
//!   [`crate::sandbox::with_timeout`] still does the thread-based
//!   timeout; the epoch path is wired up so an upcoming refactor can
//!   move to it without a config churn.

use wasmtime::{Config, Engine, OptLevel};

/// Sandbox limits a single `execute` call may use.
///
/// The defaults are conservative: 1 million instructions, 64 MiB heap,
/// no growth beyond that. Callers tighten as needed (a tiny snippet
/// running in a TUI loop deserves much less than a long batch job).
#[derive(Debug, Clone, Copy)]
pub struct SandboxLimits {
    /// Maximum wasm instructions the run may execute. wasmtime calls
    /// this "fuel"; one unit ≈ one instruction.
    pub fuel: u64,
    /// Hard cap on heap, in bytes. Past this, wasmtime traps.
    pub max_memory_bytes: usize,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            fuel: 5_000_000,
            max_memory_bytes: 64 * 1024 * 1024,
        }
    }
}

/// Build a `wasmtime::Engine` with the outl sandbox configuration.
/// Cheap to call; reuse the result across `WasmModule` instances when
/// possible.
pub fn make_engine() -> Engine {
    let mut cfg = Config::new();
    cfg.consume_fuel(true);
    cfg.epoch_interruption(true);
    cfg.cranelift_opt_level(OptLevel::Speed);
    // WASI requires multi-memory and bulk-memory; both enabled by
    // default on recent wasmtime, but be explicit.
    cfg.wasm_multi_memory(true);
    cfg.wasm_bulk_memory(true);
    Engine::new(&cfg).expect("wasmtime Config we control is always valid")
}
