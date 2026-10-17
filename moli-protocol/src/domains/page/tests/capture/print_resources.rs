use super::*;
use crate::conn::{CdpConnection, CdpInitialStoragePartition};
use axum::{Router, http::header::CONTENT_TYPE, routing::get};
use moli_core::{LayoutPolicy, OptionalResourceFetchMask, runtime::NavigationRuntimeConfig};
use moli_fetch::FetchConfig;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

// Exercise parsed public CDP dispatch, the production renderer owner and the
// normal connection output scheduler. Do not serialize the Print wait before
// routing Fetch.requestPaused: that would hide the deadlock this covers.
#[tokio::test(flavor = "multi_thread")]
async fn print_to_pdf_publishes_fetch_pause_before_reply_and_uses_first_resource() {
    const SESSION: &str = "SID-PRINT-FETCH";
    const PRINT_ID: u64 = 12704;
    const HTML: &str = concat!(
        "<!doctype html><style>",
        "html,body{margin:0;background:white;scrollbar-width:none}",
        "#asset{display:none;position:absolute;left:8px;top:8px}",
        "</style><div id=asset></div>"
    );
    let png = moli_image::encode_png(
        &moli_image::RgbaImage::try_new(1, 1, vec![17, 85, 221, 255]).unwrap(),
    )
    .unwrap()
    .bytes;
    let image_requests = Arc::new(AtomicUsize::new(0));
    let server_requests = image_requests.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = Router::new()
            .route(
                "/page",
                get(|| async { ([(CONTENT_TYPE, "text/html")], HTML) }),
            )
            .route(
                "/print-only.png",
                get(move || {
                    let png = png.clone();
                    let requests = server_requests.clone();
                    async move {
                        requests.fetch_add(1, Ordering::SeqCst);
                        ([(CONTENT_TYPE, "image/png")], png)
                    }
                }),
            );
        axum::serve(listener, app).await.unwrap();
    });

    let result = tokio::time::timeout(Duration::from_secs(20), async {
        let connection = CdpConnection::new_with_initial_storage_partition_and_runtime_config(
            CdpInitialStoragePartition::memory(),
            NavigationRuntimeConfig::new(
                FetchConfig::default(),
                OptionalResourceFetchMask::IMAGE,
                true,
                LayoutPolicy::OnDemand,
            ),
        );
        let mut ctx = TestContext::from_conn(connection);
        let page_url = format!("http://{address}/page");
        let image_url = format!("http://{address}/print-only.png");
        install_active_screenshot_page(
            &mut ctx,
            "BID-PRINT-FETCH",
            "TID-PRINT-FETCH",
            SESSION,
            &page_url,
        )
        .await;
        set_screenshot_viewport(&mut ctx, SESSION, 128, 96, 1.0, 12700).await;
        ctx.process_async(json!({
            "id":12701, "method":"Fetch.enable", "sessionId":SESSION,
            "params":{"patterns":[{
                "urlPattern":image_url, "resourceType":"Image", "requestStage":"Request"
            }]}
        }))
        .await;
        assert_eq!(take_response_by_id(&mut ctx, 12701)["result"], json!({}));

        // Install only after the resource-free document load. The inactive
        // print rule cannot make fixture navigation depend on interception.
        let css = format!(
            "@media print{{#asset{{display:block;width:40px;height:20px;\
             background:rgb(255,0,255) url({image_url:?})}}}}"
        );
        ctx.process_async(json!({
            "id":12702, "method":"Runtime.evaluate", "sessionId":SESSION,
            "params":{"expression":format!(
                "(() => {{const sheet=new CSSStyleSheet();sheet.replaceSync({});\
                 document.adoptedStyleSheets=[sheet];return true;}})()",
                json!(css)
            )}
        }))
        .await;
        let installed = take_response_by_id(&mut ctx, 12702);
        assert!(
            installed["result"]["exceptionDetails"].is_null(),
            "{installed}"
        );
        assert_eq!(installed["result"]["result"]["value"], true);
        assert_eq!(image_requests.load(Ordering::SeqCst), 0);
        assert!(
            !ctx.sent
                .iter()
                .any(|message| message["method"] == "Fetch.requestPaused")
        );
        ctx.sent.clear();

        let pending = ctx
            .conn
            .try_start_pending_command_dispatch(
                &json!({
                    "id":PRINT_ID, "method":"Page.printToPDF", "sessionId":SESSION,
                    "params":{"printBackground":true}
                })
                .to_string(),
            )
            .expect("public Print must create a pending command");
        assert_eq!(
            pending.kind_name(),
            "Page",
            "public Print must not be wrapped in Native"
        );
        let mut print_completion = Box::pin(pending.wait());
        let paused = tokio::select! {
            biased;
            completed = &mut print_completion => {
                let step = ctx.conn.complete_pending_command_dispatch(completed).await;
                let (messages, _) = ctx.complete_command_task_step_for_test(step).await;
                panic!(
                    "Print reached terminal before its paused resource could be continued: \
                     messages={messages:?}; observed={:?}; image_requests={}",
                    ctx.sent,
                    image_requests.load(Ordering::SeqCst)
                );
            }
            paused = ctx.wait_for_scheduler_message("print resource interception", |message| {
                message["method"] == "Fetch.requestPaused" && message["sessionId"] == SESSION
            }) => paused,
        };
        assert_eq!(paused["params"]["request"]["url"], image_url);
        assert_eq!(paused["params"]["resourceType"], "Image");
        assert!(paused["params"]["responseStatusCode"].is_null(), "{paused}");
        assert_eq!(image_requests.load(Ordering::SeqCst), 0);
        assert!(!ctx.sent.iter().any(|message| message["id"] == PRINT_ID));
        let request_id = paused["params"]["requestId"].as_str().unwrap().to_owned();

        ctx.process_async(json!({
            "id":12705, "method":"Fetch.continueRequest", "sessionId":SESSION,
            "params":{"requestId":request_id}
        }))
        .await;
        assert_eq!(take_response_by_id(&mut ctx, 12705)["result"], json!({}));
        let completed = print_completion.await;
        let step = ctx.conn.complete_pending_command_dispatch(completed).await;
        let (messages, _) = ctx.complete_command_task_step_for_test(step).await;
        let responses = messages
            .iter()
            .filter(|message| message["id"] == PRINT_ID)
            .collect::<Vec<_>>();
        assert_eq!(responses.len(), 1, "{messages:?}");
        let response = responses[0];
        assert!(response["error"].is_null(), "{response}");
        let pdf = screenshot_bytes(response);
        assert!(pdf.starts_with(b"%PDF-1.7"));
        assert!(pdf.ends_with(b"%%EOF\n"));
        let image = first_pdf_image(&pdf);
        assert!(image.width >= 48 && image.height >= 28);
        // A missing resource paints magenta; only the continued PNG supplies
        // this blue. Inspect the first PDF's image, not a second capture.
        for y in 16..24 {
            for x in 16..32 {
                let index = ((y * image.width + x) * 4) as usize;
                let pixel = &image.rgba[index..index + 4];
                assert_eq!(pixel, [17, 85, 221, 255], "pixel ({x}, {y})");
            }
        }
        assert_eq!(image_requests.load(Ordering::SeqCst), 1);
    })
    .await;
    server.abort();
    let server_result = server.await;
    assert!(
        matches!(&server_result, Err(error) if error.is_cancelled()),
        "fixture server did not stop after abort: {server_result:?}"
    );
    result.expect("bounded public Print/Fetch integration");
}

