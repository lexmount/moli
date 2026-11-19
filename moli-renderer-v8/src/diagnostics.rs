//! Diagnostics shared by the Pages and Workers of one Browser.

use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;
use tokio::sync::mpsc;

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

/// Result of the Web exception or unhandled rejection reporting algorithm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ErrorReportOutcome {
    /// Default reporting was canceled by a listener in the reporting chain.
    Canceled,
    /// Reporting reached the end of the chain with its default action allowed.
    Uncanceled,
    /// The reporting chain could not complete, for example when its Window was retired.
    Interrupted,
}

impl ErrorReportOutcome {
    pub(crate) fn from_default_prevented(default_prevented: bool) -> Self {
        if default_prevented {
            Self::Canceled
        } else {
            Self::Uncanceled
        }
    }
}

/// A structured observation from a Browser.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DiagnosticEvent {
    /// A script exception or promise rejection with the result of its reporting chain.
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
        /// The result of reporting, including any propagation to Worker owners.
        outcome: ErrorReportOutcome,
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
/// Producers enqueue events without waiting for the receiver. Consume events
/// regularly to limit the memory used by the unbounded queue. To stop collection,
/// close the receiver and then drain the queue. Dropping the receiver discards
/// any queued events.
pub type DiagnosticsSubscription = mpsc::UnboundedReceiver<DiagnosticEvent>;

impl DiagnosticEvent {
    pub(crate) fn runtime_exception(
        source: DiagnosticSource,
        origin: ExceptionOrigin,
        message: &str,
        filename: &str,
        location: (u32, u32),
        stack: Option<String>,
        outcome: ErrorReportOutcome,
    ) -> Self {
        let (lineno, colno) = location;
        Self::RuntimeException {
            source,
            origin,
            message: bounded(message, TEXT_MAX_BYTES),
            filename: bounded(filename, TEXT_MAX_BYTES),
            lineno,
            colno,
            stack: stack.map(bound_stack),
            outcome,
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

fn bound_stack(mut stack: String) -> String {
    stack.truncate(stack.floor_char_boundary(STACK_MAX_BYTES));
    if stack.capacity() > STACK_MAX_BYTES {
        stack.shrink_to_fit();
    }
    stack
}

type DiagnosticSender = mpsc::UnboundedSender<DiagnosticEvent>;

#[derive(Clone, Debug, Default)]
pub(crate) struct DiagnosticEmitter(Arc<OnceLock<Mutex<Option<DiagnosticSender>>>>);

impl DiagnosticEmitter {
    pub(crate) fn subscribe(&self) -> Option<DiagnosticsSubscription> {
        let mut slot = self.0.get_or_init(Mutex::default).lock();
        if slot.as_ref().is_some_and(|sender| !sender.is_closed()) {
            return None;
        }
        let (sender, receiver) = mpsc::unbounded_channel();
        *slot = Some(sender);
        Some(receiver)
    }

    pub(crate) fn emit(&self, build: impl FnOnce() -> Option<DiagnosticEvent>) {
        if let Some(sender) = self.active_sender()
            && let Some(event) = build()
        {
            let _ = sender.send(event);
        }
    }

    pub(crate) fn prepare_worker_exception(
        &self,
        source: DiagnosticSource,
        capture_stack: impl FnOnce() -> Option<String>,
    ) -> Option<WorkerExceptionDiagnostic> {
        let sender = self.active_sender()?;
        Some(WorkerExceptionDiagnostic {
            sender,
            source,
            stack: capture_stack().map(bound_stack),
        })
    }

    fn active_sender(&self) -> Option<DiagnosticSender> {
        self.0
            .get()?
            .lock()
            .as_ref()
            .filter(|sender| !sender.is_closed())
            .cloned()
    }
}

/// The subscription and source-isolate details needed by a Worker error report.
#[derive(Debug)]
pub(crate) struct WorkerExceptionDiagnostic {
    sender: DiagnosticSender,
    source: DiagnosticSource,
    stack: Option<String>,
}

impl WorkerExceptionDiagnostic {
    pub(crate) fn publish(
        self,
        message: &str,
        filename: &str,
        location: (u32, u32),
        outcome: impl FnOnce() -> ErrorReportOutcome,
    ) {
        if !self.sender.is_closed() {
            let _ = self.sender.send(DiagnosticEvent::runtime_exception(
                self.source,
                ExceptionOrigin::UncaughtException,
                message,
                filename,
                location,
                self.stack,
                outcome(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_are_lazy_without_an_open_receiver() {
        let diagnostics = DiagnosticEmitter::default();
        assert!(diagnostics.0.get().is_none());
        diagnostics.emit(|| panic!("no receiver"));
        assert!(
            diagnostics
                .prepare_worker_exception(
                    DiagnosticSource::DedicatedWorker { instance_id: 2 },
                    || { panic!("no receiver") }
                )
                .is_none()
        );

        let mut receiver = diagnostics.subscribe().expect("first subscription");
        assert!(diagnostics.clone().subscribe().is_none());
        receiver.close();
        diagnostics.emit(|| panic!("the receiver was closed"));

        let replacement = diagnostics
            .subscribe()
            .expect("closed receiver can be replaced");
        drop(replacement);
        diagnostics.emit(|| panic!("the receiver was dropped"));
        assert!(diagnostics.subscribe().is_some());
    }

    #[test]
    fn slow_diagnostic_receiver_keeps_all_events_with_bounded_messages() {
        let diagnostics = DiagnosticEmitter::default();
        let mut receiver = diagnostics.subscribe().expect("first subscription");
        let message = "€".repeat(TEXT_MAX_BYTES);
        for index in 0..256 {
            diagnostics.emit(|| {
                Some(DiagnosticEvent::script_load_failure(
                    DiagnosticSource::Page { page_id: 1 },
                    &format!("https://example.test/{index}.js"),
                    "https://example.test/missing.js",
                    &message,
                ))
            });
        }
        for index in 0..256 {
            let event = receiver.try_recv().expect("queued event");
            assert!(matches!(
                event,
                DiagnosticEvent::ScriptLoadFailure { entry_url, message, .. }
                    if entry_url == format!("https://example.test/{index}.js")
                        && message.len() == 4095
            ));
        }
        assert_eq!(receiver.try_recv(), Err(mpsc::error::TryRecvError::Empty));
    }
}
