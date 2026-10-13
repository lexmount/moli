use super::super::navigation_streaming::{
    completed_raw_body_for_replacement_test, prepare_live_response_for_test,
};
use super::*;
use crate::runtime::owner_local::RendererLayoutWorkingSetObservation;
use tokio::{io::AsyncWriteExt, net::TcpListener, sync::mpsc};

const CASE_TIMEOUT: Duration = Duration::from_secs(20);
const SIGNAL_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug)]
enum CaptureResponse {
    Ready,
    HttpFailure,
    InvalidData,
}

struct CaptureResourceServer {
    base: String,
    path: &'static str,
    requests: mpsc::UnboundedReceiver<String>,
    release: Option<oneshot::Sender<()>>,
    response_finished: Option<oneshot::Receiver<bool>>,
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl CaptureResourceServer {
    async fn new(resource: PrintOnlyResource, response: CaptureResponse) -> Self {
        let (path, content_type, mut body) = resource.resource();
        if matches!(response, CaptureResponse::InvalidData) {
            match resource {
                PrintOnlyResource::Font => body = b"invalid-font-data".to_vec(),
                PrintOnlyResource::Image => {
                    // Keep valid PNG metadata and fail actual pixel decoding.
                    // This precondition distinguishes decoder failure from a
                    // MIME/metadata rejection before any decode job exists.
                    let mut offset = 8;
                    loop {
                        let len = u32::from_be_bytes(body[offset..offset + 4].try_into().unwrap())
                            as usize;
                        if &body[offset + 4..offset + 8] == b"IDAT" {
                            assert!(len >= 2);
                            body[offset + 8] = 0;
                            body[offset + 9] = 0;
                            break;
                        }
                        offset += 12 + len;
                        assert!(offset < body.len(), "encoded fixture has IDAT");
                    }
                    assert!(moli_image::probe_raster_image(&body).is_ok());
                    assert!(moli_image::decode_raster_image(&body).is_err());
                }
            }
        }
        if matches!(response, CaptureResponse::HttpFailure) {
            body.clear();
        }
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (requests_tx, requests) = mpsc::unbounded_channel();
        let (release, release_rx) = oneshot::channel();
        let (finished, response_finished) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut first = Some((release_rx, finished));
            loop {
                let (mut stream, _) = listener.accept().await?;
                let request = read_owner_wake_http_request_head(&mut stream).await;
                let actual = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .ok_or_else(|| anyhow::anyhow!("resource request has no path"))?;
                requests_tx
                    .send(actual.to_owned())
                    .map_err(|_| anyhow::anyhow!("request receiver dropped"))?;
                anyhow::ensure!(actual == path, "unexpected request: {actual}");
                let finished = if let Some((release, finished)) = first.take() {
                    release
                        .await
                        .map_err(|_| anyhow::anyhow!("resource release was dropped"))?;
                    Some(finished)
                } else {
                    None
                };
                let status = if matches!(response, CaptureResponse::HttpFailure) {
                    "404 Not Found"
                } else {
                    "200 OK"
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let written = async {
                    stream.write_all(head.as_bytes()).await?;
                    stream.write_all(&body).await?;
                    stream.shutdown().await
                }
                .await;
                // Cancellation may have already closed the socket. Reporting
                // that outcome is sufficient; it is not a server-task panic.
                if let Some(finished) = finished {
                    let _ = finished.send(written.is_ok());
                }
            }
        });
        Self {
            base,
            path,
            requests,
            release: Some(release),
            response_finished: Some(response_finished),
            task,
        }
    }

    fn ensure_no_request(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(
            matches!(
                self.requests.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            ),
            "an unrequested/duplicate resource reached HTTP"
        );
        Ok(())
    }

    async fn wait_request(&mut self) -> anyhow::Result<()> {
        let path = tokio::time::timeout(SIGNAL_TIMEOUT, self.requests.recv())
            .await?
            .ok_or_else(|| anyhow::anyhow!("HTTP resource task ended"))?;
        anyhow::ensure!(path == self.path);
        Ok(())
    }

    async fn release_and_observe(&mut self) -> anyhow::Result<bool> {
        self.release
            .take()
            .ok_or_else(|| anyhow::anyhow!("resource released twice"))?
            .send(())
            .map_err(|_| anyhow::anyhow!("server disappeared before release"))?;
        Ok(tokio::time::timeout(SIGNAL_TIMEOUT, self.response_finished.take().unwrap()).await??)
    }
}

