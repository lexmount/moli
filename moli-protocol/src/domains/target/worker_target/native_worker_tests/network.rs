use super::*;
use crate::devtools_runtime::{
    DevToolsCommand, DevToolsCommandContext, DevToolsCommandResult, DevToolsGetNetworkDataCommand,
    DevToolsNetworkDataType, DevToolsProtocol,
};
use moli_core::page::{
    RendererCommittedNetworkObservation, RendererNetworkOutputItem, RendererNetworkSource,
    RendererWorkerNetworkSource, ScriptNetworkOutputItem,
};

async fn network_command(
    conn: &mut CdpConnection,
    request: serde_json::Value,
) -> serde_json::Value {
    // These Network commands complete synchronously in Protocol. Keep the
    // fixture's real transport instead of attaching a second test scheduler.
    let (messages, work) = conn
        .process_message_with_turn_outcome_async(&request.to_string())
        .await
        .into_parts();
    assert!(
        work.is_empty(),
        "Network observation must not create Browser work"
    );
    messages
        .into_iter()
        .find(|message| message["id"] == request["id"])
        .expect("the exact Network command must return a response")
}

fn network_data_command(session: &str, request_id: &str) -> DevToolsGetNetworkDataCommand {
    DevToolsGetNetworkDataCommand {
        context: DevToolsCommandContext {
            protocol: DevToolsProtocol::WebDriverBidi,
            session_id: Some(session.into()),
            target_id: None,
            browser_context_id: None,
        },
        request_id: request_id.into(),
        data_type: DevToolsNetworkDataType::Response,
        collector: None,
        disown: false,
    }
}

impl NativeWorkers {
    async fn network_occurrences(
        &mut self,
        count: usize,
        url: &str,
    ) -> Vec<(
        moli_core::RendererOutputResidenceIdentity,
        RendererCommittedNetworkObservation,
    )> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut result = Vec::new();
            while result.len() < count {
                let moli_core::RendererOutputTransportMessage::Publication(publication) = self.output.recv().await.unwrap() else { continue; };
                let residence = publication.cursor().stream().residence();
                for record in publication.into_records() {
                    if let moli_core::RendererOutputItem::Observation(moli_core::RendererProtocolObservation::Network(observation)) = record.into_parts().1 {
                        let committed = observation.clone().committed().await.unwrap_or_else(|| panic!("native producer from {residence:?} rejected: {observation:?}; workers: {:?}", self.service.handle().subscribe().unwrap().0.workers));
                        if matches!(&committed.occurrence().item, RendererNetworkOutputItem::Resource(item)
                            if matches!(item.as_ref(), ScriptNetworkOutputItem::SubresourceNetworkRecord(record) if record.url().as_str() == url)) {
                            result.push((residence, committed));
                        }
                    }
                }
            }
            result
        }).await.expect("the real Worker FIFO must retain its Network receipts")
    }
}

fn attach_worker_network(
    conn: &mut CdpConnection,
    context_id: &str,
    source: &RendererWorkerNetworkSource,
    session: &str,
) -> CommandOwnerScope {
    let owner = conn
        .native_worker_network_owner(context_id, source)
        .unwrap();
    let context = conn.browser_context_by_id_mut(context_id).unwrap();
    match source {
        RendererWorkerNetworkSource::Shared(instance) => {
            let target = context.shared_worker_targets.get_mut(instance).unwrap();
            target.attach_session(session.into());
            assert!(target.set_network_enabled(session, true));
        }
        RendererWorkerNetworkSource::Service { version, .. } => {
            let target = context.service_worker_targets.get_mut(version).unwrap();
            target.attach_session(session.into());
            assert!(target.set_network_enabled(session, true));
        }
    }
    conn.register_session_route_for_test(session, owner.resolve_route(conn).unwrap());
    owner
}

