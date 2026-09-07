use moli_core::RendererOutputTransportSender;

use super::{
    BackgroundEventSender, RuntimeInspectorResponseReady, RuntimeInspectorResponseReadySender,
};

#[derive(Default)]
pub(super) struct CdpSchedulerHooks {
    background_event_sender: Option<BackgroundEventSender>,
    background_navigation_completion_sender: Option<
        tokio::sync::mpsc::UnboundedSender<crate::domains::page::BackgroundNavigationCompletion>,
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

    pub(super) fn bind_runtime_inspector_response_ready(
        &mut self,
    ) -> Option<tokio::sync::mpsc::UnboundedReceiver<RuntimeInspectorResponseReady>> {
        if self.runtime_inspector_response_ready_sender.is_some() {
            return None;
        }
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        self.runtime_inspector_response_ready_sender = Some(sender);
        Some(receiver)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_completion_ingress_cannot_be_replaced_or_reopened() {
        let mut hooks = CdpSchedulerHooks::default();
        assert!(hooks.runtime_inspector_response_ready_sender().is_none());
        let mut receiver = hooks.bind_runtime_inspector_response_ready().unwrap();
        let callback_sender = hooks.runtime_inspector_response_ready_sender().unwrap();
        assert!(hooks.bind_runtime_inspector_response_ready().is_none());
        assert!(
            callback_sender.same_channel(&hooks.runtime_inspector_response_ready_sender().unwrap())
        );

        let response = RuntimeInspectorResponseReady::new(
            42,
            Some("original-session"),
            Err("original-callback".to_owned()),
        );
        callback_sender.send(response.clone()).unwrap();
        assert_eq!(receiver.try_recv().unwrap(), response);
        drop(receiver);
        assert!(hooks.bind_runtime_inspector_response_ready().is_none());
        assert_eq!(
            callback_sender.send(response.clone()).unwrap_err().0,
            response
        );
    }
}
