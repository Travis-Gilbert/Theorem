//! Cancellation as a runtime-free polled flag.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A cancellation flag shared between a [`crate::RunHandle`] and any worker
/// holding a handle to it.
///
/// The SDK models cancellation as a polled flag rather than relying on platform
/// cancellation, because the FFI binding generators (UniFFI in particular) do
/// not support cancelling an in-flight foreign future. A run loop checks
/// [`CancelToken::is_cancelled`] at each transition boundary (the state machine
/// already exposes these as natural checkpoints) and drives a clean
/// `RUN.CANCELLED`. Because the flag lives in the SDK, every generated binding
/// gets identical cancellation semantics for free.
#[derive(Clone, Debug, Default)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
}

impl CancelToken {
    /// A fresh, not-yet-cancelled token.
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Request cancellation. Idempotent.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    /// Whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// A handle that shares the same underlying flag, for handing the cancel
    /// signal to a worker without exposing the run.
    pub fn handle(&self) -> Self {
        self.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_uncancelled() {
        assert!(!CancelToken::new().is_cancelled());
    }

    #[test]
    fn cancel_sets_flag() {
        let token = CancelToken::new();
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn handle_shares_flag() {
        let token = CancelToken::new();
        let worker = token.handle();
        token.cancel();
        assert!(worker.is_cancelled());
    }
}