// Native PDF stores the bounded repeated-background raster as an image
// XObject. Decode the first PDF's lossless color and alpha streams so this
// still proves resource pixels were painted, independently of serialization.
fn first_pdf_image(pdf: &[u8]) -> moli_image::RgbaImage {
    let document = lopdf::Document::load_mem(pdf).expect("valid first PDF");
    let images = document
        .objects
        .values()
        .filter_map(|object| object.as_stream().ok())
        .filter(|stream| {
            stream
                .dict
                .get(b"Subtype")
                .and_then(|value| value.as_name())
                .ok()
                == Some(b"Image")
                && stream
                    .dict
                    .get(b"ColorSpace")
                    .and_then(|value| value.as_name())
                    .ok()
                    != Some(b"DeviceGray")
        })
        .collect::<Vec<_>>();
    assert_eq!(images.len(), 1, "one printed background image");
    let image = images[0];
    assert_eq!(
        image
            .dict
            .get(b"BitsPerComponent")
            .unwrap()
            .as_i64()
            .unwrap(),
        8
    );
    let width: u32 = image
        .dict
        .get(b"Width")
        .unwrap()
        .as_i64()
        .unwrap()
        .try_into()
        .unwrap();
    let height: u32 = image
        .dict
        .get(b"Height")
        .unwrap()
        .as_i64()
        .unwrap()
        .try_into()
        .unwrap();
    let rgb = image
        .decompressed_content()
        .expect("decode first PDF's color stream");
    let pixel_count = width as usize * height as usize;
    assert_eq!(rgb.len(), pixel_count * 3);
    let alpha = if let Ok(mask) = image.dict.get(b"SMask") {
        document
            .dereference(mask)
            .unwrap()
            .1
            .as_stream()
            .unwrap()
            .decompressed_content()
            .expect("decode first PDF's alpha stream")
    } else {
        vec![255; pixel_count]
    };
    assert_eq!(alpha.len(), pixel_count);
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for (color, alpha) in rgb.as_chunks::<3>().0.iter().zip(alpha) {
        rgba.extend_from_slice(color);
        rgba.push(alpha);
    }
    moli_image::RgbaImage::try_new(width, height, rgba).unwrap()
}

