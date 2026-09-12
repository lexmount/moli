use super::*;
use moli_core::browser::{
    BrowserContextStoragePartitionHandles, BrowserService, StoragePartitionKind,
};
use moli_protocol::{
    CdpInitialStoragePartition,
    devtools_runtime::{
        DevToolsCommand, DevToolsCommandContext, DevToolsCommandResult, DevToolsNavigateCommand,
        DevToolsNavigationWait, DevToolsProtocol,
    },
};
use serde_json::{Value, json};

#[tokio::test]
async fn native_worker_network_real_lag_preserves_console_command_and_body_fifo() {
    tokio::task::LocalSet::new()
        .run_until(async {
            for recover in [false, true] {
                assert_worker_network_fifo(recover, false).await;
            }
        })
        .await;
}

async fn command(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    request: Value,
) -> Vec<Value> {
    scheduler
        .execute_internal_protocol_message(receivers, request)
        .await
        .unwrap_or_else(|failure| panic!("{:?}", failure.into_parts().1))
        .into_messages()
}

#[tokio::test]
async fn native_dedicated_worker_network_real_lag_preserves_console_command_and_body_fifo() {
    tokio::task::LocalSet::new()
        .run_until(async {
            for recover in [false, true] {
                assert_worker_network_fifo(recover, true).await;
            }
        })
        .await;
}

