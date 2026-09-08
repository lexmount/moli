use super::{CdpConnection, CdpTurnOutcome};
use crate::domains::activity::{
    ReadyProtocolSchedulerWork, RendererCommandResponseCompletion, RendererCommandResponseOrder,
    RendererCommandResponsePermit,
};
use moli_core::RendererOutputTransportMessage;

impl CdpConnection {
    pub(crate) async fn prepare_renderer_javascript_dialog(
        &self,
        owner: &super::CommandOwnerScope,
        renderer: super::RendererPageResidenceIdentity,
        opening: std::sync::Arc<moli_core::page::RendererJavaScriptDialogOpening>,
    ) -> Option<super::TargetPreparedJavaScriptDialog> {
        // Freeze the frontend attachment before awaiting native admission.
        let route = self
            .target_page_protocol_attachment_identity_for_owner(owner)
            .zip(self.target_session_owner_frame_tree_identity_for_owner(owner))
            .zip(
                self.runtime_session_owner_slot_for_owner(owner)
                    .ok()
                    .map(|slot| slot.javascript_dialog_scope_observer()),
            );
        let dialog = self
            .browser
            .wait_for_renderer_javascript_dialog(renderer, opening)
            .await?;
        let context = self
            .browser
            .context_handle(dialog.document.web_contents().context())
            .ok()?;
        let Some(((attachment, (frame, _, _, _)), scope)) = route.filter(|((attachment, _), _)| {
            attachment.page_owner().document_id() == dialog.document.id()
        }) else {
            context.dismiss_document_javascript_dialog(dialog.document, dialog.key);
            return None;
        };
        Some(super::TargetPreparedJavaScriptDialog::capture(
            attachment, scope, &frame, context, dialog,
        ))
    }

    pub(crate) fn native_renderer_page_output_owner(
        &self,
        renderer: super::RendererPageResidenceIdentity,
    ) -> Option<crate::domains::activity::RendererPublicationOwner> {
        let document = self.browser.document_for_renderer(renderer)?;
        let context = self.browser_context_by_browser_id(document.web_contents().context())?;
        let target = context
            .page_targets
            .get_for_web_contents(document.web_contents().id())?;
        Some(
            crate::domains::activity::RendererPublicationOwner::PageTarget {
                browser_context_id: context.id.clone(),
                target_id: Some(target.target_id().to_owned()),
                renderer_page: renderer,
                page_owner: super::TargetPageResidenceIdentity::new(
                    context.id.clone(),
                    Some(target.target_id().to_owned()),
                    document.id(),
                ),
            },
        )
    }

    pub(crate) async fn wait_for_renderer_document_lifecycle(
        &self,
        renderer_page: super::RendererPageResidenceIdentity,
        event: moli_core::page::RendererDocumentLifecycleEvent,
    ) -> Option<super::DocumentLifecycleEvent> {
        #[cfg(test)]
        for context in self.browser_contexts() {
            for target in context.page_targets.iter() {
                if !context.target_has_loaded_page(target.target_id())
                    && context.routes_renderer_page_for_target(target.target_id(), renderer_page)
                {
                    let snapshot = context
                        .renderer_document_lifecycle_authoritative_snapshot_for_target(
                            target.target_id(),
                        )?;
                    return (snapshot.frame == event.frame
                        && snapshot.document == event.document
                        && snapshot.sequence() >= event.sequence)
                        .then(|| {
                            super::DocumentLifecycleEvent::new(
                                context.target_document_id(target.target_id()).unwrap(),
                                event,
                            )
                        });
                }
            }
        }
        self.browser
            .wait_for_renderer_document_lifecycle(renderer_page, event)
            .await
    }

    pub(crate) fn project_document_lifecycle_events_for_owner(
        &mut self,
        owner: &super::CommandOwnerScope,
        events: Vec<super::DocumentLifecycleEvent>,
    ) -> (
        Option<super::CommittedRendererDocumentBinding>,
        Vec<moli_core::page::RendererDocumentLifecycleEvent>,
    ) {
        let current = self.current_document_id_for_owner(owner);
        let events = events
            .into_iter()
            .filter(|event| Some(event.document()) == current)
            .map(|event| event.event())
            .collect();
        self.project_renderer_document_lifecycle_events_for_owner(owner, events)
    }

