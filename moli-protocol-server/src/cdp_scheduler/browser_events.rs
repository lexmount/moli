use moli_core::browser::{BrowserEvent, BrowserEventReceiver, BrowserEventRecord};
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

use super::{
    CdpScheduler, CdpSchedulerEventReceivers, CdpSchedulerInterleavedInput, ProtocolOutputSequence,
};

pub(super) type BrowserEventInput = Result<BrowserEventRecord, RecvError>;

#[cfg(test)]
mod worker_network_tests;

pub(super) async fn recv_browser_event(
    receiver: &mut Option<BrowserEventReceiver>,
) -> BrowserEventInput {
    match receiver {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}

impl CdpScheduler {
    pub(crate) fn is_browser_closed(&self) -> bool {
        self.browser_event_rx.is_none()
    }

    pub(crate) async fn recv_interleaved_input(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
    ) -> Option<CdpSchedulerInterleavedInput> {
        receivers
            .recv_interleaved_input(&mut self.browser_event_rx, &mut self.detached_navigations)
            .await
    }

    pub(crate) async fn drain_browser_events(&mut self) -> ProtocolOutputSequence {
        let mut output = self.project_initial_browser_snapshot().await;
        while let Some(receiver) = self.browser_event_rx.as_mut() {
            let event = match receiver.try_recv() {
                Ok(event) => Ok(event),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Lagged(count)) => Err(RecvError::Lagged(count)),
                Err(TryRecvError::Closed) => Err(RecvError::Closed),
            };
            output.append(self.handle_browser_event(event).await);
        }
        output
    }

    async fn project_initial_browser_snapshot(&mut self) -> ProtocolOutputSequence {
        match self.initial_browser_snapshot.take() {
            Some(snapshot) => ProtocolOutputSequence::from_background_events(
                self.conn.project_browser_snapshot(snapshot).await,
            ),
            None => ProtocolOutputSequence::empty(),
        }
    }

    pub(crate) async fn handle_browser_event(
        &mut self,
        event: BrowserEventInput,
    ) -> ProtocolOutputSequence {
        let mut output = self.project_initial_browser_snapshot().await;
        let events = match event {
            Ok(record) => match record.event.clone() {
                BrowserEvent::ContextCreated(context) => {
                    self.conn.project_created_browser_context(context);
                    Vec::new()
                }
                BrowserEvent::WebContentsCreated(handle) => {
                    self.conn.project_created_web_contents(handle).await
                }
                BrowserEvent::WebContentsActivated { .. } => {
                    self.conn.project_browser_web_contents_activation(record)
                }
                BrowserEvent::DocumentCommitted(document) => {
                    self.conn.project_browser_document_commit(document).await
                }
                BrowserEvent::NavigationAwaitingDecision(request) => {
                    self.conn
                        .project_browser_navigation_decision(request.web_contents, None)
                        .await
                }
                BrowserEvent::InitialDocumentAwaitingInspection { web_contents, key }
                | BrowserEvent::InitialDocumentConstructionFailed { web_contents, key } => {
                    self.conn
                        .project_browser_initial_document_inspection(web_contents, Some(key))
                        .await
                }
                BrowserEvent::NavigationResponseChanged(request) => {
                    self.conn
                        .project_browser_navigation_responses(request.web_contents)
                        .await
                }
                BrowserEvent::NavigationStarted(request)
                | BrowserEvent::NavigationFailed { request, .. } => {
                    self.conn
                        .project_browser_navigation(request.web_contents)
                        .await
                }
                BrowserEvent::WorkerCreated(_)
                | BrowserEvent::WorkerFetchPaused(_)
                | BrowserEvent::NetworkRequestStarted(_)
                | BrowserEvent::NetworkRequestCompleted(_)
                | BrowserEvent::NetworkActivity(_)
                | BrowserEvent::NetworkSourceClosed { .. }
                | BrowserEvent::WorkerUpdated(_)
                | BrowserEvent::WorkerDestroyed(_)
                | BrowserEvent::DocumentLifecycleChanged(_)
                | BrowserEvent::DocumentTitleChanged(_)
                | BrowserEvent::DialogOpened(_)
                | BrowserEvent::DialogClosed { .. } => {
                    // Native state/waiters have already advanced. Frontend
                    // visibility still consumes the exact renderer FIFO so a
                    // lifecycle event cannot overtake an earlier dialog or
                    // command response fence.
                    Vec::new()
                }
                BrowserEvent::DownloadCreated(download) => {
                    self.conn.project_created_browser_download(download)
                }
                BrowserEvent::DownloadUpdated(download) => {
                    self.conn.project_browser_download(download)
                }
                BrowserEvent::ContextDisposed(context) => {
                    self.conn.project_disposed_browser_context(context).await
                }
                BrowserEvent::WebContentsClosed {
                    web_contents,
                    activated,
                } => {
                    self.conn
                        .project_closed_web_contents(web_contents, activated, record.sequence)
                        .await
                }
            },
            Err(error) => {
                let live = match error {
                    RecvError::Lagged(_) => self.conn.subscribe_browser_events().ok(),
                    RecvError::Closed => None,
                };
                if let Some((snapshot, receiver)) = live {
                    self.browser_event_rx = Some(receiver);
                    self.conn.project_browser_snapshot(snapshot).await
                } else {
                    self.browser_event_rx = None;
                    let contexts = self
                        .conn
                        .browser_contexts()
                        .map(|context| context.browser_context_id())
                        .collect::<Vec<_>>();
                    let mut events = Vec::new();
                    for context in contexts {
                        events.extend(self.conn.project_disposed_browser_context(context).await);
                    }
                    events
                }
            }
        };
        output.append(ProtocolOutputSequence::from_background_events(events));
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_core::browser::BrowserService;
    use moli_protocol::CdpInitialStoragePartition;

    #[tokio::test]
    async fn native_network_lag_recovery_cannot_overtake_retained_renderer_fifo() {
        tokio::task::LocalSet::new()
            .run_until(async {
                for (replace_document, recover) in
                    [(false, false), (false, true), (true, false), (true, true)]
                {
                    assert_native_network_recovery_fifo(
                        replace_document,
                        recover,
                        NetworkFixture::Resource,
                    )
                    .await;
                }
            })
            .await;
    }

    #[tokio::test]
    async fn native_child_network_lag_recovery_preserves_navigation_and_command_fifo() {
        tokio::task::LocalSet::new()
            .run_until(async {
                for (replace_document, recover) in
                    [(false, false), (false, true), (true, false), (true, true)]
                {
                    assert_native_network_recovery_fifo(
                        replace_document,
                        recover,
                        NetworkFixture::ChildResponse,
                    )
                    .await;
                }
            })
            .await;
    }

    #[tokio::test]
    async fn native_child_network_failure_lag_recovery_preserves_source_and_command_fifo() {
        tokio::task::LocalSet::new()
            .run_until(async {
                for (replace_document, recover) in
                    [(false, false), (false, true), (true, false), (true, true)]
                {
                    assert_native_network_recovery_fifo(
                        replace_document,
                        recover,
                        NetworkFixture::ChildFailure,
                    )
                    .await;
                }
            })
            .await;
    }

    #[derive(Clone, Copy)]
    enum NetworkFixture {
        Resource,
        ChildResponse,
        ChildFailure,
    }

    async fn assert_native_network_recovery_fifo(
        replace_document: bool,
        recover: bool,
        fixture: NetworkFixture,
    ) {
        use moli_core::browser::{
            BrowserContextStoragePartitionHandles, BrowserNavigationOutcome,
            NavigationRequestLoadPolicy, StoragePartitionKind,
            web_contents::NavigationRequestInterception,
        };
        use moli_protocol::devtools_runtime::{
            DevToolsCommand, DevToolsCommandContext, DevToolsCommandResult,
            DevToolsNavigateCommand, DevToolsNavigationWait, DevToolsProtocol,
        };
        use serde_json::json;

        let child_document = !matches!(fixture, NetworkFixture::Resource);
        let failed = matches!(fixture, NetworkFixture::ChildFailure);
        let terminal_method = if failed {
            "Network.loadingFailed"
        } else {
            "Network.loadingFinished"
        };

        let service = BrowserService::start().unwrap();
        let browser = service.handle();
        let (mut scheduler, mut receivers) = CdpScheduler::new_with_initial_state_runtime_config(
            browser.clone(),
            CdpInitialStoragePartition::memory(),
            Default::default(),
        );
        let initial = Box::pin(
            scheduler.execute_devtools_command_with_external_load_wait_and_protocol_messages(
                &mut receivers,
                DevToolsCommand::Navigate(DevToolsNavigateCommand {
                    context: DevToolsCommandContext {
                        protocol: DevToolsProtocol::Cdp,
                        session_id: None,
                        target_id: Some(scheduler.conn.default_target_id().into()),
                        browser_context_id: None,
                    },
                    url: "data:text/html,network recovery fixture".to_owned(),
                    referrer: None,
                    wait: DevToolsNavigationWait::DocumentInstalled,
                }),
            ),
        )
        .await;
        let DevToolsCommandResult::Navigate(initial) = initial.result.unwrap() else {
            panic!("expected native navigation");
        };
        assert!(initial.error_text.is_none(), "{initial:?}");
        scheduler.drain_browser_events().await;
        for (id, method) in [
            (1, "Runtime.enable"),
            (2, "Network.enable"),
            (3, "Page.enable"),
        ] {
            let setup = scheduler
                .execute_internal_protocol_message(
                    &mut receivers,
                    json!({"id": id, "method": method}),
                )
                .await
                .unwrap_or_else(|failure| panic!("{:?}", failure.into_parts().1))
                .into_messages();
            assert!(
                setup
                    .iter()
                    .any(|message| message["id"] == id && message.get("result").is_some()),
                "{setup:?}"
            );
        }
        let contents = scheduler.conn.projected_web_contents()[0];
        let (_, mut native) = browser.subscribe().unwrap();
        let (resource_url, child_server) = if child_document {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/native-child", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).await.unwrap();
                    request.push(byte[0]);
                }
                assert!(String::from_utf8_lossy(&request).starts_with("GET /native-child "));
                if failed {
                    return;
                }
                let body = "<!doctype html><p>native child FIFO body</p>";
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            });
            (url, Some(server))
        } else {
            ("data:text/plain,native-recovery-fifo".to_owned(), None)
        };
        let operation = if child_document {
            format!(
                "const frame = document.createElement('iframe'); frame.src = {resource_url:?}; document.body.appendChild(frame)"
            )
        } else {
            format!("fetch({resource_url:?}).then(r => r.text())")
        };
        let expression = format!("console.log('before-network-recovery'); {operation}; 'started'");
        // Execute real JavaScript but retain its real renderer publications.
        // The Browser can finish the fetch before this observer drains them.
        let mut messages = Vec::new();
        let (command, navigation) = if replace_document {
            let context = browser.context_handle(contents.context()).unwrap();
            let navigation = context
                .navigate_document(
                    contents,
                    NavigationRequestInterception::new(
                        // This fixture appends a child to document.body. The
                        // parser must create that body before the inline script.
                        format!("data:text/html,<body><script>{expression}</script>")
                            .parse()
                            .unwrap(),
                        "GET".into(),
                        None,
                        Vec::new(),
                        NavigationRequestLoadPolicy::BrowserInitiated,
                    ),
                )
                .unwrap();
            (None, Some(navigation))
        } else {
            (
                Some(
                    scheduler
                        .conn
                        .process_message_with_turn_outcome_async(
                            &json!({
                                "id": 10, "method": "Runtime.evaluate", "params": {
                                    "expression": expression, "returnByValue": true,
                                },
                            })
                            .to_string(),
                        )
                        .await,
                ),
                None,
            )
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let record = native.recv().await.unwrap();
                if let BrowserEvent::NetworkRequestCompleted(occurrence) = &record.event
                    && matches!(occurrence.owner, moli_core::browser::NetworkOwner::Document(document) if document.web_contents() == contents)
                {
                    if let Some(navigation) = &navigation {
                        assert_eq!(occurrence.owner, moli_core::browser::NetworkOwner::Document(moli_core::browser::DocumentHandle::new(contents, navigation.request().document)));
                    }
                    break;
                }
                // Configure the real reserved Document, but leave its
                // commit and source publications for recovery/ingress.
                if matches!(record.event, BrowserEvent::NavigationAwaitingDecision(_)) {
                    messages.extend(
                        scheduler
                            .handle_browser_event(Ok(record))
                            .await
                            .into_messages(),
                    );
                }
            }
        })
        .await
        .unwrap_or_else(|error| panic!("the native resource completes without draining its source FIFO: replace_document={replace_document}, recover={recover}, child_document={child_document}: {error}"));
        if let Some(navigation) = navigation {
            assert!(matches!(
                navigation.wait().await.unwrap(),
                BrowserNavigationOutcome::Document(_)
            ));
        }
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
                event => panic!("the real bounded subscription must lag: {event:?}"),
            };
            let recovered = scheduler.handle_browser_event(lag).await.into_messages();
            assert!(
                recovered
                    .iter()
                    .all(|message| message["params"]["request"]["url"] != resource_url),
                "snapshot recovery must not publish Network ahead of the retained Console/source FIFO: {recovered:?}",
            );
            if !replace_document {
                assert!(
                    recovered.iter().all(|message| !message["method"]
                        .as_str()
                        .is_some_and(|method| method.starts_with("Network."))),
                    "{recovered:?}"
                );
            }
            messages.extend(recovered);
        }
        if let Some(command) = command {
            messages.extend(
                scheduler
                    .apply_renderer_owner_turn_outcome(&mut receivers, command)
                    .await
                    .unwrap_or_else(|failure| panic!("{:?}", failure.into_parts().1))
                    .into_messages(),
            );
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(request) = messages.iter().find(|message| {
                    message["method"] == "Network.requestWillBeSent"
                        && message["params"]["request"]["url"] == resource_url
                }) && messages.iter().any(|message| {
                    message["method"] == terminal_method
                        && message["params"]["requestId"] == request["params"]["requestId"]
                }) {
                    break;
                }
                let publication = receivers.renderer_publication_rx.recv().await.unwrap();
                messages.extend(
                    scheduler
                        .ingest_renderer_publication_now(publication)
                        .await
                        .into_messages(),
                );
            }
        })
        .await
        .expect("recovery must leave the original FIFO deliverable");
        // A real owner command freezes a predecessor after the resource
        // completion. Consume that prefix before checking for duplicates.
        messages.extend(
            scheduler
                .execute_internal_protocol_message(
                    &mut receivers,
                    json!({"id": 11, "method": "Runtime.evaluate", "params": {
                        "expression": "'fifo-drained'", "returnByValue": true,
                    }}),
                )
                .await
                .unwrap_or_else(|failure| panic!("{:?}", failure.into_parts().1))
                .into_messages(),
        );
        assert!(
            messages.iter().any(|message| message["id"] == 11
                && message["result"]["result"]["value"] == "fifo-drained"),
            "{messages:?}"
        );
        let console = messages
            .iter()
            .position(|message| {
                message["method"] == "Runtime.consoleAPICalled"
                    && message["params"]["args"][0]["value"] == "before-network-recovery"
            })
            .expect("original Console occurrence");
        let request = messages
            .iter()
            .position(|message| {
                message["method"] == "Network.requestWillBeSent"
                    && message["params"]["request"]["url"] == resource_url
            })
            .expect("original request occurrence");
        assert!(
            console < request,
            "replace_document={replace_document}, recover={recover}: {messages:?}"
        );
        let request_id = &messages[request]["params"]["requestId"];
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["method"] == terminal_method
                    && &message["params"]["requestId"] == request_id)
                .count(),
            1,
            "{messages:?}"
        );
        if failed {
            assert!(!messages.iter().any(|message| matches!(
                message["method"].as_str(),
                Some("Network.responseReceived" | "Network.loadingFinished")
            ) && &message["params"]["requestId"]
                == request_id));
            let body = scheduler.execute_internal_protocol_message(&mut receivers, json!({"id":12,"method":"Network.getResponseBody","params":{"requestId":request_id}})).await
                .unwrap_or_else(|failure| panic!("{:?}", failure.into_parts().1)).into_messages();
            assert!(
                body.iter().any(|message| message["id"] == 12
                    && message["error"]["message"]
                        == "No data found for resource with given identifier"),
                "{body:?}"
            );
        } else if child_document {
            let frame = &messages[request]["params"]["frameId"];
            assert_eq!(messages[request]["params"]["type"], "Document");
            let start = messages
                .iter()
                .position(|message| {
                    message["method"] == "Page.frameStartedNavigating"
                        && &message["params"]["frameId"] == frame
                })
                .unwrap();
            let finish = messages
                .iter()
                .position(|message| {
                    message["method"] == "Network.loadingFinished"
                        && &message["params"]["requestId"] == request_id
                })
                .unwrap();
            let commit = messages
                .iter()
                .position(|message| {
                    message["method"] == "Page.frameNavigated"
                        && &message["params"]["frame"]["id"] == frame
                })
                .unwrap();
            assert!(
                start < request && request < finish && finish < commit,
                "{messages:?}"
            );
            let body = scheduler.execute_internal_protocol_message(&mut receivers, json!({"id":12,"method":"Network.getResponseBody","params":{"requestId":request_id}})).await
                .unwrap_or_else(|failure| panic!("{:?}", failure.into_parts().1)).into_messages();
            assert!(
                body.iter().any(|message| message["id"] == 12
                    && message["result"]["body"]
                        .as_str()
                        .is_some_and(|body| body.contains("native child FIFO body"))),
                "{body:?}"
            );
        }
        if !replace_document {
            let response = messages
                .iter()
                .position(|message| message["id"] == 10)
                .expect("original command response");
            assert_eq!(
                messages[response]["result"]["result"]["value"], "started",
                "{messages:?}"
            );
            assert!(
                console < response,
                "the command retains its concrete output predecessor: {messages:?}"
            );
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message["id"] == 10)
                    .count(),
                1
            );
        }
        if let Some(server) = child_server {
            server.await.unwrap();
        }
        service.shutdown();
    }

    #[tokio::test]
    async fn native_navigation_events_recover_projection_holds_after_real_stream_lag() {
        use moli_core::browser::{BrowserContextStoragePartitionHandles, StoragePartitionKind};
        use moli_protocol::devtools_runtime::{
            DevToolsCommand, DevToolsCommandContext, DevToolsCommandResult,
            DevToolsNavigateCommand, DevToolsNavigationWait, DevToolsProtocol,
        };
        use serde_json::json;
        tokio::task::LocalSet::new()
            .run_until(async {
                for lagged in [false, true] {
                    let service = BrowserService::start().unwrap();
                    let browser = service.handle();
                    let (mut scheduler, mut receivers) =
                        CdpScheduler::new_with_initial_state_runtime_config(
                            browser.clone(),
                            CdpInitialStoragePartition::memory(),
                            Default::default(),
                        );
                    let initial = Box::pin(
                        scheduler.execute_devtools_command_with_external_load_wait_and_protocol_messages(
                            &mut receivers,
                            DevToolsCommand::Navigate(DevToolsNavigateCommand {
                                context: DevToolsCommandContext {
                                    protocol: DevToolsProtocol::Cdp,
                                    session_id: None,
                                    target_id: Some(scheduler.conn.default_target_id().into()),
                                    browser_context_id: None,
                                },
                                url: "data:text/html,<title>native navigation observer</title>".to_owned(),
                                referrer: None,
                                wait: DevToolsNavigationWait::DocumentInstalled,
                            }),
                        ),
                    )
                    .await;
                    let DevToolsCommandResult::Navigate(initial) = initial.result.unwrap() else {
                        panic!("expected the initial document navigation result");
                    };
                    assert!(initial.error_text.is_none(), "{initial:?}");
                    scheduler.drain_browser_events().await;
                    let contents = scheduler.conn.projected_web_contents()[0];
                    let context = browser.context_handle(contents.context()).unwrap();
                    let document = context.document_handle(contents).unwrap().unwrap();
                    let (_, mut native_events) = browser.subscribe().unwrap();
                    let navigation = context.start_document_navigation(contents).unwrap();
                    let started = std::iter::from_fn(|| native_events.try_recv().ok())
                        .find(|record| matches!(record.event, BrowserEvent::NavigationStarted(request) if request.navigation == navigation))
                        .expect("exact native start occurrence");
                    for (id, expected) in [(2, 1), (3, 0)] {
                        if expected == 0 {
                            assert!(
                                context
                                    .cancel_document_navigation(contents, &navigation)
                                    .unwrap()
                            );
                        }
                        if lagged {
                            for _ in 0..130 {
                                let transient = browser
                                    .create_context(
                                        BrowserContextStoragePartitionHandles::memory(),
                                        StoragePartitionKind::Ephemeral,
                                        None,
                                        None,
                                    )
                                    .unwrap();
                                transient.remove().unwrap();
                            }
                            assert!(matches!(
                                native_events.try_recv(),
                                Err(TryRecvError::Lagged(_))
                            ));
                            native_events = browser.subscribe().unwrap().1;
                        }
                        scheduler.drain_browser_events().await;
                        let output = scheduler
                            .execute_internal_protocol_message(
                                &mut receivers,
                                json!({
                                    "id": id, "method": "HeapProfiler.moliDiagnostics",
                                }),
                            )
                            .await
                            .unwrap_or_else(|failure| panic!("{:?}", failure.into_parts().1))
                            .into_messages();
                        let response = output.iter().find(|message| message["id"] == id).unwrap();
                        assert_eq!(
                            response["result"]["activeBrowserContext"]["activeRuntimeSlot"]["pendingDocumentProjectionCount"],
                            expected,
                            "lagged={lagged}: {response:?}"
                        );
                        assert_eq!(context.document_handle(contents).unwrap(), Some(document));
                    }
                    assert!(
                        scheduler
                            .handle_browser_event(Ok(started))
                            .await
                            .into_messages()
                            .is_empty()
                    );
                    assert!(
                        matches!(context.navigation_snapshot(contents).unwrap().attempt, Some(moli_core::browser::NavigationAttempt::Failed { request, .. }) if request.navigation == navigation)
                    );
                    service.shutdown();
                }
            })
            .await;
    }

    #[tokio::test]
    async fn native_download_events_recover_after_lag_and_outlive_the_source_page() {
        use moli_core::browser::{
            BrowserContextStoragePartitionHandles, DownloadBehavior, DownloadBody, DownloadPolicy,
            DownloadState, StoragePartitionKind, WebContentsCreation,
        };
        use serde_json::json;
        struct Directory(std::path::PathBuf);
        impl Drop for Directory {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        for (lagged, retire_context) in [(false, false), (true, false), (false, true), (true, true)]
        {
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
            let directory = Directory(std::env::temp_dir().join(format!(
                "moli-native-download-{}-{}",
                std::process::id(),
                context.id().get()
            )));
            std::fs::create_dir(&directory.0).unwrap();
            let (contents, _) = context
                .create_web_contents(WebContentsCreation::with_initial_document(
                    "about:blank".into(),
                    None,
                    None,
                ))
                .unwrap();
            let (mut scheduler, mut receivers) =
                CdpScheduler::new_with_initial_state_runtime_config(
                    browser.clone(),
                    CdpInitialStoragePartition::memory(),
                    Default::default(),
                );
            scheduler.drain_browser_events().await;
            scheduler
                .execute_internal_protocol_message(
                    &mut receivers,
                    json!({
                        "id": 1, "method": "Browser.setDownloadBehavior", "params": {
                            "behavior": "allow", "downloadPath": directory.0, "eventsEnabled": true
                        }
                    }),
                )
                .await
                .unwrap_or_else(|failure| {
                    panic!("download subscription failed: {:?}", failure.into_parts().1)
                });
            let (chunks, body) = tokio::sync::mpsc::unbounded_channel();
            let (finish, finished) = tokio::sync::oneshot::channel();
            let url = url::Url::parse("https://native-download.test/report.txt").unwrap();
            let response = moli_fetch::StreamingRawResponse::new(
                url.clone(),
                200,
                Vec::new(),
                None,
                Vec::new(),
                false,
                Vec::new(),
                body,
                moli_fetch::FetchCancelHandle::new(),
                finished,
            );
            let mut observation = context
                .start_download_response(
                    contents,
                    &DownloadPolicy {
                        behavior: DownloadBehavior::Allow,
                        download_path: Some(directory.0.to_string_lossy().into_owned()),
                    },
                    url,
                    Vec::new(),
                    DownloadBody::Streaming(Box::new(response)),
                )
                .unwrap()
                .unwrap();
            let mut messages = scheduler.drain_browser_events().await.into_messages();
            let begin = messages
                .iter()
                .find(|message| message["method"] == "Browser.downloadWillBegin")
                .unwrap();
            assert_eq!(begin["params"]["guid"], observation.guid());
            let frame = begin["params"]["frameId"].clone();
            if !retire_context {
                browser
                    .close_web_contents(contents)
                    .unwrap()
                    .close_async()
                    .await;
            }
            chunks.send(b"native download".to_vec()).unwrap();
            drop(chunks);
            if retire_context {
                while observation.snapshot().received_bytes != 15 {
                    observation.next_update().await.unwrap();
                }
                assert!(context.remove().unwrap());
                drop(finish);
            } else {
                finish.send(Ok(())).unwrap();
            }
            while observation.snapshot().state == DownloadState::Active {
                observation.next_update().await.unwrap();
            }
            let terminal_state = if retire_context {
                "canceled"
            } else {
                "completed"
            };
            assert_eq!(
                observation.snapshot().state == DownloadState::Canceled,
                retire_context
            );
            if lagged {
                for _ in 0..130 {
                    let transient = browser
                        .create_context(
                            BrowserContextStoragePartitionHandles::memory(),
                            StoragePartitionKind::Ephemeral,
                            None,
                            None,
                        )
                        .unwrap();
                    assert!(transient.remove().unwrap());
                }
            }
            let native = browser.subscribe().unwrap().0;
            messages.extend(scheduler.drain_browser_events().await.into_messages());
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message["method"] == "Browser.downloadWillBegin")
                    .count(),
                1,
                "{messages:?}"
            );
            let completed = messages
                .iter()
                .filter(|message| {
                    message["method"] == "Browser.downloadProgress"
                        && message["params"]["state"] == terminal_state
                })
                .collect::<Vec<_>>();
            assert_eq!(completed.len(), 1, "{messages:?}");
            assert_eq!(completed[0]["params"]["guid"], observation.guid());
            assert_eq!(completed[0]["params"]["receivedBytes"], 15);
            assert!(!frame.is_null());
            if retire_context {
                assert!(std::fs::read_dir(&directory.0).unwrap().next().is_none());
            } else {
                assert_eq!(
                    std::fs::read(directory.0.join("report.txt")).unwrap(),
                    b"native download"
                );
            }
            let replay = scheduler
                .conn
                .project_browser_snapshot(native.clone())
                .await;
            assert!(replay.into_iter().all(|event| {
                !event.into_protocol_message()["method"]
                    .as_str()
                    .is_some_and(|method| method.starts_with("Browser.download"))
            }));
            assert_eq!(
                browser.subscribe().unwrap().0,
                native,
                "projection cannot mutate Browser downloads"
            );
            service.shutdown();
        }
    }

    #[tokio::test]
    async fn native_activation_projects_once_and_recovers_current_selection_after_real_lag() {
        use moli_core::browser::{
            BrowserContextStoragePartitionHandles, StoragePartitionKind, WebContentsCreation,
        };
        use moli_protocol::{CdpTargetHostLifecycleDelta, CdpTargetHostLifecycleObserver};
        for lagged in [false, true] {
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
            let (first, _) = context
                .create_web_contents(WebContentsCreation::with_initial_document(
                    "about:blank#activation-first".into(),
                    None,
                    None,
                ))
                .unwrap();
            let (second, _) = context
                .create_web_contents(WebContentsCreation::with_initial_document(
                    "about:blank#activation-second".into(),
                    None,
                    None,
                ))
                .unwrap();
            context
                .activate_web_contents(first)
                .unwrap()
                .wait()
                .await
                .unwrap();
            let changes = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
            let observed = changes.clone();
            let (mut scheduler, _) = CdpScheduler::new_with_initial_state_runtime_config(
                browser.clone(),
                CdpInitialStoragePartition::memory(),
                Default::default(),
            );
            scheduler
                .conn
                .set_target_host_lifecycle_observer(CdpTargetHostLifecycleObserver::new(
                    move |delta| observed.lock().push(delta),
                ));
            scheduler.drain_browser_events().await;
            let second_targets = std::mem::take(&mut *changes.lock())
                .into_iter()
                .filter_map(|change| match change {
                    CdpTargetHostLifecycleDelta::Created(info)
                        if info.url == "about:blank#activation-second" =>
                    {
                        info.target_id.map(|id| id.into_string())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(second_targets.len(), 2, "Page and Tab directory entries");
            let stale = context
                .activate_web_contents(second)
                .unwrap()
                .wait()
                .await
                .unwrap();
            context
                .activate_web_contents(first)
                .unwrap()
                .wait()
                .await
                .unwrap();
            let current = context
                .activate_web_contents(second)
                .unwrap()
                .wait()
                .await
                .unwrap();
            if lagged {
                for _ in 0..130 {
                    let transient = browser
                        .create_context(
                            BrowserContextStoragePartitionHandles::memory(),
                            StoragePartitionKind::Ephemeral,
                            None,
                            None,
                        )
                        .unwrap();
                    assert!(transient.remove().unwrap());
                }
            }
            let native = browser.subscribe().unwrap().0;
            scheduler.drain_browser_events().await;
            let activated = std::mem::take(&mut *changes.lock())
                .into_iter()
                .filter_map(|change| match change {
                    CdpTargetHostLifecycleDelta::Activated { target_id } => Some(target_id),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(activated.len(), 2, "lagged={lagged}: {activated:?}");
            assert!(
                second_targets
                    .iter()
                    .all(|target| activated.contains(target))
            );
            assert!(
                scheduler
                    .conn
                    .project_browser_web_contents_activation(stale)
                    .is_empty()
            );
            assert!(
                scheduler
                    .conn
                    .project_browser_web_contents_activation(current)
                    .is_empty()
            );
            assert!(
                changes.lock().is_empty(),
                "replayed occurrences must be inert"
            );
            assert_eq!(
                browser.subscribe().unwrap().0,
                native,
                "projection must not mutate native selection"
            );
            assert_eq!(context.selected_web_contents_handle(), Some(second));
            service.shutdown();
        }
    }

    #[tokio::test]
    async fn native_created_pages_are_adopted_at_start_live_and_after_real_lag() {
        use moli_core::browser::{
            BrowserContextStoragePartitionHandles, StoragePartitionKind, WebContentsCreation,
        };
        for phase in ["initial", "live", "lagged"] {
            let service = BrowserService::start().unwrap();
            let browser = service.handle();
            let create = || {
                let context = browser
                    .create_context(
                        BrowserContextStoragePartitionHandles::memory(),
                        StoragePartitionKind::Ephemeral,
                        None,
                        None,
                    )
                    .unwrap();
                let (first, _) = context
                    .create_web_contents(WebContentsCreation::with_initial_document(
                        "about:blank#native-first".into(),
                        None,
                        None,
                    ))
                    .unwrap();
                let (second, _) = context
                    .create_web_contents(WebContentsCreation::with_initial_document(
                        "about:blank#native-second".into(),
                        None,
                        None,
                    ))
                    .unwrap();
                assert!(context.select_web_contents(second.id()));
                (context, first, second)
            };
            let existing = (phase == "initial").then(create);
            let (mut scheduler, mut receivers) =
                CdpScheduler::new_with_deferred_default_target_runtime(
                    browser.clone(),
                    CdpInitialStoragePartition::memory(),
                    Default::default(),
                    None,
                );
            let (context, first, second) = existing.unwrap_or_else(create);
            let (closed, _) = context.create_web_contents(Default::default()).unwrap();
            browser
                .close_web_contents(closed)
                .unwrap()
                .close_async()
                .await;
            if phase == "lagged" {
                // Overflow the actual bounded event stream, not a forged Lagged input.
                for _ in 0..130 {
                    let transient = browser
                        .create_context(
                            BrowserContextStoragePartitionHandles::memory(),
                            StoragePartitionKind::Ephemeral,
                            None,
                            None,
                        )
                        .unwrap();
                    transient.remove().unwrap();
                }
            }
            let native = browser.subscribe().unwrap().0;
            scheduler.drain_browser_events().await;
            let projected = scheduler.conn.projected_web_contents();
            assert!(
                projected.contains(&first),
                "{phase}: first native Page was not adopted"
            );
            assert!(
                projected.contains(&second),
                "{phase}: second native Page was not adopted"
            );
            assert!(
                !projected.contains(&closed),
                "{phase}: resurrected closed Page"
            );
            assert_eq!(
                projected.len(),
                2,
                "{phase}: duplicate or placeholder materialization"
            );
            assert_eq!(context.selected_web_contents_handle(), Some(second));
            assert_eq!(
                browser.subscribe().unwrap().0,
                native,
                "adoption must not recreate Browser objects"
            );
            let mut listed = Vec::new();
            for id in [1, 2] {
                let output = scheduler
                    .execute_internal_protocol_message(
                        &mut receivers,
                        serde_json::json!({"id":id, "method":"Target.getTargets"}),
                    )
                    .await
                    .unwrap_or_else(|failure| panic!("{:?}", failure.into_parts().1))
                    .into_messages();
                let targets = output.iter().find(|message| message["id"] == id).unwrap()["result"]
                    ["targetInfos"]
                    .as_array()
                    .unwrap();
                let native_ids = targets
                    .iter()
                    .filter(|target| {
                        target["type"] == "page"
                            && target["url"]
                                .as_str()
                                .is_some_and(|url| url.contains("#native-"))
                    })
                    .map(|target| target["targetId"].clone())
                    .collect::<Vec<_>>();
                assert_eq!(native_ids.len(), 2, "{phase}: {targets:?}");
                listed.push(native_ids);
            }
            assert_eq!(
                listed[0], listed[1],
                "{phase}: discovery changed native Target IDs"
            );
            assert_eq!(scheduler.conn.projected_web_contents().len(), 2);
            assert_eq!(browser.subscribe().unwrap().0, native);
            service.shutdown();
        }
    }

    #[tokio::test]
    async fn native_web_contents_close_and_lag_recovery_retire_only_exact_projections() {
        for lagged in [false, true] {
            let service = BrowserService::start().unwrap();
            let browser = service.handle();
            let (mut scheduler, mut receivers) =
                CdpScheduler::new_with_initial_state_runtime_config(
                    browser.clone(),
                    CdpInitialStoragePartition::memory(),
                    Default::default(),
                );
            let created = scheduler.execute_internal_protocol_message(&mut receivers, serde_json::json!({
                "id": 1, "method": "Target.setDiscoverTargets", "params": {"discover": true},
            })).await.unwrap_or_else(|failure| panic!("{:?}", failure.into_parts().1)).into_messages();
            assert!(
                created
                    .iter()
                    .any(|message| message["method"] == "Target.targetCreated")
            );
            let handle = scheduler.conn.projected_web_contents()[0];
            // A live occurrence, or a forged stale record, cannot retire a live projection.
            assert!(
                scheduler
                    .conn
                    .project_closed_web_contents(
                        handle,
                        None,
                        browser.subscribe().unwrap().0.sequence
                    )
                    .await
                    .is_empty()
            );
            browser
                .close_web_contents(handle)
                .unwrap()
                .close_async()
                .await;
            let output = if lagged {
                scheduler
                    .handle_browser_event(Err(RecvError::Lagged(1)))
                    .await
            } else {
                scheduler.drain_browser_events().await
            }
            .into_messages();
            assert!(scheduler.conn.projected_web_contents().is_empty());
            assert_eq!(scheduler.conn.browser_contexts().count(), 1);
            assert!(browser.contains_context(handle.context()));
            assert_eq!(
                output
                    .iter()
                    .filter(|message| message["method"] == "Target.targetDestroyed"
                        && message["params"]["targetId"] == scheduler.conn.default_target_id())
                    .count(),
                1
            );
            assert!(
                scheduler
                    .conn
                    .project_closed_web_contents(
                        handle,
                        None,
                        browser.subscribe().unwrap().0.sequence
                    )
                    .await
                    .is_empty()
            );
            assert!(
                scheduler
                    .drain_browser_events()
                    .await
                    .into_messages()
                    .is_empty()
            );
            service.shutdown();
        }
    }

    #[tokio::test]
    async fn browser_shutdown_retires_all_context_sessions_before_closed_observation() {
        let service = BrowserService::start().unwrap();
        let (mut scheduler, mut receivers) = CdpScheduler::new_with_initial_state_runtime_config(
            service.handle(),
            CdpInitialStoragePartition::memory(),
            Default::default(),
        );
        for command in [
            serde_json::json!({"id": 1, "method": "Target.attachToTarget", "params": {
                "targetId": scheduler.conn.default_target_id(), "flatten": true,
            }}),
            serde_json::json!({"id": 2, "method": "Target.createBrowserContext", "params": {}}),
        ] {
            let id = command["id"].clone();
            let output = scheduler
                .execute_internal_protocol_message(&mut receivers, command)
                .await
                .unwrap_or_else(|failure| panic!("setup failed: {:?}", failure.into_parts().1))
                .into_messages();
            assert!(
                output
                    .iter()
                    .any(|message| message["id"] == id && message.get("result").is_some()),
                "{output:?}"
            );
        }
        assert_eq!(scheduler.conn.browser_contexts().count(), 2);
        service.shutdown();
        let output = scheduler.drain_browser_events().await.into_messages();
        assert!(scheduler.is_browser_closed());
        assert!(scheduler.conn.browser_contexts().next().is_none());
        assert_eq!(
            output
                .iter()
                .filter(|message| message["method"] == "Target.detachedFromTarget")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn lagged_browser_subscription_retires_only_missing_context_projections() {
        use moli_core::browser::{
            BrowserContextStoragePartitionHandles, StoragePartitionKind, WebContentsCreation,
        };
        let service = BrowserService::start().unwrap();
        let browser = service.handle();
        let (mut scheduler, mut receivers) = CdpScheduler::new_with_initial_state_runtime_config(
            browser.clone(),
            CdpInitialStoragePartition::memory(),
            Default::default(),
        );
        let first_id = scheduler
            .conn
            .browser_contexts()
            .next()
            .unwrap()
            .browser_context_id();
        let first_target = scheduler.conn.default_target_id().to_owned();
        let peer_context = browser
            .create_context(
                BrowserContextStoragePartitionHandles::memory(),
                StoragePartitionKind::Ephemeral,
                None,
                None,
            )
            .unwrap();
        let peer_id = peer_context.id();
        let (peer_page, _) = peer_context
            .create_web_contents(WebContentsCreation::with_initial_document(
                "about:blank#lag-survivor".into(),
                None,
                None,
            ))
            .unwrap();
        assert!(peer_context.select_web_contents(peer_page.id()));
        // This fixture drives the scheduler directly, without the production
        // actor's Browser-event drain before frontend command admission.
        assert!(
            scheduler
                .drain_browser_events()
                .await
                .into_messages()
                .is_empty()
        );
        let created = scheduler
            .execute_internal_protocol_message(
                &mut receivers,
                serde_json::json!({
                    "id": 1, "method": "Target.setDiscoverTargets", "params": {"discover": true},
                }),
            )
            .await
            .unwrap_or_else(|failure| panic!("discovery failed: {:?}", failure.into_parts().1))
            .into_messages();
        let peer_target = created.iter().find(|message| {
            message["method"] == "Target.targetCreated"
                && message["params"]["targetInfo"]["type"] == "page"
                && message["params"]["targetInfo"]["url"] == "about:blank#lag-survivor"
        }).expect("shared DevTools owner must discover the native peer Page")
            ["params"]["targetInfo"]["targetId"].as_str().unwrap().to_owned();
        assert_eq!(scheduler.conn.browser_contexts().count(), 2);

        // An independent Browser observer shares neutral events, not a second
        // CDP scheduler or a competing renderer transport for the same Context.
        let (_, mut native_peer) = browser.subscribe().unwrap();
        assert!(browser.remove_context(first_id).unwrap());
        let output = scheduler
            .handle_browser_event(Err(RecvError::Lagged(1)))
            .await
            .into_messages();
        assert_eq!(
            scheduler
                .conn
                .browser_contexts()
                .map(|context| context.browser_context_id())
                .collect::<Vec<_>>(),
            [peer_id]
        );
        assert_eq!(scheduler.conn.projected_web_contents(), [peer_page]);
        assert_eq!(
            browser.subscribe().unwrap().0.selected_web_contents,
            [peer_page]
        );
        assert!(!scheduler.is_browser_closed());
        let destroyed = output
            .iter()
            .filter(|message| message["method"] == "Target.targetDestroyed")
            .map(|message| message["params"]["targetId"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(destroyed, [first_target.as_str()]);
        assert!(
            scheduler
                .drain_browser_events()
                .await
                .into_messages()
                .is_empty()
        );
        let first_disposal = native_peer.try_recv().unwrap();
        assert_eq!(
            first_disposal.event,
            BrowserEvent::ContextDisposed(first_id)
        );

        // Recovery retains the live projection and resumes receiving future
        // events. Both observers now see the exact surviving Context retire.
        assert!(browser.remove_context(peer_id).unwrap());
        let output = scheduler.drain_browser_events().await.into_messages();
        let destroyed = output
            .iter()
            .filter(|message| message["method"] == "Target.targetDestroyed")
            .map(|message| message["params"]["targetId"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(destroyed, [peer_target.as_str()]);
        assert!(scheduler.conn.browser_contexts().next().is_none());
        let second_disposal = native_peer.try_recv().unwrap();
        assert_eq!(
            second_disposal.event,
            BrowserEvent::ContextDisposed(peer_id)
        );
        assert!(second_disposal.sequence > first_disposal.sequence);
        assert!(
            scheduler
                .drain_browser_events()
                .await
                .into_messages()
                .is_empty()
        );
        service.shutdown();
        scheduler.drain_browser_events().await;
        assert!(scheduler.is_browser_closed());
        assert!(matches!(
            native_peer.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Closed)
        ));
    }
}
