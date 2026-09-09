use super::*;
use crate::conn::{BrowserContext, CdpTargetFilter};
use crate::domains::activity::RendererPublicationOwner;
use moli_core::browser::{
    BrowserContextHandle, BrowserContextStoragePartitionHandles, BrowserEvent, BrowserService,
    NavigationRequestLoadPolicy, StoragePartitionKind, WebContentsCreation, WorkerSnapshot,
};

struct NativeWorkers {
    service: BrowserService,
    context: BrowserContextHandle,
    output: moli_core::RendererOutputTransportReceiver,
}

impl NativeWorkers {
    async fn start(names: &[&str]) -> Self {
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
        let (sender, output) = moli_core::renderer_output_transport_channel();
        context
            .set_renderer_output_transport_sender(sender)
            .unwrap();
        let (contents, _) = context
            .create_web_contents(WebContentsCreation::default())
            .unwrap();
        let (_, mut events) = browser.subscribe().unwrap();
        let script = names
            .iter()
            .map(|name| {
                format!(
                    "new SharedWorker('data:text/javascript,onconnect = () => {{}}', {})",
                    serde_json::to_string(name).unwrap(),
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let url = format!("data:text/html,<script>globalThis.workers = [{script}]</script>");
        let navigation = context
            .navigate_document(
                contents,
                moli_core::browser::web_contents::NavigationRequestInterception::new(
                    url.parse().unwrap(),
                    "GET".into(),
                    None,
                    Vec::new(),
                    NavigationRequestLoadPolicy::BrowserInitiated,
                ),
            )
            .unwrap();
        assert!(matches!(
            navigation.wait().await.unwrap(),
            moli_core::browser::BrowserNavigationOutcome::Document(_)
        ));
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut created = std::collections::BTreeSet::new();
            while created.len() < names.len() {
                if let BrowserEvent::WorkerCreated(WorkerSnapshot::Shared { context: id, info }) =
                    events.recv().await.unwrap().event
                    && id == context.id()
                {
                    assert!(names.contains(&info.name.as_str()));
                    assert!(created.insert(info.name));
                }
            }
        })
        .await
        .expect("real workers must commit without a Protocol consumer");
        Self {
            service,
            context,
            output,
        }
    }

    fn connection(&self) -> CdpConnection {
        let mut conn = CdpConnection::new(
            self.service.handle(),
            crate::CdpInitialStoragePartition::memory(),
            Default::default(),
        );
        conn.set_target_discovery_for_owner(None, CdpTargetFilter::default_target_discovery());
        conn
    }

    async fn next_occurrence(
        &mut self,
    ) -> (
        moli_core::RendererOutputStreamIdentity,
        moli_core::page::RendererCommittedWorkerLifecycle,
    ) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let moli_core::RendererOutputTransportMessage::Publication(publication) = self
                    .output
                    .recv()
                    .await
                    .expect("worker transport closed before its lifecycle occurrence")
                else {
                    continue;
                };
                let stream = publication.cursor().stream();
                for record in publication.into_records() {
                    if let moli_core::RendererOutputItem::Observation(
                        moli_core::RendererProtocolObservation::WorkerLifecycle(observation),
                    ) = record.into_parts().1
                    {
                        return (
                            stream,
                            observation
                                .committed()
                                .await
                                .expect("live native input is committed"),
                        );
                    }
                }
            }
        })
        .await
        .expect("concrete Worker stream must retain its lifecycle receipt")
    }
}

