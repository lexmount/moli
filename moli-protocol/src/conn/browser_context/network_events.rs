use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, CommandOwnerScope, TargetPageResidenceIdentity,
};

impl CdpConnection {
    pub(crate) fn observes_worker_fetch_pause(
        &self,
        pause: &moli_core::browser::WorkerFetchPause,
    ) -> bool {
        self.browser_context_by_browser_id(pause.document.web_contents().context())
            .is_some_and(|context| {
                context
                    .page_targets
                    .iter()
                    .any(|target| target.fetch_owner.observes_worker_pause(&pause.pause))
            })
    }

    pub(crate) fn retire_completed_worker_fetch(
        &mut self,
        occurrence: &moli_core::page::RendererNetworkOccurrence,
    ) {
        use moli_core::page::{RendererNetworkOutputItem, RendererNetworkSource};
        let RendererNetworkSource::Worker(worker) = &occurrence.source else {
            return;
        };
        let RendererNetworkOutputItem::Resource(item) = &occurrence.item else {
            return;
        };
        if let Some(context) = self
            .browser_context
            .iter_mut()
            .chain(&mut self.inactive_browser_contexts)
            .find(|context| context.routes_renderer_browser_context_runtime(occurrence.runtime))
        {
            retire_worker_fetch_from_network(context, worker, item);
        }
    }

    pub(crate) fn committed_worker_fetch_pause(
        &self,
        occurrence: &moli_core::page::RendererNetworkOccurrence,
    ) -> Option<moli_core::browser::WorkerFetchPause> {
        let moli_core::page::RendererNetworkOutputItem::WorkerFetch { pause, .. } =
            &occurrence.item
        else {
            return None;
        };
        let context = self
            .browser_contexts()
            .find(|context| context.routes_renderer_browser_context_runtime(occurrence.runtime))?;
        self.browser
            .context_handle(context.browser_context_id())
            .ok()?
            .worker_fetch_pause(pause.clone())
    }

    pub(crate) fn worker_fetch_observer(
        &self,
        pause: &moli_core::browser::WorkerFetchPause,
    ) -> Option<CommandOwnerScope> {
        let document = pause.document;
        let context = self.browser_context_by_browser_id(document.web_contents().context())?;
        let target = context.target_id_for_web_contents(document.web_contents().id())?;
        if context.document_handle_for_target(target) != Some(document) {
            return None;
        }
        Some(CommandOwnerScope::for_page_residence(
            &TargetPageResidenceIdentity::new(
                context.id.clone(),
                Some(target.to_owned()),
                document.id(),
            ),
        ))
    }

