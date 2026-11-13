use super::*;
use crate::runtime::{RendererPrintToPdfReply, RendererPrintToPdfRequest};
use base64::Engine;

async fn capture_font_widths(page: &RendererPageHandle) -> anyhow::Result<Vec<String>> {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::PrintToPdf(
            RendererPrintToPdfRequest::default(),
        ))
        .await?;
    let RendererPageReply::PrintToPdf(RendererPrintToPdfReply::Printed(bytes)) = reply else {
        anyhow::bail!("font-order capture must produce PDF bytes");
    };
    let pdf = lopdf::Document::load_mem(&bytes)?;
    anyhow::ensure!(pdf.extract_text(&[1])?.contains("AAAA"));
    let widths = pdf
        .objects
        .values()
        .filter_map(|object| {
            let dictionary = object.as_dict().ok()?;
            (dictionary
                .get(b"Subtype")
                .and_then(|value| value.as_name())
                .ok()
                == Some(b"CIDFontType2"))
            .then(|| {
                format!(
                    "{:?}|{:?}",
                    dictionary.get(b"DW").ok(),
                    dictionary.get(b"W").ok()
                )
            })
        })
        .collect::<Vec<_>>();
    anyhow::ensure!(
        widths.len() == 1,
        "one fixture font must be embedded: {widths:?}"
    );
    Ok(widths)
}

async fn assert_capture_font_pixel(page: &RendererPageHandle, wide: bool) -> anyhow::Result<()> {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::CaptureScreenshot(
            RendererCaptureScreenshotRequest {
                purpose: RendererScreenshotPurpose::Print {
                    print_background: true,
                },
                ..RendererCaptureScreenshotRequest::viewport_png()
            },
        ))
        .await?;
    let RendererPageReply::CaptureScreenshot(RendererCaptureScreenshotReply::Captured(image)) =
        reply
    else {
        anyhow::bail!("font-order capture must produce raster bytes");
    };
    anyhow::ensure!((image.width, image.height) == (220, 96));
    let expected = if wide { [0, 0, 0, 255] } else { [255; 4] };
    anyhow::ensure!(
        decoded_png_pixel(&image.bytes, 148, 20) == expected,
        "capture must use the {} Ahem face",
        if wide { "wide" } else { "narrow" }
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn print_fonts_follow_current_declaration_order_and_manual_priority() {
    let narrow = base64::engine::general_purpose::STANDARD.encode(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../moli-layout/tests/fixtures/moli-ahem-narrow.ttf"
    )));
    let wide = base64::engine::general_purpose::STANDARD.encode(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../moli-layout/tests/fixtures/moli-ahem-wide.ttf"
    )));
    for policy in [LayoutPolicy::OnDemand, LayoutPolicy::Realtime] {
        let runtime = JsRuntime::initialize();
        runtime
            .renderer_owner_handle()
            .configure_layout_policy(policy)
            .unwrap();
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let mut page = create_test_html_page(
            &runtime,
            &loader,
            url::Url::parse("https://print-font-order.test/").unwrap(),
            "<!doctype html><body>",
        )
        .await;
        let result=tokio::time::timeout(Duration::from_secs(20), async {
            page.run_async_command(RendererPageCommand::SetViewportSurface(Some(
                crate::protocol_types::ViewportSurface {
                    inner_width:220,inner_height:96,device_pixel_ratio:1.0,..Default::default()
                }
            ))).await?;
            // Accessing/loading FontFaceSet must not make its screen CSS order
            // override the declaration order of a temporary print view.
            let setup=format!(r#"(async()=>{{
                document.head.innerHTML=`<style>
                    @media print{{@font-face{{font-family:CaptureOrder;src:url(data:font/ttf;base64,{narrow})}}}}
                    @font-face{{font-family:CaptureOrder;src:url(data:font/ttf;base64,{wide})}}
                    html,body{{margin:0;background:white;scrollbar-width:none}}
                    #probe{{position:absolute;left:8px;top:8px;display:inline-block;font:40px/40px CaptureOrder;color:black}}
                </style>`;
                document.body.innerHTML='<span id=probe>AAAA</span>';
                await document.fonts.load('40px CaptureOrder','AAAA');
                await document.fonts.ready;
                return JSON.stringify([document.getElementById('probe').getBoundingClientRect().width,document.fonts.size]);
            }})()"#);
            anyhow::ensure!(evaluate_print_fixture(&page,setup,true).await?==serde_json::json!("[160,1]"));
            let mixed=capture_font_widths(&page).await?;
            assert_capture_font_pixel(&page,true).await?;
            anyhow::ensure!(evaluate_print_fixture(&page,
                "JSON.stringify([document.getElementById('probe').getBoundingClientRect().width,document.fonts.size,matchMedia('print').matches])".to_owned(),false
            ).await?==serde_json::json!("[160,1,false]"),"printing must leave the screen font view intact");

            evaluate_print_fixture(&page,
                "document.styleSheets[0].deleteRule(0);true".to_owned(),false).await?;
            anyhow::ensure!(mixed==capture_font_widths(&page).await?,
                "the later wide declaration must win with or without the earlier print-only face");

            evaluate_print_fixture(&page,format!(r#"(async()=>{{
                const face=new FontFace('CaptureOrder',Uint8Array.from(atob('{narrow}'),c=>c.charCodeAt(0)));
                await face.load();document.fonts.add(face);return true;
            }})()"#),true).await?;
            let manual=capture_font_widths(&page).await?;
            anyhow::ensure!(manual!=mixed,"the manual narrow face must retain priority over CSS");
            assert_capture_font_pixel(&page,false).await?;
            let (pending, _) = page.run_async_command(
                RendererPageCommand::PendingSubresourceRequestCount,
            ).await?;
            anyhow::ensure!(matches!(pending, RendererPageReply::Usize(0)));
            Ok::<_,anyhow::Error>(())
        }).await;
        page.close_async().await.unwrap();
        result.unwrap().unwrap();
    }
}

