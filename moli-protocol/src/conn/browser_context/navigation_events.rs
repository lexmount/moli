use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, CommandDispatchContext, CommandOwnerScope,
    TargetPageResidenceIdentity,
};
use crate::domains::{command_output::CommandOutputBuffer, page};
use moli_core::browser::{NavigationAttempt, NavigationId, WebContentsHandle};

impl CdpConnection {
    /// Reconcile only this physical WebContents. Native completion releases
    /// native observation holds, never a command's independent response fence.
    pub async fn project_browser_navigation(
        &mut self,
        contents: WebContentsHandle,
    ) -> Vec<BackgroundProtocolEvent> {
        // A terminal attempt can retire its projection before a queued response
        // event is consumed. Recover the exact retained response first.
        let mut events = Box::pin(self.project_browser_navigation_responses(contents)).await;
        let Ok(snapshot) = self
            .browser
            .context_handle(contents.context())
            .and_then(|context| context.navigation_snapshot(contents))
        else {
            return events;
        };
        let mut allocator = std::mem::take(&mut self.network_request_id_allocator);
        let prepared = (|| {
            let context = self.browser_context_by_browser_id_mut(contents.context())?;
            let target_id = context
                .page_targets
                .get_for_web_contents(contents.id())?
                .target_id()
                .to_owned();
            let pending = match snapshot.attempt {
                Some(NavigationAttempt::Started(request)) => {
                    if context.observe_target_navigation_started(&target_id, request) {
                        context.project_document_navigation_loader_for_target(
                            &target_id,
                            Some(request.navigation),
                            &mut allocator,
                        );
                    }
                    Some(request.navigation)
                }
                _ => None,
            };
            let retired = context
                .page_targets
                .get(&target_id)?
                .runtime_slot
                .observed_document_navigations()
                .into_iter()
                .filter(|navigation| {
                    Some(*navigation) != pending
                        && Some(*navigation) != snapshot.committed.map(|request| request.navigation)
                })
                .collect::<Vec<_>>();
            let owner = context.target_document_id(&target_id).map(|document| {
                CommandOwnerScope::for_page_residence(&TargetPageResidenceIdentity::new(
                    context.id.clone(),
                    Some(target_id.clone()),
                    document,
                ))
            });
            Some((target_id, owner, retired))
        })();
        self.network_request_id_allocator = allocator;
        let Some((target_id, owner, retired)) = prepared else {
            return events;
        };
        let mut out = CommandOutputBuffer::default();
        let mut command_context = CommandDispatchContext::default();
        let mut releases = Vec::new();
        for navigation in retired {
            out.extend_background_events_after_messages(
                self.native_navigation_retirement_events(contents, navigation),
            );
            let context = self
                .browser_context_by_browser_id_mut(contents.context())
                .expect("resolved Context");
            context.discard_target_navigation_projection(&target_id, &navigation);
            if let Ok(release) = context
                .page_targets
                .get_mut(&target_id)
                .expect("resolved Target")
                .runtime_slot
                .finish_navigation_without_document_projection(&navigation)
            {
                releases.push(release);
            }
        }
        if let Some(owner) = owner {
            for release in releases {
                page::release_document_projection_output_async(
                    self,
                    &mut out,
                    &mut command_context,
                    &owner,
                    release,
                )
                .await;
            }
        }
        out.extend_background_events_after_messages(command_context.take_protocol_events());
        events.extend(out.into_plan().into_background_events(None, None));
        events
    }

    pub(crate) fn native_navigation_retirement_events(
        &mut self,
        contents: WebContentsHandle,
        navigation: NavigationId,
    ) -> Vec<BackgroundProtocolEvent> {
        // Retirement was observed after the earlier response read. Browser may
        // have failed in between: consume its exact terminal response before
        // spending publication cursors on a synthetic supersession result.
        let response = self
            .browser
            .context_handle(contents.context())
            .and_then(|context| context.navigation_responses(contents))
            .ok()
            .and_then(|responses| {
                responses
                    .into_iter()
                    .find(|response| response.request.navigation == navigation)
            });
        let mut events = response
            .map(|response| self.project_native_navigation_network(&response, true))
            .unwrap_or_default();
        let Some((pending, emit_network)) = self
            .browser_context_by_browser_id_mut(contents.context())
            .and_then(|context| {
                let target = context
                    .target_id_for_web_contents(contents.id())?
                    .to_owned();
                context.take_failed_native_navigation(&target, navigation)
            })
        else {
            return events;
        };
        let state = &pending.navigation;
        if emit_network {
            events.extend(crate::domains::network::native_navigation_failure_events(
                self, state,
            ));
        }
        if state.navigate_id.is_some() {
            let mut output = CommandOutputBuffer::default();
            let interrupted = self
                .browser_context_by_browser_id_mut(state.web_contents.context())
                .and_then(|context| {
                    let target = context
                        .page_targets
                        .get_for_web_contents(state.web_contents.id())?
                        .target_id()
                        .to_owned();
                    context.page_targets.get_mut(&target)
                })
                .is_some_and(|target| {
                    target
                        .fetch_owner
                        .retire_navigation_command(pending.navigation_permit.navigation())
                });
            if interrupted {
                output.push_error_after_messages(
                    -32000,
                    "renderer channel navigation was superseded by a newer navigation",
                );
            } else {
                page::push_superseded_navigation_result(&mut output, state);
            }
            events.extend(
                output
                    .into_plan()
                    .into_background_events(state.navigate_id, state.owner.session_id()),
            );
        }
        events
    }
}