#[tokio::test]
async fn native_shared_worker_network_snapshot_and_late_fifo_preserve_source_and_body_visibility() {
    const URL: &str = "data:text/plain,worker-network-body";
    // Real connect delivery triggers the requests while Browser retains the
    // Context. The legacy direct-evaluate test helper borrows that Context out
    // of its owner, so it cannot drive a test of concurrent native input.
    let mut fixture = NativeWorkers::start_script(
        &["first", "second"],
        false,
        "onconnect=()=>fetch('data:text/plain,worker-network-body').then(r=>r.text())",
    )
    .await;
    let observations = fixture.network_occurrences(2, URL).await;
    assert_ne!(observations[0].0, observations[1].0);
    let browser = fixture.service.handle();
    let mut snapshot = browser.subscribe().unwrap().0;
    let requests = std::mem::take(&mut snapshot.network_requests);
    assert_eq!(
        requests.len(),
        2,
        "equal URLs in separate physical Workers must not share a ledger entry"
    );
    let mut conn = fixture.connection();
    conn.project_browser_snapshot(snapshot).await;
    let context_id = conn
        .browser_context_by_browser_id(fixture.context.id())
        .unwrap()
        .id
        .clone();
    let owner = CommandOwnerScope::for_route(CdpSessionRoute::BrowserContext {
        browser_context_id: context_id.clone(),
    });
    let mut sessions = Vec::new();
    for (index, (_, observation)) in observations.iter().enumerate() {
        let RendererNetworkSource::Worker(source) = &observation.occurrence().source else {
            panic!("Worker network cannot be a Page fact");
        };
        let session = format!("SID-network-{index}");
        attach_worker_network(&mut conn, &context_id, source, &session);
        sessions.push(session);
    }
    let (collector, work) = conn
        .execute_devtools_command(DevToolsCommand::AddNetworkDataCollector(
            crate::devtools_runtime::DevToolsAddNetworkDataCollectorCommand {
                context: network_data_command(&sessions[0], "").context,
                collector_id: "shared-worker-collector".into(),
                data_types: vec![DevToolsNetworkDataType::Response],
                max_encoded_data_size: 1024,
                target_ids: Vec::new(),
                browser_context_ids: vec![context_id.clone().into()],
            },
        ))
        .await
        .into_parts();
    assert!(collector.is_ok(), "{collector:?}");
    assert!(work.is_empty());
    assert!(
        worker_network_prepared_outputs(&mut conn, &owner, observations[1].0, &observations[0].1)
            .is_empty(),
        "another Worker stream cannot consume this receipt"
    );
    let mut recover = browser.subscribe().unwrap().0;
    recover.network_requests = requests;
    let recovered = conn.project_browser_snapshot(recover.clone()).await;
    let messages = recovered
        .into_iter()
        .map(BackgroundProtocolEvent::into_protocol_message)
        .collect::<Vec<_>>();
    let starts = messages
        .iter()
        .filter(|message| message["method"] == "Network.requestWillBeSent")
        .collect::<Vec<_>>();
    assert_eq!(starts.len(), 2, "{messages:?}");
    assert_ne!(
        starts[0]["params"]["requestId"],
        starts[1]["params"]["requestId"]
    );
    assert!(
        starts
            .iter()
            .all(|message| message["params"].get("frameId").is_none())
    );
    assert_eq!(
        messages
            .iter()
            .filter(|message| message["method"] == "Network.loadingFinished")
            .count(),
        2
    );
    assert!(conn.project_browser_snapshot(recover).await.is_empty());
    for (residence, observation) in observations {
        assert!(
            worker_network_prepared_outputs(&mut conn, &owner, residence, &observation).is_empty(),
            "snapshot and delayed real FIFO may publish each request only once"
        );
    }
    for (index, session) in sessions.iter().enumerate() {
        let start = starts
            .iter()
            .find(|message| message["sessionId"] == *session)
            .unwrap();
        let request_id = start["params"]["requestId"].as_str().unwrap();
        let id = 800 + index as u64 * 10;
        let body = network_command(&mut conn, json!({"id": id, "method": "Network.getResponseBody", "sessionId": session, "params": {"requestId": request_id}})).await;
        assert_eq!(
            body,
            json!({"id": id, "sessionId": session, "result": {"body": "worker-network-body", "base64Encoded": false}})
        );
        let (data, work) = conn
            .execute_devtools_command(DevToolsCommand::GetNetworkData(network_data_command(
                session, request_id,
            )))
            .await
            .into_parts();
        assert!(work.is_empty());
        assert!(
            matches!(data.unwrap(), DevToolsCommandResult::NetworkData(data) if data.value == "worker-network-body")
        );
        let foreign = network_command(&mut conn, json!({"id": id + 1, "method": "Network.getResponseBody", "sessionId": sessions[1 - index], "params": {"requestId": request_id}})).await;
        assert_eq!(
            foreign["error"],
            json!({"code": -32000, "message": "No resource with given identifier found"})
        );
        let disabled = network_command(
            &mut conn,
            json!({"id": id + 2, "method": "Network.disable", "sessionId": session}),
        )
        .await;
        assert_eq!(disabled["result"], json!({}));
        let body = network_command(&mut conn, json!({"id": id + 3, "method": "Network.getResponseBody", "sessionId": session, "params": {"requestId": request_id}})).await;
        assert_eq!(
            body["error"],
            json!({"code": -32000, "message": "No resource with given identifier found"})
        );
    }
    fixture.service.shutdown();
}

