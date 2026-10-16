use std::sync::Arc;

use anyhow::Result;
use axum::{Router, response::Html, routing::get};
use moli_core::{
    LayoutPolicy,
    page::Page,
    runtime::{Browser, BrowserConfig},
};
use serde_json::{Value, json};
use tokio::{net::TcpListener, task::JoinHandle};

use super::{
    render_page_dump_async, render_page_dump_with_options_async, render_page_output_async,
};
use crate::{
    cli::{DumpFormat, StripOptions},
    config::FetchCommandConfig,
};

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
                async move { Html((*body).clone()) }
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

    // Full-document output must work before any other capture publishes layout.
    let full_page = render_page_output_async(
        &mut page,
        &FetchCommandConfig {
            dump_mode: Some(DumpFormat::ScreenshotFull),
            ..FetchCommandConfig::default()
        },
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

    let viewport_dimensions = png_dimensions(&viewport);
    let full_page_dimensions = png_dimensions(&full_page);
    assert_eq!(full_page_dimensions.1, 1300);
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
async fn strip_ui_keeps_content_inside_details() -> Result<()> {
    let (_browser, mut page, http_server) = load_page(
        "<!doctype html><html><body><nav><details><summary>Menu</summary><p>Navigation entry</p></details></nav><main><details><summary>Product details</summary><p>Critical product description</p><button>Buy now</button></details><details open><summary>Specifications</summary><p>Material: cotton</p></details></main></body></html>",
    )
    .await?;
    let rendered = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::Markdown,
        StripOptions {
            js: false,
            css: false,
            ui: true,
        },
        false,
        false,
        false,
        None,
    )
    .await?;
    assert!(!rendered.contains("Menu"));
    assert!(!rendered.contains("Navigation entry"));
    assert!(!rendered.contains("Buy now"));
    assert!(rendered.contains("Product details"));
    assert!(rendered.contains("Critical product description"));
    assert!(rendered.contains("Specifications"));
    assert!(rendered.contains("Material: cotton"));
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
    let (_browser, mut page, http_server) =
        load_page(r#"<!doctype html><html><body><button id="target">old</button></body></html>"#)
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
async fn semantic_dump_preserves_protocol_json_bytes_and_text_fields() -> Result<()> {
    let (_browser, mut page, http_server) = load_page(
        r#"<!doctype html><html><head><title>AX serialization</title></head><body>
          <ul><li>List item</li></ul>
          <input aria-label="Text field" value="field value">
          <input type="range" aria-label="Numeric field" value="42">
          <button disabled>Disabled action</button>
          <div aria-hidden="true"><span>Ignored text</span></div>
          <div hidden>Not rendered</div>
        </body></html>"#,
    )
    .await?;
    let protocol = page
        .accessibility_tree_payloads_for_document_async(None)
        .await?;
    let json = super::semantic::render_json(&mut page, false).await?;
    assert_eq!(json, serde_json::to_string_pretty(&protocol)?);
    let nodes: Vec<serde_json::Value> = serde_json::from_str(&json)?;
    let field_names = |node: &serde_json::Value| {
        node.as_object()
            .expect("AX object")
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(
        field_names(&nodes[0]),
        [
            "nodeId",
            "backendDOMNodeId",
            "ignored",
            "role",
            "name",
            "properties",
            "childIds",
        ]
    );
    let marker = nodes
        .iter()
        .find(|node| node["role"]["value"] == "ListMarker")
        .expect("list marker");
    assert_eq!(
        field_names(marker),
        [
            "nodeId",
            "ignored",
            "role",
            "name",
            "properties",
            "parentId",
            "childIds",
        ]
    );
    let text = super::semantic::render_text(&mut page, false).await?;
    assert!(text.contains("textbox: Text field = field value [backendNodeId="));
    assert!(text.contains("textbox: Numeric field = 42 [backendNodeId="));
    assert!(
        !text.contains("ListMarker"),
        "markers have no child reference in the protocol tree"
    );
    assert!(!text.contains("Not rendered"));
    http_server.abort();
    Ok(())
}

#[tokio::test]
async fn render_semantic_tree_dumps_include_child_frames_only_when_requested() -> Result<()> {
    let (_browser, mut page, http_server) = load_page(
            r#"<!doctype html><html><body><iframe srcdoc="<button aria-label='Child action'>inside</button>"></iframe></body></html>"#,
        )
        .await?;

    let without_frames = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::SemanticTree,
        StripOptions::default(),
        false,
        false,
        false,
        None,
    )
    .await?;
    let with_frames = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::SemanticTree,
        StripOptions::default(),
        false,
        true,
        false,
        None,
    )
    .await?;

    assert!(!without_frames.contains("Child action"));
    assert!(with_frames.contains("Child action"));

    let payloads: Vec<Value> = serde_json::from_str(&with_frames)?;
    let child_root = payloads
        .iter()
        .filter(|payload| payload["role"]["value"] == "RootWebArea")
        .nth(1)
        .expect("child frame RootWebArea");
    assert_eq!(
        child_root
            .as_object()
            .expect("child root object")
            .keys()
            .next_back()
            .map(String::as_str),
        Some("parentId"),
        "frame attachment must preserve the existing JSON field order"
    );
    let child_root_id = child_root["nodeId"].as_str().expect("child root nodeId");
    let owner_id = child_root["parentId"]
        .as_str()
        .expect("child root should be attached to its iframe owner");
    let owner = payloads
        .iter()
        .find(|payload| payload["nodeId"] == owner_id)
        .expect("iframe owner AX node");
    assert_eq!(owner["role"]["value"], "Iframe");
    assert!(
        owner["childIds"]
            .as_array()
            .expect("iframe childIds")
            .iter()
            .any(|child_id| child_id == child_root_id)
    );

    let text_without_frames = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::SemanticTreeText,
        StripOptions::default(),
        false,
        false,
        false,
        None,
    )
    .await?;
    let text_with_frames = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::SemanticTreeText,
        StripOptions::default(),
        false,
        true,
        false,
        None,
    )
    .await?;

    assert!(!text_without_frames.contains("Child action"));
    assert!(text_with_frames.contains("Child action"));

    http_server.abort();
    Ok(())
}

