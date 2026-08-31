use super::{
    ExternalRawDocumentBodyStream, JsLocalExecutor, JsRuntime, JsRuntimeOwner, PageVmInitStage,
    PreparedRendererDocument, RendererCaptureScreenshotReply, RendererDragData,
    RendererDraggedDirectory, RendererDraggedFile, RendererInputDispatchOutcome,
    RendererInspectorProtocolConfiguration, RendererInspectorSessionRestoreSnapshot,
    RendererOutputItem, RendererOutputPublication, RendererOutputResidenceIdentity,
    RendererOutputTransportMessage, RendererOutputTransportReceiver, RendererOutputTransportSender,
    RendererOwnerAction, RendererPageCommand, RendererPageHandle, RendererPageReply,
    RendererPageTestingHandle, RendererPendingPopupActivation, RendererPointerEventProperties,
    RendererPreparedDocumentCommitConfiguration, RendererProtocolObservation,
    RendererRuntimeCommandOutput, RendererRuntimeInspectorMessage,
    RendererRuntimeInspectorResponseSender,
};
use crate::local_executor::{is_on_script_execution_lane_for, scope_on_scaffold_js_local_executor};
use crate::network::ResourceRequestClient;
use crate::{
    RendererDocumentLifecycleEventKind, RendererDocumentLifecycleMilestone,
    RendererNavigationReplyPolicy, RendererReplyBoundary, RendererTopLevelNavigationDispatch,
};
use parking_lot::Mutex;
use std::collections::HashSet;
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};

mod open_streaming;

async fn prepare_test_external_raw_document(
    runtime: &JsRuntime,
    loader: &ResourceRequestClient,
    url: url::Url,
    raw_body: ExternalRawDocumentBodyStream,
) -> PreparedRendererDocument {
    prepare_test_external_raw_document_with_content_type(
        runtime,
        loader,
        url,
        "text/html",
        raw_body,
    )
    .await
}

async fn prepare_test_external_raw_document_with_content_type(
    runtime: &JsRuntime,
    loader: &ResourceRequestClient,
    url: url::Url,
    content_type: &str,
    raw_body: ExternalRawDocumentBodyStream,
) -> PreparedRendererDocument {
    prepare_test_external_raw_document_with_content_type_and_reply_boundary(
        runtime,
        loader,
        url,
        content_type,
        raw_body,
        RendererReplyBoundary::Stage,
    )
    .await
}

async fn prepare_test_external_raw_document_with_content_type_and_reply_boundary(
    runtime: &JsRuntime,
    loader: &ResourceRequestClient,
    url: url::Url,
    content_type: &str,
    raw_body: ExternalRawDocumentBodyStream,
    reply_boundary: RendererReplyBoundary,
) -> PreparedRendererDocument {
    runtime
        .prepare_streaming_raw_document_from_external_body_with_inspector_session_restores(
            runtime.reserve_page_for_creation(),
            url.clone(),
            url,
            None,
            false,
            0,
            Vec::new(),
            200,
            vec![("content-type".to_owned(), content_type.to_owned())],
            loader,
            crate::RendererWebStorageHandles::ephemeral(),
            raw_body,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
            Vec::new(),
            false,
            PageVmInitStage::Load,
            reply_boundary,
            RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            RendererNavigationReplyPolicy::FollowBeforeReply,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("external raw document should prepare")
}

struct RendererExternalActivityTestReceiver(RendererOutputTransportReceiver);

impl RendererExternalActivityTestReceiver {
    async fn recv_message(&mut self) -> RendererOutputTransportMessage {
        tokio::time::timeout(Duration::from_secs(1), self.0.recv())
            .await
            .expect("renderer output message should arrive before the test deadline")
            .expect("renderer output transport should remain open")
    }

    async fn recv(&mut self) -> Option<RendererOutputPublication> {
        loop {
            match self.0.recv().await? {
                RendererOutputTransportMessage::Publication(publication) => {
                    return Some(publication);
                }
                RendererOutputTransportMessage::StreamControl(_)
                | RendererOutputTransportMessage::PageReservationReleased { .. }
                | RendererOutputTransportMessage::CursorLeaseDeclared { .. }
                | RendererOutputTransportMessage::CursorLeaseReleased { .. } => {}
            }
        }
    }

    fn try_recv(&mut self) -> Result<RendererOutputPublication, mpsc::error::TryRecvError> {
        loop {
            match self.0.try_recv()? {
                RendererOutputTransportMessage::Publication(publication) => {
                    return Ok(publication);
                }
                RendererOutputTransportMessage::StreamControl(_)
                | RendererOutputTransportMessage::PageReservationReleased { .. }
                | RendererOutputTransportMessage::CursorLeaseDeclared { .. }
                | RendererOutputTransportMessage::CursorLeaseReleased { .. } => {}
            }
        }
    }

    fn drain(&mut self) -> Vec<RendererOutputPublication> {
        let mut publications = Vec::new();
        while let Ok(publication) = self.try_recv() {
            publications.push(publication);
        }
        publications
    }

    fn drain_runtime_binding_calls_for_page(
        &mut self,
        page: &RendererPageHandle,
    ) -> Vec<crate::native_bridge::PendingRuntimeBindingCall> {
        self.drain()
            .into_iter()
            .filter(|publication| publication_is_for_page(publication, page))
            .flat_map(RendererOutputPublication::into_records)
            .filter_map(|record| match record.into_parts().1 {
                RendererOutputItem::Observation(RendererProtocolObservation::RuntimeBinding(
                    call,
                )) => Some(call),
                _ => None,
            })
            .collect()
    }

    fn drain_runtime_inspector_messages_for_page(
        &mut self,
        page: &RendererPageHandle,
    ) -> Vec<serde_json::Value> {
        self.drain()
            .into_iter()
            .filter(|publication| publication_is_for_page(publication, page))
            .flat_map(RendererOutputPublication::into_records)
            .filter_map(|record| match record.into_parts().1 {
                RendererOutputItem::Observation(RendererProtocolObservation::RuntimeInspector(
                    batch,
                )) => Some(batch.messages),
                _ => None,
            })
            .flatten()
            .map(runtime_inspector_message_protocol_message_for_test)
            .collect()
    }

    async fn recv_top_level_location_navigation(
        &mut self,
    ) -> Option<super::RendererDocumentSourcedTopLevelLocationNavigation> {
        loop {
            if let Some(navigation) =
                self.recv()
                    .await?
                    .records()
                    .iter()
                    .find_map(|record| match record.item() {
                        super::RendererOutputItem::OwnerAction(
                            super::RendererOwnerAction::TopLevelLocationNavigation(navigation),
                        ) => Some(navigation.clone()),
                        _ => None,
                    })
            {
                return Some(navigation);
            }
        }
    }
}

fn renderer_external_activity_test_channel() -> (
    RendererOutputTransportSender,
    RendererExternalActivityTestReceiver,
) {
    let (tx, rx) = super::renderer_output_transport_channel();
    (tx, RendererExternalActivityTestReceiver(rx))
}

fn publication_is_for_page(
    publication: &RendererOutputPublication,
    page: &RendererPageHandle,
) -> bool {
    matches!(
        publication.cursor().stream().residence(),
        RendererOutputResidenceIdentity::Page {
            owner_local_host_id,
            page_id,
        } if owner_local_host_id == page.owner_local_host_id()
            && page_id == page.renderer_page_id()
    )
}

fn publication_document_lifecycle_events(
    publication: &RendererOutputPublication,
) -> impl Iterator<Item = &super::RendererDocumentLifecycleEvent> {
    publication
        .records()
        .iter()
        .filter_map(|record| match record.item() {
            super::RendererOutputItem::Observation(
                super::RendererProtocolObservation::DocumentLifecycle(event),
            ) => Some(event),
            _ => None,
        })
}

fn popup_activations_for_page(
    publications: &[RendererOutputPublication],
    page: &RendererPageHandle,
) -> Vec<RendererPendingPopupActivation> {
    publications
        .iter()
        .filter(|publication| publication_is_for_page(publication, page))
        .flat_map(RendererOutputPublication::records)
        .filter_map(|record| match record.item() {
            RendererOutputItem::OwnerAction(RendererOwnerAction::Popup(activation)) => {
                Some(activation.clone())
            }
            _ => None,
        })
        .collect()
}

async fn recv_page_lifecycle_until(
    receiver: &mut RendererExternalActivityTestReceiver,
    page: &RendererPageHandle,
    milestone: RendererDocumentLifecycleMilestone,
) -> Vec<super::RendererDocumentLifecycleEvent> {
    let mut events = Vec::new();
    while let Some(publication) = receiver.recv().await {
        if !publication_is_for_page(&publication, page) {
            continue;
        }
        let publication_events = publication_document_lifecycle_events(&publication)
            .copied()
            .collect::<Vec<_>>();
        let reached_milestone = publication_events
            .iter()
            .any(|event| event.kind == RendererDocumentLifecycleEventKind::Milestone(milestone));
        events.extend(publication_events);
        if reached_milestone {
            return events;
        }
    }
    panic!("renderer output transport closed before {milestone:?}")
}

async fn serialize_html_for_renderer_page(page: &RendererPageHandle) -> String {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::SerializeHtml)
        .await
        .expect("renderer page should serialize HTML");
    match reply {
        RendererPageReply::OptionalString(Some(html)) => html,
        _ => panic!("expected SerializeHtml string reply"),
    }
}

async fn outer_html_for_renderer_document(
    page: &RendererPageHandle,
    include_shadow_dom: bool,
) -> String {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::OuterHtmlForDocument { include_shadow_dom })
        .await
        .expect("renderer page should serialize document outer HTML");
    match reply {
        RendererPageReply::OptionalString(Some(html)) => html,
        _ => panic!("expected document outer HTML string reply"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn outer_html_document_command_includes_only_author_shadow_roots() {
    let runtime = JsRuntime::initialize();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/outer-html-shadow").unwrap();
    let page = create_test_html_page(
        &runtime,
        &loader,
        url,
        concat!(
            "<!doctype html><html><body>",
            "<x-host id='host'><template shadowrootmode='closed'>",
            "<span>shadow</span></template>light</x-host>",
            "<input id='control'>",
            "</body></html>"
        ),
    )
    .await;

    let ordinary = outer_html_for_renderer_document(&page, false).await;
    let serialize_html = serialize_html_for_renderer_page(&page).await;
    assert_eq!(ordinary, serialize_html);
    assert!(!ordinary.contains("shadowrootmode"));
    assert!(!ordinary.contains("shadow"));

    let including_shadow = outer_html_for_renderer_document(&page, true).await;
    assert!(including_shadow.contains(concat!(
        "<x-host id=\"host\"><template shadowrootmode=\"closed\">",
        "<span>shadow</span></template>light</x-host>"
    )));
    assert_eq!(
        including_shadow
            .matches("<template shadowrootmode=")
            .count(),
        1
    );
    assert!(including_shadow.contains("<input id=\"control\">"));
    assert!(!including_shadow.contains("<input id=\"control\"><template"));
}

fn initialize_layout_test_runtime() -> JsRuntimeOwner {
    let runtime = JsRuntime::initialize();
    runtime
        .renderer_owner_handle()
        .configure_layout_policy(crate::real_layout_test_policy())
        .expect("layout test policy should configure before page creation");
    runtime
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_uses_current_root_computed_background_and_viewport() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/root-screenshot").unwrap();
    let page = create_test_html_page(
        &runtime,
        &loader,
        url,
        "<!doctype html><style>html { background-color: rgb(255, 0, 0); scrollbar-width: none }</style>",
    )
    .await;
    let viewport = crate::protocol_types::ViewportSurface {
        inner_width: 4,
        inner_height: 3,
        outer_width: 4,
        outer_height: 3,
        device_pixel_ratio: 1.0,
        screen_width: 4,
        screen_height: 3,
        screen_avail_width: 4,
        screen_avail_height: 3,
    };
    let (reply, _) = page
        .run_async_command(RendererPageCommand::SetViewportSurface(Some(viewport)))
        .await
        .expect("viewport should update");
    assert!(matches!(reply, RendererPageReply::Unit));

    let red = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!((red.width, red.height), (4, 3));
    assert_eq!(decoded_png_pixel(&red.bytes, 2, 1), [255, 0, 0, 255]);
    let red_warm = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!(
        red.bytes, red_warm.bytes,
        "cold and warm Stylo caches must produce identical snapshots"
    );

    page.run_async_command(RendererPageCommand::EvaluateExpression {
        expression: "document.documentElement.style.backgroundColor = 'rgb(0, 255, 0)'".to_owned(),
        await_promise: false,
    })
    .await
    .expect("root background mutation should complete");
    let green = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!(decoded_png_pixel(&green.bytes, 2, 1), [0, 255, 0, 255]);
    assert_ne!(red.bytes, green.bytes);
    let green_warm = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!(
        green.bytes, green_warm.bytes,
        "post-mutation cold and warm snapshots must agree"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_paints_downloaded_raster_image_pixels() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    loader.set_image_fetch_enabled(true);
    let fixture = moli_image::RgbaImage::try_new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255])
        .expect("valid two-pixel image");
    let encoded = moli_image::encode_png(&fixture).expect("fixture PNG should encode");
    let (base_url, request_seen, release_response, server) =
        spawn_owner_wake_gated_binary_server_with_content_type(
            "/fixture.png",
            encoded.bytes,
            "image/png",
        )
        .await;
    let url = url::Url::parse(&format!("{base_url}/page.html")).expect("valid fixture page URL");
    let page = create_test_html_page_at_document_commit(
        &runtime,
        &loader,
        url,
        concat!(
            "<!doctype html><style>",
            "html,body{margin:0;background:white}",
            "img{display:block;width:20px;height:10px;image-rendering:pixelated}",
            "</style><img src='/fixture.png'>"
        ),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(2), request_seen)
        .await
        .expect("image request should start before the test deadline")
        .expect("image request signal should remain open");
    let viewport = crate::protocol_types::ViewportSurface {
        inner_width: 20,
        inner_height: 10,
        outer_width: 20,
        outer_height: 10,
        device_pixel_ratio: 1.0,
        screen_width: 20,
        screen_height: 10,
        screen_avail_width: 20,
        screen_avail_height: 10,
    };
    page.run_async_command(RendererPageCommand::SetViewportSurface(Some(viewport)))
        .await
        .expect("viewport should update");

    let pending = tokio::time::timeout(
        Duration::from_secs(1),
        capture_screenshot_for_renderer_page(&page),
    )
    .await
    .expect("pending image decode must not block a fresh screenshot");
    let pending_pixel = decoded_png_pixel(&pending.bytes, 2, 5);
    assert_ne!(pending_pixel, [255, 0, 0, 255]);
    assert_ne!(pending_pixel, [0, 255, 0, 255]);

    release_response
        .send(())
        .expect("image response should release once");
    server.await.expect("image fixture server should finish");
    page.run_async_command(RendererPageCommand::WaitForScriptTruthy {
        expression: "document.querySelector('img')?.complete && document.querySelector('img')?.naturalWidth === 2 && document.querySelector('img')?.naturalHeight === 1".to_owned(),
        timeout_ms: 2_000,
        loader: loader.clone(),
    })
    .await
    .expect("downloaded image should finish decode and dispatch load");

    let screenshot = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!((screenshot.width, screenshot.height), (20, 10));
    assert_ne!(screenshot.bytes, pending.bytes);
    assert_eq!(decoded_png_pixel(&screenshot.bytes, 2, 5), [255, 0, 0, 255]);
    assert_eq!(
        decoded_png_pixel(&screenshot.bytes, 17, 5),
        [0, 255, 0, 255]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn screenshot_and_screencast_paint_downloaded_svg_vectors() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    loader.set_image_fetch_enabled(true);
    let encoded = br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1" viewBox="0 0 2 1"><rect width="1" height="1" fill="#ff0000"/><rect x="1" width="1" height="1" fill="#00ff00"/></svg>"##;
    let (base_url, request_seen, release_response, server) =
        spawn_owner_wake_gated_binary_server_with_content_type(
            "/fixture.svg",
            encoded.to_vec(),
            "image/svg+xml",
        )
        .await;
    let url = url::Url::parse(&format!("{base_url}/page.html")).expect("valid fixture page URL");
    let page = create_test_html_page_at_document_commit(
        &runtime,
        &loader,
        url,
        concat!(
            "<!doctype html><style>",
            "html,body{margin:0;background:white}",
            "img{display:block;width:20px;height:10px;object-fit:fill}",
            "</style><img src='/fixture.svg'>"
        ),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(2), request_seen)
        .await
        .expect("SVG request should start before the test deadline")
        .expect("SVG request signal should remain open");
    let viewport = crate::protocol_types::ViewportSurface {
        inner_width: 20,
        inner_height: 10,
        outer_width: 20,
        outer_height: 10,
        device_pixel_ratio: 1.0,
        screen_width: 20,
        screen_height: 10,
        screen_avail_width: 20,
        screen_avail_height: 10,
    };
    page.run_async_command(RendererPageCommand::SetViewportSurface(Some(viewport)))
        .await
        .expect("viewport should update");

    let pending = tokio::time::timeout(
        Duration::from_secs(1),
        capture_screenshot_for_renderer_page(&page),
    )
    .await
    .expect("pending SVG parse must not block a fresh screenshot");
    assert_ne!(decoded_png_pixel(&pending.bytes, 2, 5), [255, 0, 0, 255]);

    release_response
        .send(())
        .expect("SVG response should release once");
    server.await.expect("SVG fixture server should finish");
    page.run_async_command(RendererPageCommand::WaitForScriptTruthy {
        expression: "document.querySelector('img')?.complete && document.querySelector('img')?.naturalWidth === 2 && document.querySelector('img')?.naturalHeight === 1".to_owned(),
        timeout_ms: 2_000,
        loader: loader.clone(),
    })
    .await
    .expect("downloaded SVG should finish parsing and dispatch load");

    let screenshot = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!(decoded_png_pixel(&screenshot.bytes, 2, 5), [255, 0, 0, 255]);
    assert_eq!(
        decoded_png_pixel(&screenshot.bytes, 17, 5),
        [0, 255, 0, 255]
    );

    let screencast = capture_screencast_frame_with_request(
        &page,
        super::RendererCaptureScreencastFrameRequest {
            format: super::RendererScreenshotFormat::Png,
            quality: 100,
            optimize_for_speed: true,
            max_width: None,
            max_height: None,
            known_visual_state: None,
        },
    )
    .await;
    assert_eq!(
        decoded_png_pixel(&screencast.image.bytes, 2, 5),
        [255, 0, 0, 255]
    );
    assert_eq!(
        decoded_png_pixel(&screencast.image.bytes, 17, 5),
        [0, 255, 0, 255]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_consumes_the_shadow_flat_tree_without_light_dom_leaks() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/layout-shadow-flat-tree").unwrap();
    let page = create_test_html_page(
        &runtime,
        &loader,
        url,
        concat!(
            "<!doctype html><style>html,body{margin:0;background:white}",
            "x-layout{display:block;width:40px}</style>",
            "<x-layout id='host'>",
            "<span slot='selected' style='display:block;width:20px;height:20px;background:red'></span>",
            "<span slot='missing' style='display:block;width:20px;height:20px;background:blue'></span>",
            "</x-layout>",
            "<div hidden style='display:block;width:20px;height:20px;background:fuchsia'></div>",
            "<script>",
            "const shadow=host.attachShadow({mode:'open'});",
            "const selected=document.createElement('slot');selected.name='selected';",
            "const suppressed=document.createElement('span');",
            "suppressed.style='display:block;width:20px;height:20px;background:yellow';",
            "selected.append(suppressed);",
            "const fallbackSlot=document.createElement('slot');fallbackSlot.name='fallback';",
            "const fallback=document.createElement('span');",
            "fallback.style='display:block;width:20px;height:20px;background:lime';",
            "fallbackSlot.append(fallback);shadow.append(selected,fallbackSlot);",
            "</script>",
        ),
    )
    .await;
    let viewport = crate::protocol_types::ViewportSurface {
        inner_width: 40,
        inner_height: 60,
        outer_width: 40,
        outer_height: 60,
        device_pixel_ratio: 1.0,
        screen_width: 40,
        screen_height: 60,
        screen_avail_width: 40,
        screen_avail_height: 60,
    };
    page.run_async_command(RendererPageCommand::SetViewportSurface(Some(viewport)))
        .await
        .expect("viewport should update");

    let screenshot = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!(decoded_png_pixel(&screenshot.bytes, 5, 5), [255, 0, 0, 255]);
    assert_eq!(
        decoded_png_pixel(&screenshot.bytes, 5, 25),
        [0, 255, 0, 255]
    );
    assert_eq!(
        decoded_png_pixel(&screenshot.bytes, 5, 45),
        [255, 255, 255, 255],
        "unassigned light DOM, suppressed slot fallback, and hidden content must not leak into layout"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_encodes_jpeg_and_limits_device_dimensions() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/jpeg-screenshot").unwrap();
    let page = create_test_html_page(
        &runtime,
        &loader,
        url,
        "<!doctype html><style>html { background: rgb(20, 80, 160) }</style>",
    )
    .await;
    let viewport = crate::protocol_types::ViewportSurface {
        inner_width: 4,
        inner_height: 3,
        outer_width: 4,
        outer_height: 3,
        device_pixel_ratio: 2.0,
        screen_width: 4,
        screen_height: 3,
        screen_avail_width: 4,
        screen_avail_height: 3,
    };
    page.run_async_command(RendererPageCommand::SetViewportSurface(Some(viewport)))
        .await
        .expect("viewport should update");

    let request = super::RendererCaptureScreenshotRequest {
        purpose: super::RendererScreenshotPurpose::Screenshot,
        format: super::RendererScreenshotFormat::Jpeg,
        quality: 80,
        region: super::RendererScreenshotRegion::Viewport,
        optimize_for_speed: false,
        max_width: Some(5),
        max_height: Some(4),
    };
    let (reply, _) = page
        .run_async_command(RendererPageCommand::CaptureScreenshot(request))
        .await
        .expect("renderer page should capture JPEG");
    let RendererPageReply::CaptureScreenshot(RendererCaptureScreenshotReply::Captured(image)) =
        reply
    else {
        panic!("expected captured JPEG reply");
    };
    assert_eq!(image.mime_type, "image/jpeg");
    assert_eq!((image.width, image.height), (5, 4));
    assert_eq!(&image.bytes[..2], &[0xff, 0xd8]);
    assert_eq!(&image.bytes[image.bytes.len() - 2..], &[0xff, 0xd9]);
}

#[tokio::test(flavor = "multi_thread")]
async fn print_capture_uses_print_media_controls_backgrounds_and_restores_screen_media() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/print-capture").unwrap();
    let page = create_test_html_page(
        &runtime,
        &loader,
        url,
        concat!(
            "<!doctype html><style>",
            "html,body{margin:0;background:white}",
            "#probe{width:10px;height:10px;background:red}",
            "iframe{position:absolute;left:10px;top:0;width:10px;height:10px;border:0}",
            "@media print{#probe{background:rgb(0,255,0)}}",
            "</style><div id='probe'></div>",
            "<iframe srcdoc='<style>html,body{margin:0;background:blue}</style>'></iframe>",
        ),
    )
    .await;
    let viewport = crate::protocol_types::ViewportSurface {
        inner_width: 20,
        inner_height: 20,
        outer_width: 20,
        outer_height: 20,
        device_pixel_ratio: 1.0,
        screen_width: 20,
        screen_height: 20,
        screen_avail_width: 20,
        screen_avail_height: 20,
    };
    page.run_async_command(RendererPageCommand::SetViewportSurface(Some(viewport)))
        .await
        .expect("viewport should update");

    let screen = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!(decoded_png_pixel(&screen.bytes, 5, 5), [255, 0, 0, 255]);
    assert_eq!(decoded_png_pixel(&screen.bytes, 15, 5), [0, 0, 255, 255]);

    let print = capture_screenshot_with_request(
        &page,
        super::RendererCaptureScreenshotRequest {
            purpose: super::RendererScreenshotPurpose::Print {
                print_background: true,
            },
            format: super::RendererScreenshotFormat::Png,
            quality: 100,
            region: super::RendererScreenshotRegion::Viewport,
            optimize_for_speed: false,
            max_width: None,
            max_height: None,
        },
    )
    .await;
    assert_eq!(decoded_png_pixel(&print.bytes, 5, 5), [0, 255, 0, 255]);
    assert_eq!(decoded_png_pixel(&print.bytes, 15, 5), [0, 0, 255, 255]);

    let no_background = capture_screenshot_with_request(
        &page,
        super::RendererCaptureScreenshotRequest {
            purpose: super::RendererScreenshotPurpose::Print {
                print_background: false,
            },
            format: super::RendererScreenshotFormat::Png,
            quality: 100,
            region: super::RendererScreenshotRegion::Viewport,
            optimize_for_speed: false,
            max_width: None,
            max_height: None,
        },
    )
    .await;
    assert_eq!(
        decoded_png_pixel(&no_background.bytes, 5, 5),
        [255, 255, 255, 255]
    );
    assert_eq!(
        decoded_png_pixel(&no_background.bytes, 15, 5),
        [255, 255, 255, 255]
    );

    let restored_screen = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!(
        decoded_png_pixel(&restored_screen.bytes, 5, 5),
        [255, 0, 0, 255]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_clip_and_full_document_keep_the_live_layout_viewport() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/capture-surfaces").unwrap();
    let page = create_test_html_page(
        &runtime,
        &loader,
        url,
        concat!(
            "<!doctype html><style>html,body{margin:0}",
            ".band{width:20px;height:20px}</style>",
            "<div class='band' style='background:red'></div>",
            "<div class='band' style='background:rgb(0,255,0)'></div>",
        ),
    )
    .await;
    let viewport = crate::protocol_types::ViewportSurface {
        inner_width: 20,
        inner_height: 20,
        outer_width: 20,
        outer_height: 20,
        device_pixel_ratio: 2.0,
        screen_width: 20,
        screen_height: 20,
        screen_avail_width: 20,
        screen_avail_height: 20,
    };
    page.run_async_command(RendererPageCommand::SetViewportSurface(Some(viewport)))
        .await
        .expect("viewport should update");

    let full = capture_screenshot_with_request(
        &page,
        super::RendererCaptureScreenshotRequest {
            purpose: super::RendererScreenshotPurpose::Screenshot,
            format: super::RendererScreenshotFormat::Png,
            quality: 100,
            region: super::RendererScreenshotRegion::FullDocument,
            optimize_for_speed: false,
            max_width: None,
            max_height: None,
        },
    )
    .await;
    assert_eq!((full.width, full.height), (40, 80));
    assert_eq!(decoded_png_pixel(&full.bytes, 10, 10), [255, 0, 0, 255]);
    assert_eq!(decoded_png_pixel(&full.bytes, 10, 60), [0, 255, 0, 255]);

    let clip = capture_screenshot_with_request(
        &page,
        super::RendererCaptureScreenshotRequest {
            purpose: super::RendererScreenshotPurpose::Screenshot,
            format: super::RendererScreenshotFormat::Png,
            quality: 100,
            region: super::RendererScreenshotRegion::PageClip(super::RendererScreenshotClip {
                x: 0.0,
                y: 20.0,
                width: 20.0,
                height: 20.0,
                scale: 0.5,
            }),
            optimize_for_speed: true,
            max_width: None,
            max_height: None,
        },
    )
    .await;
    assert_eq!((clip.width, clip.height), (20, 20));
    assert_eq!(decoded_png_pixel(&clip.bytes, 10, 10), [0, 255, 0, 255]);
}

#[tokio::test(flavor = "multi_thread")]
async fn bounded_viewport_clip_keeps_root_controls_while_page_clip_omits_them() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/viewport-control-capture").unwrap();
    let page = create_test_html_page(
        &runtime,
        &loader,
        url,
        concat!(
            "<!doctype html><style>",
            "html,body{margin:0}",
            "html{scrollbar-color:rgb(255,0,0) rgb(0,0,255)}",
            "#content{width:200px;height:200px}",
            "</style><div id='content'></div>",
        ),
    )
    .await;
    let viewport = crate::protocol_types::ViewportSurface {
        inner_width: 100,
        inner_height: 100,
        outer_width: 100,
        outer_height: 100,
        device_pixel_ratio: 1.0,
        screen_width: 100,
        screen_height: 100,
        screen_avail_width: 100,
        screen_avail_height: 100,
    };
    page.run_async_command(RendererPageCommand::SetViewportSurface(Some(viewport)))
        .await
        .expect("viewport should update");
    let clip = super::RendererScreenshotClip {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 100.0,
        scale: 1.0,
    };

    let viewport_clip = capture_screenshot_with_request(
        &page,
        super::RendererCaptureScreenshotRequest {
            purpose: super::RendererScreenshotPurpose::Screenshot,
            format: super::RendererScreenshotFormat::Png,
            quality: 100,
            region: super::RendererScreenshotRegion::ViewportClip(clip),
            optimize_for_speed: false,
            max_width: None,
            max_height: None,
        },
    )
    .await;
    assert_eq!(
        decoded_png_pixel(&viewport_clip.bytes, 95, 30),
        [255, 0, 0, 255],
        "a bounded live-viewport capture includes its fixed root thumb"
    );
    assert_eq!(
        decoded_png_pixel(&viewport_clip.bytes, 92, 8),
        [255, 0, 0, 255],
        "the root control paints Chromium's up-arrow glyph"
    );
    assert_eq!(
        decoded_png_pixel(&viewport_clip.bytes, 87, 8),
        [0, 0, 255, 255],
        "the arrow remains centered inside its author-colored button"
    );

    let page_clip = capture_screenshot_with_request(
        &page,
        super::RendererCaptureScreenshotRequest {
            purpose: super::RendererScreenshotPurpose::Screenshot,
            format: super::RendererScreenshotFormat::Png,
            quality: 100,
            region: super::RendererScreenshotRegion::PageClip(clip),
            optimize_for_speed: false,
            max_width: None,
            max_height: None,
        },
    )
    .await;
    assert_eq!(
        decoded_png_pixel(&page_clip.bytes, 95, 30),
        [255, 255, 255, 255],
        "a capture-beyond-viewport page clip omits compositor controls"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_rejects_full_document_at_the_128k_css_boundary() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/capture-budget").unwrap();
    let page = create_test_html_page(
        &runtime,
        &loader,
        url,
        "<!doctype html><style>html,body{margin:0}.page{height:131072px}</style><div class=page></div>",
    )
    .await;
    let request = super::RendererCaptureScreenshotRequest {
        purpose: super::RendererScreenshotPurpose::Screenshot,
        format: super::RendererScreenshotFormat::Png,
        quality: 100,
        region: super::RendererScreenshotRegion::FullDocument,
        optimize_for_speed: false,
        max_width: None,
        max_height: None,
    };

    let error = match page
        .run_async_command(RendererPageCommand::CaptureScreenshot(request))
        .await
    {
        Ok(_) => panic!("full-document screenshot at 128K CSS pixels must be rejected"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("must each be less than 131072 CSS pixels"),
        "{error:#}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_lays_out_real_flex_mixed_flow_and_pseudo() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/layout-screenshot-poc").unwrap();
    let page = create_test_html_page(
        &runtime,
        &loader,
        url,
        include_str!("../../tests/fixtures/layout-screenshot-poc.html"),
    )
    .await;
    let viewport = crate::protocol_types::ViewportSurface {
        inner_width: 800,
        inner_height: 600,
        outer_width: 800,
        outer_height: 600,
        device_pixel_ratio: 1.0,
        screen_width: 800,
        screen_height: 600,
        screen_avail_width: 800,
        screen_avail_height: 600,
    };
    let (reply, _) = page
        .run_async_command(RendererPageCommand::SetViewportSurface(Some(viewport)))
        .await
        .expect("viewport should update");
    assert!(matches!(reply, RendererPageReply::Unit));

    let row = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!((row.width, row.height), (800, 600));
    assert_eq!(decoded_png_pixel(&row.bytes, 50, 20), [240, 40, 40, 255]);
    assert_eq!(decoded_png_pixel(&row.bytes, 150, 20), [40, 200, 80, 255]);
    assert_eq!(decoded_png_pixel(&row.bytes, 250, 20), [40, 100, 240, 255]);
    assert_eq!(decoded_png_pixel(&row.bytes, 55, 130), [250, 200, 30, 255]);
    assert_eq!(decoded_png_pixel(&row.bytes, 10, 150), [30, 190, 210, 255]);
    assert_eq!(decoded_png_pixel(&row.bytes, 55, 170), [210, 40, 180, 255]);
    assert_eq!(decoded_png_pixel(&row.bytes, 10, 190), [240, 130, 20, 255]);
    assert_eq!(decoded_png_pixel(&row.bytes, 35, 210), [100, 50, 180, 255]);
    assert_eq!(decoded_png_pixel(&row.bytes, 3, 250), [15, 25, 35, 255]);
    assert_eq!(decoded_png_pixel(&row.bytes, 60, 265), [30, 170, 90, 255]);
    assert_eq!(decoded_png_pixel(&row.bytes, 225, 295), [245, 140, 25, 255]);
    assert_eq!(
        decoded_png_pixel(&row.bytes, 250, 295),
        [255, 255, 255, 255]
    );
    assert!(
        decoded_png_dark_pixel_count(&row.bytes, 125, 205, 235, 265) > 8,
        "Parley glyphs should produce dark pixels inside the label region"
    );

    page.run_async_command(RendererPageCommand::EvaluateExpression {
        expression: "document.querySelector('#cards').style.flexDirection = 'column'".to_owned(),
        await_promise: false,
    })
    .await
    .expect("layout mutation should complete");
    let column = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!(
        decoded_png_pixel(&column.bytes, 150, 20),
        [255, 255, 255, 255]
    );
    assert_eq!(decoded_png_pixel(&column.bytes, 50, 60), [40, 200, 80, 255]);
    assert_eq!(
        decoded_png_pixel(&column.bytes, 50, 100),
        [40, 100, 240, 255]
    );
    assert_ne!(row.bytes, column.bytes);
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_screenshot_respects_mock_layout_policy() {
    let runtime = JsRuntime::initialize();
    runtime
        .renderer_owner_handle()
        .configure_layout_policy(moli_page_types::LayoutPolicy::Mock)
        .expect("layout policy should configure before page creation");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/no-layout-screenshot").unwrap();
    let page = create_test_html_page(&runtime, &loader, url, "<!doctype html>").await;

    let (reply, _) = page
        .run_async_command(RendererPageCommand::CaptureScreenshot(
            super::RendererCaptureScreenshotRequest::viewport_png(),
        ))
        .await
        .expect("layout-disabled screenshot should return a typed reply");
    assert!(matches!(
        reply,
        RendererPageReply::CaptureScreenshot(RendererCaptureScreenshotReply::LayoutDisabled)
    ));
}

async fn capture_screenshot_for_renderer_page(
    page: &RendererPageHandle,
) -> super::RendererCapturedScreenshot {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::CaptureScreenshot(
            super::RendererCaptureScreenshotRequest::viewport_png(),
        ))
        .await
        .expect("renderer page should capture a screenshot");
    match reply {
        RendererPageReply::CaptureScreenshot(RendererCaptureScreenshotReply::Captured(image)) => {
            image
        }
        _ => panic!("expected captured screenshot reply"),
    }
}

async fn dispatch_wheel_for_action_window_test(
    page: &RendererPageHandle,
    delta_y: f64,
) -> RendererInputDispatchOutcome {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::DispatchMouseEventAtPoint {
            x: 10.0,
            y: 10.0,
            event_name: "wheel".to_owned(),
            button: -1,
            buttons: Some(0),
            click_count: 0,
            delta_x: 0.0,
            delta_y,
            pointer: RendererPointerEventProperties::default(),
            modifiers: 0,
        })
        .await
        .expect("wheel action should enter the renderer action window");
    match reply {
        RendererPageReply::InputDispatchOutcome(outcome) => outcome,
        _ => panic!("wheel action should return an input dispatch outcome"),
    }
}

async fn capture_screenshot_with_request(
    page: &RendererPageHandle,
    request: super::RendererCaptureScreenshotRequest,
) -> super::RendererCapturedScreenshot {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::CaptureScreenshot(request))
        .await
        .expect("renderer page should capture a screenshot");
    match reply {
        RendererPageReply::CaptureScreenshot(RendererCaptureScreenshotReply::Captured(image)) => {
            image
        }
        _ => panic!("expected captured screenshot reply"),
    }
}

async fn capture_screencast_frame_with_request(
    page: &RendererPageHandle,
    request: super::RendererCaptureScreencastFrameRequest,
) -> super::RendererCapturedScreencastFrame {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::CaptureScreencastFrame(request))
        .await
        .expect("renderer page should capture a screencast frame");
    match reply {
        RendererPageReply::CaptureScreencastFrame(
            super::RendererCaptureScreencastFrameReply::Captured(frame),
        ) => frame,
        _ => panic!("expected captured screencast frame reply"),
    }
}

fn decoded_png_pixel(bytes: &[u8], x: u32, y: u32) -> [u8; 4] {
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder.read_info().expect("valid PNG header");
    let mut buffer = vec![
        0;
        reader
            .output_buffer_size()
            .expect("decoded PNG buffer size fits in memory")
    ];
    let output = reader.next_frame(&mut buffer).expect("valid PNG data");
    assert_eq!(output.color_type, png::ColorType::Rgba);
    assert_eq!(output.bit_depth, png::BitDepth::Eight);
    let offset = ((y * output.width + x) * 4) as usize;
    buffer[offset..offset + 4]
        .try_into()
        .expect("one RGBA pixel")
}

fn decoded_png_dark_pixel_count(
    bytes: &[u8],
    x_start: u32,
    x_end: u32,
    y_start: u32,
    y_end: u32,
) -> usize {
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder.read_info().expect("valid PNG header");
    let mut buffer = vec![
        0;
        reader
            .output_buffer_size()
            .expect("decoded PNG buffer size fits in memory")
    ];
    let output = reader.next_frame(&mut buffer).expect("valid PNG data");
    assert_eq!(output.color_type, png::ColorType::Rgba);
    assert_eq!(output.bit_depth, png::BitDepth::Eight);
    let x_end = x_end.min(output.width);
    let y_end = y_end.min(output.height);
    let mut count = 0;
    for y in y_start.min(y_end)..y_end {
        for x in x_start.min(x_end)..x_end {
            let offset = ((y * output.width + x) * 4) as usize;
            let pixel = &buffer[offset..offset + 4];
            if pixel[0] < 80 && pixel[1] < 80 && pixel[2] < 80 && pixel[3] > 0 {
                count += 1;
            }
        }
    }
    count
}

#[test]
fn page_ids_are_unique_across_threads() {
    let runtime = JsRuntime::initialize();
    let renderer_owner = runtime.renderer_owner_handle();
    let ids = Arc::new(Mutex::new(Vec::new()));
    let mut workers = Vec::new();

    for _ in 0..4 {
        let renderer_owner = renderer_owner.clone();
        let ids = ids.clone();
        workers.push(std::thread::spawn(move || {
            let id = renderer_owner.allocate_page_id().as_u64();
            ids.lock().push(id);
        }));
    }

    for worker in workers {
        worker.join().expect("worker should finish");
    }

    let ids = ids.lock();
    assert_eq!(ids.len(), 4);
    let unique = ids.iter().copied().collect::<HashSet<_>>();
    assert_eq!(unique.len(), 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_page_handle_runs_detached_owner_cleanup() {
    let runtime = JsRuntime::initialize();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/detached-drop-cleanup").unwrap();
    let page = create_test_html_page(&runtime, &loader, url, "<!doctype html>").await;
    let testing = RendererPageTestingHandle::new_for_testing(&page);

    testing
        .owner_slot_async()
        .await
        .expect("page should initially occupy an owner slot");
    drop(page);

    let owner_slot_after_drop =
        tokio::time::timeout(Duration::from_secs(1), testing.owner_slot_async())
            .await
            .expect("detached remove-page command should not stall");
    assert!(
        owner_slot_after_drop.is_err(),
        "dropping the page handle should remove its owner slot"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_loop_runs_file_reading_callback_without_retry_command() {
    let runtime = JsRuntime::initialize();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let (base_url, callback_request_seen, release_callback_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/file-reading-owner-loop",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url,
        r#"<!doctype html><body>
<div id="drop" style="width: 100px; height: 100px">drop</div>
<script>
document.getElementById("drop").addEventListener("drop", event => {
  const reader =
    event.dataTransfer.items[0].webkitGetAsEntry().createReader();
  reader.readEntries(entries => {
    fetch("/file-reading-owner-loop", {
      method: "POST",
      body: String(entries.length)
    });
  });
});
</script>
</body>"#,
    )
    .await;

    let (reply, _) = page
        .run_async_command(RendererPageCommand::DispatchDragEventAtPoint {
            x: 10.0,
            y: 10.0,
            event_name: "drop".to_owned(),
            data: RendererDragData {
                items: Vec::new(),
                files: Vec::new(),
                directories: vec![RendererDraggedDirectory {
                    name: "fixture".to_owned(),
                    files: vec![RendererDraggedFile {
                        bytes: b"body".to_vec(),
                        mime_type: "text/plain".to_owned(),
                        name: "entry.txt".to_owned(),
                        last_modified: 1.0,
                    }],
                    directories: Vec::new(),
                }],
                drag_operations_mask: 1,
            },
            modifiers: 0,
        })
        .await
        .expect("directory drop command should run");
    assert!(
        matches!(
            reply,
            RendererPageReply::InputDispatchOutcome(ref outcome) if outcome.handled
        ),
        "directory drop command should dispatch to the target"
    );

    tokio::time::timeout(Duration::from_secs(2), callback_request_seen)
        .await
        .expect("FileReading owner wake must run the callback without a retry command")
        .expect("callback request signal should remain open");
    release_callback_response
        .send(())
        .expect("callback response should release once");
    server
        .await
        .expect("FileReading owner-loop witness server should finish");
    page.close_async()
        .await
        .expect("FileReading owner-loop page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_loop_runs_misc_platform_api_callback_without_retry_command() {
    let runtime = JsRuntime::initialize();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let (base_url, callback_request_seen, release_callback_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/misc-platform-api-owner-loop",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page =
        create_test_html_page(&runtime, &loader, url, "<!doctype html><body></body>").await;

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
navigator.webkitTemporaryStorage.queryUsageAndQuota((usage, quota) => {
  fetch("/misc-platform-api-owner-loop", {
    method: "POST",
    body: `${usage}:${quota}`
  });
});
"queued"
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("deprecated storage quota callback should queue");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("queued"))
    );

    tokio::time::timeout(Duration::from_secs(2), callback_request_seen)
        .await
        .expect("MiscPlatformApi owner wake must run the callback without a retry command")
        .expect("callback request signal should remain open");
    release_callback_response
        .send(())
        .expect("callback response should release once");
    server
        .await
        .expect("MiscPlatformApi owner-loop witness server should finish");
    page.close_async()
        .await
        .expect("MiscPlatformApi owner-loop page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn streaming_xml_document_executes_parser_blocking_xhtml_script() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/initial.xhtml").unwrap();
    let source = concat!(
        "<html xmlns='http://www.w3.org/1999/xhtml'><head>",
        "<script>try { document.write('&lt;future/&gt;'); } ",
        "catch (error) { globalThis.__xmlWriteError = error.name; } ",
        "globalThis.__initialXhtmlHandoff = 'executed';</script>",
        "</head><body /></html>",
    );
    let prepared = prepare_test_external_raw_document_with_content_type(
        &runtime,
        &loader,
        url,
        "application/xhtml+xml",
        ExternalRawDocumentBodyStream::from_bytes(source.as_bytes().to_vec()),
    )
    .await;
    let permit = prepared.issue_commit_permit();
    let (mut page, page_state, _, _, pending_download) = prepared
        .commit(permit)
        .await
        .expect("incremental XHTML document should commit");
    assert!(pending_download.is_none());

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: concat!(
                "JSON.stringify([document.contentType, ",
                "document.getElementsByTagName('script').length, ",
                "String(globalThis.__initialXhtmlHandoff), ",
                "String(globalThis.__xmlWriteError), ",
                "document.getElementsByTagName('future').length])",
            )
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("initial XHTML script side effect should be observable");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!(
            r#"["application/xhtml+xml",1,"executed","InvalidStateError",0]"#
        )),
        "XML script report: {:#?}",
        page_state.script_execution.runs,
    );
    page.close_async().await.expect("XHTML page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn streaming_unstyled_xml_converts_live_document_before_domcontentloaded() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/document.xml").unwrap();
    let source = "<semantic-root id='source'><child>xml-ready</child></semantic-root>";
    let prepared = prepare_test_external_raw_document_with_content_type(
        &runtime,
        &loader,
        url,
        "application/xml",
        ExternalRawDocumentBodyStream::from_bytes(source.as_bytes().to_vec()),
    )
    .await;
    prepared
        .update_commit_configuration(RendererPreparedDocumentCommitConfiguration {
            document_start_scripts: vec![crate::DocumentStartScript {
                registry_key: None,
                devtools_session: None,
                source: concat!(
                    "globalThis.__xmlViewerDcl = null;",
                    "document.addEventListener('DOMContentLoaded', () => {",
                    "  const source = document.getElementById('source');",
                    "  globalThis.__xmlViewerDcl = [",
                    "    document.documentElement.localName,",
                    "    source && source.parentNode && source.parentNode.id,",
                    "    source && source.textContent",
                    "  ];",
                    "});",
                )
                .to_owned(),
                world_name: None,
                has_bidi_channel_argument: false,
                bidi_channel_handoffs: Vec::new(),
            }],
            runtime_bindings: Vec::new(),
            runtime_inspector_session_restore_snapshots: Vec::new(),
            runtime_isolated_worlds: Vec::new(),
            permission_overrides: Vec::new(),
            extra_http_headers: Vec::new(),
            locale_override: None,
            timezone_override: None,
            script_execution_disabled: false,
            bypass_content_security_policy: false,
            cpu_throttling_rate: 1.0,
            emulated_media: Default::default(),
            idle_override: None,
            viewport_surface: None,
            browser_resource_runtime: loader.browser_resource_runtime(),
            navigator_identity: loader.browser_identity().clone(),
            network_offline: false,
            bypass_service_worker: false,
            cache_disabled: false,
            blocked_url_patterns: Vec::new(),
            fetch_subresource_interception_enabled: false,
            fetch_subresource_interception_resource_type: None,
        })
        .await
        .expect("prepared XML should accept the lifecycle probe");

    let permit = prepared.issue_commit_permit();
    let (mut page, _, _, _, pending_download) = prepared
        .commit(permit)
        .await
        .expect("unstyled XML document should commit");
    assert!(pending_download.is_none());
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: concat!(
                "JSON.stringify([",
                "document.documentElement.localName,",
                "document.documentElement.namespaceURI,",
                "globalThis.__xmlViewerDcl",
                "])",
            )
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("live XML viewer state should evaluate");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!(
            r#"["html","http://www.w3.org/1999/xhtml",["html","webkit-xml-viewer-source-xml","xml-ready"]]"#
        ))
    );
    page.close_async()
        .await
        .expect("unstyled XML page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn prepared_streaming_xml_document_waits_for_permit_and_uses_latest_configuration() {
    let runtime = JsRuntime::initialize();
    let baseline_isolates = runtime.document_isolate_accounting_for_diagnostics();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, mut side_effect_request_seen, release_side_effect_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/native-author-side-effect",
            "ok",
            "text/plain",
        )
        .await;
    let url = url::Url::parse(&format!("{base_url}/prepared.xhtml")).expect("prepared XHTML url");
    let source = concat!(
        "<html xmlns='http://www.w3.org/1999/xhtml'><head><script>",
        "globalThis.__nativeCommitObserved = JSON.stringify([",
        "globalThis.__nativePreload, typeof nativeBinding]);",
        "fetch('/native-author-side-effect');",
        "</script></head><body /></html>",
    );
    let prepared = prepare_test_external_raw_document_with_content_type(
        &runtime,
        &loader,
        url.clone(),
        "application/xhtml+xml",
        ExternalRawDocumentBodyStream::from_bytes(source.as_bytes().to_vec()),
    )
    .await;
    let prepared_agent = prepared.renderer_devtools_agent_token();
    let prepared_isolates = runtime.document_isolate_accounting_for_diagnostics();
    assert_eq!(prepared_isolates.created, baseline_isolates.created + 1);
    assert_eq!(prepared_isolates.live, baseline_isolates.live + 1);
    assert_eq!(prepared_isolates.reserved, baseline_isolates.reserved + 1);
    assert_eq!(
        runtime.renderer_owner_handle().len(),
        0,
        "preparing a streaming XML document must not install a Page"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut side_effect_request_seen)
            .await
            .is_err(),
        "the XML parser handoff must not execute before the commit permit"
    );

    prepared
        .update_commit_configuration(RendererPreparedDocumentCommitConfiguration {
            document_start_scripts: vec![
                crate::DocumentStartScript {
                    registry_key: None,
                    devtools_session: None,
                    source: r#"globalThis.__nativePreload = "ready";"#.to_owned(),
                    world_name: None,
                    has_bidi_channel_argument: false,
                    bidi_channel_handoffs: Vec::new(),
                },
                crate::DocumentStartScript {
                    registry_key: None,
                    devtools_session: None,
                    source: r#"globalThis.__nativeWorldPreload = "ready";"#.to_owned(),
                    world_name: Some("native-world".to_owned()),
                    has_bidi_channel_argument: false,
                    bidi_channel_handoffs: Vec::new(),
                },
            ],
            runtime_bindings: vec![crate::protocol_types::RuntimeBindingRegistration {
                devtools_session: None,
                name: "nativeBinding".to_owned(),
                execution_context_name: None,
            }],
            runtime_inspector_session_restore_snapshots: vec![
                RendererInspectorSessionRestoreSnapshot {
                    protocol_configuration: RendererInspectorProtocolConfiguration {
                        runtime_frontend_enabled: true,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ],
            runtime_isolated_worlds: vec![crate::protocol_types::RuntimeIsolatedWorldDefinition {
                name: "native-world".to_owned(),
                grant_universal_access: false,
            }],
            permission_overrides: Vec::new(),
            extra_http_headers: Vec::new(),
            locale_override: None,
            timezone_override: None,
            script_execution_disabled: false,
            bypass_content_security_policy: false,
            cpu_throttling_rate: 1.0,
            emulated_media: Default::default(),
            idle_override: None,
            viewport_surface: None,
            browser_resource_runtime: loader.browser_resource_runtime(),
            navigator_identity: loader.browser_identity().clone(),
            network_offline: false,
            bypass_service_worker: false,
            cache_disabled: false,
            blocked_url_patterns: Vec::new(),
            fetch_subresource_interception_enabled: false,
            fetch_subresource_interception_resource_type: None,
        })
        .await
        .expect("prepared streaming XML should accept live commit configuration");

    let permit = prepared.issue_commit_permit();
    let (mut page, _, diagnostics, _, pending_download) =
        prepared.commit(permit).await.expect("permit should commit");
    assert!(pending_download.is_none());
    assert_eq!(
        page.devtools_agent_token(),
        prepared_agent,
        "streaming XML commit must attach the agent reserved during prepare"
    );
    tokio::time::timeout(Duration::from_secs(2), &mut side_effect_request_seen)
        .await
        .expect("the XML parser handoff should run after commit")
        .expect("author fetch observation should stay open");
    release_side_effect_response
        .send(())
        .expect("release author fetch response");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__nativeCommitObserved".to_owned(),
            await_promise: false,
        })
        .await
        .expect("XML author-observed configuration should evaluate");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!(r#"["ready","function"]"#))
    );
    let world_context_id = diagnostics
        .initial_runtime_realms
        .iter()
        .find(|realm| realm.name == "native-world")
        .map(|realm| realm.context_id)
        .expect("the streaming XML named world should exist at initial commit");
    let (world_marker, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: world_context_id,
            expression: "globalThis.__nativeWorldPreload".to_owned(),
            await_promise: false,
        })
        .await
        .expect("streaming XML named-world preload marker should evaluate");
    assert_eq!(
        renderer_json_value(world_marker),
        Some(serde_json::json!("ready"))
    );

    page.close_async()
        .await
        .expect("committed streaming XML page should close");
    server
        .await
        .expect("NativeDom author side-effect server should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_lifecycle_best_effort_observes_autonomous_page_turns() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://child-lifecycle-observer.test/page").unwrap();
    let mut page = create_test_html_page(&runtime, &loader, url, "<!doctype html>").await;

    page.run_async_command(RendererPageCommand::EvaluateExpression {
        expression: r#"
(() => {
  globalThis.__childLifecycleObserverEvents = [];
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childLifecycleObserverEvents.push("frameload");
  frame.srcdoc = `<script>
    parent.__childLifecycleObserverEvents.push("child-script:" + (globalThis === self));
  <\/script>`;
  document.body.appendChild(frame);
  return true;
})()
"#
        .to_owned(),
        await_promise: false,
    })
    .await
    .expect("child lifecycle setup should evaluate");

    let (reply, _) = page
        .run_async_command(
            RendererPageCommand::CompleteChildFrameLifecycleWorkBestEffort {
                timeout_ms: 2_000,
                loader: loader.clone(),
            },
        )
        .await
        .expect("child lifecycle observer should finish");
    assert!(
        matches!(reply, RendererPageReply::Bool(true)),
        "owner-scheduled child work should complete before the observer deadline"
    );

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "__childLifecycleObserverEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("child lifecycle results should remain observable");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("child-script:true|frameload"))
    );

    page.close_async()
        .await
        .expect("child lifecycle observer test page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_ignored_child_navigation_releases_parent_load() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (download_base_url, download_server) = spawn_owner_wake_server_with_content_type(
        "/download.asis",
        "download me",
        "application/octet-stream",
        Duration::ZERO,
    )
    .await;
    let (completion_base_url, completion_server) = spawn_owner_wake_server_with_content_type(
        "/ignored-child-navigation-complete",
        "ok",
        "text/plain; charset=utf-8",
        Duration::ZERO,
    )
    .await;
    let download_url = format!("{download_base_url}/download.asis");
    let completion_url = format!("{completion_base_url}/ignored-child-navigation-complete");
    let page_url = url::Url::parse(&format!("{completion_base_url}/page")).expect("page URL");
    let html = format!(
        r#"<!doctype html><body><script>
globalThis.__ignoredChildNavigationEvents = [];
globalThis.__ignoredChildNavigationCompletionRequested = false;
const maybeCompleteIgnoredChildNavigation = () => {{
  const events = globalThis.__ignoredChildNavigationEvents;
  if (!globalThis.__ignoredChildNavigationCompletionRequested &&
      events.includes("timer") && events.includes("parent-load")) {{
    globalThis.__ignoredChildNavigationCompletionRequested = true;
    fetch({completion_url_literal});
  }}
}};
addEventListener("load", () => {{
  globalThis.__ignoredChildNavigationEvents.push("parent-load");
  maybeCompleteIgnoredChildNavigation();
}});
setTimeout(() => {{
  globalThis.__ignoredChildNavigationEvents.push("timer");
  maybeCompleteIgnoredChildNavigation();
}}, 0);
const frame = document.createElement("iframe");
frame.id = "download-frame";
frame.src = {download_url_literal};
document.body.appendChild(frame);
</script></body>"#,
        completion_url_literal =
            serde_json::to_string(&completion_url).expect("serialize completion URL"),
        download_url_literal =
            serde_json::to_string(&download_url).expect("serialize download URL"),
    );
    let mut page =
        create_test_html_page_at_document_commit(&runtime, &loader, page_url, &html).await;

    tokio::time::timeout(Duration::from_secs(2), download_server)
        .await
        .expect("unsupported child response should be requested")
        .expect("unsupported child response server should finish");
    if let Err(error) = tokio::time::timeout(Duration::from_secs(2), completion_server).await {
        let (state, _) = page
            .run_async_command(RendererPageCommand::EvaluateExpression {
                expression: format!(
                    r#"JSON.stringify({{
  readyState: document.readyState,
  childUrl: document.getElementById("download-frame").contentDocument.URL,
  resourceEntries: performance.getEntriesByType("resource")
    .filter(entry => entry.name === {download_url_literal}).length,
  events: globalThis.__ignoredChildNavigationEvents
}})"#,
                    download_url_literal = serde_json::to_string(&download_url)
                        .expect("serialize diagnostic download URL"),
                ),
                await_promise: false,
            })
            .await
            .expect("timed-out ignored-navigation state should remain observable");
        panic!(
            "ignored child navigation should autonomously release parent load: {error:?}; state={:?}",
            renderer_json_value(state)
        );
    }

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                r#"JSON.stringify({{
  readyState: document.readyState,
  childUrl: document.getElementById("download-frame").contentDocument.URL,
  resourceEntries: performance.getEntriesByType("resource")
    .filter(entry => entry.name === {download_url_literal}).length,
  events: globalThis.__ignoredChildNavigationEvents
}})"#,
                download_url_literal =
                    serde_json::to_string(&download_url).expect("serialize download URL"),
            ),
            await_promise: false,
        })
        .await
        .expect("ignored child navigation outcome should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!(
            r#"{"readyState":"complete","childUrl":"about:blank","resourceEntries":0,"events":["timer","parent-load"]}"#
        ))
    );

    page.close_async()
        .await
        .expect("ignored child navigation page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn javascript_location_assignment_executes_renderer_owned_task_on_delegate_pages() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://javascript-location-ghost.test/start.html").unwrap();
    let mut page = create_test_html_page_with_navigation_dispatch(
        &runtime,
        &loader,
        url,
        "<!doctype html><title>start</title>",
        RendererTopLevelNavigationDispatch::DelegateToBrowser,
    )
    .await;

    let (set_reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
globalThis.__javascriptLocationDone = new Promise(resolve => {
  globalThis.__resolveJavascriptLocation = resolve;
});
location.href = "javascript:document.title = 'LOC-RAN'; globalThis.__resolveJavascriptLocation('ran'); void 0";
"set"
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("javascript: location assignment should complete");
    assert_eq!(
        renderer_json_value(set_reply),
        Some(serde_json::json!("set"))
    );

    let (href_reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "location.href".to_owned(),
            await_promise: false,
        })
        .await
        .expect("location.href readback should complete");
    assert_eq!(
        renderer_json_value(href_reply),
        Some(serde_json::json!(
            "https://javascript-location-ghost.test/start.html"
        )),
        "assigning a javascript: URL must not ghost location.href"
    );

    let (task_reply, _) = tokio::time::timeout(
        Duration::from_secs(2),
        page.run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__javascriptLocationDone".to_owned(),
            await_promise: true,
        }),
    )
    .await
    .expect("renderer-owned javascript: task should settle its promise")
    .expect("javascript: task promise should evaluate");
    assert_eq!(
        renderer_json_value(task_reply),
        Some(serde_json::json!("ran")),
        "the javascript: task must run after the assigning command"
    );

    let (title_reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "document.title".to_owned(),
            await_promise: false,
        })
        .await
        .expect("document.title readback should complete");
    assert_eq!(
        renderer_json_value(title_reply),
        Some(serde_json::json!("LOC-RAN"))
    );

    page.close_async()
        .await
        .expect("javascript location ghost page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_continues_from_stale_to_latest_child_navigation_generation() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, completion_server) = spawn_owner_wake_server_with_content_type(
        "/latest-child-navigation-complete",
        "ok",
        "text/plain; charset=utf-8",
        Duration::ZERO,
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let html = r#"<!doctype html><body><script>
globalThis.__rapidChildNavigationLoads = 0;
const frame = document.createElement("iframe");
frame.id = "rapid-child-navigation";
frame.onload = () => {
  globalThis.__rapidChildNavigationLoads++;
  if (frame.contentDocument.body.textContent.trim() === "latest") {
    fetch("/latest-child-navigation-complete");
  }
};
frame.srcdoc = "<!doctype html><body>superseded</body>";
document.body.appendChild(frame);
frame.srcdoc = "<!doctype html><body>latest</body>";
</script></body>"#;
    let mut page =
        create_test_html_page_at_document_commit(&runtime, &loader, page_url, html).await;

    tokio::time::timeout(Duration::from_secs(2), completion_server)
        .await
        .expect(
            "the latest child generation should run after a stale FIFO head without another command",
        )
        .expect("latest child navigation completion server should finish");
    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify({
  text: document.getElementById("rapid-child-navigation").contentDocument.body.textContent.trim(),
  loads: globalThis.__rapidChildNavigationLoads
})"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("latest child navigation state should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!(r#"{"text":"latest","loads":1}"#))
    );

    page.close_async()
        .await
        .expect("rapid child navigation test page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_completes_window_load_after_child_self_navigation() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, completion_server) = spawn_owner_wake_server_with_content_type(
        "/owner-child-self-navigation-load-complete",
        "ok",
        "text/plain; charset=utf-8",
        Duration::ZERO,
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let html = r#"<!doctype html><body><script>
globalThis.__childSelfNavigateEvents = [];
globalThis.__childSelfNavigateCompletionRequested = false;
const maybeCompleteChildSelfNavigation = () => {
  const events = globalThis.__childSelfNavigateEvents;
  if (!globalThis.__childSelfNavigateCompletionRequested &&
      events.includes("message:can navigate") &&
      events.includes("parent-load")) {
    globalThis.__childSelfNavigateCompletionRequested = true;
    fetch("/owner-child-self-navigation-load-complete");
  }
};
onmessage = event => {
  globalThis.__childSelfNavigateEvents.push(`message:${event.data}`);
  maybeCompleteChildSelfNavigation();
};
addEventListener("load", () => {
  globalThis.__childSelfNavigateEvents.push("parent-load");
  maybeCompleteChildSelfNavigation();
});
const frame = document.createElement("iframe");
frame.sandbox = "allow-scripts";
frame.srcdoc = `
  <!doctype html>
  <script>
    onload = () => {
      location.href = "data:text/html,<!doctype html><script>parent.postMessage('can navigate', '*')<\\/script>";
    };
  <\/script>`;
document.body.appendChild(frame);
</script></body>"#;
    let mut page =
        create_test_html_page_at_document_commit(&runtime, &loader, page_url, html).await;

    match tokio::time::timeout(Duration::from_secs(2), completion_server).await {
        Ok(server) => server.expect("child self-navigation completion server should finish"),
        Err(error) => {
            let (state, _) = page
                .run_async_command(RendererPageCommand::EvaluateExpression {
                    expression: r#"JSON.stringify({
  events: globalThis.__childSelfNavigateEvents,
  readyState: document.readyState,
  frameCount: document.querySelectorAll('iframe').length
})"#
                    .to_owned(),
                    await_promise: false,
                })
                .await
                .expect("timed-out child self-navigation state should remain observable");
            panic!(
                "child self-navigation and parent load should complete without another command: {error:?}; state={:?}",
                renderer_json_value(state)
            );
        }
    }
    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__childSelfNavigateEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("child self-navigation events should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("message:can navigate|parent-load"))
    );

    page.close_async()
        .await
        .expect("child self-navigation page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_completes_window_load_after_child_descendant_navigation() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, completion_server) = spawn_owner_wake_server_with_content_type(
        "/owner-child-descendant-navigation-load-complete",
        "ok",
        "text/plain; charset=utf-8",
        Duration::ZERO,
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let html = r#"<!doctype html><body><script>
globalThis.__childDescendantNavigateEvents = [];
globalThis.__childDescendantNavigateCompletionRequested = false;
globalThis.maybeCompleteChildDescendantNavigation = () => {
  const events = globalThis.__childDescendantNavigateEvents;
  if (!globalThis.__childDescendantNavigateCompletionRequested &&
      events.includes("descendant-load") &&
      events.includes("parent-load")) {
    globalThis.__childDescendantNavigateCompletionRequested = true;
    fetch("/owner-child-descendant-navigation-load-complete");
  }
};
addEventListener("load", () => {
  globalThis.__childDescendantNavigateEvents.push("parent-load");
  globalThis.maybeCompleteChildDescendantNavigation();
});
const frame = document.createElement("iframe");
frame.srcdoc = `
  <!doctype html>
  <iframe src="data:text/html,initial"></iframe>
  <script>
    onload = () => {
      const descendant = document.querySelector("iframe");
      descendant.onload = () => {
        parent.__childDescendantNavigateEvents.push("descendant-load");
        parent.maybeCompleteChildDescendantNavigation();
      };
      descendant.contentWindow.location.href = "data:text/html,done";
    };
  <\/script>`;
document.body.appendChild(frame);
</script></body>"#;
    let mut page =
        create_test_html_page_at_document_commit(&runtime, &loader, page_url, html).await;

    tokio::time::timeout(Duration::from_secs(2), completion_server)
        .await
        .expect(
            "child descendant navigation and parent load should complete without another command",
        )
        .expect("child descendant-navigation completion server should finish");
    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__childDescendantNavigateEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("child descendant-navigation events should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("descendant-load|parent-load"))
    );

    page.close_async()
        .await
        .expect("child descendant-navigation page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn cpu_throttling_rate_slows_runtime_evaluate_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/cpu-throttling").unwrap();
    let mut page = create_test_html_page(&runtime, &loader, url, "<!doctype html>").await;

    let (reply, _) = page
        .run_async_command(RendererPageCommand::SetCpuThrottlingRate(3.0))
        .await
        .expect("CPU throttling rate should update live page");
    assert!(matches!(reply, RendererPageReply::Unit));

    let started = std::time::Instant::now();
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
(() => {
  const end = Date.now() + 40;
  while (Date.now() < end) {}
  return true;
})()
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("throttled evaluate should run");
    assert_eq!(renderer_json_value(reply), Some(serde_json::json!(true)));
    assert!(
        started.elapsed() >= Duration::from_millis(75),
        "rate=3 should add renderer-side delay to a CPU-bound evaluate command"
    );

    page.close_async()
        .await
        .expect("CPU throttling test page should close");
}

#[tokio::test(flavor = "current_thread")]
async fn external_raw_streaming_page_command_builds_phase_one_page() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let requested_url = url::Url::parse("https://example.test/raw-stream").unwrap();
    let final_url = url::Url::parse("https://example.test/raw-stream-final").unwrap();
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let html = "<!doctype html><html><body><main id='external-raw'>外部 raw stream</main><script>document.body.setAttribute('data-streamed','yes')</script></body></html>";
    let split = html.find("raw stream").expect("split marker");
    let first_chunk = html.as_bytes()[..split].to_vec();
    let second_chunk = html.as_bytes()[split..].to_vec();

    let producer = tokio::spawn(async move {
        body_tx
            .send(first_chunk)
            .await
            .expect("first chunk should send");
        body_tx
            .send(second_chunk)
            .await
            .expect("second chunk should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let (page, snapshot, _creation_diagnostics, _creation_artifacts, pending_download) = runtime
        .create_streaming_raw_page_from_external_body(
            requested_url.clone(),
            final_url.clone(),
            None,
            true,
            1,
            Vec::new(),
            203,
            vec![(
                "content-type".to_owned(),
                "text/html; charset=UTF-8".to_owned(),
            )],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            raw_body,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
            false,
            PageVmInitStage::Load,
            RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            RendererNavigationReplyPolicy::FollowBeforeReply,
        )
        .await
        .expect("external raw streaming page should build");
    producer.await.expect("producer should finish");

    assert!(pending_download.is_none());
    assert_eq!(snapshot.requested_url, requested_url);
    assert_eq!(snapshot.final_url(), &final_url);
    assert_eq!(snapshot.status, 203);
    assert_eq!(snapshot.navigation_redirect_count, 1);
    assert!(snapshot.navigation_redirected);
    let html = serialize_html_for_renderer_page(&page).await;
    assert!(html.contains("id=\"external-raw\""));
    assert!(html.contains("外部 raw stream"));
    assert!(html.contains("data-streamed=\"yes\""));
}

#[tokio::test(flavor = "multi_thread")]
async fn external_raw_streaming_body_failure_preserves_committed_document_and_owner() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/failed-main-document")
        .expect("failed main document url");
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    body_tx
        .send(
            b"<!doctype html><main id='partial'>partial body before transport failure</main>"
                .to_vec(),
        )
        .await
        .expect("partial main document body should send");
    let prepared = prepare_test_external_raw_document_with_content_type_and_reply_boundary(
        &runtime,
        &loader,
        url,
        "text/html",
        raw_body,
        RendererReplyBoundary::DocumentCommit,
    )
    .await;
    drop(body_tx);
    completion_tx
        .send(Err(anyhow::anyhow!(
            "synthetic partial main document body failure"
        )))
        .expect("main document body failure should send");
    let permit = prepared.issue_commit_permit();
    let (mut page, _, _, creation_artifacts, pending_download) =
        tokio::time::timeout(Duration::from_secs(5), prepared.commit(permit))
            .await
            .expect("partial main document should reach its response commit boundary")
            .expect("partial main document should attach before its body terminal");
    assert!(pending_download.is_none());
    assert!(
        creation_artifacts.lifecycle_snapshot.load.is_none(),
        "open main document must attach before load"
    );
    page.take_committed_document_post_response_continuation()
        .expect("DocumentCommit should retain parser work until its response boundary")
        .release();

    let failure_events = tokio::time::timeout(Duration::from_secs(5), async {
        let mut events = Vec::new();
        loop {
            let publication = output_rx
                .recv()
                .await
                .expect("renderer output transport should stay open");
            if !publication_is_for_page(&publication, &page) {
                continue;
            }
            let publication_events = publication_document_lifecycle_events(&publication)
                .copied()
                .collect::<Vec<_>>();
            let main_resource_failed = publication_events.iter().any(|event| {
                event.kind
                    == RendererDocumentLifecycleEventKind::Terminated {
                        last_reached: None,
                        reason: super::RendererDocumentTerminationReason::MainResourceLoadFailed,
                    }
            });
            events.extend(publication_events);
            if main_resource_failed {
                return events;
            }
        }
    })
    .await
    .expect("main-resource failure should terminate the parser lifecycle");
    assert!(failure_events.iter().all(|event| {
        event.kind
            != RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            )
            && event.kind
                != RendererDocumentLifecycleEventKind::Milestone(
                    RendererDocumentLifecycleMilestone::Load,
                )
    }));
    assert_eq!(
        runtime.renderer_owner_handle().len(),
        1,
        "a committed partial Document should remain resident like Blink's failed DocumentLoader"
    );

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "document.querySelector('#partial').textContent".to_owned(),
            await_promise: false,
        })
        .await
        .expect("committed partial Document should remain script-observable");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("partial body before transport failure"))
    );
    let (reply, _) = tokio::time::timeout(
        Duration::from_secs(2),
        page.run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "new Promise(resolve => setTimeout(() => resolve(document.readyState), 0))"
                .to_owned(),
            await_promise: true,
        }),
    )
    .await
    .expect("a failed committed Document should keep running ordinary Page tasks")
    .expect("a failed committed Document should resolve a newly scheduled timer");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("loading")),
        "stopping a failed parser must not synthesize DCL or a successful EOF"
    );
    page.close_async()
        .await
        .expect("failed committed page should close normally");

    let recovery_url =
        url::Url::parse("https://example.test/recovery-after-body-failure").expect("recovery url");
    let mut recovery_page = tokio::time::timeout(
        Duration::from_secs(5),
        create_test_html_page(
            &runtime,
            &loader,
            recovery_url,
            "<!doctype html><main id='recovered'>recovered</main>",
        ),
    )
    .await
    .expect("renderer owner should accept a page after the failed candidate");
    let (reply, _) = recovery_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "document.querySelector('#recovered').textContent".to_owned(),
            await_promise: false,
        })
        .await
        .expect("recovery page should remain usable");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("recovered"))
    );
    recovery_page
        .close_async()
        .await
        .expect("recovery page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn prepared_external_raw_document_waits_for_matching_commit_permit() {
    let runtime = JsRuntime::initialize();
    let baseline_isolates = runtime.document_isolate_accounting_for_diagnostics();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, mut side_effect_request_seen, release_side_effect_response, server) =
        spawn_owner_wake_gated_server_with_content_type("/author-side-effect", "ok", "text/plain")
            .await;
    let url = url::Url::parse(&format!("{base_url}/prepared")).expect("prepared url");
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let producer = tokio::spawn(async move {
        body_tx
            .send(
                br#"<!doctype html><script>
globalThis.__preparedAuthorScript = "executed";
localStorage.setItem("prepared-commit", "executed");
fetch("/author-side-effect");
</script><main>prepared</main>"#
                    .to_vec(),
            )
            .await
            .expect("prepared document body should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let prepared = prepare_test_external_raw_document(&runtime, &loader, url, raw_body).await;
    let prepared_agent = prepared.renderer_devtools_agent_token();
    producer
        .await
        .expect("prepared body producer should finish");
    let prepared_isolates = runtime.document_isolate_accounting_for_diagnostics();
    assert_eq!(prepared_isolates.created, baseline_isolates.created + 1);
    assert_eq!(prepared_isolates.live, baseline_isolates.live + 1);
    assert_eq!(prepared_isolates.reserved, baseline_isolates.reserved + 1);
    assert_eq!(
        runtime.renderer_owner_handle().len(),
        0,
        "prepare must not install a Page before the commit permit"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut side_effect_request_seen)
            .await
            .is_err(),
        "author fetch must not run while the prepared document is held"
    );

    let permit = prepared.issue_commit_permit();
    let (mut page, _, _, _, pending_download) =
        prepared.commit(permit).await.expect("permit should commit");
    assert!(pending_download.is_none());
    assert_eq!(
        page.devtools_agent_token(),
        prepared_agent,
        "commit must attach the agent allocated before the permit"
    );
    tokio::time::timeout(Duration::from_secs(2), &mut side_effect_request_seen)
        .await
        .expect("author fetch should run after commit")
        .expect("author fetch observation should stay open");
    release_side_effect_response
        .send(())
        .expect("release author fetch response");

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify([
globalThis.__preparedAuthorScript,
localStorage.getItem("prepared-commit")
])"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("committed author state should evaluate");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!(r#"["executed","executed"]"#))
    );
    page.close_async()
        .await
        .expect("committed prepared page should close");
    server
        .await
        .expect("author side-effect server should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn prepared_document_uses_latest_commit_configuration_before_author_script() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url =
        url::Url::parse("https://example.test/prepared-latest-inspector").expect("prepared url");
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let producer = tokio::spawn(async move {
        body_tx
            .send(
                br#"<!doctype html><script>
globalThis.__preparedCommitObserved = JSON.stringify([
  globalThis.__latestPreload,
  typeof latestBinding,
  Notification.permission
]);
</script>"#
                    .to_vec(),
            )
            .await
            .expect("prepared body should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let prepared = prepare_test_external_raw_document(&runtime, &loader, url, raw_body).await;
    producer
        .await
        .expect("prepared body producer should finish");
    prepared
        .update_commit_configuration(RendererPreparedDocumentCommitConfiguration {
            document_start_scripts: vec![
                crate::DocumentStartScript {
                    registry_key: None,
                    devtools_session: None,
                    source: r#"globalThis.__latestPreload = "ready";"#.to_owned(),
                    world_name: None,
                    has_bidi_channel_argument: false,
                    bidi_channel_handoffs: Vec::new(),
                },
                crate::DocumentStartScript {
                    registry_key: None,
                    devtools_session: None,
                    source: r#"globalThis.__latestWorld = "ready";"#.to_owned(),
                    world_name: Some("latest-world".to_owned()),
                    has_bidi_channel_argument: false,
                    bidi_channel_handoffs: Vec::new(),
                },
            ],
            runtime_bindings: vec![crate::protocol_types::RuntimeBindingRegistration {
                devtools_session: None,
                name: "latestBinding".to_owned(),
                execution_context_name: None,
            }],
            runtime_inspector_session_restore_snapshots: vec![
                RendererInspectorSessionRestoreSnapshot {
                    protocol_configuration: RendererInspectorProtocolConfiguration {
                        runtime_frontend_enabled: true,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ],
            runtime_isolated_worlds: vec![crate::protocol_types::RuntimeIsolatedWorldDefinition {
                name: "latest-world".to_owned(),
                grant_universal_access: false,
            }],
            permission_overrides: vec![crate::protocol_types::PermissionOverrideRegistration {
                permission: serde_json::Value::String("notifications".to_owned()),
                setting: "granted".to_owned(),
                origin: None,
                embedded_origin: None,
            }],
            extra_http_headers: Vec::new(),
            locale_override: None,
            timezone_override: None,
            script_execution_disabled: false,
            bypass_content_security_policy: false,
            cpu_throttling_rate: 1.0,
            emulated_media: Default::default(),
            idle_override: None,
            viewport_surface: None,
            browser_resource_runtime: loader.browser_resource_runtime(),
            navigator_identity: loader.browser_identity().clone(),
            network_offline: false,
            bypass_service_worker: false,
            cache_disabled: false,
            blocked_url_patterns: Vec::new(),
            fetch_subresource_interception_enabled: false,
            fetch_subresource_interception_resource_type: None,
        })
        .await
        .expect("prepared document should accept the latest commit configuration");

    let permit = prepared.issue_commit_permit();
    let (mut page, _, diagnostics, _, pending_download) =
        prepared.commit(permit).await.expect("permit should commit");
    assert!(pending_download.is_none());
    assert!(
        diagnostics.renderer_output_predecessor.is_some(),
        "the commit-time Runtime enable/context output must be published through the concrete Page stream"
    );
    assert!(
        diagnostics
            .initial_runtime_realms
            .iter()
            .any(|realm| realm.is_default),
        "the committed Page must expose its authoritative default-realm inventory"
    );
    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__preparedCommitObserved".to_owned(),
            await_promise: false,
        })
        .await
        .expect("author-observed commit configuration should evaluate");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!(r#"["ready","function","granted"]"#))
    );
    let world_context_id = diagnostics
        .initial_runtime_realms
        .iter()
        .find(|realm| realm.name == "latest-world")
        .map(|realm| realm.context_id)
        .expect("commit-time named world should register before initial diagnostics complete");
    let (world_marker, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: world_context_id,
            expression: "globalThis.__latestWorld".to_owned(),
            await_promise: false,
        })
        .await
        .expect("commit-time named-world preload marker should evaluate");
    assert_eq!(
        renderer_json_value(world_marker),
        Some(serde_json::json!("ready"))
    );

    page.close_async()
        .await
        .expect("committed prepared page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn prepared_document_rejects_a_peer_commit_permit_without_consuming_its_owner() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url =
        url::Url::parse("https://example.test/prepared-first").expect("first prepared url");
    let second_url =
        url::Url::parse("https://example.test/prepared-second").expect("second prepared url");
    let (first_completion_tx, first_completion_rx) = oneshot::channel();
    let (first_body_tx, first_raw_body) =
        ExternalRawDocumentBodyStream::channel(first_completion_rx);
    let first_producer = tokio::spawn(async move {
        first_body_tx
            .send(b"<!doctype html><main id='first'>first</main>".to_vec())
            .await
            .expect("first prepared body should send");
        drop(first_body_tx);
        first_completion_tx
            .send(Ok(()))
            .expect("first completion should send");
    });
    let (second_completion_tx, second_completion_rx) = oneshot::channel();
    let (second_body_tx, second_raw_body) =
        ExternalRawDocumentBodyStream::channel(second_completion_rx);
    let second_producer = tokio::spawn(async move {
        second_body_tx
            .send(b"<!doctype html><main id='second'>second</main>".to_vec())
            .await
            .expect("second prepared body should send");
        drop(second_body_tx);
        second_completion_tx
            .send(Ok(()))
            .expect("second completion should send");
    });

    let first =
        prepare_test_external_raw_document(&runtime, &loader, first_url, first_raw_body).await;
    let second =
        prepare_test_external_raw_document(&runtime, &loader, second_url, second_raw_body).await;
    first_producer
        .await
        .expect("first prepared producer should finish");
    second_producer
        .await
        .expect("second prepared producer should finish");

    let first_permit = first.issue_commit_permit();
    let mismatch = second.commit(first_permit).await;
    assert!(
        mismatch
            .as_ref()
            .is_err_and(|error| error.to_string().contains("does not belong")),
        "a peer permit must be rejected before either residence is consumed"
    );

    let first_permit = first.issue_commit_permit();
    let (mut page, _snapshot, _, _, pending_download) = first
        .commit(first_permit)
        .await
        .expect("the matching owner should remain committable");
    assert!(pending_download.is_none());
    let html = serialize_html_for_renderer_page(&page).await;
    assert!(html.contains("id=\"first\""));
    assert!(!html.contains("id=\"second\""));
    page.close_async()
        .await
        .expect("matching prepared page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn canceled_prepared_external_raw_document_has_no_author_side_effects() {
    let runtime = JsRuntime::initialize();
    let baseline_isolates = runtime.document_isolate_accounting_for_diagnostics();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, mut side_effect_request_seen, _release_side_effect_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/canceled-author-side-effect",
            "ok",
            "text/plain",
        )
        .await;
    let url = url::Url::parse(&format!("{base_url}/canceled")).expect("canceled url");
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let producer = tokio::spawn(async move {
        body_tx
            .send(
                br#"<!doctype html><script>
globalThis.__canceledPreparedAuthorScript = "executed";
localStorage.setItem("prepared-cancel", "executed");
fetch("/canceled-author-side-effect");
</script><main>cancel me</main>"#
                    .to_vec(),
            )
            .await
            .expect("canceled prepared document body should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let prepared =
        prepare_test_external_raw_document(&runtime, &loader, url.clone(), raw_body).await;
    producer
        .await
        .expect("canceled body producer should finish");
    let prepared_isolates = runtime.document_isolate_accounting_for_diagnostics();
    assert_eq!(prepared_isolates.created, baseline_isolates.created + 1);
    assert_eq!(prepared_isolates.live, baseline_isolates.live + 1);
    assert_eq!(prepared_isolates.reserved, baseline_isolates.reserved + 1);
    prepared
        .cancel()
        .await
        .expect("prepared document should cancel");
    let canceled_isolates = runtime.document_isolate_accounting_for_diagnostics();
    assert_eq!(canceled_isolates.live, baseline_isolates.live);
    assert_eq!(canceled_isolates.reserved, baseline_isolates.reserved);
    assert_eq!(canceled_isolates.destroyed, baseline_isolates.destroyed + 1);
    assert_eq!(
        runtime.renderer_owner_handle().len(),
        0,
        "cancel must not install a Page"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut side_effect_request_seen)
            .await
            .is_err(),
        "canceled author fetch must never start"
    );

    let probe_url = url.join("/probe").expect("same-origin probe url");
    let mut probe = create_test_html_page(
        &runtime,
        &loader,
        probe_url,
        "<!doctype html><main>probe</main>",
    )
    .await;
    let (reply, _) = probe
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify([
typeof globalThis.__canceledPreparedAuthorScript,
localStorage.getItem("prepared-cancel")
])"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("same-origin cancellation probe should evaluate");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!(r#"["undefined",null]"#)),
        "cancel must leave neither JS nor storage side effects"
    );
    probe
        .close_async()
        .await
        .expect("cancellation probe should close");
    server.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn canceled_prepared_document_closes_its_ordered_output_stream() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/prepared-output-cancel")
        .expect("prepared output URL");
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    drop(body_tx);
    completion_tx
        .send(Ok(()))
        .expect("prepared output completion should send");

    let prepared = prepare_test_external_raw_document(&runtime, &loader, url, raw_body).await;
    let token = prepared.token();
    let expected_residence = RendererOutputResidenceIdentity::Page {
        owner_local_host_id: token.local_host_id(),
        page_id: token.page_id(),
    };
    let opened_stream = match output_rx.recv_message().await {
        RendererOutputTransportMessage::StreamControl(
            super::RendererOutputStreamControl::Opened { stream },
        ) => stream,
        other => panic!("prepared isolate reservation must open its stream first, got {other:?}"),
    };
    assert_eq!(opened_stream.residence(), expected_residence);
    assert!(matches!(
        output_rx.recv_message().await,
        RendererOutputTransportMessage::PageReservationReleased {
            owner_local_host_id,
            page_id,
        } if owner_local_host_id == token.local_host_id() && page_id == token.page_id()
    ));

    prepared
        .cancel()
        .await
        .expect("prepared document should cancel");
    assert!(matches!(
        output_rx.recv_message().await,
        RendererOutputTransportMessage::StreamControl(
            super::RendererOutputStreamControl::Closed {
                stream,
                reason: super::RendererOutputStreamCloseReason::ResidenceRetired,
                ..
            },
        ) if stream == opened_stream
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_prepared_external_raw_document_releases_only_its_residence() {
    let runtime = JsRuntime::initialize();
    let baseline_isolates = runtime.document_isolate_accounting_for_diagnostics();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, mut side_effect_request_seen, _release_side_effect_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/dropped-author-side-effect",
            "ok",
            "text/plain",
        )
        .await;
    let dropped_url = url::Url::parse(&format!("{base_url}/dropped")).expect("dropped url");
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let producer = tokio::spawn(async move {
        body_tx
            .send(
                br#"<!doctype html><script>fetch("/dropped-author-side-effect")</script>"#.to_vec(),
            )
            .await
            .expect("dropped prepared body should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });
    let dropped =
        prepare_test_external_raw_document(&runtime, &loader, dropped_url, raw_body).await;
    producer.await.expect("dropped body producer should finish");
    drop(dropped);

    let barrier_url =
        url::Url::parse("https://example.test/prepared-drop-barrier").expect("barrier url");
    let (barrier_completion_tx, barrier_completion_rx) = oneshot::channel();
    let (barrier_body_tx, barrier_raw_body) =
        ExternalRawDocumentBodyStream::channel(barrier_completion_rx);
    drop(barrier_body_tx);
    barrier_completion_tx
        .send(Ok(()))
        .expect("barrier completion should send");
    let barrier =
        prepare_test_external_raw_document(&runtime, &loader, barrier_url, barrier_raw_body).await;
    let after_drop = runtime.document_isolate_accounting_for_diagnostics();
    assert_eq!(after_drop.created, baseline_isolates.created + 2);
    assert_eq!(after_drop.destroyed, baseline_isolates.destroyed + 1);
    assert_eq!(after_drop.live, baseline_isolates.live + 1);
    assert_eq!(after_drop.reserved, baseline_isolates.reserved + 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut side_effect_request_seen)
            .await
            .is_err(),
        "dropping a prepared document must not execute its author body"
    );

    barrier
        .cancel()
        .await
        .expect("barrier prepared document should cancel");
    let after_barrier = runtime.document_isolate_accounting_for_diagnostics();
    assert_eq!(after_barrier.live, baseline_isolates.live);
    assert_eq!(after_barrier.reserved, baseline_isolates.reserved);
    assert_eq!(after_barrier.destroyed, baseline_isolates.destroyed + 2);
    server.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn external_raw_streaming_empty_document_reaches_dom_content_loaded() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/empty.html").unwrap();
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);

    let producer = tokio::spawn(async move {
        body_tx
            .send(b"<!DOCTYPE html>\n<html></html>".to_vec())
            .await
            .expect("empty html chunk should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let (page, snapshot, _creation_diagnostics, _creation_artifacts, pending_download) = runtime
        .create_streaming_raw_page_from_external_body(
            url.clone(),
            url.clone(),
            None,
            false,
            0,
            Vec::new(),
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            raw_body,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
            false,
            PageVmInitStage::DomContentLoaded,
            RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            RendererNavigationReplyPolicy::FollowBeforeReply,
        )
        .await
        .expect("empty external raw streaming page should reach DOMContentLoaded");
    producer.await.expect("producer should finish");

    assert!(pending_download.is_none());
    assert_eq!(snapshot.final_url(), &url);
    assert_eq!(snapshot.status, 200);
    assert!(
        serialize_html_for_renderer_page(&page)
            .await
            .contains("<html")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn same_document_navigation_publication_carries_exact_source_document() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/source.html").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>source</body>",
    )
    .await;

    let (snapshot, _) = page
        .run_async_command(RendererPageCommand::PageDiagnosticsSnapshot)
        .await
        .expect("page diagnostics snapshot should be readable");
    let RendererPageReply::PageDiagnosticsSnapshot(snapshot) = snapshot else {
        panic!("expected page diagnostics snapshot");
    };
    let source_document = snapshot
        .document_lifecycle_identity()
        .expect("attached Page should expose its exact Document identity");
    while output_rx.try_recv().is_ok() {}

    page.enqueue_async_command(RendererPageCommand::EvaluateExpression {
        expression: r##"history.pushState(null, "", "#captured");"done""##.to_owned(),
        await_promise: false,
    })
    .expect("same-Document navigation command should enqueue")
    .wait()
    .await
    .expect("same-Document navigation should execute");
    let navigations = output_rx
        .drain()
        .into_iter()
        .flat_map(RendererOutputPublication::into_records)
        .filter_map(|record| match record.into_parts().1 {
            RendererOutputItem::OwnerAction(RendererOwnerAction::SameDocumentNavigation(
                navigation,
            )) => Some(navigation),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(navigations.len(), 1);
    assert_eq!(navigations[0].source_document(), source_document);
    assert_eq!(
        navigations[0].navigation().url,
        "https://example.test/source.html#captured"
    );
    page.close_async()
        .await
        .expect("same-Document navigation test page should close");
}

#[tokio::test(flavor = "current_thread")]
async fn document_open_preserves_document_sourced_navigation_handoffs() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/source.html").unwrap();
    let mut page = create_test_html_page_with_navigation_dispatch(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>source</body>",
        RendererTopLevelNavigationDispatch::DelegateToBrowser,
    )
    .await;

    let (snapshot, _) = page
        .run_async_command(RendererPageCommand::PageDiagnosticsSnapshot)
        .await
        .expect("source snapshot should be readable");
    let RendererPageReply::PageDiagnosticsSnapshot(snapshot) = snapshot else {
        panic!("expected source activity snapshot");
    };
    let source_document = snapshot
        .document_lifecycle_identity()
        .expect("source Document identity should exist");
    while output_rx.try_recv().is_ok() {}

    page.enqueue_async_command(RendererPageCommand::EvaluateExpression {
        expression: r##"
history.pushState(null, "", "#retired");
location.href = "https://example.test/pending-target.html";
document.open();
document.write("<!doctype html><body>replacement</body>");
document.close();
"done";
"##
        .to_owned(),
        await_promise: false,
    })
    .expect("document.open replacement command should enqueue")
    .wait()
    .await
    .expect("document.open replacement should execute");
    let records = output_rx
        .drain()
        .into_iter()
        .flat_map(RendererOutputPublication::into_records)
        .collect::<Vec<_>>();

    let (snapshot, _) = page
        .run_async_command(RendererPageCommand::PageDiagnosticsSnapshot)
        .await
        .expect("replacement snapshot should be readable");
    let RendererPageReply::PageDiagnosticsSnapshot(snapshot) = snapshot else {
        panic!("expected replacement activity snapshot");
    };
    let replacement_document = snapshot
        .document_lifecycle_identity()
        .expect("replacement Document identity should exist");
    assert_ne!(replacement_document, source_document);

    let navigations = records
        .iter()
        .filter_map(|record| match record.item() {
            RendererOutputItem::OwnerAction(RendererOwnerAction::SameDocumentNavigation(
                navigation,
            )) => Some(navigation),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(navigations.len(), 1);
    assert_eq!(
        navigations[0].source_document(),
        source_document,
        "document.open must not relabel the already-applied history mutation"
    );
    assert_eq!(
        navigations[0].navigation().url,
        "https://example.test/source.html#retired"
    );

    let navigation = records
        .iter()
        .find_map(|record| match record.item() {
            RendererOutputItem::OwnerAction(RendererOwnerAction::TopLevelLocationNavigation(
                navigation,
            )) => Some(navigation),
            _ => None,
        })
        .expect("expected concrete top-level location navigation action");
    assert_eq!(
        navigation.source_document(),
        source_document,
        "location action must retain the producer Document rather than adopt the replacement"
    );
    assert_ne!(navigation.source_document(), replacement_document);
    assert_eq!(navigation.url(), "https://example.test/pending-target.html");
    page.close_async()
        .await
        .expect("document.open navigation identity test page should close");
}

async fn assert_committed_navigation_bootstrap_injection_retires_page(
    injection_header: &'static str,
    expected_failure: &str,
) {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, server) = spawn_owner_wake_server_with_content_type_and_headers(
        "/replacement",
        "<!doctype html><main>replacement</main>",
        "text/html",
        vec![(injection_header, "1")],
        Duration::ZERO,
    )
    .await;
    let source_url = url::Url::parse(&format!("{base_url}/source")).expect("source URL");
    let replacement_url = format!("{base_url}/replacement");
    let replacement_url_literal =
        serde_json::to_string(&replacement_url).expect("serialize replacement URL");
    let html = format!(
        "<!doctype html><script>location.href = {replacement_url_literal};</script><main>source</main>"
    );

    let pending = start_test_html_page_with_optional_indexed_db_manager_and_navigation_dispatch(
        &runtime,
        &loader,
        source_url,
        &html,
        None,
        RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
    );
    let result = pending.await_ready().await;
    let Err(error) = result else {
        panic!("the injected post-commit bootstrap termination must reject page creation")
    };
    let failure = format!("{error:#}");
    assert!(
        failure.contains(expected_failure),
        "page creation must report the injected post-commit failure: {failure}"
    );
    server
        .await
        .expect("post-commit failure server should finish");
    assert_eq!(
        runtime.renderer_page_count_for_testing(),
        0,
        "a committed navigation bootstrap failure must not restore its inert PageVm as live"
    );

    let mut survivor = create_test_html_page(
        &runtime,
        &loader,
        url::Url::parse("about:blank").expect("survivor URL"),
        "<!doctype html><main>survivor</main>",
    )
    .await;
    survivor
        .close_async()
        .await
        .expect("renderer owner should remain usable after retiring the failed navigation");
    assert_eq!(runtime.renderer_page_count_for_testing(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn committed_navigation_bootstrap_failure_retires_page_before_follow_settlement() {
    assert_committed_navigation_bootstrap_injection_retires_page(
        "x-moli-test-fail-after-navigation-commit",
        "injected failure after main navigation commit for testing",
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn committed_navigation_bootstrap_panic_returns_committed_entry_to_owner() {
    assert_committed_navigation_bootstrap_injection_retires_page(
        "x-moli-test-panic-after-navigation-commit",
        "local task panicked before restoring its page entry",
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn external_raw_streaming_delegates_post_load_meta_refresh_to_browser() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/redirect_http_equiv.html").unwrap();
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);

    let producer = tokio::spawn(async move {
        body_tx
            .send(
                br#"<!doctype html><head><meta http-equiv="refresh" content="0;redirected.html"></head>"#
                    .to_vec(),
            )
            .await
            .expect("meta refresh html chunk should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let (page, snapshot, _creation_diagnostics, creation_artifacts, pending_download) = runtime
        .create_streaming_raw_page_from_external_body(
            url.clone(),
            url.clone(),
            None,
            false,
            0,
            Vec::new(),
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            raw_body,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
            false,
            PageVmInitStage::Load,
            RendererTopLevelNavigationDispatch::DelegateToBrowser,
            RendererNavigationReplyPolicy::ReturnWithPendingNavigation,
        )
        .await
        .expect("meta refresh streaming page should attach first document");
    producer.await.expect("producer should finish");
    assert!(pending_download.is_none());
    assert_eq!(snapshot.final_url(), &url);
    assert!(
        creation_artifacts.lifecycle_snapshot.load.is_some(),
        "the source Document must reach load before an immediate refresh comes due"
    );
    assert_eq!(
        creation_artifacts.active_document,
        creation_artifacts.lifecycle_snapshot.document
    );
    assert_eq!(
        creation_artifacts.active_epoch,
        creation_artifacts.lifecycle_snapshot.epoch
    );
    let expected_source_document = creation_artifacts.lifecycle_snapshot.into();
    let navigation = tokio::time::timeout(
        Duration::from_millis(500),
        activity_wake_rx.recv_top_level_location_navigation(),
    )
    .await
    .expect("load and its exact-source meta refresh should publish a concrete navigation action")
    .expect("external activity transport should stay open");
    assert_eq!(navigation.source_document(), expected_source_document);
    assert_eq!(navigation.url(), "https://example.test/redirected.html");
    assert_eq!(
        has_pending_location_navigation_for_test(&page).await,
        Some(false),
        "the browser-owned action must be moved into concrete output instead of remaining mutable Page state"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn external_raw_streaming_defers_dcl_handler_navigation_to_page_reply() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/dcl-handler.html").unwrap();
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let producer = tokio::spawn(async move {
        body_tx
            .send(
                br#"<!doctype html><script>document.addEventListener('DOMContentLoaded', () => location.href = '/next.html', {once:true})</script><main>source</main>"#
                    .to_vec(),
            )
            .await
            .expect("DCL handler document should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let (page, _, _, creation_artifacts, pending_download) = runtime
        .create_streaming_raw_page_from_external_body(
            url.clone(),
            url,
            None,
            false,
            0,
            Vec::new(),
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            raw_body,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
            false,
            PageVmInitStage::DomContentLoaded,
            RendererTopLevelNavigationDispatch::DelegateToBrowser,
            RendererNavigationReplyPolicy::ReturnWithPendingNavigation,
        )
        .await
        .expect("DCL handler page should reach its reply boundary");
    producer.await.expect("producer should finish");

    assert!(pending_download.is_none());
    assert!(
        creation_artifacts
            .lifecycle_snapshot
            .dom_content_loaded
            .is_some()
    );
    let expected_source_document = creation_artifacts.lifecycle_snapshot.into();
    let navigation = tokio::time::timeout(
        Duration::from_millis(500),
        activity_wake_rx.recv_top_level_location_navigation(),
    )
    .await
    .expect("the exact DCL action should publish its concrete browser navigation")
    .expect("external activity transport should stay open");
    assert_eq!(navigation.source_document(), expected_source_document);
    assert_eq!(navigation.url(), "https://example.test/next.html");
    assert_eq!(
        has_pending_location_navigation_for_test(&page).await,
        Some(false),
        "the browser-owned DCL navigation must not remain as mutable Page state"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn external_raw_streaming_dcl_reply_resumes_ordinary_page_work() {
    let runtime = JsRuntime::initialize();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/dcl-resume.html").unwrap();
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let producer = tokio::spawn(async move {
        body_tx
            .send(
                br#"<!doctype html><script>
globalThis.__ordinaryAfterDcl = new Promise(resolve => {
  document.addEventListener('DOMContentLoaded', () => {
    setTimeout(() => resolve('resumed'), 0);
  }, {once: true});
});
</script>"#
                    .to_vec(),
            )
            .await
            .expect("DCL resume document should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let (mut page, _, _, creation_artifacts, pending_download) = runtime
        .create_streaming_raw_page_from_external_body(
            url.clone(),
            url,
            None,
            false,
            0,
            Vec::new(),
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            raw_body,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
            false,
            PageVmInitStage::DomContentLoaded,
            RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            RendererNavigationReplyPolicy::FollowBeforeReply,
        )
        .await
        .expect("page should reach its DCL reply boundary");
    producer.await.expect("producer should finish");

    assert!(pending_download.is_none());
    assert!(
        creation_artifacts
            .lifecycle_snapshot
            .dom_content_loaded
            .is_some()
    );
    let (reply, _) = tokio::time::timeout(
        Duration::from_secs(2),
        page.run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "__ordinaryAfterDcl".to_owned(),
            await_promise: true,
        }),
    )
    .await
    .expect("ordinary work held at the DCL reply boundary should be resumed")
    .expect("DCL continuation result should evaluate");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("resumed"))
    );

    page.close_async()
        .await
        .expect("DCL continuation test page should close");
}

#[tokio::test(flavor = "current_thread")]
async fn renderer_owned_navigation_survives_page_creation_observer_detach() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let source_url = url::Url::parse("https://example.test/renderer-owned-source").unwrap();
    let replacement_url = url::Url::parse(
        "data:text/html,%3Cmain%20id%3D%22replacement%22%3Erenderer-owned%3C%2Fmain%3E",
    )
    .unwrap();
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let replacement_href = replacement_url.as_str().to_owned();
    let producer = tokio::spawn(async move {
        body_tx
            .send(
                format!(
                    "<!doctype html><script>window.addEventListener('load', () => location.href = {replacement_href:?}, {{once:true}})</script><main>source</main>"
                )
                .into_bytes(),
            )
            .await
            .expect("renderer-owned source should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let (mut page, _, _, creation_artifacts, pending_download) = runtime
        .create_streaming_raw_page_from_external_body_with_inspector_session_restores(
            source_url.clone(),
            source_url,
            None,
            false,
            0,
            Vec::new(),
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            raw_body,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
            Vec::new(),
            false,
            PageVmInitStage::Load,
            RendererReplyBoundary::DocumentCommit,
            RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            RendererNavigationReplyPolicy::FollowBeforeReply,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("source should attach at its document commit boundary");
    producer.await.expect("producer should finish");
    assert!(pending_download.is_none());
    assert!(creation_artifacts.lifecycle_snapshot.load.is_none());
    page.take_committed_document_post_response_continuation()
        .expect("DocumentCommit should defer parser continuation")
        .release();

    let initial_document = creation_artifacts.active_document;
    let replacement_document = tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(publication) = activity_wake_rx.recv().await {
            if let Some(document) = publication_document_lifecycle_events(&publication)
                .find(|event| {
                    event.document != initial_document
                        && event.kind
                            == RendererDocumentLifecycleEventKind::Milestone(
                                RendererDocumentLifecycleMilestone::Load,
                            )
                })
                .map(|event| event.document)
            {
                return document;
            }
        }
        panic!("external activity wake channel closed before renderer-owned replacement")
    })
    .await
    .expect("renderer ownership should outlive the detached creation observer");
    assert_ne!(replacement_document, initial_document);

    // A concrete lifecycle record is a source-owned fact, not a wake granting
    // permission to snapshot mutable Page state. The final navigation
    // continuation may still be immediately behind this publication and it
    // explicitly permits one ready command to overtake. Serializing the live
    // replacement Document consumes that bounded overtake; the following
    // owner-state query is therefore ordered after the final commit.
    let html = serialize_html_for_renderer_page(&page).await;
    assert!(html.contains("renderer-owned"));
    let final_snapshot = RendererPageTestingHandle::new_for_testing(&page)
        .current_page_state_async()
        .await
        .expect("replacement commit should refresh the owner Page state");
    assert_eq!(final_snapshot.final_url(), &replacement_url);
    assert_eq!(
        has_pending_location_navigation_for_test(&page).await,
        Some(false)
    );
    page.close_async()
        .await
        .expect("renderer-owned navigation test page should close");
}

#[tokio::test(flavor = "current_thread")]
async fn parser_failed_custom_element_construction_uses_unknown_element_surface() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/parser-failed-custom-element").unwrap();
    let html = r#"<!doctype html>
<body>
<script>
window.onerror = () => true;
globalThis.__ReturnsText = class extends HTMLElement {
  constructor() {
    super();
    return document.createTextNode("text");
  }
};
customElements.define("wpt-parser-returns-text", globalThis.__ReturnsText);
globalThis.__ReturnsObject = class extends HTMLElement {
  constructor() {
    super();
    return {};
  }
};
customElements.define("wpt-parser-returns-object", globalThis.__ReturnsObject);
globalThis.__LacksSuper = class extends HTMLElement {
  constructor() {}
};
customElements.define("wpt-parser-lacks-super", globalThis.__LacksSuper);
globalThis.__ThrowsElement = class extends HTMLElement {
  constructor() {
    throw new Error("boom");
  }
};
customElements.define("wpt-parser-throws", globalThis.__ThrowsElement);
</script>
<wpt-parser-returns-text></wpt-parser-returns-text>
<wpt-parser-returns-object></wpt-parser-returns-object>
<wpt-parser-lacks-super></wpt-parser-lacks-super>
<wpt-parser-throws></wpt-parser-throws>
</body>"#;
    let mut page = create_test_html_page(&runtime, &loader, url, html).await;

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
(() => {
  function summarize(selector, constructor) {
    const element = document.querySelector(selector);
    return [
      element instanceof HTMLElement,
      element instanceof HTMLUnknownElement,
      element instanceof constructor
    ].join(":");
  }
  return [
    summarize("wpt-parser-returns-text", globalThis.__ReturnsText),
    summarize("wpt-parser-returns-object", globalThis.__ReturnsObject),
    summarize("wpt-parser-lacks-super", globalThis.__LacksSuper),
    summarize("wpt-parser-throws", globalThis.__ThrowsElement)
  ].join("|");
})()
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("parser failed custom element fallback surface should evaluate");

    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!(
            "true:true:false|true:true:false|true:true:false|true:true:false"
        ))
    );
    page.close_async()
        .await
        .expect("parser failed custom element page should close");
}

#[tokio::test(flavor = "current_thread")]
async fn parser_custom_element_microtask_mutation_fails_before_validation() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url =
        url::Url::parse("https://example.test/parser-custom-element-microtask-failure").unwrap();
    let html = r#"<!doctype html>
<body>
<script>
window.onerror = () => true;
globalThis.__ParserMicrotaskMutates = class extends HTMLElement {
  constructor() {
    super();
    Promise.resolve().then(() => this.setAttribute("attribute", "value"));
  }
};
customElements.define("wpt-parser-microtask-mutates", globalThis.__ParserMicrotaskMutates);
</script>
<wpt-parser-microtask-mutates></wpt-parser-microtask-mutates>
</body>"#;
    let mut page = create_test_html_page(&runtime, &loader, url, html).await;

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
(() => {
  const element = document.querySelector("wpt-parser-microtask-mutates");
  return [
    element.hasAttribute("attribute"),
    element instanceof HTMLUnknownElement,
    element instanceof globalThis.__ParserMicrotaskMutates
  ].join(":");
})()
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("parser microtask mutation fallback should evaluate");

    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("false:true:false"))
    );
    page.close_async()
        .await
        .expect("parser microtask mutation page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_uses_distinct_isolates_and_isolates_contexts() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-isolate-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-isolate-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first shared</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate page should load");
    assert!(first_download.is_none());
    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    assert_eq!(
        first_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("first shared page unique document isolate count"),
        1
    );

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second shared</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared-isolate page should load");
    assert!(second_download.is_none());
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two per-page attached unique document isolate count"),
        2
    );

    let first_heap = runtime_heap_usage_for_test(&first_page).await;
    let second_heap = runtime_heap_usage_for_test(&second_page).await;
    let first_runtime = &first_heap["moli"]["runtime"];
    let second_runtime = &second_heap["moli"]["runtime"];
    assert_eq!(
        first_runtime["inspectorContextGroupScope"],
        serde_json::json!("local-root-agent"),
        "inspector context-group id should be local-root agent scoped: {first_heap:?}"
    );
    assert_eq!(
        first_runtime["inspectorSessionRegistryOwner"],
        serde_json::json!("renderer-devtools-agent"),
        "page inspector session registry should be owned by the local-root agent: {first_heap:?}"
    );
    assert_eq!(
        first_runtime["inspectorSessionRegistryLifetimeScope"],
        serde_json::json!("local-root-agent"),
        "page inspector session registry should be local-root agent scoped: {first_heap:?}"
    );
    assert_eq!(
        first_runtime["inspectorSessionCount"],
        serde_json::json!(1),
        "default page inspector session should be created at bootstrap: {first_heap:?}"
    );
    assert_eq!(
        first_runtime["inspectorDefaultContextRegistryScope"],
        serde_json::json!("page-vm-document-isolate"),
        "default context registry should be page-isolate scoped: {first_heap:?}"
    );
    assert_eq!(
        first_runtime["v8ForegroundTaskWakeScope"],
        serde_json::json!("page-vm-document-isolate"),
        "V8 foreground task wakes should be labelled as page-isolate scoped: {first_heap:?}"
    );
    assert_eq!(
        first_runtime["v8ForegroundTaskWakeContextGroupIdAvailable"],
        serde_json::json!(false),
        "V8 foreground task wakes should not claim a context-group id: {first_heap:?}"
    );
    assert_eq!(
        first_runtime["v8ForegroundTaskWakeInternalPolicy"],
        serde_json::json!("typed-page-source-and-owner-scheduler"),
        "page document isolate must expose the typed Page-source policy: {first_heap:?}"
    );
    assert_eq!(
        first_runtime["v8ForegroundTaskWakeExternalPolicy"],
        serde_json::json!("post-turn-runtime-output"),
        "external observation must follow the completed owner turn: {first_heap:?}"
    );
    let first_context_group_id = first_runtime["inspectorContextGroupId"]
        .as_i64()
        .expect("first heap usage should report inspector context group id");
    let second_context_group_id = second_runtime["inspectorContextGroupId"]
        .as_i64()
        .expect("second heap usage should report inspector context group id");
    assert!(first_context_group_id > 0);
    assert!(second_context_group_id > 0);
    assert_ne!(
        first_context_group_id, second_context_group_id,
        "distinct page document isolates must expose distinct page inspector context groups"
    );
    assert_eq!(
        first_runtime["inspectorDefaultContextRegistryCount"],
        serde_json::json!(1),
        "first isolate backend should retain only its page default context: {first_heap:?}"
    );
    assert_eq!(
        second_runtime["inspectorDefaultContextRegistryCount"],
        serde_json::json!(1),
        "second isolate backend should retain only its page default context: {second_heap:?}"
    );
    assert_eq!(
        first_runtime["inspectorContextRegistrationCount"],
        serde_json::json!(1),
        "first document should own one default Inspector context registration: {first_heap:?}"
    );
    assert_eq!(
        second_runtime["inspectorContextRegistrationCount"],
        serde_json::json!(1),
        "second document should own an independent default Inspector context registration: {second_heap:?}"
    );

    let (first_marker, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                r#"globalThis.__lm_shared_isolate_marker = "first"; globalThis.__lm_shared_isolate_marker"#
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("first page marker evaluate should run");
    assert_eq!(
        renderer_json_value(first_marker),
        Some(serde_json::json!("first"))
    );

    let (missing_marker, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_shared_isolate_marker ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page marker read should run");
    assert_eq!(
        renderer_json_value(missing_marker),
        Some(serde_json::json!("missing"))
    );

    let (second_marker, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                r#"globalThis.__lm_shared_isolate_marker = "second"; globalThis.__lm_shared_isolate_marker"#
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page marker evaluate should run");
    assert_eq!(
        renderer_json_value(second_marker),
        Some(serde_json::json!("second"))
    );

    let (first_marker_after_second_write, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_shared_isolate_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first page marker reread should run");
    assert_eq!(
        renderer_json_value(first_marker_after_second_write),
        Some(serde_json::json!("first"))
    );

    first_page
        .close_async()
        .await
        .expect("first shared page should close");
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("remaining shared unique document isolate count"),
        1
    );
    let second_heap_after_first_close = runtime_heap_usage_for_test(&second_page).await;
    let second_runtime_after_first_close = &second_heap_after_first_close["moli"]["runtime"];
    assert_eq!(
        second_runtime_after_first_close["inspectorContextGroupId"],
        serde_json::json!(second_context_group_id),
        "remaining page should keep its own inspector context group after peer close"
    );
    assert_eq!(
        second_runtime_after_first_close["inspectorDefaultContextRegistryCount"],
        serde_json::json!(1),
        "closing peer page must not affect the remaining isolate registry: {second_heap_after_first_close:?}"
    );
    assert_eq!(
        second_runtime_after_first_close["inspectorContextRegistrationCount"],
        serde_json::json!(1),
        "closing a peer document must not release the remaining document's registration"
    );

    let (observed, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(async () => {
  try {
    await WebAssembly.compile(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]));
    return "compiled";
  } catch (error) {
    return "error:" + error.name;
  }
})()"#
                .to_owned(),
            await_promise: true,
        })
        .await
        .expect("remaining Page isolate should complete async wasm compilation");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("compiled")),
        "retiring a peer Page must not break the remaining Page's typed V8 route"
    );

    second_page
        .close_async()
        .await
        .expect("second shared page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn replacement_navigation_releases_old_inspector_deferred_response_callbacks() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/deferred-navigation-source").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>deferred navigation source</body>",
    )
    .await;
    let mut pending_responses = Vec::new();
    for index in 0..12 {
        let call_id = 710_001 + index;
        let inspector_session_id = (index % 2 == 1).then(|| "session-a".to_owned());
        let (response_tx, mut response_rx) = oneshot::channel();
        let (dispatch, _) = page
            .run_async_command(
                RendererPageCommand::dispatch_runtime_protocol_message_with_deferred_response(
                    inspector_session_id,
                    serde_json::json!({
                        "id": call_id,
                        "method": "Runtime.evaluate",
                        "params": {
                            "expression": format!(
                                "(globalThis.__pendingInspectorPromises ??= [], globalThis.__pendingInspectorPromises[{index}] = new Promise(() => {{}}))"
                            ),
                            "awaitPromise": true,
                        },
                    })
                    .to_string(),
                    RendererRuntimeInspectorResponseSender::new(
                        call_id,
                        response_tx,
                    ),
                ),
            )
            .await
            .expect("never-settling Runtime.evaluate should register a deferred callback");
        assert!(matches!(
            dispatch,
            RendererPageReply::RuntimeInspectorProtocolMessages(ref messages) if messages.is_empty()
        ));
        assert!(matches!(
            response_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        pending_responses.push((call_id, response_rx));
    }

    let replacement_url = "data:text/html,<!doctype html><body>replacement</body>";
    page.run_async_command(
        RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
            expression: format!("location.href = {replacement_url:?}; 'navigating'"),
            await_promise: false,
        },
    )
    .await
    .expect("renderer-owned navigation should install the replacement PageVm");

    for (call_id, mut response_rx) in pending_responses {
        let completion = response_rx
            .try_recv()
            .expect("V8 teardown should finish each old deferred command");
        assert_eq!(completion.call_id, call_id);
        let response = completion
            .output
            .protocol_response(call_id)
            .expect("teardown completion should contain its protocol response");
        assert_eq!(response["id"], serde_json::json!(call_id));
        assert_eq!(response["error"]["code"], serde_json::json!(-32000));
        assert_eq!(
            response["error"]["message"],
            serde_json::json!("Execution context was destroyed.")
        );
    }

    let reused_call_id = 710_001;
    let (replacement_response_tx, mut replacement_response_rx) = oneshot::channel();
    let replacement_completion = page
        .enqueue_async_command(
            RendererPageCommand::dispatch_runtime_protocol_message_with_deferred_response(
                Some("session-a".to_owned()),
                serde_json::json!({
                    "id": reused_call_id,
                    "method": "Runtime.evaluate",
                    "params": {
                        "expression": "42",
                        "awaitPromise": true,
                        "returnByValue": true,
                    },
                })
                .to_string(),
                RendererRuntimeInspectorResponseSender::new(
                    reused_call_id,
                    replacement_response_tx,
                ),
            ),
        )
        .expect("replacement PageVM command should enqueue")
        .wait()
        .await
        .expect("replacement PageVM should accept a reused frontend call id");
    let (replacement_completion, _renderer_output_predecessor) =
        replacement_completion.into_completion_and_predecessor();
    let replacement_response = replacement_completion
        .runtime_inspector_output()
        .and_then(|output| output.protocol_response(reused_call_id))
        .expect("synchronous replacement response should stay in the command completion");
    assert_eq!(
        replacement_response["result"]["result"]["value"],
        serde_json::json!(42)
    );
    let (replacement_dispatch, _, _) = replacement_completion.into_parts();
    assert!(matches!(
        replacement_dispatch,
        RendererPageReply::RuntimeInspectorProtocolMessages(ref messages) if !messages.is_empty()
    ));
    assert!(matches!(
        replacement_response_rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Closed)
    ));
    page.close_async()
        .await
        .expect("replacement page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_binding_replay_cannot_consume_same_id_frontend_deferred_response() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/inspector-internal-id-collision").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>inspector internal id collision</body>",
    )
    .await;

    let colliding_call_id = 900_100_000;
    let (response_tx, mut response_rx) = oneshot::channel();
    let (dispatch, _) = page
        .run_async_command(
            RendererPageCommand::dispatch_runtime_protocol_message_with_deferred_response(
                None,
                serde_json::json!({
                    "id": colliding_call_id,
                    "method": "Runtime.evaluate",
                    "params": {
                        "expression": "new Promise(resolve => { globalThis.__resolveInspectorCollision = resolve; })",
                        "awaitPromise": true,
                        "returnByValue": true,
                    },
                })
                .to_string(),
                RendererRuntimeInspectorResponseSender::new(
                    colliding_call_id,
                    response_tx,
                ),
            ),
        )
        .await
        .expect("frontend awaitPromise should remain deferred");
    assert!(matches!(
        dispatch,
        RendererPageReply::RuntimeInspectorProtocolMessages(ref messages) if messages.is_empty()
    ));

    let binding = crate::protocol_types::RuntimeBindingRegistration {
        devtools_session: None,
        name: "internalCollisionBinding".to_owned(),
        execution_context_name: None,
    };
    set_runtime_binding_state_for_test(&page, None, Vec::new(), vec![binding])
        .await
        .expect("runtime binding state should become pending for replay");
    let replay_trigger_messages = dispatch_runtime_protocol_for_test(
        &page,
        serde_json::json!({
            "id": 710_100,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "typeof internalCollisionBinding",
                "returnByValue": true,
            },
        }),
    )
    .await
    .expect("a later frontend dispatch should replay the new binding");
    assert_eq!(
        runtime_protocol_response_by_id(&replay_trigger_messages, 710_100)
            .expect("replay trigger response")["result"]["result"]["value"],
        serde_json::json!("function")
    );
    assert!(
        matches!(
            response_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ),
        "the internal Runtime.addBinding response must not complete the same-id frontend await"
    );

    let (resolved, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "__resolveInspectorCollision('frontend-result'); 'resolved'".to_owned(),
            await_promise: false,
        })
        .await
        .expect("the page should resolve the original frontend promise");
    assert_eq!(
        renderer_json_value(resolved),
        Some(serde_json::json!("resolved"))
    );
    let completion = tokio::time::timeout(Duration::from_secs(2), &mut response_rx)
        .await
        .expect("the frontend promise response publication should not stall")
        .expect("the frontend promise response should retain its callback owner");
    assert_eq!(completion.call_id, colliding_call_id);
    let response = completion
        .output
        .protocol_response(colliding_call_id)
        .expect("frontend completion should contain its protocol response");
    assert_eq!(
        response["result"]["result"]["value"],
        serde_json::json!("frontend-result")
    );

    page.close_async()
        .await
        .expect("internal collision test page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn page_document_isolate_completes_real_v8_foreground_task() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/shared-v8-foreground-task").unwrap();

    let (page, _, _, _creation_artifacts, pending_download) = runtime
        .create_html_page_from_response(
            page_url.clone(),
            page_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>shared v8 foreground task</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("shared-isolate page should load");
    assert!(pending_download.is_none());

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(async () => {
  try {
    await WebAssembly.compile(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]));
    return "compiled";
  } catch (error) {
    return "error:" + error.name;
  }
})()"#
                .to_owned(),
            await_promise: true,
        })
        .await
        .expect("wasm compilation should finish through the Page isolate foreground-task route");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("compiled")),
        "the Page isolate must remain routable while V8 completes foreground work"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_routes_unhandled_rejections_to_originating_page() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-rejection-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-rejection-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first rejection listener</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second rejection listener</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared-isolate page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );

    for page in [&first_page, &second_page] {
        let (installed, _) = page
            .run_async_command(RendererPageCommand::EvaluateExpression {
                expression: r#"(() => {
  globalThis.__lm_shared_isolate_unhandled_rejections = [];
  globalThis.__lm_shared_isolate_rejectionhandled = [];
  addEventListener("unhandledrejection", event => {
    event.preventDefault();
    globalThis.__lm_shared_isolate_unhandled_rejections.push(String(event.reason));
  });
  addEventListener("rejectionhandled", event => {
    globalThis.__lm_shared_isolate_rejectionhandled.push(
      event.promise === globalThis.__lm_shared_isolate_late_rejection
        ? "same-promise"
        : "wrong-promise"
    );
  });
  return "installed";
})()"#
                    .to_owned(),
                await_promise: false,
            })
            .await
            .expect("unhandledrejection listener install should run");
        assert_eq!(
            renderer_json_value(installed),
            Some(serde_json::json!("installed"))
        );
    }

    let (scheduled, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_shared_isolate_late_rejection = Promise.reject("second-page-only");
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page rejection should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let (second_rejections, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify(globalThis.__lm_shared_isolate_unhandled_rejections)"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page rejection list should evaluate");
    assert_eq!(
        renderer_json_value(second_rejections),
        Some(serde_json::json!("[\"second-page-only\"]")),
        "second page should receive its own unhandled rejection"
    );

    let (first_rejections, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify(globalThis.__lm_shared_isolate_unhandled_rejections)"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("first page rejection list should evaluate");
    assert_eq!(
        renderer_json_value(first_rejections),
        Some(serde_json::json!("[]")),
        "first page must not receive the second page's unhandled rejection"
    );

    let (handler_added, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_shared_isolate_late_rejection.catch(() => "handled");
  return "handler-added";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page late rejection handler should run");
    assert_eq!(
        renderer_json_value(handler_added),
        Some(serde_json::json!("handler-added"))
    );

    let (second_rejectionhandled, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify(globalThis.__lm_shared_isolate_rejectionhandled)"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page rejectionhandled list should evaluate");
    assert_eq!(
        renderer_json_value(second_rejectionhandled),
        Some(serde_json::json!("[\"same-promise\"]")),
        "second page should receive rejectionhandled for its own promise"
    );

    let (first_rejectionhandled, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify(globalThis.__lm_shared_isolate_rejectionhandled)"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("first page rejectionhandled list should evaluate");
    assert_eq!(
        renderer_json_value(first_rejectionhandled),
        Some(serde_json::json!("[]")),
        "first page must not receive rejectionhandled for the second page's promise"
    );

    first_page
        .close_async()
        .await
        .expect("first shared page should close");
    second_page
        .close_async()
        .await
        .expect("second shared page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_window_open_routes_page_owned() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-window-open-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-window-open-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first window opener</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate window-open page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second window opener</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared-isolate window-open page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("shared window-open unique document isolate count"),
        2
    );
    output_rx.drain();

    let (popup_result, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                r#"window.open("https://example.test/shared-popup-from-second", "_blank") !== null"#
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page window.open(_blank) should run");
    assert_eq!(
        renderer_json_value(popup_result),
        Some(serde_json::json!(true))
    );

    let popup_publications = output_rx.drain();
    assert!(
        popup_activations_for_page(&popup_publications, &first_page).is_empty(),
        "page A must not receive page B's popup activation"
    );
    let second_popups = popup_activations_for_page(&popup_publications, &second_page);
    assert_eq!(second_popups.len(), 1);
    assert_eq!(second_popups[0].target_name(), "_blank");
    assert_eq!(
        second_popups[0].url(),
        "https://example.test/shared-popup-from-second"
    );
    assert!(matches!(
        second_popups[0].source(),
        crate::RendererPopupActivationSource::Window {
            window: crate::RendererWindowDocumentSource::RootFrame,
            exposes_opener: true,
            ..
        }
    ));

    let (self_result, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                r#"window.open("data:text/html,<main>first self target</main>", "_self") !== null"#
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("first page window.open(_self) should run");
    assert_eq!(
        renderer_json_value(self_result),
        Some(serde_json::json!(true))
    );

    let self_target_publications = output_rx.drain();
    assert!(
        popup_activations_for_page(&self_target_publications, &first_page).is_empty(),
        "_self must not create a popup activation on its owner page"
    );
    assert!(
        popup_activations_for_page(&self_target_publications, &second_page).is_empty(),
        "_self on page A must not create a popup activation on page B"
    );
    assert_eq!(
        has_pending_location_navigation_for_test(&first_page).await,
        Some(true),
        "_self should queue cross-document navigation only on page A"
    );
    assert_eq!(
        has_pending_location_navigation_for_test(&second_page).await,
        Some(false),
        "page B must not inherit page A's _self navigation"
    );

    first_page
        .close_async()
        .await
        .expect("first window-open page should close");
    second_page
        .close_async()
        .await
        .expect("second window-open page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_dedicated_worker_events_page_owned() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-worker-events-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-worker-events-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first worker owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate worker-event page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second worker owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared-isolate worker-event page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("shared dedicated-worker unique document isolate count"),
        2
    );

    let (installed, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
(() => {
  globalThis.__lm_dedicated_worker_events = [];
  const messageSource = `
    postMessage([
      self instanceof DedicatedWorkerGlobalScope,
      typeof Window === "function" && self instanceof Window,
      typeof document
    ].join("|"));
  `;
  const messageWorker = new Worker(
    "data:text/javascript," + encodeURIComponent(messageSource)
  );
  messageWorker.onmessage = event => {
    globalThis.__lm_dedicated_worker_events.push("message:" + event.data);
  };

  const errorWorker = new Worker(
    "data:text/javascript," + encodeURIComponent("throw new Error('worker-boom')")
  );
  errorWorker.onerror = event => {
    globalThis.__lm_dedicated_worker_events.push(
      "error:" + event.type + ":" + event.message.includes("worker-boom")
    );
    event.preventDefault();
  };
  return "installed";
})()
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page dedicated-worker probes should install");
    assert_eq!(
        renderer_json_value(installed),
        Some(serde_json::json!("installed"))
    );

    second_page
        .run_async_command(RendererPageCommand::WaitForScriptTruthy {
            expression: r#"globalThis.__lm_dedicated_worker_events?.length >= 2"#.to_owned(),
            timeout_ms: 2_000,
            loader: loader.clone(),
        })
        .await
        .expect("dedicated worker message and error events should arrive on page B");

    let (second_events, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify(globalThis.__lm_dedicated_worker_events.slice().sort())"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page worker events should evaluate");
    assert_eq!(
        renderer_json_value(second_events),
        Some(serde_json::json!(
            "[\"error:error:true\",\"message:true|false|undefined\"]"
        )),
        "page B should receive worker message/error events from a worker global, not a page realm"
    );

    let (first_events, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_dedicated_worker_events ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first page worker event marker should evaluate");
    assert_eq!(
        renderer_json_value(first_events),
        Some(serde_json::json!("missing")),
        "page A must not receive page B's dedicated-worker events"
    );

    first_page
        .close_async()
        .await
        .expect("first worker-event page should close");
    second_page
        .close_async()
        .await
        .expect("second worker-event page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn dedicated_worker_script_load_failure_does_not_dispatch_window_error() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker script 404 server");
    let addr = listener
        .local_addr()
        .expect("worker script 404 server addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker script request");
        let mut buf = [0_u8; 1024];
        let n = stream
            .read(&mut buf)
            .await
            .expect("read worker script request");
        let request = String::from_utf8_lossy(&buf[..n]);
        assert!(
            request.starts_with("GET /does-not-exist.js "),
            "unexpected worker script request: {request}"
        );
        stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .expect("write worker script 404 response");
    });
    let url = url::Url::parse(&format!("http://{addr}/page.html")).unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url,
        "<!doctype html><body>worker load failure</body>",
    )
    .await;

    let (installed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
(() => {
  globalThis.__lm_worker_script_error_events = [];
  window.onerror = message => {
    globalThis.__lm_worker_script_error_events.push("window:" + String(message));
    return true;
  };
  const worker = new Worker("does-not-exist.js");
  globalThis.__lm_missing_script_worker = worker;
  worker.onerror = event => {
    globalThis.__lm_worker_script_error_events.push([
      "worker",
      event.type,
      String(event.message).includes("HTTP request"),
      String(event.filename).endsWith("/does-not-exist.js")
    ].join(":"));
  };
  return "installed";
})()
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("worker script load failure probe should install");
    assert_eq!(
        renderer_json_value(installed),
        Some(serde_json::json!("installed"))
    );

    page.run_async_command(RendererPageCommand::WaitForScriptTruthy {
        expression: r#"globalThis.__lm_worker_script_error_events?.some(event => event.startsWith("worker:"))"#.to_owned(),
        timeout_ms: 2_000,
        loader: loader.clone(),
    })
    .await
    .expect("worker script load failure should dispatch worker error");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify(globalThis.__lm_worker_script_error_events)"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("worker script load failure events should evaluate");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("[\"worker:error:true:true\"]")),
        "worker script load failure must not bubble to window.onerror"
    );

    server
        .await
        .expect("worker script 404 server should finish");
    page.close_async()
        .await
        .expect("worker script load failure page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_indexed_db_managers_page_owned() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://shared-idb.example/app-a").unwrap();
    let second_url = url::Url::parse("https://shared-idb.example/app-b").unwrap();
    let storage_key = moli_storage_key::MoliStorageKey::first_party_from_url(&first_url, None)
        .serialized_storage_key();
    let first_root = runtime_indexed_db_test_root("shared-isolate-a");
    let second_root = runtime_indexed_db_test_root("shared-isolate-b");
    let first_manager = crate::new_indexed_db_manager(Some(first_root.clone()))
        .expect("first indexedDB manager should initialize");
    let second_manager = crate::new_indexed_db_manager(Some(second_root.clone()))
        .expect("second indexedDB manager should initialize");

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first idb</body>".to_owned(),
            Some(crate::downgrade_indexed_db_manager(&first_manager)),
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate indexedDB page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second idb</body>".to_owned(),
            Some(crate::downgrade_indexed_db_manager(&second_manager)),
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared-isolate indexedDB page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("shared indexedDB unique document isolate count"),
        2
    );

    store_indexed_db_value_for_test(&first_page, &loader, "first").await;
    store_indexed_db_value_for_test(&second_page, &loader, "second").await;

    assert!(
        runtime_indexed_db_origin_file(&first_root, &storage_key).exists(),
        "first page must write through its own browser-context indexedDB manager"
    );
    assert!(
        runtime_indexed_db_origin_file(&second_root, &storage_key).exists(),
        "second page must write through its own browser-context indexedDB manager"
    );

    first_page
        .close_async()
        .await
        .expect("first indexedDB page should close");
    second_page
        .close_async()
        .await
        .expect("second indexedDB page should close");
    let _ = std::fs::remove_dir_all(first_root);
    let _ = std::fs::remove_dir_all(second_root);
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_blob_urls_page_owned_after_other_page_close() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-blob-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-blob-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first blob owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second blob owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared-isolate page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );

    let (first_blob_url, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"URL.createObjectURL(new Blob(["first-owned"], { type: "text/plain" }))"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("first page blob URL should be created");
    assert!(
        renderer_json_value(first_blob_url)
            .and_then(|value| value.as_str().map(str::to_owned))
            .is_some_and(|url| url.starts_with("blob:https://example.test/")),
        "first page should create a blob URL"
    );

    let (second_blob_url, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                r#"URL.createObjectURL(new Blob(["second-owned"], { type: "text/plain" }))"#
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page blob URL should be created");
    let second_blob_url = renderer_json_value(second_blob_url)
        .and_then(|value| value.as_str().map(str::to_owned))
        .expect("second page should return a blob URL");
    assert!(second_blob_url.starts_with("blob:https://example.test/"));

    first_page
        .close_async()
        .await
        .expect("first shared page should close");

    let (second_blob_text, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!("fetch({second_blob_url:?}).then(response => response.text())"),
            await_promise: true,
        })
        .await
        .expect("second page blob URL should remain fetchable after first page closes");
    assert_eq!(
        renderer_json_value(second_blob_text),
        Some(serde_json::json!("second-owned")),
        "closing page A must not clean page B's blob URL in a shared document isolate"
    );

    second_page
        .close_async()
        .await
        .expect("second shared page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_domparser_detached_docs_cleanup_neutral() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-domparser-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-domparser-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first detached owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate DOMParser page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second detached peer</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared-isolate DOMParser page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached DOMParser unique document isolate count"),
        2
    );

    let (detached_status, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_shared_isolate_detached_doc =
    new DOMParser().parseFromString(
      "<!doctype html><body><p id='marker'>detached-owner</p></body>",
      "text/html"
    );
  return globalThis.__lm_shared_isolate_detached_doc
    .getElementById("marker")
    .textContent;
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("first page detached DOMParser document should evaluate");
    assert_eq!(
        renderer_json_value(detached_status),
        Some(serde_json::json!("detached-owner"))
    );

    let (second_blob_url, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"URL.createObjectURL(new Blob(["second-peer"], { type: "text/plain" }))"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page blob URL should be created");
    let second_blob_url = renderer_json_value(second_blob_url)
        .and_then(|value| value.as_str().map(str::to_owned))
        .expect("second page should return a blob URL");
    assert!(second_blob_url.starts_with("blob:https://example.test/"));

    first_page
        .close_async()
        .await
        .expect("first shared DOMParser page should close");

    let (second_blob_text, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!("fetch({second_blob_url:?}).then(response => response.text())"),
            await_promise: true,
        })
        .await
        .expect("second page blob URL should remain fetchable after first page closes");
    assert_eq!(
        renderer_json_value(second_blob_text),
        Some(serde_json::json!("second-peer")),
        "closing a page that holds DOMParser detached documents must not clean peer page resources"
    );

    second_page
        .close_async()
        .await
        .expect("second shared DOMParser page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_isolated_worlds_page_owned() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-world-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-world-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first isolated world</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second isolated world</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared-isolate page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );

    let first_world_context_id = create_isolated_world_for_test(&first_page, "shared-utility")
        .await
        .expect("first page isolated world should be created");
    let second_world_context_id = create_isolated_world_for_test(&second_page, "shared-utility")
        .await
        .expect("second page isolated world should be created");
    assert_eq!(
        first_world_context_id, second_world_context_id,
        "fresh per-page isolates should expose independent, target-scoped context-id namespaces"
    );
    let first_world_heap = runtime_heap_usage_for_test(&first_page).await;
    let second_world_heap = runtime_heap_usage_for_test(&second_page).await;
    assert_eq!(
        first_world_heap["moli"]["runtime"]["inspectorContextRegistrationCount"],
        serde_json::json!(2),
        "first document should own default and isolated Inspector registrations"
    );
    assert_eq!(
        second_world_heap["moli"]["runtime"]["inspectorContextRegistrationCount"],
        serde_json::json!(2),
        "second document should own an independent default/isolated registration pair"
    );

    let (first_world_marker, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression: r#"document.__lm_shared_isolate_world_document_marker = "isolated-document"; globalThis.__lm_shared_isolate_world_marker = document.body.textContent.trim(); globalThis.__lm_shared_isolate_world_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first isolated world marker should evaluate");
    assert_eq!(
        renderer_json_value(first_world_marker),
        Some(serde_json::json!("first isolated world"))
    );

    let (second_world_missing, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_world_context_id,
            expression: r#"globalThis.__lm_shared_isolate_world_marker ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second isolated world marker read should evaluate");
    assert_eq!(
        renderer_json_value(second_world_missing),
        Some(serde_json::json!("missing")),
        "same-name isolated worlds on different pages must not share global state"
    );

    let (second_world_marker_through_same_numeric_id, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression:
                r#"globalThis.__lm_shared_isolate_world_marker = "cross-page"; "cross-page""#
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("the reused numeric id should resolve within page B's target");
    assert_eq!(
        renderer_json_value(second_world_marker_through_same_numeric_id),
        Some(serde_json::json!("cross-page")),
        "a target-scoped numeric id must resolve to page B's own isolated world"
    );

    let (first_world_marker_after_second_page_evaluate, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression: r#"globalThis.__lm_shared_isolate_world_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first isolated world marker should remain readable");
    assert_eq!(
        renderer_json_value(first_world_marker_after_second_page_evaluate),
        Some(serde_json::json!("first isolated world")),
        "the same numeric id on page B must never route into page A's isolate"
    );

    let (second_default_missing_after_cross_page, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_shared_isolate_world_marker ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second default world marker read should evaluate after target-local world access");
    assert_eq!(
        renderer_json_value(second_default_missing_after_cross_page),
        Some(serde_json::json!("missing")),
        "target-local isolated-world access must not fall back to page B's default world"
    );

    let (first_default_missing, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_shared_isolate_world_marker ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first default world marker read should evaluate");
    assert_eq!(
        renderer_json_value(first_default_missing),
        Some(serde_json::json!("missing")),
        "isolated world globals must not leak into the page default world"
    );

    let (first_default_document_missing, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"document.__lm_shared_isolate_world_document_marker ?? "missing""#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("first default-world document wrapper should evaluate");
    assert_eq!(
        renderer_json_value(first_default_document_missing),
        Some(serde_json::json!("missing")),
        "main isolated and default worlds must keep distinct wrappers for the same Document"
    );

    first_page
        .close_async()
        .await
        .expect("first shared page should close");

    let (second_world_marker, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_world_context_id,
            expression: r#"globalThis.__lm_shared_isolate_world_marker = "second-world"; globalThis.__lm_shared_isolate_world_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second isolated world should remain usable after first page closes");
    assert_eq!(
        renderer_json_value(second_world_marker),
        Some(serde_json::json!("second-world")),
        "closing page A must not tear down page B's isolated world in a shared document isolate"
    );
    let second_world_heap_after_first_close = runtime_heap_usage_for_test(&second_page).await;
    assert_eq!(
        second_world_heap_after_first_close["moli"]["runtime"]["inspectorContextRegistrationCount"],
        serde_json::json!(2),
        "closing page A must not release page B's document-owned registrations"
    );

    second_page
        .close_async()
        .await
        .expect("second shared page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_child_default_contexts_page_owned() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-child-frame-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-child-frame-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            r#"<!doctype html><body><iframe srcdoc="<body>first child</body>"></iframe></body>"#
                .to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared child-frame page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            r#"<!doctype html><body><iframe srcdoc="<body>second child</body>"></iframe></body>"#
                .to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared child-frame page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared child-frame unique document isolate count"),
        2
    );

    let first_child_context_ids = child_default_context_ids_for_test(&first_page)
        .await
        .expect("first child default context events should replay");
    let second_child_context_ids = child_default_context_ids_for_test(&second_page)
        .await
        .expect("second child default context events should replay");
    assert_eq!(
        first_child_context_ids.len(),
        1,
        "first page should expose exactly one child default context"
    );
    assert_eq!(
        second_child_context_ids.len(),
        1,
        "second page should expose exactly one child default context"
    );
    let first_child_context_id = first_child_context_ids[0];
    let second_child_context_id = second_child_context_ids[0];
    assert_eq!(
        first_child_context_id, second_child_context_id,
        "fresh per-page isolates should expose independent, target-scoped child context ids"
    );
    let first_child_heap = runtime_heap_usage_for_test(&first_page).await;
    let second_child_heap = runtime_heap_usage_for_test(&second_page).await;
    assert_eq!(
        first_child_heap["moli"]["runtime"]["inspectorContextRegistrationCount"],
        serde_json::json!(2),
        "first document should own default and child-default Inspector registrations"
    );
    assert_eq!(
        second_child_heap["moli"]["runtime"]["inspectorContextRegistrationCount"],
        serde_json::json!(2),
        "second document should own an independent child-default registration"
    );

    let (first_child_marker, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_child_context_id,
            expression: r#"globalThis.__lm_shared_child_marker = "first-child"; globalThis.__lm_shared_child_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first child default marker should evaluate");
    assert_eq!(
        renderer_json_value(first_child_marker),
        Some(serde_json::json!("first-child"))
    );

    let (second_child_missing, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_child_context_id,
            expression: r#"globalThis.__lm_shared_child_marker ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second child default marker read should evaluate");
    assert_eq!(
        renderer_json_value(second_child_missing),
        Some(serde_json::json!("missing")),
        "child default contexts on different pages must not share global state"
    );

    let (second_child_marker_through_same_numeric_id, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_child_context_id,
            expression: r#"globalThis.__lm_shared_child_marker = "cross-page"; "cross-page""#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("the reused child context id should resolve within page B's target");
    assert_eq!(
        renderer_json_value(second_child_marker_through_same_numeric_id),
        Some(serde_json::json!("cross-page")),
        "a target-scoped child context id must resolve to page B's own child realm"
    );

    let (first_child_marker_after_second_page_evaluate, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_child_context_id,
            expression: r#"globalThis.__lm_shared_child_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first child marker should remain readable");
    assert_eq!(
        renderer_json_value(first_child_marker_after_second_page_evaluate),
        Some(serde_json::json!("first-child")),
        "the same numeric child id on page B must never route into page A's isolate"
    );

    let (second_default_missing, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_shared_child_marker ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second default marker read should evaluate");
    assert_eq!(
        renderer_json_value(second_default_missing),
        Some(serde_json::json!("missing")),
        "target-local child context evaluation must not fall back to page B's default world"
    );

    first_page
        .close_async()
        .await
        .expect("first shared child-frame page should close");

    let (second_child_marker, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_child_context_id,
            expression: r#"globalThis.__lm_shared_child_marker = "second-child"; globalThis.__lm_shared_child_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second child default context should remain usable after first page closes");
    assert_eq!(
        renderer_json_value(second_child_marker),
        Some(serde_json::json!("second-child")),
        "closing page A must not tear down page B's child default context"
    );
    let second_child_heap_after_first_close = runtime_heap_usage_for_test(&second_page).await;
    assert_eq!(
        second_child_heap_after_first_close["moli"]["runtime"]["inspectorContextRegistrationCount"],
        serde_json::json!(2),
        "closing page A must not release page B's child-default registration"
    );

    second_page
        .close_async()
        .await
        .expect("second shared child-frame page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_child_isolated_worlds_page_owned() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-child-world-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-child-world-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            r#"<!doctype html><body><iframe srcdoc="<body>first child isolated</body>"></iframe></body>"#
                .to_owned(),
            None,

            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared child-isolated-world page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            r#"<!doctype html><body><iframe srcdoc="<body>second child isolated</body>"></iframe></body>"#
                .to_owned(),
            None,

            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared child-isolated-world page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared child-isolated-world unique document isolate count"),
        2
    );

    let first_child_context_ids = child_default_context_ids_for_test(&first_page)
        .await
        .expect("first child default context events should replay");
    let second_child_context_ids = child_default_context_ids_for_test(&second_page)
        .await
        .expect("second child default context events should replay");
    assert_eq!(first_child_context_ids.len(), 1);
    assert_eq!(second_child_context_ids.len(), 1);
    let first_child_frame_id =
        child_frame_id_for_default_context_id_for_test(&first_page, first_child_context_ids[0])
            .await
            .expect("first child default context should map to a frame id");
    let second_child_frame_id =
        child_frame_id_for_default_context_id_for_test(&second_page, second_child_context_ids[0])
            .await
            .expect("second child default context should map to a frame id");

    let first_child_world_context_id = create_isolated_world_for_frame_for_test(
        &first_page,
        &first_child_frame_id,
        "shared-child-utility",
    )
    .await
    .expect("first child isolated world should be created");
    let second_child_world_context_id = create_isolated_world_for_frame_for_test(
        &second_page,
        &second_child_frame_id,
        "shared-child-utility",
    )
    .await
    .expect("second child isolated world should be created");
    assert_eq!(
        first_child_world_context_id, second_child_world_context_id,
        "fresh per-page isolates should expose independent child-world context-id namespaces"
    );

    let (first_child_world_marker, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_child_world_context_id,
            expression: r#"document.__lm_shared_child_world_document_marker = "isolated-document"; globalThis.__lm_shared_child_world_marker = document.body.textContent.trim(); globalThis.__lm_shared_child_world_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first child isolated world marker should evaluate");
    assert_eq!(
        renderer_json_value(first_child_world_marker),
        Some(serde_json::json!("first child isolated"))
    );

    let (second_child_world_missing, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_child_world_context_id,
            expression: r#"globalThis.__lm_shared_child_world_marker ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second child isolated world marker read should evaluate");
    assert_eq!(
        renderer_json_value(second_child_world_missing),
        Some(serde_json::json!("missing")),
        "same-name child isolated worlds on different pages must not share global state"
    );

    let (second_child_world_marker_through_same_numeric_id, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_child_world_context_id,
            expression: r#"globalThis.__lm_shared_child_world_marker = "cross-page"; "cross-page""#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("the reused child-world id should resolve within page B's target");
    assert_eq!(
        renderer_json_value(second_child_world_marker_through_same_numeric_id),
        Some(serde_json::json!("cross-page")),
        "a target-scoped child-world id must resolve to page B's own isolated world"
    );

    let (first_child_world_marker_after_second_page_evaluate, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_child_world_context_id,
            expression: r#"globalThis.__lm_shared_child_world_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first child isolated-world marker should remain readable");
    assert_eq!(
        renderer_json_value(first_child_world_marker_after_second_page_evaluate),
        Some(serde_json::json!("first child isolated")),
        "the same numeric child-world id on page B must never route into page A's isolate"
    );

    let (second_default_missing, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_shared_child_world_marker ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second default marker read should evaluate");
    assert_eq!(
        renderer_json_value(second_default_missing),
        Some(serde_json::json!("missing")),
        "target-local child isolated-world access must not fall back to page B's default world"
    );

    let (second_child_default_missing, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_child_context_ids[0],
            expression: r#"globalThis.__lm_shared_child_world_marker ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second child default marker read should evaluate");
    assert_eq!(
        renderer_json_value(second_child_default_missing),
        Some(serde_json::json!("missing")),
        "child isolated-world globals must not leak into page B's child default world"
    );

    let (first_child_default_document_missing, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_child_context_ids[0],
            expression: r#"document.__lm_shared_child_world_document_marker ?? "missing""#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("first child default-world document wrapper should evaluate");
    assert_eq!(
        renderer_json_value(first_child_default_document_missing),
        Some(serde_json::json!("missing")),
        "child isolated and default worlds must keep distinct wrappers for the same Document"
    );

    first_page
        .close_async()
        .await
        .expect("first shared child-isolated-world page should close");

    let (second_child_world_marker, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_child_world_context_id,
            expression: r#"globalThis.__lm_shared_child_world_marker = document.body.textContent.trim(); globalThis.__lm_shared_child_world_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second child isolated world should remain usable after first page closes");
    assert_eq!(
        renderer_json_value(second_child_world_marker),
        Some(serde_json::json!("second child isolated")),
        "closing page A must not tear down page B's child isolated world"
    );

    second_page
        .close_async()
        .await
        .expect("second shared child-isolated-world page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_reuses_navigation_isolate_and_replaces_contexts() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let navigated_url = url::Url::parse("https://example.test/shared-navigation-a").unwrap();
    let peer_url = url::Url::parse("https://example.test/shared-navigation-b").unwrap();

    let (mut navigated_page, _, _, _creation_artifacts, navigated_download) = runtime
        .create_html_page_from_response(
            navigated_url.clone(),
            navigated_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>old shared navigation document</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate navigation page should load");
    assert!(navigated_download.is_none());

    let (mut peer_page, _, _, _creation_artifacts, peer_download) = runtime
        .create_html_page_from_response(
            peer_url.clone(),
            peer_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>peer shared navigation document</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("peer shared-isolate page should load");
    assert!(peer_download.is_none());

    let navigated_testing = RendererPageTestingHandle::new_for_testing(&navigated_page);
    let peer_testing = RendererPageTestingHandle::new_for_testing(&peer_page);
    assert!(navigated_testing.shares_local_host(&peer_testing));
    assert_eq!(
        peer_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );
    assert_window_performance_surface_for_test(&navigated_page, "old document").await;
    let old_heap = runtime_heap_usage_for_test(&navigated_page).await;
    let old_runtime = &old_heap["moli"]["runtime"];
    let old_context_group_id = old_runtime["inspectorContextGroupId"]
        .as_i64()
        .expect("old document should expose inspector context group id");
    let old_window_proxy_identity_hash = old_runtime["mainWindowProxyIdentityHash"]
        .as_i64()
        .expect("old document should expose main WindowProxy identity");
    assert_eq!(
        old_runtime["inspectorSessionRegistryOwner"],
        serde_json::json!("renderer-devtools-agent"),
        "old document inspector session registry should be local-root agent owned: {old_heap:?}"
    );

    let old_world_context_id = create_isolated_world_runtime_activity_for_test(
        &navigated_page,
        None,
        "navigation-utility",
    )
    .await
    .expect("old document isolated world should be created through runtime activity");
    let old_world_heap = runtime_heap_usage_for_test(&navigated_page).await;
    assert_eq!(
        old_world_heap["moli"]["runtime"]["inspectorContextRegistrationCount"],
        serde_json::json!(2),
        "old document should own its default and isolated context registrations"
    );
    let (old_world_marker, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: old_world_context_id,
            expression: r#"globalThis.__lm_shared_isolate_navigation_marker = "old-world"; globalThis.__lm_shared_isolate_navigation_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("old isolated world marker should evaluate");
    assert_eq!(
        renderer_json_value(old_world_marker),
        Some(serde_json::json!("old-world"))
    );
    let old_document_object_id = runtime_protocol_object_id(
        &navigated_page,
        serde_json::json!({
            "id": 51,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "({ marker: 'old-document-object' })"
            }
        }),
        51,
    )
    .await
    .expect("old document Runtime.evaluate should return an objectId");
    let old_runtime_enable_events = runtime_enable_events_for_test(&navigated_page)
        .await
        .expect("old document Runtime.enable should run through V8 inspector");
    assert!(
        old_runtime_enable_events.iter().any(|message| {
            message["method"] == serde_json::json!("Runtime.executionContextCreated")
        }),
        "old document Runtime.enable should connect the renderer Runtime agent: {old_runtime_enable_events:?}"
    );
    output_rx.drain();

    let replacement_url = "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Cbody%3Enew%20shared%20navigation%20document%3C/body%3E";
    let (navigation_reply, _) = navigated_page
        .run_async_command(
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression: format!(
                    r#"(() => {{
  globalThis.__lm_shared_isolate_navigation_marker = "old-default";
  location.href = {replacement_url:?};
  return "navigating";
}})()"#
                ),
                await_promise: false,
            },
        )
        .await
        .expect("shared isolate navigation should replace the live page");
    assert_eq!(
        renderer_json_value(navigation_reply),
        Some(serde_json::json!("navigating"))
    );
    assert_eq!(
        peer_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("shared unique document isolate count after navigation replacement"),
        2,
        "navigation replacement must retain one distinct isolate for each live page"
    );

    let (new_document_text, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"document.body.textContent"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("new document body text should evaluate");
    assert_eq!(
        renderer_json_value(new_document_text),
        Some(serde_json::json!("new shared navigation document"))
    );
    assert_window_performance_surface_for_test(&navigated_page, "replacement document").await;
    let new_heap = runtime_heap_usage_for_test(&navigated_page).await;
    let new_runtime = &new_heap["moli"]["runtime"];
    assert_eq!(
        new_runtime["inspectorSessionRegistryOwner"],
        serde_json::json!("renderer-devtools-agent"),
        "replacement document inspector session registry should be local-root agent owned: {new_heap:?}"
    );
    assert_ne!(
        new_runtime["inspectorContextGroupId"],
        serde_json::json!(old_context_group_id),
        "cross-Page replacement must create a distinct local-root context group"
    );
    assert_eq!(
        new_runtime["mainWindowProxyIdentityHash"],
        serde_json::json!(old_window_proxy_identity_hash),
        "committed top-level navigation must detach and reuse the same V8 global proxy identity"
    );
    assert!(
        new_runtime["inspectorSessionCount"]
            .as_u64()
            .unwrap_or_default()
            >= 1,
        "replacement document should keep at least the default inspector session: {new_heap:?}"
    );
    assert_eq!(
        new_runtime["inspectorContextRegistrationCount"],
        serde_json::json!(1),
        "replacement must release every old-document registration and retain only its new default context"
    );
    let replacement_runtime_messages =
        output_rx.drain_runtime_inspector_messages_for_page(&navigated_page);
    let replacement_context_events = replacement_runtime_messages
        .iter()
        .filter_map(|message| message["method"].as_str())
        .filter(|method| {
            matches!(
                *method,
                "Runtime.executionContextsCleared" | "Runtime.executionContextCreated"
            )
        })
        .collect::<Vec<_>>();
    assert!(
        replacement_context_events.len() >= 2
            && replacement_context_events.last() == Some(&"Runtime.executionContextCreated")
            && replacement_context_events[..replacement_context_events.len() - 1]
                .iter()
                .all(|method| *method == "Runtime.executionContextsCleared"),
        "renderer-side document replacement should publish every old/reattached V8 context reset before exactly one new default context: {replacement_runtime_messages:?}"
    );
    assert!(
        default_execution_context_id_for_test(&navigated_page)
            .await
            .expect("replacement default execution context lookup")
            .is_some(),
        "the old backend's context-clear event must not clear the replacement backend's local default-context identity"
    );

    let old_world_still_registered = navigated_page
        .run_async_command(RendererPageCommand::HasIsolatedExecutionContextId(
            old_world_context_id,
        ))
        .await
        .expect("old isolated context membership should evaluate");
    assert_eq!(
        renderer_bool(old_world_still_registered.0),
        Some(false),
        "navigation replacement must remove the old document's isolated world from the page facade"
    );

    let old_marker_after_navigation = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: old_world_context_id,
            expression: r#"globalThis.__lm_shared_isolate_navigation_marker = "stale"; "stale""#
                .to_owned(),
            await_promise: false,
        })
        .await;
    assert!(
        old_marker_after_navigation.is_err(),
        "stale isolated-world execution context id must fail closed after navigation replacement"
    );

    let (replacement_marker_after_stale_context, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_shared_isolate_navigation_marker ?? "missing""#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement default world marker read should evaluate");
    assert_eq!(
        renderer_json_value(replacement_marker_after_stale_context),
        Some(serde_json::json!("missing")),
        "stale isolated-world context id failure must not fall back to the replacement document"
    );

    let stale_object_call = dispatch_runtime_protocol_for_test(
        &navigated_page,
        serde_json::json!({
            "id": 52,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": old_document_object_id,
                "functionDeclaration": "function() { globalThis.__staleObjectMutatedReplacement = true; return this.marker; }",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("stale old document Runtime.callFunctionOn should dispatch");
    let stale_object_response = runtime_protocol_response_by_id(&stale_object_call, 52)
        .expect("stale object call response");
    assert!(
        stale_object_response.get("error").is_some()
            || stale_object_response["result"]["exceptionDetails"].is_object(),
        "navigation replacement must reject or fail closed for the old document Runtime objectId: {stale_object_response:?}"
    );
    let (replacement_mutation_marker, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__staleObjectMutatedReplacement ?? "not-mutated""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement stale object mutation marker should evaluate");
    assert_eq!(
        renderer_json_value(replacement_mutation_marker),
        Some(serde_json::json!("not-mutated")),
        "stale old document Runtime objectId must not execute against the replacement document global"
    );

    navigated_page
        .close_async()
        .await
        .expect("navigated shared page should close");
    peer_page
        .close_async()
        .await
        .expect("peer shared page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_parser_blocking_navigation_restores_replacement_inspector_session() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, script_request_seen, release_script_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/replacement-parser-blocking.js",
            r#"globalThis.__lm_parser_blocking_replacement = "ran";"#,
            "application/javascript",
        )
        .await;
    let initial_url = url::Url::parse(&format!("{base_url}/parser-blocking-navigation-source"))
        .expect("initial parser-blocking navigation url");

    let (mut page, _, _, _creation_artifacts, pending_download) = runtime
        .create_html_page_from_response(
            initial_url.clone(),
            initial_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>parser-blocking navigation source</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("parser-blocking navigation source page should load");
    assert!(pending_download.is_none());

    runtime_enable_events_for_test(&page)
        .await
        .expect("old document Runtime.enable should establish persistent session state");
    output_rx.drain();

    let replacement_html = format!(
        r#"<!doctype html><script src="{base_url}/replacement-parser-blocking.js"></script><body>parser-blocking replacement</body>"#
    );
    let encoded_replacement_html = percent_encoding::utf8_percent_encode(
        &replacement_html,
        percent_encoding::NON_ALPHANUMERIC,
    );
    let replacement_url = format!("data:text/html;charset=utf-8,{encoded_replacement_html}");

    let release = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(2), script_request_seen)
            .await
            .expect("replacement parser-blocking script request should start")
            .expect("replacement parser-blocking script request signal should remain open");
        release_script_response
            .send(())
            .expect("replacement parser-blocking script response should release");
    });

    let (navigation_reply, _) = page
        .run_async_command(
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression: format!(r#"location.href = {replacement_url:?}; "navigating""#),
                await_promise: false,
            },
        )
        .await
        .expect("parser-blocking navigation should install the replacement PageVm");
    assert_eq!(
        renderer_json_value(navigation_reply),
        Some(serde_json::json!("navigating"))
    );
    release
        .await
        .expect("parser-blocking response release task should not panic");
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("parser-blocking server should finish")
        .expect("parser-blocking server task should not panic");

    let (replacement_state, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"[document.body.textContent, globalThis.__lm_parser_blocking_replacement].join("|")"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement parser-blocking document should remain executable");
    assert_eq!(
        renderer_json_value(replacement_state),
        Some(serde_json::json!("parser-blocking replacement|ran"))
    );

    let replacement_runtime_messages = output_rx.drain_runtime_inspector_messages_for_page(&page);
    assert!(
        replacement_runtime_messages.iter().any(|message| {
            message["method"] == serde_json::json!("Runtime.executionContextsCleared")
        }),
        "replacement session restore should deliver the old-context clear event: {replacement_runtime_messages:?}"
    );
    assert!(
        replacement_runtime_messages.iter().any(|message| {
            message["method"] == serde_json::json!("Runtime.executionContextCreated")
        }),
        "replacement session restore should deliver the new context event: {replacement_runtime_messages:?}"
    );

    page.close_async()
        .await
        .expect("parser-blocking replacement page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_navigation_churn_disposes_replaced_page_vms() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let initial_url = url::Url::parse("https://example.test/shared-navigation-churn").unwrap();

    let (mut page, _, _, _creation_artifacts, pending_download) = runtime
        .create_html_page_from_response(
            initial_url.clone(),
            initial_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>churn-initial</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("shared-isolate churn page should load");
    assert!(pending_download.is_none());

    let testing = RendererPageTestingHandle::new_for_testing(&page);
    let baseline_pending = testing
        .deferred_page_vm_drop_pending_count_async()
        .await
        .expect("initial deferred PageVm drop pending count");

    for index in 0..8 {
        let (finalizer_setup, _) = page
            .run_async_command(RendererPageCommand::EvaluateExpression {
                expression: format!(
                    r#"(async () => {{
  globalThis.__lm_context_owned_finalizer_objects = [];
  for (let objectIndex = 0; objectIndex < 32; objectIndex += 1) {{
    const element = document.createElement("div");
    element.style.color = "red";

    const sheet = new CSSStyleSheet();
    sheet.replaceSync(`.item-${{objectIndex}} {{ color: red; }}`);
    sheet.cssRules[0].style.setProperty("color", "blue");

    const blob = new Blob([`payload-${{objectIndex}}`], {{ type: "text/plain" }});
    globalThis.__lm_context_owned_finalizer_objects.push(element, sheet, blob);
  }}
  const responses = await Promise.all(
    Array.from({{ length: 8 }}, (_, responseIndex) =>
      fetch(`data:text/plain;charset=utf-8,response-{index}-${{responseIndex}}`)
    )
  );
  globalThis.__lm_context_owned_finalizer_objects.push(...responses);
  return globalThis.__lm_context_owned_finalizer_objects.length;
}})()"#
                ),
                await_promise: true,
            })
            .await
            .expect("context-owned finalizer setup should complete before navigation");
        assert_eq!(
            renderer_json_value(finalizer_setup),
            Some(serde_json::json!(104)),
            "navigation churn must retain CSS, Blob, and network body objects until old-context teardown"
        );

        let replacement_url = format!(
            "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Cbody%3Echurn-{index}%3C/body%3E"
        );
        let (navigation_reply, _) = page
            .run_async_command(
                RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                    expression: format!(
                        r#"location.href = {replacement_url:?}; "navigating-{index}""#
                    ),
                    await_promise: false,
                },
            )
            .await
            .expect("shared-isolate churn navigation should replace the live page");
        assert_eq!(
            renderer_json_value(navigation_reply),
            Some(serde_json::json!(format!("navigating-{index}")))
        );
        let (body_text, _) = page
            .run_async_command(RendererPageCommand::EvaluateExpression {
                expression: r#"document.body.textContent"#.to_owned(),
                await_promise: false,
            })
            .await
            .expect("replacement churn body text should evaluate");
        assert_eq!(
            renderer_json_value(body_text),
            Some(serde_json::json!(format!("churn-{index}")))
        );
        assert_eq!(
            testing
                .deferred_page_vm_drop_pending_count_async()
                .await
                .expect("deferred PageVm drop pending count after navigation churn"),
            baseline_pending,
            "replaced per-page PageVms must dispose without a deferred LIFO backlog"
        );
    }

    page.close_async()
        .await
        .expect("shared-isolate churn page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_drops_stale_timer_after_navigation_replacement() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let navigated_url = url::Url::parse("https://example.test/shared-stale-timer-a").unwrap();
    let peer_url = url::Url::parse("https://example.test/shared-stale-timer-b").unwrap();

    let (mut navigated_page, _, _, _creation_artifacts, navigated_download) = runtime
        .create_html_page_from_response(
            navigated_url.clone(),
            navigated_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>stale timer source</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("stale-timer source page should load");
    assert!(navigated_download.is_none());

    let (mut peer_page, _, _, _creation_artifacts, peer_download) = runtime
        .create_html_page_from_response(
            peer_url.clone(),
            peer_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>stale timer peer</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("stale-timer peer page should load");
    assert!(peer_download.is_none());

    let navigated_testing = RendererPageTestingHandle::new_for_testing(&navigated_page);
    let peer_testing = RendererPageTestingHandle::new_for_testing(&peer_page);
    assert!(navigated_testing.shares_local_host(&peer_testing));
    assert_eq!(
        peer_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("shared stale-timer unique document isolate count"),
        2
    );

    let replacement_url = "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Cbody%3Etimer%20replacement%20document%3C/body%3E";
    let (navigation_reply, _) = navigated_page
        .run_async_command(
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression: format!(
                    r#"(() => {{
  globalThis.__lm_stale_timer_marker = "old-document";
  setTimeout(() => {{
    globalThis.__lm_stale_timer_marker = "stale-timer-fired";
    globalThis.__lm_stale_timer_mutated_replacement = "stale-timer-fired";
  }}, 0);
  location.href = {replacement_url:?};
  return "navigating";
}})()"#
                ),
                await_promise: false,
            },
        )
        .await
        .expect("stale-timer page should navigate to replacement document");
    assert_eq!(
        renderer_json_value(navigation_reply),
        Some(serde_json::json!("navigating"))
    );
    assert_eq!(
        peer_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect(
                "shared unique document isolate count after stale-timer navigation replacement"
            ),
        2,
        "timer navigation replacement must retain one distinct isolate for each live page"
    );

    let (advance, _) = navigated_page
        .run_async_command(RendererPageCommand::MsToNextTimeout)
        .await
        .expect(
            "replacement timer deadline should remain observable after stale timer replacement",
        );
    match advance {
        RendererPageReply::OptionalU64(ms_to_next_timeout) => {
            assert_eq!(
                ms_to_next_timeout, None,
                "replacement document should not inherit the old document timer deadline"
            );
        }
        _ => panic!("unexpected timer-deadline reply after stale timer replacement"),
    }

    let (replacement_marker, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"[
  document.body.textContent,
  globalThis.__lm_stale_timer_marker ?? "missing",
  globalThis.__lm_stale_timer_mutated_replacement ?? "not-mutated"
].join("|")"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement stale-timer marker should evaluate");
    assert_eq!(
        renderer_json_value(replacement_marker),
        Some(serde_json::json!(
            "timer replacement document|missing|not-mutated"
        )),
        "stale old-document timer callback must not mutate the replacement document"
    );

    navigated_page
        .close_async()
        .await
        .expect("stale-timer navigated page should close");
    peer_page
        .close_async()
        .await
        .expect("stale-timer peer page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_drops_stale_fetch_after_navigation_replacement() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, fetch_request_seen, release_fetch_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/stale-fetch.txt",
            "late-fetch",
            "text/plain; charset=utf-8",
        )
        .await;
    let navigated_url =
        url::Url::parse(&format!("{base_url}/shared-stale-fetch-a")).expect("fetch source url");
    let peer_url =
        url::Url::parse(&format!("{base_url}/shared-stale-fetch-b")).expect("fetch peer url");

    let (mut navigated_page, _, _, _creation_artifacts, navigated_download) = runtime
        .create_html_page_from_response(
            navigated_url.clone(),
            navigated_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>stale fetch source</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("stale-fetch source page should load");
    assert!(navigated_download.is_none());

    let (mut peer_page, _, _, _creation_artifacts, peer_download) = runtime
        .create_html_page_from_response(
            peer_url.clone(),
            peer_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>stale fetch peer</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("stale-fetch peer page should load");
    assert!(peer_download.is_none());

    let navigated_testing = RendererPageTestingHandle::new_for_testing(&navigated_page);
    let peer_testing = RendererPageTestingHandle::new_for_testing(&peer_page);
    assert!(navigated_testing.shares_local_host(&peer_testing));
    assert_eq!(
        peer_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("shared stale-fetch unique document isolate count"),
        2
    );

    let (scheduled, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_stale_fetch_marker = "old-document";
  fetch("./stale-fetch.txt").then(
    response => response.text()
  ).then(
    text => {
      globalThis.__lm_stale_fetch_continuation = text;
      globalThis.__lm_stale_fetch_mutated_replacement = "stale-fetch-continuation";
    },
    error => {
      globalThis.__lm_stale_fetch_continuation = "error:" + error.name;
    }
  );
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("stale-fetch page should schedule fetch");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), fetch_request_seen)
        .await
        .expect("stale fetch request should reach the server before navigation")
        .expect("stale fetch request signal should send");

    let replacement_url = "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Cbody%3Efetch%20replacement%20document%3C/body%3E";
    let (navigation_reply, _) = navigated_page
        .run_async_command(
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression: format!(
                    r#"(() => {{
  location.href = {replacement_url:?};
  return "navigating";
}})()"#
                ),
                await_promise: false,
            },
        )
        .await
        .expect("stale-fetch page should navigate to replacement document");
    assert_eq!(
        renderer_json_value(navigation_reply),
        Some(serde_json::json!("navigating"))
    );
    assert_eq!(
        peer_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect(
                "shared unique document isolate count after stale-fetch navigation replacement"
            ),
        2,
        "fetch navigation replacement must retain one distinct isolate for each live page"
    );

    release_fetch_response
        .send(())
        .expect("stale fetch response release should send");
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("stale fetch server should finish after response release")
        .expect("stale fetch server task should not panic");

    let (_advance, _) = navigated_page
        .run_async_command(RendererPageCommand::MsToNextTimeout)
        .await
        .expect("replacement timer deadline should remain observable after stale fetch response");

    let (replacement_marker, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"[
  document.body.textContent,
  globalThis.__lm_stale_fetch_marker ?? "missing",
  globalThis.__lm_stale_fetch_continuation ?? "missing",
  globalThis.__lm_stale_fetch_mutated_replacement ?? "not-mutated"
].join("|")"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement stale-fetch marker should evaluate");
    assert_eq!(
        renderer_json_value(replacement_marker),
        Some(serde_json::json!(
            "fetch replacement document|missing|missing|not-mutated"
        )),
        "stale old-document fetch completion must not mutate the replacement document"
    );

    navigated_page
        .close_async()
        .await
        .expect("stale-fetch navigated page should close");
    peer_page
        .close_async()
        .await
        .expect("stale-fetch peer page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_drops_stale_module_fetch_after_navigation_replacement() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, module_request_seen, release_module_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/stale-module.js",
            r#"globalThis.__lm_stale_module_mutated_replacement = "stale-module-evaluated";
export const marker = "late-module";"#,
            "application/javascript",
        )
        .await;
    let navigated_url =
        url::Url::parse(&format!("{base_url}/shared-stale-module-a")).expect("module source url");
    let peer_url =
        url::Url::parse(&format!("{base_url}/shared-stale-module-b")).expect("module peer url");

    let (mut navigated_page, _, _, _creation_artifacts, navigated_download) = runtime
        .create_html_page_from_response(
            navigated_url.clone(),
            navigated_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>stale module source</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("stale-module source page should load");
    assert!(navigated_download.is_none());

    let (mut peer_page, _, _, _creation_artifacts, peer_download) = runtime
        .create_html_page_from_response(
            peer_url.clone(),
            peer_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>stale module peer</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("stale-module peer page should load");
    assert!(peer_download.is_none());

    let navigated_testing = RendererPageTestingHandle::new_for_testing(&navigated_page);
    let peer_testing = RendererPageTestingHandle::new_for_testing(&peer_page);
    assert!(navigated_testing.shares_local_host(&peer_testing));
    assert_eq!(
        peer_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("shared stale-module unique document isolate count"),
        2
    );

    let (scheduled, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_stale_module_marker = "old-document";
  import("./stale-module.js").then(
    module => {
      globalThis.__lm_stale_module_continuation = module.marker;
      globalThis.__lm_stale_module_mutated_replacement = "stale-module-continuation";
    },
    error => {
      globalThis.__lm_stale_module_continuation = "error:" + error.name;
    }
  );
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("stale-module page should schedule dynamic import");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), module_request_seen)
        .await
        .expect("stale module request should reach the server before navigation")
        .expect("stale module request signal should send");

    let replacement_url = "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Cbody%3Emodule%20replacement%20document%3C/body%3E";
    let (navigation_reply, _) = navigated_page
        .run_async_command(
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression: format!(
                    r#"(() => {{
  location.href = {replacement_url:?};
  return "navigating";
}})()"#
                ),
                await_promise: false,
            },
        )
        .await
        .expect("stale-module page should navigate to replacement document");
    assert_eq!(
        renderer_json_value(navigation_reply),
        Some(serde_json::json!("navigating"))
    );
    assert_eq!(
        peer_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect(
                "shared unique document isolate count after stale-module navigation replacement"
            ),
        2,
        "module navigation replacement must retain one distinct isolate for each live page"
    );

    release_module_response
        .send(())
        .expect("stale module response release should send");
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("stale module server should finish after response release")
        .expect("stale module server task should not panic");

    let (_advance, _) = navigated_page
        .run_async_command(RendererPageCommand::MsToNextTimeout)
        .await
        .expect("replacement timer deadline should remain observable after stale module response");

    let (replacement_marker, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"[
  document.body.textContent,
  globalThis.__lm_stale_module_marker ?? "missing",
  globalThis.__lm_stale_module_continuation ?? "missing",
  globalThis.__lm_stale_module_mutated_replacement ?? "not-mutated"
].join("|")"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement stale-module marker should evaluate");
    assert_eq!(
        renderer_json_value(replacement_marker),
        Some(serde_json::json!(
            "module replacement document|missing|missing|not-mutated"
        )),
        "stale old-document module fetch completion must not mutate the replacement document"
    );

    navigated_page
        .close_async()
        .await
        .expect("stale-module navigated page should close");
    peer_page
        .close_async()
        .await
        .expect("stale-module peer page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_drops_stale_worker_message_after_navigation_replacement() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, worker_fetch_seen, release_worker_fetch_response, server) =
        spawn_gated_worker_message_server().await;
    let navigated_url = url::Url::parse(&format!("{base_url}/shared-stale-worker-message-a"))
        .expect("worker message source url");
    let peer_url = url::Url::parse(&format!("{base_url}/shared-stale-worker-message-b"))
        .expect("worker message peer url");

    let (mut navigated_page, _, _, _creation_artifacts, navigated_download) = runtime
        .create_html_page_from_response(
            navigated_url.clone(),
            navigated_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>stale worker message source</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("stale-worker-message source page should load");
    assert!(navigated_download.is_none());

    let (mut peer_page, _, _, _creation_artifacts, peer_download) = runtime
        .create_html_page_from_response(
            peer_url.clone(),
            peer_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>stale worker message peer</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("stale-worker-message peer page should load");
    assert!(peer_download.is_none());

    let navigated_testing = RendererPageTestingHandle::new_for_testing(&navigated_page);
    let peer_testing = RendererPageTestingHandle::new_for_testing(&peer_page);
    assert!(navigated_testing.shares_local_host(&peer_testing));
    assert_eq!(
        peer_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("shared stale-worker-message unique document isolate count"),
        2
    );

    let (installed, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_stale_worker_message_events = [];
  globalThis.__lm_stale_worker_message_worker = new Worker("/stale-worker.js");
  globalThis.__lm_stale_worker_message_worker.onmessage = event => {
    const value = String(event.data);
    globalThis.__lm_stale_worker_message_events.push(value);
    if (value.startsWith("late:") || value.startsWith("error:")) {
      globalThis.__lm_stale_worker_message_mutated_replacement = value;
    }
  };
  return "installed";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("stale-worker-message probe should install");
    assert_eq!(
        renderer_json_value(installed),
        Some(serde_json::json!("installed"))
    );

    navigated_page
        .run_async_command(RendererPageCommand::WaitForScriptTruthy {
            expression: r#"globalThis.__lm_stale_worker_message_events?.includes("ready")"#
                .to_owned(),
            timeout_ms: 2_000,
            loader: loader.clone(),
        })
        .await
        .expect("stale-worker-message worker should report ready");

    let (scheduled_request, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_stale_worker_message_worker.postMessage("schedule-late");
  return "schedule-requested";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("stale-worker-message worker fetch should schedule");
    assert_eq!(
        renderer_json_value(scheduled_request),
        Some(serde_json::json!("schedule-requested"))
    );
    navigated_page
        .run_async_command(RendererPageCommand::WaitForScriptTruthy {
            expression: r#"globalThis.__lm_stale_worker_message_events?.includes("scheduled")"#
                .to_owned(),
            timeout_ms: 2_000,
            loader: loader.clone(),
        })
        .await
        .expect("stale-worker-message worker should confirm schedule");
    tokio::time::timeout(Duration::from_secs(2), worker_fetch_seen)
        .await
        .expect("stale worker fetch request should reach the server before navigation")
        .expect("stale worker fetch request signal should send");

    let replacement_url = "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Cbody%3Eworker%20message%20replacement%20document%3C/body%3E";
    let (navigation_reply, _) = navigated_page
        .run_async_command(
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression: format!(
                    r#"(() => {{
  location.href = {replacement_url:?};
  return "navigating";
}})()"#
                ),
                await_promise: false,
            },
        )
        .await
        .expect("stale-worker-message page should navigate to replacement document");
    assert_eq!(
        renderer_json_value(navigation_reply),
        Some(serde_json::json!("navigating"))
    );
    assert_eq!(
        peer_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect(
                "shared unique document isolate count after stale-worker-message navigation replacement"
            ),
        2,
        "worker-message navigation replacement must retain one distinct isolate for each live page"
    );

    release_worker_fetch_response
        .send(())
        .expect("stale worker fetch response release should send");
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("stale worker message server should finish after response release")
        .expect("stale worker message server task should not panic");

    let (_advance, _) = navigated_page
        .run_async_command(RendererPageCommand::MsToNextTimeout)
        .await
        .expect("replacement timer deadline should remain observable after stale worker response");

    let (replacement_marker, _) = navigated_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"[
  document.body.textContent,
  globalThis.__lm_stale_worker_message_events
    ? globalThis.__lm_stale_worker_message_events.join(",")
    : "missing",
  globalThis.__lm_stale_worker_message_mutated_replacement ?? "not-mutated"
].join("|")"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement stale-worker-message marker should evaluate");
    assert_eq!(
        renderer_json_value(replacement_marker),
        Some(serde_json::json!(
            "worker message replacement document|missing|not-mutated"
        )),
        "stale old-document worker completion/message must not mutate the replacement document"
    );

    let (peer_marker, _) = peer_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_stale_worker_message_events ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("peer stale-worker-message marker should evaluate");
    assert_eq!(
        renderer_json_value(peer_marker),
        Some(serde_json::json!("missing")),
        "stale worker message from page A must not route to peer page B"
    );

    navigated_page
        .close_async()
        .await
        .expect("stale-worker-message navigated page should close");
    peer_page
        .close_async()
        .await
        .expect("stale-worker-message peer page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_routes_dynamic_import_to_originating_page() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, server) = spawn_owner_wake_server_with_content_type(
        "/module.js",
        r#"export const marker = "second-dynamic"; export const metaUrl = import.meta.url;"#,
        "application/javascript",
        Duration::ZERO,
    )
    .await;
    let first_url =
        url::Url::parse(&format!("{base_url}/shared-dynamic-import-a")).expect("first page url");
    let second_url =
        url::Url::parse(&format!("{base_url}/shared-dynamic-import-b")).expect("second page url");

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first dynamic import owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second dynamic import owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared-isolate page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );

    first_page
        .close_async()
        .await
        .expect("first shared page should close");

    let (scheduled, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_shared_isolate_dynamic_import_marker = "pending";
  import("./module.js").then(
    module => {
      globalThis.__lm_shared_isolate_dynamic_import_marker =
        module.marker + "|" + String(module.metaUrl === new URL("./module.js", location.href).href);
    },
    error => {
      globalThis.__lm_shared_isolate_dynamic_import_marker = "error:" + error.name + ":" + error.message;
    }
  );
  return "scheduled";
})()"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page dynamic import should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(
        Duration::from_secs(2),
        second_page.run_async_command(RendererPageCommand::WaitForScriptTruthy {
            expression: r#"globalThis.__lm_shared_isolate_dynamic_import_marker !== "pending""#
                .to_owned(),
            timeout_ms: 2_000,
            loader: loader.clone(),
        }),
    )
    .await
    .expect("shared isolate dynamic import wait should complete")
    .expect("shared isolate dynamic import should resolve through the originating page");

    let (marker, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_shared_isolate_dynamic_import_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page dynamic import marker should evaluate");
    server.abort();
    assert_eq!(
        renderer_json_value(marker),
        Some(serde_json::json!("second-dynamic|true")),
        "dynamic import callbacks must route through page B's context bridge after page A closes"
    );

    second_page
        .close_async()
        .await
        .expect("second shared page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_shared_worker_alive_after_peer_page_close() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-worker-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-worker-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first shared worker client</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate SharedWorker page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second shared worker client</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared-isolate SharedWorker page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );

    install_shared_worker_count_probe(&first_page, "shared-isolate-worker")
        .await
        .expect("first shared worker count probe should install");
    wait_for_shared_worker_probe_messages(&first_page, &loader, 1, "first shared worker connect")
        .await
        .expect("first shared worker should connect");
    assert_eq!(
        shared_worker_probe_messages(&first_page)
            .await
            .expect("first shared worker messages"),
        "1"
    );

    install_shared_worker_count_probe(&second_page, "shared-isolate-worker")
        .await
        .expect("second shared worker count probe should install");
    wait_for_shared_worker_probe_messages(&second_page, &loader, 1, "second shared worker connect")
        .await
        .expect("second shared worker should connect");
    assert_eq!(
        shared_worker_probe_messages(&second_page)
            .await
            .expect("second shared worker messages"),
        "2",
        "same-key SharedWorker clients in one browser-context runtime should share a running host"
    );
    assert_eq!(
        runtime
            .browser_context_runtime()
            .shared_worker_running_worker_isolate_count_for_diagnostics(),
        1,
        "two same-key page clients should still produce one SharedWorker worker isolate"
    );

    first_page
        .close_async()
        .await
        .expect("first shared worker page should close");
    assert_eq!(
        runtime
            .browser_context_runtime()
            .shared_worker_running_worker_isolate_count_for_diagnostics(),
        1,
        "closing one page client must not terminate a SharedWorker still used by another page"
    );

    request_shared_worker_probe_count(&second_page)
        .await
        .expect("second page should post count request after first closes");
    wait_for_shared_worker_probe_messages(
        &second_page,
        &loader,
        2,
        "second shared worker count after peer close",
    )
    .await
    .expect("second page should receive count after peer closes");
    assert_eq!(
        shared_worker_probe_messages(&second_page)
            .await
            .expect("second shared worker messages after peer close"),
        "2|2",
        "remaining page client should stay connected to the same running SharedWorker host"
    );

    second_page
        .close_async()
        .await
        .expect("second shared worker page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_isolates_shared_workers_across_browser_context_runtimes() {
    let first_runtime = JsRuntime::initialize();
    let second_runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/shared-worker-cross-runtime").unwrap();

    let mut first_page = create_test_html_page(
        &first_runtime,
        &loader,
        page_url.clone(),
        "<!doctype html><body>first cross-runtime shared worker client</body>",
    )
    .await;
    let mut second_page = create_test_html_page(
        &second_runtime,
        &loader,
        page_url,
        "<!doctype html><body>second cross-runtime shared worker client</body>",
    )
    .await;

    install_shared_worker_count_probe(&first_page, "cross-runtime-worker")
        .await
        .expect("first cross-runtime shared worker count probe should install");
    wait_for_shared_worker_probe_messages(
        &first_page,
        &loader,
        1,
        "first cross-runtime shared worker connect",
    )
    .await
    .expect("first cross-runtime shared worker should connect");
    assert_eq!(
        shared_worker_probe_messages(&first_page)
            .await
            .expect("first cross-runtime shared worker messages"),
        "1"
    );

    install_shared_worker_count_probe(&second_page, "cross-runtime-worker")
        .await
        .expect("second cross-runtime shared worker count probe should install");
    wait_for_shared_worker_probe_messages(
        &second_page,
        &loader,
        1,
        "second cross-runtime shared worker connect",
    )
    .await
    .expect("second cross-runtime shared worker should connect");
    assert_eq!(
        shared_worker_probe_messages(&second_page)
            .await
            .expect("second cross-runtime shared worker messages"),
        "1",
        "same-key SharedWorker clients in different browser-context runtimes must not share a host"
    );

    request_shared_worker_probe_count(&first_page)
        .await
        .expect("first cross-runtime page should post count request");
    wait_for_shared_worker_probe_messages(
        &first_page,
        &loader,
        2,
        "first cross-runtime shared worker count after second runtime connects",
    )
    .await
    .expect("first cross-runtime page should receive count after second runtime connects");
    assert_eq!(
        shared_worker_probe_messages(&first_page)
            .await
            .expect("first cross-runtime shared worker messages after count request"),
        "1|1",
        "first runtime SharedWorker host must remain isolated from the second runtime host"
    );
    assert_eq!(
        first_runtime
            .browser_context_runtime()
            .shared_worker_running_worker_isolate_count_for_diagnostics(),
        1
    );
    assert_eq!(
        second_runtime
            .browser_context_runtime()
            .shared_worker_running_worker_isolate_count_for_diagnostics(),
        1
    );

    first_page
        .close_async()
        .await
        .expect("first cross-runtime shared worker page should close");
    second_page
        .close_async()
        .await
        .expect("second cross-runtime shared worker page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_recreates_shared_worker_after_last_page_client_close() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-worker-last-client-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-worker-last-client-b").unwrap();

    let mut first_page = create_test_html_page(
        &runtime,
        &loader,
        first_url,
        "<!doctype html><body>first last-client shared worker client</body>",
    )
    .await;
    install_shared_worker_count_probe(&first_page, "last-client-fresh-worker")
        .await
        .expect("first last-client shared worker count probe should install");
    wait_for_shared_worker_probe_messages(&first_page, &loader, 1, "first last-client connect")
        .await
        .expect("first last-client shared worker should connect");
    assert_eq!(
        shared_worker_probe_messages(&first_page)
            .await
            .expect("first last-client shared worker messages"),
        "1"
    );
    first_page
        .close_async()
        .await
        .expect("first last-client shared worker page should close");

    let mut second_page = create_test_html_page(
        &runtime,
        &loader,
        second_url,
        "<!doctype html><body>second last-client shared worker client</body>",
    )
    .await;
    install_shared_worker_count_probe(&second_page, "last-client-fresh-worker")
        .await
        .expect("second last-client shared worker count probe should install");
    wait_for_shared_worker_probe_messages(
        &second_page,
        &loader,
        1,
        "fresh same-key shared worker after last client close",
    )
    .await
    .expect("fresh same-key shared worker should connect after last client close");
    assert_eq!(
        shared_worker_probe_messages(&second_page)
            .await
            .expect("second last-client shared worker messages"),
        "1",
        "closing the last page client should terminate the old host before the same key is constructed again"
    );

    second_page
        .close_async()
        .await
        .expect("second last-client shared worker page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_opaque_document_shared_worker_storage_keys_distinct() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let document_url = url::Url::parse("data:text/html,<p>opaque</p>").unwrap();

    let mut first_page = create_test_html_page(
        &runtime,
        &loader,
        document_url.clone(),
        "<!doctype html><body>first opaque shared worker client</body>",
    )
    .await;
    let mut second_page = create_test_html_page(
        &runtime,
        &loader,
        document_url,
        "<!doctype html><body>second opaque shared worker client</body>",
    )
    .await;

    install_shared_worker_count_probe(&first_page, "opaque-storage-key-worker")
        .await
        .expect("first opaque shared worker count probe should install");
    wait_for_shared_worker_probe_messages(&first_page, &loader, 1, "first opaque connect")
        .await
        .expect("first opaque shared worker should connect");
    assert_eq!(
        shared_worker_probe_messages(&first_page)
            .await
            .expect("first opaque shared worker messages"),
        "1"
    );

    install_shared_worker_count_probe(&second_page, "opaque-storage-key-worker")
        .await
        .expect("second opaque shared worker count probe should install");
    wait_for_shared_worker_probe_messages(&second_page, &loader, 1, "second opaque connect")
        .await
        .expect("second opaque shared worker should connect");
    assert_eq!(
        shared_worker_probe_messages(&second_page)
            .await
            .expect("second opaque shared worker messages"),
        "1",
        "same-key SharedWorker clients from distinct opaque documents must not share a host"
    );

    request_shared_worker_probe_count(&first_page)
        .await
        .expect("first opaque page should post count request");
    wait_for_shared_worker_probe_messages(
        &first_page,
        &loader,
        2,
        "first opaque shared worker count after second opaque page connects",
    )
    .await
    .expect("first opaque page should receive count after second opaque page connects");
    assert_eq!(
        shared_worker_probe_messages(&first_page)
            .await
            .expect("first opaque shared worker messages after count request"),
        "1|1",
        "first opaque document SharedWorker host must remain isolated from the second opaque document host"
    );

    first_page
        .close_async()
        .await
        .expect("first opaque shared worker page should close");
    second_page
        .close_async()
        .await
        .expect("second opaque shared worker page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_removes_only_navigated_shared_worker_client() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-worker-navigation-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-worker-navigation-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first shared worker navigation client</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate SharedWorker navigation page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second shared worker navigation client</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared-isolate SharedWorker navigation page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );

    install_shared_worker_count_probe(&first_page, "shared-isolate-navigation-worker")
        .await
        .expect("first shared worker navigation count probe should install");
    wait_for_shared_worker_probe_messages(
        &first_page,
        &loader,
        1,
        "first shared worker navigation connect",
    )
    .await
    .expect("first shared worker navigation client should connect");
    assert_eq!(
        shared_worker_probe_messages(&first_page)
            .await
            .expect("first shared worker navigation messages"),
        "1"
    );

    install_shared_worker_count_probe(&second_page, "shared-isolate-navigation-worker")
        .await
        .expect("second shared worker navigation count probe should install");
    wait_for_shared_worker_probe_messages(
        &second_page,
        &loader,
        1,
        "second shared worker navigation connect",
    )
    .await
    .expect("second shared worker navigation client should connect");
    assert_eq!(
        shared_worker_probe_messages(&second_page)
            .await
            .expect("second shared worker navigation messages"),
        "2"
    );
    assert_eq!(
        runtime
            .browser_context_runtime()
            .shared_worker_running_worker_isolate_count_for_diagnostics(),
        1,
        "same-key SharedWorker clients should share one worker isolate before navigation"
    );

    let replacement_url = "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Cbody%3Ereplacement%20shared%20worker%20navigation%20document%3C/body%3E";
    let (navigation_reply, _) = first_page
        .run_async_command(
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression: format!(
                    r#"(() => {{
  location.href = {replacement_url:?};
  return "navigating";
}})()"#
                ),
                await_promise: false,
            },
        )
        .await
        .expect("shared isolate peer navigation should replace only page A");
    assert_eq!(
        renderer_json_value(navigation_reply),
        Some(serde_json::json!("navigating"))
    );
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("shared unique document isolate count after peer navigation replacement"),
        2,
        "page navigation replacement must retain one distinct isolate for each live page"
    );
    assert_eq!(
        runtime
            .browser_context_runtime()
            .shared_worker_running_worker_isolate_count_for_diagnostics(),
        1,
        "navigating one page client must not terminate a SharedWorker still used by another page"
    );

    request_shared_worker_probe_count(&second_page)
        .await
        .expect("second page should post count request after peer navigation");
    wait_for_shared_worker_probe_messages(
        &second_page,
        &loader,
        2,
        "second shared worker count after peer navigation",
    )
    .await
    .expect("second page should receive count after peer navigation");
    assert_eq!(
        shared_worker_probe_messages(&second_page)
            .await
            .expect("second shared worker messages after peer navigation"),
        "2|2",
        "navigation replacement must remove only page A's client endpoint"
    );

    first_page
        .close_async()
        .await
        .expect("navigated shared worker page should close");
    second_page
        .close_async()
        .await
        .expect("second shared worker navigation page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_rejects_cross_page_runtime_object_ids() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-runtime-object-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-runtime-object-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first runtime object owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate runtime-object page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second runtime object owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared-isolate runtime-object page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );

    let first_object_id = runtime_protocol_object_id(
        &first_page,
        serde_json::json!({
            "id": 41,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "({ marker: 'first-page-object' })"
            }
        }),
        41,
    )
    .await
    .expect("first page Runtime.evaluate should return an objectId");

    let first_call = dispatch_runtime_protocol_for_test(
        &first_page,
        serde_json::json!({
            "id": 42,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": first_object_id,
                "functionDeclaration": "function() { return this.marker; }",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("first page Runtime.callFunctionOn should dispatch");
    let first_call_response =
        runtime_protocol_response_by_id(&first_call, 42).expect("first page call response");
    assert_eq!(
        first_call_response["result"]["result"]["value"],
        serde_json::json!("first-page-object")
    );

    let first_promise_object_id = runtime_protocol_object_id(
        &first_page,
        serde_json::json!({
            "id": 45,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "Promise.resolve('first-page-promise')"
            }
        }),
        45,
    )
    .await
    .expect("first page Runtime.evaluate should return a promise objectId");

    let first_await = dispatch_runtime_protocol_for_test(
        &first_page,
        serde_json::json!({
            "id": 46,
            "method": "Runtime.awaitPromise",
            "params": {
                "promiseObjectId": first_promise_object_id,
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("first page Runtime.awaitPromise should dispatch");
    let first_await_response = runtime_protocol_response_by_id(&first_await, 46)
        .expect("first page awaitPromise response");
    assert_eq!(
        first_await_response["result"]["result"]["value"],
        serde_json::json!("first-page-promise")
    );

    let second_call = dispatch_runtime_protocol_for_test(
        &second_page,
        serde_json::json!({
            "id": 43,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": first_object_id,
                "functionDeclaration": "function() { return this.marker; }",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("second page Runtime.callFunctionOn should dispatch");
    let second_call_response =
        runtime_protocol_response_by_id(&second_call, 43).expect("second page call response");
    assert!(
        second_call_response.get("error").is_some()
            || second_call_response["result"]["exceptionDetails"].is_object(),
        "page B must reject or fail closed for page A's Runtime objectId: {second_call_response:?}"
    );

    let second_await = dispatch_runtime_protocol_for_test(
        &second_page,
        serde_json::json!({
            "id": 47,
            "method": "Runtime.awaitPromise",
            "params": {
                "promiseObjectId": first_promise_object_id,
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("second page Runtime.awaitPromise should dispatch");
    let second_await_response = runtime_protocol_response_by_id(&second_await, 47)
        .expect("second page awaitPromise response");
    assert!(
        second_await_response.get("error").is_some()
            || second_await_response["result"]["exceptionDetails"].is_object(),
        "page B must reject or fail closed for page A's Runtime promiseObjectId: {second_await_response:?}"
    );

    let (second_marker, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.marker ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page marker check should evaluate");
    assert_eq!(
        renderer_json_value(second_marker),
        Some(serde_json::json!("missing")),
        "cross-page Runtime.callFunctionOn must not execute with page A's receiver in page B"
    );

    first_page
        .close_async()
        .await
        .expect("first runtime-object page should close");

    let closed_target_call = dispatch_runtime_protocol_for_test(
        &second_page,
        serde_json::json!({
            "id": 44,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": first_object_id,
                "functionDeclaration": "function() { globalThis.__closedTargetObjectMutatedPeer = true; return this.marker; }",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("closed target Runtime.callFunctionOn should dispatch through peer page");
    let closed_target_call_response = runtime_protocol_response_by_id(&closed_target_call, 44)
        .expect("closed target call response");
    assert!(
        closed_target_call_response.get("error").is_some()
            || closed_target_call_response["result"]["exceptionDetails"].is_object(),
        "page B must reject or fail closed for page A's Runtime objectId after page A closes: {closed_target_call_response:?}"
    );

    let closed_target_await = dispatch_runtime_protocol_for_test(
        &second_page,
        serde_json::json!({
            "id": 48,
            "method": "Runtime.awaitPromise",
            "params": {
                "promiseObjectId": first_promise_object_id,
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("closed target Runtime.awaitPromise should dispatch through peer page");
    let closed_target_await_response = runtime_protocol_response_by_id(&closed_target_await, 48)
        .expect("closed target awaitPromise response");
    assert!(
        closed_target_await_response.get("error").is_some()
            || closed_target_await_response["result"]["exceptionDetails"].is_object(),
        "page B must reject or fail closed for page A's Runtime promiseObjectId after page A closes: {closed_target_await_response:?}"
    );

    let (second_closed_target_marker, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__closedTargetObjectMutatedPeer ?? "not-mutated""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second page closed-target mutation marker should evaluate");
    assert_eq!(
        renderer_json_value(second_closed_target_marker),
        Some(serde_json::json!("not-mutated")),
        "closed target Runtime.callFunctionOn must not execute page A's stale object in page B"
    );

    second_page
        .close_async()
        .await
        .expect("second runtime-object page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_scopes_same_numeric_runtime_evaluate_context_id_to_page() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-runtime-context-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-runtime-context-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first runtime context owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate runtime-context page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second runtime context owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared-isolate runtime-context page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );

    let first_world_context_id =
        create_isolated_world_for_test(&first_page, "shared-runtime-context-world")
            .await
            .expect("first runtime-context isolated world should be created");
    let second_world_context_id =
        create_isolated_world_for_test(&second_page, "shared-runtime-context-world")
            .await
            .expect("second runtime-context isolated world should be created");
    assert_eq!(
        first_world_context_id, second_world_context_id,
        "fresh isolates should be allowed to reuse target-scoped executionContextId values"
    );

    let first_evaluate = dispatch_runtime_protocol_with_context_resolution_for_test(
        &first_page,
        "evaluate",
        serde_json::json!({
            "id": 61,
            "method": "Runtime.evaluate",
            "params": {
                "contextId": first_world_context_id,
                "expression": "globalThis.__runtimeContextOwner = 'first-page'; globalThis.__runtimeContextOwner",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("page A Runtime.evaluate should dispatch with its own context id");
    let first_evaluate_response =
        runtime_protocol_response_by_id(&first_evaluate, 61).expect("page A evaluate response");
    assert_eq!(
        first_evaluate_response["result"]["result"]["value"],
        serde_json::json!("first-page")
    );

    let second_same_numeric_evaluate = dispatch_runtime_protocol_with_context_resolution_for_test(
        &second_page,
        "evaluate",
        serde_json::json!({
            "id": 62,
            "method": "Runtime.evaluate",
            "params": {
                "contextId": first_world_context_id,
                "expression": "globalThis.__runtimeContextOwner = 'cross-page'; globalThis.__runtimeContextOwner",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("page B Runtime.evaluate should dispatch with peer context id");
    let second_same_numeric_evaluate_response =
        runtime_protocol_response_by_id(&second_same_numeric_evaluate, 62)
            .expect("page B target-local evaluate response");
    assert_eq!(
        second_same_numeric_evaluate_response["result"]["result"]["value"],
        serde_json::json!("cross-page"),
        "the reused numeric id must resolve to page B's own isolated world"
    );

    let (first_context_marker_after_peer_attempt, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression: r#"globalThis.__runtimeContextOwner"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first isolated world marker should evaluate after peer attempt");
    assert_eq!(
        renderer_json_value(first_context_marker_after_peer_attempt),
        Some(serde_json::json!("first-page")),
        "page B's target-local Runtime.evaluate must not mutate page A"
    );

    let (second_default_marker_after_peer_attempt, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__runtimeContextOwner ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second default marker should evaluate after target-local attempt");
    assert_eq!(
        renderer_json_value(second_default_marker_after_peer_attempt),
        Some(serde_json::json!("missing")),
        "target-local isolated-world context id must not fall back to page B's default world"
    );

    let (second_world_marker_after_peer_attempt, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_world_context_id,
            expression: r#"globalThis.__runtimeContextOwner ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second isolated world marker should evaluate after target-local attempt");
    assert_eq!(
        renderer_json_value(second_world_marker_after_peer_attempt),
        Some(serde_json::json!("cross-page")),
        "the reused numeric id must mutate only page B's isolated world"
    );

    first_page
        .close_async()
        .await
        .expect("first runtime-context page should close");
    second_page
        .close_async()
        .await
        .expect("second runtime-context page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_scopes_same_numeric_runtime_evaluate_default_context_id_to_page() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url =
        url::Url::parse("https://example.test/shared-runtime-default-context-a").unwrap();
    let second_url =
        url::Url::parse("https://example.test/shared-runtime-default-context-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first default runtime context owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate default runtime-context page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second default runtime context owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared-isolate default runtime-context page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );

    let first_default_context_id = default_or_initial_execution_context_id_for_test(&first_page)
        .await
        .expect("first page should expose a default execution context id");
    let second_default_context_id = default_or_initial_execution_context_id_for_test(&second_page)
        .await
        .expect("second page should expose a default execution context id");
    assert_eq!(
        first_default_context_id, second_default_context_id,
        "fresh isolates should be allowed to reuse target-scoped default context ids"
    );

    let first_evaluate = dispatch_runtime_protocol_with_context_resolution_for_test(
        &first_page,
        "evaluate",
        serde_json::json!({
            "id": 63,
            "method": "Runtime.evaluate",
            "params": {
                "contextId": first_default_context_id,
                "expression": "globalThis.__runtimeDefaultContextOwner = 'first-page'; globalThis.__runtimeDefaultContextOwner",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("page A Runtime.evaluate should dispatch with its own default context id");
    let first_evaluate_response =
        runtime_protocol_response_by_id(&first_evaluate, 63).expect("page A evaluate response");
    assert_eq!(
        first_evaluate_response["result"]["result"]["value"],
        serde_json::json!("first-page")
    );

    let second_same_numeric_evaluate = dispatch_runtime_protocol_with_context_resolution_for_test(
        &second_page,
        "evaluate",
        serde_json::json!({
            "id": 64,
            "method": "Runtime.evaluate",
            "params": {
                "contextId": first_default_context_id,
                "expression": "globalThis.__runtimeDefaultContextOwner = 'cross-page'; globalThis.__runtimeDefaultContextOwner",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("page B Runtime.evaluate should dispatch with peer default context id");
    let second_same_numeric_evaluate_response =
        runtime_protocol_response_by_id(&second_same_numeric_evaluate, 64)
            .expect("page B target-local default evaluate response");
    assert_eq!(
        second_same_numeric_evaluate_response["result"]["result"]["value"],
        serde_json::json!("cross-page"),
        "the reused numeric id must resolve to page B's own default world"
    );

    let (first_marker_after_peer_attempt, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__runtimeDefaultContextOwner"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first default marker should evaluate after peer attempt");
    assert_eq!(
        renderer_json_value(first_marker_after_peer_attempt),
        Some(serde_json::json!("first-page")),
        "page B's target-local Runtime.evaluate must not mutate page A"
    );

    let (second_marker_after_peer_attempt, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__runtimeDefaultContextOwner ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second default marker should evaluate after target-local access");
    assert_eq!(
        renderer_json_value(second_marker_after_peer_attempt),
        Some(serde_json::json!("cross-page")),
        "the reused numeric id must mutate only page B's default world"
    );

    first_page
        .close_async()
        .await
        .expect("first default runtime-context page should close");
    second_page
        .close_async()
        .await
        .expect("second default runtime-context page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_scopes_same_numeric_runtime_evaluate_child_context_id_to_page() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-runtime-child-context-a").unwrap();
    let second_url =
        url::Url::parse("https://example.test/shared-runtime-child-context-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            r#"<!doctype html><body><iframe srcdoc="<body>first runtime child</body>"></iframe></body>"#
                .to_owned(),
            None,

            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate child runtime-context page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            r#"<!doctype html><body><iframe srcdoc="<body>second runtime child</body>"></iframe></body>"#
                .to_owned(),
            None,

            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared-isolate child runtime-context page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );

    let first_child_context_ids = child_default_context_ids_for_test(&first_page)
        .await
        .expect("first child context events should replay");
    let second_child_context_ids = child_default_context_ids_for_test(&second_page)
        .await
        .expect("second child context events should replay");
    assert_eq!(
        first_child_context_ids.len(),
        1,
        "first page should expose exactly one child default context"
    );
    assert_eq!(
        second_child_context_ids.len(),
        1,
        "second page should expose exactly one child default context"
    );
    let first_child_context_id = first_child_context_ids[0];
    let second_child_context_id = second_child_context_ids[0];
    assert_eq!(
        first_child_context_id, second_child_context_id,
        "fresh isolates should be allowed to reuse target-scoped child context ids"
    );

    let first_evaluate = dispatch_runtime_protocol_with_context_resolution_for_test(
        &first_page,
        "evaluate",
        serde_json::json!({
            "id": 65,
            "method": "Runtime.evaluate",
            "params": {
                "contextId": first_child_context_id,
                "expression": "globalThis.__runtimeChildContextOwner = 'first-child'; globalThis.__runtimeChildContextOwner",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("page A Runtime.evaluate should dispatch with its own child context id");
    let first_evaluate_response = runtime_protocol_response_by_id(&first_evaluate, 65)
        .expect("page A child evaluate response");
    assert_eq!(
        first_evaluate_response["result"]["result"]["value"],
        serde_json::json!("first-child")
    );

    let second_same_numeric_evaluate = dispatch_runtime_protocol_with_context_resolution_for_test(
        &second_page,
        "evaluate",
        serde_json::json!({
            "id": 66,
            "method": "Runtime.evaluate",
            "params": {
                "contextId": first_child_context_id,
                "expression": "globalThis.__runtimeChildContextOwner = 'cross-page'; globalThis.__runtimeChildContextOwner",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("page B Runtime.evaluate should dispatch with peer child context id");
    let second_same_numeric_evaluate_response =
        runtime_protocol_response_by_id(&second_same_numeric_evaluate, 66)
            .expect("page B target-local child evaluate response");
    assert_eq!(
        second_same_numeric_evaluate_response["result"]["result"]["value"],
        serde_json::json!("cross-page"),
        "the reused numeric id must resolve to page B's own child realm"
    );

    let (first_marker_after_peer_attempt, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_child_context_id,
            expression: r#"globalThis.__runtimeChildContextOwner"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first child marker should evaluate after peer attempt");
    assert_eq!(
        renderer_json_value(first_marker_after_peer_attempt),
        Some(serde_json::json!("first-child")),
        "page B's target-local Runtime.evaluate must not mutate page A's child frame"
    );

    let (second_default_marker_after_peer_attempt, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__runtimeChildContextOwner ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second default marker should evaluate after target-local child access");
    assert_eq!(
        renderer_json_value(second_default_marker_after_peer_attempt),
        Some(serde_json::json!("missing")),
        "target-local child context access must not fall back to page B's default world"
    );

    let (second_child_marker_after_peer_attempt, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_child_context_id,
            expression: r#"globalThis.__runtimeChildContextOwner ?? "missing""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second child marker should evaluate after target-local access");
    assert_eq!(
        renderer_json_value(second_child_marker_after_peer_attempt),
        Some(serde_json::json!("cross-page")),
        "the reused numeric id must mutate only page B's child realm"
    );

    first_page
        .close_async()
        .await
        .expect("first child runtime-context page should close");
    second_page
        .close_async()
        .await
        .expect("second child runtime-context page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn child_default_runtime_evaluate_pending_await_promise_does_not_leak_internal_token() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/child-runtime-await-promise").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><body><iframe srcdoc="<body>pending promise child</body>"></iframe></body>"#,
    )
    .await;

    let child_context_ids = child_default_context_ids_for_test(&page)
        .await
        .expect("child context events should replay");
    assert_eq!(
        child_context_ids.len(),
        1,
        "page should expose exactly one child default context"
    );
    let child_context_id = child_context_ids[0];

    let evaluate = dispatch_runtime_protocol_with_context_resolution_for_test(
        &page,
        "evaluate",
        serde_json::json!({
            "id": 67,
            "method": "Runtime.evaluate",
            "params": {
                "contextId": child_context_id,
                "expression": "new Promise(() => {})",
                "awaitPromise": true,
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("child Runtime.evaluate awaitPromise should dispatch");
    let response =
        runtime_protocol_response_by_id(&evaluate, 67).expect("child awaitPromise response");
    let messages_json =
        serde_json::to_string(&evaluate).expect("Runtime.evaluate messages should serialize");
    assert!(
        !messages_json.contains("__moliAwaitPromiseToken"),
        "child Runtime.evaluate awaitPromise leaked internal polling token: {evaluate:?}"
    );
    assert!(
        !messages_json.contains("__moliPendingPromise"),
        "child Runtime.evaluate awaitPromise leaked internal pending marker: {evaluate:?}"
    );
    assert!(
        response.get("error").is_some() || response["result"]["exceptionDetails"].is_object(),
        "pending child Runtime.evaluate awaitPromise must fail closed instead of returning an internal success payload: {response:?}"
    );

    page.close_async()
        .await
        .expect("child runtime awaitPromise page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_release_object_group_page_local() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-runtime-release-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-runtime-release-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first release group owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate release-group page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second release group owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared-isolate release-group page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );

    let first_object_id = runtime_protocol_object_id(
        &first_page,
        serde_json::json!({
            "id": 51,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "({ marker: 'first-release-group-object' })",
                "objectGroup": "same-release-group"
            }
        }),
        51,
    )
    .await
    .expect("first page Runtime.evaluate should return grouped objectId");
    let second_object_id = runtime_protocol_object_id(
        &second_page,
        serde_json::json!({
            "id": 52,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "({ marker: 'second-release-group-object' })",
                "objectGroup": "same-release-group"
            }
        }),
        52,
    )
    .await
    .expect("second page Runtime.evaluate should return grouped objectId");

    let release = dispatch_runtime_protocol_for_test(
        &first_page,
        serde_json::json!({
            "id": 53,
            "method": "Runtime.releaseObjectGroup",
            "params": { "objectGroup": "same-release-group" }
        }),
    )
    .await
    .expect("first page Runtime.releaseObjectGroup should dispatch");
    let release_response =
        runtime_protocol_response_by_id(&release, 53).expect("first page release response");
    assert_eq!(release_response["result"], serde_json::json!({}));

    let first_call_after_release = dispatch_runtime_protocol_for_test(
        &first_page,
        serde_json::json!({
            "id": 54,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": first_object_id,
                "functionDeclaration": "function() { return this.marker; }",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("first page call after release should dispatch");
    let first_call_after_release_response =
        runtime_protocol_response_by_id(&first_call_after_release, 54)
            .expect("first page call-after-release response");
    assert!(
        first_call_after_release_response.get("error").is_some()
            || first_call_after_release_response["result"]["exceptionDetails"].is_object(),
        "page A releaseObjectGroup should remove page A's grouped handle: {first_call_after_release_response:?}"
    );

    let second_call_after_peer_release = dispatch_runtime_protocol_for_test(
        &second_page,
        serde_json::json!({
            "id": 55,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": second_object_id,
                "functionDeclaration": "function() { return this.marker; }",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("second page call after peer release should dispatch");
    let second_call_after_peer_release_response =
        runtime_protocol_response_by_id(&second_call_after_peer_release, 55)
            .expect("second page call-after-peer-release response");
    assert_eq!(
        second_call_after_peer_release_response["result"]["result"]["value"],
        serde_json::json!("second-release-group-object"),
        "page A releaseObjectGroup must not clear page B's same-name group in the shared isolate"
    );

    first_page
        .close_async()
        .await
        .expect("first release-group page should close");
    second_page
        .close_async()
        .await
        .expect("second release-group page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_replays_runtime_contexts_per_page() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-runtime-replay-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-runtime-replay-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first runtime replay owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate runtime-replay page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second runtime replay owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared-isolate runtime-replay page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );

    let first_world_context_id =
        create_isolated_world_for_test(&first_page, "shared-runtime-world")
            .await
            .expect("first runtime replay isolated world should be created");
    let second_world_context_id =
        create_isolated_world_for_test(&second_page, "shared-runtime-world")
            .await
            .expect("second runtime replay isolated world should be created");
    let first_events = runtime_enable_events_for_test(&first_page)
        .await
        .expect("first page Runtime.enable replay should run");
    let first_context_ids = runtime_execution_context_ids(&first_events);
    assert!(
        first_context_ids.contains(&first_world_context_id),
        "first Runtime.enable replay should include first page isolated world: {first_events:?}"
    );
    let first_isolated_context =
        runtime_execution_context_by_id(&first_events, first_world_context_id)
            .expect("first Runtime.enable replay should expose the first isolated world context");
    assert!(
        first_isolated_context["uniqueId"].as_str().is_some(),
        "first isolated context should come from V8 RuntimeAgent native replay, not Moli synthetic fallback: {first_isolated_context:?}"
    );
    let first_isolated_unique_id = first_isolated_context["uniqueId"]
        .as_str()
        .expect("first isolated context uniqueId")
        .to_owned();

    let second_events = runtime_enable_events_for_test(&second_page)
        .await
        .expect("second page Runtime.enable replay should run");
    let second_context_ids = runtime_execution_context_ids(&second_events);
    assert!(
        second_context_ids.contains(&second_world_context_id),
        "second Runtime.enable replay should include second page isolated world: {second_events:?}"
    );
    let second_isolated_context =
        runtime_execution_context_by_id(&second_events, second_world_context_id)
            .expect("second Runtime.enable replay should expose the second isolated world context");
    assert!(
        second_isolated_context["uniqueId"].as_str().is_some(),
        "second isolated context should come from V8 RuntimeAgent native replay, not Moli synthetic fallback: {second_isolated_context:?}"
    );
    let second_isolated_unique_id = second_isolated_context["uniqueId"]
        .as_str()
        .expect("second isolated context uniqueId");
    assert_ne!(
        first_isolated_unique_id, second_isolated_unique_id,
        "target-scoped numeric context ids may collide, but V8 uniqueIds must identify different realms"
    );
    assert!(
        !runtime_execution_context_unique_ids(&first_events).contains(&second_isolated_unique_id),
        "first Runtime.enable replay must not include page B's realm uniqueId: {first_events:?}"
    );
    assert!(
        !runtime_execution_context_unique_ids(&second_events)
            .contains(&first_isolated_unique_id.as_str()),
        "second Runtime.enable replay must not include page A's realm uniqueId: {second_events:?}"
    );

    let first_default_context_count = runtime_default_context_count(&first_events);
    let second_default_context_count = runtime_default_context_count(&second_events);
    assert_eq!(
        first_default_context_count, 1,
        "first Runtime.enable replay should expose one page default context"
    );
    assert_eq!(
        second_default_context_count, 1,
        "second Runtime.enable replay should expose one page default context"
    );

    first_page
        .close_async()
        .await
        .expect("first runtime-replay page should close");
    second_page
        .close_async()
        .await
        .expect("second runtime-replay page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_enable_waits_for_queued_child_realm_before_reporting_contexts() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/runtime-enable-child-barrier").expect("test url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>child realm barrier</body>",
    )
    .await;

    // Queue Runtime.enable before yielding. The Page scheduler admits the
    // child-realm wake after the setup command, then permits one already-ready
    // command to overtake that Page turn. This makes the production race
    // deterministic: Runtime.enable must park behind the exact-Document realm
    // task instead of reporting an incomplete current-context inventory.
    let setup = page
        .enqueue_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  const frame = document.createElement("iframe");
  frame.id = "runtime-enable-barrier-child";
  document.body.appendChild(frame);
  void frame.contentWindow.Function;
  return "queued";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .expect("child realm setup command should enqueue");
    let enable = page
        .enqueue_async_command(RendererPageCommand::runtime_enable_events(Some(
            "SID-runtime-enable-child-barrier".to_owned(),
        )))
        .expect("Runtime.enable command should enqueue behind child realm setup");

    let setup = setup
        .wait()
        .await
        .expect("child realm setup command should complete");
    let (setup, _) = setup.into_completion_and_predecessor();
    let (setup_reply, _, _) = setup.into_parts();
    assert_eq!(
        renderer_json_value(setup_reply),
        Some(serde_json::json!("queued"))
    );

    let enable = enable
        .wait()
        .await
        .expect("Runtime.enable should resume after child realm materialization");
    let (enable, _) = enable.into_completion_and_predecessor();
    let (enable_reply, _, _) = enable.into_parts();
    let RendererPageReply::RuntimeInspectorProtocolMessages(output) = enable_reply else {
        panic!("Runtime.enable should return inspector protocol messages");
    };
    let messages = output
        .into_messages()
        .into_iter()
        .map(runtime_inspector_message_protocol_message_for_test)
        .collect::<Vec<_>>();
    let child_context = messages.iter().find(|message| {
        message["method"] == serde_json::json!("Runtime.executionContextCreated")
            && message["params"]["context"]["auxData"]["isDefault"] == serde_json::json!(true)
            && message["params"]["context"]["auxData"]["frameId"].is_string()
    });
    assert!(
        child_context.is_some(),
        "Runtime.enable must report the child context created by its materialization prerequisite: {messages:?}"
    );

    page.close_async()
        .await
        .expect("Runtime.enable child barrier page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_enable_events_for_new_inspector_session_replays_existing_isolated_worlds() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/runtime-enable-new-session").expect("test url");

    let (page, _, _, _creation_artifacts, pending_download) = runtime
        .create_html_page_from_response(
            page_url.clone(),
            page_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>new inspector session</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("new-session runtime replay test page should load");
    assert!(pending_download.is_none());

    let world_context_id = create_isolated_world_for_test(&page, "new-session-utility")
        .await
        .expect("isolated world should be created before the inspector session enables Runtime");

    let events = runtime_enable_events_for_inspector_session_for_test(
        &page,
        Some("SID-new-runtime-session"),
    )
    .await
    .expect("new inspector session Runtime.enable should run");
    let context_ids = runtime_execution_context_ids(&events);
    assert!(
        context_ids.contains(&world_context_id),
        "new inspector session Runtime.enable should replay existing isolated world: {events:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_enable_events_include_renderer_root_frame_id_for_default_context() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/runtime-enable-root-frame").expect("test url");
    let root_frame_id = "TID-runtime-enable-root-frame";

    let (mut page, _, _, _creation_artifacts, pending_download) = runtime
        .create_html_page_from_response_with_inspector_session_restores(
            page_url.clone(),
            page_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>root frame id</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
            Vec::new(),
            Some(root_frame_id.to_owned()),
            None,
        )
        .await
        .expect("root-frame test page should load");
    assert!(pending_download.is_none());

    let events = runtime_enable_events_for_test(&page)
        .await
        .expect("Runtime.enable replay should run");
    let default_contexts = events
        .iter()
        .filter(|message| {
            message.get("method") == Some(&serde_json::json!("Runtime.executionContextCreated"))
                && message["params"]["context"]["auxData"]["isDefault"] == serde_json::json!(true)
                && message["params"]["context"]["auxData"]["type"] == serde_json::json!("default")
        })
        .collect::<Vec<_>>();

    assert_eq!(
        default_contexts.len(),
        1,
        "Runtime.enable should expose exactly one top-level default context: {events:?}"
    );
    assert_eq!(
        default_contexts[0]["params"]["context"]["auxData"]["frameId"],
        serde_json::json!(root_frame_id),
        "renderer Runtime.enable output should carry the root frame id before protocol emission"
    );
    assert_eq!(
        default_contexts[0]["params"]["context"]["origin"],
        serde_json::json!("https://example.test"),
        "renderer Runtime.enable output should carry the document security origin"
    );

    page.close_async()
        .await
        .expect("root-frame test page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_scopes_runtime_bindings_to_page_worlds() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-runtime-binding-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-runtime-binding-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first runtime binding owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate runtime-binding page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second runtime binding owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared-isolate runtime-binding page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );
    output_rx.drain();

    let first_world_context_id =
        create_isolated_world_for_test(&first_page, "shared-binding-world")
            .await
            .expect("first runtime binding isolated world should be created");
    let second_world_context_id =
        create_isolated_world_for_test(&second_page, "shared-binding-world")
            .await
            .expect("second runtime binding isolated world should be created");
    add_runtime_binding_for_test(
        &first_page,
        "sharedBinding",
        Some("shared-binding-world"),
        None,
    )
    .await
    .expect("first page scoped runtime binding should install");

    let (first_binding_type, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression: r#"typeof sharedBinding"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first isolated world binding type should evaluate");
    assert_eq!(
        renderer_json_value(first_binding_type),
        Some(serde_json::json!("function"))
    );

    let (second_binding_type, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_world_context_id,
            expression: r#"typeof sharedBinding"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second isolated world binding type should evaluate");
    assert_eq!(
        renderer_json_value(second_binding_type),
        Some(serde_json::json!("undefined")),
        "Runtime.addBinding scoped to page A's isolated world name must not install on page B's same-name isolated world"
    );

    let (first_binding_call_result, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression: r#"sharedBinding("from-first-world"); "called""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first isolated world binding call should evaluate");
    assert_eq!(
        renderer_json_value(first_binding_call_result),
        Some(serde_json::json!("called"))
    );
    let first_binding_calls = output_rx.drain_runtime_binding_calls_for_page(&first_page);
    assert_eq!(first_binding_calls.len(), 1);
    assert_eq!(first_binding_calls[0].name, "sharedBinding");
    assert_eq!(first_binding_calls[0].payload, "from-first-world");
    assert_eq!(
        first_binding_calls[0].execution_context_id, first_world_context_id,
        "binding call should map back to page A's compatibility context id"
    );

    let second_binding_calls = output_rx.drain_runtime_binding_calls_for_page(&second_page);
    assert!(
        second_binding_calls.is_empty(),
        "page B must not receive binding calls from page A's isolated world"
    );

    first_page
        .close_async()
        .await
        .expect("first runtime-binding page should close");
    second_page
        .close_async()
        .await
        .expect("second runtime-binding page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn set_runtime_binding_state_updates_renderer_inspector_session_store() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/renderer-runtime-binding-state").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url,
        "<!doctype html><body>renderer runtime binding state</body>",
    )
    .await;
    let binding = crate::protocol_types::RuntimeBindingRegistration {
        devtools_session: None,
        name: "rendererSessionStoredBinding".to_owned(),
        execution_context_name: None,
    };

    set_runtime_binding_state_for_test(&page, None, vec![binding.clone()], vec![binding])
        .await
        .expect("renderer runtime binding state should update");
    runtime_enable_events_for_test(&page)
        .await
        .expect("Runtime.enable should replay renderer-stored bindings");

    let (binding_type, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"typeof rendererSessionStoredBinding"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("renderer-stored binding type should evaluate");
    assert_eq!(
        renderer_json_value(binding_type),
        Some(serde_json::json!("function")),
        "Runtime.enable should replay bindings from renderer inspector session state"
    );

    page.close_async()
        .await
        .expect("renderer binding state test page should close");

    let clearing_url =
        url::Url::parse("https://example.test/renderer-runtime-binding-state-cleared").unwrap();
    let mut clearing_page = create_test_html_page(
        &runtime,
        &loader,
        clearing_url,
        "<!doctype html><body>renderer runtime binding state cleared</body>",
    )
    .await;
    let scoped_binding = crate::protocol_types::RuntimeBindingRegistration {
        devtools_session: None,
        name: "rendererSessionClearedBinding".to_owned(),
        execution_context_name: Some("cleared-binding-world".to_owned()),
    };

    set_runtime_binding_state_for_test(
        &clearing_page,
        None,
        vec![scoped_binding.clone()],
        vec![scoped_binding],
    )
    .await
    .expect("renderer runtime binding state should accept pending named-world binding");
    set_runtime_binding_state_for_test(&clearing_page, None, Vec::new(), Vec::new())
        .await
        .expect("renderer runtime binding state should clear pending named-world binding");
    runtime_enable_events_for_test(&clearing_page)
        .await
        .expect("Runtime.enable should run with cleared renderer session state");
    let cleared_world_context_id =
        create_isolated_world_for_test(&clearing_page, "cleared-binding-world")
            .await
            .expect("cleared binding world should be created");
    let (cleared_binding_type, _) = clearing_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: cleared_world_context_id,
            expression: r#"typeof rendererSessionClearedBinding"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("cleared renderer-stored binding type should evaluate");
    assert_eq!(
        renderer_json_value(cleared_binding_type),
        Some(serde_json::json!("undefined")),
        "cleared renderer inspector session state must not replay stale named-world bindings"
    );

    clearing_page
        .close_async()
        .await
        .expect("renderer binding clearing test page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn apply_runtime_protocol_state_keeps_session_binding_replay_scoped() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url =
        url::Url::parse("https://example.test/runtime-protocol-state-session-bindings").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url,
        "<!doctype html><body>runtime protocol state session bindings</body>",
    )
    .await;
    let stored_only_binding = crate::protocol_types::RuntimeBindingRegistration {
        devtools_session: None,
        name: "storedOnlySessionBinding".to_owned(),
        execution_context_name: Some("stored-only-world".to_owned()),
    };

    let (reply, _) = page
        .run_async_command(RendererPageCommand::apply_runtime_protocol_state(
            Some("SID-primary".to_owned()),
            Vec::new(),
            Vec::new(),
            vec![stored_only_binding],
            Vec::new(),
        ))
        .await
        .expect("runtime protocol state should apply");
    assert!(
        matches!(reply, RendererPageReply::Unit),
        "expected ApplyRuntimeProtocolState to return unit reply"
    );

    let context_id = create_isolated_world_runtime_activity_for_test(
        &page,
        Some("SID-primary"),
        "stored-only-world",
    )
    .await
    .expect("runtime activity should create stored-only world");
    let (binding_type, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: context_id,
            expression: "typeof storedOnlySessionBinding".to_owned(),
            await_promise: false,
        })
        .await
        .expect("stored-only binding type should evaluate");
    assert_eq!(
        renderer_json_value(binding_type),
        Some(serde_json::json!("undefined")),
        "ApplyRuntimeProtocolState must not copy page-level stored bindings into the current inspector session replay store"
    );

    page.close_async()
        .await
        .expect("renderer protocol-state session binding test page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn inspector_output_flushes_v8_state_for_commands_and_notifications() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/inspector-state-output").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url,
        "<!doctype html><body>inspector state output</body>",
    )
    .await;

    let (enable_messages, enable_output) = dispatch_runtime_protocol_with_output_for_test(
        &page,
        serde_json::json!({
            "id": 701,
            "method": "Runtime.enable",
        }),
    )
    .await
    .expect("Runtime.enable should dispatch");
    assert!(
        runtime_protocol_response_by_id(&enable_messages, 701)
            .is_some_and(|message| message.get("error").is_none()),
        "Runtime.enable should return a successful response: {enable_messages:?}"
    );
    let command_state = enable_output
        .v8_state_update()
        .expect("a V8 command response should flush the latest session state");
    assert!(
        !command_state.is_empty(),
        "Runtime.enable should produce a non-empty V8 state cookie"
    );
    assert!(
        output_rx.drain().iter().all(|publication| {
            publication.records().iter().all(|record| {
                !matches!(
                    record.item(),
                    RendererOutputItem::Observation(
                        RendererProtocolObservation::RuntimeInspector(batch)
                    ) if batch.messages.iter().any(|message| matches!(
                        message,
                        RendererRuntimeInspectorMessage::Protocol(message)
                            if message.get("id").and_then(serde_json::Value::as_i64) == Some(701)
                    ))
                )
            })
        }),
        "a synchronous Runtime response must not leak into the live notification stream"
    );

    page.run_async_command(RendererPageCommand::EvaluateExpression {
        expression: "console.log('state-notification-marker')".to_owned(),
        await_promise: false,
    })
    .await
    .expect("console evaluation should complete");
    assert!(
        output_rx.drain().iter().any(|publication| {
            publication.records().iter().any(|record| matches!(
                record.item(),
                RendererOutputItem::Observation(RendererProtocolObservation::RuntimeInspector(batch))
                    if batch.v8_state_update
                        .as_ref()
                        .is_some_and(|state| !state.is_empty())
                        && batch.messages.iter().any(|message| matches!(
                            message,
                            RendererRuntimeInspectorMessage::Protocol(message)
                                if message.get("method")
                                    == Some(&serde_json::json!("Runtime.consoleAPICalled"))
                        ))
            ))
        }),
        "the producing turn's concrete Runtime notification must carry the latest V8 state"
    );

    page.close_async()
        .await
        .expect("inspector state output page should close");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owner_command_fences_exact_cursor_when_later_output_is_already_published() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/concurrent-command-output").unwrap();
    let mut page =
        create_test_html_page(&runtime, &loader, url, "<!doctype html><body>before</body>").await;

    let (enable_messages, _) = dispatch_runtime_protocol_with_output_for_test(
        &page,
        serde_json::json!({
            "id": 711,
            "method": "Runtime.enable",
        }),
    )
    .await
    .expect("Runtime.enable should dispatch");
    assert!(
        runtime_protocol_response_by_id(&enable_messages, 711)
            .is_some_and(|message| message.get("error").is_none()),
        "Runtime.enable should succeed before the command-output race: {enable_messages:?}"
    );
    output_rx.drain();

    runtime.publish_next_command_output_before_owner_settlement_for_testing();
    let pending_evaluate = page
        .enqueue_async_command(RendererPageCommand::EvaluateExpression {
            expression: "console.log('concurrent-command-output'); 'done'".to_owned(),
            await_promise: false,
        })
        .expect("Runtime.evaluate should enqueue");
    let concurrent_publication = tokio::time::timeout(Duration::from_secs(3), output_rx.recv())
        .await
        .expect("the test publication should not hang")
        .expect("the test hook should settle the pending owner-command records");
    assert!(
        concurrent_publication
            .records()
            .iter()
            .any(|record| matches!(
                record.item(),
                RendererOutputItem::Observation(RendererProtocolObservation::RuntimeInspector(batch))
                    if batch.messages.iter().any(|message| matches!(
                        message,
                        RendererRuntimeInspectorMessage::Protocol(message)
                            if message.get("method")
                                == Some(&serde_json::json!("Runtime.consoleAPICalled"))
                    ))
            )),
        "the early publication must own the Runtime notification emitted by the owner command"
    );
    let trailing_publication = tokio::time::timeout(Duration::from_secs(3), output_rx.recv())
        .await
        .expect("the trailing test publication should not hang")
        .expect("final owner settlement should publish a later independent batch");
    assert!(
        trailing_publication.records().iter().any(|record| matches!(
            record.item(),
            RendererOutputItem::Observation(RendererProtocolObservation::RuntimeLifecycleError {
                text,
                execution_context_id: None,
            }) if text == "test trailing output publication"
        )),
        "final Page settlement should publish the independent trailing test output"
    );
    assert_eq!(
        trailing_publication.cursor().sequence(),
        concurrent_publication.cursor().sequence() + 1,
        "the independent output must follow the command publication"
    );

    let evaluate_output = tokio::time::timeout(Duration::from_secs(3), pending_evaluate.wait())
        .await
        .expect("owner command should not hang after the IO publication")
        .expect("owner command should complete after its records were settled by IO");
    assert_eq!(
        evaluate_output
            .renderer_output_predecessor()
            .expect("owner response must retain the concurrently settled prefix")
            .cursor(),
        concurrent_publication.cursor()
    );
    assert_ne!(
        evaluate_output
            .renderer_output_predecessor()
            .expect("owner response must retain an exact output prefix")
            .cursor(),
        trailing_publication.cursor(),
        "the response fence must not drift to output published after its command batch"
    );

    page.close_async()
        .await
        .expect("concurrent command output page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn page_diagnostics_snapshot_is_read_only_for_current_inspector_output() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/exact-document-inspector-snapshot").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url,
        "<!doctype html><body>exact document inspector snapshot</body>",
    )
    .await;

    let (identity_reply, _) = page
        .run_async_command(RendererPageCommand::PageDiagnosticsSnapshot)
        .await
        .expect("initial page diagnostics snapshot should complete");
    let RendererPageReply::PageDiagnosticsSnapshot(identity_snapshot) = identity_reply else {
        panic!("page diagnostics command should return an activity snapshot");
    };
    let current_document = identity_snapshot
        .document_lifecycle_identity()
        .expect("snapshot should carry the current Document identity");
    while output_rx.try_recv().is_ok() {}

    dispatch_runtime_protocol_with_output_for_test(
        &page,
        serde_json::json!({
            "id": 702,
            "method": "Runtime.enable",
        }),
    )
    .await
    .expect("Runtime.enable should dispatch");
    while output_rx.try_recv().is_ok() {}
    page.enqueue_async_command(RendererPageCommand::EvaluateExpression {
        expression: "console.log('exact-document-snapshot-marker')".to_owned(),
        await_promise: false,
    })
    .expect("console evaluation should enqueue")
    .wait()
    .await
    .expect("console evaluation should complete");
    assert!(
        output_rx.drain().iter().any(|publication| {
            publication.records().iter().any(|record| matches!(
                record.item(),
                RendererOutputItem::Observation(RendererProtocolObservation::RuntimeInspector(batch))
                    if batch.messages.iter().any(|message| matches!(
                        message,
                        RendererRuntimeInspectorMessage::Protocol(message)
                            if message.get("method")
                                == Some(&serde_json::json!("Runtime.consoleAPICalled"))
                    ))
            ))
        }),
        "console output must be frozen in the producing turn's concrete publication"
    );

    let (snapshot_reply, _) = page
        .run_async_command(RendererPageCommand::PageDiagnosticsSnapshot)
        .await
        .expect("read-only activity snapshot should complete");
    let RendererPageReply::PageDiagnosticsSnapshot(snapshot) = snapshot_reply else {
        panic!("activity command should return a snapshot");
    };
    assert_eq!(
        snapshot.document_lifecycle_identity(),
        Some(current_document)
    );
    assert_eq!(
        snapshot.diagnostics.pending_inspector_messages, 0,
        "published Inspector messages must not remain as a diagnostics-owned output queue"
    );
    assert!(
        snapshot
            .runtime_observable_source()
            .is_some_and(|source| source.source_items().iter().any(|item| matches!(
                item,
                super::RendererRuntimeObservableSourceItem::ConsoleMessage { message, .. }
                    if message.message == "log: exact-document-snapshot-marker"
            ))),
        "diagnostics should expose read-only source state without owning the protocol publication"
    );

    page.close_async()
        .await
        .expect("read-only Inspector diagnostics page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_stateful_inspector_command_preserves_v8_state() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/inspector-failed-state").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url,
        "<!doctype html><body>failed inspector state</body>",
    )
    .await;

    for request in [
        serde_json::json!({"id": 711, "method": "Profiler.enable"}),
        serde_json::json!({
            "id": 712,
            "method": "Profiler.setSamplingInterval",
            "params": {"interval": 937},
        }),
        serde_json::json!({"id": 713, "method": "Profiler.start"}),
    ] {
        let response_id = request["id"].as_i64().expect("numeric request id");
        let (messages, _) = dispatch_runtime_protocol_with_output_for_test(&page, request)
            .await
            .expect("Profiler setup command should dispatch");
        assert!(
            runtime_protocol_response_by_id(&messages, response_id)
                .is_some_and(|message| message.get("error").is_none()),
            "Profiler setup command {response_id} should succeed: {messages:?}"
        );
    }
    let (_, before_failed_command) = dispatch_runtime_protocol_with_output_for_test(
        &page,
        serde_json::json!({"id": 714, "method": "Runtime.enable"}),
    )
    .await
    .expect("state checkpoint command should dispatch");
    let before_failed_command = before_failed_command
        .v8_state_update()
        .cloned()
        .expect("state checkpoint should flush a cookie");

    let (failed_messages, failed_output) = dispatch_runtime_protocol_with_output_for_test(
        &page,
        serde_json::json!({
            "id": 715,
            "method": "Profiler.setSamplingInterval",
            "params": {"interval": 123},
        }),
    )
    .await
    .expect("invalid stateful command should still return a protocol response");
    assert!(
        runtime_protocol_response_by_id(&failed_messages, 715)
            .is_some_and(|message| message.get("error").is_some()),
        "changing the sampling interval while profiling should fail: {failed_messages:?}"
    );
    assert_eq!(
        failed_output.v8_state_update(),
        Some(&before_failed_command),
        "a failed stateful command must not advance the opaque V8 state"
    );

    page.close_async()
        .await
        .expect("failed inspector state page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_scopes_same_numeric_runtime_binding_context_id_to_page() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-binding-context-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-binding-context-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first binding context owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate binding-context page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second binding context owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared-isolate binding-context page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );
    output_rx.drain();

    let first_world_context_id =
        create_isolated_world_for_test(&first_page, "shared-binding-context-world")
            .await
            .expect("first binding-context isolated world should be created");
    let second_world_context_id =
        create_isolated_world_for_test(&second_page, "shared-binding-context-world")
            .await
            .expect("second binding-context isolated world should be created");
    assert_eq!(
        first_world_context_id, second_world_context_id,
        "fresh isolates should be allowed to reuse target-scoped binding context ids"
    );

    add_runtime_binding_for_test(
        &second_page,
        "sharedContextBinding",
        None,
        Some(first_world_context_id),
    )
    .await
    .expect("the reused numeric id should install a binding in page B's local realm");

    let (first_binding_type_after_peer_attempt, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression: r#"typeof sharedContextBinding"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first isolated world binding type should evaluate after page B install");
    assert_eq!(
        renderer_json_value(first_binding_type_after_peer_attempt),
        Some(serde_json::json!("undefined")),
        "page B's target-scoped context id must not install into page A's realm"
    );

    let (second_binding_type_after_peer_attempt, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_world_context_id,
            expression: r#"typeof sharedContextBinding"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second isolated world binding type should evaluate after local install");
    assert_eq!(
        renderer_json_value(second_binding_type_after_peer_attempt),
        Some(serde_json::json!("function")),
        "the reused numeric id must resolve to page B's own realm"
    );

    let (second_binding_call_result, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_world_context_id,
            expression: r#"sharedContextBinding("from-second-context-id"); "called""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second isolated-world binding call should evaluate");
    assert_eq!(
        renderer_json_value(second_binding_call_result),
        Some(serde_json::json!("called"))
    );
    let second_binding_calls = output_rx.drain_runtime_binding_calls_for_page(&second_page);
    assert_eq!(second_binding_calls.len(), 1);
    assert_eq!(second_binding_calls[0].name, "sharedContextBinding");
    assert_eq!(second_binding_calls[0].payload, "from-second-context-id");
    assert_eq!(
        second_binding_calls[0].execution_context_id, second_world_context_id,
        "binding call should map back to page B's target-scoped context id"
    );

    add_runtime_binding_for_test(
        &first_page,
        "sharedContextBinding",
        None,
        Some(first_world_context_id),
    )
    .await
    .expect("page A should install binding into its own isolated world by context id");

    let (first_binding_type_after_owner_install, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression: r#"typeof sharedContextBinding"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first isolated world binding type should evaluate after owner install");
    assert_eq!(
        renderer_json_value(first_binding_type_after_owner_install),
        Some(serde_json::json!("function"))
    );

    let (first_binding_call_result, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression: r#"sharedContextBinding("from-first-context-id"); "called""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first isolated world binding call should evaluate");
    assert_eq!(
        renderer_json_value(first_binding_call_result),
        Some(serde_json::json!("called"))
    );

    let first_binding_calls = output_rx.drain_runtime_binding_calls_for_page(&first_page);
    assert_eq!(first_binding_calls.len(), 1);
    assert_eq!(first_binding_calls[0].name, "sharedContextBinding");
    assert_eq!(first_binding_calls[0].payload, "from-first-context-id");
    assert_eq!(
        first_binding_calls[0].execution_context_id, first_world_context_id,
        "binding call should map back to page A's compatibility context id"
    );

    let second_binding_calls_after_first_call =
        output_rx.drain_runtime_binding_calls_for_page(&second_page);
    assert!(
        second_binding_calls_after_first_call.is_empty(),
        "page B must not receive binding calls from page A's context-id binding"
    );

    first_page
        .close_async()
        .await
        .expect("first binding-context page should close");
    second_page
        .close_async()
        .await
        .expect("second binding-context page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_remove_binding_page_local() {
    let runtime = JsRuntime::initialize();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-remove-binding-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-remove-binding-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first remove-binding owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate remove-binding page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second remove-binding owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared-isolate remove-binding page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );
    output_rx.drain();

    let first_world_context_id =
        create_isolated_world_for_test(&first_page, "shared-remove-binding-world")
            .await
            .expect("first remove-binding isolated world should be created");
    let second_world_context_id =
        create_isolated_world_for_test(&second_page, "shared-remove-binding-world")
            .await
            .expect("second remove-binding isolated world should be created");
    add_runtime_binding_for_test(
        &first_page,
        "sharedRemoveBinding",
        Some("shared-remove-binding-world"),
        None,
    )
    .await
    .expect("first page scoped runtime binding should install");
    add_runtime_binding_for_test(
        &second_page,
        "sharedRemoveBinding",
        Some("shared-remove-binding-world"),
        None,
    )
    .await
    .expect("second page scoped runtime binding should install");

    remove_runtime_binding_for_test(&first_page, "sharedRemoveBinding")
        .await
        .expect("first page scoped runtime binding should be removed");

    let (first_binding_type_after_remove, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression: r#"typeof sharedRemoveBinding"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first isolated world binding type should evaluate after remove");
    assert_eq!(
        renderer_json_value(first_binding_type_after_remove),
        Some(serde_json::json!("undefined"))
    );

    let (second_binding_type_after_peer_remove, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_world_context_id,
            expression: r#"typeof sharedRemoveBinding"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second isolated world binding type should evaluate after peer remove");
    assert_eq!(
        renderer_json_value(second_binding_type_after_peer_remove),
        Some(serde_json::json!("function")),
        "Runtime.removeBinding on page A must not remove page B's same-name binding in a shared document isolate"
    );

    let (second_binding_call_result, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_world_context_id,
            expression: r#"sharedRemoveBinding("from-second-world"); "called""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second isolated world binding call should evaluate after peer remove");
    assert_eq!(
        renderer_json_value(second_binding_call_result),
        Some(serde_json::json!("called"))
    );
    let second_binding_calls = output_rx.drain_runtime_binding_calls_for_page(&second_page);
    assert_eq!(second_binding_calls.len(), 1);
    assert_eq!(second_binding_calls[0].name, "sharedRemoveBinding");
    assert_eq!(second_binding_calls[0].payload, "from-second-world");
    assert_eq!(
        second_binding_calls[0].execution_context_id, second_world_context_id,
        "binding call should map back to page B's compatibility context id after page A removal"
    );

    let first_binding_calls = output_rx.drain_runtime_binding_calls_for_page(&first_page);
    assert!(
        first_binding_calls.is_empty(),
        "page A must not receive binding calls after removing its page-local binding"
    );

    first_page
        .close_async()
        .await
        .expect("first remove-binding page should close");
    second_page
        .close_async()
        .await
        .expect("second remove-binding page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_scopes_document_start_scripts_to_page_worlds() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-preload-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-preload-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first document-start owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate document-start page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second document-start owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared-isolate document-start page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );

    let script = crate::DocumentStartScript {
        registry_key: None,
        devtools_session: None,
        source: r#"globalThis.__sharedPreloadOwner = "first-page";"#.to_owned(),
        world_name: Some("shared-preload-world".to_owned()),
        has_bidi_channel_argument: false,
        bidi_channel_handoffs: Vec::new(),
    };
    let first_preload_result = first_page
        .run_async_command(RendererPageCommand::AddDocumentStartScriptRuntimeActivity {
            inspector_session_id: None,
            script: script.clone(),
            run_immediately: true,
        })
        .await
        .expect("first page document-start script should run")
        .0;
    let RendererPageReply::DocumentStartScriptResult(Some((first_world_context_id, first_created))) =
        first_preload_result
    else {
        panic!("expected first document-start script to create an isolated world");
    };
    assert!(first_created);

    let second_world_context_id =
        create_isolated_world_for_test(&second_page, "shared-preload-world")
            .await
            .expect("second document-start isolated world should be created");
    let (first_preload_value, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression: r#"globalThis.__sharedPreloadOwner"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first isolated world preload value should evaluate");
    assert_eq!(
        renderer_json_value(first_preload_value),
        Some(serde_json::json!("first-page"))
    );

    let (second_preload_value, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_world_context_id,
            expression: r#"globalThis.__sharedPreloadOwner ?? "absent""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second isolated world preload value should evaluate");
    assert_eq!(
        renderer_json_value(second_preload_value),
        Some(serde_json::json!("absent")),
        "Page.addScriptToEvaluateOnNewDocument(worldName=...) state must not leak into another page's same-name isolated world on a shared document isolate"
    );

    first_page
        .close_async()
        .await
        .expect("first document-start page should close");
    second_page
        .close_async()
        .await
        .expect("second document-start page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_page_isolate_policy_keeps_stored_document_start_scripts_page_local() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let first_url = url::Url::parse("https://example.test/shared-stored-preload-a").unwrap();
    let second_url = url::Url::parse("https://example.test/shared-stored-preload-b").unwrap();

    let (mut first_page, _, _, _creation_artifacts, first_download) = runtime
        .create_html_page_from_response(
            first_url.clone(),
            first_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>first stored preload owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("first shared-isolate stored-preload page should load");
    assert!(first_download.is_none());

    let (mut second_page, _, _, _creation_artifacts, second_download) = runtime
        .create_html_page_from_response(
            second_url.clone(),
            second_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>second stored preload owner</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("second shared-isolate stored-preload page should load");
    assert!(second_download.is_none());

    let first_testing = RendererPageTestingHandle::new_for_testing(&first_page);
    let second_testing = RendererPageTestingHandle::new_for_testing(&second_page);
    assert!(first_testing.shares_local_host(&second_testing));
    assert_eq!(
        second_testing
            .host_unique_document_isolate_count_async()
            .await
            .expect("two shared attached unique document isolate count"),
        2
    );

    let stored_script = crate::DocumentStartScript {
        registry_key: None,
        devtools_session: None,
        source: r#"globalThis.__sharedStoredPreloadOwner = "first-page";"#.to_owned(),
        world_name: Some("shared-stored-preload-world".to_owned()),
        has_bidi_channel_argument: false,
        bidi_channel_handoffs: Vec::new(),
    };
    set_stored_document_start_scripts_for_test(&first_page, vec![stored_script])
        .await
        .expect("first page stored document-start script should install");

    let first_replacement_url = "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Cbody%3Efirst%20stored%20replacement%3C/body%3E";
    let (first_navigation_reply, _) = first_page
        .run_async_command(
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression: format!(
                    r#"(() => {{
  location.href = {first_replacement_url:?};
  return "navigating-first";
}})()"#
                ),
                await_promise: false,
            },
        )
        .await
        .expect("first stored-preload page should navigate");
    assert_eq!(
        renderer_json_value(first_navigation_reply),
        Some(serde_json::json!("navigating-first"))
    );

    let first_world_context_id =
        create_isolated_world_for_test(&first_page, "shared-stored-preload-world")
            .await
            .expect("first stored-preload isolated world should be available");
    let (first_stored_preload_value, _) = first_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: first_world_context_id,
            expression: r#"globalThis.__sharedStoredPreloadOwner ?? "absent""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("first stored-preload isolated world should evaluate");
    assert_eq!(
        renderer_json_value(first_stored_preload_value),
        Some(serde_json::json!("first-page"))
    );

    let second_replacement_url = "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Cbody%3Esecond%20stored%20replacement%3C/body%3E";
    let (second_navigation_reply, _) = second_page
        .run_async_command(
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression: format!(
                    r#"(() => {{
  location.href = {second_replacement_url:?};
  return "navigating-second";
}})()"#
                ),
                await_promise: false,
            },
        )
        .await
        .expect("second stored-preload page should navigate");
    assert_eq!(
        renderer_json_value(second_navigation_reply),
        Some(serde_json::json!("navigating-second"))
    );

    let second_world_context_id =
        create_isolated_world_for_test(&second_page, "shared-stored-preload-world")
            .await
            .expect("second stored-preload isolated world should be created");
    let (second_stored_preload_value, _) = second_page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: second_world_context_id,
            expression: r#"globalThis.__sharedStoredPreloadOwner ?? "absent""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("second stored-preload isolated world should evaluate");
    assert_eq!(
        renderer_json_value(second_stored_preload_value),
        Some(serde_json::json!("absent")),
        "stored Page.addScriptToEvaluateOnNewDocument(worldName=...) state must not leak into another page's later navigation on a shared document isolate"
    );

    first_page
        .close_async()
        .await
        .expect("first stored-preload page should close");
    second_page
        .close_async()
        .await
        .expect("second stored-preload page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn stored_document_start_script_remove_uses_registry_key_namespace() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let url = url::Url::parse("https://example.test/stored-preload-registry-key").unwrap();

    let (mut page, _, _, _creation_artifacts, download) = runtime
        .create_html_page_from_response(
            url.clone(),
            url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>stored preload key namespace</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("registry-key stored-preload page should load");
    assert!(download.is_none());

    set_stored_document_start_scripts_for_test(
        &page,
        vec![
            crate::DocumentStartScript {
                registry_key: Some("default:1".to_owned()),
                devtools_session: None,
                source: r#"globalThis.__defaultPreload = "default";"#.to_owned(),
                world_name: Some("registry-key-world".to_owned()),
                has_bidi_channel_argument: false,
                bidi_channel_handoffs: Vec::new(),
            },
            crate::DocumentStartScript {
                registry_key: Some("target:TID-1:1".to_owned()),
                devtools_session: None,
                source: r#"globalThis.__targetPreload = "target";"#.to_owned(),
                world_name: Some("registry-key-world".to_owned()),
                has_bidi_channel_argument: false,
                bidi_channel_handoffs: Vec::new(),
            },
        ],
    )
    .await
    .expect("stored document-start scripts should install");

    let (remove_reply, _) = page
        .run_async_command(RendererPageCommand::RemoveDocumentStartScriptByRegistryKey(
            "target:TID-1:1".to_owned(),
        ))
        .await
        .expect("registry-key remove should run");
    assert!(matches!(remove_reply, RendererPageReply::Unit));

    let replacement_url = "data:text/html;charset=utf-8,%3C!doctype%20html%3E%3Cbody%3Eregistry%20key%20replacement%3C/body%3E";
    let (navigation_reply, _) = page
        .run_async_command(
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression: format!(
                    r#"(() => {{
  location.href = {replacement_url:?};
  return "navigating";
}})()"#
                ),
                await_promise: false,
            },
        )
        .await
        .expect("registry-key stored-preload page should navigate");
    assert_eq!(
        renderer_json_value(navigation_reply),
        Some(serde_json::json!("navigating"))
    );

    let context_id = create_isolated_world_for_test(&page, "registry-key-world")
        .await
        .expect("registry-key isolated world should be created");
    let (value, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: context_id,
            expression: r#"JSON.stringify({
                defaultValue: globalThis.__defaultPreload,
                targetValue: globalThis.__targetPreload ?? "absent"
            })"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("registry-key isolated world value should evaluate");
    assert_eq!(
        renderer_json_value(value),
        Some(serde_json::json!(
            r#"{"defaultValue":"default","targetValue":"absent"}"#
        ))
    );

    page.close_async()
        .await
        .expect("registry-key stored-preload page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_loop_applies_subresource_fetch_completion_without_wait_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, server) = spawn_owner_service_worker_response_sequence(vec![
        ("/api", "text/plain; charset=utf-8", "owner-wake-body"),
        ("/effect", "text/plain; charset=utf-8", "ok"),
    ])
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let (page, _, _creation_diagnostics, _creation_artifacts, pending_download) = runtime
        .create_html_page_from_response(
            page_url.clone(),
            page_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>owner wake</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("page should load");
    assert!(pending_download.is_none());

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_owner_wake_fetch_marker = "pending";
  fetch("/api")
    .then(response => response.text())
    .then(text => {
      globalThis.__lm_owner_wake_fetch_marker = text;
      return fetch("/effect");
    });
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("fetch scheduling evaluate should run");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("the fetch reaction must issue its effect request without another Page command")
        .expect("the subresource response server should finish");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_owner_wake_fetch_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("completed fetch marker should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("owner-wake-body"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_applies_main_parser_module_terminal_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerMainModuleLivenessServer {
        base_url,
        module_request_seen,
        release_module_response,
        effect_request_seen,
        task: server,
    } = spawn_owner_main_module_liveness_server(
        "/owner-main-parser-module.js",
        r#"globalThis.__lm_owner_main_parser_module = "executed";
fetch("/owner-main-parser-module-effect");"#,
        "/owner-main-parser-module-effect",
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>initial main parser module page</body>",
    )
    .await;

    let replacement_html = format!(
        r#"<!doctype html><body>
<script type="module" src="{base_url}/owner-main-parser-module.js"></script>
</body>"#
    );
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                "document.open(); document.write({replacement_html:?}); document.close(); 'scheduled'"
            ),
            await_promise: false,
        })
        .await
        .expect("document.write parser module should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );
    tokio::time::timeout(Duration::from_secs(2), module_request_seen)
        .await
        .expect("main parser module request should start without another command")
        .expect("main parser module request signal should remain open");
    release_module_response
        .send(())
        .expect("main parser module response should be released once");
    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect(
            "the producer wake and owner continuations should evaluate the parser module without another command",
        )
        .expect("main parser module effect signal should remain open");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lm_owner_main_parser_module".to_owned(),
            await_promise: false,
        })
        .await
        .expect("completed main parser module marker should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("executed"))
    );

    page.close_async()
        .await
        .expect("main parser module owner-liveness page should close");
    server
        .await
        .expect("main parser module owner-liveness server should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_applies_main_parser_module_reaction_and_followup_without_observation_command()
 {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerMainModuleReactionLivenessServer {
        base_url,
        module_request_seen,
        release_module_response,
        evaluation_started,
        effect_request_seen,
        script_load_event_seen,
        task: server,
    } = spawn_owner_main_module_reaction_liveness_server().await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>main parser module reaction owner liveness</body>",
    )
    .await;

    let replacement_html = format!(
        r#"<!doctype html><body>
<script type="module" src="{base_url}/owner-main-tla-module.js" onload="fetch('/owner-main-tla-script-load')"></script>
</body>"#
    );
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                "document.open(); document.write({replacement_html:?}); document.close(); 'scheduled'"
            ),
            await_promise: false,
        })
        .await
        .expect("main parser TLA module should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );
    tokio::time::timeout(Duration::from_secs(2), module_request_seen)
        .await
        .expect("main parser TLA module request should start")
        .expect("main parser TLA module request signal should remain open");
    release_module_response
        .send(())
        .expect("main parser TLA module response should release once");
    tokio::time::timeout(Duration::from_secs(2), evaluation_started)
        .await
        .expect("owner scheduler should start the TLA evaluation")
        .expect("TLA evaluation-start signal should remain open");

    let (resolved, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "__resolveLmOwnerMainTla(); 'resolved'".to_owned(),
            await_promise: false,
        })
        .await
        .expect("TLA gate should resolve");
    assert_eq!(
        renderer_json_value(resolved),
        Some(serde_json::json!("resolved"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("resolving the TLA gate should resume module evaluation without another command")
        .expect("main parser TLA effect signal should remain open");
    tokio::time::timeout(Duration::from_secs(2), script_load_event_seen)
        .await
        .expect(
            "typed module reaction and its parser-owned follow-up should dispatch the script load event without another command",
        )
        .expect("main parser TLA script-load signal should remain open");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerMainTlaState".to_owned(),
            await_promise: false,
        })
        .await
        .expect("completed main parser TLA marker should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("completed"))
    );

    page.close_async()
        .await
        .expect("main parser module-reaction liveness page should close");
    server
        .await
        .expect("main parser module-reaction liveness server should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_completes_runtime_module_graph_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerMainModuleLivenessServer {
        base_url,
        module_request_seen,
        release_module_response,
        effect_request_seen,
        task: server,
    } = spawn_owner_main_module_liveness_server(
        "/owner-main-runtime-dependency.js",
        "export const dependency = true;",
        "/owner-main-runtime-module-effect",
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>runtime module owner liveness</body>",
    )
    .await;

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                r#"(() => {{
  globalThis.__lm_owner_main_runtime_module = "pending";
  const script = document.createElement("script");
  script.type = "module";
  script.textContent = `
    import "{base_url}/owner-main-runtime-dependency.js";
    globalThis.__lm_owner_main_runtime_module = "executed";
    fetch("{base_url}/owner-main-runtime-module-effect");
  `;
  document.body.appendChild(script);
  return "scheduled";
}})()"#,
            ),
            await_promise: false,
        })
        .await
        .expect("runtime module should install");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );
    tokio::time::timeout(Duration::from_secs(2), module_request_seen)
        .await
        .expect("runtime module dependency request should start without another command")
        .expect("runtime module dependency request signal should remain open");
    release_module_response
        .send(())
        .expect("runtime module dependency response should be released once");
    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect(
            "producer wake and owner continuations should evaluate the runtime module without another command",
        )
        .expect("runtime module effect signal should remain open");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lm_owner_main_runtime_module".to_owned(),
            await_promise: false,
        })
        .await
        .expect("completed runtime module marker should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("executed"))
    );

    page.close_async()
        .await
        .expect("runtime module owner-liveness page should close");
    server
        .await
        .expect("runtime module owner-liveness server should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_applies_child_module_reaction_and_followup_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerInlineModuleReactionLivenessServer {
        base_url,
        evaluation_started,
        effect_request_seen,
        task: server,
    } = spawn_owner_inline_module_reaction_liveness_server(
        "/owner-child-tla-evaluation-started",
        "/owner-child-tla-effect",
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>child module reaction owner liveness</body>",
    )
    .await;
    let child_html = r#"<!doctype html><body><script type="module">
const ownerChildTlaGate = new Promise(resolve => {
  parent.__resolveLmOwnerChildTla = resolve;
});
fetch("/owner-child-tla-evaluation-started");
await ownerChildTlaGate;
parent.__lmOwnerChildTlaState = "completed";
fetch("/owner-child-tla-effect");
</script></body>"#;

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                r#"(() => {{
  globalThis.__lmOwnerChildTlaState = "pending";
  const frame = document.createElement("iframe");
  frame.srcdoc = {child_html:?};
  document.body.appendChild(frame);
  return "scheduled";
}})()"#
            ),
            await_promise: false,
        })
        .await
        .expect("child TLA module should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );
    tokio::time::timeout(Duration::from_secs(2), evaluation_started)
        .await
        .expect("owner scheduler should start the child TLA evaluation")
        .expect("child TLA evaluation-start signal should remain open");
    page.run_async_command(RendererPageCommand::EvaluateExpression {
        expression: "__resolveLmOwnerChildTla(); 'resolved'".to_owned(),
        await_promise: false,
    })
    .await
    .expect("child TLA gate should resolve");

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect(
            "typed child module reaction and DocumentScriptReady follow-up should run without another command",
        )
        .expect("child TLA effect signal should remain open");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerChildTlaState".to_owned(),
            await_promise: false,
        })
        .await
        .expect("completed child TLA marker should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("completed"))
    );
    page.close_async()
        .await
        .expect("child module-reaction liveness page should close");
    server
        .await
        .expect("child module-reaction liveness server should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_applies_main_modulepreload_terminal_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerModulepreloadLivenessServer {
        base_url,
        module_request_seen,
        release_module_response,
        effect_request_seen,
        task: server,
    } = spawn_owner_modulepreload_liveness_server(
        "/owner-main-modulepreload.js",
        "export const ownerMainModulepreload = true;",
        "/owner-main-modulepreload-load-event",
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>main modulepreload owner liveness</body>",
    )
    .await;

    let replacement_html = format!(
        r#"<!doctype html><head>
<link rel="modulepreload"
      href="{base_url}/owner-main-modulepreload.js"
      onload="globalThis.__lm_owner_main_modulepreload = 'loaded'; fetch('{base_url}/owner-main-modulepreload-load-event')"
      onerror="globalThis.__lm_owner_main_modulepreload = 'failed'">
</head><body>replacement</body>"#
    );
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                "document.open(); document.write({replacement_html:?}); document.close(); 'scheduled'"
            ),
            await_promise: false,
        })
        .await
        .expect("document.write modulepreload should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );
    tokio::time::timeout(Duration::from_secs(2), module_request_seen)
        .await
        .expect("main modulepreload request should start without another command")
        .expect("main modulepreload request signal should remain open");
    release_module_response
        .send(())
        .expect("main modulepreload response should be released once");
    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect(
            "typed terminal and link-event continuations should finish without an observation command",
        )
        .expect("main modulepreload effect signal should remain open");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lm_owner_main_modulepreload".to_owned(),
            await_promise: false,
        })
        .await
        .expect("main modulepreload load marker should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("loaded"))
    );

    page.close_async()
        .await
        .expect("main modulepreload owner-liveness page should close");
    server
        .await
        .expect("main modulepreload owner-liveness server should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_runs_joined_main_modulepreload_graph_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerModulepreloadLivenessServer {
        base_url,
        module_request_seen,
        release_module_response,
        effect_request_seen,
        task: server,
    } = spawn_owner_modulepreload_liveness_server(
        "/owner-joined-main-module.js",
        r#"globalThis.__lmJoinedMainModulepreloadEvents.push("module");
globalThis.__lmMaybeFinishJoinedMainModulepreload();"#,
        "/owner-joined-main-module-executed",
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>joined main modulepreload owner liveness</body>",
    )
    .await;

    let replacement_html = format!(
        r#"<!doctype html><head>
<script>
globalThis.__lmJoinedMainModulepreloadEvents = [];
globalThis.__lmJoinedMainModulepreloadDone = false;
globalThis.__lmMaybeFinishJoinedMainModulepreload = () => {{
  const events = globalThis.__lmJoinedMainModulepreloadEvents;
  if (!globalThis.__lmJoinedMainModulepreloadDone &&
      events.includes("preload-load") && events.includes("module")) {{
    globalThis.__lmJoinedMainModulepreloadDone = true;
    fetch("/owner-joined-main-module-executed");
  }}
}};
</script>
<link rel="modulepreload"
      href="{base_url}/owner-joined-main-module.js"
      onload="globalThis.__lmJoinedMainModulepreloadEvents.push('preload-load'); globalThis.__lmMaybeFinishJoinedMainModulepreload()">
<script type="module" src="{base_url}/owner-joined-main-module.js"></script>
</head><body>replacement</body>"#
    );
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                "document.open(); document.write({replacement_html:?}); document.close(); 'scheduled'"
            ),
            await_promise: false,
        })
        .await
        .expect("joined main modulepreload fixture should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );
    tokio::time::timeout(Duration::from_secs(2), module_request_seen)
        .await
        .expect("main modulepreload should own the joined root fetch")
        .expect("joined main module request signal should remain open");
    release_module_response
        .send(())
        .expect("joined main module response should be released once");
    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("joined main module graph should finish through owner continuations")
        .expect("joined main module effect signal should remain open");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmJoinedMainModulepreloadEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("joined main modulepreload ordering should remain observable");
    assert!(
        matches!(
            renderer_json_value(events),
            Some(value)
                if value == serde_json::json!("preload-load|module")
                    || value == serde_json::json!("module|preload-load")
        ),
        "module-map terminal must autonomously resume both clients; Chromium does not guarantee their relative order"
    );

    page.close_async()
        .await
        .expect("joined main modulepreload owner-liveness page should close");
    server
        .await
        .expect("joined main modulepreload owner-liveness server should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_applies_production_child_module_terminal() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerChildModuleGraphServer {
        base_url,
        root_request_seen,
        release_root_response,
        dependency_request_seen,
        release_dependency_response,
        effect_request_seen,
        task: server,
    } = spawn_owner_child_module_graph_server().await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>child module owner turn</body>",
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_owner_child_module_events = [];
  const frame = document.createElement("iframe");
  frame.srcdoc = `<script type="module" src="/child-owner-module.js"><\/script>`;
  document.body.appendChild(frame);
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("child parser module should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );
    tokio::time::timeout(Duration::from_secs(2), root_request_seen)
        .await
        .expect("child module root request should reach the test server")
        .expect("child module root request signal should remain open");
    release_root_response
        .send(())
        .expect("child module root response should be released once");

    tokio::time::timeout(Duration::from_secs(2), dependency_request_seen)
        .await
        .expect("authorized root application should start its static dependency request")
        .expect("child module dependency request signal should remain open");
    release_dependency_response
        .send(())
        .expect("child module dependency response should be released once");

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("child module evaluation must run without an observation command")
        .expect("child module effect request signal should remain open");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lm_owner_child_module_events.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("completed child module effects should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("dependency|root")),
        "the observation command must only read work already completed by owner turns"
    );

    page.close_async()
        .await
        .expect("child module owner-turn page should close");
    server.await.expect("child module server should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_dispatches_production_dedicated_worker_message_without_observation_command()
 {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-dedicated-worker-delivered",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>DedicatedWorker owner turn</body>",
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lmOwnerDedicatedWorkerEvents = [];
  const source = `postMessage("go")`;
  globalThis.__lmOwnerDedicatedWorker = new Worker(
    "data:text/javascript," + encodeURIComponent(source)
  );
  globalThis.__lmOwnerDedicatedWorker.onmessage = event => {
    globalThis.__lmOwnerDedicatedWorkerEvents.push("message:" + event.data);
    fetch("/owner-dedicated-worker-delivered");
  };
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("DedicatedWorker delivery should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch the typed DedicatedWorker task")
        .expect("DedicatedWorker handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("DedicatedWorker effect response should release once");
    effect_server
        .await
        .expect("DedicatedWorker effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerDedicatedWorkerEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("DedicatedWorker handler result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("message:go"))
    );

    page.close_async()
        .await
        .expect("DedicatedWorker owner-liveness page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_dispatches_production_shared_worker_error_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-shared-worker-error-delivered",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>SharedWorker owner turn</body>",
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lmOwnerSharedWorkerEvents = [];
  const brokenSource = "function( { broken syntax";
  globalThis.__lmOwnerSharedWorker = new SharedWorker(
    "data:text/javascript," + encodeURIComponent(brokenSource),
    "owner-shared-worker-error"
  );
  globalThis.__lmOwnerSharedWorker.onerror = event => {
    globalThis.__lmOwnerSharedWorkerEvents.push("error:" + event.type);
    fetch("/owner-shared-worker-error-delivered");
  };
  globalThis.__lmOwnerSharedWorker.port.start();
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("SharedWorker error should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch the typed SharedWorker client event")
        .expect("SharedWorker error-handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("SharedWorker effect response should release once");
    effect_server
        .await
        .expect("SharedWorker effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerSharedWorkerEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("SharedWorker handler result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("error:error"))
    );

    page.close_async()
        .await
        .expect("SharedWorker owner-liveness page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_settles_production_webcrypto_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-webcrypto-settled",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>WebCrypto owner turn</body>",
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lmOwnerWebCryptoResult = "pending";
  crypto.subtle.digest("SHA-256", new TextEncoder().encode("owner-webcrypto"))
    .then(bytes => {
      globalThis.__lmOwnerWebCryptoResult = String(bytes.byteLength);
      fetch("/owner-webcrypto-settled");
    }, error => {
      globalThis.__lmOwnerWebCryptoResult = `${error.name}:${error.message}`;
    });
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("WebCrypto digest should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should settle the typed WebCrypto task")
        .expect("WebCrypto Promise reaction effect signal should remain open");

    release_effect_response
        .send(())
        .expect("WebCrypto effect response should release once");
    effect_server
        .await
        .expect("WebCrypto effect server should finish");

    let (result, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerWebCryptoResult".to_owned(),
            await_promise: false,
        })
        .await
        .expect("WebCrypto Promise result should remain observable");
    assert_eq!(renderer_json_value(result), Some(serde_json::json!("32")));

    page.close_async()
        .await
        .expect("WebCrypto owner-liveness page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn webcrypto_checkpoint_reconciles_document_replacement_before_restoring_page_residence() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/webcrypto-checkpoint-document-open")
        .expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>initial WebCrypto document</body>",
    )
    .await;
    let document_open_session = "SID-document-open-agent";
    runtime_enable_events_for_inspector_session_for_test(&page, Some(document_open_session))
        .await
        .expect("auxiliary Runtime session should attach before document.open");
    let inspector_before = runtime_heap_usage_for_test(&page).await;
    let inspector_before = &inspector_before["moli"]["runtime"];
    let context_group_before = inspector_before["inspectorContextGroupId"].clone();
    let session_count_before = inspector_before["inspectorSessionCount"].clone();
    let registration_count_before = inspector_before["inspectorContextRegistrationCount"].clone();
    assert_eq!(session_count_before, serde_json::json!(2));
    assert_eq!(registration_count_before, serde_json::json!(1));

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  crypto.subtle.digest("SHA-256", new TextEncoder().encode("replace-document"))
    .then(() => {
      document.open();
      document.write('<main id="webcrypto-checkpoint-replacement">replacement</main>');
      document.close();
    });
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("WebCrypto replacement reaction should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("the WebCrypto task-end checkpoint should install and schedule replacement lifecycle");
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                "document.querySelector('#webcrypto-checkpoint-replacement')?.textContent ?? 'missing'"
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement DOM should evaluate");
    assert_eq!(
        renderer_json_value(replacement),
        Some(serde_json::json!("replacement"))
    );
    let inspector_after = runtime_heap_usage_for_test(&page).await;
    let inspector_after = &inspector_after["moli"]["runtime"];
    assert_eq!(
        inspector_after["inspectorContextGroupId"], context_group_before,
        "same-Page document.open must preserve the local-root context group"
    );
    assert_eq!(
        inspector_after["inspectorSessionCount"], session_count_before,
        "same-Page document.open must not detach frontend V8 sessions"
    );
    assert_eq!(
        inspector_after["inspectorContextRegistrationCount"], registration_count_before,
        "same-Page document.open must preserve the existing Window context registration"
    );
    assert_eq!(
        inspector_after["inspectorSessionRegistryOwner"],
        serde_json::json!("renderer-devtools-agent")
    );
    runtime_enable_events_for_inspector_session_for_test(&page, Some(document_open_session))
        .await
        .expect("the existing auxiliary Runtime session should remain dispatchable");

    page.close_async()
        .await
        .expect("WebCrypto checkpoint replacement page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_dispatches_production_broadcast_channel_delivery_without_observation_command()
 {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-broadcast-channel-delivered",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>BroadcastChannel owner turn</body>",
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lmOwnerBroadcastChannelEvents = [];
  globalThis.__lmOwnerBroadcastChannelReceiver = new BroadcastChannel("owner-delivery");
  globalThis.__lmOwnerBroadcastChannelReceiver.onmessage = event => {
    globalThis.__lmOwnerBroadcastChannelEvents.push("message:" + event.data);
    fetch("/owner-broadcast-channel-delivered");
  };
  globalThis.__lmOwnerBroadcastChannelSender = new BroadcastChannel("owner-delivery");
  globalThis.__lmOwnerBroadcastChannelSender.postMessage("go");
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("BroadcastChannel delivery should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch the typed BroadcastChannel task")
        .expect("BroadcastChannel handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("BroadcastChannel effect response should release once");
    effect_server
        .await
        .expect("BroadcastChannel effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerBroadcastChannelEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("BroadcastChannel handler result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("message:go"))
    );

    page.close_async()
        .await
        .expect("BroadcastChannel owner-liveness page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn broadcast_channel_checkpoint_reconciles_replacement_before_page_restore() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/broadcast-checkpoint-document-open")
        .expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>initial BroadcastChannel document</body>",
    )
    .await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__broadcastCheckpointReceiver =
    new BroadcastChannel("broadcast-checkpoint-replacement");
  __broadcastCheckpointReceiver.onmessage = () => {
    Promise.resolve().then(() => {
      document.open();
      document.write('<main id="broadcast-checkpoint-replacement">replacement</main>');
      document.close();
    });
  };
  globalThis.__broadcastCheckpointSender =
    new BroadcastChannel("broadcast-checkpoint-replacement");
  __broadcastCheckpointSender.postMessage("replace");
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("BroadcastChannel replacement reaction should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("the BroadcastChannel task-end checkpoint should install replacement lifecycle");
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                "document.querySelector('#broadcast-checkpoint-replacement')?.textContent ?? 'missing'"
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement DOM should evaluate");
    assert_eq!(
        renderer_json_value(replacement),
        Some(serde_json::json!("replacement"))
    );

    page.close_async()
        .await
        .expect("BroadcastChannel checkpoint replacement page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_dispatches_storage_event_without_timer_or_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-storage-event-delivered",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><body>
<iframe id="recipient"></iframe>
<script>
globalThis.__lmOwnerStorageEvents = [];
recipient.contentWindow.addEventListener("storage", event => {
  parent.__lmOwnerStorageEvents.push(event.key + ":" + event.newValue);
  fetch("/owner-storage-event-delivered");
});
</script>
</body>"#,
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"localStorage.setItem("owner-storage-key", "go"); "scheduled""#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("StorageEvent delivery should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch StorageEvent without another command")
        .expect("StorageEvent handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("StorageEvent effect response should release once");
    effect_server
        .await
        .expect("StorageEvent effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerStorageEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("StorageEvent handler result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("owner-storage-key:go"))
    );

    page.close_async()
        .await
        .expect("StorageEvent owner-liveness page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_event_checkpoint_reconciles_replacement_before_page_restore() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/storage-event-checkpoint-document-open")
        .expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body><iframe id=\"storage-source\"></iframe></body>",
    )
    .await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  addEventListener("storage", () => {
    Promise.resolve().then(() => {
      document.open();
      document.write('<main id="storage-event-checkpoint-replacement">replacement</main>');
      document.close();
    });
  }, { once: true });
  document.getElementById("storage-source").contentWindow.localStorage
    .setItem("storage-event-checkpoint-key", "replace-document");
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("StorageEvent replacement reaction should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect(
        "the StorageEvent task-end checkpoint should install and schedule replacement lifecycle",
    );
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                "document.querySelector('#storage-event-checkpoint-replacement')?.textContent ?? 'missing'"
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement DOM should evaluate");
    assert_eq!(
        renderer_json_value(replacement),
        Some(serde_json::json!("replacement"))
    );

    page.close_async()
        .await
        .expect("StorageEvent checkpoint replacement page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_dispatches_hashchange_without_timer_driving_or_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-hashchange-delivered",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><body>
<script>
globalThis.__lmOwnerHashChanges = [];
addEventListener("hashchange", event => {
  __lmOwnerHashChanges.push(event.oldURL + "->" + event.newURL);
  fetch("/owner-hashchange-delivered");
});
</script>
</body>"#,
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r##"location.hash = "#typed"; "scheduled""##.to_owned(),
            await_promise: false,
        })
        .await
        .expect("fragment navigation should schedule hashchange");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch hashchange without another command")
        .expect("hashchange handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("hashchange effect response should release once");
    effect_server
        .await
        .expect("hashchange effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerHashChanges.length.toString()".to_owned(),
            await_promise: false,
        })
        .await
        .expect("hashchange handler result should remain observable");
    assert_eq!(renderer_json_value(events), Some(serde_json::json!("1")));

    page.close_async()
        .await
        .expect("hashchange owner-liveness page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn hashchange_checkpoint_reconciles_replacement_before_page_restore() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/hashchange-checkpoint-document-open")
        .expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>initial hashchange document</body>",
    )
    .await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r##"(() => {
  addEventListener("hashchange", () => {
    Promise.resolve().then(() => {
      document.open();
      document.write('<main id="hashchange-checkpoint-replacement">replacement</main>');
      document.close();
    });
  }, { once: true });
  location.hash = "#replace";
  return "scheduled";
})()"##
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("hashchange replacement reaction should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("the hashchange task-end checkpoint should install replacement lifecycle");
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                "document.querySelector('#hashchange-checkpoint-replacement')?.textContent ?? 'missing'"
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement DOM should evaluate");
    assert_eq!(
        renderer_json_value(replacement),
        Some(serde_json::json!("replacement"))
    );

    page.close_async()
        .await
        .expect("hashchange checkpoint replacement page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_runs_history_traversal_without_timer_or_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-history-traversal-applied",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r##"<!doctype html><body>
<script>
history.pushState(null, "", "#entry");
globalThis.__lmOwnerHistoryTraversalLog = [];
addEventListener("popstate", () => {
  __lmOwnerHistoryTraversalLog.push("popstate:" + location.hash);
  fetch("/owner-history-traversal-applied");
}, { once: true });
</script>
</body>"##,
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"history.back(); "scheduled""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("history traversal should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should run history traversal without another command")
        .expect("history traversal handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("history traversal effect response should release once");
    effect_server
        .await
        .expect("history traversal effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerHistoryTraversalLog.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("history traversal handler result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("popstate:"))
    );

    page.close_async()
        .await
        .expect("history traversal owner-liveness page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_runs_document_scroll_rendering_update_without_timer_or_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-scroll-rendering-update-applied",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><body>
<script>
globalThis.__lmOwnerScrollLog = [];
document.addEventListener("scroll", () => {
  __lmOwnerScrollLog.push("scroll:" + scrollY);
  fetch("/owner-scroll-rendering-update-applied");
}, { once: true });
document.addEventListener("scrollend", () => {
  __lmOwnerScrollLog.push("scrollend:" + scrollY);
}, { once: true });
</script>
</body>"#,
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"scrollTo(0, 25); "scheduled""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("Window scroll should enter the rendering source");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch scroll without another command")
        .expect("scroll handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("scroll effect response should release once");
    effect_server
        .await
        .expect("scroll effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerScrollLog.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("scroll handler result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("scroll:25|scrollend:25"))
    );

    page.close_async()
        .await
        .expect("rendering-update owner-liveness page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_applies_a_wheel_batch_at_the_fixed_action_window_deadline() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/action-window-intersection-applied",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html>
<style>
html, body { margin: 0; }
body { height: 1200px; }
#target { position: absolute; top: 250px; width: 20px; height: 20px; }
</style>
<div id="target"></div>
<script>
globalThis.__lmActionWindowWheelLog = [];
globalThis.__lmActionWindowIoLog = [];
addEventListener("wheel", event => {
  __lmActionWindowWheelLog.push("event:" + event.deltaY);
  Promise.resolve().then(() => {
    __lmActionWindowWheelLog.push("microtask:" + event.deltaY);
  });
}, { capture: true });
</script>"#,
    )
    .await;
    page.run_async_command(RendererPageCommand::SetViewportSurface(Some(
        crate::protocol_types::ViewportSurface {
            inner_width: 200,
            inner_height: 200,
            outer_width: 200,
            outer_height: 200,
            device_pixel_ratio: 1.0,
            screen_width: 200,
            screen_height: 200,
            screen_avail_width: 200,
            screen_avail_height: 200,
        },
    )))
    .await
    .expect("action-window viewport should update");
    let (observer_installed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
globalThis.__lmActionWindowObserver = new IntersectionObserver(entries => {
  const entry = entries.find(candidate => candidate.target.id === "target");
  if (!entry) return;
  __lmActionWindowIoLog.push(entry.isIntersecting);
  if (__lmActionWindowIoLog.length === 2) {
    fetch("/action-window-intersection-applied");
  }
});
__lmActionWindowObserver.observe(document.getElementById("target"));
"installed"
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("IntersectionObserver should install");
    assert_eq!(
        renderer_json_value(observer_installed),
        Some(serde_json::json!("installed"))
    );
    let (initial_intersection, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "JSON.stringify(__lmActionWindowIoLog)".to_owned(),
            await_promise: false,
        })
        .await
        .expect("initial intersection state should be observable");
    assert_eq!(
        renderer_json_value(initial_intersection),
        Some(serde_json::json!("[false]"))
    );

    let opened_at = std::time::Instant::now();
    for delta_y in [100.0, -100.0, 100.0] {
        let outcome = dispatch_wheel_for_action_window_test(&page, delta_y).await;
        assert!(outcome.handled, "wheel admission should be acknowledged");
    }

    tokio::time::timeout(Duration::from_secs(3), effect_request_seen)
        .await
        .expect("the owner scheduler should apply the wheel batch at its deadline")
        .expect("IntersectionObserver effect signal should remain open");
    assert!(
        opened_at.elapsed() >= Duration::from_millis(900),
        "the fixed one-second action window must not apply immediately"
    );
    release_effect_response
        .send(())
        .expect("intersection effect response should release once");
    effect_server
        .await
        .expect("intersection effect server should finish");

    let (state, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify({
  scrollY,
  wheelLog: __lmActionWindowWheelLog,
  ioLog: __lmActionWindowIoLog
})"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("applied action-window state should remain observable");
    assert_eq!(
        renderer_json_value(state),
        Some(serde_json::json!(
            r#"{"scrollY":100,"wheelLog":["event:100","event:-100","event:100","microtask:100","microtask:-100","microtask:100"],"ioLog":[false,true]}"#
        ))
    );

    page.close_async()
        .await
        .expect("action-window deadline page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn screenshot_and_screencast_flush_pending_wheel_actions_before_paint() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/action-window-capture-barriers").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url,
        r#"<!doctype html>
<style>
html, body { margin: 0; background: white; }
html { scrollbar-width: none; }
body { height: 1200px; }
#witness { position: fixed; inset: 0; background: white; }
</style>
<div id="witness"></div>
<script>
globalThis.__lmActionWindowCaptureDeltas = [];
addEventListener("wheel", event => {
  __lmActionWindowCaptureDeltas.push(event.deltaY);
  document.getElementById("witness").style.background =
    __lmActionWindowCaptureDeltas.length === 1 ? "rgb(255, 0, 0)" : "rgb(0, 255, 0)";
}, { capture: true });
</script>"#,
    )
    .await;
    page.run_async_command(RendererPageCommand::SetViewportSurface(Some(
        crate::protocol_types::ViewportSurface {
            inner_width: 20,
            inner_height: 20,
            outer_width: 20,
            outer_height: 20,
            device_pixel_ratio: 1.0,
            screen_width: 20,
            screen_height: 20,
            screen_avail_width: 20,
            screen_avail_height: 20,
        },
    )))
    .await
    .expect("capture barrier viewport should update");

    assert!(
        dispatch_wheel_for_action_window_test(&page, 10.0)
            .await
            .handled
    );
    let screenshot = capture_screenshot_for_renderer_page(&page).await;
    assert_eq!(
        decoded_png_pixel(&screenshot.bytes, 10, 10),
        [255, 0, 0, 255]
    );

    assert!(
        dispatch_wheel_for_action_window_test(&page, 20.0)
            .await
            .handled
    );
    let screencast = capture_screencast_frame_with_request(
        &page,
        super::RendererCaptureScreencastFrameRequest {
            format: super::RendererScreenshotFormat::Png,
            quality: 100,
            optimize_for_speed: true,
            max_width: None,
            max_height: None,
            known_visual_state: None,
        },
    )
    .await;
    assert_eq!(
        decoded_png_pixel(&screencast.image.bytes, 10, 10),
        [0, 255, 0, 255]
    );

    let (state, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "JSON.stringify({ scrollY, deltas: __lmActionWindowCaptureDeltas })"
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("capture barrier state should remain observable");
    assert_eq!(
        renderer_json_value(state),
        Some(serde_json::json!(r#"{"scrollY":30,"deltas":[10,20]}"#))
    );

    page.close_async()
        .await
        .expect("capture barrier page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn wheel_batch_stops_on_document_replacement() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/action-window-document-open").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url,
        r#"<!doctype html>
<style>html, body { margin: 0; } body { height: 1200px; }</style>
<script>
globalThis.__lmRetiredDeltas = [];
globalThis.__lmReplacementDeltas = [];
document.addEventListener("wheel", event => {
  __lmRetiredDeltas.push(event.deltaY);
  document.open();
  document.write("<!doctype html><body style='height:1200px'>replacement</body>");
  document.close();
  document.addEventListener("wheel", replacementEvent => {
    __lmReplacementDeltas.push(replacementEvent.deltaY);
  }, { capture: true });
}, { capture: true, once: true });
</script>"#,
    )
    .await;

    for delta_y in [10.0, 20.0, 30.0] {
        assert!(
            dispatch_wheel_for_action_window_test(&page, delta_y)
                .await
                .handled
        );
    }

    let (state, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify({
  retired: __lmRetiredDeltas,
  replacement: __lmReplacementDeltas
})"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("the explicit read barrier should apply the pending wheel batch");
    assert_eq!(
        renderer_json_value(state),
        Some(serde_json::json!(r#"{"retired":[10],"replacement":[]}"#)),
        "actions admitted for the retired lifecycle must not continue in its document.open replacement"
    );

    page.close_async()
        .await
        .expect("document replacement action-window page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_runs_post_parse_autofocus_after_domcontentloaded_without_a_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-post-parse-autofocus-applied",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page_at_document_commit(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><body>
<input id="owner-autofocus" autofocus>
<script>
globalThis.__lmOwnerAutofocusLog = [];
document.addEventListener("DOMContentLoaded", () => {
  __lmOwnerAutofocusLog.push("dcl");
  Promise.resolve().then(() => __lmOwnerAutofocusLog.push("dcl-microtask"));
});
document.getElementById("owner-autofocus").addEventListener("focus", () => {
  __lmOwnerAutofocusLog.push("focus");
  Promise.resolve().then(() => __lmOwnerAutofocusLog.push("focus-microtask"));
  fetch("/owner-post-parse-autofocus-applied");
}, { once: true });
</script>
</body>"#,
    )
    .await;

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should run autofocus without another command")
        .expect("autofocus handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("autofocus effect response should release once");
    effect_server
        .await
        .expect("autofocus effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerAutofocusLog.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("autofocus handler result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("dcl|dcl-microtask|focus|focus-microtask"))
    );

    page.close_async()
        .await
        .expect("autofocus rendering owner-liveness page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_runs_animation_start_rendering_update_without_timer_or_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-animation-rendering-update-applied",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><head>
<style>
@keyframes owner-animation { from { left: 0px; } to { left: 10px; } }
#animated { position: relative; animation: owner-animation 1s linear; }
</style>
</head><body><div id="animated"></div>
<script>globalThis.__lmOwnerAnimationEvents = 0;</script>
</body>"#,
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
document.getElementById("animated").addEventListener("animationstart", () => {
  __lmOwnerAnimationEvents++;
  fetch("/owner-animation-rendering-update-applied");
}, { once: true });
"scheduled"
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("animation listener should enter the rendering source");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch animationstart without another command")
        .expect("animation handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("animation effect response should release once");
    effect_server
        .await
        .expect("animation effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "String(globalThis.__lmOwnerAnimationEvents)".to_owned(),
            await_promise: false,
        })
        .await
        .expect("animation handler result should remain observable");
    assert_eq!(renderer_json_value(events), Some(serde_json::json!("1")));

    page.close_async()
        .await
        .expect("animation rendering owner-liveness page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_dispatches_dom_manipulation_fifo_without_timer_or_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-dom-manipulation-applied",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><body>
<details id="owner-toggle"><summary>summary</summary></details>
<script>
globalThis.__lmOwnerDomManipulationLog = [];
document.getElementById("owner-toggle").addEventListener("toggle", event => {
  __lmOwnerDomManipulationLog.push("toggle:" + event.oldState + "->" + event.newState);
  Promise.resolve().then(() => {
    __lmOwnerDomManipulationLog.push("toggle:microtask");
  });
}, { once: true });
</script>
</body>"#,
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
const image = new Image();
image.addEventListener("load", () => {
  __lmOwnerDomManipulationLog.push("image:load");
  Promise.resolve().then(() => {
    __lmOwnerDomManipulationLog.push("image:microtask");
    fetch("/owner-dom-manipulation-applied");
  });
}, { once: true });
document.body.appendChild(image);
document.getElementById("owner-toggle").open = true;
image.src = "/not-fetched-by-policy.png";
"scheduled"
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("toggle and image mutation should enter the DOM-manipulation source");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch the shared DOM FIFO without another command")
        .expect("image handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("DOM-manipulation effect response should release once");
    effect_server
        .await
        .expect("DOM-manipulation effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerDomManipulationLog.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("DOM-manipulation handler order should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!(
            "toggle:closed->open|toggle:microtask|image:load|image:microtask"
        ))
    );

    page.close_async()
        .await
        .expect("DOM-manipulation owner-liveness page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn element_toggle_checkpoint_reconciles_replacement_before_page_restore() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/element-toggle-checkpoint-document-open")
        .expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><body>
<details id="element-toggle-checkpoint"><summary>summary</summary></details>
</body>"#,
    )
    .await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  const details = document.getElementById("element-toggle-checkpoint");
  details.addEventListener("toggle", () => {
    Promise.resolve().then(() => {
      document.open();
      document.write('<main id="element-toggle-replacement">replacement</main>');
      document.close();
    });
  }, { once: true });
  details.open = true;
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("element-toggle replacement reaction should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("the element-toggle task-end checkpoint should install replacement lifecycle");
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                "document.querySelector('#element-toggle-replacement')?.textContent ?? 'missing'"
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement DOM should evaluate");
    assert_eq!(
        renderer_json_value(replacement),
        Some(serde_json::json!("replacement"))
    );

    page.close_async()
        .await
        .expect("element-toggle checkpoint replacement page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn image_load_checkpoint_reconciles_replacement_before_page_restore() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/image-checkpoint-document-open").expect("page URL");
    let mut page =
        create_test_html_page(&runtime, &loader, page_url, "<!doctype html><body></body>").await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  const image = new Image();
  image.addEventListener("load", () => {
    Promise.resolve().then(() => {
      document.open();
      document.write('<main id="image-replacement">replacement</main>');
      document.close();
    });
  }, { once: true });
  document.body.appendChild(image);
  image.src = "/not-fetched-by-policy.png";
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("image replacement reaction should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("the image task-end checkpoint should install replacement lifecycle");
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "document.querySelector('#image-replacement')?.textContent ?? 'missing'"
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement DOM should evaluate");
    assert_eq!(
        renderer_json_value(replacement),
        Some(serde_json::json!("replacement"))
    );

    page.close_async()
        .await
        .expect("image checkpoint replacement page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn connected_style_checkpoint_reconciles_replacement_before_page_restore() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/style-checkpoint-document-open").expect("page URL");
    let mut page =
        create_test_html_page(&runtime, &loader, page_url, "<!doctype html><body></body>").await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  const style = document.createElement("style");
  style.textContent = "body { color: teal; }";
  style.addEventListener("load", () => {
    Promise.resolve().then(() => {
      document.open();
      document.write('<main id="style-replacement">replacement</main>');
      document.close();
    });
  }, { once: true });
  document.head.appendChild(style);
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("connected-style replacement reaction should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("the connected-style task-end checkpoint should install replacement lifecycle");
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "document.querySelector('#style-replacement')?.textContent ?? 'missing'"
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement DOM should evaluate");
    assert_eq!(
        renderer_json_value(replacement),
        Some(serde_json::json!("replacement"))
    );

    page.close_async()
        .await
        .expect("connected-style checkpoint replacement page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn connected_style_microtasks_run_before_the_delayed_window_load_task() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/style-microtask-before-window-load")
        .expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        concat!(
            "<!doctype html><head>",
            "<script>",
            "globalThis.__styleLoadOrder = [];",
            "window.addEventListener('load', () => __styleLoadOrder.push('window-load'));",
            "</script>",
            "<style id='ordered-style'>body { color: olive; }</style>",
            "<script>",
            "document.getElementById('ordered-style').addEventListener('load', () => {",
            "  __styleLoadOrder.push('style-load');",
            "  Promise.resolve().then(() => __styleLoadOrder.push('style-microtask'));",
            "});",
            "</script>",
            "</head><body></body>",
        ),
    )
    .await;

    let (order, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "__styleLoadOrder.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("connected-style/window load order should evaluate");
    assert_eq!(
        renderer_json_value(order),
        Some(serde_json::json!("style-load|style-microtask|window-load")),
        "the element event task must release its delay before returning, while its task-end checkpoint still runs before the later window-load task"
    );

    page.close_async()
        .await
        .expect("connected-style load-order page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_completes_text_track_load_without_a_timer_or_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-text-track-load-applied",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url::Url::parse(&format!("{base_url}/page")).expect("page URL"),
        "<!doctype html><body></body>",
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
const video = document.createElement("video");
const track = document.createElement("track");
track.default = true;
track.src = "data:text/vtt,WEBVTT";
globalThis.__lmOwnerTypedTrackEvents = [];
track.addEventListener("load", () => {
  __lmOwnerTypedTrackEvents.push(`load:${track.readyState}`);
  fetch("/owner-text-track-load-applied");
}, { once: true });
video.append(track);
document.body.append(video);
globalThis.__lmOwnerTypedDefaultTrack = track;
track.track.mode
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("default track insertion should enter the shared DOM source");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("disabled")),
        "the insertion command itself may not apply the later task"
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("text-track load must dispatch without another command")
        .expect("text-track load effect signal should remain open");

    release_effect_response
        .send(())
        .expect("text-track effect response should release once");
    effect_server
        .await
        .expect("text-track owner-liveness server should finish");

    let (state, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify({
  mode: globalThis.__lmOwnerTypedDefaultTrack.track.mode,
  readyState: globalThis.__lmOwnerTypedDefaultTrack.readyState,
  events: globalThis.__lmOwnerTypedTrackEvents
})"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("completed text-track load should remain observable");
    assert_eq!(
        renderer_json_value(state),
        Some(serde_json::json!(
            r#"{"mode":"showing","readyState":2,"events":["load:2"]}"#
        ))
    );

    page.close_async()
        .await
        .expect("text-track owner-liveness page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_dispatches_user_interaction_without_timer_or_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-user-interaction-applied",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><body>
<input id="owner-selection" value="abcd">
<script>
globalThis.__lmOwnerUserInteractionLog = [];
document.getElementById("owner-selection").addEventListener("select", event => {
  __lmOwnerUserInteractionLog.push(`${event.type}:${event.bubbles}`);
  fetch("/owner-user-interaction-applied");
}, { once: true });
</script>
</body>"#,
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                r#"document.getElementById("owner-selection").setSelectionRange(0, 2); "scheduled""#
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("selection mutation should schedule one user-interaction task");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch select without another command")
        .expect("select handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("user-interaction effect response should release once");
    effect_server
        .await
        .expect("user-interaction effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerUserInteractionLog.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("user-interaction handler result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("select:true"))
    );

    page.close_async()
        .await
        .expect("user-interaction owner-liveness page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn user_interaction_checkpoint_reconciles_replacement_before_page_restore() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/user-interaction-checkpoint-document-open")
            .expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>initial user-interaction document</body>",
    )
    .await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  const dialog = document.createElement("dialog");
  dialog.addEventListener("close", () => {
    Promise.resolve().then(() => {
      document.open();
      document.write('<main id="user-interaction-checkpoint-replacement">replacement</main>');
      document.close();
    });
  });
  document.body.append(dialog);
  dialog.show();
  dialog.close();
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("user-interaction replacement reaction should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect(
        "the user-interaction task-end checkpoint should install and schedule replacement lifecycle",
    );
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                "document.querySelector('#user-interaction-checkpoint-replacement')?.textContent ?? 'missing'"
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement DOM should evaluate");
    assert_eq!(
        renderer_json_value(replacement),
        Some(serde_json::json!("replacement"))
    );

    page.close_async()
        .await
        .expect("user-interaction checkpoint replacement page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_dispatches_media_events_without_timer_or_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-media-event-applied",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><body><video id="owner-media"></video>
<script>
globalThis.__lmOwnerMediaEventLog = [];
const media = document.getElementById("owner-media");
for (const type of ["seeking", "seeked"]) {
  media.addEventListener(type, () => {
    __lmOwnerMediaEventLog.push(type);
    if (type === "seeked") fetch("/owner-media-event-applied");
  });
}
</script></body>"#,
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"document.getElementById("owner-media").currentTime = 1; "scheduled""#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("media seek should enter the media-element event source");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch both media events without another command")
        .expect("media event effect signal should remain open");

    release_effect_response
        .send(())
        .expect("media event effect response should release once");
    effect_server
        .await
        .expect("media event effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify({
  events: globalThis.__lmOwnerMediaEventLog,
  seeking: document.getElementById("owner-media").seeking
})"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("media event result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!(
            r#"{"events":["seeking","seeked"],"seeking":false}"#
        ))
    );

    page.close_async()
        .await
        .expect("media event owner-liveness page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_finishes_navigation_api_task_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-navigation-api-task-applied",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>Navigation API owner turn</body>",
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r##"(() => {
  globalThis.__lmOwnerNavigationApiTaskLog = [];
  navigation.onnavigatesuccess = () => {
    __lmOwnerNavigationApiTaskLog.push("success:" + location.hash);
    fetch("/owner-navigation-api-task-applied");
  };
  navigation.navigate("/next-document");
  const result = navigation.navigate("#replacement");
  result.finished.then(() => __lmOwnerNavigationApiTaskLog.push("finished"));
  return "scheduled";
})()"##
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("Navigation API finished task should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should run the Navigation API task without another command")
        .expect("Navigation API handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("Navigation API effect response should release once");
    effect_server
        .await
        .expect("Navigation API effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerNavigationApiTaskLog.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("Navigation API task result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("success:#replacement|finished"))
    );

    page.close_async()
        .await
        .expect("Navigation API owner-liveness page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_dispatches_production_window_message_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-window-message-delivered",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>Window.postMessage owner turn</body>",
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lmOwnerWindowMessageEvents = [];
  onmessage = event => {
    __lmOwnerWindowMessageEvents.push("message:" + event.data);
    fetch("/owner-window-message-delivered");
  };
  postMessage("go", "*");
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("Window.postMessage should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch the typed Window.postMessage task")
        .expect("Window.postMessage handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("Window.postMessage effect response should release once");
    effect_server
        .await
        .expect("Window.postMessage effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerWindowMessageEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("Window.postMessage handler result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("message:go"))
    );

    page.close_async()
        .await
        .expect("Window.postMessage owner-liveness page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn window_message_checkpoint_reconciles_document_replacement_before_restoring_page_residence()
{
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/window-message-checkpoint-document-open")
        .expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>initial Window.postMessage document</body>",
    )
    .await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  onmessage = () => {
    Promise.resolve().then(() => {
      document.open();
      document.write('<main id="window-message-checkpoint-replacement">replacement</main>');
      document.close();
    });
  };
  postMessage("replace-document", "*");
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("Window.postMessage replacement reaction should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect(
        "the Window.postMessage task-end checkpoint should install and schedule replacement lifecycle",
    );
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                "document.querySelector('#window-message-checkpoint-replacement')?.textContent ?? 'missing'"
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement DOM should evaluate");
    assert_eq!(
        renderer_json_value(replacement),
        Some(serde_json::json!("replacement"))
    );

    page.close_async()
        .await
        .expect("Window.postMessage checkpoint replacement page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_dispatches_production_message_port_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/owner-message-port-delivered",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>MessagePort owner turn</body>",
    )
    .await;
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lmOwnerMessagePortEvents = [];
  globalThis.__lmOwnerMessagePortChannel = new MessageChannel();
  const { port1, port2 } = __lmOwnerMessagePortChannel;
  port1.onmessage = event => {
    __lmOwnerMessagePortEvents.push("message:" + event.data);
    fetch("/owner-message-port-delivered");
  };
  port2.postMessage("go");
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("MessagePort delivery should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("owner scheduler should dispatch the typed MessagePort task")
        .expect("MessagePort handler effect signal should remain open");

    release_effect_response
        .send(())
        .expect("MessagePort effect response should release once");
    effect_server
        .await
        .expect("MessagePort effect server should finish");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmOwnerMessagePortEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("MessagePort handler result should remain observable");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("message:go"))
    );

    page.close_async()
        .await
        .expect("MessagePort owner-liveness page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn message_port_checkpoint_reconciles_document_replacement_before_restoring_page_residence() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/message-port-checkpoint-document-open")
        .expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>initial MessagePort document</body>",
    )
    .await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lmCheckpointMessagePortChannel = new MessageChannel();
  const { port1, port2 } = __lmCheckpointMessagePortChannel;
  port1.onmessage = () => {
    Promise.resolve().then(() => {
      document.open();
      document.write('<main id="message-port-checkpoint-replacement">replacement</main>');
      document.close();
    });
  };
  port2.postMessage("replace-document");
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("MessagePort replacement reaction should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect(
        "the MessagePort task-end checkpoint should install and schedule replacement lifecycle",
    );
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                "document.querySelector('#message-port-checkpoint-replacement')?.textContent ?? 'missing'"
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement DOM should evaluate");
    assert_eq!(
        renderer_json_value(replacement),
        Some(serde_json::json!("replacement"))
    );

    page.close_async()
        .await
        .expect("MessagePort checkpoint replacement page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_service_worker_responds_with_body_accessed_opaque_cache_response() {
    let (base_url, worker_server) = spawn_owner_service_worker_response_sequence(vec![(
        "/app/worker.js",
        "text/javascript; charset=utf-8",
        r#"
self.addEventListener("install", event => {
  event.waitUntil(Promise.resolve());
});
self.addEventListener("activate", event => {
  event.waitUntil(clients.claim());
});

function assertOpaqueResponse(response, label) {
  response.body;
  if (response.type !== "opaque" ||
      response.status !== 0 ||
      response.body !== null ||
      response.bodyUsed) {
    throw new Error(label + ":" + [
      response.type,
      response.status,
      response.body === null,
      response.bodyUsed
    ].join("/"));
  }
}

function maybeClone(response, cloneMode) {
  if (cloneMode === "clone-response") {
    const clone = response.clone();
    assertOpaqueResponse(clone, "clone-response");
    return clone;
  }
  if (cloneMode === "clone-unused") {
    const unused = response.clone();
    assertOpaqueResponse(unused, "clone-unused");
  }
  return response;
}

async function passThroughCacheIfNeeded(event, response, cacheMode) {
  if (cacheMode !== "cache") {
    return response;
  }
  const cacheName = event.request.url;
  await self.caches.delete(cacheName);
  const cache = await self.caches.open(cacheName);
  await cache.put(event.request, response);
  const matched = await cache.match(event.request.url);
  assertOpaqueResponse(matched, "matched");
  await self.caches.delete(cacheName);
  return matched;
}

self.addEventListener("fetch", event => {
  const url = new URL(event.request.url);
  if (!url.pathname.endsWith("/TestRequest")) {
    return;
  }
  event.respondWith(fetch(url.searchParams.get("jsonp"), { mode: "no-cors" })
    .then(async response => {
      assertOpaqueResponse(response, "original");
      const selected = maybeClone(response, url.searchParams.get("clone"));
      assertOpaqueResponse(selected, "selected");
      const finalResponse = await passThroughCacheIfNeeded(
        event,
        selected,
        url.searchParams.get("passThroughCache")
      );
      assertOpaqueResponse(finalResponse, "final");
      return finalResponse;
    }));
});
"#,
    )])
    .await;
    let (cross_base_url, cross_server) = spawn_owner_service_worker_response_sequence(vec![
        (
            "/app/respond-with-body-accessed-response.jsonp",
            "application/javascript",
            "globalThis.__serviceWorkerOpaqueBodyAccessedCallback('OK');",
        );
        6
    ])
    .await;
    let opaque_jsonp_url =
        format!("{cross_base_url}/app/respond-with-body-accessed-response.jsonp");
    let opaque_jsonp_url_literal =
        serde_json::to_string(&opaque_jsonp_url).expect("serialize opaque JSONP URL");

    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse(&format!("{base_url}/app/page.html")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>service worker opaque response</body>",
    )
    .await;

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                r#"
(async () => {{
  await navigator.serviceWorker.register("worker.js", {{ scope: "./" }});
  await navigator.serviceWorker.ready;
  const body = document.body;
  const run = (clone, cacheMode) => new Promise((resolve, reject) => {{
    const script = document.createElement("script");
    const callbackName = "__serviceWorkerOpaqueBodyAccessedCallback";
    globalThis[callbackName] = value => {{
      delete globalThis[callbackName];
      script.remove();
      resolve(["opaque", clone, cacheMode, value].join(":"));
    }};
    script.onerror = () => {{
      delete globalThis[callbackName];
      reject(new Error("script error:" + clone + "/" + cacheMode));
    }};
    script.src =
      "TestRequest?clone=" + clone +
      "&passThroughCache=" + cacheMode +
      "&jsonp=" + encodeURIComponent({opaque_jsonp_url_literal});
    body.appendChild(script);
  }});
  const runMode = async cacheMode => [
    await run("none", cacheMode),
    await run("clone-response", cacheMode),
    await run("clone-unused", cacheMode)
  ].join(",");
  return [await runMode("direct"), await runMode("cache")].join("|");
}})()
"#
            ),
            await_promise: true,
        })
        .await
        .expect("owner scheduler should settle the ServiceWorker response sequence");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!(concat!(
            "opaque:none:direct:OK,",
            "opaque:clone-response:direct:OK,",
            "opaque:clone-unused:direct:OK|",
            "opaque:none:cache:OK,",
            "opaque:clone-response:cache:OK,",
            "opaque:clone-unused:cache:OK"
        )))
    );

    worker_server
        .await
        .expect("service worker script server should finish");
    cross_server
        .await
        .expect("opaque response server should finish");
    page.close_async()
        .await
        .expect("ServiceWorker opaque-response page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn page_producer_wake_is_admitted_during_sustained_command_input() {
    const COMMAND_COUNT: usize = 32;

    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, delivery_request_seen, release_delivery_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/page-admission-command-fairness",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>Page admission command fairness</body>",
    )
    .await;

    // Queue the producer setup first, then populate the command channel
    // without yielding. The BroadcastChannel wake must pass the owner
    // admission boundary before that already-ready command batch drains.
    let setup = page
        .enqueue_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lmPageAdmissionCommandCount = 0;
  globalThis.__lmPageAdmissionCountAtDelivery = null;
  const receiver = new BroadcastChannel('page-admission-command-fairness');
  const sender = new BroadcastChannel('page-admission-command-fairness');
  globalThis.__lmPageAdmissionChannels = { receiver, sender };
  receiver.onmessage = () => {
    globalThis.__lmPageAdmissionCountAtDelivery =
      globalThis.__lmPageAdmissionCommandCount;
    fetch('/page-admission-command-fairness');
  };
  sender.postMessage('go');
  return 'scheduled';
})()"#
                .to_owned(),
            await_promise: false,
        })
        .expect("BroadcastChannel setup command should enqueue");
    let mut command_batch = Vec::with_capacity(COMMAND_COUNT);
    for _ in 0..COMMAND_COUNT {
        command_batch.push(
            page.enqueue_async_command(RendererPageCommand::EvaluateExpression {
                expression: "++globalThis.__lmPageAdmissionCommandCount".to_owned(),
                await_promise: false,
            })
            .expect("command-fairness probe should enqueue"),
        );
    }
    let setup_completion = setup
        .wait()
        .await
        .expect("BroadcastChannel setup command should run");
    let (setup_completion, _renderer_output_predecessor) =
        setup_completion.into_completion_and_predecessor();
    let (setup_reply, _, _) = setup_completion.into_parts();
    assert_eq!(
        renderer_json_value(setup_reply),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), delivery_request_seen)
        .await
        .expect("Page admissions must not remain hidden behind the command queue")
        .expect("delivery effect request signal should remain open");
    release_delivery_response
        .send(())
        .expect("delivery effect response should release once");
    server
        .await
        .expect("Page admission fairness server should finish");
    for command in command_batch {
        command
            .wait()
            .await
            .expect("queued command-fairness probe should complete");
    }

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmPageAdmissionCountAtDelivery".to_owned(),
            await_promise: false,
        })
        .await
        .expect("Page admission fairness result should remain observable");
    let count_at_delivery = renderer_json_value(observed)
        .and_then(|value| value.as_u64())
        .expect("delivery handler should capture the command count");
    assert!(
        count_at_delivery < COMMAND_COUNT as u64,
        "a ready Page producer wake must not wait for the entire command queue: {count_at_delivery}"
    );
    assert!(
        count_at_delivery <= 1,
        "one admitted producer Page turn may allow at most one ready command to overtake: {count_at_delivery}"
    );

    page.close_async()
        .await
        .expect("Page admission fairness page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn due_page_deadline_is_admitted_during_sustained_command_input() {
    const COMMAND_COUNT: usize = 1_024;

    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, timer_request_seen, release_timer_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/page-deadline-command-fairness",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>Page deadline command fairness</body>",
    )
    .await;

    let setup = page
        .enqueue_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lmPageDeadlineCommandCount = 0;
  globalThis.__lmPageDeadlineCountAtCallback = null;
  setTimeout(() => {
    globalThis.__lmPageDeadlineCountAtCallback =
      globalThis.__lmPageDeadlineCommandCount;
    fetch('/page-deadline-command-fairness');
  }, 0);
  return 'scheduled';
})()"#
                .to_owned(),
            await_promise: false,
        })
        .expect("zero-delay timer setup command should enqueue");
    let mut command_batch = Vec::with_capacity(COMMAND_COUNT);
    for _ in 0..COMMAND_COUNT {
        command_batch.push(
            page.enqueue_async_command(RendererPageCommand::EvaluateExpression {
                expression: "++globalThis.__lmPageDeadlineCommandCount".to_owned(),
                await_promise: false,
            })
            .expect("deadline command-fairness probe should enqueue"),
        );
    }
    setup
        .wait()
        .await
        .expect("zero-delay timer setup command should run");

    tokio::time::timeout(Duration::from_secs(2), timer_request_seen)
        .await
        .expect("a due Page deadline must not remain hidden behind the command queue")
        .expect("timer effect request signal should remain open");
    release_timer_response
        .send(())
        .expect("timer effect response should release once");
    server
        .await
        .expect("Page deadline fairness server should finish");
    for command in command_batch {
        command
            .wait()
            .await
            .expect("queued deadline command-fairness probe should complete");
    }

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmPageDeadlineCountAtCallback".to_owned(),
            await_promise: false,
        })
        .await
        .expect("Page deadline fairness result should remain observable");
    let count_at_callback = renderer_json_value(observed)
        .and_then(|value| value.as_u64())
        .expect("timer callback should capture the command count");
    assert!(
        count_at_callback < COMMAND_COUNT as u64,
        "a Page deadline that becomes due must interrupt sustained command admission: {count_at_callback}"
    );

    page.close_async()
        .await
        .expect("Page deadline fairness page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_bounds_ordinary_starvation_of_document_lifecycle() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, completion_request_seen, release_completion_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/page-turn-class-fairness-complete",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let html = r#"<!doctype html><body><script>
globalThis.__lmPageTurnClassFairness = {
  deliveries: 0,
  domContentLoadedAt: null,
  loadAt: null,
  completionRequested: false,
};
const maybeCompletePageTurnClassFairness = () => {
  const state = globalThis.__lmPageTurnClassFairness;
  if (state.deliveries === 64 && state.loadAt !== null && !state.completionRequested) {
    state.completionRequested = true;
    fetch('/page-turn-class-fairness-complete');
  }
};
const receiver = new BroadcastChannel('page-turn-class-fairness');
const sender = new BroadcastChannel('page-turn-class-fairness');
globalThis.__lmPageTurnClassFairnessChannels = { receiver, sender };
receiver.onmessage = () => {
  const state = globalThis.__lmPageTurnClassFairness;
  state.deliveries += 1;
  if (state.deliveries < 64) sender.postMessage(state.deliveries);
  maybeCompletePageTurnClassFairness();
};
document.addEventListener('DOMContentLoaded', () => {
  globalThis.__lmPageTurnClassFairness.domContentLoadedAt =
    globalThis.__lmPageTurnClassFairness.deliveries;
});
window.addEventListener('load', () => {
  globalThis.__lmPageTurnClassFairness.loadAt =
    globalThis.__lmPageTurnClassFairness.deliveries;
  maybeCompletePageTurnClassFairness();
});
sender.postMessage(0);
</script></body>"#;
    let mut page =
        create_test_html_page_at_document_commit(&runtime, &loader, page_url, html).await;

    tokio::time::timeout(Duration::from_secs(2), completion_request_seen)
        .await
        .expect("ordinary and lifecycle turns should both make autonomous progress")
        .expect("fairness completion request signal should remain open");
    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "JSON.stringify(globalThis.__lmPageTurnClassFairness)".to_owned(),
            await_promise: false,
        })
        .await
        .expect("page-turn fairness state should remain observable");
    let serialized = renderer_json_value(observed).expect("fairness state should serialize");
    let state: serde_json::Value = serde_json::from_str(
        serialized
            .as_str()
            .expect("fairness state should serialize as JSON"),
    )
    .expect("fairness state JSON should parse");
    assert_eq!(state["deliveries"], serde_json::json!(64));
    assert_eq!(state["completionRequested"], serde_json::json!(true));
    assert!(
        state["domContentLoadedAt"]
            .as_u64()
            .is_some_and(|at| at < 64),
        "sustained ordinary delivery must yield to DOMContentLoaded before draining: {state}"
    );
    assert!(
        state["loadAt"].as_u64().is_some_and(|at| at < 64),
        "sustained ordinary delivery must yield to load before draining: {state}"
    );

    release_completion_response
        .send(())
        .expect("fairness completion response should release once");
    server
        .await
        .expect("fairness completion server should finish");
    page.close_async()
        .await
        .expect("page-turn fairness page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn blocked_lifecycle_fairness_yield_preserves_ordinary_liveness() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    loader.set_image_fetch_enabled(true);
    let (resource_base_url, resource_request_seen, release_resource_response, resource_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/blocked-lifecycle-resource",
            "<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'/>",
            "image/svg+xml",
        )
        .await;
    let (ordinary_base_url, ordinary_request_seen, release_ordinary_response, ordinary_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/blocked-lifecycle-ordinary-complete",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let (load_base_url, load_request_seen, release_load_response, load_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/blocked-lifecycle-load-complete",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{ordinary_base_url}/page")).expect("page URL");
    let resource_url = format!("{resource_base_url}/blocked-lifecycle-resource");
    let ordinary_completion_url =
        format!("{ordinary_base_url}/blocked-lifecycle-ordinary-complete");
    let load_completion_url = format!("{load_base_url}/blocked-lifecycle-load-complete");
    let html = format!(
        r#"<!doctype html><body>
<img src={resource_url_literal}>
<script>
globalThis.__lmBlockedLifecycleFairness = {{ deliveries: 0, load: false }};
const receiver = new BroadcastChannel('blocked-lifecycle-fairness');
const sender = new BroadcastChannel('blocked-lifecycle-fairness');
globalThis.__lmBlockedLifecycleFairnessChannels = {{ receiver, sender }};
receiver.onmessage = () => {{
  const state = globalThis.__lmBlockedLifecycleFairness;
  state.deliveries += 1;
  if (state.deliveries < 64) {{
    sender.postMessage(state.deliveries);
  }} else {{
    fetch({ordinary_completion_url_literal});
  }}
}};
window.addEventListener('load', () => {{
  globalThis.__lmBlockedLifecycleFairness.load = true;
  fetch({load_completion_url_literal});
}});
sender.postMessage(0);
</script></body>"#,
        resource_url_literal =
            serde_json::to_string(&resource_url).expect("serialize resource URL"),
        ordinary_completion_url_literal = serde_json::to_string(&ordinary_completion_url)
            .expect("serialize ordinary completion URL"),
        load_completion_url_literal =
            serde_json::to_string(&load_completion_url).expect("serialize load completion URL"),
    );
    let mut page =
        create_test_html_page_at_document_commit(&runtime, &loader, page_url, &html).await;

    tokio::time::timeout(Duration::from_secs(2), resource_request_seen)
        .await
        .expect("load-blocking resource should start")
        .expect("load-blocking resource signal should remain open");
    tokio::time::timeout(Duration::from_secs(2), ordinary_request_seen)
        .await
        .expect("ordinary source must continue after lifecycle reports Blocked")
        .expect("ordinary completion signal should remain open");
    let (before_release, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "JSON.stringify(globalThis.__lmBlockedLifecycleFairness)".to_owned(),
            await_promise: false,
        })
        .await
        .expect("blocked-lifecycle fairness state should remain observable");
    assert_eq!(
        renderer_json_value(before_release),
        Some(serde_json::json!(r#"{"deliveries":64,"load":false}"#)),
        "ordinary work must drain while load remains blocked"
    );

    release_ordinary_response
        .send(())
        .expect("ordinary completion response should release once");
    ordinary_server
        .await
        .expect("ordinary completion server should finish");
    release_resource_response
        .send(())
        .expect("load-blocking response should release once");
    resource_server
        .await
        .expect("load-blocking resource server should finish");
    tokio::time::timeout(Duration::from_secs(2), load_request_seen)
        .await
        .expect("released resource should wake the exact lifecycle resident")
        .expect("load completion signal should remain open");
    release_load_response
        .send(())
        .expect("load completion response should release once");
    load_server
        .await
        .expect("load completion server should finish");

    page.close_async()
        .await
        .expect("blocked-lifecycle fairness page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_dispatches_universal_isolated_world_broadcast_channel_to_exact_realm() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_server) = spawn_owner_wake_server_with_content_type(
        "/owner-universal-world-broadcast-delivered",
        "ok",
        "text/plain; charset=utf-8",
        Duration::ZERO,
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>universal isolated BroadcastChannel world</body>",
    )
    .await;

    let (world_reply, _) = page
        .run_async_command(RendererPageCommand::CreateIsolatedWorld {
            name: "universal-broadcast-channel".to_owned(),
            grant_universal_access: true,
            frame_id: None,
        })
        .await
        .expect("universal isolated world should be created");
    let RendererPageReply::ExecutionContextId(world_context_id) = world_reply else {
        panic!("CreateIsolatedWorld should return its execution context id");
    };

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: world_context_id,
            expression: r#"(() => {
  globalThis.__lmUniversalBroadcastEvents = [];
  globalThis.__lmUniversalBroadcastReceiver = new BroadcastChannel("universal-world-owner");
  globalThis.__lmUniversalBroadcastReceiver.onmessage = event => {
    __lmUniversalBroadcastEvents.push("message:" + event.data);
    fetch("/owner-universal-world-broadcast-delivered");
  };
  globalThis.__lmUniversalBroadcastSender = new BroadcastChannel("universal-world-owner");
  __lmUniversalBroadcastSender.postMessage("go");
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("isolated-world BroadcastChannel delivery should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_server)
        .await
        .expect("owner scheduler should dispatch the universal-world delivery")
        .expect("universal-world BroadcastChannel effect server should finish");

    let (world_events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id: world_context_id,
            expression: "globalThis.__lmUniversalBroadcastEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("isolated-world BroadcastChannel result should remain observable");
    assert_eq!(
        renderer_json_value(world_events),
        Some(serde_json::json!("message:go"))
    );

    let (default_world_marker, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "typeof globalThis.__lmUniversalBroadcastEvents".to_owned(),
            await_promise: false,
        })
        .await
        .expect("default world should remain observable");
    assert_eq!(
        renderer_json_value(default_world_marker),
        Some(serde_json::json!("undefined")),
        "delivery must remain bound to the accepting isolated realm"
    );

    page.close_async()
        .await
        .expect("universal-world BroadcastChannel page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_applies_child_document_terminal_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerChildDocumentLivenessServer {
        base_url,
        document_request_seen,
        release_document_response,
        effect_request_seen,
        task: server,
    } = spawn_owner_child_document_liveness_server().await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>child document owner liveness</body>",
    )
    .await;

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_owner_child_document = "pending";
  const frame = document.createElement("iframe");
  frame.src = "/owner-child-document.html";
  document.body.appendChild(frame);
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("external child document should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );
    tokio::time::timeout(Duration::from_secs(2), document_request_seen)
        .await
        .expect("owner scheduler should start the child navigation without another command")
        .expect("child document request signal should remain open");
    release_document_response
        .send(())
        .expect("child document response should be released once");
    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("typed terminal and child script follow-up should run without observation")
        .expect("child document effect request signal should remain open");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lm_owner_child_document".to_owned(),
            await_promise: false,
        })
        .await
        .expect("child document marker should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("committed"))
    );

    page.close_async()
        .await
        .expect("child document owner-liveness page should close");
    server
        .await
        .expect("child document owner-liveness server should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_runs_child_parser_classic_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerChildClassicLivenessServer {
        base_url,
        source_request_seen,
        release_source_response,
        effect_request_seen,
        task: server,
    } = spawn_owner_child_classic_liveness_server().await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>child classic owner liveness</body>",
    )
    .await;

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                r#"(() => {{
  globalThis.__lm_owner_child_classic = "pending";
  const frame = document.createElement("iframe");
  frame.srcdoc = `<base href="{base_url}/">
    <script src="{base_url}/owner-child-classic.js"><\/script>`;
  document.body.appendChild(frame);
  return "scheduled";
}})()"#
            ),
            await_promise: false,
        })
        .await
        .expect("child parser classic should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), source_request_seen)
        .await
        .expect("typed classic fetch-start should reach the network without another command")
        .expect("classic source request signal should remain open");
    release_source_response
        .send(())
        .expect("classic source response should be released once");
    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("typed completion and script execution should finish without observation commands")
        .expect("classic script effect request signal should remain open");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lm_owner_child_classic".to_owned(),
            await_promise: false,
        })
        .await
        .expect("child classic marker should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("executed"))
    );

    page.close_async()
        .await
        .expect("child classic owner-liveness page should close");
    server
        .await
        .expect("child classic owner-liveness server should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_runs_child_dynamic_import_fanout_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerDynamicImportLivenessServer {
        base_url,
        dynamic_root_request_seen,
        release_dynamic_root_response,
        effect_request_seen,
        task: server,
    } = spawn_owner_dynamic_import_liveness_server().await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>dynamic import owner liveness</body>",
    )
    .await;

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                r#"(() => {{
  globalThis.__lm_dynamic_owner_liveness = "pending";
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <base href="{base_url}/">
    <script type="module" src="{base_url}/dynamic-owner-entry.js"><\/script>
  `;
  document.body.appendChild(frame);
  return "scheduled";
}})()"#
            ),
            await_promise: false,
        })
        .await
        .expect("child dynamic import should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), dynamic_root_request_seen)
        .await
        .expect("dynamic-import root request should start without another command")
        .expect("dynamic-import root request signal should remain open");
    release_dynamic_root_response
        .send(())
        .expect("dynamic-import root response should be released once");
    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("dynamic-import fanout and evaluation should finish without observation commands")
        .expect("dynamic-import effect request signal should remain open");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lm_dynamic_owner_liveness".to_owned(),
            await_promise: false,
        })
        .await
        .expect("completed dynamic-import marker should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("fulfilled:42"))
    );

    page.close_async()
        .await
        .expect("dynamic-import owner-liveness page should close");
    server
        .await
        .expect("dynamic-import owner-liveness server should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_starts_and_completes_child_modulepreload_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerModulepreloadLivenessServer {
        base_url,
        module_request_seen,
        release_module_response,
        effect_request_seen,
        task: server,
    } = spawn_owner_modulepreload_liveness_server(
        "/owner-modulepreload.js",
        "export const ownerModulepreload = true;",
        "/owner-modulepreload-load-event",
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>modulepreload owner liveness</body>",
    )
    .await;

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                r#"(() => {{
  setTimeout(() => {{
    const frame = document.createElement("iframe");
    frame.srcdoc = `
      <link rel="modulepreload"
            href="{base_url}/owner-modulepreload.js"
            onload="fetch('{base_url}/owner-modulepreload-load-event')">
    `;
    document.body.appendChild(frame);
  }}, 0);
  return "scheduled";
}})()"#
            ),
            await_promise: false,
        })
        .await
        .expect("timer-backed modulepreload fixture should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    match tokio::time::timeout(Duration::from_secs(2), module_request_seen).await {
        Ok(result) => result.expect("modulepreload request signal should remain open"),
        Err(_) => {
            let (diagnostic, _) = page
                .run_async_command(RendererPageCommand::EvaluateExpression {
                    expression: r#"JSON.stringify((() => {
  const frame = document.querySelector("iframe");
  const child = frame?.contentDocument;
  return {
    frame: Boolean(frame),
    child: Boolean(child),
    readyState: child?.readyState ?? "missing",
    link: Boolean(child?.querySelector('link[rel="modulepreload"]'))
  };
})())"#
                        .to_owned(),
                    await_promise: false,
                })
                .await
                .expect("modulepreload timeout diagnostic should evaluate");
            panic!(
                "typed modulepreload start did not reach the network without another command; child state: {:?}",
                renderer_json_value(diagnostic)
            );
        }
    }
    release_module_response
        .send(())
        .expect("modulepreload response should be released once");
    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("modulepreload completion and link event should run without another command")
        .expect("modulepreload link-event effect signal should remain open");

    page.close_async()
        .await
        .expect("modulepreload owner-liveness page should close");
    server
        .await
        .expect("modulepreload owner-liveness server should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_runs_joined_modulepreload_graph_without_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let OwnerModulepreloadLivenessServer {
        base_url,
        module_request_seen,
        release_module_response,
        effect_request_seen,
        task: server,
    } = spawn_owner_modulepreload_liveness_server(
        "/owner-joined-module.js",
        r#"parent.__lmJoinedModulepreloadEvents.push("module");
fetch(parent.__lmJoinedModulepreloadEvents[0] === "preload-load"
  ? "/owner-joined-module-executed"
  : "/owner-joined-module-order-error");"#,
        "/owner-joined-module-executed",
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>joined modulepreload owner liveness</body>",
    )
    .await;

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                r#"(() => {{
  globalThis.__lmJoinedModulepreloadEvents = [];
  setTimeout(() => {{
    const frame = document.createElement("iframe");
    frame.srcdoc = `
      <link rel="modulepreload"
            href="{base_url}/owner-joined-module.js"
            onload="parent.__lmJoinedModulepreloadEvents.push('preload-load')">
      <script type="module" src="{base_url}/owner-joined-module.js"><\/script>
    `;
    document.body.appendChild(frame);
  }}, 0);
  return "scheduled";
}})()"#
            ),
            await_promise: false,
        })
        .await
        .expect("joined modulepreload fixture should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), module_request_seen)
        .await
        .expect("modulepreload should own the joined root fetch without another command")
        .expect("joined module request signal should remain open");
    release_module_response
        .send(())
        .expect("joined module response should be released once");
    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("joined parser root should execute from owner continuations")
        .expect("joined module execution effect signal should remain open");

    let (events, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmJoinedModulepreloadEvents.join('|')".to_owned(),
            await_promise: false,
        })
        .await
        .expect("joined modulepreload ordering should remain observable after liveness proof");
    assert_eq!(
        renderer_json_value(events),
        Some(serde_json::json!("preload-load|module")),
        "link terminal fanout should precede execution of the same-URL joined module"
    );

    page.close_async()
        .await
        .expect("joined modulepreload owner-liveness page should close");
    server
        .await
        .expect("joined modulepreload owner-liveness server should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_loop_ticks_page_timer_from_active_timer_index() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, timer_request_seen, release_timer_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/timer-index-fired",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let (page, _, _creation_diagnostics, _creation_artifacts, pending_download) = runtime
        .create_html_page_from_response(
            page_url.clone(),
            page_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>timer index</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("page should load");
    assert!(pending_download.is_none());

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_owner_timer_index_marker = "pending";
  setTimeout(() => {
    globalThis.__lm_owner_timer_index_marker = "fired";
    fetch("/timer-index-fired");
  }, 25);
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("timer scheduling evaluate should run");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), timer_request_seen)
        .await
        .expect("the active timer index must run the callback without another Page command")
        .expect("timer callback effect signal should remain open");
    release_timer_response
        .send(())
        .expect("timer callback response should release once");
    server.await.expect("timer callback server should finish");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_owner_timer_index_marker"#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("marker evaluate should run after owner timer wake");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("fired")),
        "owner loop did not tick the page timer from the active timer index"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn timer_checkpoint_reconciles_replacement_before_page_restore() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/timer-checkpoint-document-open").expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>initial timer document</body>",
    )
    .await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  setTimeout(() => {
    Promise.resolve().then(() => {
      document.open();
      document.write('<main id="timer-checkpoint-replacement">replacement</main>');
      document.close();
    });
  }, 0);
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("timer replacement reaction should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("the timer task-end checkpoint should install and schedule replacement lifecycle");
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                "document.querySelector('#timer-checkpoint-replacement')?.textContent ?? 'missing'"
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement DOM should evaluate");
    assert_eq!(
        renderer_json_value(replacement),
        Some(serde_json::json!("replacement"))
    );

    page.close_async()
        .await
        .expect("timer checkpoint replacement page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_loop_applies_indexed_db_task_without_an_observation_command() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, effect_request_seen, release_effect_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/indexed-db-task-applied",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let indexed_db_manager =
        crate::new_indexed_db_manager(None).expect("IndexedDB manager should initialize");
    let mut page = create_test_html_page_with_indexed_db_manager(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>indexed db source</body>",
        &indexed_db_manager,
    )
    .await;

    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lmIndexedDbSourceProbe = { idb: "pending" };
  const request = indexedDB.open(`source-${Math.random()}`, 1);
  request.onupgradeneeded = () => { globalThis.__lmIndexedDbSourceProbe.idb = "upgrade"; };
  request.onerror = () => {
    globalThis.__lmIndexedDbSourceProbe.idb =
      `error:${request.error && request.error.name}`;
  };
  request.onsuccess = () => {
    globalThis.__lmIndexedDbSourceProbe.idb = "success";
    request.result.close();
    fetch("/indexed-db-task-applied");
  };
  return "scheduled";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("IndexedDB source probe should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), effect_request_seen)
        .await
        .expect("the IndexedDB success task must run without another Page command")
        .expect("IndexedDB effect signal should remain open");
    release_effect_response
        .send(())
        .expect("IndexedDB effect response should release once");
    server.await.expect("IndexedDB effect server should finish");

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmIndexedDbSourceProbe.idb".to_owned(),
            await_promise: false,
        })
        .await
        .expect("IndexedDB owner-turn result should remain observable");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("success")),
        "the production wake must follow application of the concrete IDB task, not merely its enqueue"
    );

    page.close_async()
        .await
        .expect("IndexedDB owner-turn page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_runtime_expression_await_uses_page_wake_or_timer_deadline() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/runtime-expression-await").expect("page url");
    let (page, _, _creation_diagnostics, _creation_artifacts, pending_download) = runtime
        .create_html_page_from_response(
            page_url.clone(),
            page_url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            "<!doctype html><body>runtime expression await</body>".to_owned(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("page should load");
    assert!(pending_download.is_none());

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"new Promise(resolve => {
  setTimeout(() => resolve("owner-await-timer"), 25);
})"#
            .to_owned(),
            await_promise: true,
        })
        .await
        .expect("owner runtime expression await should settle from page timer");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("owner-await-timer"))
    );

    let (globals, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify(Object.getOwnPropertyNames(globalThis).sort())"#
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("global property snapshot should evaluate");
    let globals = renderer_json_value(globals)
        .and_then(|value| value.as_str().map(str::to_owned))
        .expect("global property snapshot should be a string");
    assert!(
        !globals.contains("__lmAwaitPromise"),
        "page-level await must not leave legacy global token properties: {globals}"
    );
    assert!(
        !globals.contains("__moliAwaitPromiseToken"),
        "page-level await must not expose legacy await token payloads: {globals}"
    );
    assert!(
        !globals.contains("__moliCompleteRuntimeExpressionAwait"),
        "page-level await must not expose an internal completion binding: {globals}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn script_snapshot_ignores_window_indexed_child_frame_globals() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/snapshot-child-frame-index").expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html>
<iframe srcdoc="<p>child</p>"></iframe>
<script>globalThis.__lm_snapshot_marker = "parent";</script>"#,
    )
    .await;

    let snapshot = RendererPageTestingHandle::new_for_testing(&page)
        .current_page_state_async()
        .await
        .expect("snapshot should refresh");
    let globals = snapshot.script_execution.globals();

    assert_eq!(
        globals.get("__lm_snapshot_marker"),
        Some(&crate::types::JsValueSnapshot::String("parent".to_owned()))
    );
    assert!(
        !globals.contains_key("0"),
        "Window child-frame indexed properties must not be reported as script globals: {globals:?}"
    );

    page.close_async()
        .await
        .expect("snapshot child frame test page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn script_snapshot_does_not_stringify_unsupported_globals() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/snapshot-unsupported-globals").expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html>
<script>
globalThis.__lm_snapshot_to_string_calls = 0;
globalThis.__lm_snapshot_object = {
  toString() {
    globalThis.__lm_snapshot_to_string_calls += 1;
    return "side-effect";
  }
};
globalThis.__lm_snapshot_large_array = new Array(25000).fill("large-item-value");
const __lm_snapshot_revocable_array = Proxy.revocable([], {});
globalThis.__lm_snapshot_revoked_array = __lm_snapshot_revocable_array.proxy;
__lm_snapshot_revocable_array.revoke();
</script>"#,
    )
    .await;

    let snapshot = RendererPageTestingHandle::new_for_testing(&page)
        .current_page_state_async()
        .await
        .expect("snapshot should refresh");
    let globals = snapshot.script_execution.globals();

    assert_eq!(
        globals.get("__lm_snapshot_object"),
        Some(&crate::types::JsValueSnapshot::Unsupported(
            "[object]".to_owned()
        ))
    );
    assert_eq!(
        globals.get("__lm_snapshot_large_array"),
        Some(&crate::types::JsValueSnapshot::Unsupported(
            "[array]".to_owned()
        ))
    );
    assert_eq!(
        globals.get("__lm_snapshot_revoked_array"),
        Some(&crate::types::JsValueSnapshot::Unsupported(
            "[object]".to_owned()
        ))
    );
    assert_eq!(
        globals.get("__lm_snapshot_to_string_calls"),
        Some(&crate::types::JsValueSnapshot::Number(0.0)),
        "snapshot must not run user-defined toString while describing unsupported globals"
    );

    page.close_async()
        .await
        .expect("unsupported globals snapshot test page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn protocol_turn_keeps_page_facts_current_and_marks_globals_snapshot_dirty() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/globals-snapshot/start").expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><title>before</title><script>
globalThis.__lm_snapshot_before = 1;
</script>"#,
    )
    .await;

    let initial = RendererPageTestingHandle::new_for_testing(&page)
        .current_page_state_async()
        .await
        .expect("initial page state");
    assert!(initial.script_execution.globals_are_fresh());
    assert_eq!(
        initial.script_execution.global("__lm_snapshot_before"),
        Some(&crate::types::JsValueSnapshot::Number(1.0))
    );

    let output = page
        .enqueue_protocol_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  delete globalThis.__lm_snapshot_before;
  globalThis.__lm_snapshot_after = 2;
  document.title = "after";
  history.pushState({}, "", "/globals-snapshot/after");
  return "updated";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .expect("protocol command should enqueue")
        .wait()
        .await
        .expect("protocol command should finish");
    let protocol_state = output.completion().page_state();

    assert_eq!(
        protocol_state.script_execution.globals_snapshot_state(),
        crate::types::ScriptGlobalsSnapshotState::Dirty
    );
    assert!(protocol_state.script_execution.fresh_globals().is_none());
    assert_eq!(
        protocol_state
            .script_execution
            .global("__lm_snapshot_before"),
        Some(&crate::types::JsValueSnapshot::Number(1.0)),
        "dirty compatibility access must remain the last complete snapshot"
    );
    assert!(
        protocol_state
            .script_execution
            .global("__lm_snapshot_after")
            .is_none()
    );
    assert_eq!(protocol_state.document_title(), "after");
    assert_eq!(
        protocol_state.final_url().as_str(),
        "https://example.test/globals-snapshot/after"
    );

    let (_, refreshed) = page
        .run_async_command(RendererPageCommand::RefreshFullPageState)
        .await
        .expect("full report refresh should finish");
    assert!(refreshed.script_execution.globals_are_fresh());
    assert!(
        refreshed
            .script_execution
            .global("__lm_snapshot_before")
            .is_none()
    );
    assert_eq!(
        refreshed.script_execution.global("__lm_snapshot_after"),
        Some(&crate::types::JsValueSnapshot::Number(2.0))
    );

    page.close_async()
        .await
        .expect("globals freshness test page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn awaited_protocol_turn_preserves_thin_capture_policy_across_wakes() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/globals-snapshot/await").expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>await globals snapshot</body>",
    )
    .await;

    let output = page
        .enqueue_protocol_command(RendererPageCommand::EvaluateExpression {
            expression: r#"new Promise(resolve => {
  setTimeout(() => {
    globalThis.__lm_snapshot_after_await = "settled";
    resolve("done");
  }, 10);
})"#
            .to_owned(),
            await_promise: true,
        })
        .expect("awaited protocol command should enqueue")
        .wait()
        .await
        .expect("awaited protocol command should finish");

    assert_eq!(
        output
            .completion()
            .page_state()
            .script_execution
            .globals_snapshot_state(),
        crate::types::ScriptGlobalsSnapshotState::Dirty
    );
    let (_, refreshed) = page
        .run_async_command(RendererPageCommand::RefreshFullPageState)
        .await
        .expect("full report refresh should finish");
    assert_eq!(
        refreshed
            .script_execution
            .global("__lm_snapshot_after_await"),
        Some(&crate::types::JsValueSnapshot::String("settled".to_owned()))
    );

    page.close_async()
        .await
        .expect("awaited globals freshness test page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn protocol_navigation_never_reuses_the_replaced_document_globals_snapshot() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let initial_url =
        url::Url::parse("https://example.test/globals-snapshot/old").expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        initial_url,
        r#"<!doctype html><script>globalThis.__lm_old_document_global = "old";</script>"#,
    )
    .await;
    let replacement_html = r#"<!doctype html><title>new document</title><script>
globalThis.__lm_new_document_global = "new";
</script>"#;
    let encoded_replacement_html =
        percent_encoding::utf8_percent_encode(replacement_html, percent_encoding::NON_ALPHANUMERIC);
    let replacement_url = format!("data:text/html;charset=utf-8,{encoded_replacement_html}");

    let output = page
        .enqueue_protocol_command(
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression: format!(r#"location.href = {replacement_url:?}; "navigating""#),
                await_promise: false,
            },
        )
        .expect("protocol navigation command should enqueue")
        .wait()
        .await
        .expect("protocol navigation command should finish");
    let replacement_state = output.completion().page_state();

    assert_eq!(replacement_state.document_title(), "new document");
    assert_eq!(
        replacement_state.script_execution.globals_snapshot_state(),
        crate::types::ScriptGlobalsSnapshotState::Dirty
    );
    assert!(
        replacement_state
            .script_execution
            .global("__lm_old_document_global")
            .is_none(),
        "an old-Document snapshot must never cross the replacement boundary"
    );
    assert!(
        replacement_state
            .script_execution
            .global("__lm_new_document_global")
            .is_none(),
        "the replacement's pre-lifecycle full capture may predate its script, so Dirty must not pretend that value is current"
    );

    let (_, refreshed) = page
        .run_async_command(RendererPageCommand::RefreshFullPageState)
        .await
        .expect("replacement full report refresh should finish");
    assert!(refreshed.script_execution.globals_are_fresh());
    assert_eq!(
        refreshed
            .script_execution
            .global("__lm_new_document_global"),
        Some(&crate::types::JsValueSnapshot::String("new".to_owned()))
    );

    page.close_async()
        .await
        .expect("replacement globals freshness test page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_page_creation_applies_document_write_terminal_from_stable_page_route() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, server) = spawn_owner_wake_server_with_content_type(
        "/owner-document-write.js",
        "globalThis.__lm_owner_document_write_events.push('external');",
        "application/javascript",
        Duration::ZERO,
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let html = r#"<!doctype html><body><script>
globalThis.__lm_owner_document_write_events = ['inline-before'];
document.write(`<script src="/owner-document-write.js" onload="globalThis.__lm_owner_document_write_events.push('load')"><\/script><main id="owner-written-tail">written</main>`);
globalThis.__lm_owner_document_write_events.push('inline-after');
</script><p id="owner-parser-tail">parser</p></body>"#;

    let mut page = create_test_html_page(&runtime, &loader, page_url, html).await;
    server
        .await
        .expect("owner document.write script server should finish");

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify({
  events: globalThis.__lm_owner_document_write_events,
  writtenTail: !!document.getElementById('owner-written-tail'),
  parserTail: !!document.getElementById('owner-parser-tail')
})"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("owner-routed document.write result should evaluate");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!(
            r#"{"events":["inline-before","inline-after","external","load"],"writtenTail":true,"parserTail":true}"#
        )),
        "the public Page creation path must admit, authorize, and apply the typed terminal before completing creation"
    );

    page.close_async()
        .await
        .expect("owner document.write page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_applies_popup_terminal_from_stable_page_route() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, server) = spawn_owner_wake_server_with_content_type(
        "/owner-popup.html",
        concat!(
            "<!doctype html><script>",
            "opener.__lm_owner_popup_events.push('response-script');",
            "opener.__lm_resolve_owner_popup('applied');",
            "</script><p id='owner-popup-body'>popup body</p>",
        ),
        "text/html",
        Duration::ZERO,
    )
    .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let html = r#"<!doctype html><body><script>
globalThis.__lm_owner_popup_events = ['before-open'];
globalThis.__lm_owner_popup_applied = new Promise(resolve => {
  globalThis.__lm_resolve_owner_popup = resolve;
});
globalThis.__lm_owner_popup = open('/owner-popup.html', 'owner-popup');
globalThis.__lm_owner_popup_events.push('after-open');
</script></body>"#;

    let mut page = create_test_html_page(&runtime, &loader, page_url, html).await;
    server
        .await
        .expect("owner-routed popup response server should finish");

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"__lm_owner_popup_applied.then(() => JSON.stringify({
  events: __lm_owner_popup_events,
  body: __lm_owner_popup.document.getElementById('owner-popup-body').textContent
}))"#
                .to_owned(),
            await_promise: true,
        })
        .await
        .expect("owner scheduler should apply the typed popup terminal and resolve its observer");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!(
            r#"{"events":["before-open","after-open","response-script"],"body":"popup body"}"#
        )),
        "the public owner path must admit one popup wake, authorize its exact target, and apply the terminal"
    );

    page.close_async()
        .await
        .expect("owner-routed popup page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_page_creation_replays_ready_document_write_after_older_timer_turn() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, request_seen, timer_effect_seen, release_response, server) =
        spawn_gated_resource_with_concurrent_effect(
            "/owner-document-write-after-timer.js",
            "globalThis.__lm_owner_document_write_timer_events.push('external');",
            "application/javascript",
            "/owner-document-write-timer-fired",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let html = r#"<!doctype html><body><script>
globalThis.__lm_owner_document_write_timer_events = ['inline-before'];
setTimeout(() => {
  globalThis.__lm_owner_document_write_timer_events.push('timer');
  fetch('/owner-document-write-timer-fired');
}, 0);
document.write(`<script src="/owner-document-write-after-timer.js" onload="globalThis.__lm_owner_document_write_timer_events.push('load')"><\/script>`);
globalThis.__lm_owner_document_write_timer_events.push('inline-after');
</script></body>"#;

    let mut creation = Box::pin(create_test_html_page(&runtime, &loader, page_url, html));
    tokio::select! {
        seen = request_seen => {
            seen.expect("document.write request should reach the gated server");
        }
        _ = &mut creation => {
            panic!("page creation must remain parked while the script response is gated");
        }
    }
    tokio::time::timeout(Duration::from_secs(2), timer_effect_seen)
        .await
        .expect("the due timer must run while the resource response is still gated")
        .expect("timer effect signal should remain open");
    release_response
        .send(())
        .expect("document.write response should release after the timer turn");
    let mut page = creation.await;
    server
        .await
        .expect("owner timer/document.write script server should finish");
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "JSON.stringify(globalThis.__lm_owner_document_write_timer_events)"
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("owner timer/document.write result should evaluate");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!(
            r#"["inline-before","inline-after","timer","external","load"]"#
        )),
        "a ready typed terminal must receive a fresh internal Page admission after the older timer wins the first turn"
    );

    page.close_async()
        .await
        .expect("owner timer/document.write page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn followed_navigation_replays_ready_document_write_after_older_timer_turn() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, request_seen, release_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/replacement-document-write-after-timer.js",
            "globalThis.__lm_replacement_document_write_events.push('external');",
            "application/javascript",
        )
        .await;
    let initial_url = url::Url::parse(&format!("{base_url}/initial")).expect("initial page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        initial_url,
        "<!doctype html><body>initial</body>",
    )
    .await;
    let replacement_html = format!(
        r#"<!doctype html><script>
globalThis.__lm_replacement_document_write_events = ['inline-before'];
setTimeout(() => globalThis.__lm_replacement_document_write_events.push('timer'), 0);
document.write(`<script src="{base_url}/replacement-document-write-after-timer.js" onload="globalThis.__lm_replacement_document_write_events.push('load')"><\/script>`);
globalThis.__lm_replacement_document_write_events.push('inline-after');
</script>"#,
    );
    let encoded_replacement_html = percent_encoding::utf8_percent_encode(
        &replacement_html,
        percent_encoding::NON_ALPHANUMERIC,
    );
    let replacement_url = format!("data:text/html;charset=utf-8,{encoded_replacement_html}");
    let release = tokio::spawn(async move {
        request_seen
            .await
            .expect("replacement document.write request should start");
        release_response
            .send(())
            .expect("replacement document.write response should release");
    });

    let (reply, _) = page
        .run_async_command(
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression: format!(r#"location.href = {replacement_url:?}; "navigating""#),
                await_promise: false,
            },
        )
        .await
        .expect("document.write replacement navigation should complete naturally");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("navigating"))
    );
    release
        .await
        .expect("replacement response release task should finish");
    server
        .await
        .expect("replacement document.write server should finish");

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "JSON.stringify(globalThis.__lm_replacement_document_write_events)"
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement timer/document.write result should evaluate");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!(
            r#"["inline-before","inline-after","timer","external","load"]"#
        )),
        "followed navigation must preserve source-specific admission across PageVm installation"
    );

    page.close_async()
        .await
        .expect("replacement document.write page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn domcontentloaded_page_creation_reply_resumes_owner_to_load_without_external_work() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/dcl-reply-load-tail").expect("page url");
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let producer = tokio::spawn(async move {
        body_tx
            .send(b"<!doctype html><body>ready</body>".to_vec())
            .await
            .expect("html chunk should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let (mut page, _, _, creation_artifacts, pending_download) = runtime
        .create_streaming_raw_page_from_external_body(
            page_url.clone(),
            page_url,
            None,
            false,
            0,
            Vec::new(),
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            raw_body,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
            false,
            PageVmInitStage::DomContentLoaded,
            RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            RendererNavigationReplyPolicy::FollowBeforeReply,
        )
        .await
        .expect("page should reply at DOMContentLoaded");
    producer.await.expect("producer should finish");
    assert!(pending_download.is_none());
    assert!(
        creation_artifacts
            .lifecycle_snapshot
            .dom_content_loaded
            .is_some()
    );
    assert!(creation_artifacts.lifecycle_snapshot.load.is_none());

    let load_events = tokio::time::timeout(
        Duration::from_millis(500),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("renderer owner should resume the DCL page reply through load");
    assert!(load_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load)
    )));

    page.close_async()
        .await
        .expect("DCL reply load-tail page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn document_commit_background_dcl_completion_resumes_owner_to_load() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let predecessor_url = url::Url::parse("about:blank").expect("predecessor page url");
    let mut predecessor =
        create_test_html_page(&runtime, &loader, predecessor_url, "<body>initial</body>").await;
    let page_url =
        url::Url::parse("https://example.test/document-commit-dcl-tail").expect("page url");
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let producer = tokio::spawn(async move {
        body_tx
            .send(b"<!doctype html><body>ready</body>".to_vec())
            .await
            .expect("html chunk should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let (mut page, _, _, creation_artifacts, pending_download) = runtime
        .create_streaming_raw_page_from_external_body_with_inspector_session_restores(
            page_url.clone(),
            page_url,
            None,
            false,
            0,
            Vec::new(),
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            raw_body,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
            Vec::new(),
            false,
            PageVmInitStage::DomContentLoaded,
            RendererReplyBoundary::DocumentCommit,
            RendererTopLevelNavigationDispatch::DelegateToBrowser,
            RendererNavigationReplyPolicy::ReturnWithPendingNavigation,
            Some("TID-1".to_owned()),
            None,
            None,
            None,
        )
        .await
        .expect("page should attach at document commit");
    producer.await.expect("producer should finish");
    assert!(pending_download.is_none());
    assert!(
        creation_artifacts
            .lifecycle_snapshot
            .dom_content_loaded
            .is_none()
    );
    assert!(creation_artifacts.lifecycle_snapshot.load.is_none());
    predecessor
        .close_async()
        .await
        .expect("predecessor page should close after replacement attach");
    page.take_committed_document_post_response_continuation()
        .expect("DocumentCommit should defer parser continuation")
        .release();

    let observed = tokio::time::timeout(
        Duration::from_millis(500),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("document-commit background continuation should reach load");
    let milestones = observed
        .iter()
        .filter_map(|event| match event.kind {
            RendererDocumentLifecycleEventKind::Milestone(milestone) => Some(milestone),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        milestones,
        vec![
            RendererDocumentLifecycleMilestone::DomContentLoaded,
            RendererDocumentLifecycleMilestone::Load,
        ]
    );

    page.close_async()
        .await
        .expect("document-commit DCL-tail page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn document_title_observation_precedes_dcl_for_exact_document() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/title-before-dcl").expect("page url");
    let mut page = create_test_html_page_at_document_commit(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><title>committed title</title><main>ready</main>",
    )
    .await;

    let (observed, title_identity, dcl_identity) =
        tokio::time::timeout(Duration::from_secs(2), async {
            let mut observed = Vec::new();
            let mut title_identity = None;
            loop {
                let publication = activity_wake_rx
                    .recv()
                    .await
                    .expect("renderer output channel should stay open");
                if !publication_is_for_page(&publication, &page) {
                    continue;
                }
                for record in publication.records() {
                    match record.item() {
                        RendererOutputItem::Observation(
                            RendererProtocolObservation::DocumentTitleChanged(change),
                        ) => {
                            assert_eq!(change.title, "committed title");
                            assert!(
                                title_identity.replace(change.source_document).is_none(),
                                "an unchanged title must not be published twice"
                            );
                            observed.push("title");
                        }
                        RendererOutputItem::Observation(
                            RendererProtocolObservation::DocumentLifecycle(event),
                        ) if event.kind
                            == RendererDocumentLifecycleEventKind::Milestone(
                                RendererDocumentLifecycleMilestone::DomContentLoaded,
                            ) =>
                        {
                            observed.push("dcl");
                            let identity = super::RendererDocumentLifecycleIdentity {
                                frame: event.frame,
                                document: event.document,
                                epoch: event.epoch,
                            };
                            return (observed, title_identity, identity);
                        }
                        _ => {}
                    }
                }
            }
        })
        .await
        .expect("title and DCL observations should be published");

    assert_eq!(observed, vec!["title", "dcl"]);
    assert_eq!(
        title_identity,
        Some(dcl_identity),
        "the title observation must be sourced from the exact DCL document"
    );

    page.close_async()
        .await
        .expect("title observation page should close");
}

// This witness needs one renderer-owner worker and one observer worker.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lifecycle_record_precedes_handler_navigation_action_record() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/dcl-handler-navigation").expect("page url");
    let html = r#"<!doctype html>
<script>
document.addEventListener("DOMContentLoaded", () => {
  location.href = "https://example.test/final";
}, { once: true });
</script>
<main>DCL handler navigation</main>"#;
    let mut page = create_test_html_page_at_document_commit_with_navigation_dispatch(
        &runtime,
        &loader,
        page_url,
        html,
        RendererTopLevelNavigationDispatch::DelegateToBrowser,
        RendererNavigationReplyPolicy::ReturnWithPendingNavigation,
    )
    .await;

    // DocumentCommit returns before the owner resumes through DCL, so the
    // lifecycle/action tail may already be queued here. Keep that concrete
    // FIFO intact; the page and record filters below ignore creation output
    // that is unrelated to this witness.

    let observed = tokio::time::timeout(Duration::from_secs(2), async {
        let mut observed = Vec::new();
        while observed.len() < 2 {
            let publication = activity_wake_rx
                .recv()
                .await
                .expect("renderer output channel should stay open");
            if !publication_is_for_page(&publication, &page) {
                continue;
            }
            for record in publication.records() {
                match record.item() {
                    super::RendererOutputItem::Observation(
                        super::RendererProtocolObservation::DocumentLifecycle(event),
                    ) if event.kind
                        == RendererDocumentLifecycleEventKind::Milestone(
                            RendererDocumentLifecycleMilestone::DomContentLoaded,
                        ) =>
                    {
                        observed.push("lifecycle")
                    }
                    super::RendererOutputItem::OwnerAction(
                        super::RendererOwnerAction::TopLevelLocationNavigation(_),
                    ) => observed.push("action"),
                    _ => {}
                }
            }
        }
        observed
    })
    .await
    .expect("DCL handler navigation should publish lifecycle and action records");

    assert_eq!(
        observed,
        vec!["lifecycle", "action"],
        "a milestone reached before a handler side effect must precede that action in the concrete FIFO"
    );

    page.close_async()
        .await
        .expect("DCL handler navigation page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn document_commit_release_runs_parser_script_location_handoff_in_background() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/parser-location-source").expect("page url");
    let mut page = create_test_html_page_at_document_commit_with_navigation_dispatch(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html><script>location.href = "/final"</script>"#,
        RendererTopLevelNavigationDispatch::DelegateToBrowser,
        RendererNavigationReplyPolicy::ReturnWithPendingNavigation,
    )
    .await;

    tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(publication) = activity_wake_rx.recv().await {
            if !publication_is_for_page(&publication, &page) {
                continue;
            }
            if publication.records().iter().any(|record| {
                matches!(
                    record.item(),
                    super::RendererOutputItem::OwnerAction(
                        super::RendererOwnerAction::TopLevelLocationNavigation(_)
                    )
                )
            }) {
                return;
            }
        }
        panic!("renderer output channel closed before parser location handoff");
    })
    .await
    .expect("released parser continuation should publish its location handoff");

    page.close_async()
        .await
        .expect("parser location handoff page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_loop_completes_post_dcl_async_script_without_wait_command() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, async_script_request_seen, release_async_script_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/async.js",
            "globalThis.__lm_post_dcl_async_marker = 'executed';",
            "application/javascript",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let html = r#"<!doctype html><body>
<script async src="/async.js"></script>
<script>globalThis.__lm_dcl_script_marker = "inline";</script>
</body>"#;
    let producer = tokio::spawn(async move {
        body_tx
            .send(html.as_bytes().to_vec())
            .await
            .expect("html chunk should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let (page, _, _creation_diagnostics, creation_artifacts, pending_download) = runtime
        .create_streaming_raw_page_from_external_body(
            page_url.clone(),
            page_url,
            None,
            false,
            0,
            Vec::new(),
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            raw_body,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
            false,
            PageVmInitStage::DomContentLoaded,
            RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            RendererNavigationReplyPolicy::FollowBeforeReply,
        )
        .await
        .expect("page should reach DOMContentLoaded");
    producer.await.expect("producer should finish");
    assert!(pending_download.is_none());
    tokio::time::timeout(Duration::from_millis(500), async_script_request_seen)
        .await
        .expect("async script request should reach the gated server")
        .expect("gated async script request channel should stay open");

    assert!(
        creation_artifacts
            .lifecycle_snapshot
            .dom_content_loaded
            .is_some()
    );
    assert!(creation_artifacts.lifecycle_snapshot.load.is_none());
    while activity_wake_rx.try_recv().is_ok() {}

    release_async_script_response
        .send(())
        .expect("release gated async script response");

    let load_events = tokio::time::timeout(
        Duration::from_secs(2),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("page owner should reach load after the async-script completion");
    assert!(load_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load)
    )));

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_post_dcl_async_marker ?? "pending""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("marker evaluate should run");
    assert_eq!(
        renderer_json_value(observed),
        Some(serde_json::json!("executed")),
        "page owner should execute post-DCL async script work without a wait-driver command"
    );
    server
        .await
        .expect("post-DCL async script server should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn synchronous_document_close_schedules_replacement_lifecycle_turn() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/synchronous-document-close").expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>initial</body>",
    )
    .await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
document.open();
document.write('<main id="replacement">replacement</main>');
document.close();
'closed'
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("synchronous document replacement should return");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("closed"))
    );

    let replacement_events = tokio::time::timeout(
        Duration::from_millis(500),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("the installed replacement resident should be scheduled without another driver");
    assert!(replacement_events.iter().any(|event| matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
        )
    )));

    let (replacement, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "document.querySelector('#replacement')?.textContent ?? 'missing'"
                .to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement DOM should evaluate");
    assert_eq!(
        renderer_json_value(replacement),
        Some(serde_json::json!("replacement"))
    );

    page.close_async()
        .await
        .expect("synchronous document.close test page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn command_turn_output_scope_is_removed_after_command_error() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url = url::Url::parse("https://example.test/command-turn-error").expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>initial</body>",
    )
    .await;

    let (invalid_response_tx, _invalid_response_rx) = oneshot::channel();
    let invalid = page
        .enqueue_async_command(
            RendererPageCommand::dispatch_runtime_protocol_message_with_context_resolution_and_deferred_response(
                None,
                "evaluate".to_owned(),
                "{".to_owned(),
                RendererRuntimeInspectorResponseSender::new(
                    710_220,
                    invalid_response_tx,
                ),
            ),
        )
        .expect("invalid Runtime.evaluate should enqueue")
        .wait()
        .await;
    assert!(
        invalid.is_err(),
        "malformed protocol JSON should fail the command"
    );

    let call_id = 710_221;
    let (response_tx, _response_rx) = oneshot::channel();
    let completion = page
        .enqueue_async_command(
            RendererPageCommand::dispatch_runtime_protocol_message_with_deferred_response(
                None,
                serde_json::json!({
                    "id": call_id,
                    "method": "Runtime.evaluate",
                    "params": {
                        "expression": "42",
                        "returnByValue": true,
                    },
                })
                .to_string(),
                RendererRuntimeInspectorResponseSender::new(call_id, response_tx),
            ),
        )
        .expect("the command after an error should enqueue")
        .wait()
        .await
        .expect("the failed command must not leave an active command-turn output scope");
    assert_eq!(
        completion
            .runtime_inspector_output()
            .and_then(|output| output.protocol_response(call_id))
            .expect("the next Runtime command should retain its response")["result"]["result"]["value"],
        serde_json::json!(42)
    );

    page.close_async()
        .await
        .expect("command-turn error cleanup test page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_document_close_completion_parks_lifecycle_until_capability_release() {
    let runtime = JsRuntime::initialize();
    let (activity_wake_tx, mut activity_wake_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(activity_wake_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let page_url =
        url::Url::parse("https://example.test/runtime-command-boundary").expect("page url");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>initial</body>",
    )
    .await;

    let _ = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("initial lifecycle events should be drainable");
    while activity_wake_rx.try_recv().is_ok() {}

    let call_id = 710_221;
    let (response_tx, _response_rx) = oneshot::channel();
    let completion = page
        .enqueue_async_command(
            RendererPageCommand::dispatch_runtime_protocol_message_with_deferred_response(
                None,
                serde_json::json!({
                    "id": call_id,
                    "method": "Runtime.evaluate",
                    "params": {
                        "expression": "document.open(); document.write('<main>replacement</main>'); document.close(); 'done'",
                        "returnByValue": true,
                    },
                })
                .to_string(),
                RendererRuntimeInspectorResponseSender::new(
                    call_id,
                    response_tx,
                ),
            ),
        )
        .expect("Runtime.evaluate should enqueue")
        .wait()
        .await
        .expect("Runtime.evaluate should complete at its renderer command boundary");
    assert!(
        completion.completion().has_post_response_continuation(),
        "the exact post-response continuation belongs to the final completion"
    );
    let (completion, renderer_output_predecessor) = completion.into_completion_and_predecessor();
    assert_eq!(
        completion
            .runtime_inspector_output()
            .and_then(|output| output.protocol_response(call_id))
            .expect("Runtime command completion should retain the response")["result"]["result"]["value"],
        serde_json::json!("done")
    );
    let (reply, _, continuation) = completion.into_parts();
    assert!(matches!(
        reply,
        RendererPageReply::RuntimeInspectorProtocolMessages(ref messages) if !messages.is_empty()
    ));
    let renderer_output_predecessor = renderer_output_predecessor
        .expect("document replacement output must fence the Runtime response");
    let command_publications = activity_wake_rx.drain();
    assert!(command_publications.iter().any(|publication| {
        publication.cursor() == renderer_output_predecessor.cursor()
            && publication.records().iter().any(|record| {
                matches!(
                    record.item(),
                    super::RendererOutputItem::Observation(
                        super::RendererProtocolObservation::DocumentLifecycle(event)
                    ) if matches!(
                        event.kind,
                        RendererDocumentLifecycleEventKind::Started { .. }
                    )
                )
            })
    }));
    let continuation = continuation
        .expect("document.close should return an exact post-response lifecycle capability");

    let (reply, _) = page
        .run_async_command(RendererPageCommand::TakeDocumentLifecycleEvents)
        .await
        .expect("parked lifecycle facts should be inspectable");
    let RendererPageReply::DocumentLifecycleEvents(events_before_release) = reply else {
        panic!("unexpected parked lifecycle reply");
    };
    assert!(events_before_release.iter().all(|event| !matches!(
        event.kind,
        RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::DomContentLoaded
                | RendererDocumentLifecycleMilestone::Load
        )
    )));

    continuation.release();
    let reached_load = tokio::time::timeout(
        Duration::from_millis(500),
        recv_page_lifecycle_until(
            &mut activity_wake_rx,
            &page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("released capability should schedule the exact lifecycle resident");
    assert!(reached_load.iter().any(|event| {
        event.kind
            == RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::Load,
            )
    }));

    page.close_async()
        .await
        .expect("runtime command boundary test page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn load_target_observer_remains_pending_after_domcontentloaded() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (
        base_url,
        async_script_request_seen,
        domcontentloaded_request_seen,
        release_async_script_response,
        server,
    ) = spawn_owner_lifecycle_gated_async_server().await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let producer = tokio::spawn(async move {
        body_tx
            .send(
                br#"<!doctype html><body>
<script async src="/async.js"></script>
<script>
document.addEventListener("DOMContentLoaded", () => {
  fetch("/domcontentloaded-seen");
}, { once: true });
</script>
</body>"#
                    .to_vec(),
            )
            .await
            .expect("html chunk should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let creation = runtime.create_streaming_raw_page_from_external_body(
        page_url.clone(),
        page_url,
        None,
        false,
        0,
        Vec::new(),
        200,
        vec![("content-type".to_owned(), "text/html".to_owned())],
        &loader,
        crate::RendererWebStorageHandles::ephemeral(),
        raw_body,
        None,
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        None,
        None,
        false,
        1.0,
        Default::default(),
        None,
        false,
        Vec::new(),
        false,
        None,
        false,
        PageVmInitStage::Load,
        RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
        RendererNavigationReplyPolicy::FollowBeforeReply,
    );
    tokio::pin!(creation);

    tokio::select! {
        _result = &mut creation => {
            panic!("Load observer completed before its gated async script request")
        }
        seen = async_script_request_seen => {
            seen.expect("gated async script request channel should stay open");
        }
    }
    tokio::select! {
        _result = &mut creation => {
            panic!("Load observer completed before DOMContentLoaded was observable")
        }
        seen = domcontentloaded_request_seen => {
            seen.expect("DOMContentLoaded signal request channel should stay open");
        }
    }
    producer.await.expect("producer should finish");

    let (probe_tx, probe_rx) = oneshot::channel();
    probe_tx.send(()).expect("readiness probe should send");
    tokio::select! {
        biased;
        _result = &mut creation => {
            panic!("Load observer returned at DOMContentLoaded while async work was blocked")
        }
        _ = probe_rx => {}
    }

    release_async_script_response
        .send(())
        .expect("release gated async script response");
    let (mut page, _, _creation_diagnostics, creation_artifacts, pending_download) =
        tokio::time::timeout(Duration::from_secs(2), creation)
            .await
            .expect("Load observer should complete after the async script response")
            .expect("page should reach Load");
    assert!(pending_download.is_none());
    assert!(
        creation_artifacts
            .lifecycle_snapshot
            .dom_content_loaded
            .is_some()
    );
    assert!(creation_artifacts.lifecycle_snapshot.load.is_some());
    let (marker, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_load_target_async_marker ?? "pending""#.to_owned(),
            await_promise: false,
        })
        .await
        .expect("async marker should evaluate");
    assert_eq!(
        renderer_json_value(marker),
        Some(serde_json::json!("executed"))
    );

    page.close_async()
        .await
        .expect("Load observer test page should close");
    server.await.expect("lifecycle gated server should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn blocked_lifecycle_page_does_not_prevent_peer_page_load() {
    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, async_script_request_seen, release_async_script_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/blocked-page-async.js",
            "globalThis.__lm_blocked_page_async_marker = 'executed';",
            "application/javascript",
        )
        .await;
    let blocked_page_url =
        url::Url::parse(&format!("{base_url}/blocked-page")).expect("blocked page url");
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let producer = tokio::spawn(async move {
        body_tx
            .send(
                br#"<!doctype html><body>
<script async src="/blocked-page-async.js"></script>
blocked page
</body>"#
                    .to_vec(),
            )
            .await
            .expect("blocked page html chunk should send");
        drop(body_tx);
        completion_tx.send(Ok(())).expect("completion should send");
    });

    let blocked_creation = runtime.create_streaming_raw_page_from_external_body(
        blocked_page_url.clone(),
        blocked_page_url,
        None,
        false,
        0,
        Vec::new(),
        200,
        vec![("content-type".to_owned(), "text/html".to_owned())],
        &loader,
        crate::RendererWebStorageHandles::ephemeral(),
        raw_body,
        None,
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        None,
        None,
        false,
        1.0,
        Default::default(),
        None,
        false,
        Vec::new(),
        false,
        None,
        false,
        PageVmInitStage::Load,
        RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
        RendererNavigationReplyPolicy::FollowBeforeReply,
    );
    tokio::pin!(blocked_creation);
    tokio::select! {
        _result = &mut blocked_creation => {
            panic!("blocked page reached Load before its async response")
        }
        seen = async_script_request_seen => {
            seen.expect("blocked page async request channel should stay open");
        }
    }
    producer.await.expect("blocked page producer should finish");

    let peer_url = url::Url::parse("https://peer-page.test/ready").expect("peer page url");
    let mut peer_page = tokio::time::timeout(
        Duration::from_secs(2),
        create_test_html_page(
            &runtime,
            &loader,
            peer_url,
            r#"<!doctype html><script>globalThis.__lm_peer_page_marker = "loaded";</script>"#,
        ),
    )
    .await
    .expect("a blocked lifecycle page must not starve a peer page");
    let (peer_marker, _) = peer_page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lm_peer_page_marker".to_owned(),
            await_promise: false,
        })
        .await
        .expect("peer page marker should evaluate");
    assert_eq!(
        renderer_json_value(peer_marker),
        Some(serde_json::json!("loaded"))
    );

    let (probe_tx, probe_rx) = oneshot::channel();
    probe_tx.send(()).expect("readiness probe should send");
    tokio::select! {
        biased;
        _result = &mut blocked_creation => {
            panic!("blocked page completed while its producer was still gated")
        }
        _ = probe_rx => {}
    }

    release_async_script_response
        .send(())
        .expect("release blocked page async response");
    let (mut blocked_page, _, _, creation_artifacts, pending_download) =
        tokio::time::timeout(Duration::from_secs(2), blocked_creation)
            .await
            .expect("blocked page should resume from its producer wake")
            .expect("blocked page should reach Load");
    assert!(pending_download.is_none());
    assert!(creation_artifacts.lifecycle_snapshot.load.is_some());
    assert!(
        RendererPageTestingHandle::new_for_testing(&blocked_page)
            .shares_local_host(&RendererPageTestingHandle::new_for_testing(&peer_page)),
        "the isolation check must exercise two pages scheduled by the same owner-local host"
    );

    blocked_page
        .close_async()
        .await
        .expect("blocked lifecycle page should close");
    peer_page
        .close_async()
        .await
        .expect("peer lifecycle page should close");
    server.await.expect("blocked page server should finish");
}

fn renderer_json_value(reply: RendererPageReply) -> Option<serde_json::Value> {
    match reply {
        RendererPageReply::RuntimeEvaluationResult(result) => {
            result.into_protocol_payload().get("value").cloned()
        }
        _ => None,
    }
}

async fn assert_window_performance_surface_for_test(page: &RendererPageHandle, phase: &str) {
    let (surface, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify({
  sameObject: performance === performance,
  prototype: Object.getPrototypeOf(performance) === Performance.prototype,
  finiteTimeOrigin: Number.isFinite(performance.timeOrigin)
})"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .unwrap_or_else(|error| panic!("{phase} Performance surface should evaluate: {error}"));
    assert_eq!(
        renderer_json_value(surface),
        Some(serde_json::json!(
            r#"{"sameObject":true,"prototype":true,"finiteTimeOrigin":true}"#
        )),
        "{phase} must bind Performance to its realm's canonical Window"
    );
}

async fn runtime_heap_usage_for_test(page: &RendererPageHandle) -> serde_json::Value {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::RuntimeHeapUsage)
        .await
        .expect("runtime heap usage command should run");
    match reply {
        RendererPageReply::RuntimeHeapUsage(usage) => usage.to_diagnostics_json(),
        _ => panic!("runtime heap usage should return typed heap usage"),
    }
}

fn renderer_bool(reply: RendererPageReply) -> Option<bool> {
    match reply {
        RendererPageReply::Bool(value) => Some(value),
        _ => None,
    }
}

async fn has_pending_location_navigation_for_test(page: &RendererPageHandle) -> Option<bool> {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::HasPendingLocationNavigation)
        .await
        .expect("pending location navigation state should evaluate");
    renderer_bool(reply)
}

async fn store_indexed_db_value_for_test(
    page: &RendererPageHandle,
    loader: &ResourceRequestClient,
    value: &str,
) {
    let expression = format!(
        r#"
(() => {{
  globalThis.__lm_runtime_indexed_db_store = "pending";
  const open = indexedDB.open("shared-manager-db", 1);
  open.onerror = () => {{
    globalThis.__lm_runtime_indexed_db_store = `open-error:${{open.error && open.error.name}}`;
  }};
  open.onupgradeneeded = () => {{
    open.result.createObjectStore("kv");
  }};
  open.onsuccess = () => {{
    const db = open.result;
    const tx = db.transaction("kv", "readwrite");
    const put = tx.objectStore("kv").put({value:?}, "key");
    put.onerror = () => {{
      globalThis.__lm_runtime_indexed_db_store = `put-error:${{put.error && put.error.name}}`;
    }};
    tx.oncomplete = () => {{
      db.close();
      globalThis.__lm_runtime_indexed_db_store = "stored";
    }};
  }};
  return "scheduled";
}})()
"#
    );
    let (scheduled, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression,
            await_promise: false,
        })
        .await
        .expect("indexedDB store should schedule");
    assert_eq!(
        renderer_json_value(scheduled),
        Some(serde_json::json!("scheduled"))
    );
    page.run_async_command(RendererPageCommand::WaitForScriptTruthy {
        expression: r#"globalThis.__lm_runtime_indexed_db_store === "stored""#.to_owned(),
        timeout_ms: 2_000,
        loader: loader.clone(),
    })
    .await
    .expect("indexedDB store should complete");
}

fn runtime_indexed_db_test_root(label: &str) -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "moli-runtime-indexeddb-{label}-{}-{nonce}",
        std::process::id()
    ))
}

fn runtime_indexed_db_origin_file(root: &std::path::Path, origin: &str) -> std::path::PathBuf {
    let mut encoded = String::with_capacity(origin.len() * 2);
    for byte in origin.as_bytes() {
        use std::fmt::Write;
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    if encoded.len() > 180 {
        encoded = moli_crypto::sha256_hex(origin.as_bytes());
        encoded.insert_str(0, "h-");
    }
    root.join(format!("{encoded}.json"))
}

async fn create_isolated_world_for_test(
    page: &RendererPageHandle,
    name: &str,
) -> anyhow::Result<i64> {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::CreateIsolatedWorld {
            name: name.to_owned(),
            grant_universal_access: false,
            frame_id: None,
        })
        .await?;
    match reply {
        RendererPageReply::ExecutionContextId(execution_context_id) => Ok(execution_context_id),
        _ => Err(anyhow::anyhow!(
            "expected CreateIsolatedWorld to return an execution context id"
        )),
    }
}

async fn create_isolated_world_runtime_activity_for_test(
    page: &RendererPageHandle,
    inspector_session_id: Option<&str>,
    name: &str,
) -> anyhow::Result<i64> {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::CreateIsolatedWorldRuntimeActivity {
            inspector_session_id: inspector_session_id.map(str::to_owned),
            frame_id: None,
            name: name.to_owned(),
            grant_universal_access: false,
        })
        .await?;
    match reply {
        RendererPageReply::ExecutionContextId(execution_context_id) => Ok(execution_context_id),
        _ => Err(anyhow::anyhow!(
            "expected CreateIsolatedWorldRuntimeActivity to return an execution context id"
        )),
    }
}

async fn create_isolated_world_for_frame_for_test(
    page: &RendererPageHandle,
    frame_id: &str,
    name: &str,
) -> anyhow::Result<i64> {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::CreateIsolatedWorld {
            name: name.to_owned(),
            grant_universal_access: false,
            frame_id: Some(frame_id.to_owned()),
        })
        .await?;
    match reply {
        RendererPageReply::ExecutionContextId(execution_context_id) => Ok(execution_context_id),
        _ => Err(anyhow::anyhow!(
            "expected frame CreateIsolatedWorld to return an execution context id"
        )),
    }
}

async fn default_or_initial_execution_context_id_for_test(
    page: &RendererPageHandle,
) -> anyhow::Result<Option<i64>> {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::DefaultOrInitialExecutionContextId)
        .await?;
    match reply {
        RendererPageReply::OptionalExecutionContextId(execution_context_id) => {
            Ok(execution_context_id)
        }
        _ => Err(anyhow::anyhow!(
            "expected DefaultOrInitialExecutionContextId to return an optional execution context id"
        )),
    }
}

async fn default_execution_context_id_for_test(
    page: &RendererPageHandle,
) -> anyhow::Result<Option<i64>> {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::DefaultExecutionContextId)
        .await?;
    match reply {
        RendererPageReply::OptionalExecutionContextId(execution_context_id) => {
            Ok(execution_context_id)
        }
        _ => Err(anyhow::anyhow!(
            "expected DefaultExecutionContextId to return an optional execution context id"
        )),
    }
}

async fn child_frame_id_for_default_context_id_for_test(
    page: &RendererPageHandle,
    execution_context_id: i64,
) -> anyhow::Result<String> {
    let (reply, _) = page
        .run_async_command(
            RendererPageCommand::ChildFrameIdForDefaultExecutionContextId(execution_context_id),
        )
        .await?;
    match reply {
        RendererPageReply::OptionalString(Some(frame_id)) => Ok(frame_id),
        _ => Err(anyhow::anyhow!(
            "expected child frame id lookup to return an optional string"
        )),
    }
}

async fn child_default_context_ids_for_test(page: &RendererPageHandle) -> anyhow::Result<Vec<i64>> {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::LiveChildDefaultRuntimeRealmInventory)
        .await?;
    match reply {
        RendererPageReply::RuntimeRealmInventory(realms) => {
            Ok(realms.into_iter().map(|realm| realm.context_id).collect())
        }
        _ => Err(anyhow::anyhow!(
            "expected child default context replay to return runtime realm inventory"
        )),
    }
}

async fn create_test_html_page(
    runtime: &JsRuntime,
    loader: &ResourceRequestClient,
    url: url::Url,
    html: &str,
) -> RendererPageHandle {
    create_test_html_page_with_optional_indexed_db_manager(runtime, loader, url, html, None).await
}

async fn create_test_html_page_with_indexed_db_manager(
    runtime: &JsRuntime,
    loader: &ResourceRequestClient,
    url: url::Url,
    html: &str,
    indexed_db_manager: &crate::SharedIndexedDbManager,
) -> RendererPageHandle {
    create_test_html_page_with_optional_indexed_db_manager(
        runtime,
        loader,
        url,
        html,
        Some(crate::downgrade_indexed_db_manager(indexed_db_manager)),
    )
    .await
}

async fn create_test_html_page_with_optional_indexed_db_manager(
    runtime: &JsRuntime,
    loader: &ResourceRequestClient,
    url: url::Url,
    html: &str,
    indexed_db_manager: Option<crate::WeakIndexedDbManager>,
) -> RendererPageHandle {
    create_test_html_page_with_optional_indexed_db_manager_and_navigation_dispatch(
        runtime,
        loader,
        url,
        html,
        indexed_db_manager,
        RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
    )
    .await
}

async fn create_test_html_page_with_navigation_dispatch(
    runtime: &JsRuntime,
    loader: &ResourceRequestClient,
    url: url::Url,
    html: &str,
    top_level_navigation_dispatch: RendererTopLevelNavigationDispatch,
) -> RendererPageHandle {
    create_test_html_page_with_optional_indexed_db_manager_and_navigation_dispatch(
        runtime,
        loader,
        url,
        html,
        None,
        top_level_navigation_dispatch,
    )
    .await
}

async fn create_test_html_page_with_optional_indexed_db_manager_and_navigation_dispatch(
    runtime: &JsRuntime,
    loader: &ResourceRequestClient,
    url: url::Url,
    html: &str,
    indexed_db_manager: Option<crate::WeakIndexedDbManager>,
    top_level_navigation_dispatch: RendererTopLevelNavigationDispatch,
) -> RendererPageHandle {
    let pending = start_test_html_page_with_optional_indexed_db_manager_and_navigation_dispatch(
        runtime,
        loader,
        url,
        html,
        indexed_db_manager,
        top_level_navigation_dispatch,
    );
    let (page, _, _, _creation_artifacts, pending_download) = pending
        .await_ready()
        .await
        .expect("test HTML page should load");
    assert!(pending_download.is_none());
    page
}

fn start_test_html_page_with_optional_indexed_db_manager_and_navigation_dispatch(
    runtime: &JsRuntime,
    loader: &ResourceRequestClient,
    url: url::Url,
    html: &str,
    indexed_db_manager: Option<crate::WeakIndexedDbManager>,
    top_level_navigation_dispatch: RendererTopLevelNavigationDispatch,
) -> super::PendingHtmlPage {
    runtime
        .start_create_html_page_from_response_with_inspector_session_restores(
            runtime.reserve_page_for_creation(),
            url.clone(),
            url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            loader,
            crate::RendererWebStorageHandles::ephemeral(),
            html.to_owned(),
            indexed_db_manager,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
            Vec::new(),
            None,
            None,
            top_level_navigation_dispatch,
            None,
        )
        .expect("test HTML page should start")
}

async fn create_test_html_page_at_document_commit(
    runtime: &JsRuntime,
    loader: &ResourceRequestClient,
    url: url::Url,
    html: &str,
) -> RendererPageHandle {
    create_test_html_page_at_document_commit_with_navigation_dispatch(
        runtime,
        loader,
        url,
        html,
        RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
        RendererNavigationReplyPolicy::FollowBeforeReply,
    )
    .await
}

async fn create_test_html_page_at_document_commit_with_navigation_dispatch(
    runtime: &JsRuntime,
    loader: &ResourceRequestClient,
    url: url::Url,
    html: &str,
    top_level_navigation_dispatch: RendererTopLevelNavigationDispatch,
    navigation_reply_policy: RendererNavigationReplyPolicy,
) -> RendererPageHandle {
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let html = html.as_bytes().to_vec();
    let producer = tokio::spawn(async move {
        body_tx
            .send(html)
            .await
            .expect("document-commit HTML body should send");
        drop(body_tx);
        completion_tx
            .send(Ok(()))
            .expect("document-commit HTML body should complete");
    });
    let (mut page, _, _, creation_artifacts, pending_download) = runtime
        .create_streaming_raw_page_from_external_body_with_inspector_session_restores(
            url.clone(),
            url,
            None,
            false,
            0,
            Vec::new(),
            200,
            vec![("content-type".to_owned(), "text/html".to_owned())],
            loader,
            crate::RendererWebStorageHandles::ephemeral(),
            raw_body,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            false,
            1.0,
            Default::default(),
            None,
            false,
            Vec::new(),
            false,
            None,
            Vec::new(),
            false,
            PageVmInitStage::Load,
            RendererReplyBoundary::DocumentCommit,
            top_level_navigation_dispatch,
            navigation_reply_policy,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("test HTML page should attach at document commit");
    producer
        .await
        .expect("document-commit HTML producer should finish");
    assert!(pending_download.is_none());
    assert!(
        creation_artifacts.lifecycle_snapshot.load.is_none(),
        "document-commit fixture must return before load"
    );
    page.take_committed_document_post_response_continuation()
        .expect("DocumentCommit should defer parser continuation")
        .release();
    page
}

async fn install_shared_worker_count_probe(
    page: &RendererPageHandle,
    name: &str,
) -> anyhow::Result<()> {
    let source_literal =
        serde_json::to_string(SHARED_WORKER_CONNECTION_COUNT_SOURCE).expect("serialize source");
    let name_literal = serde_json::to_string(name).expect("serialize shared worker name");
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!(
                r#"
(() => {{
  globalThis.__lm_shared_worker_probe_messages = [];
  globalThis.__lm_shared_worker_probe = new SharedWorker(
    "data:text/javascript," + encodeURIComponent({source_literal}),
    {name_literal}
  );
  globalThis.__lm_shared_worker_probe.port.addEventListener("message", event => {{
    globalThis.__lm_shared_worker_probe_messages.push(String(event.data));
  }});
  globalThis.__lm_shared_worker_probe.port.start();
  return "installed";
}})()
"#
            ),
            await_promise: false,
        })
        .await?;
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("installed"))
    );
    Ok(())
}

async fn request_shared_worker_probe_count(page: &RendererPageHandle) -> anyhow::Result<()> {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lm_shared_worker_probe.port.postMessage("count");
  return "posted";
})()"#
                .to_owned(),
            await_promise: false,
        })
        .await?;
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("posted"))
    );
    Ok(())
}

async fn wait_for_shared_worker_probe_messages(
    page: &RendererPageHandle,
    loader: &ResourceRequestClient,
    expected_count: usize,
    context: &str,
) -> anyhow::Result<()> {
    let expression =
        format!("globalThis.__lm_shared_worker_probe_messages?.length >= {expected_count}");
    tokio::time::timeout(
        Duration::from_secs(5),
        page.run_async_command(RendererPageCommand::WaitForScriptTruthy {
            expression,
            timeout_ms: 5_000,
            loader: loader.clone(),
        }),
    )
    .await
    .map_err(|_| anyhow::anyhow!("{context}: timed out waiting for SharedWorker message"))?
    .map_err(|error| anyhow::anyhow!("{context}: SharedWorker message wait failed: {error}"))?;
    Ok(())
}

async fn shared_worker_probe_messages(page: &RendererPageHandle) -> anyhow::Result<String> {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"globalThis.__lm_shared_worker_probe_messages.join("|")"#.to_owned(),
            await_promise: false,
        })
        .await?;
    renderer_json_value(reply)
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| anyhow::anyhow!("expected SharedWorker probe messages string"))
}

async fn runtime_protocol_object_id(
    page: &RendererPageHandle,
    request: serde_json::Value,
    response_id: i64,
) -> anyhow::Result<String> {
    let messages = dispatch_runtime_protocol_for_test(page, request).await?;
    runtime_protocol_response_by_id(&messages, response_id)
        .and_then(|response| response["result"]["result"]["objectId"].as_str())
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("expected Runtime response {response_id} to carry objectId"))
}

async fn dispatch_runtime_protocol_for_test(
    page: &RendererPageHandle,
    request: serde_json::Value,
) -> anyhow::Result<Vec<serde_json::Value>> {
    dispatch_runtime_protocol_with_output_for_test(page, request)
        .await
        .map(|(messages, _)| messages)
}

async fn dispatch_runtime_protocol_with_output_for_test(
    page: &RendererPageHandle,
    request: serde_json::Value,
) -> anyhow::Result<(Vec<serde_json::Value>, RendererRuntimeCommandOutput)> {
    let raw_json = serde_json::to_string(&request)?;
    let output = page
        .enqueue_async_command(RendererPageCommand::dispatch_runtime_protocol_message(
            None, raw_json,
        ))
        .expect("runtime protocol command should enqueue")
        .wait()
        .await?;
    let (completion, _) = output.into_completion_and_predecessor();
    let output = completion.into_runtime_inspector_output().ok_or_else(|| {
        anyhow::anyhow!("expected Runtime protocol dispatch to return inspector protocol messages")
    })?;
    let messages = output
        .messages()
        .iter()
        .cloned()
        .map(RendererRuntimeInspectorMessage::into_v8_inspector_message)
        .collect();
    Ok((messages, output))
}

async fn dispatch_runtime_protocol_with_context_resolution_for_test(
    page: &RendererPageHandle,
    action: &str,
    request: serde_json::Value,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let raw_json = serde_json::to_string(&request)?;
    let (reply, _) = page
        .run_async_command(
            RendererPageCommand::dispatch_runtime_protocol_message_with_context_resolution(
                None,
                action.to_owned(),
                raw_json,
            ),
        )
        .await?;
    match reply {
        RendererPageReply::RuntimeInspectorProtocolMessages(messages) => Ok(messages
            .into_messages()
            .into_iter()
            .map(RendererRuntimeInspectorMessage::into_v8_inspector_message)
            .collect()),
        _ => Err(anyhow::anyhow!(
            "expected Runtime protocol dispatch with context resolution to return inspector protocol messages"
        )),
    }
}

fn runtime_protocol_response_by_id(
    messages: &[serde_json::Value],
    response_id: i64,
) -> Option<&serde_json::Value> {
    messages
        .iter()
        .find(|message| message.get("id").and_then(serde_json::Value::as_i64) == Some(response_id))
}

async fn runtime_enable_events_for_test(
    page: &RendererPageHandle,
) -> anyhow::Result<Vec<serde_json::Value>> {
    runtime_enable_events_for_inspector_session_for_test(page, None).await
}

async fn runtime_enable_events_for_inspector_session_for_test(
    page: &RendererPageHandle,
    inspector_session_id: Option<&str>,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::runtime_enable_events(
            inspector_session_id.map(str::to_owned),
        ))
        .await?;
    match reply {
        RendererPageReply::RuntimeInspectorProtocolMessages(output) => Ok(output
            .into_messages()
            .into_iter()
            .map(runtime_inspector_message_protocol_message_for_test)
            .collect()),
        _ => Err(anyhow::anyhow!(
            "expected RuntimeEnableEvents to return Runtime inspector messages"
        )),
    }
}

fn runtime_inspector_message_protocol_message_for_test(
    message: RendererRuntimeInspectorMessage,
) -> serde_json::Value {
    match message {
        RendererRuntimeInspectorMessage::Protocol(message) => message.into_value(),
        RendererRuntimeInspectorMessage::RuntimeContext(event) => {
            runtime_context_restore_event_protocol_message_for_test(event)
        }
    }
}

fn runtime_context_restore_event_protocol_message_for_test(
    event: crate::protocol_types::RuntimeContextRestoreEvent,
) -> serde_json::Value {
    match event {
        crate::protocol_types::RuntimeContextRestoreEvent::Created(event) => {
            let crate::protocol_types::RuntimeExecutionContextRestoreEvent {
                context_id,
                realm_id,
                frame_id,
                origin,
                name,
                is_default,
                context_type,
                grant_universal_access,
            } = event;
            let mut aux_data = serde_json::Map::new();
            if let Some(frame_id) = frame_id {
                aux_data.insert("frameId".to_owned(), serde_json::json!(frame_id));
            }
            aux_data.insert("isDefault".to_owned(), serde_json::json!(is_default));
            aux_data.insert("type".to_owned(), serde_json::json!(context_type));
            if let Some(grant_universal_access) = grant_universal_access {
                aux_data.insert(
                    "grantUniversalAccess".to_owned(),
                    serde_json::json!(grant_universal_access),
                );
            }
            serde_json::json!({
                "method": "Runtime.executionContextCreated",
                "params": {
                    "context": {
                        "id": context_id,
                        "uniqueId": realm_id,
                        "origin": origin,
                        "name": name,
                        "auxData": serde_json::Value::Object(aux_data),
                    },
                },
            })
        }
        crate::protocol_types::RuntimeContextRestoreEvent::Destroyed(event) => {
            let crate::protocol_types::RuntimeExecutionContextRestoreEvent {
                context_id,
                realm_id,
                ..
            } = event;
            serde_json::json!({
                "method": "Runtime.executionContextDestroyed",
                "params": {
                    "executionContextId": context_id,
                    "executionContextUniqueId": realm_id,
                },
            })
        }
        crate::protocol_types::RuntimeContextRestoreEvent::Cleared(_event) => {
            serde_json::json!({
                "method": "Runtime.executionContextsCleared",
                "params": {},
            })
        }
    }
}

fn runtime_execution_context_ids(messages: &[serde_json::Value]) -> Vec<i64> {
    messages
        .iter()
        .filter(|message| {
            message.get("method") == Some(&serde_json::json!("Runtime.executionContextCreated"))
        })
        .filter_map(|message| message["params"]["context"]["id"].as_i64())
        .collect()
}

fn runtime_execution_context_unique_ids(messages: &[serde_json::Value]) -> Vec<&str> {
    messages
        .iter()
        .filter(|message| {
            message.get("method") == Some(&serde_json::json!("Runtime.executionContextCreated"))
        })
        .filter_map(|message| message["params"]["context"]["uniqueId"].as_str())
        .collect()
}

fn runtime_execution_context_by_id(
    messages: &[serde_json::Value],
    context_id: i64,
) -> Option<&serde_json::Value> {
    messages
        .iter()
        .filter(|message| {
            message.get("method") == Some(&serde_json::json!("Runtime.executionContextCreated"))
        })
        .map(|message| &message["params"]["context"])
        .find(|context| context["id"].as_i64() == Some(context_id))
}

fn runtime_default_context_count(messages: &[serde_json::Value]) -> usize {
    messages
        .iter()
        .filter(|message| {
            message.get("method") == Some(&serde_json::json!("Runtime.executionContextCreated"))
                && message["params"]["context"]["auxData"]["isDefault"] == serde_json::json!(true)
                && message["params"]["context"]["auxData"]["type"] == serde_json::json!("default")
                && message["params"]["context"]["auxData"]["frameId"]
                    .as_str()
                    .is_none()
        })
        .count()
}

async fn add_runtime_binding_for_test(
    page: &RendererPageHandle,
    name: &str,
    execution_context_name: Option<&str>,
    execution_context_id: Option<i64>,
) -> anyhow::Result<()> {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::add_runtime_binding(
            None,
            name.to_owned(),
            execution_context_name.map(str::to_owned),
            execution_context_id,
        ))
        .await?;
    match reply {
        RendererPageReply::Unit => Ok(()),
        _ => Err(anyhow::anyhow!(
            "expected AddRuntimeBinding to return unit reply"
        )),
    }
}

async fn remove_runtime_binding_for_test(
    page: &RendererPageHandle,
    name: &str,
) -> anyhow::Result<()> {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::RemoveRuntimeBinding(name.to_owned()))
        .await?;
    match reply {
        RendererPageReply::Unit => Ok(()),
        _ => Err(anyhow::anyhow!(
            "expected RemoveRuntimeBinding to return unit reply"
        )),
    }
}

async fn set_stored_document_start_scripts_for_test(
    page: &RendererPageHandle,
    scripts: Vec<crate::DocumentStartScript>,
) -> anyhow::Result<()> {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::SetStoredDocumentStartScripts(scripts))
        .await?;
    match reply {
        RendererPageReply::Unit => Ok(()),
        _ => Err(anyhow::anyhow!(
            "expected SetStoredDocumentStartScripts to return unit reply"
        )),
    }
}

async fn set_runtime_binding_state_for_test(
    page: &RendererPageHandle,
    inspector_session_id: Option<String>,
    stored_runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
    session_runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
) -> anyhow::Result<()> {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::SetRuntimeBindingState {
            inspector_session_id,
            stored_runtime_bindings,
            session_runtime_bindings,
        })
        .await?;
    match reply {
        RendererPageReply::Unit => Ok(()),
        _ => Err(anyhow::anyhow!(
            "expected SetRuntimeBindingState to return unit reply"
        )),
    }
}

const SHARED_WORKER_CONNECTION_COUNT_SOURCE: &str = r#"
let connections = 0;
onconnect = (event) => {
  connections++;
  const port = event.ports[0];
  port.onmessage = () => {
    port.postMessage(String(connections));
  };
  port.postMessage(String(connections));
};
"#;

async fn spawn_owner_service_worker_response_sequence(
    responses: Vec<(&'static str, &'static str, &'static str)>,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind owner ServiceWorker response sequence server");
    let addr = listener
        .local_addr()
        .expect("owner ServiceWorker response sequence server address");
    let server = tokio::spawn(async move {
        for (expected_path, content_type, body) in responses {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept owner ServiceWorker response sequence request");
            let request = read_owner_wake_http_request_head(&mut stream).await;
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("owner ServiceWorker response sequence request path");
            assert_eq!(path, expected_path);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write owner ServiceWorker response sequence response");
        }
    });
    (format!("http://{addr}"), server)
}

async fn spawn_owner_wake_server_with_content_type(
    expected_path: &'static str,
    body: &'static str,
    content_type: &'static str,
    delay: Duration,
) -> (String, tokio::task::JoinHandle<()>) {
    spawn_owner_wake_server_with_content_type_and_headers(
        expected_path,
        body,
        content_type,
        Vec::new(),
        delay,
    )
    .await
}

async fn spawn_owner_wake_server_with_content_type_and_headers(
    expected_path: &'static str,
    body: &'static str,
    content_type: &'static str,
    response_headers: Vec<(&'static str, &'static str)>,
    delay: Duration,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind owner wake fetch server");
    let addr = listener.local_addr().expect("server local addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept owner wake fetch request");
        let request = read_owner_wake_http_request_head(&mut stream).await;
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("request path");
        assert_eq!(path, expected_path);
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        let response_headers = response_headers
            .into_iter()
            .map(|(name, value)| format!("{name}: {value}\r\n"))
            .collect::<String>();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\n{response_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write owner wake fetch response");
    });
    (format!("http://{addr}"), server)
}

async fn spawn_owner_wake_gated_binary_server_with_content_type(
    expected_path: &'static str,
    body: Vec<u8>,
    content_type: &'static str,
) -> (
    String,
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind owner wake gated binary fetch server");
    let addr = listener.local_addr().expect("server local addr");
    let (request_seen_tx, request_seen_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept owner wake gated binary fetch request");
        let request = read_owner_wake_http_request_head(&mut stream).await;
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("request path");
        assert_eq!(path, expected_path);
        request_seen_tx
            .send(())
            .expect("signal owner wake gated binary request seen");
        release_rx
            .await
            .expect("wait for owner wake gated binary response release");
        let response_head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(response_head.as_bytes())
            .await
            .expect("write owner wake binary fetch response head");
        stream
            .write_all(&body)
            .await
            .expect("write owner wake binary fetch response body");
    });
    (
        format!("http://{addr}"),
        request_seen_rx,
        release_tx,
        server,
    )
}

struct OwnerChildModuleGraphServer {
    base_url: String,
    root_request_seen: oneshot::Receiver<()>,
    release_root_response: oneshot::Sender<()>,
    dependency_request_seen: oneshot::Receiver<()>,
    release_dependency_response: oneshot::Sender<()>,
    effect_request_seen: oneshot::Receiver<()>,
    task: tokio::task::JoinHandle<()>,
}

struct OwnerMainModuleLivenessServer {
    base_url: String,
    module_request_seen: oneshot::Receiver<()>,
    release_module_response: oneshot::Sender<()>,
    effect_request_seen: oneshot::Receiver<()>,
    task: tokio::task::JoinHandle<()>,
}

struct OwnerMainModuleReactionLivenessServer {
    base_url: String,
    module_request_seen: oneshot::Receiver<()>,
    release_module_response: oneshot::Sender<()>,
    evaluation_started: oneshot::Receiver<()>,
    effect_request_seen: oneshot::Receiver<()>,
    script_load_event_seen: oneshot::Receiver<()>,
    task: tokio::task::JoinHandle<()>,
}

struct OwnerInlineModuleReactionLivenessServer {
    base_url: String,
    evaluation_started: oneshot::Receiver<()>,
    effect_request_seen: oneshot::Receiver<()>,
    task: tokio::task::JoinHandle<()>,
}

struct OwnerChildDocumentLivenessServer {
    base_url: String,
    document_request_seen: oneshot::Receiver<()>,
    release_document_response: oneshot::Sender<()>,
    effect_request_seen: oneshot::Receiver<()>,
    task: tokio::task::JoinHandle<()>,
}

struct OwnerChildClassicLivenessServer {
    base_url: String,
    source_request_seen: oneshot::Receiver<()>,
    release_source_response: oneshot::Sender<()>,
    effect_request_seen: oneshot::Receiver<()>,
    task: tokio::task::JoinHandle<()>,
}

struct OwnerModulepreloadLivenessServer {
    base_url: String,
    module_request_seen: oneshot::Receiver<()>,
    release_module_response: oneshot::Sender<()>,
    effect_request_seen: oneshot::Receiver<()>,
    task: tokio::task::JoinHandle<()>,
}

struct OwnerDynamicImportLivenessServer {
    base_url: String,
    dynamic_root_request_seen: oneshot::Receiver<()>,
    release_dynamic_root_response: oneshot::Sender<()>,
    effect_request_seen: oneshot::Receiver<()>,
    task: tokio::task::JoinHandle<()>,
}

async fn spawn_owner_child_document_liveness_server() -> OwnerChildDocumentLivenessServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind owner child-document liveness server");
    let addr = listener
        .local_addr()
        .expect("child-document liveness server address");
    let (document_request_seen_tx, document_request_seen) = oneshot::channel();
    let (release_document_response, release_document_response_rx) = oneshot::channel();
    let (effect_request_seen_tx, effect_request_seen) = oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut document_stream, _) = listener
            .accept()
            .await
            .expect("accept child document request");
        let document_request = read_owner_wake_http_request_head(&mut document_stream).await;
        let document_path = document_request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("child document request path");
        assert_eq!(document_path, "/owner-child-document.html");
        document_request_seen_tx
            .send(())
            .expect("signal child document request");
        release_document_response_rx
            .await
            .expect("wait for child document response release");
        let document_body = r#"<!doctype html><script>
parent.__lm_owner_child_document = "committed";
fetch("/owner-child-document-effect");
</script>"#;
        let document_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            document_body.len(),
            document_body,
        );
        document_stream
            .write_all(document_response.as_bytes())
            .await
            .expect("write child document response");

        let (mut effect_stream, _) = listener
            .accept()
            .await
            .expect("accept child document effect request");
        let effect_request = read_owner_wake_http_request_head(&mut effect_stream).await;
        let effect_path = effect_request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("child document effect path");
        assert_eq!(effect_path, "/owner-child-document-effect");
        effect_request_seen_tx
            .send(())
            .expect("signal child document effect request");
        effect_stream
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .expect("write child document effect response");
    });
    OwnerChildDocumentLivenessServer {
        base_url: format!("http://{addr}"),
        document_request_seen,
        release_document_response,
        effect_request_seen,
        task,
    }
}

async fn spawn_owner_child_classic_liveness_server() -> OwnerChildClassicLivenessServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind owner child-classic liveness server");
    let addr = listener
        .local_addr()
        .expect("child-classic liveness server address");
    let (source_request_seen_tx, source_request_seen) = oneshot::channel();
    let (release_source_response, release_source_response_rx) = oneshot::channel();
    let (effect_request_seen_tx, effect_request_seen) = oneshot::channel();
    let task = tokio::spawn(async move {
        let mut source_request_seen_tx = Some(source_request_seen_tx);
        let mut release_source_response_rx = Some(release_source_response_rx);
        let mut effect_request_seen_tx = Some(effect_request_seen_tx);
        let mut served_source = false;
        for _ in 0..2 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept owner child-classic request");
            let request = read_owner_wake_http_request_head(&mut stream).await;
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("child-classic request path");
            let (status, content_type, body) = match path {
                "/owner-child-classic.js" => {
                    assert!(!served_source, "child classic source must be fetched once");
                    served_source = true;
                    source_request_seen_tx
                        .take()
                        .expect("child classic source request should occur once")
                        .send(())
                        .expect("signal child classic source request");
                    release_source_response_rx
                        .take()
                        .expect("child classic source response gate should be consumed once")
                        .await
                        .expect("wait for child classic source response release");
                    (
                        "200 OK",
                        "application/javascript",
                        r#"parent.__lm_owner_child_classic = "executed";
fetch("/owner-child-classic-effect");"#,
                    )
                }
                "/owner-child-classic-effect" => {
                    assert!(served_source, "classic effect must follow source fetch");
                    effect_request_seen_tx
                        .take()
                        .expect("child classic effect request should occur once")
                        .send(())
                        .expect("signal child classic effect request");
                    ("204 No Content", "text/plain", "")
                }
                path => panic!("unexpected child classic request path: {path}"),
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write owner child-classic response");
        }
        assert!(
            served_source,
            "child classic source response must be served"
        );
    });
    OwnerChildClassicLivenessServer {
        base_url: format!("http://{addr}"),
        source_request_seen,
        release_source_response,
        effect_request_seen,
        task,
    }
}

async fn spawn_owner_dynamic_import_liveness_server() -> OwnerDynamicImportLivenessServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind owner dynamic-import liveness server");
    let addr = listener
        .local_addr()
        .expect("dynamic-import liveness server address");
    let (dynamic_root_request_seen_tx, dynamic_root_request_seen) = oneshot::channel();
    let (release_dynamic_root_response, release_dynamic_root_response_rx) = oneshot::channel();
    let (effect_request_seen_tx, effect_request_seen) = oneshot::channel();
    let task = tokio::spawn(async move {
        let mut dynamic_root_request_seen_tx = Some(dynamic_root_request_seen_tx);
        let mut release_dynamic_root_response_rx = Some(release_dynamic_root_response_rx);
        let mut effect_request_seen_tx = Some(effect_request_seen_tx);
        let mut served_paths = HashSet::new();
        for _ in 0..5 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept owner dynamic-import request");
            let request = read_owner_wake_http_request_head(&mut stream).await;
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("dynamic-import request path");
            assert!(
                served_paths.insert(path.to_owned()),
                "dynamic-import fixture must request each resource once: {path}"
            );
            let (status, content_type, body) = match path {
                "/dynamic-owner-entry.js" => (
                    "200 OK",
                    "application/javascript",
                    r#"import("./dynamic-owner-root.js").then(
  ({ total }) => {
    parent.__lm_dynamic_owner_liveness = "fulfilled:" + String(total);
    fetch("/dynamic-owner-effect");
  },
  () => { parent.__lm_dynamic_owner_liveness = "rejected"; }
);"#,
                ),
                "/dynamic-owner-root.js" => {
                    dynamic_root_request_seen_tx
                        .take()
                        .expect("dynamic root request should occur once")
                        .send(())
                        .expect("signal dynamic root request");
                    release_dynamic_root_response_rx
                        .take()
                        .expect("dynamic root response gate should be consumed once")
                        .await
                        .expect("wait for dynamic root response release");
                    (
                        "200 OK",
                        "application/javascript",
                        r#"import { left } from "./dynamic-owner-left.js";
import { right } from "./dynamic-owner-right.js";
export const total = left + right;"#,
                    )
                }
                "/dynamic-owner-left.js" => (
                    "200 OK",
                    "application/javascript",
                    "export const left = 19;",
                ),
                "/dynamic-owner-right.js" => (
                    "200 OK",
                    "application/javascript",
                    "export const right = 23;",
                ),
                "/dynamic-owner-effect" => {
                    effect_request_seen_tx
                        .take()
                        .expect("dynamic-import effect should occur once")
                        .send(())
                        .expect("signal dynamic-import effect request");
                    ("204 No Content", "text/plain", "")
                }
                path => panic!("unexpected dynamic-import liveness request path: {path}"),
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write owner dynamic-import response");
        }
        assert_eq!(served_paths.len(), 5);
    });
    OwnerDynamicImportLivenessServer {
        base_url: format!("http://{addr}"),
        dynamic_root_request_seen,
        release_dynamic_root_response,
        effect_request_seen,
        task,
    }
}

async fn spawn_owner_modulepreload_liveness_server(
    module_path: &'static str,
    module_body: &'static str,
    effect_path: &'static str,
) -> OwnerModulepreloadLivenessServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind owner modulepreload liveness server");
    let addr = listener
        .local_addr()
        .expect("modulepreload liveness server address");
    let (module_request_seen_tx, module_request_seen) = oneshot::channel();
    let (release_module_response, release_module_response_rx) = oneshot::channel();
    let (effect_request_seen_tx, effect_request_seen) = oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut module_stream, _) = listener
            .accept()
            .await
            .expect("accept modulepreload request");
        let module_request = read_owner_wake_http_request_head(&mut module_stream).await;
        let observed_module_path = module_request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("modulepreload request path");
        assert_eq!(
            observed_module_path, module_path,
            "the modulepreload fetch must be the first externally visible request"
        );
        module_request_seen_tx
            .send(())
            .expect("signal modulepreload request");
        release_module_response_rx
            .await
            .expect("wait for modulepreload response release");
        let module_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            module_body.len(),
            module_body
        );
        module_stream
            .write_all(module_response.as_bytes())
            .await
            .expect("write modulepreload response");

        let (mut effect_stream, _) = listener
            .accept()
            .await
            .expect("accept modulepreload effect request");
        let effect_request = read_owner_wake_http_request_head(&mut effect_stream).await;
        let observed_effect_path = effect_request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("modulepreload effect request path");
        assert_eq!(observed_effect_path, effect_path);
        effect_request_seen_tx
            .send(())
            .expect("signal modulepreload effect request");
        effect_stream
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .expect("write modulepreload effect response");
    });
    OwnerModulepreloadLivenessServer {
        base_url: format!("http://{addr}"),
        module_request_seen,
        release_module_response,
        effect_request_seen,
        task,
    }
}

async fn spawn_owner_main_module_liveness_server(
    module_path: &'static str,
    module_body: &'static str,
    effect_path: &'static str,
) -> OwnerMainModuleLivenessServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind owner main module server");
    let addr = listener.local_addr().expect("main module server address");
    let (module_request_seen_tx, module_request_seen) = oneshot::channel();
    let (release_module_response, release_module_response_rx) = oneshot::channel();
    let (effect_request_seen_tx, effect_request_seen) = oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut module_stream, _) = listener.accept().await.expect("accept main module request");
        let module_request = read_owner_wake_http_request_head(&mut module_stream).await;
        let observed_module_path = module_request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("main module request path");
        assert_eq!(observed_module_path, module_path);
        module_request_seen_tx
            .send(())
            .expect("signal main module request");
        release_module_response_rx
            .await
            .expect("wait for main module response release");
        let module_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            module_body.len(),
            module_body
        );
        module_stream
            .write_all(module_response.as_bytes())
            .await
            .expect("write main module response");

        let (mut effect_stream, _) = listener
            .accept()
            .await
            .expect("accept main module effect request");
        let effect_request = read_owner_wake_http_request_head(&mut effect_stream).await;
        let observed_effect_path = effect_request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("main module effect request path");
        assert_eq!(observed_effect_path, effect_path);
        effect_request_seen_tx
            .send(())
            .expect("signal main module effect request");
        effect_stream
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .expect("write main module effect response");
    });
    OwnerMainModuleLivenessServer {
        base_url: format!("http://{addr}"),
        module_request_seen,
        release_module_response,
        effect_request_seen,
        task,
    }
}

async fn spawn_owner_main_module_reaction_liveness_server() -> OwnerMainModuleReactionLivenessServer
{
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind owner main module-reaction server");
    let addr = listener
        .local_addr()
        .expect("main module-reaction server address");
    let (module_request_seen_tx, module_request_seen) = oneshot::channel();
    let (release_module_response, release_module_response_rx) = oneshot::channel();
    let (evaluation_started_tx, evaluation_started) = oneshot::channel();
    let (effect_request_seen_tx, effect_request_seen) = oneshot::channel();
    let (script_load_event_seen_tx, script_load_event_seen) = oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut module_stream, _) = listener
            .accept()
            .await
            .expect("accept main TLA module request");
        let module_request = read_owner_wake_http_request_head(&mut module_stream).await;
        let module_path = module_request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("main TLA module request path");
        assert_eq!(module_path, "/owner-main-tla-module.js");
        module_request_seen_tx
            .send(())
            .expect("signal main TLA module request");
        release_module_response_rx
            .await
            .expect("wait for main TLA module response release");
        let module_body = r#"
globalThis.__lmOwnerMainTlaState = "pending";
const ownerMainTlaGate = new Promise(resolve => {
  globalThis.__resolveLmOwnerMainTla = resolve;
});
fetch("/owner-main-tla-evaluation-started");
await ownerMainTlaGate;
globalThis.__lmOwnerMainTlaState = "completed";
fetch("/owner-main-tla-effect");
"#;
        let module_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            module_body.len(),
            module_body
        );
        module_stream
            .write_all(module_response.as_bytes())
            .await
            .expect("write main TLA module response");

        let (mut started_stream, _) = listener
            .accept()
            .await
            .expect("accept main TLA evaluation-start request");
        let started_request = read_owner_wake_http_request_head(&mut started_stream).await;
        let started_path = started_request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("main TLA evaluation-start path");
        assert_eq!(started_path, "/owner-main-tla-evaluation-started");
        evaluation_started_tx
            .send(())
            .expect("signal main TLA evaluation start");
        started_stream
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .expect("write main TLA evaluation-start response");

        let mut effect_request_seen_tx = Some(effect_request_seen_tx);
        let mut script_load_event_seen_tx = Some(script_load_event_seen_tx);
        for _ in 0..2 {
            let (mut effect_stream, _) = listener
                .accept()
                .await
                .expect("accept main TLA terminal effect request");
            let effect_request = read_owner_wake_http_request_head(&mut effect_stream).await;
            let effect_path = effect_request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("main TLA terminal effect path");
            match effect_path {
                "/owner-main-tla-effect" => effect_request_seen_tx
                    .take()
                    .expect("main TLA effect should be requested once")
                    .send(())
                    .expect("signal main TLA effect request"),
                "/owner-main-tla-script-load" => script_load_event_seen_tx
                    .take()
                    .expect("main TLA script load should be dispatched once")
                    .send(())
                    .expect("signal main TLA script-load request"),
                other => panic!("unexpected main TLA terminal effect path: {other}"),
            }
            effect_stream
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .expect("write main TLA terminal effect response");
        }
        assert!(
            effect_request_seen_tx.is_none() && script_load_event_seen_tx.is_none(),
            "main TLA evaluation and parser follow-up must both publish their effects"
        );
    });
    OwnerMainModuleReactionLivenessServer {
        base_url: format!("http://{addr}"),
        module_request_seen,
        release_module_response,
        evaluation_started,
        effect_request_seen,
        script_load_event_seen,
        task,
    }
}

async fn spawn_owner_inline_module_reaction_liveness_server(
    evaluation_started_path: &'static str,
    effect_path: &'static str,
) -> OwnerInlineModuleReactionLivenessServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind owner inline module-reaction server");
    let addr = listener
        .local_addr()
        .expect("inline module-reaction server address");
    let (evaluation_started_tx, evaluation_started) = oneshot::channel();
    let (effect_request_seen_tx, effect_request_seen) = oneshot::channel();
    let task = tokio::spawn(async move {
        for (expected_path, signal) in [
            (evaluation_started_path, evaluation_started_tx),
            (effect_path, effect_request_seen_tx),
        ] {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept inline module-reaction request");
            let request = read_owner_wake_http_request_head(&mut stream).await;
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("inline module-reaction request path");
            assert_eq!(path, expected_path);
            signal
                .send(())
                .expect("signal inline module-reaction request");
            stream
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .expect("write inline module-reaction response");
        }
    });
    OwnerInlineModuleReactionLivenessServer {
        base_url: format!("http://{addr}"),
        evaluation_started,
        effect_request_seen,
        task,
    }
}

async fn spawn_owner_child_module_graph_server() -> OwnerChildModuleGraphServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind owner child module graph server");
    let addr = listener.local_addr().expect("module graph server address");
    let (root_request_seen_tx, root_request_seen) = oneshot::channel();
    let (release_root_response, release_root_response_rx) = oneshot::channel();
    let (dependency_request_seen_tx, dependency_request_seen) = oneshot::channel();
    let (release_dependency_response, release_dependency_response_rx) = oneshot::channel();
    let (effect_request_seen_tx, effect_request_seen) = oneshot::channel();
    let task = tokio::spawn(async move {
        let mut served_root = false;
        let mut served_dependency = false;
        let mut root_request_seen_tx = Some(root_request_seen_tx);
        let mut release_root_response_rx = Some(release_root_response_rx);
        let mut dependency_request_seen_tx = Some(dependency_request_seen_tx);
        let mut release_dependency_response_rx = Some(release_dependency_response_rx);
        let mut effect_request_seen_tx = Some(effect_request_seen_tx);
        for _ in 0..3 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept owner child module graph request");
            let request = read_owner_wake_http_request_head(&mut stream).await;
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("module graph request path");
            let (status, content_type, body) = match path {
                "/child-owner-module.js" => {
                    assert!(!served_root, "module root must be fetched once");
                    served_root = true;
                    root_request_seen_tx
                        .take()
                        .expect("module root request should occur once")
                        .send(())
                        .expect("signal module root request");
                    release_root_response_rx
                        .take()
                        .expect("module root response gate should be consumed once")
                        .await
                        .expect("wait for module root response release");
                    (
                        "200 OK",
                        "application/javascript",
                        r#"import "./child-owner-dependency.js";
parent.__lm_owner_child_module_events.push("root");
fetch("/child-owner-module-effect");"#,
                    )
                }
                "/child-owner-dependency.js" => {
                    assert!(
                        served_root,
                        "dependency fetch must follow the root response"
                    );
                    assert!(!served_dependency, "module dependency must be fetched once");
                    served_dependency = true;
                    dependency_request_seen_tx
                        .take()
                        .expect("module dependency request should occur once")
                        .send(())
                        .expect("signal module dependency request");
                    release_dependency_response_rx
                        .take()
                        .expect("module dependency response gate should be consumed once")
                        .await
                        .expect("wait for module dependency response release");
                    (
                        "200 OK",
                        "application/javascript",
                        r#"parent.__lm_owner_child_module_events.push("dependency");"#,
                    )
                }
                "/child-owner-module-effect" => {
                    assert!(
                        served_dependency,
                        "module evaluation effect must follow dependency fetch"
                    );
                    effect_request_seen_tx
                        .take()
                        .expect("module effect request should occur once")
                        .send(())
                        .expect("signal module effect request");
                    ("204 No Content", "text/plain", "")
                }
                path => panic!("unexpected child module graph request path: {path}"),
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write owner child module graph response");
        }
        assert!(served_root && served_dependency);
    });
    OwnerChildModuleGraphServer {
        base_url: format!("http://{addr}"),
        root_request_seen,
        release_root_response,
        dependency_request_seen,
        release_dependency_response,
        effect_request_seen,
        task,
    }
}

async fn spawn_owner_wake_gated_server_with_content_type(
    expected_path: &'static str,
    body: &'static str,
    content_type: &'static str,
) -> (
    String,
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind owner wake gated fetch server");
    let addr = listener.local_addr().expect("server local addr");
    let (request_seen_tx, request_seen_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept owner wake gated fetch request");
        let request = read_owner_wake_http_request_head(&mut stream).await;
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("request path");
        assert_eq!(path, expected_path);
        request_seen_tx
            .send(())
            .expect("signal owner wake gated request seen");
        release_rx
            .await
            .expect("wait for owner wake gated response release");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write owner wake gated fetch response");
    });
    (
        format!("http://{addr}"),
        request_seen_rx,
        release_tx,
        server,
    )
}

async fn spawn_gated_resource_with_concurrent_effect(
    resource_path: &'static str,
    resource_body: &'static str,
    resource_content_type: &'static str,
    effect_path: &'static str,
) -> (
    String,
    oneshot::Receiver<()>,
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind gated resource/effect server");
    let addr = listener.local_addr().expect("server local addr");
    let (resource_seen_tx, resource_seen_rx) = oneshot::channel();
    let (effect_seen_tx, effect_seen_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut resource_stream, _) = listener
            .accept()
            .await
            .expect("accept gated resource request");
        let request = read_owner_wake_http_request_head(&mut resource_stream).await;
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("gated resource request path");
        assert_eq!(path, resource_path);
        resource_seen_tx
            .send(())
            .expect("signal gated resource request");

        // Keep the resource response parked while the renderer owner admits
        // an independent timer turn. Accepting the effect on the same listener
        // proves that turn ran; no protocol-output wake is used as a scheduler
        // observation surrogate.
        let (mut effect_stream, _) = listener
            .accept()
            .await
            .expect("accept concurrent effect request");
        let request = read_owner_wake_http_request_head(&mut effect_stream).await;
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("concurrent effect request path");
        assert_eq!(path, effect_path);
        effect_seen_tx
            .send(())
            .expect("signal concurrent effect request");
        effect_stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
            )
            .await
            .expect("write concurrent effect response");

        release_rx.await.expect("release gated resource response");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {resource_content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            resource_body.len(),
            resource_body
        );
        resource_stream
            .write_all(response.as_bytes())
            .await
            .expect("write gated resource response");
    });
    (
        format!("http://{addr}"),
        resource_seen_rx,
        effect_seen_rx,
        release_tx,
        server,
    )
}

async fn spawn_owner_lifecycle_gated_async_server() -> (
    String,
    oneshot::Receiver<()>,
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind owner lifecycle gated server");
    let addr = listener.local_addr().expect("server local addr");
    let (async_request_seen_tx, async_request_seen_rx) = oneshot::channel();
    let (domcontentloaded_request_seen_tx, domcontentloaded_request_seen_rx) = oneshot::channel();
    let (release_async_tx, release_async_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut async_request_seen_tx = Some(async_request_seen_tx);
        let mut domcontentloaded_request_seen_tx = Some(domcontentloaded_request_seen_tx);
        let mut release_async_rx = Some(release_async_rx);
        let mut handlers = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept owner lifecycle request");
            let request = read_owner_wake_http_request_head(&mut stream).await;
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("request path");
            match path {
                "/async.js" => {
                    async_request_seen_tx
                        .take()
                        .expect("async script request should occur once")
                        .send(())
                        .expect("signal gated async script request");
                    let release = release_async_rx
                        .take()
                        .expect("async response gate should be consumed once");
                    handlers.push(tokio::spawn(async move {
                        release
                            .await
                            .expect("wait for gated async response release");
                        let body = "globalThis.__lm_load_target_async_marker = 'executed';";
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        stream
                            .write_all(response.as_bytes())
                            .await
                            .expect("write gated async script response");
                    }));
                }
                "/domcontentloaded-seen" => {
                    domcontentloaded_request_seen_tx
                        .take()
                        .expect("DOMContentLoaded signal request should occur once")
                        .send(())
                        .expect("signal observed DOMContentLoaded request");
                    handlers.push(tokio::spawn(async move {
                        let response = "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        stream
                            .write_all(response.as_bytes())
                            .await
                            .expect("write DOMContentLoaded signal response");
                    }));
                }
                path => panic!("unexpected owner lifecycle request path: {path}"),
            }
        }
        for handler in handlers {
            handler
                .await
                .expect("owner lifecycle response handler should finish");
        }
    });
    (
        format!("http://{addr}"),
        async_request_seen_rx,
        domcontentloaded_request_seen_rx,
        release_async_tx,
        server,
    )
}

async fn spawn_gated_worker_message_server() -> (
    String,
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    const WORKER_SCRIPT: &str = r#"
self.onmessage = event => {
  if (event.data === "schedule-late") {
    fetch("/stale-worker-message.txt").then(
      response => response.text()
    ).then(
      text => postMessage("late:" + text),
      error => postMessage("error:" + error.name)
    );
    postMessage("scheduled");
  }
};
postMessage("ready");
"#;
    const WORKER_MESSAGE_BODY: &str = "worker-late-body";

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind gated worker message server");
    let addr = listener.local_addr().expect("server local addr");
    let (request_seen_tx, request_seen_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut worker_stream, _) = listener
            .accept()
            .await
            .expect("accept worker script request");
        let worker_request = read_owner_wake_http_request_head(&mut worker_stream).await;
        let worker_path = worker_request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("worker script request path");
        assert_eq!(worker_path, "/stale-worker.js");
        let worker_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            WORKER_SCRIPT.len(),
            WORKER_SCRIPT
        );
        worker_stream
            .write_all(worker_response.as_bytes())
            .await
            .expect("write worker script response");

        let (mut message_stream, _) = listener
            .accept()
            .await
            .expect("accept worker message fetch request");
        let message_request = read_owner_wake_http_request_head(&mut message_stream).await;
        let message_path = message_request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("worker message request path");
        assert_eq!(message_path, "/stale-worker-message.txt");
        request_seen_tx
            .send(())
            .expect("signal worker message fetch request seen");
        release_rx
            .await
            .expect("wait for worker message response release");
        let message_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            WORKER_MESSAGE_BODY.len(),
            WORKER_MESSAGE_BODY
        );
        let _ = message_stream.write_all(message_response.as_bytes()).await;
    });
    (
        format!("http://{addr}"),
        request_seen_rx,
        release_tx,
        server,
    )
}

async fn read_owner_wake_http_request_head(stream: &mut tokio::net::TcpStream) -> String {
    let mut request = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let read = stream
            .read(&mut byte)
            .await
            .expect("read owner wake request byte");
        assert_ne!(read, 0, "owner wake request closed before headers ended");
        request.push(byte[0]);
        if request.ends_with(b"\r\n\r\n") {
            return String::from_utf8_lossy(&request).into_owned();
        }
    }
}

#[cfg(debug_assertions)]
#[test]
fn renderer_owner_local_runtime_thread_affinity_is_sticky() {
    let runtime = JsRuntime::initialize();
    let renderer_owner = runtime.renderer_owner_handle();

    renderer_owner
        .bind_or_check_local_runtime_thread()
        .expect("first local-runtime entry should bind current thread");

    let renderer_owner_for_thread = renderer_owner.clone();
    let error = std::thread::spawn(move || {
        renderer_owner_for_thread
            .bind_or_check_local_runtime_thread()
            .expect_err("cross-thread local-runtime entry should fail")
            .to_string()
    })
    .join()
    .expect("worker should finish");

    assert!(
        error.contains("different thread"),
        "cross-thread local-runtime entry should report thread-affinity mismatch"
    );
}

#[test]
fn owner_local_runtime_access_is_allowed_on_plain_current_thread_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let executor = JsLocalExecutor::new();

    runtime.block_on(async move {
        assert_eq!(
            super::owner_local_runtime_access_path(&executor),
            super::OwnerLocalRuntimeAccessPath::CurrentThreadFallback,
            "plain current-thread runtime should use the current-thread owner-local runtime fallback"
        );
    });
}

#[test]
fn owner_local_runtime_access_is_allowed_on_matching_executor_lane() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();
    let executor = JsLocalExecutor::new();

    local.block_on(&runtime, async move {
        let executor_for_assert = executor.clone();
        executor
            .run(async move {
                assert_eq!(
                    super::owner_local_runtime_access_path(&executor_for_assert),
                    super::OwnerLocalRuntimeAccessPath::DirectNamedLane,
                    "matching executor lane should use the direct named-lane owner-local runtime path"
                );
            })
            .await;
    });
}

#[test]
fn owner_local_runtime_access_is_rejected_on_different_executor_lane() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();
    let first_executor = JsLocalExecutor::new();
    let second_executor = JsLocalExecutor::new();

    local.block_on(&runtime, async move {
        first_executor
            .run(async move {
                assert_eq!(
                    super::owner_local_runtime_access_path(&second_executor),
                    super::OwnerLocalRuntimeAccessPath::ExecutorHop,
                    "different executor lane should require an owner-local runtime hop"
                );
            })
            .await;
    });
}

#[test]
fn owner_local_runtime_access_is_rejected_on_scaffold_lane() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();
    let executor = JsLocalExecutor::new();

    local.block_on(&runtime, async move {
        scope_on_scaffold_js_local_executor(async move {
            assert_eq!(
                super::owner_local_runtime_access_path(&executor),
                super::OwnerLocalRuntimeAccessPath::ExecutorHop,
                "parse-time scaffold lane should not access page owner-local runtime directly"
            );
        })
        .await;
    });
}

#[test]
fn script_execution_domain_is_allowed_on_plain_current_thread_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let executor = JsLocalExecutor::new();

    runtime.block_on(async move {
        assert!(
            super::is_on_script_execution_domain_for(&executor),
            "plain current-thread runtime should remain a valid script execution fallback"
        );
    });
}

#[test]
fn script_execution_domain_is_allowed_on_matching_executor_lane() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();
    let executor = JsLocalExecutor::new();

    local.block_on(&runtime, async move {
        let executor_for_assert = executor.clone();
        executor
            .run(async move {
                assert!(
                    super::is_on_script_execution_domain_for(&executor_for_assert),
                    "matching executor lane should remain a valid script execution domain"
                );
            })
            .await;
    });
}

#[test]
fn script_execution_domain_is_allowed_on_scaffold_lane() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();
    let executor = JsLocalExecutor::new();

    local.block_on(&runtime, async move {
        scope_on_scaffold_js_local_executor(async move {
            assert!(
                super::is_on_script_execution_domain_for(&executor),
                "parse-time scaffold lane should remain a valid script execution domain"
            );
        })
        .await;
    });
}

#[test]
fn script_execution_domain_is_rejected_on_different_executor_lane() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();
    let first_executor = JsLocalExecutor::new();
    let second_executor = JsLocalExecutor::new();

    local.block_on(&runtime, async move {
        first_executor
            .run(async move {
                assert!(
                    !super::is_on_script_execution_domain_for(&second_executor),
                    "a different executor lane should not count as this page's script execution domain"
                );
            })
            .await;
    });
}

#[test]
fn script_execution_lane_is_rejected_on_plain_current_thread_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let executor = JsLocalExecutor::new();

    runtime.block_on(async move {
        assert!(
            !is_on_script_execution_lane_for(&executor),
            "plain current-thread runtime fallback must not count as a lane-backed script execution domain"
        );
    });
}

#[test]
fn script_execution_lane_is_allowed_on_matching_executor_lane() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();
    let executor = JsLocalExecutor::new();

    local.block_on(&runtime, async move {
        let executor_for_assert = executor.clone();
        executor
            .run(async move {
                assert!(
                    is_on_script_execution_lane_for(&executor_for_assert),
                    "matching executor lane should count as a lane-backed script execution domain"
                );
            })
            .await;
    });
}

#[test]
fn script_execution_lane_is_allowed_on_scaffold_lane() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();
    let executor = JsLocalExecutor::new();

    local.block_on(&runtime, async move {
        scope_on_scaffold_js_local_executor(async move {
            assert!(
                is_on_script_execution_lane_for(&executor),
                "scaffold lane should count as a lane-backed script execution domain"
            );
        })
        .await;
    });
}

#[test]
fn script_execution_lane_is_rejected_on_different_executor_lane() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();
    let first_executor = JsLocalExecutor::new();
    let second_executor = JsLocalExecutor::new();

    local.block_on(&runtime, async move {
        first_executor
            .run(async move {
                assert!(
                    !is_on_script_execution_lane_for(&second_executor),
                    "a different executor lane must not count as this page's lane-backed script execution domain"
                );
            })
            .await;
    });
}

#[test]
fn scaffold_lane_uses_distinct_script_and_runtime_access_paths() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();
    let executor = JsLocalExecutor::new();

    local.block_on(&runtime, async move {
        scope_on_scaffold_js_local_executor(async move {
            assert_eq!(
                super::script_execution_domain_path(&executor),
                super::ScriptExecutionDomainPath::DirectScaffoldLane,
                "parse-time scaffold should stay a script-execution domain"
            );
            assert_eq!(
                super::owner_local_runtime_access_path(&executor),
                super::OwnerLocalRuntimeAccessPath::ExecutorHop,
                "parse-time scaffold must not become an owner-local runtime direct path"
            );
        })
        .await;
    });
}

#[test]
fn current_thread_fallback_uses_distinct_direct_paths() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let executor = JsLocalExecutor::new();

    runtime.block_on(async move {
        assert_eq!(
            super::script_execution_domain_path(&executor),
            super::ScriptExecutionDomainPath::CurrentThreadFallback,
            "plain current-thread runtime should remain a script-execution fallback"
        );
        assert_eq!(
            super::script_execution_lane_path(&executor),
            super::ScriptExecutionLanePath::Inaccessible,
            "plain current-thread runtime fallback should no longer count as a lane-backed script execution path"
        );
        assert_eq!(
            super::owner_local_runtime_access_path(&executor),
            super::OwnerLocalRuntimeAccessPath::CurrentThreadFallback,
            "plain current-thread runtime should remain a current-thread owner-local runtime fallback"
        );
        assert_eq!(
            super::owner_local_runtime_entry_path(&executor),
            super::OwnerLocalRuntimeEntryPath::ExecutorHop,
            "plain current-thread runtime fallback should no longer count as a direct owner-local runtime entry path"
        );
    });
}

#[test]
fn current_thread_fallback_is_rejected_on_multithread_runtime() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("multi-thread runtime should build");
    let executor = JsLocalExecutor::new();

    runtime.block_on(async move {
        assert!(
            matches!(
                executor.current_access_context(),
                crate::local_executor::JsLocalExecutorAccessContext::Outside
            ),
            "multi-thread runtimes should stay outside any direct current-thread fallback context"
        );
        assert_eq!(
            super::script_execution_domain_path(&executor),
            super::ScriptExecutionDomainPath::Inaccessible,
            "multi-thread runtimes should not count as direct script-execution fallbacks"
        );
        assert_eq!(
            super::owner_local_runtime_access_path(&executor),
            super::OwnerLocalRuntimeAccessPath::ExecutorHop,
            "multi-thread runtimes should not count as direct owner-local-runtime fallbacks"
        );
    });
}

#[test]
fn named_owner_execution_lane_is_rejected_on_plain_current_thread_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let executor = JsLocalExecutor::new();

    runtime.block_on(async move {
        assert!(
            !super::is_on_named_owner_execution_lane_for(&executor),
            "plain current-thread fallback must not count as a named owner lane"
        );
    });
}

#[test]
fn named_owner_execution_lane_is_allowed_on_matching_executor_lane() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();
    let executor = JsLocalExecutor::new();

    local.block_on(&runtime, async move {
        let executor_for_assert = executor.clone();
        executor
            .run(async move {
                assert!(
                    super::is_on_named_owner_execution_lane_for(&executor_for_assert),
                    "matching executor lane should count as the named owner lane"
                );
            })
            .await;
    });
}

#[test]
fn named_owner_execution_lane_is_rejected_on_scaffold_lane() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();
    let executor = JsLocalExecutor::new();

    local.block_on(&runtime, async move {
        scope_on_scaffold_js_local_executor(async move {
            assert!(
                !super::is_on_named_owner_execution_lane_for(&executor),
                "scaffold lane must not count as the named owner lane"
            );
        })
        .await;
    });
}

#[test]
fn named_owner_execution_lane_is_rejected_on_different_executor_lane() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();
    let first_executor = JsLocalExecutor::new();
    let second_executor = JsLocalExecutor::new();

    local.block_on(&runtime, async move {
        first_executor
            .run(async move {
                assert!(
                    !super::is_on_named_owner_execution_lane_for(&second_executor),
                    "a different executor lane must not count as this page's named owner lane"
                );
            })
            .await;
    });
}

#[test]
fn parse_time_scaffold_lane_is_allowed_on_scaffold_lane() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();

    local.block_on(&runtime, async move {
        scope_on_scaffold_js_local_executor(async move {
            assert!(
                super::is_on_parse_time_scaffold_lane(),
                "parse-time scaffold lane should be recognized as itself"
            );
        })
        .await;
    });
}

#[test]
fn parse_time_scaffold_lane_is_rejected_on_plain_current_thread_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(async move {
        assert!(
            !super::is_on_parse_time_scaffold_lane(),
            "plain current-thread fallback must not count as the parse-time scaffold lane"
        );
    });
}

#[test]
fn parse_time_scaffold_lane_is_rejected_on_named_executor_lane() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");
    let local = tokio::task::LocalSet::new();
    let executor = JsLocalExecutor::new();

    local.block_on(&runtime, async move {
        executor
            .run(async move {
                assert!(
                    !super::is_on_parse_time_scaffold_lane(),
                    "named owner lanes must not count as the parse-time scaffold lane"
                );
            })
            .await;
    });
}