async fn assert_worker_network_fifo(recover: bool, dedicated: bool) {
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (mut scheduler, mut receivers) = CdpScheduler::new_with_initial_state_runtime_config(
        browser.clone(),
        CdpInitialStoragePartition::memory(),
        Default::default(),
    );
    let initial = Box::pin(scheduler.execute_devtools_command_with_external_load_wait_and_protocol_messages(
        &mut receivers,
        DevToolsCommand::Navigate(DevToolsNavigateCommand {
            context: DevToolsCommandContext { protocol: DevToolsProtocol::Cdp, session_id: None,
                target_id: Some(scheduler.conn.default_target_id().into()), browser_context_id: None },
            url: if dedicated {
                "data:text/html,<script>globalThis.worker = new Worker('data:text/javascript,onmessage=()=>{}', {name:'network-fifo'})</script>"
            } else {
                "data:text/html,<script>globalThis.worker = new SharedWorker('data:text/javascript,onconnect=()=>{}', 'network-fifo')</script>"
            }.into(),
            referrer: None, wait: DevToolsNavigationWait::DocumentInstalled,
        }),
    )).await;
    let DevToolsCommandResult::Navigate(initial) = initial.result.unwrap() else {
        panic!("native Page navigation required");
    };
    assert!(initial.error_text.is_none(), "{initial:?}");
    let target = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let messages = command(
                &mut scheduler,
                &mut receivers,
                json!({"id": 1, "method": "Target.getTargets"}),
            )
            .await;
            if let Some(target) = messages
                .iter()
                .find(|message| message["id"] == 1)
                .and_then(|message| message["result"]["targetInfos"].as_array())
                .and_then(|targets| {
                    targets.iter().find(|target| {
                        target["type"] == if dedicated { "worker" } else { "shared_worker" }
                            // Created exposes a loading Dedicated target. Its URL is
                            // published by ScriptCompleted, after execution binding;
                            // Runtime.enable alone can acknowledge only projection.
                            && (!dedicated || target["url"] == "data:text/javascript,onmessage=()=>{}")
                    })
                })
            {
                break target["targetId"].as_str().unwrap().to_owned();
            }
            let publication = receivers.renderer_publication_rx.recv().await.unwrap();
            scheduler.ingest_renderer_publication_now(publication).await;
        }
    })
    .await
    .expect("real Worker execution must reach the production target directory");
    let attached = command(&mut scheduler, &mut receivers, json!({"id": 2, "method": "Target.attachToTarget", "params": {"targetId": target, "flatten": true}})).await;
    let session =
        attached.iter().find(|message| message["id"] == 2).unwrap()["result"]["sessionId"]
            .as_str()
            .unwrap()
            .to_owned();
    for (id, method) in [(3, "Network.enable"), (4, "Runtime.enable")] {
        let setup = command(
            &mut scheduler,
            &mut receivers,
            json!({"id": id, "method": method, "sessionId": session}),
        )
        .await;
        assert!(
            setup
                .iter()
                .any(|message| message["id"] == id && message.get("result").is_some()),
            "{setup:?}"
        );
    }
    let (_, mut native) = browser.subscribe().unwrap();
    let outcome = scheduler.conn.process_message_with_turn_outcome_async(&json!({
        "id": 10, "method": "Runtime.evaluate", "sessionId": session, "params": {
            "expression": "console.log('before-worker-network'); fetch('data:text/plain,worker-fifo-body').then(r=>r.text()).then(body=>{ console.log('after-worker-network'); return body; })",
            "awaitPromise": true, "returnByValue": true,
        }
    }).to_string()).await;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut request = None;
        loop {
            let event = native.recv().await.unwrap().event;
            let (BrowserEvent::NetworkRequestStarted(occurrence) | BrowserEvent::NetworkRequestCompleted(occurrence)) = &event else { continue };
            if !matches!(occurrence.owner, moli_core::browser::NetworkOwner::Worker(_)) { continue; }
            let moli_core::page::RendererNetworkOutputItem::Resource(item) = &occurrence.renderer.item else { continue };
            match item.as_ref() {
                moli_core::page::ScriptNetworkOutputItem::SubresourceRequestStarted(start)
                    if start.url().as_str() == "data:text/plain,worker-fifo-body" => {
                    assert!(matches!(event, BrowserEvent::NetworkRequestStarted(_)));
                    assert!(request.replace((occurrence.owner, occurrence.renderer.source.identity(), start.handle())).is_none(), "one native admission");
                }
                moli_core::page::ScriptNetworkOutputItem::SubresourceBodyFinished(body)
                    if request.as_ref().is_some_and(|(owner, source, handle)|
                        *owner == occurrence.owner && *source == occurrence.renderer.source.identity() && *handle == body.handle()) => {
                    assert!(matches!(event, BrowserEvent::NetworkRequestCompleted(_)));
                    break;
                }
                _ => {}
            }
        }
    }).await.unwrap_or_else(|error| panic!(
        "native Worker response must complete before Protocol consumes its FIFO: {error:?}; dedicated={dedicated}, recover={recover}; outcome={outcome:#?}; snapshot={:#?}",
        browser.subscribe().unwrap().0,
    ));
    if recover {
        for _ in 0..130 {
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
        let lag = match scheduler.browser_event_rx.as_mut().unwrap().try_recv() {
            Err(TryRecvError::Lagged(count)) => Err(RecvError::Lagged(count)),
            event => panic!("expected real bounded Browser lag: {event:?}"),
        };
        let recovered = scheduler.handle_browser_event(lag).await.into_messages();
        assert!(
            recovered.iter().all(|message| !message["method"]
                .as_str()
                .is_some_and(|method| method.starts_with("Network."))),
            "snapshot must not overtake Worker Console/FIFO: {recovered:?}"
        );
    }
    let mut messages = scheduler
        .apply_renderer_owner_turn_outcome(&mut receivers, outcome)
        .await
        .unwrap_or_else(|failure| panic!("{:?}", failure.into_parts().1))
        .into_messages();
    messages.extend(
        command(
            &mut scheduler,
            &mut receivers,
            json!({"id": 11, "method": "Runtime.evaluate", "sessionId": session,
        "params": {"expression": "'worker-fifo-drained'", "returnByValue": true}}),
        )
        .await,
    );
    let requests = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| {
            message["method"] == "Network.requestWillBeSent"
                && message["params"]["request"]["url"] == "data:text/plain,worker-fifo-body"
        })
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 1, "{messages:?}");
    let (request_index, request) = requests[0];
    assert_eq!(request["sessionId"], session);
    assert!(request["params"].get("frameId").is_none());
    let request_id = request["params"]["requestId"].as_str().unwrap();
    assert_eq!(
        messages
            .iter()
            .filter(|message| message["method"] == "Network.loadingFinished"
                && message["params"]["requestId"] == request_id)
            .count(),
        1
    );
    let console = |value| {
        messages
            .iter()
            .position(|message| {
                message["method"] == "Runtime.consoleAPICalled"
                    && message["params"]["args"][0]["value"] == value
            })
            .unwrap_or_else(|| panic!("missing {value}: {messages:?}"))
    };
    let response_index = messages
        .iter()
        .position(|message| {
            message["id"] == 10 && message["result"]["result"]["value"] == "worker-fifo-body"
        })
        .unwrap();
    assert!(
        console("before-worker-network") < request_index
            && request_index < console("after-worker-network")
            && console("after-worker-network") < response_index,
        "{messages:?}"
    );
    assert!(messages.iter().any(|message| message["id"] == 11
        && message["result"]["result"]["value"] == "worker-fifo-drained"));
    let body = command(&mut scheduler, &mut receivers, json!({"id": 12, "method": "Network.getResponseBody", "sessionId": session, "params": {"requestId": request_id}})).await;
    assert!(
        body.iter().any(|message| message["id"] == 12
            && message["result"] == json!({"body": "worker-fifo-body", "base64Encoded": false})),
        "{body:?}"
    );
    let cleared = command(
        &mut scheduler,
        &mut receivers,
        json!({"id": 13, "method": "Network.clearBrowserCache"}),
    )
    .await;
    assert!(
        cleared
            .iter()
            .any(|message| message["id"] == 13 && message["result"] == json!({})),
        "{cleared:?}"
    );
    let body = command(&mut scheduler, &mut receivers, json!({"id": 14, "method": "Network.getResponseBody", "sessionId": session, "params": {"requestId": request_id}})).await;
    assert!(
        body.iter().any(|message| message["id"] == 14
            && message["error"]
                == json!({"code": -32000, "message": "No resource with given identifier found"})),
        "Context body cleanup must include its Worker: {body:?}"
    );
    service.shutdown();
}
