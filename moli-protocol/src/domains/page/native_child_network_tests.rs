use super::*;
use crate::conn::{RendererPageResidenceIdentity, TargetPageResidenceIdentity};
use moli_core::browser::{
    BrowserContextStoragePartitionHandles, BrowserNavigationOutcome, BrowserService,
    NavigationRequestLoadPolicy, StoragePartitionKind, web_contents::NavigationRequestInterception,
};
use moli_core::page::RendererNetworkOutputItem;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn native_child_network_receipts_recover_once_and_reject_foreign_or_replaced_roots() {
    for (status, headers, commits_child) in [
        (200, "Content-Type: text/html", true),
        (204, "Content-Type: text/html", false),
        (205, "Content-Type: text/html", false),
        (200, "Content-Type: application/octet-stream", false),
        (
            200,
            "Content-Type: text/html\r\nContent-Disposition: attachment",
            false,
        ),
        (
            200,
            "Content-Type: text/html\r\nContent-Security-Policy: frame-ancestors 'none'",
            false,
        ),
    ] {
        assert_native_child_network_receipt(Some((status, headers)), commits_child).await;
    }
}

#[tokio::test]
async fn native_child_network_failure_receipts_recover_once_and_reject_foreign_or_replaced_roots() {
    assert_native_child_network_receipt(None, false).await;
}

