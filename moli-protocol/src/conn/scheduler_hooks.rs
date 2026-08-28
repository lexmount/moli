use moli_core::RendererOutputTransportSender;

use super::{BackgroundEventSender, RuntimeInspectorResponseReadySender};

#[derive(Default)]
pub(super) struct CdpSchedulerHooks {
    background_event_sender: Option<BackgroundEventSender>,
    background_navigation_completion_sender: Option<
        tokio::sync::mpsc::UnboundedSender<crate::domains::page::BackgroundNavigationCompletion>,
    >,
    document_continuation_completion_sender: Option<
        tokio::sync::mpsc::UnboundedSender<
            crate::domains::page::BackgroundDocumentContinuationCompletion,
        >,
    >,
    renderer_publication_sender: Option<RendererOutputTransportSender>,
    runtime_inspector_response_ready_sender: Option<RuntimeInspectorResponseReadySender>,
}

impl CdpSchedulerHooks {
    pub(super) fn set_background_event_sender(&mut self, sender: BackgroundEventSender) {
        self.background_event_sender = Some(sender);
    }

    pub(super) fn background_event_sender(&self) -> Option<BackgroundEventSender> {
        self.background_event_sender.clone()
    }

    pub(super) fn has_background_event_sender(&self) -> bool {
        self.background_event_sender.is_some()
    }

    pub(super) fn set_runtime_inspector_response_ready_sender(
        &mut self,
        sender: RuntimeInspectorResponseReadySender,
    ) {
        self.runtime_inspector_response_ready_sender = Some(sender);
    }

    pub(super) fn runtime_inspector_response_ready_sender(
        &self,
    ) -> Option<RuntimeInspectorResponseReadySender> {
        self.runtime_inspector_response_ready_sender.clone()
    }

    pub(super) fn set_background_navigation_completion_sender(
        &mut self,
        sender: tokio::sync::mpsc::UnboundedSender<
            crate::domains::page::BackgroundNavigationCompletion,
        >,
    ) {
        self.background_navigation_completion_sender = Some(sender);
    }

    pub(super) fn background_navigation_completion_sender(
        &self,
    ) -> Option<
        tokio::sync::mpsc::UnboundedSender<crate::domains::page::BackgroundNavigationCompletion>,
    > {
        self.background_navigation_completion_sender.clone()
    }

    pub(super) fn has_background_navigation_completion_sender(&self) -> bool {
        self.background_navigation_completion_sender.is_some()
    }

    pub(super) fn set_document_continuation_completion_sender(
        &mut self,
        sender: tokio::sync::mpsc::UnboundedSender<
            crate::domains::page::BackgroundDocumentContinuationCompletion,
        >,
    ) {
        self.document_continuation_completion_sender = Some(sender);
    }

    pub(super) fn document_continuation_completion_sender(
        &self,
    ) -> Option<
        tokio::sync::mpsc::UnboundedSender<
            crate::domains::page::BackgroundDocumentContinuationCompletion,
        >,
    > {
        self.document_continuation_completion_sender.clone()
    }

    pub(super) fn has_document_continuation_completion_sender(&self) -> bool {
        self.document_continuation_completion_sender.is_some()
    }

    pub(super) fn set_renderer_publication_sender(
        &mut self,
        sender: RendererOutputTransportSender,
    ) {
        self.renderer_publication_sender = Some(sender);
    }

    pub(super) fn renderer_publication_sender(&self) -> Option<RendererOutputTransportSender> {
        self.renderer_publication_sender.clone()
    }
}
