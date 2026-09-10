use super::*;
use crate::conn::{BrowserContext, CdpTargetFilter};
use crate::domains::activity::RendererPublicationOwner;
use moli_core::browser::{
    BrowserContextHandle, BrowserContextStoragePartitionHandles, BrowserEvent, BrowserService,
    NavigationRequestLoadPolicy, StoragePartitionKind, WebContentsCreation, WorkerSnapshot,
};

mod network;

#[tokio::test]
async fn dedicated_worker_creation_rejects_a_renderer_page_identity_mismatch() {
    let fixture = NativeWorkers::start_named(&["native-owner"], true).await;
    let mut snapshot = fixture.service.handle().subscribe().unwrap().0;
    let info = snapshot
        .workers
        .iter()
        .find_map(|worker| match worker {
            WorkerSnapshot::Dedicated { worker, .. } => Some(worker.info.clone()),
            _ => None,
        })
        .unwrap();
    snapshot.workers.clear();
    let mut conn = fixture.connection();
    conn.project_browser_snapshot(snapshot).await;
    let context_id = conn
        .browser_context_by_browser_id(fixture.context.id())
        .unwrap()
        .id
        .clone();
    let mut foreign = info.clone();
    let RendererDedicatedWorkerOwner::Document { page_id, .. } = &mut foreign.owner else {
        panic!("fixture must create its Worker from a real Document");
    };
    *page_id = moli_core::PageId::new_for_testing(0);
    assert!(
        register_native_dedicated_worker_projection(&mut conn, &context_id, foreign).is_empty()
    );
    assert!(
        conn.browser_context_by_id(&context_id)
            .unwrap()
            .dedicated_worker_targets
            .is_empty()
    );
    assert!(
        !register_native_dedicated_worker_projection(&mut conn, &context_id, info).is_empty(),
        "the exact native creator, unlike the mismatched Page, must be admitted"
    );
    fixture.service.shutdown();
}

struct NativeWorkers {
    service: BrowserService,
    context: BrowserContextHandle,
    output: moli_core::RendererOutputTransportReceiver,
    occurrences: std::collections::VecDeque<(
        moli_core::RendererOutputStreamIdentity,
        moli_core::page::RendererWorkerLifecycleObservation,
    )>,
}

impl NativeWorkers {
    async fn navigate_service_worker(
        &self,
        url: &str,
    ) -> moli_core::browser::ServiceWorkerSnapshot {
        let (snapshot, mut events) = self.service.handle().subscribe().unwrap();
        let contents = snapshot
            .web_contents
            .into_iter()
            .find(|contents| self.context.contains_web_contents(*contents))
            .unwrap();
        let navigation = self
            .context
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
            tokio::time::timeout(std::time::Duration::from_secs(5), navigation.wait())
                .await
                .unwrap_or_else(|_| panic!(
                    "navigation pending: {:?}",
                    self.service.handle().subscribe().unwrap().0
                ))
                .unwrap(),
            moli_core::browser::BrowserNavigationOutcome::Document(_)
        ));
        wait_service_worker_state(&mut events, |worker| {
            worker.info.status == RendererServiceWorkerVersionStatus::Activated
                && matches!(
                    worker.execution,
                    moli_core::browser::ServiceWorkerExecution::Running(_)
                )
        })
        .await
    }

    async fn start(names: &[&str]) -> Self {
        Self::start_named(names, false).await
    }

    async fn start_named(names: &[&str], dedicated: bool) -> Self {
        Self::start_script(
            names,
            dedicated,
            if dedicated {
                "onmessage = () => {}"
            } else {
                "onconnect = () => {}"
            },
        )
        .await
    }

    async fn start_script(names: &[&str], dedicated: bool, script: &str) -> Self {
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
        let script_url = serde_json::to_string(&format!("data:text/javascript,{script}")).unwrap();
        let script = names
            .iter()
            .map(|name| {
                if dedicated {
                    format!(
                        "new Worker({script_url}, {})",
                        serde_json::json!({ "name": name })
                    )
                } else {
                    format!(
                        "new SharedWorker({script_url}, {})",
                        serde_json::to_string(name).unwrap()
                    )
                }
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
                let name = match events.recv().await.unwrap().event {
                    BrowserEvent::WorkerCreated(WorkerSnapshot::Shared { context: id, info })
                        if !dedicated && id == context.id() =>
                    {
                        info.name
                    }
                    BrowserEvent::WorkerUpdated(WorkerSnapshot::Dedicated {
                        context: id,
                        worker,
                    }) if dedicated && id == context.id() => worker.info.name,
                    _ => continue,
                };
                assert!(names.contains(&name.as_str()));
                assert!(created.insert(name));
            }
        })
        .await
        .expect("real workers must commit without a Protocol consumer");
        Self {
            service,
            context,
            output,
            occurrences: Default::default(),
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
                if let Some((stream, observation)) = self.occurrences.pop_front() {
                    return (
                        stream,
                        observation
                            .committed()
                            .await
                            .expect("live native input is committed"),
                    );
                }
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
                        self.occurrences.push_back((stream, observation));
                    }
                }
            }
        })
        .await
        .expect("concrete Worker stream must retain its lifecycle receipt")
    }
}

