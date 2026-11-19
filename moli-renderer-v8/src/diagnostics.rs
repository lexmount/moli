//! Best-effort diagnostics shared by the Pages and Workers of one Browser.

use std::sync::{Arc, OnceLock};

use tokio::sync::broadcast;

const CAPACITY: usize = 128;
const TEXT_MAX_BYTES: usize = 4 * 1024;
const STACK_MAX_BYTES: usize = 16 * 1024;

/// The Page or Worker that produced a diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DiagnosticSource {
    /// A top-level Page and all of its descendant frames share this identity.
    Page {
        page_id: u64,
    },
    DedicatedWorker {
        instance_id: u64,
    },
    SharedWorker {
        instance_id: u64,
    },
    ServiceWorker {
        version_id: u64,
    },
}

/// The error reporting entry point that produced a runtime exception diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ExceptionOrigin {
    /// An uncaught exception or an explicit error report such as `reportError()`.
    UncaughtException,
    /// A Promise reported by the `unhandledrejection` algorithm.
    UnhandledRejection,
}

/// A structured, best-effort observation from a Browser.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DiagnosticEvent {
    /// A reported script exception or promise rejection.
    RuntimeException {
        source: DiagnosticSource,
        origin: ExceptionOrigin,
        message: String,
        filename: String,
        /// One-based source line, or zero when unavailable.
        lineno: u32,
        /// One-based source column, or zero when unavailable.
        colno: u32,
        /// Native stack frames, when available.
        stack: Option<String>,
        /// Whether the source realm canceled default reporting for the event.
        default_prevented: bool,
    },
    /// A script entry or dependency could not be loaded.
    ScriptLoadFailure {
        source: DiagnosticSource,
        /// URL of the entry script, or its Document URL for an inline module.
        entry_url: String,
        /// Request URL of the resource that failed to load.
        resource_url: String,
        message: String,
    },
}

/// A live subscription to future diagnostics in one Browser.
///
/// A slow receiver skips older events and receives a lag error with the number
/// skipped. Dropping the receiver removes this subscription.
pub type DiagnosticsSubscription = broadcast::Receiver<Arc<DiagnosticEvent>>;

impl DiagnosticEvent {
    pub(crate) fn runtime_exception(
        source: DiagnosticSource,
        origin: ExceptionOrigin,
        message: &str,
        filename: &str,
        (lineno, colno): (u32, u32),
        stack: Option<&str>,
        default_prevented: bool,
    ) -> Self {
        Self::RuntimeException {
            source,
            origin,
            message: bounded(message, TEXT_MAX_BYTES),
            filename: bounded(filename, TEXT_MAX_BYTES),
            lineno,
            colno,
            stack: stack.map(|stack| bounded(stack, STACK_MAX_BYTES)),
            default_prevented,
        }
    }

    pub(crate) fn script_load_failure(
        source: DiagnosticSource,
        entry_url: &str,
        resource_url: &str,
        message: &str,
    ) -> Self {
        Self::ScriptLoadFailure {
            source,
            entry_url: bounded(entry_url, TEXT_MAX_BYTES),
            resource_url: bounded(resource_url, TEXT_MAX_BYTES),
            message: bounded(message, TEXT_MAX_BYTES),
        }
    }
}

fn bounded(value: &str, max_bytes: usize) -> String {
    value[..value.floor_char_boundary(max_bytes)].to_owned()
}

#[derive(Clone, Debug, Default)]
pub(crate) struct DiagnosticEmitter(Arc<OnceLock<broadcast::Sender<Arc<DiagnosticEvent>>>>);

impl DiagnosticEmitter {
    pub(crate) fn subscribe(&self) -> DiagnosticsSubscription {
        self.0
            .get_or_init(|| broadcast::Sender::new(CAPACITY))
            .subscribe()
    }

    pub(crate) fn emit(&self, build: impl FnOnce() -> Option<DiagnosticEvent>) {
        if let Some(sender) = self.0.get()
            && sender.receiver_count() != 0
            && let Some(event) = build()
        {
            let _ = sender.send(Arc::new(event));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_are_lazy_after_the_last_receiver_is_dropped() {
        let diagnostics = DiagnosticEmitter::default();
        assert!(diagnostics.0.get().is_none());
        diagnostics.emit(|| panic!("no receiver"));
        let receiver = diagnostics.subscribe();
        drop(receiver);
        diagnostics.emit(|| panic!("the receiver was dropped"));
    }

    #[test]
    fn slow_diagnostic_receivers_lag_and_messages_are_bounded() {
        let diagnostics = DiagnosticEmitter::default();
        let mut slow = diagnostics.subscribe();
        let mut fast = diagnostics.subscribe();
        let message = "€".repeat(TEXT_MAX_BYTES);
        for _ in 0..=CAPACITY {
            diagnostics.emit(|| {
                Some(DiagnosticEvent::script_load_failure(
                    DiagnosticSource::Page { page_id: 1 },
                    "https://example.test/missing.js",
                    "https://example.test/missing.js",
                    &message,
                ))
            });
            let event = fast.try_recv().expect("event for fast receiver");
            assert!(matches!(
                event.as_ref(),
                DiagnosticEvent::ScriptLoadFailure { message, .. } if message.len() == 4095
            ));
        }
        assert_eq!(
            slow.try_recv(),
            Err(broadcast::error::TryRecvError::Lagged(1))
        );
        assert!(slow.try_recv().is_ok());
    }
}
