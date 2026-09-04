use anyhow::Result;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use moli_core::page::{
    Page, RendererPageDumpFormat, RendererPageDumpOptions, RendererPageDumpStripOptions,
    SubresourceResponseWaitCriteria,
};
use serde_json::{Map, Value, json};

use crate::{
    cli::StripOptions,
    network_trace::{NetworkTraceConfigSummary, NetworkTraceOptions, render_network_trace},
};

pub(super) async fn render_json(
    page: &mut Page,
    strip: StripOptions,
    with_base: bool,
    with_frames: bool,
    trace_network: bool,
    response_wait: Option<&SubresourceResponseWaitCriteria>,
    network_trace_config: Option<&NetworkTraceConfigSummary>,
    network_trace_options: NetworkTraceOptions,
) -> Result<String> {
    let html = render_html(page, strip, with_base, with_frames).await?;
    let network = if trace_network {
        let main_document_html = if html_dump_needs_dom_postprocess(strip, with_base, with_frames) {
            page.serialize_html_async().await?
        } else {
            html.clone()
        };
        Some(render_network_trace(
            page,
            &main_document_html,
            response_wait,
            network_trace_config,
            network_trace_options,
        ))
    } else {
        None
    };
    let title = page.document_title();
    render_json_payload(
        page.final_url().as_str(),
        page.status(),
        Some(&title),
        page.headers(),
        page.navigation_redirect_chain(),
        &html,
        network,
    )
}

pub(super) fn render_json_payload(
    final_url: &str,
    status: u16,
    title: Option<&str>,
    headers: &[(String, String)],
    redirect_chain: &[moli_core::page::NavigationRedirect],
    html: &str,
    network: Option<Value>,
) -> Result<String> {
    // Keep --dump json as a stable machine interface for scrapling-style callers.
    let mut payload = response_metadata(final_url, status, title, headers, redirect_chain);
    payload.insert("html".to_owned(), json!(html));
    if let Some(network) = network {
        payload.insert("network".to_owned(), network);
    }
    Ok(serde_json::to_string_pretty(&Value::Object(payload))?)
}

pub(super) fn render_raw_json_payload(
    final_url: &str,
    status: u16,
    headers: &[(String, String)],
    redirect_chain: &[moli_core::page::NavigationRedirect],
    body: &[u8],
) -> Result<String> {
    let mut payload = response_metadata(final_url, status, None, headers, redirect_chain);
    payload.insert("html".to_owned(), Value::Null);
    // Raw responses may contain arbitrary bytes. Always use one lossless
    // representation instead of changing the schema based on UTF-8 validity.
    payload.insert(
        "body_base64".to_owned(),
        Value::String(BASE64_STANDARD.encode(body)),
    );
    Ok(serde_json::to_string_pretty(&Value::Object(payload))?)
}

fn response_metadata(
    final_url: &str,
    status: u16,
    title: Option<&str>,
    headers: &[(String, String)],
    redirect_chain: &[moli_core::page::NavigationRedirect],
) -> Map<String, Value> {
    let mut payload = Map::new();
    payload.insert("final_url".to_owned(), json!(final_url));
    payload.insert("status".to_owned(), json!(status));
    payload.insert("title".to_owned(), json!(title));
    payload.insert("headers".to_owned(), render_headers(headers));
    payload.insert(
        "redirect_chain".to_owned(),
        Value::Array(
            redirect_chain
                .iter()
                .map(render_navigation_redirect)
                .collect(),
        ),
    );
    payload
}

fn render_headers(headers: &[(String, String)]) -> Value {
    Value::Array(
        headers
            .iter()
            .map(|(name, value)| json!({ "name": name, "value": value }))
            .collect(),
    )
}

fn render_navigation_redirect(redirect: &moli_core::page::NavigationRedirect) -> Value {
    json!({
        "from_url": redirect.from_url.as_str(),
        "to_url": redirect.to_url.as_str(),
        "status": redirect.status,
        "headers": render_headers(&redirect.headers),
    })
}

pub(super) async fn render_html(
    page: &mut Page,
    strip: StripOptions,
    with_base: bool,
    with_frames: bool,
) -> Result<String> {
    if !html_dump_needs_dom_postprocess(strip, with_base, with_frames) {
        return page.serialize_html_async().await;
    }

    page.render_page_dump_async(RendererPageDumpOptions {
        format: RendererPageDumpFormat::Html,
        strip: renderer_strip_options(strip),
        with_base,
        with_frames,
    })
    .await
}

fn html_dump_needs_dom_postprocess(
    strip: StripOptions,
    with_base: bool,
    with_frames: bool,
) -> bool {
    strip.js || strip.ui || strip.css || with_base || with_frames
}

pub(super) async fn render_markdown(
    page: &mut Page,
    strip: StripOptions,
    with_frames: bool,
) -> Result<String> {
    page.render_page_dump_async(RendererPageDumpOptions {
        format: RendererPageDumpFormat::Markdown,
        strip: renderer_strip_options(strip),
        with_base: false,
        with_frames,
    })
    .await
}

fn renderer_strip_options(strip: StripOptions) -> RendererPageDumpStripOptions {
    RendererPageDumpStripOptions {
        js: strip.js,
        ui: strip.ui,
        css: strip.css,
    }
}