async fn wait_service_worker_state(
    events: &mut tokio::sync::broadcast::Receiver<moli_core::browser::BrowserEventRecord>,
    predicate: impl Fn(&moli_core::browser::ServiceWorkerSnapshot) -> bool,
) -> moli_core::browser::ServiceWorkerSnapshot {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let BrowserEvent::WorkerUpdated(WorkerSnapshot::Service { worker, .. }) =
                events.recv().await.unwrap().event
                && predicate(&worker)
            {
                return worker;
            }
        }
    })
    .await
    .expect("ServiceWorker must make native progress without Protocol draining its output")
}

#[tokio::test]
async fn native_service_worker_snapshot_recovers_missed_restart_without_rotating_version() {
    use moli_core::browser::ServiceWorkerExecution;
    let server = moli_test_support::FixtureServer::spawn().await.unwrap();
    let mut fixture = NativeWorkers::start(&[]).await;
    let first = fixture
        .navigate_service_worker(&server.url("/native-service-worker/"))
        .await;
    let (_, created) = fixture.next_occurrence().await;
    assert!(matches!(
        created.lifecycle(),
        RendererWorkerLifecycle::Service(RendererServiceWorkerLifecycle::Created { .. })
    ));
    let browser = fixture.service.handle();
    let mut conn = fixture.connection();
    conn.project_browser_snapshot(browser.subscribe().unwrap().0)
        .await;
    let context_id = conn
        .browser_context_by_browser_id(fixture.context.id())
        .unwrap()
        .id
        .clone();
    let target = &conn
        .browser_context_by_id(&context_id)
        .unwrap()
        .service_worker_targets[&first.info.version_id];
    let target_id = target.target_id.clone();
    let version = target.version_identity(&context_id).unwrap();
    assert!(target.worker_running());
    assert_eq!(target.active_renderer_run(), first.execution.active_run());

    let (_, mut events) = browser.subscribe().unwrap();
    fixture
        .context
        .execute_service_worker_command(ServiceWorkerCommand::StopVersion {
            version_id: first.info.version_id,
        })
        .unwrap();
    wait_service_worker_state(&mut events, |worker| {
        worker.execution == ServiceWorkerExecution::Stopped
    })
    .await;
    fixture
        .context
        .execute_service_worker_command(ServiceWorkerCommand::Start {
            scope: first.info.scope_url.parse().unwrap(),
        })
        .unwrap();
    let restarted = wait_service_worker_state(&mut events, |worker| {
        matches!(worker.execution, ServiceWorkerExecution::Running(_))
    })
    .await;
    assert_ne!(
        first.execution.active_run(),
        restarted.execution.active_run()
    );
    let recovered = conn
        .project_browser_snapshot(browser.subscribe().unwrap().0)
        .await;
    assert_eq!(
        protocol_event_count(&recovered, "Target.targetCreated", "service_worker"),
        0
    );
    let target = &conn
        .browser_context_by_id(&context_id)
        .unwrap()
        .service_worker_targets[&first.info.version_id];
    assert_eq!(target.target_id, target_id);
    assert_eq!(target.version_identity(&context_id), Some(version.clone()));
    assert_eq!(
        target.active_renderer_run(),
        restarted.execution.active_run()
    );
    assert!(target.worker_running());
    assert!(worker_lifecycle_prepared_outputs(&mut conn, created).is_empty());
    loop {
        let (_, committed) = fixture.next_occurrence().await;
        let restarted_fact = matches!(committed.lifecycle(), RendererWorkerLifecycle::Service(RendererServiceWorkerLifecycle::Started { run, .. }) if Some(run) == restarted.execution.active_run());
        assert!(worker_lifecycle_prepared_outputs(&mut conn, committed).is_empty());
        if restarted_fact {
            break;
        }
    }

    fixture
        .context
        .execute_service_worker_command(ServiceWorkerCommand::StopVersion {
            version_id: first.info.version_id,
        })
        .unwrap();
    wait_service_worker_state(&mut events, |worker| {
        worker.execution == ServiceWorkerExecution::Stopped
    })
    .await;
    conn.project_browser_snapshot(browser.subscribe().unwrap().0)
        .await;
    let target = &conn
        .browser_context_by_id(&context_id)
        .unwrap()
        .service_worker_targets[&first.info.version_id];
    assert_eq!(target.target_id, target_id);
    assert_eq!(target.version_identity(&context_id), Some(version));
    assert!(target.active_renderer_run().is_none());
    let late = service_worker_observation_prepared_outputs(
        &mut conn,
        context_id.clone(),
        RendererServiceWorkerObservation::Console {
            version_id: first.info.version_id,
            run: restarted.execution.active_run().unwrap().clone(),
            message: RendererServiceWorkerConsoleMessage {
                message: "late retired run".into(),
                args: Vec::new(),
                stack: None,
            },
        },
    );
    assert!(late.is_empty());
    assert!(
        conn.browser_context_by_id(&context_id)
            .unwrap()
            .service_worker_targets[&first.info.version_id]
            .active_renderer_run()
            .is_none()
    );
    fixture.service.shutdown();
    server.shutdown().await;
}