#[tokio::test]
async fn render_semantic_tree_with_frames_recurses_into_nested_frames() -> Result<()> {
    let (_browser, mut page, http_server) = load_page(
            r#"<!doctype html><html><body><iframe srcdoc="<iframe srcdoc='&lt;button aria-label=&quot;Nested action&quot;&gt;inside&lt;/button&gt;'></iframe>"></iframe></body></html>"#,
        )
        .await?;

    let rendered = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::SemanticTreeText,
        StripOptions::default(),
        false,
        true,
        false,
        None,
    )
    .await?;

    assert!(rendered.contains("Nested action"));
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
async fn html_with_frames_preserves_shadow_content_and_strips_only_the_export() -> Result<()> {
    let (_browser, mut page, server) = load_page(
        r#"<!doctype html><iframe id="child" srcdoc="<body>Light body</body>"></iframe>"#,
    )
    .await?;
    page.evaluate_runtime_expression_with_await_async(
        r#"(() => {
          const doc = document.getElementById('child').contentDocument;
          for (const mode of ['open', 'closed']) {
            const host = doc.body.appendChild(doc.createElement('section'));
            const root = host.attachShadow({mode});
            root.innerHTML = `<p style="color:red" onclick="void 0">${mode} shadow body</p><style>.shadow-style-marker{color:red}</style><script>/* shadow-script-marker */</script>`;
          }
        })()"#,
        true,
    ).await?;
    for strip in [
        StripOptions::default(),
        StripOptions {
            js: true,
            css: true,
            ui: false,
        },
    ] {
        let rendered = render_page_dump_with_options_async(
            &mut page,
            DumpFormat::Html,
            strip,
            false,
            true,
            false,
            None,
        )
        .await?;
        for mode in ["open", "closed"] {
            assert!(
                rendered.contains(&format!("{mode} shadow body")),
                "{rendered}"
            );
            assert!(
                rendered.contains(&format!("shadowrootmode=&quot;{mode}&quot;")),
                "{rendered}"
            );
        }
        assert!(rendered.contains("Light body"));
        assert_eq!(rendered.contains("shadow-script-marker"), !strip.js);
        assert_eq!(rendered.contains("shadow-style-marker"), !strip.css);
        assert_eq!(rendered.contains("onclick="), !strip.js);
        assert_eq!(rendered.contains("style="), !strip.css);
    }
    let live = page.evaluate_runtime_expression_with_await_async(
        "document.getElementById('child').contentDocument.querySelector('section').shadowRoot.innerHTML",
        true,
    ).await?;
    assert!(live.to_string().contains("shadow-script-marker"));
    assert!(live.to_string().contains("shadow-style-marker"));
    server.abort();
    Ok(())
}

#[tokio::test]
async fn html_with_frames_preserves_each_documents_scripting_serialization() -> Result<()> {
    for sandbox in ["", "sandbox='allow-same-origin'"] {
        let (_browser, mut page, server) = load_page(&format!(
            r#"<!doctype html><iframe id="child" {sandbox} srcdoc="<body></body>"></iframe>"#,
        ))
        .await?;
        page.evaluate_runtime_expression_with_await_async(
            "(() => { const doc = document.getElementById('child').contentDocument;
               const node = doc.body.appendChild(doc.createElement('noscript'));
               node.textContent = '<literal-marker>'; })()",
            true,
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
        let expected = if sandbox.is_empty() {
            "&lt;noscript&gt;&lt;literal-marker&gt;&lt;/noscript&gt;"
        } else {
            "&lt;noscript&gt;&amp;lt;literal-marker&amp;gt;&lt;/noscript&gt;"
        };
        assert!(rendered.contains(expected), "{rendered}");
        server.abort();
    }
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
                env_proxy_fallback: false,
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
    assert_eq!(payload["network"]["config"]["env_proxy_fallback"], false);
    assert_eq!(payload["network"]["config"]["proxy_bearer_token"], true);
    assert_eq!(payload["network"]["config"]["connect_timeout_ms"], 2500);
    http_server.abort();
    Ok(())
}

