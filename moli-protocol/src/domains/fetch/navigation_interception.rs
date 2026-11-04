use std::collections::HashMap;

use crate::automation::DevToolsNetworkResourceType;
use crate::conn::{
    CdpConnection, CommandOwnerScope, PendingFetchNavigation, PendingSubresourceFetchOwnerKind,
    PendingSubresourceFetchRequestStage, TargetFetchSubresourceInterceptionSnapshot,
};

type NavigationHopIds = HashMap<Option<String>, (String, Option<String>)>;

/// Every Fetch handler advances its own hop identity, including unmatched
/// hops. The transport's navigation identity is independent of these IDs.
pub(super) fn navigation_hop_ids(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    snapshot: &TargetFetchSubresourceInterceptionSnapshot,
    network_id: &str,
    first_id: &str,
    redirected: bool,
) -> Option<NavigationHopIds> {
    let mut ids = HashMap::new();
    for (index, session_id) in snapshot
        .fetch_session_ids(owner.session_id())
        .into_iter()
        .enumerate()
    {
        let id = if index == 0 {
            first_id.to_owned()
        } else {
            conn.allocate_pending_subresource_fetch_request_ids_for_owner(owner)?
                .0
        };
        let previous = conn
            .runtime_session_owner_slot_mut_for_owner(owner)
            .and_then(|slot| {
                slot.replace_navigation_fetch_request_id(
                    network_id.to_owned(),
                    session_id.clone(),
                    id.clone(),
                )
            });
        ids.insert(session_id, (id, previous.filter(|_| redirected)));
    }
    Some(ids)
}

pub(crate) fn prepare_navigation_request_pause(
    conn: &mut CdpConnection,
    pending: &mut PendingFetchNavigation,
) -> bool {
    let owner = &pending.navigation.owner;
    let Some(snapshot) = conn.target_fetch_subresource_interception_snapshot_for_owner(owner)
    else {
        return false;
    };
    let redirected = !pending.navigation.redirect_chain.is_empty();
    let Some(ids) = navigation_hop_ids(
        conn,
        owner,
        &snapshot,
        &pending.navigation.loader_id,
        &pending.fetch_request_id,
        redirected,
    ) else {
        return false;
    };
    let blocked = snapshot.matching_network_intercepts(
        crate::conn::FetchRequestStage::Request,
        DevToolsNetworkResourceType::Document,
        &pending.navigation.requested_url,
    );
    let mut stages = Vec::new();
    for stage in snapshot.matching_request_stage_pause_sessions(
        owner.session_id(),
        DevToolsNetworkResourceType::Document,
        &pending.navigation.requested_url,
    ) {
        let (id, previous) = if stage.owner_kind == PendingSubresourceFetchOwnerKind::Fetch {
            ids.get(&stage.session_id)
                .cloned()
                .unwrap_or_else(|| (pending.fetch_request_id.clone(), None))
        } else {
            let Some((id, _)) =
                conn.allocate_pending_subresource_fetch_request_ids_for_owner(owner)
            else {
                return false;
            };
            (id, None)
        };
        stages.push(PendingSubresourceFetchRequestStage {
            session_id: stage.session_id,
            owner_kind: stage.owner_kind,
            request_id: id,
            redirected_request_id: previous,
            blocked_intercepts: blocked.clone(),
        });
    }
    if stages.is_empty() {
        return false;
    }
    let stage = stages.remove(0);
    pending.fetch_request_id = stage.request_id;
    pending.interception_session_id = stage.session_id;
    pending.redirected_request_id = stage.redirected_request_id;
    pending.remaining_request_stages = stages;
    true
}
