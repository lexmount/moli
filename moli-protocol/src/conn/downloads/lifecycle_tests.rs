use std::path::PathBuf;

use moli_core::browser::{DownloadBehavior, DownloadPolicy};
use moli_fetch::{FetchCancelHandle, StreamingRawResponse};
use tokio::sync::{mpsc, oneshot};

use super::*;
use crate::conn::BrowserContext;

struct TestDirectory(PathBuf);
impl TestDirectory {
    fn new() -> Self {
        let mut nonce = [0; 16];
        moli_crypto::fill_secure_random(&mut nonce).unwrap();
        let path = std::env::temp_dir().join(format!("moli-download-projection-{nonce:02x?}"));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture() -> (TestDirectory, CdpConnection) {
    let directory = TestDirectory::new();
    let mut conn = CdpConnection::new();
    let mut source = BrowserContext::new("CTX-source".into());
    source.set_active_target_id("TID-source");
    source.attach_active_session("SID-source");
    conn.install_browser_context_fixture_for_test(source);
    conn.set_browser_download_events_enabled_for_session(None, true);
    conn.configure_download_policy(
        Some("CTX-source"),
        DownloadPolicy {
            behavior: DownloadBehavior::AllowAndName,
            download_path: Some(directory.0.to_str().unwrap().to_owned()),
        },
        Some(true),
    )
    .unwrap();
    (directory, conn)
}

fn projection(conn: &mut CdpConnection, body: DownloadBody) -> DownloadProjection {
    let owner = CommandOwnerScope::for_session("SID-source");
    let web_contents = conn.browser_web_contents_for_owner(&owner).unwrap();
    let (policy, automation_events_enabled) = conn
        .download_configuration_for_browser_context(web_contents.context())
        .unwrap();
    let frame_id = conn
        .browser_context_by_browser_id(web_contents.context())
        .unwrap()
        .download_frame_id_for_web_contents(web_contents)
        .unwrap()
        .to_owned();
    let event_route = conn.download_event_route(&owner, automation_events_enabled);
    let observation = conn
        .browser_context_by_browser_id_mut(web_contents.context())
        .unwrap()
        .start_download_response(
            web_contents,
            &policy,
            Url::parse("https://source.test/report.txt").unwrap(),
            Vec::new(),
            body,
        )
        .unwrap()
        .unwrap();
    DownloadProjection {
        frame_id,
        event_route,
        observation,
        started: false,
    }
}

async fn wait_for(
    observation: &mut DownloadObservation,
    predicate: impl Fn(&DownloadSnapshot) -> bool,
) -> DownloadSnapshot {
    let mut snapshot = observation.snapshot();
    while !predicate(&snapshot) {
        snapshot = observation
            .next_update()
            .await
            .expect("Browser task must publish expected state");
    }
    snapshot
}

async fn terminal(observation: &mut DownloadObservation) -> DownloadSnapshot {
    wait_for(observation, |snapshot| {
        snapshot.state != DownloadState::Active
    })
    .await
}

async fn transfer_during_response_flush(abandon: bool) {
    let (_directory, mut conn) = fixture();
    let (sender, mut events) = mpsc::unbounded_channel();
    conn.set_background_event_sender(sender);
    let (permit, flush) = conn.begin_command_response_flush_permit();
    let mut command_context = CommandDispatchContext::new(flush);
    let mut permit = Some(permit);
    let projection = projection(
        &mut conn,
        DownloadBody::Buffered(b"without frontend".to_vec()),
    );
    let mut monitor = projection.observation.clone();
    let mut inline = Vec::new();
    conn.observe_download(projection, &mut inline, true, &mut command_context)
        .await;
    assert!(inline.is_empty());
    let initial = command_context.take_post_response_events();
    assert_eq!(
        initial[0].download_will_begin_frame_id(),
        Some("TID-source")
    );
    if abandon {
        drop(permit.take());
    }
    assert!(matches!(
        terminal(&mut monitor).await.state,
        DownloadState::Completed { .. }
    ));
    assert!(
        events.try_recv().is_err(),
        "download must finish while frontend observation remains gated"
    );
    assert_eq!(
        conn.start_open_download_as_stream(monitor.guid())
            .unwrap()
            .await
            .unwrap()
            .unwrap(),
        b"without frontend"
    );
    assert_eq!(
        conn.cancel_download(monitor.guid()),
        Err("Download item is no longer active".into())
    );
    if let Some(permit) = permit {
        permit.finish();
        loop {
            let message = events.recv().await.unwrap().into_protocol_message();
            if message["params"]["state"] == "completed" {
                assert_eq!(message["params"]["guid"], monitor.guid());
                assert_eq!(message["params"]["receivedBytes"], 16);
                break;
            }
        }
    }
}

#[tokio::test]
async fn frontend_response_flush_only_gates_download_observation() {
    transfer_during_response_flush(false).await;
}

#[tokio::test]
async fn abandoned_frontend_response_does_not_abandon_download_execution() {
    transfer_during_response_flush(true).await;
}

#[tokio::test]
async fn admitted_download_freezes_context_policy_and_observation_separately() {
    let (directory, mut conn) = fixture();
    let projection = projection(&mut conn, DownloadBody::Buffered(b"snapshot".to_vec()));
    let mut monitor = projection.observation.clone();
    conn.configure_download_policy(
        Some("CTX-source"),
        DownloadPolicy {
            behavior: DownloadBehavior::Deny,
            download_path: None,
        },
        Some(false),
    )
    .unwrap();
    let source = conn
        .browser_context
        .replace(BrowserContext::new("CTX-foreground".into()))
        .unwrap();
    conn.push_inactive_browser_context_fixture_for_test(source);
    assert_eq!(projection.frame_id, "TID-source");
    assert!(projection.event_route.automation_events_enabled);
    assert!(!conn.automation_download_events_enabled_for_context(Some("CTX-source")));
    let mut out = Vec::new();
    conn.observe_download(
        projection,
        &mut out,
        false,
        &mut CommandDispatchContext::default(),
    )
    .await;
    assert_eq!(
        terminal(&mut monitor).await.state,
        DownloadState::Completed {
            artifact_path: directory.0.join(monitor.guid())
        }
    );
    assert_eq!(out[0].download_will_begin_frame_id(), Some("TID-source"));
    assert_eq!(
        conn.start_open_download_as_stream(monitor.guid())
            .unwrap()
            .await
            .unwrap()
            .unwrap(),
        b"snapshot"
    );
}

fn stream() -> (
    DownloadBody,
    mpsc::UnboundedSender<Vec<u8>>,
    oneshot::Sender<anyhow::Result<()>>,
    FetchCancelHandle,
) {
    let (chunks, receiver) = mpsc::unbounded_channel();
    let (completion, finished) = oneshot::channel();
    let cancel = FetchCancelHandle::new();
    let response = StreamingRawResponse::new(
        Url::parse("https://source.test/report.txt").unwrap(),
        200,
        Vec::new(),
        None,
        Vec::new(),
        false,
        Vec::new(),
        receiver,
        cancel.clone(),
        finished,
    );
    (
        DownloadBody::Streaming(Box::new(response)),
        chunks,
        completion,
        cancel,
    )
}

#[tokio::test]
async fn session_detach_does_not_cancel_the_context_download() {
    let (_directory, mut conn) = fixture();
    let (body, chunks, completion, cancel) = stream();
    let projection = projection(&mut conn, body);
    let mut monitor = projection.observation.clone();
    drop(projection);
    conn.detach_known_session_event_plan("TID-source", "SID-source", None, None);
    assert!(conn.session_route(Some("SID-source")).is_none());
    chunks.send(b"after detach".to_vec()).unwrap();
    drop(chunks);
    completion.send(Ok(())).unwrap();
    assert!(matches!(
        terminal(&mut monitor).await.state,
        DownloadState::Completed { .. }
    ));
    assert!(!cancel.is_cancelled());
    assert_eq!(
        conn.start_open_download_as_stream(monitor.guid())
            .unwrap()
            .await
            .unwrap()
            .unwrap(),
        b"after detach"
    );
}

#[tokio::test]
async fn retiring_context_cancels_download_and_new_same_wire_context_cannot_read_it() {
    let (directory, mut conn) = fixture();
    let (body, chunks, _completion, cancel) = stream();
    let projection = projection(&mut conn, body);
    let mut monitor = projection.observation.clone();
    drop(projection);
    chunks.send(b"partial".to_vec()).unwrap();
    wait_for(&mut monitor, |snapshot| snapshot.received_bytes == 7).await;
    assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 1);
    drop(
        conn.remove_browser_context_by_id_restoring_active_async("CTX-source", None)
            .await
            .unwrap(),
    );
    assert_eq!(terminal(&mut monitor).await.state, DownloadState::Canceled);
    assert!(cancel.is_cancelled());
    assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
    conn.insert_browser_context(BrowserContext::new("CTX-source".into()));
    assert_eq!(
        conn.cancel_download(monitor.guid()),
        Err("No download item found for the given GUID".into())
    );
    assert!(
        matches!(conn.start_open_download_as_stream(monitor.guid()), Err(error) if error == "No download item found for the given GUID")
    );
}

fn navigation_download_state(conn: &CdpConnection) -> NavigationDispatchState {
    use crate::conn::{
        NavigationRequestLoadPolicy, NavigationResultProjection,
        NavigationSourceDocumentSecurityContext,
    };

    let owner = CommandOwnerScope::for_session("SID-source");
    NavigationDispatchState {
        navigate_id: None,
        web_contents: conn.browser_web_contents_for_owner(&owner).unwrap(),
        owner,
        result_projection: NavigationResultProjection::Cdp(serde_json::json!({})),
        frame_id: "TID-source".into(),
        session_id: Some("SID-source".into()),
        request_id: None,
        loader_id: "LOADER-source".into(),
        request_announced: false,
        requested_url: Url::parse("https://source.test/report.txt").unwrap(),
        request_method: "GET".into(),
        request_body: None,
        request_body_bytes: None,
        request_headers: Vec::new(),
        request_load_policy: NavigationRequestLoadPolicy::DocumentInitiated,
        timestamp: 0.0,
        source_document_security: NavigationSourceDocumentSecurityContext::new(
            "https://source.test".into(),
            "Secure".into(),
        ),
    }
}

async fn replace_source_download_context(conn: &mut CdpConnection, directory: &TestDirectory) {
    drop(
        conn.remove_browser_context_by_id_restoring_active_async("CTX-source", None)
            .await
            .unwrap(),
    );
    let mut replacement = BrowserContext::new("CTX-source".into());
    replacement.set_active_target_id("TID-source");
    replacement.attach_active_session("SID-source");
    conn.install_browser_context_fixture_for_test(replacement);
    conn.configure_download_policy(
        Some("CTX-source"),
        DownloadPolicy {
            behavior: DownloadBehavior::AllowAndName,
            download_path: Some(directory.0.to_str().unwrap().to_owned()),
        },
        Some(true),
    )
    .unwrap();
}

#[tokio::test]
async fn navigation_download_uses_its_frozen_frame_after_session_detach_and_selection_change() {
    let (directory, mut conn) = fixture();
    let state = navigation_download_state(&conn);
    conn.detach_known_session_event_plan("TID-source", "SID-source", None, None);
    assert!(conn.target_owner_identity_for_owner(&state.owner).is_none());
    let source = conn
        .browser_context
        .replace(BrowserContext::new("CTX-foreground".into()))
        .unwrap();
    conn.push_inactive_browser_context_fixture_for_test(source);
    let mut out = Vec::new();
    conn.handle_navigation_download_response_async(
        &mut out,
        &state,
        state.requested_url.clone(),
        CompletedDownloadBodyArtifact::from_body(
            DownloadBody::Buffered(b"navigation".to_vec()),
            Vec::new(),
        ),
        &mut CommandDispatchContext::default(),
    )
    .await
    .unwrap();
    let begin = out
        .iter()
        .find_map(|event| {
            let message = event.clone().into_protocol_message();
            (message["method"] == "Browser.downloadWillBegin").then_some(message)
        })
        .expect("Browser observer must receive the detached navigation's download");
    let guid = begin["params"]["guid"].as_str().unwrap();
    assert_eq!(begin["params"]["frameId"], "TID-source");
    assert_eq!(
        std::fs::read(directory.0.join(guid)).unwrap(),
        b"navigation"
    );
    assert_eq!(
        conn.start_open_download_as_stream(guid)
            .unwrap()
            .await
            .unwrap()
            .unwrap(),
        b"navigation"
    );
    assert_eq!(conn.browser_context.as_ref().unwrap().id, "CTX-foreground");
}

#[tokio::test]
async fn navigation_download_does_not_enter_a_replacement_with_the_same_wire_identity() {
    let (directory, mut conn) = fixture();
    let state = navigation_download_state(&conn);
    replace_source_download_context(&mut conn, &directory).await;

    let mut out = Vec::new();
    conn.handle_navigation_download_response_async(
        &mut out,
        &state,
        state.requested_url.clone(),
        CompletedDownloadBodyArtifact::from_body(
            DownloadBody::Buffered(b"stale navigation".to_vec()),
            Vec::new(),
        ),
        &mut CommandDispatchContext::default(),
    )
    .await
    .unwrap();

    assert!(out.is_empty());
    assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
}

#[tokio::test]
async fn prepared_renderer_download_does_not_enter_a_replacement_web_contents() {
    let (directory, mut conn) = fixture();
    let owner = CommandOwnerScope::for_session("SID-source");
    let prepared = conn
        .prepare_download_activation_for_owner(
            &owner,
            RendererPendingDownloadActivation {
                url: "https://source.test/report.txt".to_owned(),
                suggested_filename: Some("report.txt".to_owned()),
                response: Some(moli_core::page::RendererPendingDownloadResponse {
                    final_url: "https://source.test/report.txt".to_owned(),
                    status: 200,
                    headers: Vec::new(),
                    body: b"stale renderer".to_vec(),
                }),
            },
        )
        .unwrap();
    replace_source_download_context(&mut conn, &directory).await;

    let mut out = Vec::new();
    conn.handle_prepared_download_activation_background_events_async(
        &mut out,
        prepared,
        &mut CommandDispatchContext::default(),
    )
    .await
    .unwrap();

    assert!(out.is_empty());
    assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
}
