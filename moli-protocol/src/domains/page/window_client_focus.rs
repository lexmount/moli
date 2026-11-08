use anyhow::Result;
use moli_core::page::{RendererPendingWindowClientFocus, RendererWindowDocumentSource};

use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, CommandWorkflowContext, PageTargetHostIdentity,
    ProtocolWorkflowDriver, RendererPageResidenceIdentity, TargetPageResidenceIdentity,
};
use crate::domains::activity::ProtocolOutputPayloads;
use crate::domains::target::activation::{PreparedTargetActivation, TargetActivationOutput};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WindowClientFocusOutput {
    page: TargetPageResidenceIdentity,
    renderer: RendererPageResidenceIdentity,
    request: RendererPendingWindowClientFocus,
}

#[derive(Debug, Default)]
pub(crate) struct WindowClientFocusPreparedOutputs(Vec<WindowClientFocusOutput>);

impl WindowClientFocusPreparedOutputs {
    pub(crate) fn from_renderer(
        source: Option<(TargetPageResidenceIdentity, RendererPageResidenceIdentity)>,
        request: RendererPendingWindowClientFocus,
    ) -> Self {
        Self(match source {
            Some((page, renderer)) => vec![WindowClientFocusOutput {
                page,
                renderer,
                request,
            }],
            None => Vec::new(),
        })
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.0.extend(other.0);
    }
}

pub(crate) struct PreparedWindowClientFocusProjection(Vec<WindowClientFocusOutput>);

impl PreparedWindowClientFocusProjection {
    pub(in crate::domains) fn take(payloads: &mut ProtocolOutputPayloads) -> Option<Self> {
        let outputs = payloads.window_client_focus_mut()?;
        (!outputs.0.is_empty()).then(|| Self(std::mem::take(&mut outputs.0)))
    }

    pub(crate) async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        for output in self.0 {
            let Some((activation, guard)) =
                driver.with_state(move |state| state.prepare_window_client_focus(output))?
            else {
                continue;
            };
            let result = activation.run(driver).await;
            let events = driver
                .with_state(move |state| state.complete_window_client_focus(guard, result))?;
            context.context_mut().protocol_events_mut().extend(events);
        }
        Ok(())
    }
}

pub(crate) struct WindowClientFocusActivationGuard {
    source: WindowClientFocusOutput,
    target_id: String,
    host: PageTargetHostIdentity,
}

impl WindowClientFocusOutput {
    fn target(&self, conn: &CdpConnection) -> Option<String> {
        if !self.request.is_current() {
            return None;
        }
        let context = conn.browser_context_by_id(self.page.browser_context_id())?;
        if !context.target_page_residence_is_current(&self.page) {
            return None;
        }
        let source_target = context.page_target(self.page.target_id()?)?;
        if source_target.closing
            || source_target
                .runtime_slot
                .loaded_page()
                .map(RendererPageResidenceIdentity::from_page)
                != Some(self.renderer)
            || source_target
                .runtime_slot
                .committed_renderer_document_binding()?
                .renderer_document_identity()
                != self.request.source_document
        {
            return None;
        }
        match &self.request.window {
            RendererWindowDocumentSource::RootFrame
            | RendererWindowDocumentSource::ChildFrame { .. } => {
                self.page.target_id().map(str::to_owned)
            }
            RendererWindowDocumentSource::LightweightPopup { popup_id, .. } => {
                context.target_id_for_popup_id(*popup_id).map(str::to_owned)
            }
        }
    }

    pub(crate) fn prepare(
        self,
        conn: &mut CdpConnection,
    ) -> Option<(PreparedTargetActivation, WindowClientFocusActivationGuard)> {
        let target_id = self.target(conn)?;
        let target = conn
            .browser_context_by_id(self.page.browser_context_id())?
            .page_target(&target_id)?;
        if target.closing {
            return None;
        }
        let host = target.host_identity();
        let activation = crate::domains::target::activation::prepare_target_activation_for_target(
            conn, &target_id,
        )
        .ok()??;
        Some((
            activation,
            WindowClientFocusActivationGuard {
                source: self,
                target_id,
                host,
            },
        ))
    }
}

impl WindowClientFocusActivationGuard {
    pub(crate) fn complete(
        self,
        conn: &mut CdpConnection,
        result: Result<TargetActivationOutput>,
    ) -> Vec<BackgroundProtocolEvent> {
        let current = self.source.target(conn).as_deref() == Some(self.target_id.as_str())
            && conn
                .browser_context_by_id(self.source.page.browser_context_id())
                .and_then(|context| context.page_target(&self.target_id))
                .is_some_and(|target| {
                    !target.closing && self.host.matches(&target.host_identity())
                });
        let (activated, events) = match result {
            Ok(output) => (current, output.finish(conn)),
            Err(error) => {
                tracing::debug!(%error,"WindowClient.focus browser activation failed");
                (false, Vec::new())
            }
        };
        self.source.request.finish(activated);
        events
    }
}
