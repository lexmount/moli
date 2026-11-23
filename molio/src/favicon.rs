use std::{io::Cursor, sync::Arc};

use gpui::RenderImage;
use image::{Frame, ImageReader, Limits, RgbaImage};
use moli_core::{RendererViewportNavigation, runtime::Browser};
use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, Request, RequestCredentialsMode, RequestMode,
    RequestResourceType, ensure_http_status_success,
};
use resvg::tiny_skia::{Pixmap, Transform};
use url::Url;

const ICON_PIXELS: u32 = 32;
const MAX_ICON_BYTES: usize = 2 * 1024 * 1024;

pub async fn load(
    browser: &Browser,
    navigation: &RendererViewportNavigation,
) -> Option<Arc<RenderImage>> {
    for url in navigation.favicon_urls.iter() {
        let Ok(bytes) = fetch_icon(browser, &navigation.url, url.clone()).await else {
            continue;
        };
        if let Ok(Some(icon)) = tokio::task::spawn_blocking(move || decode(&bytes)).await {
            return Some(icon);
        }
    }
    None
}

async fn fetch_icon(browser: &Browser, document: &Url, url: Url) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(matches!(url.scheme(), "http" | "https" | "data"));
    let request = Request::new_browser("GET", url, None, Vec::new(), document.into())
        .with_initiator_url(document)
        .without_inferred_referrer()
        .with_request_mode(RequestMode::NoCors)
        .with_credentials_mode(RequestCredentialsMode::Include)
        .with_browser_request_metadata(BrowserRequestMetadata::Image)
        .with_resource_type(RequestResourceType::Image)
        .with_page_network_policy();
    let mut response = browser
        .resource_request_client()
        .fetch_raw_stream_with_cancel(request, FetchCancelHandle::new())
        .await?;
    ensure_http_status_success(response.final_url.as_str(), response.status, false)?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.next_chunk().await {
        anyhow::ensure!(
            chunk.len() <= MAX_ICON_BYTES - bytes.len(),
            "favicon is too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    response.finish().await?;
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> Option<Arc<RenderImage>> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut image = if reader.format().is_some() {
        let mut limits = Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(4096);
        limits.max_alloc = Some(moli_image::MAX_DECODED_RGBA_BYTES as u64);
        reader.limits(limits);
        reader
            .decode()
            .ok()?
            .thumbnail(ICON_PIXELS, ICON_PIXELS)
            .into_rgba8()
    } else {
        rasterize_svg(bytes)?
    };
    for pixel in image.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new([Frame::new(image)])))
}

fn rasterize_svg(bytes: &[u8]) -> Option<RgbaImage> {
    // The web image parser resolves SVG with external images disabled.
    let image = moli_image::decode_svg_image(bytes).ok()?;
    let tree = image.tree();
    let size = tree.size();
    let scale = ICON_PIXELS as f32 / size.width().max(size.height());
    let width = (size.width() * scale).round().max(1.) as u32;
    let height = (size.height() * scale).round().max(1.) as u32;
    let mut pixmap = Pixmap::new(width, height)?;
    resvg::render(
        tree,
        Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect();
    RgbaImage::from_raw(width, height, rgba)
}
