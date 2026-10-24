use super::*;
use crate::runtime::{
    RendererCaptureScreencastFrameReply, RendererCaptureScreencastFrameRequest,
    RendererScreenshotFormat,
};

#[tokio::test(flavor = "current_thread")]
async fn screencast_reuses_one_raster_for_image_and_video() -> anyhow::Result<()> {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), Url::parse("https://raw-video.test/")?);
        page.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: 100, inner_height: 60, ..Default::default()
        }))?;
        page.vm_mut().eval("document.head.innerHTML='<style>html,body{margin:0;width:100%;height:100%;background:red}</style>'")?;
        let request = |known_visual_state| RendererCaptureScreencastFrameRequest {
            capture_video: true, format: RendererScreenshotFormat::Png, quality: 100, optimize_for_speed: true,
            base_background_color: [255; 4], max_width: Some(50), max_height: Some(30), known_visual_state,
        };
        let publishes_before = page.vm().layout_snapshot_cache_observability_for_test().2;
        let before = std::time::Instant::now();
        let RendererCaptureScreencastFrameReply::Captured(first) = page.capture_screencast_frame(request(None))? else {
            anyhow::bail!("initial raw frame");
        };
        assert_eq!(first.viewport_size, (100, 60));
        let raw = first.video_frame.as_ref().expect("raw screencast payload");
        assert_eq!((raw.image.width, raw.image.height), (50, 30));
        assert_eq!(&raw.image.rgba[..4], &[255, 0, 0, 255]);
        assert_eq!(moli_image::decode_png(&first.image.bytes)?, raw.image);
        assert_eq!(page.vm().layout_snapshot_cache_observability_for_test().2, publishes_before + 1);
        assert!(raw.captured_at >= before && raw.captured_at <= std::time::Instant::now());
        let publishes = page.vm().layout_snapshot_cache_observability_for_test().2;
        assert!(matches!(page.capture_screencast_frame(request(Some(first.visual_state.clone())))?, RendererCaptureScreencastFrameReply::Unchanged));
        assert_eq!(page.vm().layout_snapshot_cache_observability_for_test().2, publishes);
        page.vm_mut().eval("document.documentElement.style.background='blue';document.body.style.background='blue'")?;
        let RendererCaptureScreencastFrameReply::Captured(changed) = page.capture_screencast_frame(request(Some(first.visual_state)))? else {
            anyhow::bail!("mutation must produce a new raw frame");
        };
        assert_eq!(&changed.video_frame.as_ref().unwrap().image.rgba[..4], &[0, 0, 255, 255]);
        Ok::<_, anyhow::Error>(())
    }).await
}
