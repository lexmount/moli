//! PNG screenshot exports from the renderer capture API.

use anyhow::{Context, Result, bail};
use moli_core::page::{
    Page, RendererCaptureScreenshotReply, RendererCaptureScreenshotRequest,
    RendererScreenshotRegion,
};

struct CapturedScreenshot {
    mime_type: String,
    bytes: Vec<u8>,
}

pub(super) async fn render_screenshot(
    page: &mut Page,
    region: RendererScreenshotRegion,
    dump_mode: &str,
) -> Result<Vec<u8>> {
    let mut request = RendererCaptureScreenshotRequest::viewport_png();
    request.region = region;
    let image = capture_page_screenshot(page, request, dump_mode).await?;
    if image.mime_type != "image/png" {
        bail!(
            "screenshot renderer returned `{}` instead of PNG",
            image.mime_type
        );
    }
    Ok(image.bytes)
}

async fn capture_page_screenshot(
    page: &mut Page,
    request: RendererCaptureScreenshotRequest,
    dump_mode: &str,
) -> Result<CapturedScreenshot> {
    let pending = page
        .start_capture_screenshot_with_request(request)
        .with_context(|| format!("failed to start --dump {dump_mode} capture"))?;
    let completion = pending
        .wait()
        .await
        .with_context(|| format!("failed to complete --dump {dump_mode} capture"))?;
    match page.finish_capture_screenshot(completion)? {
        RendererCaptureScreenshotReply::Captured(image) => Ok(CapturedScreenshot {
            mime_type: image.mime_type,
            bytes: image.bytes.to_vec(),
        }),
        RendererCaptureScreenshotReply::LayoutDisabled => {
            bail!(
                "--dump {dump_mode} requires --layout or --realtime (MOLI_LAYOUT=true or MOLI_REALTIME=true)"
            )
        }
        RendererCaptureScreenshotReply::NoDocument => {
            bail!("--dump {dump_mode} requires a loaded HTML document")
        }
    }
}