#[tokio::test]
async fn native_service_worker_fifo_admits_the_real_host_before_early_console() {
    let server = moli_test_support::FixtureServer::spawn().await.unwrap();
    let mut fixture = NativeWorkers::start(&[]).await;
    // Create the worker before installing a live Protocol navigation-decision
    // provider. This test consumes the Worker FIFO, not a Page command loop.
    let worker = fixture
        .navigate_service_worker(&server.url("/native-service-worker/"))
        .await;
    let mut conn = fixture.connection();
    conn.project_created_browser_context(fixture.context.id());
    let context_id = conn
        .browser_context_by_browser_id(fixture.context.id())
        .unwrap()
        .id
        .clone();
    let mut console_seen = false;
    let mut started_runs = 0;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let moli_core::RendererOutputTransportMessage::Publication(publication) =
                fixture.output.recv().await.unwrap()
            else {
                continue;
            };
            for record in publication.into_records() {
                match record.into_parts().1 {
                    moli_core::RendererOutputItem::Observation(
                        moli_core::RendererProtocolObservation::WorkerLifecycle(observation),
                    ) => {
                        let committed = observation.committed().await.unwrap();
                        let started = matches!(
                            committed.lifecycle(),
                            RendererWorkerLifecycle::Service(
                                RendererServiceWorkerLifecycle::Started { .. }
                            )
                        );
                        let outputs = worker_lifecycle_prepared_outputs(&mut conn, committed);
                        worker_target_background_events_async(&mut conn, outputs).await;
                        if started {
                            assert!(
                                console_seen,
                                "top-level Console precedes public Started in the concrete FIFO"
                            );
                            started_runs += 1;
                            if started_runs == 2 {
                                return;
                            }
                            console_seen = false;
                            fixture
                                .context
                                .execute_service_worker_command(ServiceWorkerCommand::StopVersion {
                                    version_id: worker.info.version_id,
                                })
                                .unwrap();
                            fixture
                                .context
                                .execute_service_worker_command(ServiceWorkerCommand::Start {
                                    scope: worker.info.scope_url.parse().unwrap(),
                                })
                                .unwrap();
                        }
                    }
                    moli_core::RendererOutputItem::Observation(
                        moli_core::RendererProtocolObservation::ServiceWorker(observation),
                    ) => {
                        if let RendererServiceWorkerObservation::Console { run, message, .. } =
                            &observation
                        {
                            assert_eq!(message.message, "log: before native Started");
                            if started_runs == 0 {
                                assert_eq!(Some(run), worker.execution.active_run());
                            } else {
                                assert_ne!(Some(run), worker.execution.active_run());
                            }
                            let target = &conn
                                .browser_context_by_id(&context_id)
                                .unwrap()
                                .service_worker_targets[&worker.info.version_id];
                            assert_eq!(target.active_renderer_run(), Some(run));
                            assert!(!target.worker_running());
                            console_seen = true;
                        }
                        let outputs = service_worker_observation_prepared_outputs(
                            &mut conn,
                            context_id.clone(),
                            observation,
                        );
                        worker_target_background_events_async(&mut conn, outputs).await;
                    }
                    _ => {}
                }
            }
        }
    })
    .await
    .expect("native receipt and early output must retain source FIFO order");
    fixture.service.shutdown();
    server.shutdown().await;
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
    let moli_core::browser::WorkerHandle::Shared { instance, .. } = worker.handle() else {
        unreachable!()
    };
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
async fn native_dedicated_worker_snapshot_recovers_script_without_replaying_fifo_receipts() {
    let mut fixture = NativeWorkers::start_named(&["native-snapshot"], true).await;
    let (_, created) = fixture.next_occurrence().await;
    let (_, completed) = fixture.next_occurrence().await;
    assert!(matches!(
        created.lifecycle(),
        RendererWorkerLifecycle::DedicatedCreated(_)
    ));
    let RendererWorkerLifecycle::DedicatedScriptCompleted {
        instance_id,
        script,
    } = completed.lifecycle()
    else {
        panic!("script completion follows creation in the source FIFO")
    };
    let instance = *instance_id;
    let browser = fixture.service.handle();
    let snapshot = browser.subscribe().unwrap().0;
    let WorkerSnapshot::Dedicated { worker, .. } = &snapshot.workers[0] else {
        unreachable!()
    };
    assert!(std::sync::Arc::ptr_eq(
        worker.main_script.as_ref().unwrap(),
        script
    ));
    let mut conn = fixture.connection();
    let events = conn.project_browser_snapshot(snapshot).await;
    assert_eq!(
        protocol_event_count(&events, "Target.targetCreated", "worker"),
        1
    );
    let target = &conn
        .browser_context_by_browser_id(fixture.context.id())
        .unwrap()
        .dedicated_worker_targets[&instance];
    let target_id = target.target_id.clone();
    assert_eq!(target.url, script.script_url);
    assert!(
        std::ptr::eq(target.main_script().unwrap(), script.as_ref()),
        "Protocol replay must share the native fact, not copy its response"
    );
    assert!(worker_lifecycle_prepared_outputs(&mut conn, created.clone()).is_empty());
    assert!(worker_lifecycle_prepared_outputs(&mut conn, completed.clone()).is_empty());
    let events = conn
        .project_browser_snapshot(browser.subscribe().unwrap().0)
        .await;
    assert_eq!(
        protocol_event_count(&events, "Target.targetCreated", "worker"),
        0
    );
    assert_eq!(
        protocol_event_count(&events, "Target.targetInfoChanged", "worker"),
        0
    );

    assert!(fixture.context.close_dedicated_worker(instance));
    let (_, destroyed) = fixture.next_occurrence().await;
    assert_eq!(
        destroyed.lifecycle(),
        &RendererWorkerLifecycle::DedicatedDestroyed(instance)
    );
    let snapshot = browser.subscribe().unwrap().0;
    assert!(snapshot.workers.is_empty());
    let events = conn.project_browser_snapshot(snapshot).await;
    assert_eq!(
        events
            .iter()
            .filter(|event| {
                let message = (*event).clone().into_protocol_message();
                message["method"] == "Target.targetDestroyed"
                    && message["params"]["targetId"] == target_id
            })
            .count(),
        1
    );
    for receipt in [created, completed, destroyed] {
        assert!(worker_lifecycle_prepared_outputs(&mut conn, receipt).is_empty());
    }
    assert!(
        conn.browser_context_by_browser_id(fixture.context.id())
            .unwrap()
            .dedicated_worker_targets
            .is_empty()
    );
    fixture.service.shutdown();
}

