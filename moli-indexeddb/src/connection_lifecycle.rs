use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::TransactionRequestWake;

/// Survives removal of the native connection without retaining its manager.
#[derive(Clone, Debug, Default)]
pub struct DatabaseCloseSignal(Arc<AtomicBool>);

impl DatabaseCloseSignal {
    pub fn is_forced_closed(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub(crate) fn already_closed(wake: TransactionRequestWake) -> Self {
        let signal = Self::default();
        signal.0.store(true, Ordering::Release);
        wake();
        signal
    }
}

#[derive(Clone, Default)]
pub(crate) struct DatabaseConnectionLifecycle {
    signal: DatabaseCloseSignal,
    wake: Option<TransactionRequestWake>,
}

impl std::fmt::Debug for DatabaseConnectionLifecycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatabaseConnectionLifecycle")
            .field("signal", &self.signal)
            .finish_non_exhaustive()
    }
}

impl DatabaseConnectionLifecycle {
    pub(crate) fn observe(&mut self, wake: TransactionRequestWake) -> DatabaseCloseSignal {
        self.wake = Some(wake);
        self.signal.clone()
    }

    /// Mark closure before waking canceled transactions. The caller publishes
    /// this wake only after rollback and coordinator cancellation are complete.
    pub(crate) fn mark_forced_closed(&mut self) -> Option<TransactionRequestWake> {
        self.signal.0.store(true, Ordering::Release);
        self.wake.take()
    }
}
