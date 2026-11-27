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
    let config = BrowserConfig::default().with_layout_policy(LayoutPolicy::Lazy);
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
          <a href="docs/start.html#top">Docs</a>
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
    let link_url = page.final_url().join("docs/start.html#top")?;
    assert!(text.contains(&format!("link: Docs → {link_url} [backendNodeId=")));
    assert!(
        !text.contains("ListMarker"),
        "markers have no child reference in the protocol tree"
    );
    assert!(!text.contains("Not rendered"));
    http_server.abort();
    Ok(())
}

#[tokio::test]
async fn semantic_presentational_links_keep_native_roles_and_targets() -> Result<()> {
    let (_browser, mut page, server) = load_page(
        r#"<!doctype html><a href="/go" role="presentation">Go presentation</a>
        <a href="/go" role="none" tabindex="-1">Go none</a>
        <a href="/go" role="button">Explicit button</a>
        <button role="presentation">Native button</button>"#,
    )
    .await?;
    let (nodes, text) = semantic_nodes_and_text(&mut page).await?;
    let target = page.final_url().join("/go")?;
    for name in ["Go presentation", "Go none"] {
        assert_semantic_target(&nodes, &text, name, "link", Some(target.as_str()), true);
    }
    assert_semantic_target(&nodes, &text, "Explicit button", "button", None, false);
    assert_semantic_target(&nodes, &text, "Native button", "button", None, false);
    server.abort();
    Ok(())
}

