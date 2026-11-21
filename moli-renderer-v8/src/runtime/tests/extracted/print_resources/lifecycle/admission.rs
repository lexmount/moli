use super::*;
use crate::runtime::{RendererPrintToPdfReply, RendererPrintToPdfRequest};

#[tokio::test(flavor = "multi_thread")]
async fn print_local_resources_keep_the_default_network_mask_disabled() {
    use base64::Engine;
    for resource in [PrintOnlyResource::Font, PrintOnlyResource::Image] {
        let runtime = JsRuntime::initialize();
        runtime
            .renderer_owner_handle()
            .configure_layout_policy(LayoutPolicy::OnDemand)
            .unwrap();
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let mask = loader.optional_resource_fetch_mask();
        assert!(!mask.intersects(
            crate::protocol_types::OptionalResourceFetchMask::FONT
                | crate::protocol_types::OptionalResourceFetchMask::IMAGE
        ));
        let mut server = CaptureResourceServer::new(resource, CaptureResponse::Ready).await;
        let mut page = fixture(&runtime, &loader, &server, resource, false)
            .await
            .unwrap();
        let result = tokio::time::timeout(CASE_TIMEOUT, async {
            let (_, mime, bytes) = resource.resource();
            let url = format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            );
            let css = serde_json::to_string(&resource.css(&url))?;
            evaluate_print_fixture(
                &page,
                format!("document.adoptedStyleSheets[0].replaceSync({css});true"),
                false,
            )
            .await?;
            let baseline = screen_baseline(&page).await?;
            server.ensure_no_request()?;
            let (reply, _) = page.run_async_command(capture_command()).await?;
            let RendererPageReply::CaptureScreenshot(RendererCaptureScreenshotReply::Captured(
                bytes,
            )) = reply
            else {
                anyhow::bail!("local-resource Print must produce owned image bytes");
            };
            anyhow::ensure!((bytes.width, bytes.height) == (128, 96));
            let expected = match resource {
                PrintOnlyResource::Font => [0, 0, 0, 255],
                PrintOnlyResource::Image => [17, 85, 221, 255],
            };
            anyhow::ensure!(
                decoded_png_pixel(&bytes.bytes, 12, 12) == expected,
                "local resources must be present in the first print reply"
            );
            pending_zero(&page).await?;
            server.ensure_no_request()?;
            screen_unchanged(&page, baseline).await?;
            anyhow::ensure!(loader.optional_resource_fetch_mask() == mask);
            Ok::<_, anyhow::Error>(())
        })
        .await;
        cleanup(&mut page, server, result).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn print_background_policy_excludes_unneeded_and_disabled_manifest_entries() {
    let runtime = JsRuntime::initialize();
    runtime
        .renderer_owner_handle()
        .configure_layout_policy(LayoutPolicy::OnDemand)
        .unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    assert_eq!(
        loader.optional_resource_fetch_mask(),
        crate::protocol_types::OptionalResourceFetchMask::NONE
    );
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url::Url::parse("https://print-budget.test/page").unwrap(),
        "<!doctype html><body>print budget</body>",
    )
    .await;
    let result = tokio::time::timeout(CASE_TIMEOUT, async {
        evaluate_print_fixture(&page, r#"(() => {
            const sheet=new CSSStyleSheet();
            sheet.replaceSync('@media print { .asset { display:block;height:1px } }');
            document.adoptedStyleSheets=[sheet];
            document.body.innerHTML=Array.from({length:65}, (_, i)=>
              `<div class=asset style="background-image:url(https://example.test/bg-${i}.png)">x</div>`).join('');
            return true;
        })()"#.to_owned(), false).await?;
        for print_background in [false, true] {
            let (reply, _) = page.run_async_command(RendererPageCommand::PrintToPdf(RendererPrintToPdfRequest { print_background, ..Default::default() })).await?;
            anyhow::ensure!(matches!(reply, RendererPageReply::PrintToPdf(RendererPrintToPdfReply::Printed(_))), "disabled/unrequested images cannot exhaust the manifest: print_background={print_background}");
        }
        pending_zero(&page).await?;
        Ok::<_, anyhow::Error>(())
    }).await;
    page.close_async().await.unwrap();
    result.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn print_background_false_does_not_wait_for_a_gated_background() {
    assert_background_false_does_not_wait_for_a_gated_background(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn print_raster_background_false_does_not_wait_for_a_gated_background() {
    assert_background_false_does_not_wait_for_a_gated_background(true).await;
}

async fn assert_background_false_does_not_wait_for_a_gated_background(raster: bool) {
    let runtime = JsRuntime::initialize();
    runtime
        .renderer_owner_handle()
        .configure_layout_policy(LayoutPolicy::OnDemand)
        .unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_optional_resource_fetch_mask(PrintOnlyResource::Image.mask());
    let mut server =
        CaptureResourceServer::new(PrintOnlyResource::Image, CaptureResponse::Ready).await;
    let mut page = fixture(&runtime, &loader, &server, PrintOnlyResource::Image, false)
        .await
        .unwrap();
    let result = tokio::time::timeout(CASE_TIMEOUT, async {
        let command = if raster {
            RendererPageCommand::CaptureScreenshot(RendererCaptureScreenshotRequest {
                purpose: RendererScreenshotPurpose::Print { print_background: false },
                ..RendererCaptureScreenshotRequest::viewport_png()
            })
        } else {
            RendererPageCommand::PrintToPdf(RendererPrintToPdfRequest {
                print_background: false,
                ..Default::default()
            })
        };
        let mut print = Box::pin(page.run_async_command(command));
        tokio::select! {
            biased;
            request = server.requests.recv() => anyhow::bail!("unrequested background reached the gated server: {request:?}"),
            reply = &mut print => {
                let (reply, _) = reply?;
                match reply {
                    RendererPageReply::PrintToPdf(RendererPrintToPdfReply::Printed(bytes)) if !raster => {
                        anyhow::ensure!(bytes.starts_with(b"%PDF-"));
                    }
                    RendererPageReply::CaptureScreenshot(RendererCaptureScreenshotReply::Captured(image)) if raster => {
                        anyhow::ensure!((image.width, image.height) == (128, 96));
                        anyhow::ensure!(!image.bytes.is_empty());
                    }
                    _ => anyhow::bail!("print must produce the requested output"),
                }
            }
        }
        server.ensure_no_request()?;
        pending_zero(&page).await?;
        Ok::<_, anyhow::Error>(())
    }).await;
    page.close_async().await.unwrap();
    server.task.abort();
    let _ = server.task.await;
    result.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn print_background_false_keeps_mask_using_the_same_image_url() {
    let runtime = JsRuntime::initialize();
    runtime
        .renderer_owner_handle()
        .configure_layout_policy(LayoutPolicy::OnDemand)
        .unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_optional_resource_fetch_mask(PrintOnlyResource::Image.mask());
    let mut server =
        CaptureResourceServer::new(PrintOnlyResource::Image, CaptureResponse::Ready).await;
    let mut page = fixture(&runtime, &loader, &server, PrintOnlyResource::Image, false)
        .await
        .unwrap();
    let result = tokio::time::timeout(CASE_TIMEOUT, async {
        let rule=format!("@media print{{#asset{{mask-image:url({:?})}}}}",format!("{}{}",server.base,server.path));
        evaluate_print_fixture(&page,format!("document.adoptedStyleSheets[0].insertRule({},document.adoptedStyleSheets[0].cssRules.length);document.getElementById('asset').textContent='masked';true",serde_json::to_string(&rule)?),false).await?;
        let mut print=Box::pin(page.run_async_command(RendererPageCommand::PrintToPdf(RendererPrintToPdfRequest { print_background:false,..Default::default() })));
        tokio::select! {
            _completed=&mut print => anyhow::bail!("mask resource was not prepared"),
            seen=server.wait_request() => seen?,
        }
        server.release_and_observe().await?;
        let (reply, _)=print.await?;
        anyhow::ensure!(matches!(reply,RendererPageReply::PrintToPdf(RendererPrintToPdfReply::Printed(_))),"print must complete with a PDF");
        server.ensure_no_request()?;
        pending_zero(&page).await?;
        Ok::<_, anyhow::Error>(())
    }).await;
    page.close_async().await.unwrap();
    server.task.abort();
    let _ = server.task.await;
    result.unwrap().unwrap();
}
