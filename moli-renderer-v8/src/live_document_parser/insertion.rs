use super::{
    DocumentParserCloseDisposition, DocumentParserLifetime, DocumentParserRunState,
    DocumentParserSessionControlHandle, LiveDocumentParserDiscoverySignals,
    LiveDocumentParserOwner, LiveDocumentParserStepOutcome, ParserResumeOwner, ParserStopReason,
    ParserSuspensionCause, advance_next_live_document_parser_step,
};
use std::{cell::RefCell, rc::Rc};

/// Synchronous access to the owning session's input and control state.
/// This handle cannot finish a parser, replace its backend, or outlive its
/// owner's cancellation. It never owns a separate input queue or lifetime.
#[derive(Clone)]
pub(crate) struct ParserInsertionHandle {
    pub(super) controller: crate::document_runtime::ParserInsertionController,
    pub(super) control: DocumentParserSessionControlHandle,
    pub(super) discovery_signals: Rc<RefCell<LiveDocumentParserDiscoverySignals>>,
}

impl ParserInsertionHandle {
    pub(crate) fn control_handle(&self) -> DocumentParserSessionControlHandle {
        self.control.clone()
    }

    pub(crate) fn run_state(&self) -> DocumentParserRunState {
        self.control.run_state()
    }

    pub(crate) fn lifetime(&self) -> DocumentParserLifetime {
        self.control.lifetime()
    }

    pub(crate) fn request_close(&self) -> DocumentParserCloseDisposition {
        self.control.request_close()
    }

    pub(crate) fn is_suspended(&self) -> bool {
        matches!(self.run_state(), DocumentParserRunState::Suspended { .. })
    }

    pub(crate) fn suspend(&self, cause: ParserSuspensionCause) {
        let _ = self.control.suspend(cause, ParserResumeOwner::ParserDriver);
    }

    pub(crate) fn stop(&self, reason: ParserStopReason) {
        self.control.stop(reason);
    }

    pub(crate) fn enqueue_script_input_html(&self, chunk: String) {
        self.controller
            .input_session()
            .enqueue_script_input_html(chunk);
    }

    pub(crate) fn append_to_current_inserted_input(&self, chunk: &str) -> bool {
        self.controller
            .with_parser_stream(|stream| stream.append_to_current_inserted_input(chunk))
    }

    pub(crate) fn append_at_current_insertion_point(&self, chunk: &str) {
        self.controller
            .with_parser_stream(|stream| stream.append_at_current_insertion_point(chunk));
    }

    pub(crate) fn queue_arrived_chunk(&self, chunk: String) {
        self.controller
            .with_parser_stream(|stream| stream.append_to_end(chunk));
    }

    pub(crate) fn input_is_empty(&self) -> bool {
        self.controller
            .with_parser_stream(|stream| !stream.has_pending_input())
    }

    pub(crate) fn advance_queued_or_resume_step(
        &self,
        owner: &mut impl LiveDocumentParserOwner,
    ) -> LiveDocumentParserStepOutcome {
        let _pump = self.control.begin_pump();
        let advance = self
            .controller
            .with_parser_stream(|stream| advance_next_live_document_parser_step(stream, 0, owner));
        self.discovery_signals
            .borrow_mut()
            .extend(advance.discovery_signals);
        advance.outcome
    }

    pub(crate) fn take_discovery_signals(&self) -> LiveDocumentParserDiscoverySignals {
        std::mem::take(&mut *self.discovery_signals.borrow_mut())
    }
}
