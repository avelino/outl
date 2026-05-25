//! Cross-platform sandbox helpers shared by every runtime.
//!
//! For now this is just a timeout primitive — `with_timeout` spawns the
//! work on a worker thread and gives the caller back an `ExecError::Timeout`
//! if the channel doesn't deliver in time. The worker thread is *not*
//! joined: if it overruns, it keeps running until it finishes or the
//! process exits. That's a known leak for runtimes without cooperative
//! cancellation (the toy Lisp). The wasmtime backends coming in M2
//! cancel cooperatively via [`wasmtime::Engine::increment_epoch`], so
//! they'll route through the same helper without the leak.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::runtime::ExecError;

/// Run `work` on a worker thread; return its result or
/// `Err(ExecError::Timeout)` if it doesn't finish within `timeout`.
///
/// `work` must be `'static + Send` so it can move to the worker. Pass
/// owned data in (clone if needed) — borrowing across the boundary
/// would force `'static` lifetimes everywhere upstream.
pub fn with_timeout<F, T>(timeout: Duration, work: F) -> Result<T, ExecError>
where
    F: FnOnce() -> Result<T, ExecError> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = mpsc::sync_channel::<Result<T, ExecError>>(1);
    thread::Builder::new()
        .name("outl-exec".into())
        .spawn(move || {
            // If the receiver has been dropped (timeout fired), this
            // send fails silently — that's fine, we're just throwing
            // the result away.
            let _ = tx.send(work());
        })
        .map_err(|e| ExecError::Sandbox(format!("spawn worker: {e}")))?;

    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(ExecError::Timeout(timeout)),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(ExecError::Sandbox(
            "worker thread vanished without a result".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_work_returns_value() {
        let v: Result<i32, ExecError> = with_timeout(Duration::from_secs(2), || Ok(42));
        assert!(matches!(v, Ok(42)));
    }

    #[test]
    fn slow_work_times_out() {
        let v: Result<(), ExecError> = with_timeout(Duration::from_millis(50), || {
            std::thread::sleep(Duration::from_millis(500));
            Ok(())
        });
        assert!(matches!(v, Err(ExecError::Timeout(_))));
    }

    #[test]
    fn inner_error_propagates() {
        let v: Result<(), ExecError> = with_timeout(Duration::from_secs(2), || {
            Err(ExecError::Language("oops".into()))
        });
        match v {
            Err(ExecError::Language(m)) => assert_eq!(m, "oops"),
            other => panic!("expected Language error, got {other:?}"),
        }
    }
}
