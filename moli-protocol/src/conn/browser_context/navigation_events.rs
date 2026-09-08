use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, CommandDispatchContext, CommandOwnerScope,
    TargetPageResidenceIdentity,
};
use crate::domains::{command_output::CommandOutputBuffer, page};
use moli_core::browser::{NavigationAttempt, WebContentsHandle};

impl CdpConnection {
    /// Reconcile only this physical WebContents. Native completion releases
    /// native observation holds, never a command's independent response fence.
    pub async fn project_browser_navigation(
        &mut self,
        contents: WebContentsHandle,
    ) -> Vec<BackgroundProtocolEvent> {
        let Ok(snapshot) = self
            .browser
            .context_handle(contents.context())
            .and_then(|context| context.navigation_snapshot(contents))
        else {
            return Vec::new();
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
            let mut releases = Vec::new();
            for navigation in retired {
                context.discard_target_navigation_projection(&target_id, &navigation);
                if let Ok(release) = context
                    .page_targets
                    .get_mut(&target_id)?
                    .runtime_slot
                    .finish_navigation_without_document_projection(&navigation)
                {
                    releases.push(release);
                }
            }
            Some((owner, releases))
        })();
        self.network_request_id_allocator = allocator;
        let Some((Some(owner), releases)) = prepared else {
            return Vec::new();
        };
        let mut out = CommandOutputBuffer::default();
        let mut command_context = CommandDispatchContext::default();
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
        out.extend_background_events_after_messages(command_context.take_protocol_events());
        out.into_plan().into_background_events(None, None)
    }
}
