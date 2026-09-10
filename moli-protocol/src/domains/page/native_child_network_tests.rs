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
        context.bind_page_navigation_engines(Default::default(), None);
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
                    Vec::new(),
                    NavigationRequestLoadPolicy::BrowserInitiated,
                ),
            )
            .unwrap();
        let BrowserNavigationOutcome::Document(commit) = navigation.wait().await.unwrap() else {
            panic!("native parent must commit");
        };
        let (committed, load) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let moli_core::RendererOutputTransportMessage::Publication(publication) = output.recv().await.unwrap() else { continue; };
                    for record in publication.into_records() {
                        let (committed, load) = match record.into_parts().1 {
                            moli_core::RendererOutputItem::OwnerAction(moli_core::RendererOwnerAction::ChildFrameLoad { event, network: Some(network), .. }) => (network.committed().await.unwrap(), Some(event)),
                            moli_core::RendererOutputItem::Observation(moli_core::RendererProtocolObservation::Network(network)) => (network.committed().await.unwrap(), None),
                            _ => continue,
                        };
                        if matches!(&committed.occurrence().item, RendererNetworkOutputItem::ChildDocument(response) if response.snapshot.request_url == child_url) {
                            return (committed, load);
                        }
                    }
                }
            }).await.expect("a completed child fetch retains its native receipt without DevTools");
        let RendererNetworkOutputItem::ChildDocument(activity) = &committed.occurrence().item
        else {
            unreachable!()
        };
        let native_result = activity.snapshot.response.as_ref();
        assert_eq!(
            native_result.map(|response| response.status).ok(),
            response.map(|(status, _)| status)
        );
        let native_error = native_result.err().cloned();
        if let Some(error) = &native_error {
            assert!(!error.is_empty());
        }
        let loader_id = activity.loader_id.clone();
        let frame_id = activity.frame_id.clone();
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
                    Vec::new(),
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
        assert_eq!(snapshot.network_requests.iter().filter(|request| request.owner == moli_core::browser::NetworkOwner::Document(commit.document) && matches!(&request.state, moli_core::browser::NetworkRequestState::ChildDocument(stored) if std::sync::Arc::ptr_eq(stored, activity))).count(), 1);
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
            assert!(
                PagePreparedOutputs::from_browser_child_document_network(
                    &conn, owner, source, &committed
                )
                .child_frame_activities
                .is_empty(),
                "a receipt cannot bind through a colliding local Page ID"
            );
        }
        conn.runtime_session_owner_slot_mut_for_owner(&owner)
            .unwrap()
            .enable_primary_network_events();
        let mut events = if recover {
            conn.project_browser_snapshot(snapshot.clone()).await
        } else {
            Vec::new()
        };
        let prepared = match load {
            Some(load) => PagePreparedOutputs::from_renderer_child_frame_load(
                &conn,
                &owner,
                committed.occurrence().source.document().unwrap().1,
                load,
                Some(source),
                Some(&committed),
            ),
            None => PagePreparedOutputs::from_browser_child_document_network(
                &conn,
                &owner,
                Some(source),
                &committed,
            ),
        };
        assert_eq!(prepared.child_frame_activities.len(), 1);
        for activity in prepared.child_frame_activities {
            emit_prepared_child_frame_activity(&mut conn, &mut events, activity, None).await;
        }
        for activity in PagePreparedOutputs::from_browser_child_document_network(
            &conn,
            &owner,
            Some(source),
            &committed,
        )
        .child_frame_activities
        {
            emit_prepared_child_frame_activity(&mut conn, &mut events, activity, None).await;
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
        assert!(
            PagePreparedOutputs::from_browser_child_document_network(
                &conn,
                &owner,
                Some(source),
                &committed
            )
            .child_frame_activities
            .is_empty()
        );
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
