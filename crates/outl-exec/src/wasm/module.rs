//! `WasmModule` — the adapter that turns a WASI `.wasm` binary into a
//! [`crate::Runtime`].
//!
//! Use this to host any language whose interpreter ships as a WASI
//! module: pass the bytes, point the language tag, done. The same
//! adapter will eventually back Lisp/JS/Python/Lua once their WASM
//! builds are stable.
//!
//! Contract for the hosted module:
//!
//! - It's a `_start`-style WASI command (`wasmtime <file>` runs it).
//! - Source is delivered on stdin.
//! - Output goes to stdout, diagnostics to stderr.
//! - Exit code 0 = success; non-zero = user-script error.
//!
//! That contract makes every hosted interpreter testable on the host
//! with `wasmtime` directly, without our crate in the loop.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use wasmtime::{Engine, Linker, Module, Store};
use wasmtime_wasi::p1::WasiP1Ctx;
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};
use wasmtime_wasi::WasiCtxBuilder;

use crate::runtime::{ExecContext, ExecError, ExecOutput, ExitStatus, OutputFormat, Runtime};
use crate::wasm::engine::SandboxLimits;

/// A WASM-hosted language runtime.
///
/// Built from a fully-loaded `wasmtime::Module`. Cloning is cheap —
/// the underlying engine/module are `Arc`-shared internally.
pub struct WasmModule {
    language: &'static str,
    engine: Engine,
    module: Module,
    limits: SandboxLimits,
}

impl WasmModule {
    /// Construct from raw WASI module bytes (already validated by
    /// wasmtime on load).
    pub fn from_bytes(
        language: &'static str,
        engine: &Engine,
        wasm: &[u8],
    ) -> Result<Self, ExecError> {
        let module = Module::from_binary(engine, wasm)
            .map_err(|e| ExecError::Sandbox(format!("load wasm: {e}")))?;
        Ok(Self {
            language,
            engine: engine.clone(),
            module,
            limits: SandboxLimits::default(),
        })
    }

    /// Override sandbox limits (fuel + memory cap). The default is
    /// usually enough; tighten for long-running daemons, loosen for
    /// batch jobs.
    pub fn with_limits(mut self, limits: SandboxLimits) -> Self {
        self.limits = limits;
        self
    }
}

impl Runtime for WasmModule {
    fn language(&self) -> &'static str {
        self.language
    }

    fn execute(&self, source: &str, ctx: &ExecContext) -> Result<ExecOutput, ExecError> {
        let start = Instant::now();

        // Pipes: stdin = source bytes, stdout/stderr = in-memory
        // buffers we read back after the run.
        let stdin_pipe = MemoryInputPipe::new(source.as_bytes().to_vec());
        let stdout_pipe = MemoryOutputPipe::new(64 * 1024);
        let stderr_pipe = MemoryOutputPipe::new(64 * 1024);

        let stdout_read = stdout_pipe.clone();
        let stderr_read = stderr_pipe.clone();

        let wasi = WasiCtxBuilder::new()
            .stdin(stdin_pipe)
            .stdout(stdout_pipe)
            .stderr(stderr_pipe)
            // No env, no preopens, no sockets. The script gets to
            // read its stdin and write its stdout. Nothing else.
            .build_p1();

        let mut store = Store::new(&self.engine, wasi);
        store
            .set_fuel(self.limits.fuel)
            .map_err(|e| ExecError::Sandbox(format!("set fuel: {e}")))?;

        // Epoch interruption: bump the engine epoch from a worker
        // thread after `ctx.timeout`. The next wasm instruction will
        // trap and we'll convert it to ExecError::Timeout.
        store.set_epoch_deadline(1);
        let timeout_engine = self.engine.clone();
        let cancel_guard = Arc::new(Mutex::new(false));
        let cancel_for_thread = cancel_guard.clone();
        let timeout = ctx.timeout;
        std::thread::Builder::new()
            .name("outl-wasm-watchdog".into())
            .spawn(move || {
                std::thread::sleep(timeout);
                if !*cancel_for_thread.lock().unwrap() {
                    timeout_engine.increment_epoch();
                }
            })
            .map_err(|e| ExecError::Sandbox(format!("spawn watchdog: {e}")))?;

        let mut linker: Linker<WasiP1Ctx> = Linker::new(&self.engine);
        wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |s| s)
            .map_err(|e| ExecError::Sandbox(format!("link wasi: {e}")))?;

        let instance = linker
            .instantiate(&mut store, &self.module)
            .map_err(|e| ExecError::Sandbox(format!("instantiate: {e}")))?;

        let start_func = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .map_err(|e| ExecError::Sandbox(format!("module missing `_start`: {e}")))?;
        let call_result = start_func.call(&mut store, ());

        // Tell the watchdog we're done so it doesn't kick a future run.
        *cancel_guard.lock().unwrap() = true;

        let stdout = pipe_to_string(&stdout_read);
        let stderr = pipe_to_string(&stderr_read);
        let duration = start.elapsed();

        match call_result {
            Ok(()) => Ok(ExecOutput {
                stdout,
                stderr,
                duration,
                exit: ExitStatus::Ok,
                format: OutputFormat::Text,
            }),
            Err(e) => {
                // Classify the trap.
                let msg = format!("{e:?}");
                if msg.contains("out of fuel") || msg.contains("Interrupt") {
                    return Err(ExecError::Timeout(timeout));
                }
                // WASI `_start` exits via a special trap that carries
                // the exit code; pull it out if present.
                let exit = if let Some(exit_code) =
                    e.downcast_ref::<wasmtime_wasi::I32Exit>().map(|i| i.0)
                {
                    if exit_code == 0 {
                        ExitStatus::Ok
                    } else {
                        ExitStatus::NonZero(exit_code)
                    }
                } else {
                    ExitStatus::Trap(format!("{e}"))
                };
                Ok(ExecOutput {
                    stdout,
                    stderr,
                    duration,
                    exit,
                    format: OutputFormat::Text,
                })
            }
        }
    }
}

fn pipe_to_string(p: &MemoryOutputPipe) -> String {
    String::from_utf8_lossy(p.contents().as_ref()).into_owned()
}