    pub(crate) fn apply_renderer_output_stream_control(
        &mut self,
        control: moli_core::RendererOutputStreamControl,
    ) {
        match control {
            moli_core::RendererOutputStreamControl::Opened { stream } => {
                let residence = stream.residence();
                let owners = if self
                    .scheduler_state
                    .renderer_output_ingress
                    .has_owner_reservation(residence)
                {
                    Vec::new()
                } else {
                    crate::domains::activity::renderer_publication_owners(self, residence)
                };
                let owner = match owners.as_slice() {
                    [] => None,
                    [owner] => Some(owner.clone()),
                    _ => {
                        panic!(
                            "renderer output residence {residence:?} must resolve to at most one \
                             protocol owner when its stream opens, got {}",
                            owners.len()
                        )
                    }
                };
                self.scheduler_state
                    .renderer_output_ingress
                    .open(stream, owner);
            }
            moli_core::RendererOutputStreamControl::Closed {
                stream,
                last_published_sequence,
                ..
            } => {
                self.scheduler_state
                    .renderer_output_ingress
                    .close(stream, last_published_sequence);
                if let Some(renderer_page) =
                    super::RendererPageResidenceIdentity::from_residence(stream.residence())
                {
                    self.finish_renderer_page_output_retirement(renderer_page);
                }
            }
        }
    }

    fn finish_renderer_page_output_retirement(
        &mut self,
        renderer_page: super::RendererPageResidenceIdentity,
    ) {
        for browser_context in self
            .browser_context
            .iter_mut()
            .chain(self.inactive_browser_contexts.iter_mut())
        {
            for target in browser_context.page_targets.iter_mut() {
                target
                    .runtime_slot
                    .finish_renderer_page_output_retirement(renderer_page);
            }
        }
    }

    pub(crate) fn bind_renderer_page_output_owner(
        &mut self,
        renderer_page: super::RendererPageResidenceIdentity,
        page_owner: super::TargetPageResidenceIdentity,
    ) {
        let owner = crate::domains::activity::RendererPublicationOwner::PageTarget {
            browser_context_id: page_owner.browser_context_id().to_owned(),
            target_id: page_owner.target_id().map(str::to_owned),
            renderer_page,
            page_owner,
        };
        self.scheduler_state.renderer_output_ingress.bind_owner(
            moli_core::RendererOutputResidenceIdentity::Page {
                owner_local_host_id: renderer_page.owner_local_host_id(),
                page_id: renderer_page.page_id(),
            },
            owner,
        );
    }

    pub(crate) fn release_renderer_page_output_owner_reservation(
        &mut self,
        owner_local_host_id: moli_core::RendererOwnerLocalHostId,
        page_id: moli_core::PageId,
    ) {
        self.scheduler_state
            .renderer_output_ingress
            .release_page_owner_reservation(owner_local_host_id, page_id);
    }

    pub(crate) fn declare_renderer_output_cursor_lease(
        &mut self,
        cursor: moli_core::RendererOutputCursor,
        lease_id: moli_core::RendererOutputFenceLeaseId,
    ) {
        self.scheduler_state
            .renderer_output_ingress
            .declare_cursor_lease(cursor, lease_id);
    }

    pub(crate) fn release_renderer_output_cursor_lease(
        &mut self,
        stream: moli_core::RendererOutputStreamIdentity,
        lease_id: moli_core::RendererOutputFenceLeaseId,
    ) {
        self.scheduler_state
            .renderer_output_ingress
            .release_cursor_lease(stream, lease_id);
    }

    pub(crate) fn admit_renderer_output_publication(
        &mut self,
        publication: moli_core::RendererOutputPublication,
    ) -> crate::domains::activity::RendererOutputIngressAdmission {
        self.scheduler_state
            .renderer_output_ingress
            .admit(publication)
    }

    pub(crate) fn complete_renderer_output_projection(
        &mut self,
        cursor: moli_core::RendererOutputCursor,
    ) -> crate::domains::activity::RendererOutputIngressAdmission {
        let ready = self
            .scheduler_state
            .renderer_output_ingress
            .complete_projection(cursor);
        if ready.is_empty() {
            crate::domains::activity::RendererOutputIngressAdmission::Buffered
        } else {
            crate::domains::activity::RendererOutputIngressAdmission::Ready(ready)
        }
    }

    pub fn renderer_output_cursor_is_projected(
        &self,
        cursor: moli_core::RendererOutputCursor,
    ) -> bool {
        self.scheduler_state
            .renderer_output_ingress
            .is_projection_complete(cursor)
    }

