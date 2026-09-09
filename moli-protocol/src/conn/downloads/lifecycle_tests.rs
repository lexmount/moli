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
    let mut conn = crate::test_support::connection();
    let mut source = conn.new_browser_context_fixture_for_test("CTX-source");
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

fn projection(
    conn: &mut CdpConnection,
    body: DownloadBody,
) -> (DownloadProjection, DownloadObservation) {
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
    (
        DownloadProjection::new(frame_id, event_route, observation.guid().to_owned()),
        observation,
    )
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
    // This scenario observes an active transfer before completing it under the
    // response gate. A buffered body can finish before observer admission, putting
    // its terminal event in the initial batch instead of the background channel.
    let (body, chunks, completion, _) = stream();
    let (projection, observation) = projection(&mut conn, body);
    let mut monitor = observation.clone();
    let mut inline = Vec::new();
    conn.observe_download(
        projection,
        observation,
        &mut inline,
        true,
        &mut command_context,
    )
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
    chunks.send(b"without frontend".to_vec()).unwrap();
    drop(chunks);
    completion.send(Ok(())).unwrap();
    assert!(matches!(
        terminal(&mut monitor).await.state,
        DownloadState::Completed { .. }
    ));
    assert!(conn.project_browser_download(monitor.event()).is_empty());
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
async fn download_flush_and_native_updates_share_one_frontend_fifo() {
    let (_directory, mut conn) = fixture();
    let (sender, mut events) = mpsc::unbounded_channel();
    conn.set_background_event_sender(sender);
    let (permit, flush) = conn.begin_command_response_flush_permit();
    let mut command_context = CommandDispatchContext::new(flush);
    let (body, chunks, completion, _) = stream();
    let (projection, observation) = projection(&mut conn, body);
    let mut monitor = observation.clone();
    conn.observe_download(
        projection,
        observation,
        &mut Vec::new(),
        true,
        &mut command_context,
    )
    .await;
    chunks.send(b"before".to_vec()).unwrap();
    wait_for(&mut monitor, |snapshot| snapshot.received_bytes == 6).await;
    assert!(conn.project_browser_download(monitor.event()).is_empty());
    assert!(events.try_recv().is_err());
    permit.finish();
    // Do not drain the queued pre-flush progress before a later native terminal
    // arrives. Both must still use one FIFO, not two competing output paths.
    chunks.send(b"after".to_vec()).unwrap();
    drop(chunks);
    completion.send(Ok(())).unwrap();
    terminal(&mut monitor).await;
    assert!(
        conn.project_browser_download(monitor.event()).is_empty(),
        "native completion must not bypass already queued flush progress"
    );
    let mut progress = Vec::new();
    while let Ok(event) = events.try_recv() {
        let message = event.into_protocol_message();
        if message["method"] == "Browser.downloadProgress" {
            progress.push((
                message["params"]["state"].clone(),
                message["params"]["receivedBytes"].clone(),
            ));
        }
    }
    assert_eq!(
        progress,
        [
            (serde_json::json!("inProgress"), serde_json::json!(6)),
            (serde_json::json!("completed"), serde_json::json!(11)),
        ]
    );
}

#[tokio::test]
async fn admitted_download_freezes_context_policy_and_observation_separately() {
    let (directory, mut conn) = fixture();
    let (projection, observation) =
        projection(&mut conn, DownloadBody::Buffered(b"snapshot".to_vec()));
    let mut monitor = observation.clone();
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
        observation,
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
    let (projection, observation) = projection(&mut conn, body);
    let mut monitor = observation.clone();
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
    let (sender, mut events) = mpsc::unbounded_channel();
    conn.set_background_event_sender(sender);
    let (body, chunks, _completion, cancel) = stream();
    let (projection, observation) = projection(&mut conn, body);
    let mut monitor = observation.clone();
    conn.observe_download(
        projection,
        observation,
        &mut Vec::new(),
        true,
        &mut CommandDispatchContext::default(),
    )
    .await;
    chunks.send(b"partial".to_vec()).unwrap();
    wait_for(&mut monitor, |snapshot| snapshot.received_bytes == 7).await;
    assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 1);
    let removed = conn
        .remove_browser_context_by_id_restoring_active_async("CTX-source", None)
        .await
        .unwrap();
    assert!(removed.remove_from_browser().unwrap());
    drop(removed);
    assert_eq!(terminal(&mut monitor).await.state, DownloadState::Canceled);
    assert!(cancel.is_cancelled());
    assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
    assert!(conn.project_retired_context_downloads().is_empty());
    assert!(
        conn.download_projections.is_empty(),
        "retired records must release their projection"
    );
    let mut canceled = 0;
    while let Ok(event) = events.try_recv() {
        let message = event.into_protocol_message();
        if message["method"] == "Browser.downloadProgress"
            && message["params"]["state"] == "canceled"
        {
            assert_eq!(message["params"]["guid"], monitor.guid());
            canceled += 1;
        }
    }
    assert_eq!(
        canceled, 1,
        "recover the exact record after its Context has disappeared"
    );
    conn.insert_browser_context(conn.new_browser_context_fixture_for_test("CTX-source"));
    assert_eq!(
        conn.cancel_download(monitor.guid()),
        Err("No download item found for the given GUID".into())
    );
    assert!(
        matches!(conn.start_open_download_as_stream(monitor.guid()), Err(error) if error == "No download item found for the given GUID")
    );
}

