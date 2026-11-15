//! Child classic sources keep transport observations independent of parser work.
//! Ordinary requests retain the shared script cache; intercepted requests use
//! the same per-hop native transport as other Document resources.
use super::*;
use crate::network::{
    context::DocumentResourceLoader,
    interception_output::ResourceInterceptionContext,
    loads::{ResourceLoadDisposition, ResourceLoadKind, ResourceLoadLease},
};
use crate::types::{
    PendingSubresourceFetchInfo, SubresourceRequestInitiatorType, SubresourceRequestStarted,
};

pub(super) struct ChildClassicScriptNetworkLoad {
    request: moli_fetch::Request,
    info: PendingSubresourceFetchInfo,
    context: ResourceInterceptionContext,
    lease: ResourceLoadLease,
    cancel: moli_fetch::FetchCancelHandle,
    intercepted: bool,
}

impl PageScriptHost {
    pub(super) fn prepare_child_classic_script_network_load(
        &mut self,
        loader: &DocumentResourceLoader,
        script: &PreparedScript,
        attribution: &mut ChildClassicScriptNetworkAttribution,
    ) -> Option<ChildClassicScriptNetworkLoad> {
        if !matches!(script.url.scheme(), "http" | "https") {
            return None;
        }
        let context = self.native_interception_context()?;
        let cancel = moli_fetch::FetchCancelHandle::new();
        let lease = loader.register_load(
            ResourceLoadKind::Script,
            ResourceLoadDisposition::Ordinary,
            Some(cancel.clone()),
        )?;
        let mut request = crate::planning::external_script_request(
            script,
            &loader.fetch_context().request_origin(),
            None,
        );
        if let Some(owner) =
            self.owner_dispatch_scope_for_node(DomHandle::new(script.node_id.index()))
            && let Some(policy) = self.script_fetch_redirect_policy_for_owner(owner, script)
        {
            request = request.with_redirect_policy(policy);
        }
        let info = PendingSubresourceFetchInfo {
            internal_id: crate::network::interception_control::next_subresource_request_id(),
            network_request_handle: Some(
                crate::network::interception_control::next_subresource_network_handle(),
            ),
            frame_id: attribution.frame_id.clone(),
            document_url: attribution.document_url.clone(),
            url: script.url.clone(),
            websocket_socket_id: None,
            method: "GET".into(),
            request_headers: request.request_headers.clone(),
            request_body: None,
            request_body_bytes: None,
            resource_type: SubresourceResourceType::Script,
            request_cookie_report: None,
        };
        let intercepted = self.should_intercept_subresource(SubresourceResourceType::Script);
        self.record_subresource_request_started(SubresourceRequestStarted::new(
            info.network_request_handle
                .expect("source request has a network handle"),
            info.frame_id.clone(),
            info.document_url.clone(),
            info.url.clone(),
            info.method.clone(),
            info.request_headers.clone(),
            None,
            SubresourceResourceType::Script,
            SubresourceRequestInitiatorType::Script,
            None,
        ));
        attribution.network_observed = true;
        Some(ChildClassicScriptNetworkLoad {
            request,
            info,
            context,
            lease,
            cancel,
            intercepted,
        })
    }
}

fn failed(message: String) -> crate::planning::PreparedScriptSourceLoadOutcome {
    crate::planning::PreparedScriptSourceLoadOutcome {
        source_result: Err(message.clone()),
        source_bytes: None,
        network_result: Some(std::sync::Arc::new(Err(message))),
        network_request_handle: None,
    }
}

pub(super) async fn load_child_classic_script(
    network: Option<ChildClassicScriptNetworkLoad>,
    script: &PreparedScript,
    loader: DocumentResourceLoader,
    character_set: Option<&str>,
    browser: crate::runtime::RendererBrowserContextRuntime,
    client_id: crate::service_worker_runtime::ServiceWorkerClientId,
    document_url: url::Url,
) -> crate::planning::PreparedScriptSourceLoadOutcome {
    let origin = loader.fetch_context().request_origin();
    let ordinary = || {
        crate::planning::load_service_worker_aware_external_script_source_outcome(
            script,
            &origin,
            loader.request_client(),
            loader.task_runner(),
            character_set,
            None,
            browser.clone(),
            client_id,
            document_url.clone(),
        )
    };
    let Some(network) = network else {
        return ordinary().await;
    };
    if network.intercepted {
        // A controlled child script keeps its existing Service Worker behavior.
        // Only an actual network fallback enters Fetch interception.
        let service_worker = browser.fetch_service_worker_subresource_for_client_with_metadata(
            client_id,
            document_url.clone(),
            &network.request,
            loader.request_client(),
            loader.task_runner(),
            crate::service_worker_runtime::ServiceWorkerRequestDestination::Script,
            SubresourceResourceType::Script,
        );
        let response = tokio::select! {
            biased;
            () = network.cancel.cancelled() => Err(anyhow::anyhow!("net::ERR_ABORTED")),
            response = service_worker => response,
        };
        match response {
            Ok(None) => {}
            response => {
                let outcome = match response {
                    Ok(Some(response)) => {
                        crate::planning::external_script_source_load_outcome_from_response_inner(
                            script,
                            *response.response,
                            character_set,
                            response.response_filter,
                            None,
                        )
                    }
                    Err(error) => failed(error.to_string()),
                    Ok(None) => unreachable!(),
                };
                observe_outcome(&network, &outcome);
                return outcome;
            }
        }
        let completed = crate::network_host::controlled_xhr::ControlledResourceLoad::for_resource(
            network.context,
            network.request,
            network.info,
            network.lease,
            network.cancel,
            SubresourceRequestInitiatorType::Script,
        )
        .with_initial_request_announced()
        .run()
        .await;
        return match completed.result {
            Ok(response) => {
                let response = match completed.network_record.outcome() {
                    crate::types::SubresourceNetworkOutcome::Success { response_body, .. } => {
                        match response_body.materialize_bytes() {
                            Ok(bytes) => crate::protocol_types::NavigationResponse::from_head_and_materialized_body(
                                response.head(), moli_fetch::ResponseBody::materialized_bytes(bytes),
                            ),
                            Err(error) => return failed(error.to_string()),
                        }
                    }
                    _ => response.into(),
                };
                crate::planning::external_script_source_load_outcome_from_response(
                    script,
                    &origin,
                    response,
                    character_set,
                )
            }
            Err(error) => failed(error),
        };
    }
    // The cache owns a shared producer; this cancels only this Document's wait.
    let outcome = tokio::select! {
        biased;
        ()=network.cancel.cancelled()=>failed("net::ERR_ABORTED".into()),
        outcome=ordinary()=>outcome,
    };
    observe_outcome(&network, &outcome);
    outcome
}

fn observe_outcome(
    network: &ChildClassicScriptNetworkLoad,
    outcome: &crate::planning::PreparedScriptSourceLoadOutcome,
) {
    if let Some(result) = outcome.network_result.as_deref() {
        let record = PageScriptHost::get_subresource_network_record_with_initiator(
            network.info.frame_id.clone(),
            network.info.document_url.clone(),
            network.info.url.clone(),
            SubresourceResourceType::Script,
            SubresourceRequestInitiatorType::Script,
            result,
            None,
        )
        .with_request_handle(
            network
                .info
                .network_request_handle
                .expect("script network handle"),
        );
        network
            .context
            .output
            .record_subresource_network(network.context.source_document, record);
    }
}