#[tokio::test]
async fn native_service_worker_network_held_output_and_old_receipt_cannot_cross_restart() {
    use moli_core::browser::ServiceWorkerExecution;
    let server = moli_test_support::FixtureServer::spawn().await.unwrap();
    let mut fixture = NativeWorkers::start(&[]).await;
    let first = fixture
        .navigate_service_worker(&server.url("/native-worker-network/"))
        .await;
    let observation = fixture
        .network_occurrences(1, &server.url("/native-worker-network/probe"))
        .await
        .pop()
        .unwrap();
    let RendererNetworkSource::Worker(source) = &observation.1.occurrence().source else {
        panic!("native Service Worker source required");
    };
    let browser = fixture.service.handle();
    let mut snapshot = browser.subscribe().unwrap().0;
    snapshot.network_requests.clear();
    let mut conn = fixture.connection();
    conn.project_browser_snapshot(snapshot).await;
    let context_id = conn
        .browser_context_by_browser_id(fixture.context.id())
        .unwrap()
        .id
        .clone();
    let network_owner = attach_worker_network(&mut conn, &context_id, source, "SID-network");
    let target_id = conn
        .network_owner_identity_for_owner(&network_owner)
        .unwrap()
        .1
        .unwrap();
    let (collector, work) = conn
        .execute_devtools_command(DevToolsCommand::AddNetworkDataCollector(
            crate::devtools_runtime::DevToolsAddNetworkDataCollectorCommand {
                context: network_data_command("SID-network", "").context,
                collector_id: "worker-collector".into(),
                data_types: vec![DevToolsNetworkDataType::Response],
                max_encoded_data_size: 1024,
                target_ids: vec![target_id.into()],
                browser_context_ids: Vec::new(),
            },
        ))
        .await
        .into_parts();
    assert!(collector.is_ok(), "{collector:?}");
    assert!(work.is_empty());
    let owner = CommandOwnerScope::for_route(CdpSessionRoute::BrowserContext {
        browser_context_id: context_id.clone(),
    });
    let held = worker_network_prepared_outputs(&mut conn, &owner, observation.0, &observation.1);
    assert!(!held.is_empty());
    let old_request = held
        .worker_target_lifecycle_outputs
        .iter()
        .find_map(|output| {
            let WorkerTargetLifecycleOutput::ServiceWorkerRuntimeEvents { events, .. } = output
            else {
                return None;
            };
            events
                .iter()
                .find(|event| event.protocol_method() == Some("Network.requestWillBeSent"))
                .map(|event| {
                    event.clone().into_protocol_message()["params"]["requestId"]
                        .as_str()
                        .unwrap()
                        .to_owned()
                })
        })
        .unwrap();
    assert!(
        conn.network_agent_for_owner(&network_owner)
            .unwrap()
            .captured_response_body(&old_request)
            .is_some()
    );
    let mut get_data = network_data_command("SID-network", &old_request);
    get_data.collector = Some("worker-collector".into());
    let (data, work) = conn
        .execute_devtools_command(DevToolsCommand::GetNetworkData(get_data))
        .await
        .into_parts();
    assert!(work.is_empty());
    assert!(
        matches!(data.unwrap(), DevToolsCommandResult::NetworkData(data) if data.value == "native worker network body")
    );
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
    let next = fixture
        .network_occurrences(1, &server.url("/native-worker-network/probe"))
        .await
        .pop()
        .unwrap();
    let recovered = conn
        .project_browser_snapshot(browser.subscribe().unwrap().0)
        .await;
    assert_eq!(
        recovered
            .iter()
            .filter(|event| event.protocol_method() == Some("Network.loadingFinished"))
            .count(),
        1
    );
    assert!(
        worker_target_background_events_async(&mut conn, held)
            .await
            .is_empty(),
        "retired run output cannot be delivered to the successor's attachment"
    );
    assert!(
        worker_network_prepared_outputs(&mut conn, &owner, observation.0, &observation.1)
            .is_empty()
    );
    assert!(
        worker_network_prepared_outputs(&mut conn, &owner, next.0, &next.1).is_empty(),
        "new run snapshot and its delayed receipt deduplicate"
    );
    assert!(
        conn.network_agent_for_owner(&network_owner)
            .unwrap()
            .captured_response_body(&old_request)
            .is_none()
    );
    fixture.service.shutdown();
    server.shutdown().await;
}
