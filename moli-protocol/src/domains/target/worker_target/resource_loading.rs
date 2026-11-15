use super::*;
use moli_core::network::ResourceInterceptionEvent;

pub(crate) fn prepare(
    conn: &mut CdpConnection,
    browser_context_id: String,
    target_id: String,
    event: ResourceInterceptionEvent,
) -> Option<(
    Vec<BackgroundProtocolEvent>,
    Vec<crate::domains::fetch::PreparedSubresourceAutoContinue>,
    Vec<crate::domains::activity::PreparedSubresourceContinueWait>,
)> {
    conn.browser_context_by_id(&browser_context_id)?
        .worker_resource_target(&target_id)?;
    let route = if conn
        .browser_context_by_id(&browser_context_id)?
        .shared_worker_target(&target_id)
        .is_some()
    {
        CdpSessionRoute::SharedWorkerTarget {
            browser_context_id: browser_context_id.clone(),
            target_id: target_id.clone(),
        }
    } else {
        CdpSessionRoute::DedicatedWorkerTarget {
            browser_context_id: browser_context_id.clone(),
            target_id: target_id.clone(),
        }
    };
    let owner = CommandOwnerScope::for_route(route);
    let outputs = match event {
        ResourceInterceptionEvent::Network(item) => {
            let delivery =
                conn.ingest_worker_network_output(&browser_context_id, &target_id, &item)?;
            network::NetworkPreparedOutputs::from_renderer_live_delivery(delivery)
        }
        ResourceInterceptionEvent::Request { info, pause } => {
            crate::domains::fetch::worker_fetch_pause_prepared_outputs(
                conn,
                &owner,
                browser_context_id,
                target_id,
                *info,
                pause,
            )
        }
        ResourceInterceptionEvent::Continue { event, pause } => {
            let action = crate::domains::activity::prepare_worker_subresource_continue_action(
                conn, &owner, *event, pause,
            );
            network::NetworkPreparedOutputs::from_worker_continue(action)
        }
    };
    Some(outputs.project_worker(conn, &owner))
}
