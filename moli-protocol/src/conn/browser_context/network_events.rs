use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, CommandOwnerScope, TargetPageResidenceIdentity,
};

impl CdpConnection {
    pub(in crate::conn) fn project_browser_network_snapshot(
        &mut self,
        requests: Vec<moli_core::browser::NetworkRequestSnapshot>,
    ) -> Vec<BackgroundProtocolEvent> {
        let mut events = Vec::new();
        for request in requests {
            let Some(context) =
                self.browser_context_by_browser_id(request.document.web_contents().context())
            else {
                continue;
            };
            let Some(target) =
                context.target_id_for_web_contents(request.document.web_contents().id())
            else {
                continue;
            };
            // An old physical Document cannot donate its recovery records to a
            // replacement, even when that replacement has the same Target id.
            if context.document_handle_for_target(target) != Some(request.document) {
                continue;
            }
            let owner = CommandOwnerScope::for_page_residence(&TargetPageResidenceIdentity::new(
                context.id.clone(),
                Some(target.to_owned()),
                request.document.id(),
            ));
            for item in request.output_items() {
                if let Some(mut delivery) = self.project_network_output_item_for_owner(
                    &owner,
                    None,
                    request.renderer_document,
                    &item,
                ) {
                    crate::domains::network::emit_prepared_renderer_network_live_background_events(
                        self,
                        &mut events,
                        &owner,
                        &mut delivery,
                    );
                }
            }
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_core::browser::{
        BrowserContextStoragePartitionHandles, BrowserEvent, BrowserService,
        NavigationRequestLoadPolicy, StoragePartitionKind,
    };

    #[tokio::test]
    async fn native_network_snapshot_recovers_without_devtools_and_deduplicates_its_late_fifo() {
        let service = BrowserService::start().unwrap();
        let browser = service.handle();
        let context = browser
            .create_context(
                BrowserContextStoragePartitionHandles::memory(),
                StoragePartitionKind::Ephemeral,
                None,
                None,
            )
            .unwrap();
        context.bind_page_navigation_engines(Default::default(), None);
        let (sender, mut output) = moli_core::renderer_output_transport_channel();
        context
            .set_renderer_output_transport_sender(sender)
            .unwrap();
        let (contents, _) = context.create_web_contents(Default::default()).unwrap();
        let (_, mut events) = browser.subscribe().unwrap();
        let navigation = context.navigate_document(contents,
            moli_core::browser::web_contents::NavigationRequestInterception::new(
                "data:text/html,<script>fetch('data:text/plain,native-recovery').then(r=>r.text())</script>".parse().unwrap(),
                "GET".into(), None, Vec::new(), NavigationRequestLoadPolicy::BrowserInitiated)).unwrap();
        let moli_core::browser::BrowserNavigationOutcome::Document(commit) =
            navigation.wait().await.unwrap()
        else {
            panic!("native Document must commit");
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let BrowserEvent::NetworkRequestCompleted(occurrence) =
                    events.recv().await.unwrap().event
                    && occurrence.document == commit.document
                {
                    break;
                }
            }
        })
        .await
        .expect("the native resource must complete before a DevTools connection exists");
        let committed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let moli_core::RendererOutputTransportMessage::Publication(publication) = output.recv().await.unwrap() else { continue; };
                for record in publication.into_records() {
                    if let moli_core::RendererOutputItem::Observation(moli_core::RendererProtocolObservation::Network(observation)) = record.into_parts().1 {
                        let committed = observation.committed().await.unwrap();
                        if matches!(committed.occurrence().item.as_ref(), moli_core::page::ScriptNetworkOutputItem::SubresourceNetworkRecord(record) if record.url().as_str() == "data:text/plain,native-recovery") {
                            return committed;
                        }
                    }
                }
            }
        }).await.expect("the exact source FIFO retains its native receipt");
        let (peer, _) = context.create_web_contents(Default::default()).unwrap();
        let peer_navigation = context
            .navigate_document(
                peer,
                moli_core::browser::web_contents::NavigationRequestInterception::new(
                    "data:text/html,peer".parse().unwrap(),
                    "GET".into(),
                    None,
                    Vec::new(),
                    NavigationRequestLoadPolicy::BrowserInitiated,
                ),
            )
            .unwrap();
        let moli_core::browser::BrowserNavigationOutcome::Document(peer_commit) =
            peer_navigation.wait().await.unwrap()
        else {
            panic!("peer Document must commit");
        };
        let peer_renderer = context
            .document_renderer_residence(peer_commit.document)
            .unwrap();
        assert_eq!(
            peer_renderer.page_id(),
            committed.occurrence().document.document.page_id
        );
        assert_ne!(
            peer_renderer.owner_local_host_id(),
            committed.occurrence().owner_local_host_id
        );
        let mut snapshot = browser.subscribe().unwrap().0;
        let requests = std::mem::take(&mut snapshot.network_requests);
        assert_eq!(requests.len(), 1);
        let mut conn = CdpConnection::new(
            browser.clone(),
            crate::CdpInitialStoragePartition::memory(),
            Default::default(),
        );
        // First recover physical membership, then exercise Network recovery
        // while enabled, with the original source FIFO deliberately delayed.
        conn.project_browser_snapshot(snapshot).await;
        let projection = conn.browser_context_by_browser_id(context.id()).unwrap();
        let peer_target = projection.target_id_for_web_contents(peer.id()).unwrap();
        let peer_owner = CommandOwnerScope::for_page_residence(&TargetPageResidenceIdentity::new(
            projection.id.clone(),
            Some(peer_target.to_owned()),
            peer_commit.document.id(),
        ));
        assert!(
            conn.ingest_browser_network_observation_for_owner(
                &peer_owner,
                Some(peer_renderer),
                &committed
            )
            .is_none(),
            "a receipt from another renderer owner cannot be rebound through colliding local Page ids"
        );
        let projection = conn.browser_context_by_browser_id(context.id()).unwrap();
        let target = projection
            .target_id_for_web_contents(contents.id())
            .unwrap()
            .to_owned();
        let owner = CommandOwnerScope::for_page_residence(&TargetPageResidenceIdentity::new(
            projection.id.clone(),
            Some(target.clone()),
            commit.document.id(),
        ));
        conn.browser_context_by_browser_id_mut(context.id())
            .unwrap()
            .page_targets
            .get_mut(&target)
            .unwrap()
            .runtime_slot
            .enable_primary_network_events();
        let recovered = conn.project_browser_network_snapshot(requests.clone());
        assert_eq!(
            recovered
                .iter()
                .filter(|event| event.protocol_method() == Some("Network.requestWillBeSent"))
                .count(),
            1
        );
        assert_eq!(
            recovered
                .iter()
                .filter(|event| event.protocol_method() == Some("Network.loadingFinished"))
                .count(),
            1
        );
        assert!(
            conn.project_browser_network_snapshot(requests.clone())
                .is_empty()
        );
        let renderer = crate::conn::RendererPageResidenceIdentity::from_parts(
            committed.occurrence().owner_local_host_id,
            committed.occurrence().document.document.page_id,
        );
        assert!(
            conn.ingest_browser_network_observation_for_owner(&owner, Some(renderer), &committed)
                .is_none()
        );
        conn.install_navigation_fixture_for_owner_for_test("data:text/html,replacement", &owner)
            .await;
        assert_ne!(
            context.document_handle(contents).unwrap(),
            Some(commit.document)
        );
        assert!(conn.project_browser_network_snapshot(requests).is_empty());
        assert!(
            conn.ingest_browser_network_observation_for_owner(&owner, Some(renderer), &committed)
                .is_none()
        );
        service.shutdown();
    }
}
