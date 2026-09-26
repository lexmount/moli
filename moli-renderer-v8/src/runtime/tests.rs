use super::{
    ExternalRawDocumentBodyStream, JsRuntime, JsRuntimeOwner, PageVmInitStage,
    PreparedRendererDocument, RendererCaptureScreenshotReply, RendererInputDispatchOutcome,
    RendererOutputItem, RendererOutputPublication, RendererOutputResidenceIdentity,
    RendererOutputTransportMessage, RendererOutputTransportReceiver, RendererOutputTransportSender,
    RendererOwnerAction, RendererPageCommand, RendererPageHandle, RendererPageReply,
    RendererPageTestingHandle, RendererPendingPopupActivation, RendererPointerEventProperties,
    RendererProtocolObservation, RendererRuntimeCommandOutput, RendererRuntimeInspectorMessage,
};
use crate::local_executor::{is_on_script_execution_lane_for, scope_on_scaffold_js_local_executor};
use crate::network::ResourceRequestClient;
use crate::{
    RendererDocumentLifecycleEventKind, RendererDocumentLifecycleMilestone,
    RendererNavigationReplyPolicy, RendererReplyBoundary, RendererTopLevelNavigationDispatch,
};
use std::collections::HashSet;
use std::io::Cursor;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};

mod open_streaming;
mod redirect_chain;

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
        .prepare_streaming_raw_document_from_external_body(
            runtime.reserve_page_for_creation(),
            url.clone(),
            url,
            None,
            false,
            0,
            Vec::new(),
            200,
            vec![("content-type".to_owned(), content_type.as_bytes().to_vec())],
            loader,
            crate::RendererWebStorageHandles::ephemeral(),
            raw_body,
            false,
            PageVmInitStage::Load,
            reply_boundary,
            RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            RendererNavigationReplyPolicy::FollowBeforeReply,
            None,
            None,
            crate::RendererDocumentOptions {
                ..Default::default()
            },
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

fn initialize_layout_test_runtime() -> JsRuntimeOwner {
    let runtime = JsRuntime::initialize();
    runtime
        .renderer_owner_handle()
        .configure_layout_policy(crate::real_layout_test_policy())
        .expect("layout test policy should configure before page creation");
    runtime
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

fn mouse_input_json_witness(reply: &RendererPageReply) -> serde_json::Value {
    let RendererPageReply::RuntimeEvaluationResult(result) = reply else {
        panic!(
            "expected JSON-string input witness; unexpected reply variant: {:?}",
            std::mem::discriminant(reply)
        );
    };
    let payload = result.as_protocol_payload();
    let serialized = payload
        .get("value")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("expected JSON-string input witness; raw payload: {payload:?}"));
    serde_json::from_str(serialized).unwrap_or_else(|error| {
        panic!("invalid input witness JSON: {error}; raw payload: {payload:?}")
    })
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

// This witness needs one renderer-owner worker and one observer worker.

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
        .start_create_html_page_from_response(
            runtime.reserve_page_for_creation(),
            url.clone(),
            url,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            loader,
            crate::RendererWebStorageHandles::ephemeral(),
            html.to_owned(),
            None,
            top_level_navigation_dispatch,
            crate::RendererDocumentOptions {
                indexed_db_manager,
                ..Default::default()
            },
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
        .create_streaming_raw_page_from_external_body(
            url.clone(),
            url,
            None,
            false,
            0,
            Vec::new(),
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            loader,
            crate::RendererWebStorageHandles::ephemeral(),
            raw_body,
            false,
            PageVmInitStage::Load,
            RendererReplyBoundary::DocumentCommit,
            top_level_navigation_dispatch,
            navigation_reply_policy,
            None,
            None,
            crate::RendererDocumentOptions {
                ..Default::default()
            },
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
mod extracted;
