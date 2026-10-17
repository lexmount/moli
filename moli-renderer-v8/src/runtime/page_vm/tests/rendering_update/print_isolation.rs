use super::*;
use crate::runtime::{
    RendererCaptureScreencastFrameReply, RendererCaptureScreenshotRequest, RendererPrintToPdfReply,
    RendererPrintToPdfRequest,
};

fn screen_token(page: &PageVm) -> crate::runtime::RendererVisualStateToken {
    page.vm().visual_state_token(
        page.document_lifecycle.identity(),
        moli_layout::PaintViewport::new(320, 100, 1.0),
        [255; 4],
    )
}

#[tokio::test(flavor = "current_thread")]
async fn native_pdf_preserves_live_fonts_and_screencast_input_identity() {
    run_page_vm_async_test(async move {
        use base64::Engine;

        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        loader.set_optional_resource_fetch_mask(crate::protocol_types::OptionalResourceFetchMask::FONT);
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://print-isolation.test/")?,
        );
        page.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: 320,
            inner_height: 100,
            ..Default::default()
        }))?;
        let font = base64::engine::general_purpose::STANDARD.encode(include_bytes!(
            "../../../../../../moli-layout/tests/fixtures/moli-ahem.woff2"
        ));
        page.vm_mut().eval(&format!(r#"
            document.head.innerHTML=`<style>
              body{{margin:0}} #target{{width:40px;height:40px}}
              .print{{display:none}}
              @media screen{{@font-face{{font-family:ScreenOnly;src:url(data:font/woff2;base64,{font})}}}}
              @media print{{
                @font-face{{font-family:PrintOnly;src:url(data:font/woff2;base64,{font})}}
                #target{{width:80px}} .screen{{display:none}} .print{{display:block}}
              }}
            </style>`;
            document.body.innerHTML='<div id=target><span class=screen>SCREEN</span><span class=print>PRINT</span></div>';
        "#))?;
        page.capture_screenshot(RendererCaptureScreenshotRequest::viewport_png())?;
        page.vm_mut().eval(r#"
            window.liveFonts=document.fonts;
            window.liveReady=liveFonts.ready;
            window.screenFace=[...liveFonts].find(face=>face.family==='ScreenOnly');
        "#)?;
        assert_eq!(page.vm_mut().eval("String(!!screenFace)")?, "true");
        let RendererCaptureScreencastFrameReply::Captured(first) = page
            .capture_screencast_frame(viewport_screencast_request(None))? else {
            anyhow::bail!("expected initial screen capture");
        };
        let original = screen_token(&page);
        assert_eq!(original, first.visual_state);
        let fonts = page.vm_mut().document_web_font_counts_for_test();
        let publishes = page.vm().layout_snapshot_cache_observability_for_test().2;
        let media = page.document_settings.emulated_media.clone();

        let RendererPrintToPdfReply::Printed(bytes) = page.print_to_pdf(Default::default())? else {
            anyhow::bail!("expected PDF");
        };
        let document = lopdf::Document::load_mem(&bytes)?;
        assert_eq!(
            document.extract_text(&document.get_pages().keys().copied().collect::<Vec<_>>())?
                .split_whitespace().collect::<Vec<_>>().join(" "),
            "PRINT"
        );
        assert_eq!(page.document_settings.emulated_media, media);
        assert_eq!(screen_token(&page), original);
        assert_eq!(page.vm_mut().document_web_font_counts_for_test(), fonts);
        assert_eq!(page.vm().layout_snapshot_cache_observability_for_test().2, publishes);
        assert_eq!(page.vm_mut().eval("JSON.stringify([document.fonts===liveFonts,liveFonts.has(screenFace),liveFonts.ready===liveReady,target.offsetWidth,matchMedia('print').matches])")?, "[true,true,true,40,false]");
        assert_eq!(screen_token(&page), original);
        assert!(matches!(
            page.capture_screencast_frame(viewport_screencast_request(Some(first.visual_state)))?,
            RendererCaptureScreencastFrameReply::Unchanged
        ));

        let mut invalid = RendererPrintToPdfRequest::default();
        invalid.options.width = 0.0;
        assert!(matches!(page.print_to_pdf(invalid)?, RendererPrintToPdfReply::Failed(_)));
        assert_eq!(screen_token(&page), original);
        assert_eq!(page.vm_mut().document_web_font_counts_for_test(), fonts);
        assert_eq!(page.document_settings.emulated_media, media);

        page.vm_mut().eval("target.style.width='60px'")?;
        let RendererCaptureScreencastFrameReply::Captured(changed) = page
            .capture_screencast_frame(viewport_screencast_request(Some(original.clone())))? else {
            anyhow::bail!("a real source mutation must invalidate screencast");
        };
        assert_ne!(changed.visual_state, original);
        assert_eq!(page.vm_mut().eval("String(target.offsetWidth)")?, "60");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("temporary print styles must preserve live screen inputs");
}
