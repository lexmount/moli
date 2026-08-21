use anyhow::{Context, Result, bail};
use moli_core::page::{
    DocumentNodeSnapshot, Page, RendererCaptureScreenshotReply, RendererCaptureScreenshotRequest,
    RendererPageDumpFormat, RendererPageDumpOptions, RendererPageDumpStripOptions,
    RendererScreenshotFormat, RendererScreenshotPurpose, RendererScreenshotRegion,
    SubresourceResponseWaitCriteria, is_renderer_backend_node_id,
};
use moli_core::runtime::RawDocument;
#[cfg(test)]
use moli_core::{LayoutPolicy, runtime::Browser, runtime::BrowserConfig};
use serde_json::{Map, Value, json};

use crate::{
    cli::{DumpFormat, StripOptions},
    config::FetchCommandConfig,
    network_trace::{NetworkTraceConfigSummary, NetworkTraceOptions, render_network_trace},
};

struct CapturedRaster {
    mime_type: String,
    width: u32,
    height: u32,
    bytes: Vec<u8>,
}

pub async fn render_page_output_async(
    page: &mut Page,
    command: &FetchCommandConfig,
) -> Result<Vec<u8>> {
    match command.dump_mode.unwrap_or(DumpFormat::Html) {
        DumpFormat::Screenshot => {
            render_screenshot_dump_async(page, RendererScreenshotRegion::Viewport, "screenshot")
                .await
        }
        DumpFormat::ScreenshotFull => {
            render_screenshot_dump_async(
                page,
                RendererScreenshotRegion::FullDocument,
                "screenshot_full",
            )
            .await
        }
        DumpFormat::Pdf => render_pdf_dump_async(page).await,
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

pub fn render_raw_document_dump(
    raw: &RawDocument,
    command: &FetchCommandConfig,
) -> Result<Vec<u8>> {
    match command.dump_mode.unwrap_or(DumpFormat::Html) {
        DumpFormat::Html => Ok(raw.body_bytes().to_vec()),
        DumpFormat::Json => {
            let html = String::from_utf8_lossy(raw.body_bytes());
            let payload = render_json_payload(raw.final_url().as_str(), raw.status(), &html, None)?;
            Ok(payload.into_bytes())
        }
        DumpFormat::Markdown
        | DumpFormat::Screenshot
        | DumpFormat::ScreenshotFull
        | DumpFormat::Pdf
        | DumpFormat::SemanticTree
        | DumpFormat::SemanticTreeText => {
            anyhow::bail!("raw non-HTML document output only supports --dump html or --dump json")
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
            render_json_dump_async(
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
        DumpFormat::Html => render_html_dump_async(page, strip, with_base, with_frames).await,
        DumpFormat::Markdown => render_markdown_dump_async(page, strip, with_frames).await,
        DumpFormat::Screenshot | DumpFormat::ScreenshotFull | DumpFormat::Pdf => {
            bail!("binary dump formats are only supported by the fetch CLI output path")
        }
        DumpFormat::SemanticTree => render_semantic_tree_dump_async(page).await,
        DumpFormat::SemanticTreeText => render_semantic_tree_text_dump_async(page).await,
    }
}

async fn render_screenshot_dump_async(
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

async fn render_pdf_dump_async(page: &mut Page) -> Result<Vec<u8>> {
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
    moli_protocol::build_default_raster_pdf(&image.bytes, image.width, image.height)
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
        }),
        RendererCaptureScreenshotReply::LayoutDisabled => {
            bail!("--dump {dump_mode} requires --layout or MOLI_LAYOUT=true")
        }
        RendererCaptureScreenshotReply::NoDocument => {
            bail!("--dump {dump_mode} requires a loaded HTML document")
        }
    }
}

async fn render_json_dump_async(
    page: &mut Page,
    strip: StripOptions,
    with_base: bool,
    with_frames: bool,
    trace_network: bool,
    response_wait: Option<&SubresourceResponseWaitCriteria>,
    network_trace_config: Option<&NetworkTraceConfigSummary>,
    network_trace_options: NetworkTraceOptions,
) -> Result<String> {
    let html = render_html_dump_async(page, strip, with_base, with_frames).await?;
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
    render_json_payload(page.final_url().as_str(), page.status(), &html, network)
}

fn render_json_payload(
    final_url: &str,
    status: u16,
    html: &str,
    network: Option<Value>,
) -> Result<String> {
    // Keep --dump json as a small stable machine interface for scrapling-style callers.
    let mut payload = Map::new();
    payload.insert("final_url".to_owned(), json!(final_url));
    payload.insert("status".to_owned(), json!(status));
    payload.insert("html".to_owned(), json!(html));
    if let Some(network) = network {
        payload.insert("network".to_owned(), network);
    }
    Ok(serde_json::to_string_pretty(&Value::Object(payload))?)
}

async fn render_html_dump_async(
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

async fn render_markdown_dump_async(
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

async fn render_semantic_tree_dump_async(page: &mut Page) -> Result<String> {
    let payloads = page
        .accessibility_tree_payloads_for_document_async(None)
        .await?;
    Ok(serde_json::to_string_pretty(&payloads)?)
}

async fn render_semantic_tree_text_dump_async(page: &mut Page) -> Result<String> {
    let payloads = page
        .accessibility_tree_payloads_for_document_async(None)
        .await?;
    if payloads.is_empty() {
        return Ok(String::new());
    }

    let mut by_id = std::collections::HashMap::new();
    for payload in &payloads {
        if let Some(id) = payload.get("nodeId").and_then(Value::as_str) {
            by_id.insert(id.to_owned(), payload);
        }
    }

    let mut out = String::new();
    if let Some(root_id) = payloads[0].get("nodeId").and_then(Value::as_str) {
        render_semantic_node_text(root_id, &by_id, 0, &mut out);
    }
    Ok(out.trim_end().to_owned())
}

fn render_semantic_node_text(
    node_id: &str,
    by_id: &std::collections::HashMap<String, &Value>,
    depth: usize,
    out: &mut String,
) {
    let Some(payload) = by_id.get(node_id) else {
        return;
    };

    let role = payload["role"]["value"].as_str().unwrap_or("Unknown");
    let name = payload["name"]["value"].as_str().unwrap_or_default();
    let value = payload["value"]["value"].as_str().unwrap_or_default();
    let backend = payload["backendDOMNodeId"].as_u64().unwrap_or(0);

    out.push_str(&"  ".repeat(depth));
    out.push_str("- ");
    out.push_str(role);
    if !name.is_empty() {
        out.push_str(": ");
        out.push_str(name);
    }
    if !value.is_empty() {
        out.push_str(" = ");
        out.push_str(value);
    }
    if backend != 0 {
        out.push_str(&format!(" [backendNodeId={backend}]"));
    }
    out.push('\n');

    for child_id in payload["childIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        render_semantic_node_text(child_id, by_id, depth + 1, out);
    }
}

pub async fn summarize_node_details_async(page: &mut Page, backend_node_id: u32) -> Result<Value> {
    let snapshot = document_node_snapshot_for_backend_node_id(page, backend_node_id).await?;
    let Some(snapshot) = snapshot else {
        return Ok(json!({
            "error": format!("backendNodeId `{backend_node_id}` not found")
        }));
    };
    let accessibility = accessibility_node_payload_for_backend_node_id(page, backend_node_id)
        .await?
        .unwrap_or_else(|| json!({}));
    let mut options = Vec::new();
    if snapshot.local_name == "select" {
        for child in &snapshot.children {
            if child.local_name != "option" {
                continue;
            }
            options.push(json!({
                "text": child.children.iter().find(|grandchild| grandchild.node_name == "#text").map(|text| text.node_value.clone()).unwrap_or_default(),
                "value": attribute_value(child, "value").unwrap_or_default(),
                "selected": attribute_value(child, "selected").is_some(),
            }));
        }
    }

    Ok(json!({
        "backendNodeId": backend_node_id,
        "tag": snapshot.local_name.clone(),
        "role": accessibility["role"]["value"].as_str().unwrap_or_default(),
        "name": accessibility["name"]["value"].as_str().unwrap_or_default(),
        "value": accessibility["value"]["value"].as_str().unwrap_or_default(),
        "inputType": attribute_value(&snapshot, "type").unwrap_or_default(),
        "placeholder": attribute_value(&snapshot, "placeholder").unwrap_or_default(),
        "href": attribute_value(&snapshot, "href").unwrap_or_default(),
        "checked": attribute_value(&snapshot, "checked").is_some(),
        "disabled": attribute_value(&snapshot, "disabled").is_some(),
        "options": options,
    }))
}

async fn accessibility_node_payload_for_backend_node_id(
    page: &mut Page,
    backend_node_id: u32,
) -> Result<Option<Value>> {
    if backend_node_id == 0 || !is_renderer_backend_node_id(backend_node_id) {
        return Ok(None);
    }
    let pending = page.start_accessibility_node_payload_for_backend_node_id(backend_node_id)?;
    let completion = pending.wait().await?;
    Ok(page
        .finish_accessibility_payloads_for_backend_node_id(completion)?
        .and_then(|payloads| payloads.payloads)
        .and_then(|payloads| payloads.into_iter().next()))
}

async fn document_node_snapshot_for_backend_node_id(
    page: &mut Page,
    backend_node_id: u32,
) -> Result<Option<DocumentNodeSnapshot>> {
    const NODE_DETAILS_SNAPSHOT_DEPTH: i32 = 2;

    if backend_node_id == 0 {
        return Ok(None);
    }

    if !is_renderer_backend_node_id(backend_node_id) {
        return Ok(None);
    }

    let pending = page.start_document_node_snapshot_for_backend_node_id(
        backend_node_id,
        NODE_DETAILS_SNAPSHOT_DEPTH,
        false,
    )?;
    let completion = pending.wait().await?;
    let snapshot = page.finish_document_node_snapshot_for_backend_node_id(completion)?;
    Ok(snapshot.map(|snapshot| snapshot.snapshot))
}

fn attribute_value(snapshot: &DocumentNodeSnapshot, name: &str) -> Option<String> {
    snapshot
        .attributes
        .iter()
        .find(|attribute| attribute.local_name == name)
        .map(|attribute| attribute.value.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::get};
    use std::sync::Arc;
    use tokio::{net::TcpListener, task::JoinHandle};

    async fn load_page(html: &str) -> Result<(Browser, Page, JoinHandle<()>)> {
        load_page_with_config(html, BrowserConfig::default()).await
    }

    async fn load_page_with_config(
        html: &str,
        config: BrowserConfig,
    ) -> Result<(Browser, Page, JoinHandle<()>)> {
        let browser = Browser::new(config)?;
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let body = Arc::new(html.to_owned());
        let server_body = Arc::clone(&body);
        let http_server = tokio::spawn(async move {
            let app = Router::new().route(
                "/",
                get(move || {
                    let body = Arc::clone(&server_body);
                    async move { (*body).clone() }
                }),
            );
            axum::serve(listener, app).await.unwrap();
        });
        let page = browser.fetch(&format!("http://{addr}/")).await?;
        Ok((browser, page, http_server))
    }

    fn png_dimensions(bytes: &[u8]) -> (u32, u32) {
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(&bytes[12..16], b"IHDR");
        (
            u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
            u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
        )
    }

    #[tokio::test]
    async fn render_full_page_screenshot_extends_beyond_viewport() -> Result<()> {
        let config = BrowserConfig::default().with_layout_policy(LayoutPolicy::OnDemand);
        let (_browser, mut page, http_server) = load_page_with_config(
            concat!(
                "<!doctype html><style>html,body{margin:0}",
                "main{height:1300px;background:linear-gradient(red,blue)}</style>",
                "<main></main>",
            ),
            config,
        )
        .await?;

        let viewport = render_page_output_async(
            &mut page,
            &FetchCommandConfig {
                dump_mode: Some(DumpFormat::Screenshot),
                ..FetchCommandConfig::default()
            },
        )
        .await?;
        let full_page = render_page_output_async(
            &mut page,
            &FetchCommandConfig {
                dump_mode: Some(DumpFormat::ScreenshotFull),
                ..FetchCommandConfig::default()
            },
        )
        .await?;

        let viewport_dimensions = png_dimensions(&viewport);
        let full_page_dimensions = png_dimensions(&full_page);
        assert_eq!(full_page_dimensions.0, viewport_dimensions.0);
        assert!(full_page_dimensions.1 > viewport_dimensions.1);

        http_server.abort();
        Ok(())
    }

    #[tokio::test]
    async fn render_page_dump_default_html_uses_renderer_live_serialize() -> Result<()> {
        let (_browser, mut page, http_server) =
            load_page(r#"<!doctype html><html><body><main id="target">old</main></body></html>"#)
                .await?;

        let mutation = json!({
            "id": 17,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "document.getElementById('target').textContent = 'live'; 'done';",
                "returnByValue": true
            }
        });
        let pending = page.start_runtime_protocol_message(serde_json::to_string(&mutation)?)?;
        let completion = pending.wait().await?;

        let rendered = render_page_dump_with_options_async(
            &mut page,
            DumpFormat::Html,
            StripOptions::default(),
            false,
            false,
            false,
            None,
        )
        .await?;

        assert!(rendered.contains(r#"<main id="target">live</main>"#));
        assert!(!rendered.contains(r#"<main id="target">old</main>"#));

        let _ = page.finish_runtime_protocol_message(completion)?;
        http_server.abort();
        Ok(())
    }

    #[tokio::test]
    async fn render_page_dump_postprocessed_html_uses_renderer_live_dump() -> Result<()> {
        let (_browser, mut page, http_server) = load_page(
            r#"<!doctype html><html><body><script>window.old=true;</script><main id="target" style="color:red" onclick="old()">old</main></body></html>"#,
        )
        .await?;

        let mutation = json!({
            "id": 19,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "document.getElementById('target').textContent = 'live'; 'done';",
                "returnByValue": true
            }
        });
        let pending = page.start_runtime_protocol_message(serde_json::to_string(&mutation)?)?;
        let completion = pending.wait().await?;

        let rendered = render_page_dump_with_options_async(
            &mut page,
            DumpFormat::Html,
            StripOptions {
                js: true,
                css: true,
                ui: false,
            },
            true,
            false,
            false,
            None,
        )
        .await?;

        assert!(rendered.contains(r#"<main id="target">live</main>"#));
        assert!(rendered.contains("<base href="));
        assert!(
            !rendered.contains(r#"<main id="target" style="color:red" onclick="old()">old</main>"#)
        );
        assert!(!rendered.contains("<script>"));
        assert!(!rendered.contains("onclick="));
        assert!(!rendered.contains("style="));

        let _ = page.finish_runtime_protocol_message(completion)?;
        http_server.abort();
        Ok(())
    }

    #[tokio::test]
    async fn render_markdown_dump_uses_renderer_live_dump() -> Result<()> {
        let (_browser, mut page, http_server) =
            load_page(r#"<!doctype html><html><body><main id="target">old</main></body></html>"#)
                .await?;

        let mutation = json!({
            "id": 20,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "document.getElementById('target').textContent = 'live'; 'done';",
                "returnByValue": true
            }
        });
        let pending = page.start_runtime_protocol_message(serde_json::to_string(&mutation)?)?;
        let completion = pending.wait().await?;

        let rendered = render_page_dump_with_options_async(
            &mut page,
            DumpFormat::Markdown,
            StripOptions::default(),
            false,
            false,
            false,
            None,
        )
        .await?;

        assert_eq!(rendered, "live");
        assert!(!rendered.contains("old"));

        let _ = page.finish_runtime_protocol_message(completion)?;
        http_server.abort();
        Ok(())
    }

    #[tokio::test]
    async fn render_semantic_tree_dump_uses_renderer_live_accessibility_tree() -> Result<()> {
        let (_browser, mut page, http_server) = load_page(
            r#"<!doctype html><html><body><button id="target">old</button></body></html>"#,
        )
        .await?;

        let mutation = json!({
            "id": 18,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "document.getElementById('target').textContent = 'live'; 'done';",
                "returnByValue": true
            }
        });
        let pending = page.start_runtime_protocol_message(serde_json::to_string(&mutation)?)?;
        let completion = pending.wait().await?;

        let rendered = render_page_dump_with_options_async(
            &mut page,
            DumpFormat::SemanticTree,
            StripOptions::default(),
            false,
            false,
            false,
            None,
        )
        .await?;

        assert!(rendered.contains(r#""value": "live""#));
        assert!(!rendered.contains(r#""value": "old""#));

        let _ = page.finish_runtime_protocol_message(completion)?;
        http_server.abort();
        Ok(())
    }

    #[tokio::test]
    async fn render_page_dump_with_options_async_inlines_child_frames() -> Result<()> {
        let (_browser, mut page, http_server) = load_page(
            r#"<!doctype html><html><body><iframe id="child" srcdoc="<p>child frame</p>"></iframe></body></html>"#,
        )
        .await?;

        let rendered = render_page_dump_with_options_async(
            &mut page,
            DumpFormat::Html,
            StripOptions::default(),
            false,
            true,
            false,
            None,
        )
        .await?;

        assert!(rendered.contains("data-moli-frame-url="));
        assert!(rendered.contains("child frame"));
        http_server.abort();
        Ok(())
    }

    #[tokio::test]
    async fn render_page_dump_async_includes_network_trace_config_summary() -> Result<()> {
        let (_browser, mut page, http_server) =
            load_page(r#"<!doctype html><html><body>ok</body></html>"#).await?;

        let rendered = render_page_dump_async(
            &mut page,
            &FetchCommandConfig {
                dump_mode: Some(DumpFormat::Json),
                trace_network: true,
                network_trace_config: Some(crate::network_trace::NetworkTraceConfigSummary {
                    explicit_http_proxy: true,
                    libcurl_env_proxy_fallback: false,
                    http_no_proxy: true,
                    proxy_bearer_token: true,
                    tls_verify_host: true,
                    obey_robots: false,
                    http_cache: false,
                    connect_timeout_ms: Some(2500),
                    request_timeout_ms: 5000,
                    max_concurrent: Some(16),
                    max_host_open: Some(4),
                    max_host_connections: Some(6),
                    effective_max_host_connections: Some(6),
                    max_total_connections: Some(64),
                    http2_max_concurrent_streams: Some(100),
                    max_response_size: Some(1024),
                    block_private_networks: false,
                    block_cidr_count: 0,
                }),
                ..FetchCommandConfig::default()
            },
        )
        .await?;
        let payload: Value = serde_json::from_str(&rendered)?;

        assert_eq!(payload["network"]["config"]["explicit_http_proxy"], true);
        assert_eq!(
            payload["network"]["config"]["libcurl_env_proxy_fallback"],
            false
        );
        assert_eq!(payload["network"]["config"]["proxy_bearer_token"], true);
        assert_eq!(payload["network"]["config"]["connect_timeout_ms"], 2500);
        http_server.abort();
        Ok(())
    }
}