fn capture_command() -> RendererPageCommand {
    RendererPageCommand::CaptureScreenshot(RendererCaptureScreenshotRequest {
        purpose: RendererScreenshotPurpose::Print {
            print_background: true,
        },
        ..RendererCaptureScreenshotRequest::viewport_png()
    })
}

async fn frames(page: &RendererPageHandle) -> anyhow::Result<()> {
    evaluate_print_fixture(page, "new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>setTimeout(()=>resolve(true),0))))".to_owned(), true).await?;
    Ok(())
}

async fn pending_zero(page: &RendererPageHandle) -> anyhow::Result<()> {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::PendingSubresourceRequestCount)
        .await?;
    anyhow::ensure!(
        matches!(reply, RendererPageReply::Usize(0)),
        "capture left pending transport requests"
    );
    Ok(())
}

async fn fixture(
    runtime: &JsRuntime,
    loader: &ResourceRequestClient,
    server: &CaptureResourceServer,
    resource: PrintOnlyResource,
    child: bool,
) -> anyhow::Result<RendererPageHandle> {
    let html = concat!(
        "<!doctype html><style>html,body{margin:0;background:white;scrollbar-width:none}",
        "#screen{position:absolute;left:0;top:64px;width:40px;height:16px;background:red}",
        "#asset{display:none;position:absolute;left:8px;top:8px}</style>",
        "<div id=screen></div><span id=asset>__TEXT__</span>"
    )
    .replace(
        "__TEXT__",
        match resource {
            PrintOnlyResource::Font => "iiii",
            PrintOnlyResource::Image => "",
        },
    );
    let page = create_test_html_page(
        runtime,
        loader,
        url::Url::parse(&format!("{}/page.html", server.base))?,
        &html,
    )
    .await;
    page.run_async_command(RendererPageCommand::SetViewportSurface(Some(
        crate::protocol_types::ViewportSurface {
            inner_width: 128,
            inner_height: 96,
            outer_width: 128,
            outer_height: 96,
            screen_width: 128,
            screen_height: 96,
            screen_avail_width: 128,
            screen_avail_height: 96,
            device_pixel_ratio: 1.0,
            ..Default::default()
        },
    )))
    .await?;
    let css = resource.css(&format!("{}{}", server.base, server.path));
    let installation = if child {
        let child_html = format!(
            "{html}<script>const s=new CSSStyleSheet();s.replaceSync({});document.adoptedStyleSheets=[s];</script>",
            serde_json::to_string(&css)?
        );
        format!(
            r#"const s=new CSSStyleSheet();s.replaceSync('@media print{{#screen{{width:80px;background:lime}}}}');document.adoptedStyleSheets=[s];
          const f=document.createElement('iframe');f.id='print-frame';f.style.cssText='width:64px;height:48px;border:0';
          const loaded=new Promise(resolve=>f.onload=resolve);f.srcdoc={};document.body.append(f);await loaded;"#,
            serde_json::to_string(&child_html)?
        )
    } else {
        format!(
            "const s=new CSSStyleSheet();s.replaceSync({});document.adoptedStyleSheets=[s];",
            serde_json::to_string(&css)?
        )
    };
    evaluate_print_fixture(&page, format!(r#"(async()=>{{
      window.printEvents=[];window.screenFonts=document.fonts;
      for(const type of ['loading','loadingdone','loadingerror']) screenFonts.addEventListener(type,()=>printEvents.push('fonts:'+type));
      matchMedia('print').addEventListener('change',()=>printEvents.push('media'));
      {installation}return true;
    }})()"#), true).await?;
    frames(&page).await?;
    Ok(page)
}

async fn screen_baseline(
    page: &RendererPageHandle,
) -> anyhow::Result<(
    RendererVisualStateToken,
    RendererLayoutWorkingSetObservation,
    serde_json::Value,
)> {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::CaptureScreencastFrame(
            screen_cast_request(None),
        ))
        .await?;
    let RendererPageReply::CaptureScreencastFrame(RendererCaptureScreencastFrameReply::Captured(
        screen,
    )) = reply
    else {
        anyhow::bail!("missing screen baseline")
    };
    let observation = RendererPageTestingHandle::new_for_testing(page)
        .layout_working_set_observation_async()
        .await?;
    Ok((screen.visual_state, observation, screen_state(page).await?))
}

