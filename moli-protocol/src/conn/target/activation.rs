use crate::conn::{BackgroundProtocolEvent, CdpConnection, CommandOwnerScope};
use moli_core::browser::{
    BrowserEvent, BrowserEventRecord, BrowserSequence, PendingWebContentsActivation,
    WebContentsHandle,
};

/// Page events stay with the command's completion so they precede its response.
#[derive(Debug)]
pub(crate) struct CompletedTargetActivation {
    protocol_events: Vec<BackgroundProtocolEvent>,
}

impl CompletedTargetActivation {
    pub(crate) fn into_protocol_events(self) -> Vec<BackgroundProtocolEvent> {
        self.protocol_events
    }
}

impl CdpConnection {
    pub fn project_browser_web_contents_activation(
        &mut self,
        record: BrowserEventRecord,
    ) -> Vec<BackgroundProtocolEvent> {
        let BrowserEvent::WebContentsActivated {
            web_contents,
            previous,
        } = record.event
        else {
            return Vec::new();
        };
        self.project_browser_selection(web_contents, previous, record.sequence)
    }

    /// Both command replies and the event subscription observe the same native
    /// occurrence. Snapshot recovery supplies its own observation high-water mark.
    pub(in crate::conn) fn project_browser_selection(
        &mut self,
        selected: WebContentsHandle,
        previous: Option<WebContentsHandle>,
        sequence: BrowserSequence,
    ) -> Vec<BackgroundProtocolEvent> {
        let Some(native_selection) = self
            .browser
            .context_handle(selected.context())
            .ok()
            .and_then(|context| context.selected_web_contents_snapshot())
        else {
            return Vec::new();
        };
        let Some(context) = self.browser_context_by_browser_id_mut(selected.context()) else {
            return Vec::new();
        };
        if context
            .projected_selection
            .is_some_and(|(seen, _)| seen >= sequence)
            || native_selection.web_contents != selected
            || native_selection.sequence > sequence
        {
            return Vec::new();
        }
        let Some(target) = context.page_targets.get_for_web_contents(selected.id()) else {
            return Vec::new();
        };
        let selected_target_id = target.target_id().to_owned();
        let previous = context
            .projected_selection
            .map(|(_, contents)| contents)
            .or_else(|| {
                previous
                    .filter(|handle| handle.context() == selected.context())
                    .map(|handle| handle.id())
            });
        let changed = previous != Some(selected.id());
        let previous_target_id = previous
            .filter(|id| *id != selected.id())
            .and_then(|id| context.page_targets.get_for_web_contents(id))
            .map(|target| target.target_id().to_owned());
        context.projected_selection = Some((sequence, selected.id()));
        if self
            .browser_context
            .as_ref()
            .is_some_and(|context| context.browser_context_id() == selected.context())
        {
            self.refresh_active_browser_context_loader();
        }
        // Recovery advances the observation watermark for every Context, but
        // must not reactivate unchanged peers. An explicit native activation of
        // the already-selected Page still carries this exact selection revision.
        if changed || native_selection.sequence == sequence {
            self.notify_target_host_activated(&selected_target_id);
        }
        let mut events = Vec::new();
        if changed {
            for (target_id, visible) in previous_target_id
                .as_deref()
                .map(|target| (target, false))
                .into_iter()
                .chain(std::iter::once((selected_target_id.as_str(), true)))
            {
                events.extend(
                    self.page_screencast_session_ids_for_target(target_id)
                        .into_iter()
                        .map(|session| {
                            BackgroundProtocolEvent::page_screencast_visibility_changed(
                                session.as_deref(),
                                visible,
                            )
                        }),
                );
            }
        }
        events
    }

    pub(crate) async fn project_target_activation_async(
        &mut self,
        pending: PendingWebContentsActivation,
    ) -> CompletedTargetActivation {
        let protocol_events = match pending.wait().await {
            Ok(event) => self.project_browser_web_contents_activation(event),
            Err(error) => {
                tracing::warn!(%error, "created target activation did not complete");
                Vec::new()
            }
        };
        CompletedTargetActivation { protocol_events }
    }

    pub(crate) async fn select_page_target_for_connection_async(
        &mut self,
        target_id: &str,
    ) -> anyhow::Result<Option<CompletedTargetActivation>> {
        let Some(context) = self.browser_context.as_ref() else {
            anyhow::bail!("BrowserContextNotLoaded");
        };
        let Some(handle) = context.web_contents_handle_for_target(target_id) else {
            return Ok(None);
        };
        let event = self
            .select_browser_web_contents_async(handle)
            .await
            .map_err(anyhow::Error::msg)?;
        Ok(Some(CompletedTargetActivation {
            protocol_events: self.project_browser_web_contents_activation(event),
        }))
    }

    pub(crate) fn page_screencast_session_ids_for_target(
        &mut self,
        target_id: &str,
    ) -> Vec<Option<String>> {
        let Some(route) = self.target_session_route_for_target_id(target_id) else {
            return Vec::new();
        };
        let owner = CommandOwnerScope::for_route(route);
        self.page_event_session_ids_for_owner(&owner)
            .into_iter()
            .filter(|session_id| {
                let event_owner = owner.for_target_event_session(self, session_id.as_deref());
                self.target_page_session_state_for_owner(&event_owner)
                    .is_some_and(|state| state.page_screencast.is_active())
            })
            .collect()
    }
}