#[tokio::test]
async fn markdown_with_frames_preserves_document_positions_nested_and_cross_origin_content()
-> Result<()> {
    let (_other_browser, other_page, other_server) = load_page("<base href='/assets/'><p>Cross origin body</p><a href='./details'>Foreign link</a><img src='./cover.png' alt='Foreign image'>").await?;
    let cross_url = other_page.final_url().to_string();
    let html = format!(
        r#"<p>Before</p><iframe srcdoc="&lt;p&gt;Child body&lt;/p&gt;&lt;a href='./local-details'&gt;Local link&lt;/a&gt;&lt;iframe srcdoc='&amp;lt;p&amp;gt;Nested body&amp;lt;/p&amp;gt;'&gt;&lt;/iframe&gt;"></iframe><p>Between</p><iframe src="{cross_url}"></iframe><p>After</p><iframe srcdoc=""></iframe>"#
    );
    let (_browser, mut page, server) = load_page(&html).await?;
    let parent_url = page.final_url().to_string();
    let plain = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::Markdown,
        StripOptions::default(),
        false,
        false,
        false,
        None,
    )
    .await?;
    assert!(!plain.contains("Child body") && !plain.contains("Cross origin body"));
    let framed = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::Markdown,
        StripOptions::default(),
        false,
        true,
        false,
        None,
    )
    .await?;
    let positions = [
        "Before",
        "Child body",
        "Nested body",
        "Between",
        "Cross origin body",
        "After",
        "Embedded content",
    ]
    .map(|text| {
        framed
            .find(text)
            .unwrap_or_else(|| panic!("missing {text}: {framed}"))
    });
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "{framed}"
    );
    assert!(
        framed.contains(&format!("[Local link]({}local-details)", parent_url)),
        "{framed}"
    );
    assert!(
        framed.contains(&format!("[Foreign link]({}assets/details)", cross_url)),
        "{framed}"
    );
    assert!(
        framed.contains(&format!("![Foreign image]({}assets/cover.png)", cross_url)),
        "{framed}"
    );
    server.abort();
    other_server.abort();
    Ok(())
}

#[tokio::test]
async fn markdown_with_frames_uses_latest_frame_document_and_links_for_failures() -> Result<()> {
    let (_browser, mut page, server) = load_page(r#"<p>Parent</p><iframe id="child" srcdoc="&lt;p&gt;Old body&lt;/p&gt;"></iframe><iframe title="Unavailable" src="http://127.0.0.1:1/missing"></iframe>"#).await?;
    page.evaluate_runtime_expression_with_await_async("new Promise(resolve => { const frame = document.getElementById('child'); frame.onload=resolve; frame.srcdoc='<p>Latest body</p>'; })", true).await?;
    let text = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::Markdown,
        StripOptions::default(),
        false,
        true,
        false,
        None,
    )
    .await?;
    assert!(
        text.contains("Latest body") && !text.contains("Old body"),
        "{text}"
    );
    assert!(
        text.contains("[Unavailable](http://127.0.0.1:1/missing)"),
        "{text}"
    );
    server.abort();
    Ok(())
}

#[tokio::test]
async fn iframe_media_urls_use_their_own_document_base() -> Result<()> {
    let (_browser, mut page, server) = load_page(r#"<base href="https://parent.example/page/"><video poster="parent.jpg" src="parent.mp4"></video><iframe srcdoc="&lt;base href='https://cdn.example/assets/'&gt;&lt;video poster='cover.jpg' src='movie.mp4'&gt;&lt;/video&gt;"></iframe>"#).await?;
    let rendered = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::Markdown,
        StripOptions::default(),
        false,
        true,
        false,
        None,
    )
    .await?;
    server.abort();
    assert!(
        rendered.contains("![Video poster](https://cdn.example/assets/cover.jpg)"),
        "{rendered}"
    );
    assert!(
        rendered.contains("[Video](https://cdn.example/assets/movie.mp4)"),
        "{rendered}"
    );
    assert!(
        rendered.contains("![Video poster](https://parent.example/page/parent.jpg)"),
        "{rendered}"
    );
    assert!(
        rendered.contains("[Video](https://parent.example/page/parent.mp4)"),
        "{rendered}"
    );
    Ok(())
}
