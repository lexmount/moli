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