#[tokio::test]
async fn semantic_presentational_roles_follow_standard_global_aria_properties() -> Result<()> {
    let (_browser, mut page, server) = load_page(
        r#"<!doctype html>
        <h2 role="none" aria-disabled="false">Disabled property</h2>
        <h2 role="presentation" aria-dropeffect="none">Drop property</h2>
        <h2 role="none" aria-errormessage="message">Error property</h2>
        <h2 role="presentation" aria-grabbed="false">Grabbed property</h2>
        <h2 role="none" aria-haspopup="false">Popup property</h2>
        <h2 role="presentation" aria-hidden="false">Visible hidden property</h2>
        <h2 role="none" aria-invalid="false">Invalid property</h2>
        <h2 role="none" aria-label="Canonical label">Visual label</h2>
        <h2 role="none" aria-description="Description">Description extension</h2>
        <h2 role="none" aria-braillelabel="Braille">Braille extension</h2>
        <h2 role="none" aria-brailleroledescription="Braille role">Braille role extension</h2>
        <h2 role="none" aria-labeledby="message">Misspelled label</h2>
        <h2 role="none" aria-level="3">Role-specific property</h2>
        <h2 id="foreign" role="none">Foreign namespace</h2>
        <h2 id="foreign-unprefixed" role="none">Foreign unprefixed namespace</h2>
        <h2 id="aria-namespace" role="none">ARIA namespace</h2>
        <h2 role="none" aria-hidden="true">Hidden secret</h2>
        <h2 role="button" aria-hidden="false">Explicit button</h2>
        <span id="message" hidden>Hidden message</span>"#,
    )
    .await?;
    let setup = page.evaluate_runtime_expression_by_value_async(
        r#"(() => {
            const cases = [
                ['foreign', 'urn:test', 'x:aria-label', 'Foreign label'],
                ['foreign-unprefixed', 'urn:test', 'aria-live', 'polite'],
                ['aria-namespace', 'http://www.w3.org/ns/wai-aria/', 'aria:aria-label', 'Namespaced label'],
            ];
            for (const [id, namespace, name, value] of cases)
                document.getElementById(id).setAttributeNS(namespace, name, value);
            return cases.every(([id, namespace, name, value]) =>
                document.getElementById(id).getAttributeNS(namespace, name.split(':').pop()) === value);
        })()"#,
    ).await?;
    assert_eq!(setup["value"], true, "{setup}");
    let (nodes, text) = semantic_nodes_and_text(&mut page).await?;
    let mut headings = nodes
        .iter()
        .filter(|node| node["role"]["value"] == "heading")
        .map(|node| node["name"]["value"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    headings.sort_unstable();
    let mut expected = vec![
        "Disabled property",
        "Drop property",
        "Error property",
        "Grabbed property",
        "Popup property",
        "Visible hidden property",
        "Invalid property",
        "Canonical label",
        "Description extension",
        "Braille extension",
        "Braille role extension",
    ];
    expected.sort_unstable();
    assert_eq!(headings, expected, "{text}");
    assert!(!text.contains("Hidden secret"), "{text}");
    assert_semantic_target(&nodes, &text, "Explicit button", "button", None, false);
    server.abort();
    Ok(())
}

#[tokio::test]
async fn semantic_html_whitespace_excludes_vertical_tab() -> Result<()> {
    let (_browser, mut page, server) = load_page(
        r#"<!doctype html><base href="/assets/">
        <h2 id="vt" role="presentation" tabindex="&#11;0">VT heading</h2>
        <h2 id="tab" role="presentation" tabindex="&#9;0">TAB heading</h2>
        <img src="&#11;" alt="VT image"><img src="&#9;" alt="TAB image">
        <img src="" alt="Empty image"><img src=" " alt="Space image">"#,
    )
    .await?;
    let tab_indices = page
        .evaluate_runtime_expression_by_value_async(
            "[document.getElementById('vt').tabIndex, document.getElementById('tab').tabIndex]",
        )
        .await?;
    assert_eq!(tab_indices["value"], json!([-1, 0]), "{tab_indices}");
    let (nodes, text) = semantic_nodes_and_text(&mut page).await?;
    assert!(
        nodes.iter().any(
            |node| node["role"]["value"] == "heading" && node["name"]["value"] == "TAB heading"
        ),
        "{text}"
    );
    assert!(!nodes.iter().any(|node| node["role"]["value"] == "heading" && node["name"]["value"] == "VT heading"), "{text}");
    let base = page.final_url().join("/assets/")?;
    assert_semantic_target(
        &nodes,
        &text,
        "VT image",
        "image",
        Some(base.as_str()),
        false,
    );
    for name in ["TAB image", "Empty image", "Space image"] {
        assert_semantic_target(&nodes, &text, name, "image", None, false);
    }
    server.abort();
    Ok(())
}

#[tokio::test]
async fn semantic_url_trimming_preserves_unicode_path_data() -> Result<()> {
    let (_browser, mut page, server) = load_page(
        r#"<!doctype html><base href="/assets/">
        <a href="&nbsp;">NBSP</a><a href=" &#x2003; ">EM space</a>
        <a href="">Empty</a><a href=" &#x9;&#xA; ">HTML spaces</a>"#,
    )
    .await?;
    let (nodes, text) = semantic_nodes_and_text(&mut page).await?;
    for (name, path) in [
        ("NBSP", "/assets/%C2%A0"),
        ("EM space", "/assets/%E2%80%83"),
        ("Empty", "/assets/"),
        ("HTML spaces", "/assets/"),
    ] {
        let target = page.final_url().join(path)?;
        assert_semantic_target(&nodes, &text, name, "link", Some(target.as_str()), true);
    }
    server.abort();
    Ok(())
}

#[tokio::test]
async fn semantic_svg_href_ignores_unrelated_namespaces() -> Result<()> {
    let (_browser, mut page, server) = load_page(
        r#"<!doctype html><base href="/assets/">
        <svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">
          <a id="legacy" xlink:href="/real" aria-label="Legacy"><text>Legacy</text></a>
          <a id="primary" href="/primary" xlink:href="/real" aria-label="Primary"><text>Primary</text></a>
          <a id="empty" href="" xlink:href="/real" aria-label="Empty SVG"><text>Empty</text></a>
          <a id="foreign" role="link" aria-label="Foreign"><text>Foreign</text></a>
        </svg>"#,
    ).await?;
    let result = page.evaluate_runtime_expression_by_value_async(
        "(() => { for (const node of document.querySelectorAll('svg a')) node.setAttributeNS('urn:example', 'href', '/wrong'); return document.getElementById('legacy').getAttributeNS('urn:example', 'href'); })()"
    ).await?;
    assert_eq!(result["value"], "/wrong", "{result}");
    let (nodes, text) = semantic_nodes_and_text(&mut page).await?;
    for (name, path) in [
        ("Legacy", "/real"),
        ("Primary", "/primary"),
        ("Empty SVG", "/assets/"),
    ] {
        let target = page.final_url().join(path)?;
        assert_semantic_target(&nodes, &text, name, "link", Some(target.as_str()), true);
    }
    assert_semantic_target(&nodes, &text, "Foreign", "link", None, false);
    server.abort();
    Ok(())
}

#[tokio::test]
async fn semantic_mathml_links_do_not_invent_native_targets() -> Result<()> {
    let (_browser, mut page, server) = load_page(
        r#"<!doctype html><math><a role="link" href="/other" aria-label="Math reference">M</a></math>
        <a href="/other">HTML reference</a>"#,
    ).await?;
    let (nodes, text) = semantic_nodes_and_text(&mut page).await?;
    assert_semantic_target(&nodes, &text, "Math reference", "link", None, false);
    let target = page.final_url().join("/other")?;
    assert_semantic_target(
        &nodes,
        &text,
        "HTML reference",
        "link",
        Some(target.as_str()),
        true,
    );
    server.abort();
    Ok(())
}

async fn semantic_nodes_and_text(page: &mut Page) -> Result<(Vec<Value>, String)> {
    let nodes = serde_json::from_str(&super::semantic::render_json(page, false).await?)?;
    let text = super::semantic::render_text(page, false).await?;
    Ok((nodes, text))
}

fn assert_semantic_target(
    nodes: &[Value],
    text: &str,
    name: &str,
    role: &str,
    target: Option<&str>,
    show_target: bool,
) {
    let node = nodes
        .iter()
        .find(|node| node["name"]["value"] == name && node["role"]["value"] == role)
        .unwrap_or_else(|| panic!("missing {role}: {name}: {text}"));
    let urls = node["properties"]
        .as_array()
        .expect("AX properties")
        .iter()
        .filter(|property| property["name"] == "url")
        .map(|property| property["value"]["value"].as_str().expect("AX URL"))
        .collect::<Vec<_>>();
    assert_eq!(
        urls,
        target.into_iter().collect::<Vec<_>>(),
        "{role}: {name} URL"
    );
    let arrow = if show_target {
        format!(" → {}", target.expect("link target"))
    } else {
        String::new()
    };
    let expected = format!(
        "- {role}: {name}{arrow} [backendNodeId={}]",
        node["backendDOMNodeId"]
    );
    assert!(
        text.lines().any(|line| line.trim_start() == expected),
        "missing {expected}: {text}"
    );
}

#[tokio::test]
async fn semantic_link_urls_follow_computed_roles() -> Result<()> {
    let (_browser, mut page, http_server) =
        load_page(include_str!("../../tests/fixtures/semantic-link-urls.html")).await?;
    let nodes: Vec<Value> =
        serde_json::from_str(&super::semantic::render_json(&mut page, false).await?)?;
    let text = super::semantic::render_text(&mut page, false).await?;
    let origin = page.final_url().clone();
    for (name, role, target, show_target) in [
        ("Docs", "link", Some("/assets/guide#top"), true),
        ("Footnote", "doc-noteref", Some("/assets/#note"), true),
        ("Backlink", "doc-backlink", Some("/article#back"), true),
        (
            "Bibliography",
            "doc-biblioref",
            Some("https://references.test/book"),
            true,
        ),
        ("Glossary", "doc-glossref", Some("/glossary#term"), true),
        ("Anchor button", "button", None, false),
        ("Anchor image", "image", None, false),
        ("Empty href", "link", Some("/assets/"), true),
        ("Help", "link", None, false),
        ("Image href", "link", None, false),
        ("Image footnote", "doc-noteref", None, false),
        ("Photo", "image", Some("/assets/photo.svg"), false),
        ("Picture button", "button", None, false),
        ("Cover", "doc-cover", Some("/assets/cover.svg"), false),
        (
            "Symbol",
            "graphics-symbol",
            Some("/assets/symbol.svg"),
            false,
        ),
        ("Submit", "button", Some("/assets/submit.svg"), false),
        ("Input link", "link", None, false),
        ("Script only", "link", None, false),
        ("SVG link", "link", Some("/assets/vector#top"), true),
        ("SVG legacy link", "link", Some("/assets/legacy#top"), true),
    ] {
        let node = nodes
            .iter()
            .find(|node| node["name"]["value"] == name && node["role"]["value"] == role)
            .unwrap_or_else(|| panic!("missing {role}: {name}: {nodes:?}"));
        let properties = node["properties"].as_array().expect("AX properties");
        let urls = properties
            .iter()
            .filter(|property| property["name"] == "url")
            .map(|property| property["value"]["value"].as_str().expect("AX URL"))
            .collect::<Vec<_>>();
        let target = target.map(|target| origin.join(target)).transpose()?;
        assert_eq!(
            urls,
            target
                .as_ref()
                .map(|url| url.as_str())
                .into_iter()
                .collect::<Vec<_>>(),
            "{role}: {name} URL"
        );
        let arrow = if show_target {
            format!(" → {}", target.as_ref().expect("link target"))
        } else {
            String::new()
        };
        let expected = format!(
            "- {role}: {name}{arrow} [backendNodeId={}]",
            node["backendDOMNodeId"]
        );
        assert!(
            text.lines().any(|line| line.trim_start() == expected),
            "missing {expected:?}: {text}"
        );
    }
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
async fn html_serialization_refreshes_page_state_and_preserves_reply_url_pairing() -> Result<()> {
    use moli_core::page::RendererHtmlSerializeOptions;
    let (_browser, mut page, http_server) =
        load_page("<!doctype html><p id=target>before</p>").await?;
    let mut pending_updates = Vec::new();
    for case in 0..4 {
        let expected = page
            .final_url()
            .join(&format!("/serialized-{case}?paired=1"))?;
        let old_url = page.final_url().clone();
        let command = json!({ "id": 720 + case, "method": "Runtime.evaluate", "params": {
            "expression": format!("history.replaceState(null, '', {}); document.getElementById('target').textContent = 'case-{case}'; 'done'", serde_json::to_string(expected.as_str())?),
            "returnByValue": true
        }});
        let pending = page.start_runtime_protocol_message(serde_json::to_string(&command)?)?;
        // The renderer has changed, but the caller has not applied its PageState.
        pending_updates.push(pending.wait().await?);
        assert_eq!(page.final_url(), &old_url);
        let html = match case {
            0 => page.serialize_html_async().await?,
            1 => {
                let document = page
                    .serialize_document_async(RendererHtmlSerializeOptions {
                        with_base: true,
                        ..Default::default()
                    })
                    .await?;
                assert_eq!(document.url, expected.as_str());
                document.html
            }
            2 => {
                let pending = page.start_serialize_html()?;
                let document = page.finish_serialized_document(pending.wait().await?)?;
                assert_eq!(document.url, expected.as_str());
                document.html
            }
            _ => {
                let output = render_page_dump_with_options_async(
                    &mut page,
                    DumpFormat::Json,
                    StripOptions {
                        css: true,
                        ..Default::default()
                    },
                    true,
                    false,
                    true,
                    None,
                )
                .await?;
                let payload: Value = serde_json::from_str(&output)?;
                assert_eq!(payload["final_url"], expected.as_str());
                assert_eq!(
                    payload["network"]["main_document"]["url"],
                    expected.as_str()
                );
                payload["html"].as_str().unwrap().to_owned()
            }
        };
        assert!(html.contains(&format!("case-{case}")), "{html}");
        assert_eq!(page.final_url(), &expected);
    }
    drop(pending_updates);
    http_server.abort();
    Ok(())
}

#[tokio::test]
async fn html_dump_profiles_preserve_golden_markup_and_the_live_source() -> Result<()> {
    let (_browser, mut page, http_server) = load_page("<!doctype html><html><head><style>p{color:red}</style></head><body><nav>MENU</nav><p style=\"color:red\" onclick=\"void 0\">KEEP &amp; text</p><script>window.secret=1</script><x-card></x-card></body></html>").await?;
    let setup = page.evaluate_runtime_expression_by_value_async(r#"
        globalThis.exportShadow=document.querySelector('x-card').attachShadow({mode:'closed',slotAssignment:'manual'});
        exportShadow.innerHTML='<b>SHADOW &amp; text</b>';
        globalThis.exportChanges=0;
        globalThis.exportObserver=new MutationObserver(records=>exportChanges+=records.length);
        exportObserver.observe(document,{subtree:true,childList:true,attributes:true,characterData:true});
        JSON.stringify([document.documentElement.outerHTML,exportShadow.innerHTML,document.baseURI])
    "#).await?;
    assert!(setup.get("exception").is_none(), "{setup}");
    assert!(setup["value"].is_string(), "{setup}");
    for bits in 0..8 {
        let strip = StripOptions {
            js: bits & 1 != 0,
            css: bits & 2 != 0,
            ui: bits & 4 != 0,
        };
        let mut expected = String::from("<!DOCTYPE html><html><head>");
        if !strip.css {
            expected.push_str("<style>p{color:red}</style>");
        }
        expected.push_str("</head><body>");
        if !strip.ui {
            expected.push_str("<nav>MENU</nav>");
        }
        expected.push_str("<p");
        if !strip.css {
            expected.push_str(" style=\"color:red\"");
        }
        if !strip.js {
            expected.push_str(" onclick=\"void 0\"");
        }
        expected.push_str(">KEEP &amp; text</p>");
        if !strip.js {
            expected.push_str("<script>window.secret=1</script>");
        }
        expected.push_str("<x-card><template shadowrootmode=\"closed\" shadowrootslotassignment=\"manual\"><b>SHADOW &amp; text</b></template></x-card></body></html>");
        for _ in 0..2 {
            let output = render_page_dump_with_options_async(
                &mut page,
                DumpFormat::Html,
                strip,
                false,
                false,
                false,
                None,
            )
            .await?;
            assert_eq!(output, expected, "strip={bits}");
            let snapshot=page.evaluate_runtime_expression_by_value_async("JSON.stringify([document.documentElement.outerHTML,exportShadow.innerHTML,document.baseURI])").await?;
            assert_eq!(snapshot["value"], setup["value"]);
            let changes = page
                .evaluate_runtime_expression_by_value_async(
                    "exportChanges+exportObserver.takeRecords().length",
                )
                .await?;
            assert_eq!(changes["value"], 0);
        }
    }
    http_server.abort();
    Ok(())
}

#[tokio::test]
async fn html_dump_filters_template_contents_and_includes_inert_author_shadow_roots() -> Result<()>
{
    let (_browser, mut page, http_server) = load_page("<!doctype html><template id=content><nav>MENU</nav><script>secret=1</script><style>b{color:red}</style><div style=\"color:red\" onclick=\"void 0\">KEEP</div><x-card></x-card><iframe srcdoc=\"<script>inert=1</script><p>unparsed</p>\"></iframe></template>").await?;
    let setup=page.evaluate_runtime_expression_by_value_async("document.getElementById('content').content.querySelector('x-card').attachShadow({mode:'closed'}).innerHTML='<style>b{color:red}</style><b>SHADOW</b>'; 'ready'").await?;
    assert!(setup.get("exception").is_none(), "{setup}");
    let before = page.serialize_html_async().await?;
    assert!(
        before.contains("shadowrootmode=\"closed\"") && before.contains("SHADOW"),
        "{before}"
    );
    let output = render_page_dump_with_options_async(
        &mut page,
        DumpFormat::Html,
        StripOptions {
            js: true,
            css: true,
            ui: true,
        },
        false,
        true,
        false,
        None,
    )
    .await?;
    for removed in ["MENU", "secret=1", "color:red", "onclick="] {
        assert!(!output.contains(removed), "{output}");
    }
    assert!(
        output.contains("<div>KEEP</div>") && output.contains("<b>SHADOW</b>"),
        "{output}"
    );
    // Inert owners have no live child Document. Preserve their raw srcdoc;
    // exporting is not an HTML sanitizer and must not parse/load that string.
    assert!(
        output
            .contains("srcdoc=\"&lt;script&gt;inert=1&lt;/script&gt;&lt;p&gt;unparsed&lt;/p&gt;\""),
        "{output}"
    );
    assert_eq!(page.serialize_html_async().await?, before);
    http_server.abort();
    Ok(())
}
