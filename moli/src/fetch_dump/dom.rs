use anyhow::Result;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use moli_core::page::{
    Page, RendererContentStripOptions, RendererHtmlSerializeOptions, RendererMarkdownOptions,
    SubresourceResponseWaitCriteria,
};
use serde_json::{Map, Value, json};

use crate::{
    cli::StripOptions,
    network_trace::{
        NetworkTraceConfigSummary, NetworkTraceOptions, capture_network_activity,
        render_network_trace,
    },
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
    let activity = trace_network.then(|| {
        capture_network_activity(
            page,
            response_wait,
            network_trace_config,
            network_trace_options,
        )
    });
    let options = html_serialize_options(strip, with_base, with_frames);
    let document = page.serialize_document_async(options).await?;
    // Keep the metadata of this HTML reply even if obtaining an unfiltered
    // trace body below observes a later Document.
    let title = page.document_title();
    let mut payload = response_metadata(
        &document.url,
        page.status(),
        Some(&title),
        page.headers(),
        page.navigation_redirect_chain(),
    );
    if let Some(activity) = activity {
        let raw_html = if options != RendererHtmlSerializeOptions::default() {
            Some(page.serialize_html_async().await?)
        } else {
            None
        };
        let body = raw_html.as_deref().unwrap_or(&document.html);
        payload.insert(
            "network".to_owned(),
            render_network_trace(page, body, activity),
        );
    }
    payload.insert("html".to_owned(), Value::String(document.html));
    Ok(serde_json::to_string_pretty(&Value::Object(payload))?)
}

pub(super) fn render_raw_json_payload(
    final_url: &str,
    status: u16,
    headers: &[(String, Vec<u8>)],
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
    headers: &[(String, Vec<u8>)],
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

fn render_headers(headers: &[(String, Vec<u8>)]) -> Value {
    Value::Array(
        headers
            .iter()
            .map(|(name, value)| json!({ "name": name, "value": moli_fetch::decode_header_value(value) }))
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
    page.serialize_document_async(html_serialize_options(strip, with_base, with_frames))
        .await
        .map(|document| document.html)
}

fn html_serialize_options(
    strip: StripOptions,
    with_base: bool,
    with_frames: bool,
) -> RendererHtmlSerializeOptions {
    RendererHtmlSerializeOptions {
        strip: renderer_strip_options(strip),
        with_base,
        with_frames,
        ..Default::default()
    }
}

pub(super) async fn render_markdown(
    page: &mut Page,
    strip: StripOptions,
    _with_frames: bool,
) -> Result<String> {
    page.render_markdown_async(RendererMarkdownOptions {
        strip: renderer_strip_options(strip),
    })
    .await
}

fn renderer_strip_options(strip: StripOptions) -> RendererContentStripOptions {
    RendererContentStripOptions {
        js: strip.js,
        ui: strip.ui,
        css: strip.css,
    }
}