#[tokio::test]
async fn native_dedicated_worker_old_document_receipts_cannot_create_on_replacement() {
    let mut fixture = NativeWorkers::start_named(&["old-document"], true).await;
    let (_, created) = fixture.next_occurrence().await;
    let (_, completed) = fixture.next_occurrence().await;
    let RendererWorkerLifecycle::DedicatedCreated(old) = created.lifecycle() else {
        unreachable!()
    };
    let old_instance = old.instance_id;
    let browser = fixture.service.handle();
    let (snapshot, mut events) = browser.subscribe().unwrap();
    let contents = snapshot.web_contents[0];
    let navigation = fixture.context.navigate_document(
        contents,
        moli_core::browser::web_contents::NavigationRequestInterception::new(
            "data:text/html,<script>globalThis.worker = new Worker('data:text/javascript,onmessage = () => {}', {name:'replacement'})</script>".parse().unwrap(),
            "GET".into(), None, Vec::new(), NavigationRequestLoadPolicy::BrowserInitiated,
        ),
    ).unwrap();
    let moli_core::browser::BrowserNavigationOutcome::Document(document) =
        navigation.wait().await.unwrap()
    else {
        unreachable!()
    };
    let replacement = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut retired = false;
        let mut replacement = None;
        loop {
            match events.recv().await.unwrap().event {
                BrowserEvent::WorkerUpdated(WorkerSnapshot::Dedicated { worker, .. })
                    if worker.info.instance_id != old_instance =>
                {
                    replacement = Some(worker)
                }
                BrowserEvent::WorkerDestroyed(moli_core::browser::WorkerHandle::Dedicated {
                    instance,
                    ..
                }) if instance == old_instance => retired = true,
                _ => {}
            }
            if retired && let Some(worker) = replacement.take() {
                break worker;
            }
        }
    })
    .await
    .expect("replacement must complete while the exact old Worker retires");
    let mut conn = fixture.connection();
    conn.project_created_browser_context(fixture.context.id());
    conn.project_created_web_contents(contents).await;
    conn.project_browser_document_commit(document.document)
        .await;
    assert!(
        conn.browser_context_by_browser_id(fixture.context.id())
            .unwrap()
            .worker_snapshot_sequence
            .is_none(),
        "exercise exact owner rejection, not recovery suppression"
    );
    assert!(worker_lifecycle_prepared_outputs(&mut conn, created).is_empty());
    assert!(worker_lifecycle_prepared_outputs(&mut conn, completed).is_empty());
    let mut projected = false;
    while !projected {
        let (_, receipt) = fixture.next_occurrence().await;
        projected = matches!(receipt.lifecycle(),
            RendererWorkerLifecycle::DedicatedScriptCompleted { instance_id, .. }
            if *instance_id == replacement.info.instance_id);
        let outputs = worker_lifecycle_prepared_outputs(&mut conn, receipt);
        worker_target_background_events_async(&mut conn, outputs).await;
    }
    let context = conn
        .browser_context_by_browser_id(fixture.context.id())
        .unwrap();
    assert_eq!(context.dedicated_worker_targets.len(), 1);
    let target = &context.dedicated_worker_targets[&replacement.info.instance_id];
    assert_eq!(target.name, "replacement");
    assert_eq!(target.url, replacement.main_script.unwrap().script_url);
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
