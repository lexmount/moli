use anyhow::{Context, Result, bail};
use moli_core::page::{
    Page, RendererCaptureScreenshotReply, RendererCaptureScreenshotRequest, RendererPdfTextLayer,
    RendererScreenshotFormat, RendererScreenshotPurpose, RendererScreenshotRegion,
};

struct CapturedRaster {
    mime_type: String,
    width: u32,
    height: u32,
    bytes: Vec<u8>,
    text_layer: Option<RendererPdfTextLayer>,
}

pub(super) async fn render_screenshot(
    page: &mut Page,
    region: RendererScreenshotRegion,
    dump_mode: &str,
) -> Result<Vec<u8>> {
    let mut request = RendererCaptureScreenshotRequest::viewport_png();
    request.region = region;
    let image = capture_page_raster(page, request, dump_mode).await?;
    if image.mime_type != "image/png" {
        bail!(
            "screenshot renderer returned `{}` instead of PNG",
            image.mime_type
        );
    }
    Ok(image.bytes)
}

pub(super) async fn render_pdf(page: &mut Page) -> Result<Vec<u8>> {
    let image = capture_page_raster(
        page,
        RendererCaptureScreenshotRequest {
            purpose: RendererScreenshotPurpose::Print {
                print_background: false,
            },
            format: RendererScreenshotFormat::Jpeg,
            quality: 90,
            region: RendererScreenshotRegion::FullDocument,
            optimize_for_speed: false,
            max_width: None,
            max_height: None,
        },
        "pdf",
    )
    .await?;
    if image.mime_type != "image/jpeg" {
        bail!(
            "PDF renderer returned `{}` instead of JPEG",
            image.mime_type
        );
    }
    moli_protocol::build_default_raster_pdf(
        &image.bytes,
        image.width,
        image.height,
        image.text_layer.as_ref(),
    )
    .context("failed to encode PDF output")
}

async fn capture_page_raster(
    page: &mut Page,
    request: RendererCaptureScreenshotRequest,
    dump_mode: &str,
) -> Result<CapturedRaster> {
    let pending = page
        .start_capture_screenshot_with_request(request)
        .with_context(|| format!("failed to start --dump {dump_mode} capture"))?;
    let completion = pending
        .wait()
        .await
        .with_context(|| format!("failed to complete --dump {dump_mode} capture"))?;
    match page.finish_capture_screenshot(completion)? {
        RendererCaptureScreenshotReply::Captured(image) => Ok(CapturedRaster {
            mime_type: image.mime_type,
            width: image.width,
            height: image.height,
            bytes: image.bytes.to_vec(),
            text_layer: image.text_layer,
        }),
        RendererCaptureScreenshotReply::LayoutDisabled => {
            bail!("--dump {dump_mode} requires --layout or MOLI_LAYOUT=true")
        }
        RendererCaptureScreenshotReply::NoDocument => {
            bail!("--dump {dump_mode} requires a loaded HTML document")
        }
    }
}
