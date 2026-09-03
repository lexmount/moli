mod dom;
mod node_details;
mod raster;
mod semantic;

#[cfg(test)]
mod tests;

use anyhow::{Result, bail};
use moli_core::{
    page::{Page, RendererScreenshotRegion, SubresourceResponseWaitCriteria},
    runtime::RawDocument,
};

use crate::{
    cli::{DumpFormat, StripOptions},
    config::FetchCommandConfig,
    network_trace::{NetworkTraceConfigSummary, NetworkTraceOptions},
};

pub use node_details::summarize_node_details_async;

pub async fn render_page_output_async(
    page: &mut Page,
    command: &FetchCommandConfig,
) -> Result<Vec<u8>> {
    match command.dump_mode.unwrap_or(DumpFormat::Html) {
        DumpFormat::Screenshot => {
            raster::render_screenshot(page, RendererScreenshotRegion::Viewport, "screenshot").await
        }
        DumpFormat::ScreenshotFull => {
            raster::render_screenshot(
                page,
                RendererScreenshotRegion::FullDocument,
                "screenshot_full",
            )
            .await
        }
        DumpFormat::Pdf => raster::render_pdf(page).await,
        _ => render_page_dump_async(page, command)
            .await
            .map(String::into_bytes),
    }
}

pub async fn render_page_dump_async(
    page: &mut Page,
    command: &FetchCommandConfig,
) -> Result<String> {
    let dump_mode = command.dump_mode.unwrap_or(DumpFormat::Html);
    render_page_dump_with_trace_config_async(
        page,
        dump_mode,
        command.strip,
        command.with_base,
        command.with_frames,
        command.trace_network,
        command.response_wait.as_ref(),
        command.network_trace_config.as_ref(),
        NetworkTraceOptions {
            include_matched_response_body: command.trace_matched_response_body,
        },
    )
    .await
}

pub fn render_raw_document_output(
    raw: &RawDocument,
    command: &FetchCommandConfig,
) -> Result<Vec<u8>> {
    match command.dump_mode {
        None => Ok(raw.body_bytes().to_vec()),
        Some(DumpFormat::Html) => {
            bail!("--dump html requires a renderable HTML document, not a raw download")
        }
        Some(DumpFormat::Json) => {
            let html = String::from_utf8_lossy(raw.body_bytes());
            let redirect_chain = raw
                .navigation_redirect_chain()
                .iter()
                .cloned()
                .map(Into::into)
                .collect::<Vec<_>>();
            let payload = dom::render_json_payload(
                raw.final_url().as_str(),
                raw.status(),
                None,
                raw.headers(),
                &redirect_chain,
                &html,
                None,
            )?;
            Ok(payload.into_bytes())
        }
        Some(
            DumpFormat::Markdown
            | DumpFormat::Screenshot
            | DumpFormat::ScreenshotFull
            | DumpFormat::Pdf
            | DumpFormat::SemanticTree
            | DumpFormat::SemanticTreeText,
        ) => {
            bail!("raw download output only supports automatic output or --dump json")
        }
    }
}

pub async fn render_page_dump_with_options_async(
    page: &mut Page,
    dump_mode: DumpFormat,
    strip: StripOptions,
    with_base: bool,
    with_frames: bool,
    trace_network: bool,
    response_wait: Option<&SubresourceResponseWaitCriteria>,
) -> Result<String> {
    render_page_dump_with_trace_config_async(
        page,
        dump_mode,
        strip,
        with_base,
        with_frames,
        trace_network,
        response_wait,
        None,
        NetworkTraceOptions::default(),
    )
    .await
}

async fn render_page_dump_with_trace_config_async(
    page: &mut Page,
    dump_mode: DumpFormat,
    strip: StripOptions,
    with_base: bool,
    with_frames: bool,
    trace_network: bool,
    response_wait: Option<&SubresourceResponseWaitCriteria>,
    network_trace_config: Option<&NetworkTraceConfigSummary>,
    network_trace_options: NetworkTraceOptions,
) -> Result<String> {
    match dump_mode {
        DumpFormat::Json => {
            dom::render_json(
                page,
                strip,
                with_base,
                with_frames,
                trace_network,
                response_wait,
                network_trace_config,
                network_trace_options,
            )
            .await
        }
        DumpFormat::Html => dom::render_html(page, strip, with_base, with_frames).await,
        DumpFormat::Markdown => dom::render_markdown(page, strip, with_frames).await,
        DumpFormat::Screenshot | DumpFormat::ScreenshotFull | DumpFormat::Pdf => {
            bail!("binary dump formats are only supported by the fetch CLI output path")
        }
        DumpFormat::SemanticTree => semantic::render_json(page, with_frames).await,
        DumpFormat::SemanticTreeText => semantic::render_text(page, with_frames).await,
    }
}
