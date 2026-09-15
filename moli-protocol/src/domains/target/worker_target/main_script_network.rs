use moli_core::page::{ScriptNetworkOutputItem, SubresourceBodyFinishedResult};

use super::{
    BackgroundProtocolEvent, CdpConnection, CdpSessionRoute, CommandOwnerScope,
    DedicatedWorkerOwner, DevToolsNetworkResourceType, dedicated_worker_loading_error_text,
    monotonic_timestamp_seconds, network,
};

impl CdpConnection {
    /// Main-script notifications span two frontends. Freeze the creator's
    /// request/extra-info delivery at ingress, and retain only the Worker header
    /// and terminal projections for its existing debugger-pause replay contract.
    pub(crate) fn project_dedicated_worker_main_script_network_item(
        &mut self,
        owner: &CommandOwnerScope,
        item: &ScriptNetworkOutputItem,
    ) -> Option<Vec<BackgroundProtocolEvent>> {
        let CdpSessionRoute::DedicatedWorkerTarget {
            browser_context_id,
            target_id,
        } = owner.resolve_route(self)?
        else {
            return None;
        };
        let context = self.browser_context_by_id(&browser_context_id)?;
        let target = context.dedicated_worker_target(&target_id)?;
        let handle = match item {
            ScriptNetworkOutputItem::SubresourceRequestStarted(request)
                if request.is_worker_main_script() =>
            {
                request.handle()
            }
            ScriptNetworkOutputItem::SubresourceResponseStarted(response) => response.handle(),
            ScriptNetworkOutputItem::SubresourceDataReceived(data) => data.handle(),
            ScriptNetworkOutputItem::SubresourceBodyFinished(body) => body.handle(),
            _ => return None,
        };
        if !matches!(item, ScriptNetworkOutputItem::SubresourceRequestStarted(_))
            && target.main_script_request != Some(handle)
        {
            return None;
        }
        let creator = target.owner.target_id(context).map(str::to_owned);
        let creator_is_worker = matches!(target.owner, DedicatedWorkerOwner::Worker(_));
        let target = self
            .browser_context_by_id_mut(&browser_context_id)?
            .dedicated_worker_target_mut(&target_id)?;
        let timestamp = monotonic_timestamp_seconds();
        let mut events = Vec::new();
        match item {
            ScriptNetworkOutputItem::SubresourceRequestStarted(request) => {
                if target.main_script_request.is_some() {
                    return Some(events);
                }
                target.main_script_request = Some(handle);
                let Some(creator) = creator else {
                    return Some(events);
                };
                for session in &target.owner_network_sessions {
                    network::emit_request_will_be_sent(
                        &mut events,
                        session.as_deref(),
                        &target_id,
                        &creator,
                        &creator,
                        timestamp,
                        request.document_url(),
                        request.url(),
                        request.method(),
                        request.request_body(),
                        &request.request_headers().to_byte_strings(),
                        DevToolsNetworkResourceType::Script,
                        request.request_initiator_type(),
                        None,
                        false,
                        None,
                        &[],
                    );
                }
                if creator_is_worker {
                    for event in &mut events {
                        event.bind_network_to_worker_target(&creator);
                    }
                }
            }
            ScriptNetworkOutputItem::SubresourceResponseStarted(response) => {
                if target.main_script_response_event.is_some()
                    || target.main_script_terminal_event.is_some()
                {
                    return Some(events);
                }
                let has_extra = response.network_request_headers().is_some();
                if let Some(headers) = response.network_request_headers() {
                    let empty = moli_cookie_jar::StoredCookieQueryReport::default();
                    for session in &target.owner_network_sessions {
                        network::emit_request_will_be_sent_extra_info(
                            &mut events,
                            session.as_deref(),
                            &target_id,
                            headers,
                            response.request_cookie_report().unwrap_or(&empty),
                            timestamp,
                        );
                        network::emit_response_received_extra_info(
                            &mut events,
                            session.as_deref(),
                            &target_id,
                            response.response_headers(),
                            response.status(),
                            response.cookie_set_reports(),
                        );
                    }
                }
                let mut worker_events = Vec::new();
                network::emit_response_received_without_extra_info_event(
                    &mut worker_events,
                    None,
                    &target_id,
                    &target_id,
                    &target_id,
                    timestamp,
                    response.final_url(),
                    response.status(),
                    response.status_text(),
                    response.response_headers(),
                    0,
                    response.from_cache(),
                    response.negotiated_http_version(),
                    has_extra,
                    DevToolsNetworkResourceType::Script,
                );
                target.main_script_response_event = worker_events.pop();
            }
            ScriptNetworkOutputItem::SubresourceBodyFinished(body) => {
                if target.main_script_terminal_event.is_some() {
                    return Some(events);
                }
                let mut worker_events = Vec::new();
                match body.result() {
                    SubresourceBodyFinishedResult::Ready(body) => network::emit_loading_finished(
                        &mut worker_events,
                        None,
                        &target_id,
                        &target_id,
                        &target_id,
                        timestamp,
                        body.len(),
                        DevToolsNetworkResourceType::Script,
                    ),
                    SubresourceBodyFinishedResult::Failed(error_text)
                    | SubresourceBodyFinishedResult::FailedWithPartialBody { error_text, .. } => {
                        network::emit_loading_failed(
                            &mut worker_events,
                            None,
                            &target_id,
                            &target_id,
                            &target_id,
                            timestamp,
                            dedicated_worker_loading_error_text(error_text),
                            DevToolsNetworkResourceType::Script,
                        )
                    }
                }
                target.main_script_terminal_event = worker_events.pop();
            }
            // No Worker frontend exists while its main body is streaming. A late
            // paused attachment receives the header/terminal, not chunk history.
            ScriptNetworkOutputItem::SubresourceDataReceived(_) => {}
            _ => unreachable!(),
        }
        Some(events)
    }
}