    /// Freeze the enabled listeners and their events synchronously at source
    /// ingress. The existing Worker attachment/run outputs own later delivery.
    pub(crate) fn project_native_worker_network_item(
        &mut self,
        owner: &CommandOwnerScope,
        item: &moli_core::page::ScriptNetworkOutputItem,
    ) -> Vec<BackgroundProtocolEvent> {
        let Some((_, Some(target_id))) = self.network_owner_identity_for_owner(owner) else {
            return Vec::new();
        };
        let mut allocator = std::mem::take(&mut self.network_request_id_allocator);
        let delivery = self.network_agent_for_owner_mut(owner).map(|agent| {
            agent.ingest_renderer_output_item_and_prepare_live_delivery(
                item,
                "",
                None,
                None,
                None,
                &mut allocator,
            )
        });
        self.network_request_id_allocator = allocator;
        let Some(mut delivery) = delivery else {
            return Vec::new();
        };
        let mut events = Vec::new();
        crate::domains::network::emit_prepared_renderer_network_live_background_events(
            self,
            &mut events,
            owner,
            &mut delivery,
        );
        for event in &mut events {
            event.bind_network_to_worker_target(&target_id);
        }
        events
    }
    pub(in crate::conn) fn project_browser_network_snapshot(
        &mut self,
        requests: Vec<moli_core::browser::NetworkRequestSnapshot>,
    ) -> Vec<BackgroundProtocolEvent> {
        // Browser broadcast loss does not lose the renderer FIFO. Even a late
        // Document binding replays the journal's frozen publications. Restore
        // request state only for a snapshot-only observer: a second live path
        // would bypass source order, Document and command-response fences and
        // consume the phase dedupe before the original publication arrives.
        if self.scheduler_hooks.renderer_publication_sender().is_some() {
            return Vec::new();
        }
        let mut events = Vec::new();
        for request in requests {
            if let moli_core::page::RendererNetworkSource::Worker(source) = &request.renderer_source
            {
                let Some(context_id) = self
                    .browser_context_by_browser_id(request.owner.context())
                    .map(|context| context.id.clone())
                else {
                    continue;
                };
                let owner = self.native_worker_network_owner(&context_id, source);
                for item in request.output_items() {
                    if let moli_core::page::RendererNetworkOutputItem::Resource(item) = item {
                        if let Some(context) =
                            self.browser_context_by_browser_id_mut(request.owner.context())
                        {
                            retire_worker_fetch_from_network(context, source, &item);
                        }
                        if let Some(owner) = &owner {
                            events.extend(self.project_native_worker_network_item(owner, &item));
                        }
                    }
                }
                continue;
            }
            let moli_core::browser::NetworkOwner::Document(document) = request.owner else {
                continue;
            };
            let Some((_, renderer_document)) = request.renderer_source.document() else {
                continue;
            };
            let Some(context) =
                self.browser_context_by_browser_id(document.web_contents().context())
            else {
                continue;
            };
            let Some(target) = context.target_id_for_web_contents(document.web_contents().id())
            else {
                continue;
            };
            // An old physical Document cannot donate its recovery records to a
            // replacement, even when that replacement has the same Target id.
            if context.document_handle_for_target(target) != Some(document) {
                continue;
            }
            let owner = CommandOwnerScope::for_page_residence(&TargetPageResidenceIdentity::new(
                context.id.clone(),
                Some(target.to_owned()),
                document.id(),
            ));
            for item in request.output_items() {
                match item {
                    moli_core::page::RendererNetworkOutputItem::WorkerFetch { .. } => unreachable!(
                        "pause snapshots are independent from Network request snapshots"
                    ),
                    moli_core::page::RendererNetworkOutputItem::ChildDocument(response) => {
                        let Some(binding) = self
                            .target_root_document_protocol_attachment_identity_for_owner(
                                &owner,
                                renderer_document,
                            )
                        else {
                            continue;
                        };
                        let mut recovered = Vec::new();
                        crate::domains::network::emit_child_document_navigation_network_background_events(
                            self, &mut recovered, &owner, &response.frame_id, &response.loader_id,
                            &response.loader_id, crate::conn::monotonic_timestamp_seconds(), &response.snapshot,
                        );
                        events.extend(recovered.into_iter().filter_map(|event| {
                            event.bind_to_root_document_route(self, &owner, binding.root_document())
                        }));
                    }
                    moli_core::page::RendererNetworkOutputItem::Resource(item) => {
                        if let Some(mut delivery) = self.project_network_output_item_for_owner(
                            &owner,
                            None,
                            renderer_document,
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
            }
        }
        events
    }
}

fn retire_worker_fetch_from_network(
    context: &mut crate::conn::BrowserContext,
    worker: &moli_core::page::RendererWorkerIdentity,
    item: &moli_core::page::ScriptNetworkOutputItem,
) {
    use moli_core::page::ScriptNetworkOutputItem;
    let handle = match item {
        ScriptNetworkOutputItem::SubresourceNetworkRecord(record) => record.request_handle(),
        ScriptNetworkOutputItem::SubresourceBodyFinished(body) => Some(body.handle()),
        _ => None,
    };
    if let Some(handle) = handle {
        for target in context.page_targets.iter_mut() {
            target
                .fetch_owner
                .retire_worker_requests(worker, Some(handle));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_core::browser::{
        BrowserContextStoragePartitionHandles, BrowserEvent, BrowserService,
        NavigationRequestLoadPolicy, StoragePartitionKind,
    };

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum PauseStage {
        Request,
        Auth,
        Response,
    }

    #[tokio::test]
    async fn native_worker_request_snapshot_recovers_once_and_terminal_cleanup_keeps_document_request()
     {
        recover_worker_pause_snapshot(PauseStage::Request).await;
    }

    #[tokio::test]
    async fn native_worker_auth_snapshot_recovers_without_request_stage_projection() {
        recover_worker_pause_snapshot(PauseStage::Auth).await;
    }

    #[tokio::test]
    async fn native_worker_response_snapshot_recovers_without_request_stage_projection() {
        recover_worker_pause_snapshot(PauseStage::Response).await;
    }

    async fn recover_worker_pause_snapshot(stage: PauseStage) {
        use axum::{Router, routing::get};
        use moli_core::page::{RendererWorkerFetchStage, WorkerFetchDecision};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let url = format!(
            "http://{address}/{}",
            if stage == PauseStage::Auth {
                "auth"
            } else {
                "body"
            }
        );
        let html = format!(
            "<!doctype html><script>const worker=new Worker('/worker.js');worker.onmessage=e=>{{if(e.data==='ready')worker.postMessage({url:?});}};</script>"
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, Router::new()
                .route("/page", get(move || { let html = html.clone(); async move { ([("content-type", "text/html")], html) } }))
                .route("/worker.js", get(|| async { ([("content-type", "text/javascript")], "onmessage=async e=>{const r=await fetch(e.data);postMessage(await r.text());};postMessage('ready');") }))
                .route("/body", get(|| async { "snapshot-body" }))
                .route("/auth", get(|| async { (axum::http::StatusCode::UNAUTHORIZED, [("www-authenticate", "Basic realm=\"snapshot\"")], "snapshot-auth") }))
            ).await.unwrap();
        });
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
        context
            .install_web_contents_fetch_interception_policy(
                contents,
                true,
                Some(moli_core::page::SubresourceResourceType::Fetch),
            )
            .unwrap();
        let (_, mut native) = browser.subscribe().unwrap();
        let navigation = context
            .navigate_document(
                contents,
                moli_core::browser::web_contents::NavigationRequestInterception::new(
                    format!("http://{address}/page").parse().unwrap(),
                    "GET".into(),
                    None,
                    Vec::new(),
                    NavigationRequestLoadPolicy::BrowserInitiated,
                ),
            )
            .unwrap();
        let moli_core::browser::BrowserNavigationOutcome::Document(commit) =
            navigation.wait().await.unwrap()
        else {
            panic!("Document must commit");
        };
        // Inline script runs with the Context resident in its native owner;
        // the test-only arbitrary evaluator temporarily removes that owner.
        let pause = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let BrowserEvent::WorkerFetchPaused(pause) = native.recv().await.unwrap().event {
                    if matches!(pause.pause.stage(), RendererWorkerFetchStage::Request(_))
                        && stage != PauseStage::Request
                    {
                        context
                            .start_worker_fetch_decision(
                                pause,
                                WorkerFetchDecision::ContinueRequest {
                                    url: None,
                                    method: None,
                                    body: None,
                                    headers: None,
                                    intercept_response: stage == PauseStage::Response,
                                    handle_auth_requests: stage == PauseStage::Auth,
                                },
                            )
                            .unwrap()
                            .wait()
                            .await
                            .unwrap();
                    } else {
                        break pause;
                    }
                }
            }
        })
        .await
        .expect("native stage must pause without Protocol");
        assert!(matches!(
            (stage, pause.pause.stage()),
            (PauseStage::Request, RendererWorkerFetchStage::Request(_))
                | (PauseStage::Auth, RendererWorkerFetchStage::Auth(_))
                | (PauseStage::Response, RendererWorkerFetchStage::Response(_))
        ));
        let late_fifo = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let moli_core::RendererOutputTransportMessage::Publication(publication) = output.recv().await.unwrap() else { continue; };
                for record in publication.into_records() {
                    if let moli_core::RendererOutputItem::Observation(moli_core::RendererProtocolObservation::Network(observation)) = record.into_parts().1
                        && let Some(committed) = observation.committed().await
                        && matches!(&committed.occurrence().item, moli_core::page::RendererNetworkOutputItem::WorkerFetch { pause: observed, .. } if observed == &pause.pause) {
                        return committed;
                    }
                }
            }
        }).await.expect("pause must retain its original FIFO receipt");
        let mut snapshot = browser.subscribe().unwrap().0;
        let pauses = std::mem::take(&mut snapshot.worker_fetch_pauses);
        assert_eq!(pauses, vec![pause.clone()]);
        let mut conn = CdpConnection::new(
            browser.clone(),
            crate::CdpInitialStoragePartition::memory(),
            Default::default(),
        );
        conn.project_browser_snapshot(snapshot).await;
        let projection = conn.browser_context_by_browser_id(context.id()).unwrap();
        let target = projection
            .target_id_for_web_contents(contents.id())
            .unwrap()
            .to_owned();
        let observer = TargetPageResidenceIdentity::new(
            projection.id.clone(),
            Some(target.clone()),
            commit.document.id(),
        );
        conn.browser_context_by_browser_id_mut(context.id())
            .unwrap()
            .page_targets
            .get_mut(&target)
            .unwrap()
            .fetch_owner
            .configure(
                None,
                true,
                vec![crate::conn::FetchInterceptionPattern {
                    url_pattern: "*".into(),
                    resource_type_filter: None,
                    request_stage: if stage == PauseStage::Request {
                        crate::conn::FetchRequestStage::Request
                    } else {
                        crate::conn::FetchRequestStage::Response
                    },
                }],
            );
        let recovered = conn
            .project_browser_snapshot(browser.subscribe().unwrap().0)
            .await;
        let method = if stage == PauseStage::Auth {
            "Fetch.authRequired"
        } else {
            "Fetch.requestPaused"
        };
        assert_eq!(
            recovered
                .iter()
                .filter(|event| event.protocol_method() == Some(method))
                .count(),
            1
        );
        assert!(pause.pause.is_available());
        assert!(conn.observes_worker_fetch_pause(&pause));
        assert!(
            conn.project_browser_snapshot(browser.subscribe().unwrap().0)
                .await
                .iter()
                .all(|event| event.protocol_method() != Some(method))
        );
        let late_pause = conn
            .committed_worker_fetch_pause(late_fifo.occurrence())
            .unwrap();
        assert!(
            crate::domains::fetch::native_worker_fetch_prepared_outputs(&mut conn, late_pause)
                .await
                .is_none(),
            "late source FIFO must not publish a recovered stage twice"
        );
        if stage == PauseStage::Request {
            let owner = &mut conn
                .browser_context_by_browser_id_mut(context.id())
                .unwrap()
                .page_targets
                .get_mut(&target)
                .unwrap()
                .fetch_owner;
            let wire_id = recovered
                .iter()
                .find(|event| event.protocol_method() == Some(method))
                .unwrap()
                .clone()
                .into_parts()
                .0["params"]["requestId"]
                .as_str()
                .unwrap()
                .to_owned();
            let worker = owner
                .take_pending_subresource_fetch_request(&wire_id, None)
                .unwrap();
            let mut document = worker.clone();
            document.residence =
                crate::conn::PendingSubresourceFetchResidence::InstalledPage(observer);
            document.network_request_handle = None;
            document.network_request_id = "document-collision".into();
            owner.register_in_flight_subresource_fetch_request(Some(wire_id), worker);
            owner.register_in_flight_subresource_fetch_request(
                Some("document-collision".into()),
                document,
            );
        }
        context
            .start_worker_fetch_decision(pause.clone(), WorkerFetchDecision::Release)
            .unwrap()
            .wait()
            .await
            .unwrap();
        let terminal = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let BrowserEvent::NetworkRequestCompleted(occurrence) = native.recv().await.unwrap().event
                    && matches!(&occurrence.renderer.item, moli_core::page::RendererNetworkOutputItem::Resource(item) if matches!(item.as_ref(), moli_core::page::ScriptNetworkOutputItem::SubresourceNetworkRecord(record) if record.request_handle() == Some(pause.pause.handle()))) { break occurrence; }
            }
        }).await.expect("released native request must finish");
        // A snapshot-only observer has no live FIFO consumer to retire this
        // stage. Native completion must clean it up even without a Worker
        // attachment or Network listener; a late FIFO remains idempotent.
        conn.project_browser_snapshot(browser.subscribe().unwrap().0)
            .await;
        assert!(!conn.observes_worker_fetch_pause(&pause));
        conn.retire_completed_worker_fetch(&terminal.renderer);
        let owner = &mut conn
            .browser_context_by_browser_id_mut(context.id())
            .unwrap()
            .page_targets
            .get_mut(&target)
            .unwrap()
            .fetch_owner;
        if stage == PauseStage::Request {
            assert!(
                owner
                    .take_in_flight_subresource_fetch_request(
                        crate::conn::SubresourceFetchKey::Worker(pause.pause.handle())
                    )
                    .is_none()
            );
            assert_eq!(
                owner
                    .take_in_flight_subresource_fetch_request(pause.pause.handle().get())
                    .unwrap()
                    .pending
                    .network_request_id,
                "document-collision"
            );
        }
        assert!(!owner.has_pending_fetch_state_for_test());
        assert!(
            browser
                .subscribe()
                .unwrap()
                .0
                .worker_fetch_pauses
                .is_empty()
        );
        service.shutdown();
        server.abort();
    }

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
                    && occurrence.owner
                        == moli_core::browser::NetworkOwner::Document(commit.document)
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
                        if let moli_core::page::RendererNetworkOutputItem::Resource(item) = &committed.occurrence().item
                            && matches!(item.as_ref(), moli_core::page::ScriptNetworkOutputItem::SubresourceNetworkRecord(record) if record.url().as_str() == "data:text/plain,native-recovery") {
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
            committed
                .occurrence()
                .source
                .document()
                .unwrap()
                .1
                .document
                .page_id
        );
        assert_ne!(
            peer_renderer.owner_local_host_id(),
            committed.occurrence().source.document().unwrap().0
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
            committed.occurrence().source.document().unwrap().0,
            committed
                .occurrence()
                .source
                .document()
                .unwrap()
                .1
                .document
                .page_id,
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