async fn assert_print_service_worker_fallback_policy(intercept: bool, offline: bool) {
    const SESSION: &str = "SID-PRINT-SW-POLICY";
    const PRINT_ID: u64 = 12810;
    let png = moli_image::encode_png(
        &moli_image::RgbaImage::try_new(1, 1, vec![17, 85, 221, 255]).unwrap(),
    )
    .unwrap()
    .bytes;
    let requests = Arc::new(AtomicUsize::new(0));
    let server_requests = requests.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = Router::new()
            .route(
                "/page",
                get(|| async {
                    (
                        [(CONTENT_TYPE, "text/html")],
                        "<!doctype html><style>#asset{display:none}</style><div id=asset></div>",
                    )
                }),
            )
            .route(
                "/worker.js",
                get(|| async {
                    (
                        [(CONTENT_TYPE, "text/javascript")],
                        r#"
                self.addEventListener('install', event=>event.waitUntil(self.skipWaiting()));
                self.addEventListener('activate', event=>event.waitUntil(clients.claim()));
                self.addEventListener('fetch', event=>{});
            "#,
                    )
                }),
            )
            .route(
                "/print-only.png",
                get(move || {
                    let requests = server_requests.clone();
                    let png = png.clone();
                    async move {
                        requests.fetch_add(1, Ordering::SeqCst);
                        ([(CONTENT_TYPE, "image/png")], png)
                    }
                }),
            );
        axum::serve(listener, app).await.unwrap();
    });
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        let connection=CdpConnection::new_with_initial_storage_partition_and_runtime_config(
            CdpInitialStoragePartition::memory(), NavigationRuntimeConfig::new(FetchConfig::default(), OptionalResourceFetchMask::IMAGE, true, LayoutPolicy::OnDemand));
        let mut ctx=TestContext::from_conn(connection);
        let image_url=format!("http://{address}/print-only.png");
        install_active_screenshot_page(&mut ctx,"BID-PRINT-SW-POLICY","TID-PRINT-SW-POLICY",SESSION,&format!("http://{address}/page")).await;
        ctx.process_async(json!({"id":12799,"method":"Network.enable","sessionId":SESSION})).await;
        assert_eq!(take_response_by_id(&mut ctx,12799)["result"],json!({}));
        ctx.process_async(json!({"id":12800,"method":"Runtime.evaluate","sessionId":SESSION,"params":{
            "expression":r#"(async()=>{
                await navigator.serviceWorker.register('/worker.js',{scope:'/'});
                await navigator.serviceWorker.ready;
                if(!navigator.serviceWorker.controller) await new Promise(resolve=>navigator.serviceWorker.addEventListener('controllerchange',resolve,{once:true}));
                return navigator.serviceWorker.controller!==null;
            })()"#, "awaitPromise":true,"returnByValue":true
        }})).await;
        wait_until_messages(&mut ctx,Some(SESSION),"print SW controller",|messages|messages.iter().any(|m|m["id"]==12800)).await;
        let controlled=take_response_by_id(&mut ctx,12800);
        assert_eq!(controlled["result"]["result"]["value"],true,"{controlled}");
        let css=format!("@media print{{#asset{{display:block;width:40px;height:20px;background:magenta url({image_url:?})}}}}");
        ctx.process_async(json!({"id":12801,"method":"Runtime.evaluate","sessionId":SESSION,"params":{
            "expression":format!("const sheet=new CSSStyleSheet();sheet.replaceSync({});document.adoptedStyleSheets=[sheet];true",json!(css))
        }})).await;
        let installed=take_response_by_id(&mut ctx,12801);
        assert!(installed["result"]["exceptionDetails"].is_null(),"{installed}");
        assert_eq!(requests.load(Ordering::SeqCst),0);
        if intercept {
            ctx.process_async(json!({"id":12802,"method":"Fetch.enable","sessionId":SESSION,"params":{"patterns":[{"urlPattern":image_url,"resourceType":"Image","requestStage":"Request"}]}})).await;
            assert_eq!(take_response_by_id(&mut ctx,12802)["result"],json!({}));
        }
        let policy = if offline {
            json!({"id":12803,"method":"Network.emulateNetworkConditions","sessionId":SESSION,"params":{"offline":true,"latency":0,"downloadThroughput":-1,"uploadThroughput":-1}})
        } else {
            json!({"id":12803,"method":"Network.setBlockedURLs","sessionId":SESSION,"params":{"urls":[image_url]}})
        };
        ctx.process_async(policy).await;
        assert_eq!(take_response_by_id(&mut ctx,12803)["result"],json!({}));
        ctx.sent.clear();
        let pending=ctx.conn.try_start_pending_command_dispatch(&json!({"id":PRINT_ID,"method":"Page.printToPDF","sessionId":SESSION,"params":{"printBackground":true}}).to_string()).unwrap();
        let mut completion=Box::pin(pending.wait());
        if intercept {
            let paused=tokio::select! {
                completed=&mut completion => {
                    let step=ctx.conn.complete_pending_command_dispatch(completed).await;
                    let (messages,_)=ctx.complete_command_task_step_for_test(step).await;
                    panic!("print ended before Fetch continuation: {messages:?}");
                }
                paused=ctx.wait_for_scheduler_message("print SW Fetch pause",|message|message["method"]=="Fetch.requestPaused" && message["sessionId"]==SESSION) => paused,
            };
            assert_eq!(paused["params"]["request"]["url"],image_url);
            ctx.process_async(json!({"id":12804,"method":"Fetch.continueRequest","sessionId":SESSION,"params":{"requestId":paused["params"]["requestId"]}})).await;
            assert_eq!(take_response_by_id(&mut ctx,12804)["result"],json!({}));
        }
        let step=ctx.conn.complete_pending_command_dispatch(completion.await).await;
        let (messages,_)=ctx.complete_command_task_step_for_test(step).await;
        let response=messages.iter().find(|m|m["id"]==PRINT_ID).unwrap();
        assert!(response["error"].is_null(),"{response}");
        assert!(screenshot_bytes(response).starts_with(b"%PDF-1.7"));
        assert_eq!(requests.load(Ordering::SeqCst),0,"SW network fallback must retain page policy: intercept={intercept}, offline={offline}");
    }).await;
    server.abort();
    let _ = server.await;
    result.expect("bounded print SW network-policy test");
}

#[tokio::test(flavor = "multi_thread")]
async fn print_sw_fallback_keeps_blocked_urls_without_fetch_pause() {
    assert_print_service_worker_fallback_policy(false, false).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn print_sw_fallback_keeps_offline_without_fetch_pause() {
    assert_print_service_worker_fallback_policy(false, true).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn print_sw_fallback_keeps_blocked_urls_after_fetch_pause() {
    assert_print_service_worker_fallback_policy(true, false).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn print_sw_fallback_keeps_offline_after_fetch_pause() {
    assert_print_service_worker_fallback_policy(true, true).await;
}
