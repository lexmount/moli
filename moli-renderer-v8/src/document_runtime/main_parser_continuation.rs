use super::DocumentRuntime;
use crate::{
    frame_owner_model::FrameDocumentTaskOwner,
    page_task_queue::{
        MainParserContinuationRequest, RendererPageMainParserContinuationOwner,
        RendererPageMainParserContinuationProducer, RendererPageMainParserContinuationSender,
    },
};

/// Document-owned endpoint for the main parser's one-way resume protocol.
///
/// The Page Networking source owns resident task ordering. This state only
/// retains the producer bound to the current exact Document and the admission
/// fact set by a selected continuation task. Parser runtime/input state never
/// lives here.
#[derive(Debug)]
pub(super) struct MainParserContinuationState {
    sender: RendererPageMainParserContinuationSender,
    producer: Option<RendererPageMainParserContinuationProducer>,
    phase_one_active: bool,
    admitted: bool,
}

impl MainParserContinuationState {
    pub(super) fn new(sender: RendererPageMainParserContinuationSender) -> Self {
        Self {
            sender,
            producer: None,
            phase_one_active: false,
            admitted: false,
        }
    }

    fn reset_for_document_replacement(&mut self) {
        self.producer = None;
        self.phase_one_active = false;
        self.admitted = false;
    }
}

impl DocumentRuntime {
    pub(crate) fn bind_main_parser_continuation_producer(&mut self, owner: FrameDocumentTaskOwner) {
        self.main_parser_continuation.producer =
            Some(self.main_parser_continuation.sender.bind_producer(owner));
        self.main_parser_continuation.admitted = false;
    }

    /// Enable parser continuation admission for the one phase-one runtime that
    /// owns this exact Document.
    pub(crate) fn activate_main_parser_continuation(&mut self, owner: FrameDocumentTaskOwner) {
        let producer = self
            .main_parser_continuation
            .producer
            .as_ref()
            .expect("main parser continuation producer must be bound before phase one starts");
        assert_eq!(
            producer.owner().document_owner(),
            owner,
            "phase-one parser owner must match its bound continuation producer"
        );
        self.main_parser_continuation.phase_one_active = true;
        self.main_parser_continuation.admitted = false;
    }

    pub(crate) fn deactivate_main_parser_continuation(&mut self) {
        self.main_parser_continuation.phase_one_active = false;
        self.main_parser_continuation.admitted = false;
    }

    pub(crate) fn main_parser_continuation_producer(
        &self,
    ) -> Option<RendererPageMainParserContinuationProducer> {
        self.main_parser_continuation
            .phase_one_active
            .then(|| self.main_parser_continuation.producer.clone())
            .flatten()
    }

    /// Request a parser opportunity after first committing the producer's
    /// authoritative state change.
    pub(crate) fn request_main_parser_continuation_if_active(&self) -> bool {
        let Some(producer) = self.main_parser_continuation_producer() else {
            return false;
        };
        match producer.request() {
            Ok(
                MainParserContinuationRequest::Enqueued
                | MainParserContinuationRequest::AlreadyQueued,
            ) => true,
            Err(_) => panic!(
                "active main parser continuation route closed before its Document was retired: {producer:?}"
            ),
        }
    }

    /// Consume one selected Networking task into the phase-one admission bit.
    ///
    /// Exact root-Document authorization is performed by `PageVm`; this layer
    /// additionally checks the current Document/runtime producer and whether a
    /// parser residence is still active.
    pub(crate) fn admit_selected_main_parser_continuation(
        &mut self,
        owner: RendererPageMainParserContinuationOwner,
    ) -> bool {
        let current = self
            .main_parser_continuation
            .producer
            .as_ref()
            .is_some_and(|producer| producer.owner() == owner);
        if !self.main_parser_continuation.phase_one_active || !current {
            return false;
        }
        self.main_parser_continuation.admitted = true;
        true
    }

    pub(crate) fn take_main_parser_continuation_admission(&mut self) -> bool {
        std::mem::take(&mut self.main_parser_continuation.admitted)
    }

    pub(crate) fn has_main_parser_continuation_admission(&self) -> bool {
        self.main_parser_continuation.admitted
    }

    pub(super) fn reset_main_parser_continuation_for_document_replacement(&mut self) {
        self.main_parser_continuation
            .reset_for_document_replacement();
    }
}