async fn assert_print_reuses_loaded_font_source(renamed: bool) {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::{io::AsyncWriteExt, net::TcpListener};

    for policy in [LayoutPolicy::OnDemand, LayoutPolicy::Realtime] {
        let requests = Arc::new(AtomicUsize::new(0));
        let seen = requests.clone();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server: tokio::task::JoinHandle<anyhow::Result<()>> = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await?;
                let request = read_owner_wake_http_request_head(&mut stream).await;
                anyhow::ensure!(request.starts_with("GET /font.ttf "), "{request}");
                let first = seen.fetch_add(1, Ordering::SeqCst) == 0;
                let (status, body): (_, &[u8]) = if first {
                    (
                        "200 OK",
                        include_bytes!(concat!(
                            env!("CARGO_MANIFEST_DIR"),
                            "/../moli-layout/tests/fixtures/moli-ahem-wide.ttf"
                        )),
                    )
                } else {
                    ("404 Not Found", &[])
                };
                stream.write_all(format!(
                    "HTTP/1.1 {status}\r\nContent-Type: font/ttf\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()
                ).as_bytes()).await?;
                stream.write_all(body).await?;
                stream.shutdown().await?;
            }
        });
        let runtime = JsRuntime::initialize();
        runtime
            .renderer_owner_handle()
            .configure_layout_policy(policy)
            .unwrap();
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        );
        let mut page = create_test_html_page(
            &runtime,
            &loader,
            url::Url::parse(&format!("{base}/page")).unwrap(),
            "<!doctype html><body>",
        )
        .await;
        let result = tokio::time::timeout(Duration::from_secs(20), async {
            page.run_async_command(RendererPageCommand::SetViewportSurface(Some(
                crate::protocol_types::ViewportSurface {
                    inner_width:220,inner_height:96,device_pixel_ratio:1.0,..Default::default()
                }
            ))).await?;
            let print_family = if renamed { "CaptureAlias" } else { "CaptureSource" };
            let setup = format!(r#"(async()=>{{
                document.head.innerHTML=`<style>
                    @media screen{{@font-face{{font-family:CaptureSource;src:url({base}/font.ttf)}}}}
                    html,body{{margin:0;background:white;scrollbar-width:none}}
                    #probe{{position:absolute;left:8px;top:8px;display:inline-block;font:40px/40px CaptureSource;color:black}}
                    @media print{{@font-face{{font-family:{print_family};src:url({base}/font.ttf)}}
                        #probe{{font-family:{print_family}}}}}
                </style>`;
                document.body.innerHTML='<span id=probe>AAAA</span>';
                await document.fonts.load('40px CaptureSource','AAAA');
                await document.fonts.ready;
                window.liveFonts=document.fonts;window.liveReady=liveFonts.ready;
                window.screenFace=[...liveFonts][0];
                return JSON.stringify([probe.getBoundingClientRect().width,liveFonts.size,screenFace.status]);
            }})()"#);
            anyhow::ensure!(evaluate_print_fixture(&page,setup,true).await?==serde_json::json!("[160,1,\"loaded\"]"));
            anyhow::ensure!(requests.load(Ordering::SeqCst)==1,"screen must load the shared source once");
            let (reply, _) = page.run_async_command(RendererPageCommand::PrintToPdf(
                RendererPrintToPdfRequest::default(),
            )).await?;
            let RendererPageReply::PrintToPdf(RendererPrintToPdfReply::Printed(bytes)) = reply else {
                anyhow::bail!("cached source print must produce a PDF");
            };
            anyhow::ensure!(requests.load(Ordering::SeqCst)==1,
                "printing must reuse the loaded source instead of requesting its now-unavailable URL: {policy:?}, renamed={renamed}, requests={}",requests.load(Ordering::SeqCst));
            let pdf=lopdf::Document::load_mem(&bytes)?;
            anyhow::ensure!(pdf.extract_text(&[1])?.contains("AAAA"));
            let font_names=pdf.objects.values().filter_map(|object|
                object.as_dict().ok()?.get(b"FontName").ok()?.as_name().ok()
            ).collect::<Vec<_>>();
            anyhow::ensure!(font_names.len()==1 && font_names[0].ends_with(b"MoliAhem-Regular"),
                "the PDF must embed the loaded fixture font: {font_names:?}");
            assert_capture_font_pixel(&page,true).await?;
            anyhow::ensure!(requests.load(Ordering::SeqCst)==1,"raster print must share the same cached source");
            anyhow::ensure!(evaluate_print_fixture(&page,
                "JSON.stringify([probe.getBoundingClientRect().width,document.fonts===liveFonts,document.fonts.ready===liveReady,liveFonts.size,liveFonts.has(screenFace),screenFace.status,matchMedia('print').matches])".to_owned(),false
            ).await?==serde_json::json!("[160,true,true,1,true,\"loaded\",false]"),
                "temporary print declarations must not change live screen fonts");
            Ok::<_,anyhow::Error>(())
        }).await;
        page.close_async().await.unwrap();
        server.abort();
        let server_result = server.await;
        assert!(
            matches!(server_result,Err(error) if error.is_cancelled()),
            "font fixture must stay live until shutdown"
        );
        result.unwrap().unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn print_reuses_loaded_font_source_for_same_family() {
    assert_print_reuses_loaded_font_source(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn print_reuses_loaded_font_source_for_renamed_family() {
    assert_print_reuses_loaded_font_source(true).await;
}