async fn screen_unchanged(
    page: &RendererPageHandle,
    before: (
        RendererVisualStateToken,
        RendererLayoutWorkingSetObservation,
        serde_json::Value,
    ),
) -> anyhow::Result<()> {
    let after = RendererPageTestingHandle::new_for_testing(page)
        .layout_working_set_observation_async()
        .await?;
    anyhow::ensure!(
        after.vm_creation_id == before.1.vm_creation_id && after.document == before.1.document
    );
    anyhow::ensure!(after.screen.2 == before.1.screen.2 && after.screen.3 == before.1.screen.3);
    anyhow::ensure!(
        after.inputs == before.1.inputs,
        "capture completion changed canonical inputs"
    );
    let (reply, _) = page
        .run_async_command(RendererPageCommand::CaptureScreencastFrame(
            screen_cast_request(Some(before.0)),
        ))
        .await?;
    anyhow::ensure!(
        matches!(
            reply,
            RendererPageReply::CaptureScreencastFrame(
                RendererCaptureScreencastFrameReply::Unchanged
            )
        ),
        "capture changed screen visual state"
    );
    anyhow::ensure!(screen_state(page).await? == before.2);
    Ok(())
}

async fn assert_fallback(resource: PrintOnlyResource, disabled: bool, response: CaptureResponse) {
    let runtime = JsRuntime::initialize();
    runtime
        .renderer_owner_handle()
        .configure_layout_policy(LayoutPolicy::Realtime)
        .unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mask = if disabled {
        crate::protocol_types::OptionalResourceFetchMask::empty()
    } else {
        resource.mask()
    };
    loader.set_optional_resource_fetch_mask(mask);
    let mut server = CaptureResourceServer::new(resource, response).await;
    let mut page = fixture(&runtime, &loader, &server, resource, false)
        .await
        .unwrap();
    let result = tokio::time::timeout(CASE_TIMEOUT, async {
        pending_zero(&page).await?;
        server.ensure_no_request()?;
        let baseline = screen_baseline(&page).await?;
        let pending = page.enqueue_async_command(capture_command())?;
        if !disabled {
            server.wait_request().await?;
            anyhow::ensure!(
                server.release_and_observe().await?,
                "ordinary response should reach the capture connection"
            );
        }
        // This is the first and only reply used to prove fallback correctness.
        let (reply, _) = pending.wait().await?.into_reply_and_state();
        let RendererPageReply::CaptureScreenshot(RendererCaptureScreenshotReply::Captured(first)) =
            reply
        else {
            anyhow::bail!("ordinary resource failure must still produce the first Print")
        };
        anyhow::ensure!((first.width, first.height) == (128, 96));
        anyhow::ensure!(decoded_png_pixel(&first.bytes, 60, 70) == [0, 255, 0, 255]);
        if matches!(resource, PrintOnlyResource::Image) {
            anyhow::ensure!(
                decoded_png_pixel(&first.bytes, 12, 12) == [255, 0, 255, 255],
                "first Print must use CSS fallback, not a missing-resource error or future pixels"
            );
        }
        pending_zero(&page).await?;
        server.ensure_no_request()?;
        screen_unchanged(&page, baseline).await?;
        anyhow::ensure!(loader.optional_resource_fetch_mask() == mask);
        // Separate admission-release evidence, after first bytes were checked.
        // Remove dependencies; this reply never replaces the stored first one.
        evaluate_print_fixture(
            &page,
            "document.adoptedStyleSheets=[];true".to_owned(),
            false,
        )
        .await?;
        let (reply, _) = page.run_async_command(capture_command()).await?;
        anyhow::ensure!(matches!(
            reply,
            RendererPageReply::CaptureScreenshot(RendererCaptureScreenshotReply::Captured(_))
        ));
        pending_zero(&page).await?;
        server.ensure_no_request()?;
        Ok::<_, anyhow::Error>(())
    })
    .await;
    cleanup(&mut page, server, result).await;
}