async fn assert_native_child_network_receipt(response: Option<(u16, &str)>, commits_child: bool) {
    for recover in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let child_url = format!("http://{}/child", listener.local_addr().unwrap());
        let server_response = response.map(|(status, headers)| (status, headers.to_owned()));
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
            }
            assert!(String::from_utf8_lossy(&request).starts_with("GET /child "));
            let Some((status, headers)) = server_response else {
                return;
            };
            let body = if matches!(status, 204 | 205) {
                ""
            } else {
                "<!doctype html><p>native child receipt</p>"
            };
            stream.write_all(format!("HTTP/1.1 {status} OK\r\n{headers}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
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
        context.bind_page_navigation_engines(Default::default());
        let (sender, mut output) = moli_core::renderer_output_transport_channel();
        context
            .set_renderer_output_transport_sender(sender)
            .unwrap();
        let (contents, _) = context.create_web_contents(Default::default()).unwrap();
        let navigation = context
            .navigate_document(
                contents,
                NavigationRequestInterception::new(
                    format!("data:text/html,<iframe src='{child_url}'></iframe>")
                        .parse()
                        .unwrap(),
                    "GET".into(),
                    None,
                    Vec::new().into(),
                    NavigationRequestLoadPolicy::BrowserInitiated,
                ),
            )
            .unwrap();
        let BrowserNavigationOutcome::Document(commit) = navigation.wait().await.unwrap() else {
            panic!("native parent must commit");
        };
        let (receipts, load) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut receipts = Vec::new();
            let mut handle = None;
            let mut frame_id = None;
            let mut terminal = false;
            let mut load = None;
            loop {
                let moli_core::RendererOutputTransportMessage::Publication(publication) = output.recv().await.unwrap() else { continue; };
                for record in publication.into_records() {
                    match record.into_parts().1 {
                        moli_core::RendererOutputItem::OwnerAction(moli_core::RendererOwnerAction::ChildFrameLoad { event, .. })
                            if Some(event.frame_id.as_str()) == frame_id.as_deref() => { load = Some(event); }
                        moli_core::RendererOutputItem::Observation(moli_core::RendererProtocolObservation::Network(network)) => {
                            let committed = network.committed().await.unwrap();
                            let RendererNetworkOutputItem::Resource(item) = &committed.occurrence().item else { continue; };
                            let belongs = match item.as_ref() {
                                moli_core::page::ScriptNetworkOutputItem::SubresourceRequestStarted(request) if request.url().as_str() == child_url => {
                                    handle = Some(request.handle()); frame_id = request.frame_id().map(str::to_owned); true
                                }
                                moli_core::page::ScriptNetworkOutputItem::SubresourceResponseStarted(response) => Some(response.handle()) == handle,
                                moli_core::page::ScriptNetworkOutputItem::SubresourceDataReceived(data) => Some(data.handle()) == handle,
                                moli_core::page::ScriptNetworkOutputItem::SubresourceBodyFinished(body) if Some(body.handle()) == handle => { terminal = true; true }
                                _ => false,
                            };
                            if belongs { receipts.push(committed); }
                        }
                        _ => {}
                    }
                }
                if terminal && (load.is_some() || !commits_child) { break (receipts, load); }
            }
        }).await.expect("the original child request must publish its stages independently of child Load");
        let committed = receipts.last().unwrap();
        let RendererNetworkOutputItem::Resource(started) = &receipts[0].occurrence().item else {
            unreachable!()
        };
        let moli_core::page::ScriptNetworkOutputItem::SubresourceRequestStarted(started) =
            started.as_ref()
        else {
            panic!("start first")
        };
        assert_eq!(
            started.resource_type(),
            moli_core::page::SubresourceResourceType::Document
        );
        let loader_id = started.navigation_loader_id().unwrap().to_owned();
        let frame_id = started.frame_id().unwrap().to_owned();
        let RendererNetworkOutputItem::Resource(terminal) = &committed.occurrence().item else {
            unreachable!()
        };
        let moli_core::page::ScriptNetworkOutputItem::SubresourceBodyFinished(terminal) =
            terminal.as_ref()
        else {
            panic!("terminal last")
        };
        let native_error = match terminal.result() {
            moli_core::page::SubresourceBodyFinishedResult::Ready(_) => None,
            moli_core::page::SubresourceBodyFinishedResult::Failed(error) => {
                assert!(!error.is_empty());
                Some(error.clone())
            }
            other => panic!("unexpected response: {other:?}"),
        };
        let heads = receipts
            .iter()
            .filter_map(|receipt| match &receipt.occurrence().item {
                RendererNetworkOutputItem::Resource(item) => match item.as_ref() {
                    moli_core::page::ScriptNetworkOutputItem::SubresourceResponseStarted(head) => {
                        Some(head.status())
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            heads,
            response
                .map(|(status, _)| status)
                .into_iter()
                .collect::<Vec<_>>()
        );
        assert_eq!(load.is_some(), commits_child, "response={response:?}");
        let source = RendererPageResidenceIdentity::from_parts(
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
        let (peer, _) = context.create_web_contents(Default::default()).unwrap();
        let peer_navigation = context
            .navigate_document(
                peer,
                NavigationRequestInterception::new(
                    "data:text/html,peer".parse().unwrap(),
                    "GET".into(),
                    None,
                    Vec::new().into(),
                    NavigationRequestLoadPolicy::BrowserInitiated,
                ),
            )
            .unwrap();
        let BrowserNavigationOutcome::Document(peer_commit) = peer_navigation.wait().await.unwrap()
        else {
            panic!("peer must commit");
        };
        let peer_renderer = context
            .document_renderer_residence(peer_commit.document)
            .unwrap();
        assert_eq!(source.page_id(), peer_renderer.page_id());
        assert_ne!(
            source.owner_local_host_id(),
            peer_renderer.owner_local_host_id()
        );
        context.select_web_contents(contents.id());
        let snapshot = browser.subscribe().unwrap().0;
        assert_eq!(snapshot.network_requests.iter().filter(|request| request.owner == moli_core::browser::NetworkOwner::Document(commit.document)
            && matches!(&request.state, moli_core::browser::NetworkRequestState::Completed { body, .. } if std::sync::Arc::ptr_eq(body, terminal))).count(), 1);
        let mut membership = snapshot.clone();
        membership.network_requests.clear();
        let mut conn = CdpConnection::new(
            browser.clone(),
            crate::CdpInitialStoragePartition::memory(),
            Default::default(),
        );
        conn.project_browser_snapshot(membership).await;
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
        let peer_owner = CommandOwnerScope::for_page_residence(&TargetPageResidenceIdentity::new(
            projection.id.clone(),
            Some(
                projection
                    .target_id_for_web_contents(peer.id())
                    .unwrap()
                    .to_owned(),
            ),
            peer_commit.document.id(),
        ));
        for (owner, source) in [
            (&owner, None),
            (&owner, Some(peer_renderer)),
            (&peer_owner, Some(source)),
        ] {
            for receipt in &receipts {
                assert!(
                    conn.ingest_browser_network_observation_for_owner(owner, source, receipt)
                        .is_none(),
                    "a receipt cannot bind through a colliding local Page ID"
                );
            }
        }
        conn.runtime_session_owner_slot_mut_for_owner(&owner)
            .unwrap()
            .enable_primary_network_events();
        let mut events = if recover {
            conn.project_browser_snapshot(snapshot.clone()).await
        } else {
            Vec::new()
        };
        for _ in 0..2 {
            for receipt in &receipts {
                if let Some(mut delivery) =
                    conn.ingest_browser_network_observation_for_owner(&owner, Some(source), receipt)
                {
                    crate::domains::network::emit_prepared_renderer_network_live_background_events(
                        &mut conn,
                        &mut events,
                        &owner,
                        &mut delivery,
                    );
                }
            }
        }
        if let Some(load) = load {
            let prepared = PagePreparedOutputs::from_renderer_child_frame_load(
                &conn,
                &owner,
                committed.occurrence().source.document().unwrap().1,
                load,
            );
            assert_eq!(prepared.child_frame_activities.len(), 1);
            for activity in prepared.child_frame_activities {
                emit_prepared_child_frame_activity(&mut conn, &mut events, activity, None).await;
            }
        }
        assert_eq!(
            events
                .iter()
                .filter(|event| event.protocol_method() == Some("Network.requestWillBeSent"))
                .count(),
            1,
            "response={response:?}, recover={recover}"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.protocol_method() == Some("Network.loadingFinished"))
                .count(),
            usize::from(response.is_some()),
            "response={response:?}, recover={recover}"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.protocol_method() == Some("Network.loadingFailed"))
                .count(),
            usize::from(response.is_none()),
            "rejecting a Document is not a network transfer failure"
        );
        let messages = events
            .into_iter()
            .map(BackgroundProtocolEvent::into_protocol_message)
            .collect::<Vec<_>>();
        let request = messages
            .iter()
            .position(|message| message["method"] == "Network.requestWillBeSent")
            .unwrap();
        assert_eq!(messages[request]["params"]["request"]["url"], child_url);
        assert_eq!(messages[request]["params"]["loaderId"], loader_id);
        assert_eq!(messages[request]["params"]["frameId"], frame_id);
        if let Some(error) = native_error {
            let failed = messages
                .iter()
                .position(|message| message["method"] == "Network.loadingFailed")
                .unwrap();
            assert!(request < failed);
            assert_eq!(
                messages[failed]["params"]["requestId"],
                messages[request]["params"]["requestId"]
            );
            assert_eq!(messages[failed]["params"]["errorText"], error);
            assert_eq!(messages[failed]["params"]["canceled"], false);
            assert!(
                !messages
                    .iter()
                    .any(|message| message["method"] == "Network.responseReceived")
            );
            let body = conn
                .network_agent_for_owner(&owner)
                .unwrap()
                .captured_response_body(&loader_id)
                .expect("the failed request is known, not an unknown ID");
            assert_eq!(
                body.body_bytes_limited(1024).unwrap_err().to_string(),
                "No data found for resource with given identifier"
            );
        }
        conn.install_navigation_fixture_for_owner_for_test("data:text/html,replaced", &owner)
            .await;
        for receipt in &receipts {
            assert!(
                conn.ingest_browser_network_observation_for_owner(&owner, Some(source), receipt)
                    .is_none()
            );
        }
        assert!(
            !conn
                .project_browser_snapshot(snapshot)
                .await
                .iter()
                .any(|event| event
                    .protocol_method()
                    .is_some_and(|method| method.starts_with("Network.")))
        );
        server.await.unwrap();
        service.shutdown();
    }
}