async fn pending_native_download(
    conn: &mut CdpConnection,
) -> (
    moli_core::browser::BrowserContextHandle,
    moli_core::browser::BrowserNavigationWaiter,
    moli_core::browser::web_contents::NavigationInterceptionPermit,
) {
    use moli_core::browser::{
        NavigationDecision, NavigationDecisionStage, web_contents::NavigationRequestInterception,
    };
    let waiter = conn
        .start_native_navigation_fixture_for_test(
            &CommandOwnerScope::for_session("SID-source"),
            "LOADER-source",
            NavigationRequestInterception::new(
                Url::parse("https://source.test/report.txt").unwrap(),
                "GET".into(),
                None,
                Vec::new(),
                crate::conn::NavigationRequestLoadPolicy::DocumentInitiated,
            ),
            NavigationDecision::Fulfill {
                status: 200,
                headers: vec![(
                    "Content-Disposition".into(),
                    "attachment; filename=report.txt".into(),
                )],
                body: b"navigation".to_vec(),
            },
        )
        .unwrap();
    let request = waiter.request();
    let native = conn
        .browser
        .context_handle(request.web_contents.context())
        .unwrap();
    let (_, mut events) = conn.browser.subscribe().unwrap();
    loop {
        if let Some(paused) = native.navigation_decision(request.web_contents).unwrap() {
            assert_eq!(paused.permit.navigation(), request.navigation);
            assert!(matches!(
                paused.stage,
                NavigationDecisionStage::Response { .. }
            ));
            return (native, waiter, paused.permit);
        }
        events.recv().await.expect("native response decision");
    }
}

async fn replace_source_download_context(conn: &mut CdpConnection, directory: &TestDirectory) {
    drop(
        conn.remove_browser_context_by_id_restoring_active_async("CTX-source", None)
            .await
            .unwrap(),
    );
    let mut replacement = conn.new_browser_context_fixture_for_test("CTX-source");
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
    let (native, waiter, permit) = pending_native_download(&mut conn).await;
    let contents = waiter.request().web_contents;
    conn.detach_known_session_event_plan("TID-source", "SID-source", None, None);
    assert!(
        conn.target_owner_identity_for_owner(&CommandOwnerScope::for_session("SID-source"))
            .is_none()
    );
    let source = conn
        .browser_context
        .replace(BrowserContext::new("CTX-foreground".into()))
        .unwrap();
    conn.push_inactive_browser_context_fixture_for_test(source);
    assert!(
        native
            .resolve_navigation_decision(
                contents,
                permit,
                moli_core::browser::NavigationDecision::Continue,
            )
            .unwrap()
    );
    assert!(
        matches!(waiter.wait().await.unwrap(), moli_core::browser::BrowserNavigationOutcome::Download { url }
        if url.as_str() == "https://source.test/report.txt")
    );
    let record = conn
        .browser
        .subscribe()
        .unwrap()
        .0
        .downloads
        .into_iter()
        .find(|record| record.event.web_contents == contents)
        .expect("Browser admitted the exact navigation download");
    let mut monitor = record.observation.clone();
    let mut out = conn.project_created_browser_download(record);
    assert!(matches!(
        terminal(&mut monitor).await.state,
        DownloadState::Completed { .. }
    ));
    out.extend(conn.project_browser_download(monitor.event()));
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
    let (native, waiter, permit) = pending_native_download(&mut conn).await;
    let contents = waiter.request().web_contents;
    assert!(
        native.remove().unwrap(),
        "dispose the physical Browser Context"
    );
    replace_source_download_context(&mut conn, &directory).await;

    native
        .resolve_navigation_decision(
            contents,
            permit,
            moli_core::browser::NavigationDecision::Continue,
        )
        .expect_err("the original native Context was disposed");
    assert!(waiter.wait().await.is_err());
    assert!(conn.browser.subscribe().unwrap().0.downloads.is_empty());

    assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
}

#[tokio::test]
async fn retiring_only_the_download_projection_does_not_cancel_native_navigation() {
    let (directory, mut conn) = fixture();
    let (native, waiter, permit) = pending_native_download(&mut conn).await;
    let contents = waiter.request().web_contents;
    // Removing a DevTools projection does not dispose its Browser Context.
    replace_source_download_context(&mut conn, &directory).await;
    let replacement = conn
        .browser_web_contents_for_owner(&CommandOwnerScope::for_session("SID-source"))
        .unwrap();
    assert_ne!(replacement.context(), contents.context());
    assert!(
        native
            .resolve_navigation_decision(
                contents,
                permit,
                moli_core::browser::NavigationDecision::Continue,
            )
            .unwrap()
    );
    assert!(matches!(
        waiter.wait().await.unwrap(),
        moli_core::browser::BrowserNavigationOutcome::Download { .. }
    ));
    let records = conn.browser.subscribe().unwrap().0.downloads;
    assert!(
        !records
            .iter()
            .any(|record| record.event.web_contents == replacement)
    );
    let record = records
        .into_iter()
        .find(|record| record.event.web_contents == contents)
        .expect("the original Browser Context owns its download without a projection");
    let mut monitor = record.observation.clone();
    assert!(
        conn.project_created_browser_download(record).is_empty(),
        "the same wire identity must not receive the retired projection's event"
    );
    assert!(matches!(
        terminal(&mut monitor).await.state,
        DownloadState::Completed { .. }
    ));
    assert_eq!(
        std::fs::read(directory.0.join(monitor.guid())).unwrap(),
        b"navigation"
    );
    assert!(
        conn.start_open_download_as_stream(monitor.guid()).is_err(),
        "the replacement Context must not expose the old Context's artifact"
    );
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