    /// Materializes one already-frozen protocol output work item.
    ///
    /// This path never scans a source for payload or reconstructs a route from
    /// current target state. The work owns both. Completion performs only the
    /// exact attachment authorization required to prevent a detached session
    /// or replacement Page from receiving historical output.
    pub async fn complete_ready_protocol_scheduler_work_turn(
        &mut self,
        work: crate::domains::activity::ProtocolSchedulerWork,
    ) -> CdpTurnOutcome {
        match work.into_ready() {
            ReadyProtocolSchedulerWork::ProtocolObservation(work) => {
                CdpTurnOutcome::new_with_protocol_events(
                    work.into_background_events(self),
                    Vec::new(),
                )
            }
            #[cfg(test)]
            ReadyProtocolSchedulerWork::MainDocumentLoadOwnerAction(completion) => {
                self.complete_deferred_main_document_load_completion_for_scheduler(
                    super::CompletedDeferredMainDocumentLoadCompletion::new(*completion),
                )
                .await
            }
            ReadyProtocolSchedulerWork::BidiChannelOwnerAction(action) => {
                let mut protocol_events = Vec::new();
                self.complete_bidi_channel_owner_action_with_background_events_async(
                    action,
                    &mut protocol_events,
                )
                .await;
                CdpTurnOutcome::new_with_protocol_events(
                    protocol_events,
                    self.take_scheduler_events(),
                )
            }
            ReadyProtocolSchedulerWork::TopLevelLocationNavigationOwnerAction(action) => {
                let (owner_scope, page_owner, navigation) = action.into_parts();
                let mut protocol_events = Vec::new();
                crate::domains::page::navigate_page_owned_top_level_location_background_events_async(
                    self,
                    &mut protocol_events,
                    &owner_scope,
                    &page_owner,
                    navigation,
                )
                .await;
                CdpTurnOutcome::new_with_protocol_events(
                    protocol_events,
                    self.take_scheduler_events(),
                )
            }
            ReadyProtocolSchedulerWork::TargetStartupOwnerAction(action) => {
                crate::domains::target::complete_target_startup_owner_action_async(self, action)
                    .await
            }
            ReadyProtocolSchedulerWork::PageTargetTerminationOwnerAction(action) => {
                crate::domains::page::complete_page_target_termination_owner_action_async(
                    self, action,
                )
                .await
            }
        }
    }

    /// Ingests one concrete renderer publication and returns the protocol
    /// output/scheduler work produced from its exact owner route.
    ///
    /// The publication is consumed once. This facade does not execute Page
    /// work or rescan a source after ingress.
    pub async fn ingest_renderer_output_turn_async(
        &mut self,
        publication: RendererOutputTransportMessage,
        order: &mut RendererCommandResponseOrder,
    ) -> CdpTurnOutcome {
        let mut command_context = super::CommandDispatchContext::default();
        let protocol_events = crate::domains::activity::ingest_renderer_output_transport_async(
            self,
            publication,
            order,
            &mut command_context,
        )
        .await;
        CdpTurnOutcome::new_with_protocol_and_post_response_events(
            protocol_events,
            command_context.take_post_response_events(),
            self.take_scheduler_events(),
        )
    }

    pub async fn project_protocol_local_command_outputs_turn_async(
        &mut self,
        session_id: Option<&str>,
    ) -> CdpTurnOutcome {
        let mut command_context = super::CommandDispatchContext::default();
        let owner = super::CommandOwnerScope::capture(self, session_id);
        crate::domains::activity::project_protocol_local_command_outputs(
            self,
            &owner,
            &mut command_context,
        )
        .await;
        CdpTurnOutcome::new_with_protocol_and_post_response_events(
            command_context.take_protocol_events(),
            command_context.take_post_response_events(),
            self.take_scheduler_events(),
        )
    }

    pub async fn release_renderer_command_response_permit_turn_async(
        &mut self,
        order: &mut RendererCommandResponseOrder,
        permit: RendererCommandResponsePermit,
    ) -> RendererCommandResponseCompletion {
        let mut command_context = super::CommandDispatchContext::default();
        let terminal = order.release(self, permit, &mut command_context).await;
        RendererCommandResponseCompletion::new(
            terminal,
            CdpTurnOutcome::new_with_protocol_and_post_response_events(
                command_context.take_protocol_events(),
                command_context.take_post_response_events(),
                self.take_scheduler_events(),
            ),
        )
    }

    pub async fn cancel_renderer_command_response_permit_turn_async(
        &mut self,
        order: &mut RendererCommandResponseOrder,
        permit: RendererCommandResponsePermit,
    ) -> RendererCommandResponseCompletion {
        let mut command_context = super::CommandDispatchContext::default();
        let terminal = order.cancel(self, permit, &mut command_context).await;
        RendererCommandResponseCompletion::new(
            terminal,
            CdpTurnOutcome::new_with_protocol_and_post_response_events(
                command_context.take_protocol_events(),
                command_context.take_post_response_events(),
                self.take_scheduler_events(),
            ),
        )
    }
}