#[tokio::test]
async fn native_shared_worker_snapshots_recover_lag_without_replaying_old_fifo_receipts() {
    use tokio::sync::broadcast::error::TryRecvError;
    let mut fixture = NativeWorkers::start(&["snapshot-worker"]).await;
    let (_, created) = fixture.next_occurrence().await;
    let browser = fixture.service.handle();
    let (snapshot, mut slow) = browser.subscribe().unwrap();
    let worker = snapshot.workers[0].clone();
    let mut conn = fixture.connection();
    let events = conn.project_browser_snapshot(snapshot).await;
    assert_eq!(
        protocol_event_count(&events, "Target.targetCreated", "shared_worker"),
        1
    );
    assert!(worker_lifecycle_prepared_outputs(&mut conn, created.clone()).is_empty());
    let moli_core::browser::WorkerHandle::Shared { instance, .. } = worker.handle();
    assert!(fixture.context.close_shared_worker(instance));
    let (_, destroyed) = fixture.next_occurrence().await;
    assert!(
        matches!(destroyed.lifecycle(), moli_core::page::RendererWorkerLifecycle::SharedDestroyed(actual) if *actual == instance)
    );
    // Force real loss in the bounded Browser stream, including the terminal
    // occurrence. Recovery must reconcile native membership, not guess pairs.
    for _ in 0..129 {
        browser
            .create_context(
                BrowserContextStoragePartitionHandles::memory(),
                StoragePartitionKind::Ephemeral,
                None,
                None,
            )
            .unwrap()
            .remove()
            .unwrap();
    }
    assert!(matches!(slow.try_recv(), Err(TryRecvError::Lagged(_))));
    let (snapshot, _) = browser.subscribe().unwrap();
    assert!(snapshot.workers.is_empty());
    let events = conn.project_browser_snapshot(snapshot).await;
    assert_eq!(
        events
            .iter()
            .filter(|event| (*event).clone().into_protocol_message()["method"]
                == "Target.targetDestroyed")
            .count(),
        1
    );
    assert!(worker_lifecycle_prepared_outputs(&mut conn, created).is_empty());
    assert!(worker_lifecycle_prepared_outputs(&mut conn, destroyed).is_empty());
    assert!(
        conn.browser_context_by_browser_id(fixture.context.id())
            .unwrap()
            .shared_worker_targets
            .is_empty()
    );
    fixture.service.shutdown();
}

#[tokio::test]
async fn native_shared_worker_interleaved_sources_do_not_share_a_fifo_progress_cursor() {
    let mut fixture = NativeWorkers::start(&["first-source", "second-source"]).await;
    let (_, first) = fixture.next_occurrence().await;
    let (_, second) = fixture.next_occurrence().await;
    let mut occurrences = [first, second];
    occurrences.sort_by_key(|occurrence| occurrence.browser_sequence());
    let mut conn = fixture.connection();
    conn.project_created_browser_context(fixture.context.id());
    // Each worker has its own FIFO; arrival order across sources is not the
    // Browser total order. Neither valid creation may be discarded as stale.
    for occurrence in occurrences.into_iter().rev() {
        let outputs = worker_lifecycle_prepared_outputs(&mut conn, occurrence);
        assert!(!outputs.is_empty());
        let events = worker_target_background_events_async(&mut conn, outputs).await;
        assert_eq!(
            protocol_event_count(&events, "Target.targetCreated", "shared_worker"),
            1
        );
    }
    assert_eq!(
        conn.browser_context_by_browser_id(fixture.context.id())
            .unwrap()
            .shared_worker_targets
            .len(),
        2
    );
    fixture.service.shutdown();
}

#[tokio::test]
async fn native_shared_worker_late_receipt_and_stream_cannot_rebind_a_reused_context_wire_id() {
    let mut fixture = NativeWorkers::start(&["retired-context-worker"]).await;
    let (stream, created) = fixture.next_occurrence().await;
    let mut conn = fixture.connection();
    conn.browser_context = Some(BrowserContext::from_browser_handle(
        "BID-reused".into(),
        fixture.context.clone(),
    ));
    let owner = RendererPublicationOwner::BrowserContext {
        browser_context_id: "BID-reused".into(),
    };
    assert!(owner.resolve(&conn, stream).is_some());
    fixture.context.remove().unwrap();
    let replacement = fixture
        .service
        .handle()
        .create_context(
            BrowserContextStoragePartitionHandles::memory(),
            StoragePartitionKind::Ephemeral,
            None,
            None,
        )
        .unwrap();
    conn.browser_context = Some(BrowserContext::from_browser_handle(
        "BID-reused".into(),
        replacement,
    ));
    assert!(owner.resolve(&conn, stream).is_none());
    assert!(worker_lifecycle_prepared_outputs(&mut conn, created).is_empty());
    assert!(
        conn.browser_context
            .as_ref()
            .unwrap()
            .shared_worker_targets
            .is_empty()
    );
    fixture.service.shutdown();
}

fn protocol_event_count(
    events: &[BackgroundProtocolEvent],
    method: &str,
    target_type: &str,
) -> usize {
    events
        .iter()
        .filter(|event| {
            let message = (*event).clone().into_protocol_message();
            message["method"] == method && message["params"]["targetInfo"]["type"] == target_type
        })
        .count()
}