async fn cleanup(
    page: &mut RendererPageHandle,
    server: CaptureResourceServer,
    result: std::result::Result<anyhow::Result<()>, tokio::time::error::Elapsed>,
) {
    let closed = tokio::time::timeout(SIGNAL_TIMEOUT, page.close_async()).await;
    server.task.abort();
    let stopped = server.task.await;
    assert!(
        closed.as_ref().is_ok_and(|value| value.is_ok()),
        "close: {closed:?}; case: {result:?}"
    );
    assert!(
        stopped.as_ref().is_err_and(|error| error.is_cancelled()),
        "server: {stopped:?}; case: {result:?}"
    );
    result
        .expect("capture boundary case must terminate")
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn print_font_mask_disabled_does_not_admit_http() {
    assert_fallback(PrintOnlyResource::Font, true, CaptureResponse::Ready).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn print_image_mask_disabled_does_not_admit_http() {
    assert_fallback(PrintOnlyResource::Image, true, CaptureResponse::Ready).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn print_font_http_failure_returns_first_fallback() {
    assert_fallback(PrintOnlyResource::Font, false, CaptureResponse::HttpFailure).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn print_image_http_failure_returns_first_fallback() {
    assert_fallback(
        PrintOnlyResource::Image,
        false,
        CaptureResponse::HttpFailure,
    )
    .await;
}
#[tokio::test(flavor = "multi_thread")]
async fn print_invalid_font_returns_first_fallback() {
    assert_fallback(PrintOnlyResource::Font, false, CaptureResponse::InvalidData).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn print_image_decode_failure_returns_first_fallback() {
    assert_fallback(
        PrintOnlyResource::Image,
        false,
        CaptureResponse::InvalidData,
    )
    .await;
}

#[derive(Clone, Copy, Debug)]
enum Retirement {
    DocumentOpen,
    Navigation,
    ChildDetach,
    Deadline,
    ViewportChange,
}

const REPLACEMENT_HTML: &str = concat!(
    "<!doctype html><style>html,body{margin:0;background:white;scrollbar-width:none}",
    "#screen{position:absolute;left:0;top:64px;width:52px;height:16px;background:cyan}",
    "</style><div id=screen></div>",
    "<script>window.printEvents=[];window.screenFonts=document.fonts;</script>"
);

async fn assert_retirement(resource: PrintOnlyResource, retirement: Retirement) {
    let runtime = JsRuntime::initialize();
    runtime
        .renderer_owner_handle()
        .configure_layout_policy(LayoutPolicy::Realtime)
        .unwrap();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_optional_resource_fetch_mask(resource.mask());
    let mut server = CaptureResourceServer::new(resource, CaptureResponse::Ready).await;
    let mut page = fixture(
        &runtime,
        &loader,
        &server,
        resource,
        matches!(retirement, Retirement::ChildDetach),
    )
    .await
    .unwrap();
    let result = tokio::time::timeout(CASE_TIMEOUT, async {
        pending_zero(&page).await?;
        server.ensure_no_request()?;
        // enqueue returns an owned receiver: the actual Page remains available
        // for a new owner command while this capture awaits its HTTP resource.
        let pending = page.enqueue_async_command(capture_command())?;
        server.wait_request().await?;
        match retirement {
            Retirement::DocumentOpen => {
                evaluate_print_fixture(
                    &page,
                    format!(
                        "document.open();document.write({});document.close();true",
                        serde_json::to_string(REPLACEMENT_HTML)?
                    ),
                    false,
                )
                .await?;
            }
            Retirement::Navigation => {
                let testing = RendererPageTestingHandle::new_for_testing(&page);
                let before = testing.renderer_page_view_async().await?;
                let raw = completed_raw_body_for_replacement_test(REPLACEMENT_HTML.as_bytes()).await;
                let prepared = prepare_live_response_for_test(
                    &runtime,
                    &page,
                    &loader,
                    url::Url::parse(&format!("{}/replacement.html", server.base))?,
                    "text/html",
                    raw,
                )
                .await;
                let permit = prepared.issue_commit_permit();
                let replacement = prepared.commit_page_replacement(permit).await?;
                let (_, _, artifacts) = page.adopt_document_replacement(replacement)?;
                if let Some(continuation) = page.take_committed_document_post_response_continuation() {
                    continuation.release();
                }
                loop {
                    let publication = output_rx.recv().await
                        .ok_or_else(|| anyhow::anyhow!("replacement output route closed"))?;
                    if publication_document_lifecycle_events(&publication).any(|event| {
                        event.document == artifacts.active_document
                            && event.kind == RendererDocumentLifecycleEventKind::Milestone(
                                RendererDocumentLifecycleMilestone::Load,
                            )
                    }) {
                        break;
                    }
                }
                let after = testing.renderer_page_view_async().await?;
                anyhow::ensure!(
                    after.page_id == before.page_id && after.vm_creation_id != before.vm_creation_id,
                    "navigation must replace the real Document on the same Page"
                );
            }
            Retirement::ChildDetach => {
                anyhow::ensure!(
                    evaluate_print_fixture(
                        &page,
                        "document.getElementById('print-frame').remove();!document.getElementById('print-frame')".to_owned(),
                        false,
                    )
                    .await? == serde_json::json!(true)
                );
            }
            Retirement::Deadline => {}
            Retirement::ViewportChange => {
                page.run_async_command(RendererPageCommand::SetViewportSurface(Some(
                    crate::protocol_types::ViewportSurface {
                        inner_width: 160,
                        inner_height: 96,
                        outer_width: 160,
                        outer_height: 96,
                        screen_width: 160,
                        screen_height: 96,
                        screen_avail_width: 160,
                        screen_avail_height: 96,
                        device_pixel_ratio: 1.0,
                        ..Default::default()
                    },
                ))).await?;
            }
        }
        // No response has been released. A successful fallback cannot satisfy
        // retirement or the finite resource deadline contract.
        let error = match pending.wait().await {
            Err(error) => format!("{error:#}"),
            Ok(_) => anyhow::bail!("retired capture returned a successful owned reply"),
        };
        let expected_error = if matches!(retirement, Retirement::Deadline) {
            error.contains("timed out")
        } else {
            !error.contains("timed out")
                && (error.contains("Document")
                    || error.contains("document")
                    || error.contains("retired")
                    || error.contains("no longer belongs to this Page slot"))
        };
        anyhow::ensure!(
            expected_error,
            "expected {retirement:?} failure before the held response, got: {error}"
        );
        frames(&page).await?;
        pending_zero(&page).await?;
        let baseline = screen_baseline(&page).await?;
        let expected_width = if matches!(retirement, Retirement::ChildDetach | Retirement::Deadline | Retirement::ViewportChange) { 40 } else { 52 };
        anyhow::ensure!(baseline.2["width"] == expected_width);
        if matches!(retirement, Retirement::ViewportChange) {
            anyhow::ensure!(baseline.2["viewport"] == serde_json::json!([160, 96]));
        }

        // Make the former server response available only AFTER the first
        // command was rejected. A canceled socket may already be closed;
        // either transport outcome must leave the current screen untouched.
        let _socket_still_writable = server.release_and_observe().await?;
        frames(&page).await?;
        pending_zero(&page).await?;
        server.ensure_no_request()?;
        screen_unchanged(&page, baseline).await?;
        anyhow::ensure!(loader.optional_resource_fetch_mask() == resource.mask());

        // This distinct, resource-free request proves the stable Page's
        // command admission was released, without replacing the first reply.
        if matches!(retirement, Retirement::Deadline | Retirement::ViewportChange) {
            evaluate_print_fixture(
                &page,
                "document.adoptedStyleSheets=[];true".to_owned(),
                false,
            ).await?;
        }
        let (reply, _) = page.run_async_command(capture_command()).await?;
        anyhow::ensure!(
            matches!(reply, RendererPageReply::CaptureScreenshot(RendererCaptureScreenshotReply::Captured(_))),
            "Page must accept a fresh Print after retiring the prior capture"
        );
        pending_zero(&page).await?;
        server.ensure_no_request()?;
        Ok::<_, anyhow::Error>(())
    })
    .await;
    cleanup(&mut page, server, result).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn print_font_document_open_rejects_held_capture_and_ignores_late_response() {
    assert_retirement(PrintOnlyResource::Font, Retirement::DocumentOpen).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn print_image_navigation_rejects_held_capture_and_ignores_late_response() {
    assert_retirement(PrintOnlyResource::Image, Retirement::Navigation).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn print_child_detach_rejects_held_capture_and_ignores_late_response() {
    assert_retirement(PrintOnlyResource::Image, Retirement::ChildDetach).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn print_resource_deadline_rejects_held_capture_and_releases_admission() {
    assert_retirement(PrintOnlyResource::Font, Retirement::Deadline).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn print_viewport_change_rejects_held_capture_and_releases_admission() {
    assert_retirement(PrintOnlyResource::Font, Retirement::ViewportChange).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn mock_print_is_layout_disabled_without_resource_admission() {
    let runtime = JsRuntime::initialize();
    runtime
        .renderer_owner_handle()
        .configure_layout_policy(LayoutPolicy::Mock)
        .unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_optional_resource_fetch_mask(PrintOnlyResource::Image.mask());
    let mut server =
        CaptureResourceServer::new(PrintOnlyResource::Image, CaptureResponse::Ready).await;
    let mut page = fixture(&runtime, &loader, &server, PrintOnlyResource::Image, false)
        .await
        .unwrap();
    let result = tokio::time::timeout(CASE_TIMEOUT, async {
        pending_zero(&page).await?;
        server.ensure_no_request()?;
        let (reply, _) = page.run_async_command(capture_command()).await?;
        anyhow::ensure!(matches!(
            reply,
            RendererPageReply::CaptureScreenshot(RendererCaptureScreenshotReply::LayoutDisabled)
        ));
        pending_zero(&page).await?;
        server.ensure_no_request()?;
        Ok::<_, anyhow::Error>(())
    })
    .await;
    cleanup(&mut page, server, result).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn on_demand_first_print_resources_preserve_screen_sample_and_observers() {
    for resource in [PrintOnlyResource::Font, PrintOnlyResource::Image] {
        assert_first_print_resource_with_policy(resource, LayoutPolicy::OnDemand).await;
    }
}

async fn child_screen_state(page: &RendererPageHandle) -> anyhow::Result<serde_json::Value> {
    let value = evaluate_print_fixture(
        page,
        r#"JSON.stringify((()=>{
            const frame=document.getElementById('print-frame');
            const child=frame.contentWindow,doc=frame.contentDocument;
            const rect=frame.getBoundingClientRect();
            return {
                frame:[rect.x,rect.y,rect.width,rect.height],
                viewport:[child.innerWidth,child.innerHeight],
                width:doc.getElementById('screen').offsetWidth,
                assetWidth:doc.getElementById('asset').offsetWidth,
                print:child.matchMedia('print').matches,
                fontsSame:doc.fonts===child.screenFonts,
                fontCount:doc.fonts.size,
                events:child.printEvents
            };
        })())"#
            .to_owned(),
        false,
    )
    .await?;
    serde_json::from_str(
        value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("child screen state must be a JSON string"))?,
    )
    .map_err(Into::into)
}

async fn assert_first_child_print_resource(resource: PrintOnlyResource) {
    let runtime = JsRuntime::initialize();
    runtime
        .renderer_owner_handle()
        .configure_layout_policy(LayoutPolicy::Realtime)
        .unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_optional_resource_fetch_mask(resource.mask());
    let mut server = CaptureResourceServer::new(resource, CaptureResponse::Ready).await;
    let mut page = fixture(&runtime, &loader, &server, resource, true)
        .await
        .unwrap();
    let result = tokio::time::timeout(CASE_TIMEOUT, async {
        // Keep the existing child fixture and resource rules. Only make its
        // placement deterministic; the child viewport remains 64 by 48.
        evaluate_print_fixture(
            &page,
            r#"(()=>{
                const frame=document.getElementById('print-frame');
                frame.style.cssText='position:absolute;left:16px;top:8px;width:64px;height:48px;border:0';
                const child=frame.contentWindow,doc=frame.contentDocument;
                child.screenFonts=doc.fonts;child.printEvents=[];
                for(const type of ['loading','loadingdone','loadingerror'])
                    doc.fonts.addEventListener(type,()=>child.printEvents.push('fonts:'+type));
                child.matchMedia('print').addEventListener('change',()=>child.printEvents.push('media'));
                return true;
            })()"#
                .to_owned(),
            false,
        )
        .await?;
        frames(&page).await?;
        let before_child = child_screen_state(&page).await?;
        anyhow::ensure!(
            before_child == serde_json::json!({
                "frame":[16,8,64,48],"viewport":[64,48],"width":40,
                "assetWidth":0,"print":false,"fontsSame":true,"fontCount":0,"events":[]
            }),
            "child screen precondition: {before_child}"
        );
        let baseline = screen_baseline(&page).await?;
        anyhow::ensure!(baseline.1.screen.3.is_some());
        // The existing native snapshot serializes both current and published
        // input stamps. Each must include root and child exact Document owners.
        // This is member identity evidence, not an extra per-box observer API.
        let inputs = baseline.1.inputs.as_deref().ok_or_else(|| {
            anyhow::anyhow!("Realtime child capture needs a native input baseline")
        })?;
        anyhow::ensure!(
            inputs.contains("published=Some(ScreenLayoutInputs {")
                && inputs.matches("DocumentLayoutInputs {").count() == 4,
            "expected root and child in current/published input stamps: {inputs}"
        );
        let before_root_state = baseline.2.clone();
        pending_zero(&page).await?;
        server.ensure_no_request()?;

        // The only resource URL belongs to the child. Its HTTP gate proves
        // that the real owner parks this first Print and later resumes it.
        let pending = page.enqueue_async_command(capture_command())?;
        server.wait_request().await?;
        anyhow::ensure!(
            server.release_and_observe().await?,
            "child response must reach the held capture connection"
        );
        let (reply, _) = pending.wait().await?.into_reply_and_state();
        let RendererPageReply::CaptureScreenshot(RendererCaptureScreenshotReply::Captured(first)) =
            reply
        else {
            anyhow::bail!("first child-resource Print must produce owned bytes")
        };
        anyhow::ensure!((first.width, first.height) == (128, 96));
        anyhow::ensure!(
            decoded_png_pixel(&first.bytes, 60, 70) == [0, 255, 0, 255],
            "root print media must apply in the first reply"
        );
        match resource {
            PrintOnlyResource::Font => {
                // Existing Ahem samples, translated by the iframe's (16,8).
                // Four 20px i glyphs advance 48px inside the 64px child viewport.
                for (x, y) in [(26, 18), (46, 18), (66, 18), (66, 28)] {
                    anyhow::ensure!(
                        decoded_png_pixel(&first.bytes, x, y) == [0, 0, 0, 255],
                        "first reply needs the child's downloaded font at ({x},{y})"
                    );
                }
                anyhow::ensure!(decoded_png_pixel(&first.bytes, 74, 20) == [255; 4]);
            }
            PrintOnlyResource::Image => {
                for (x, y) in [(28, 20), (58, 30)] {
                    anyhow::ensure!(
                        decoded_png_pixel(&first.bytes, x, y) == [17, 85, 221, 255],
                        "first reply needs the child's downloaded image at ({x},{y})"
                    );
                }
            }
        }

        // This helper reads native publication/root/retained metrics and the
        // whole input/member stamp BEFORE any post-Print geometry getter or
        // capture. It then requires the known screen token to be Unchanged.
        screen_unchanged(&page, baseline).await?;
        anyhow::ensure!(child_screen_state(&page).await? == before_child);
        frames(&page).await?;
        anyhow::ensure!(child_screen_state(&page).await? == before_child);
        anyhow::ensure!(screen_state(&page).await? == before_root_state);
        pending_zero(&page).await?;
        server.ensure_no_request()?;
        anyhow::ensure!(loader.optional_resource_fetch_mask() == resource.mask());

        // Separate admission-release evidence after validating the first
        // image and all isolation assertions. This cannot repair its pixels.
        evaluate_print_fixture(
            &page,
            "document.getElementById('print-frame').contentDocument.adoptedStyleSheets=[];true"
                .to_owned(),
            false,
        )
        .await?;
        let (reply, _) = page.run_async_command(capture_command()).await?;
        anyhow::ensure!(matches!(
            reply,
            RendererPageReply::CaptureScreenshot(RendererCaptureScreenshotReply::Captured(_))
        ));
        pending_zero(&page).await?;
        server.ensure_no_request()?;
        Ok::<_, anyhow::Error>(())
    })
    .await;
    cleanup(&mut page, server, result).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn first_print_loads_child_font_without_mutating_screen_members() {
    assert_first_child_print_resource(PrintOnlyResource::Font).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn first_print_loads_child_image_without_mutating_screen_members() {
    assert_first_child_print_resource(PrintOnlyResource::Image).await;
}
