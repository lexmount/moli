use moli_test_support as support;

use anyhow::Result;
use moli_core::runtime::{Browser, BrowserConfig as AppConfig, RenderedDomWaitUntil};
use support::FixtureServer;
use tokio::time::Duration;
use url::Url;

async fn wait_for_body_attribute(
    browser: &Browser,
    page: &mut moli_core::page::Page,
    attr: &str,
    expected: &str,
) -> Result<()> {
    let attr = serde_json::to_string(attr)?;
    let expected = serde_json::to_string(expected)?;
    browser
        .wait_for_script_truthy(
            page,
            &format!("document.body?.getAttribute({attr}) === {expected}"),
            Duration::from_secs(2),
        )
        .await
}

async fn wait_for_body_attribute_contains(
    browser: &Browser,
    page: &mut moli_core::page::Page,
    attr: &str,
    needle: &str,
) -> Result<()> {
    let attr = serde_json::to_string(attr)?;
    let needle = serde_json::to_string(needle)?;
    browser
        .wait_for_script_truthy(
            page,
            &format!("document.body?.getAttribute({attr})?.includes({needle}) === true"),
            Duration::from_secs(2),
        )
        .await
}

#[tokio::test]
async fn child_browsing_context_window_wrapper_routes_post_message_to_child_target() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-post-message"))
        .await?;
    wait_for_body_attribute(&browser, &mut page, "data-message", "ping").await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-message=\"ping\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-source-ok=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_window_graph_uses_stable_live_proxies() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-window-graph"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-length=\"2\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-w0-proxy=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-w1-proxy=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-w1-top=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-w1-parent=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-w1-self-graph=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-w1-frame-element-id=\"f1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-w1-document-identity=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-w1-document-text=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-w1-default-view=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-w1-parent-window-missing=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-w1-proxy-after-load=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-w1-document-identity-after-load=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-w1-document-text-after-load=\"child\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_window_wrapper_owns_runtime_backing() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-runtime-backing"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-location-distinct=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-distinct=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-navigation-distinct=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-location-href=\"{}\"", "about:blank")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-location-identity=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-location-stringifier=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-location-stringifier=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-url-aligned=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-base-inherited=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_location_navigation_stays_window_local() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    for mode in [
        "assign-replace",
        "href",
        "window-location",
        "document-location",
    ] {
        assert_child_location_navigation_stays_window_local(&browser, &server, mode).await?;
    }

    server.shutdown().await;
    Ok(())
}

async fn assert_child_location_navigation_stays_window_local(
    browser: &Browser,
    server: &FixtureServer,
    mode: &str,
) -> Result<()> {
    let mut page = browser
        .fetch(&server.url(&format!(
            "/compat/window-child-browsing-context-location-navigation?mode={mode}"
        )))
        .await?;
    let initial_url = "about:blank";
    let final_url = if mode == "assign-replace" {
        server.url("/compat/window-child-browsing-context-delayed-child?replaced=1")
    } else {
        server.url(&format!(
            "/compat/window-child-browsing-context-delayed-child?mode={mode}"
        ))
    };

    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-child-document-text-after-load') === 'delayed'",
            Duration::from_secs(2),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-mode=\"{mode}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-location-unchanged=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-history-unchanged=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-history-advanced=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-current-entry-index=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-history-state-null=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-child-current-entry-url=\"{}\"", final_url)),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-child-location-href=\"{}\"", final_url)),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-document-location-immediate=\"{}\"",
                final_url
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-document-url-immediate=\"{}\"",
                initial_url
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-same-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-same-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-document-same-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-default-view-same-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-default-view-document-same-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-document-still-committed-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-same-pending-microtask=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-same-pending-microtask=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-document-same-pending-microtask=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-default-view-same-pending-microtask=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-default-view-document-same-pending-microtask=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-location-target-pending-microtask=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-document-location-pending-microtask=\"{}\"",
                final_url
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-document-url-pending-microtask=\"{}\"",
                initial_url
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-document-still-committed-pending-microtask=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-same-after-load=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-replaced-after-load=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-document-same-after-load=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-default-view-same-after-load=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-default-view-document-same-after-load=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-document-location-after-load=\"{}\"",
                final_url
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-document-url-after-load=\"{}\"",
                final_url
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-document-text-after-load=\"delayed\"")
    );

    Ok(())
}

#[tokio::test]
async fn child_browsing_context_location_pathname_keeps_committed_document_until_load() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    for mode in ["pathname", "document-pathname"] {
        assert_child_location_component_keeps_committed_document_until_load(
            &browser,
            &server,
            mode,
            "",
            &server.url("/compat/window-child-browsing-context-delayed-child"),
            "/compat/window-child-browsing-context-delayed-child",
            "",
            "delayed",
        )
        .await?;
    }

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_location_search_keeps_committed_document_until_load() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    for mode in ["search", "document-search"] {
        assert_child_location_component_keeps_committed_document_until_load(
            &browser,
            &server,
            mode,
            "",
            &server.url("/compat/window-child-browsing-context-target-name-a?via=search"),
            "/compat/window-child-browsing-context-target-name-a",
            "?via=search",
            "name-a",
        )
        .await?;
    }

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_location_host_keeps_committed_document_until_load() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let final_url =
        alternate_localhost_url(&server.url("/compat/window-child-browsing-context-target-name-a"));

    for mode in ["host", "document-host"] {
        assert_child_location_component_keeps_committed_document_until_load(
            &browser,
            &server,
            mode,
            "",
            &final_url,
            "/compat/window-child-browsing-context-target-name-a",
            "",
            "name-a",
        )
        .await?;
    }

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_location_hostname_keeps_committed_document_until_load() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let final_url =
        alternate_localhost_url(&server.url("/compat/window-child-browsing-context-target-name-a"));

    for mode in ["hostname", "document-hostname"] {
        assert_child_location_component_keeps_committed_document_until_load(
            &browser,
            &server,
            mode,
            "",
            &final_url,
            "/compat/window-child-browsing-context-target-name-a",
            "",
            "name-a",
        )
        .await?;
    }

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_location_port_keeps_committed_document_until_load() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let target_server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let final_url = target_server.url("/compat/window-child-browsing-context-target-name-a");
    let target_port = fixture_url_port(&final_url);
    let fixture_query_suffix = format!("&targetPort={target_port}");

    for mode in ["port", "document-port"] {
        assert_child_location_component_keeps_committed_document_until_load(
            &browser,
            &server,
            mode,
            &fixture_query_suffix,
            &final_url,
            "/compat/window-child-browsing-context-target-name-a",
            "",
            "name-a",
        )
        .await?;
    }

    target_server.shutdown().await;
    server.shutdown().await;
    Ok(())
}

async fn assert_child_location_component_keeps_committed_document_until_load(
    browser: &Browser,
    server: &FixtureServer,
    mode: &str,
    fixture_query_suffix: &str,
    final_url: &str,
    final_pathname: &str,
    final_search: &str,
    final_text: &str,
) -> Result<()> {
    let mut page = browser
        .fetch(&server.url(&format!(
            "/compat/window-child-browsing-context-location-pathname-pending-document-coherence?mode={mode}{fixture_query_suffix}"
        )))
        .await?;
    let initial_url = server.url("/compat/window-child-browsing-context-target-name-a");
    let final_is_cross_origin =
        Url::parse(&initial_url)?.origin() != Url::parse(final_url)?.origin();
    let wait_expression = if final_is_cross_origin {
        format!(
            "document.body.getAttribute('data-child-location-after-load') === '{}'",
            final_url
        )
    } else {
        format!(
            "document.body.getAttribute('data-child-document-text-after-load') === '{}'",
            final_text
        )
    };

    browser
        .wait_for_script_truthy(&mut page, &wait_expression, Duration::from_secs(2))
        .await?;

    let html = page.serialize_html_async().await.unwrap();
    assert!(html.contains(&format!("data-mode=\"{mode}\"")), "{}", html);
    for attr in [
        "data-window-same-immediate=\"true\"",
        "data-document-same-immediate=\"true\"",
        "data-window-document-same-immediate=\"true\"",
        "data-document-default-view-same-immediate=\"true\"",
        "data-default-view-document-same-immediate=\"true\"",
        "data-child-document-still-committed-immediate=\"true\"",
        "data-child-location-target-immediate=\"true\"",
        "data-window-same-pending-microtask=\"true\"",
        "data-document-same-pending-microtask=\"true\"",
        "data-window-document-same-pending-microtask=\"true\"",
        "data-document-default-view-same-pending-microtask=\"true\"",
        "data-default-view-document-same-pending-microtask=\"true\"",
        "data-child-document-still-committed-pending-microtask=\"true\"",
        "data-child-location-target-pending-microtask=\"true\"",
        "data-document-replaced-after-load=\"true\"",
    ] {
        assert!(html.contains(attr), "{attr}\n{html}");
    }
    if final_is_cross_origin {
        for attr in [
            "data-child-content-document-null-after-load=\"true\"",
            "data-child-document-url-after-load=\"null\"",
            "data-child-document-text-after-load=\"null\"",
            "data-window-same-after-load=\"true\"",
            "data-window-document-same-after-load=\"false\"",
            "data-document-default-view-same-after-load=\"false\"",
            "data-default-view-document-same-after-load=\"false\"",
            "data-child-location-href-read-error-after-load=\"SecurityError:true\"",
        ] {
            assert!(html.contains(attr), "{attr}\n{html}");
        }
    } else {
        for attr in [
            "data-child-content-document-null-after-load=\"false\"",
            "data-window-same-after-load=\"true\"",
            "data-window-document-same-after-load=\"true\"",
            "data-document-default-view-same-after-load=\"true\"",
            "data-default-view-document-same-after-load=\"true\"",
        ] {
            assert!(html.contains(attr), "{attr}\n{html}");
        }
        assert_html_contains_attr(&html, "data-child-document-text-after-load", final_text);
    }
    for attr in [
        ("data-child-location-immediate", final_url),
        ("data-child-location-pending-microtask", final_url),
        ("data-child-location-after-load", final_url),
        ("data-child-document-url-immediate", &initial_url),
        ("data-child-document-url-pending-microtask", &initial_url),
    ] {
        assert_html_contains_attr(&html, attr.0, attr.1);
    }
    if !final_is_cross_origin {
        assert_html_contains_attr(&html, "data-child-document-url-after-load", final_url);
    }
    for attr in [
        "data-child-pathname-immediate",
        "data-child-pathname-pending-microtask",
        "data-child-pathname-after-load",
    ] {
        assert_html_contains_attr(&html, attr, final_pathname);
    }
    for attr in [
        "data-child-search-immediate",
        "data-child-search-pending-microtask",
        "data-child-search-after-load",
    ] {
        assert_html_contains_attr(&html, attr, final_search);
    }

    Ok(())
}

fn alternate_localhost_url(url: &str) -> String {
    let mut url = Url::parse(url).expect("fixture URL should parse");
    url.set_host(Some("localhost"))
        .expect("fixture URL host should be settable");
    url.to_string()
}

fn fixture_url_port(url: &str) -> u16 {
    Url::parse(url)
        .expect("fixture URL should parse")
        .port()
        .expect("fixture URL should include an explicit port")
}

fn assert_html_contains_attr(html: &str, attr: &str, value: &str) {
    assert!(html.contains(&format!("{attr}=\"{value}\"")), "{html}");
}

fn evaluated_string(value: serde_json::Value) -> Option<String> {
    value
        .get("value")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

#[tokio::test]
async fn child_browsing_context_classic_scripts_share_child_global_bindings() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-script-globals"))
        .await?;

    fn evaluated_string(value: serde_json::Value) -> Option<String> {
        value
            .get("value")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    }

    assert_eq!(
        evaluated_string(page.evaluate_runtime_expression_async(
            "document.getElementById('child').contentDocument?.body?.getAttribute('data-shared') ?? 'null'"
        ).await?),
        Some("shared-from-first-script".to_owned())
    );
    assert_eq!(
        evaluated_string(page.evaluate_runtime_expression_async(
            "document.getElementById('child').contentDocument?.body?.getAttribute('data-reader') ?? 'null'"
        ).await?),
        Some("shared-from-first-script".to_owned())
    );
    assert_eq!(
        evaluated_string(page.evaluate_runtime_expression_async(
            "document.getElementById('child').contentDocument?.body?.getAttribute('data-window-shared') ?? 'null'"
        ).await?),
        Some("shared-from-first-script".to_owned())
    );
    assert_eq!(
        evaluated_string(page.evaluate_runtime_expression_async(
            "document.getElementById('child').contentDocument?.body?.getAttribute('data-window-reader-type') ?? 'null'"
        ).await?),
        Some("function".to_owned())
    );
    assert_eq!(
        evaluated_string(
            page.evaluate_runtime_expression_async("String('childShared' in window)")
                .await?,
        ),
        Some("false".to_owned())
    );
    assert_eq!(
        evaluated_string(
            page.evaluate_runtime_expression_async("String('childReader' in window)")
                .await?,
        ),
        Some("false".to_owned())
    );
    assert_eq!(
        evaluated_string(
            page.evaluate_runtime_expression_async(
                "(() => {\
                  const cw = document.getElementById('child').contentWindow;\
                  const childDocument = document.getElementById('child').contentDocument;\
                  const div = childDocument.createElement('div');\
                  const descriptor = Object.getOwnPropertyDescriptor(cw, 'HTMLDivElement');\
                  return [\
                    typeof cw.Node,\
                    typeof cw.Element,\
                    typeof cw.HTMLElement,\
                    typeof cw.HTMLDivElement,\
                    String(div instanceof cw.HTMLDivElement),\
                    String(div instanceof cw.HTMLElement),\
                    String(div instanceof cw.Element),\
                    String(div instanceof cw.Node),\
                    Object.prototype.toString.call(div),\
                    String(cw.HTMLDivElement.prototype.constructor === cw.HTMLDivElement),\
                    String(descriptor && descriptor.enumerable === false),\
                    String(descriptor && descriptor.writable === true),\
                    String(descriptor && descriptor.configurable === true)\
                  ].join('|');\
                })()"
            )
            .await?,
        ),
        Some("function|function|function|function|true|true|true|true|[object HTMLDivElement]|true|true|true|true".to_owned())
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_attribute_navigation_preserves_local_history() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/compat/window-child-browsing-context-attribute-navigation-history"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(5),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-location-unchanged=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-history-unchanged=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-history-length=\"3\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-location-after-back=\"{}\"",
                server.url("/compat/window-child-browsing-context-target-name-b")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-document-location-after-back=\"{}\"",
                server.url("/compat/window-child-browsing-context-target-name-b")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-current-entry-after-back=\"{}\"",
                server.url("/compat/window-child-browsing-context-target-name-b")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_history_relative_urls_stay_child_local() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-history-relative-urls"))
        .await?;
    browser
        .wait_for_page_delay(&mut page, Duration::from_millis(250))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-location-unchanged=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-history-unchanged=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-history-length=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-location-href=\"{}\"",
                server.url("/compat/relative-child?step=push#frag")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-document-location-href=\"{}\"",
                server.url("/compat/relative-child?step=push#frag")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-current-entry-url=\"{}\"",
                server.url("/compat/relative-child?step=push#frag")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-history-state=\"{&quot;step&quot;:&quot;replace&quot;}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-history-unchanged-after-back=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-path-back-reload-count=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-path-back-location-href=\"{}\"",
                server.url("/compat/window-child-browsing-context-target-name-a?base=1")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-path-back-document-location-href=\"{}\"",
                server.url("/compat/window-child-browsing-context-target-name-a?base=1")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-path-back-current-entry-url=\"{}\"",
                server.url("/compat/window-child-browsing-context-target-name-a?base=1")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-path-back-history-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-path-back-history-length=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-path-back-can-forward=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-path-back-current-same-document=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_navigation_state_stays_window_local() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-navigation-state"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-location-unchanged=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-history-unchanged=\"true\"")
    );
    let child_a_url = server.url("/compat/window-child-browsing-context-target-name-a");
    let child_b_url = server.url("/compat/window-child-browsing-context-target-name-b");
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-history-unchanged-sync=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-child-location-href-sync=\"{child_a_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-current-entry-url-sync=\"{child_a_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-current-entry-index-sync=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-current-entry-state-sync=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-document-location-href-sync=\"{child_a_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-result-sync-shape=\"object|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-currententrychange-count-sync=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-property-currententrychange-count-sync=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-result-settled-timeout=\"false|false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-currententrychange-count-timeout=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-property-currententrychange-count-timeout=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-location-href-after-load=\"{child_b_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-current-entry-url-after-load=\"{child_b_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-current-entry-index-after-load=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-current-entry-state-after-load=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-history-state-after-load=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-current-entry-state-type-after-load=\"undefined\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-document-location-href-after-load=\"{child_b_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-history-unchanged-after-load=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-activation-after-load=\"{child_b_url}|{child_a_url}|push\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-entries-meta-after-load=\"2|true|true|{child_a_url},{child_b_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-currententrychange-count-after-load=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-property-currententrychange-count-after-load=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_navigation_push_state_stays_window_local() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-navigation-push-state"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-location-unchanged=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-history-unchanged=\"true\"")
    );
    let child_a_url = server.url("/compat/window-child-browsing-context-target-name-a");
    let child_b_url = server.url("/compat/window-child-browsing-context-target-name-b");
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-history-unchanged-sync=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-child-location-href-sync=\"{child_a_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-current-entry-url-sync=\"{child_a_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-current-entry-index-sync=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-current-entry-state-sync=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-document-location-href-sync=\"{child_a_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-result-sync-shape=\"object|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-currententrychange-count-sync=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-property-currententrychange-count-sync=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-result-settled-timeout=\"false|false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-currententrychange-count-timeout=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-property-currententrychange-count-timeout=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-location-href-after-load=\"{child_b_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-current-entry-url-after-load=\"{child_b_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-current-entry-index-after-load=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-current-entry-state-after-load=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-history-state-after-load=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-current-entry-state-type-after-load=\"undefined\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-history-length-after-load=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-activation-after-load=\"{child_b_url}|{child_a_url}|push\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-entries-meta-after-load=\"2|true|true|{child_a_url},{child_b_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-currententrychange-count-after-load=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-property-currententrychange-count-after-load=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-document-location-href-after-load=\"{child_b_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_navigation_back_cross_document_destination_surface() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url(
            "/compat/window-child-browsing-context-navigation-back-cross-document-destination-surface",
        ))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-child-load-count') === '3'",
            Duration::from_secs(2),
        )
        .await?;

    let child_a_url = server.url("/compat/window-child-browsing-context-target-name-a");
    let child_b_url = server.url("/compat/window-child-browsing-context-target-name-b");
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-mode=\"back\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-back-sync=\"{child_b_url}|true|committed,finished|true|true\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-load-count=\"3\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-child-location-href=\"{child_a_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-child-current-entry-url=\"{child_a_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-current-entry-index=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-entries=\"{child_a_url},{child_b_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-activation=\"{child_a_url}|{child_b_url}|traverse\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-back-settled=\"false|false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-back-values=\"|\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_navigation_traverse_to_cross_document_destination_surface()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url(
            "/compat/window-child-browsing-context-navigation-back-cross-document-destination-surface?mode=traverse",
        ))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-child-load-count') === '3'",
            Duration::from_secs(2),
        )
        .await?;

    let child_a_url = server.url("/compat/window-child-browsing-context-target-name-a");
    let child_b_url = server.url("/compat/window-child-browsing-context-target-name-b");
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-mode=\"traverse\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-back-sync=\"{child_b_url}|true|committed,finished|true|true\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-load-count=\"3\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-child-location-href=\"{child_a_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-child-current-entry-url=\"{child_a_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-current-entry-index=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-entries=\"{child_a_url},{child_b_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-activation=\"{child_a_url}|{child_b_url}|traverse\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-back-settled=\"false|false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-back-values=\"|\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_history_back_cross_document_destination_surface() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url(
            "/compat/window-child-browsing-context-navigation-back-cross-document-destination-surface?mode=history-back",
        ))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-child-load-count') === '3'",
            Duration::from_secs(2),
        )
        .await?;

    let child_a_url = server.url("/compat/window-child-browsing-context-target-name-a");
    let child_b_url = server.url("/compat/window-child-browsing-context-target-name-b");
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-mode=\"history-back\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-back-sync=\"{child_b_url}|true\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-load-count=\"3\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-child-location-href=\"{child_a_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-child-current-entry-url=\"{child_a_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-current-entry-index=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-entries=\"{child_a_url},{child_b_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-activation=\"{child_a_url}|{child_b_url}|traverse\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_traversal_keeps_committed_document_until_load() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    for mode in ["back", "traverse", "history-back", "history-go-back"] {
        assert_child_traversal_pending_document_coherence(&browser, &server, mode).await?;
    }

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_forward_traversal_keeps_committed_document_until_load() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    for mode in [
        "forward",
        "traverse",
        "history-forward",
        "history-go-forward",
    ] {
        assert_child_forward_traversal_pending_document_coherence(&browser, &server, mode).await?;
    }

    server.shutdown().await;
    Ok(())
}

async fn assert_child_traversal_pending_document_coherence(
    browser: &Browser,
    server: &FixtureServer,
    mode: &str,
) -> Result<()> {
    let mut page = browser
        .fetch(&server.url(&format!(
            "/compat/window-child-browsing-context-traversal-pending-document-coherence?mode={mode}"
        )))
        .await?;
    let child_a_url = server.url("/compat/window-child-browsing-context-target-name-a");
    let child_b_url = server.url("/compat/window-child-browsing-context-target-name-b");

    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-child-load-count') === '3'",
            Duration::from_secs(2),
        )
        .await?;

    let html = page.serialize_html_async().await.unwrap();
    macro_rules! assert_html_contains {
        ($needle:expr) => {
            assert!(html.contains($needle), "{}", html);
        };
    }

    assert_html_contains!(&format!("data-mode=\"{mode}\""));
    if matches!(mode, "history-back" | "history-go-back") {
        assert_html_contains!("data-result-return=\"undefined\"");
    } else {
        assert_html_contains!("data-result-shape=\"committed,finished|true|true\"");
    }
    assert_html_contains!("data-window-same-immediate=\"true\"");
    assert_html_contains!("data-document-same-immediate=\"true\"");
    assert_html_contains!("data-window-document-same-immediate=\"true\"");
    assert_html_contains!("data-document-default-view-same-immediate=\"true\"");
    assert_html_contains!("data-default-view-document-same-immediate=\"true\"");
    assert_html_contains!(&format!("data-location-immediate=\"{child_b_url}\""));
    assert_html_contains!(&format!("data-document-url-immediate=\"{child_b_url}\""));
    assert_html_contains!(&format!(
        "data-document-location-immediate=\"{child_b_url}\""
    ));
    assert_html_contains!(&format!("data-current-entry-immediate=\"{child_b_url}\""));
    assert_html_contains!("data-current-entry-same-immediate=\"true\"");
    assert_html_contains!("data-document-still-committed-immediate=\"true\"");
    assert_html_contains!("data-child-body-still-source-immediate=\"name-b\"");

    assert_html_contains!("data-window-same-pending-microtask=\"true\"");
    assert_html_contains!("data-document-same-pending-microtask=\"true\"");
    assert_html_contains!("data-window-document-same-pending-microtask=\"true\"");
    assert_html_contains!("data-document-default-view-same-pending-microtask=\"true\"");
    assert_html_contains!("data-default-view-document-same-pending-microtask=\"true\"");
    assert_html_contains!(&format!(
        "data-location-pending-microtask=\"{child_b_url}\""
    ));
    assert_html_contains!(&format!(
        "data-document-url-pending-microtask=\"{child_b_url}\""
    ));
    assert_html_contains!(&format!(
        "data-document-location-pending-microtask=\"{child_b_url}\""
    ));
    assert_html_contains!(&format!(
        "data-current-entry-pending-microtask=\"{child_b_url}\""
    ));
    assert_html_contains!("data-current-entry-same-pending-microtask=\"true\"");
    assert_html_contains!("data-document-still-committed-pending-microtask=\"true\"");
    assert_html_contains!("data-child-body-still-source-pending-microtask=\"name-b\"");

    assert_html_contains!("data-child-load-count=\"3\"");
    assert_html_contains!("data-window-same-after-load=\"true\"");
    assert_html_contains!("data-document-replaced-after-load=\"true\"");
    assert_html_contains!("data-window-document-same-after-load=\"true\"");
    assert_html_contains!("data-document-default-view-same-after-load=\"true\"");
    assert_html_contains!("data-default-view-document-same-after-load=\"true\"");
    assert_html_contains!(&format!("data-location-after-load=\"{child_a_url}\""));
    assert_html_contains!(&format!("data-document-url-after-load=\"{child_a_url}\""));
    assert_html_contains!(&format!("data-current-entry-after-load=\"{child_a_url}\""));
    assert_html_contains!("data-child-body-after-load=\"name-a\"");

    Ok(())
}

async fn assert_child_forward_traversal_pending_document_coherence(
    browser: &Browser,
    server: &FixtureServer,
    mode: &str,
) -> Result<()> {
    let mut page = browser
        .fetch(&server.url(&format!(
            "/compat/window-child-browsing-context-forward-traversal-pending-document-coherence?mode={mode}"
        )))
        .await?;
    let child_a_url = server.url("/compat/window-child-browsing-context-target-name-a");
    let child_b_url = server.url("/compat/window-child-browsing-context-target-name-b");

    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-child-load-count') === '4'",
            Duration::from_secs(2),
        )
        .await?;

    let html = page.serialize_html_async().await.unwrap();
    macro_rules! assert_html_contains {
        ($needle:expr) => {
            assert!(html.contains($needle), "{}", html);
        };
    }

    assert_html_contains!(&format!("data-mode=\"{mode}\""));
    if matches!(mode, "history-forward" | "history-go-forward") {
        assert_html_contains!("data-result-return=\"undefined\"");
    } else {
        assert_html_contains!("data-result-shape=\"committed,finished|true|true\"");
    }
    assert_html_contains!("data-window-same-immediate=\"true\"");
    assert_html_contains!("data-document-same-immediate=\"true\"");
    assert_html_contains!("data-window-document-same-immediate=\"true\"");
    assert_html_contains!("data-document-default-view-same-immediate=\"true\"");
    assert_html_contains!("data-default-view-document-same-immediate=\"true\"");
    assert_html_contains!(&format!("data-location-immediate=\"{child_a_url}\""));
    assert_html_contains!(&format!("data-document-url-immediate=\"{child_a_url}\""));
    assert_html_contains!(&format!(
        "data-document-location-immediate=\"{child_a_url}\""
    ));
    assert_html_contains!(&format!("data-current-entry-immediate=\"{child_a_url}\""));
    assert_html_contains!("data-current-entry-same-immediate=\"true\"");
    assert_html_contains!("data-document-still-committed-immediate=\"true\"");
    assert_html_contains!("data-child-body-still-source-immediate=\"name-a\"");

    assert_html_contains!("data-window-same-pending-microtask=\"true\"");
    assert_html_contains!("data-document-same-pending-microtask=\"true\"");
    assert_html_contains!("data-window-document-same-pending-microtask=\"true\"");
    assert_html_contains!("data-document-default-view-same-pending-microtask=\"true\"");
    assert_html_contains!("data-default-view-document-same-pending-microtask=\"true\"");
    assert_html_contains!(&format!(
        "data-location-pending-microtask=\"{child_a_url}\""
    ));
    assert_html_contains!(&format!(
        "data-document-url-pending-microtask=\"{child_a_url}\""
    ));
    assert_html_contains!(&format!(
        "data-document-location-pending-microtask=\"{child_a_url}\""
    ));
    assert_html_contains!(&format!(
        "data-current-entry-pending-microtask=\"{child_a_url}\""
    ));
    assert_html_contains!("data-current-entry-same-pending-microtask=\"true\"");
    assert_html_contains!("data-document-still-committed-pending-microtask=\"true\"");
    assert_html_contains!("data-child-body-still-source-pending-microtask=\"name-a\"");

    assert_html_contains!("data-child-load-count=\"4\"");
    assert_html_contains!("data-window-same-after-load=\"true\"");
    assert_html_contains!("data-document-replaced-after-load=\"true\"");
    assert_html_contains!("data-window-document-same-after-load=\"true\"");
    assert_html_contains!("data-document-default-view-same-after-load=\"true\"");
    assert_html_contains!("data-default-view-document-same-after-load=\"true\"");
    assert_html_contains!(&format!("data-location-after-load=\"{child_b_url}\""));
    assert_html_contains!(&format!("data-document-url-after-load=\"{child_b_url}\""));
    assert_html_contains!(&format!("data-current-entry-after-load=\"{child_b_url}\""));
    assert_html_contains!("data-child-body-after-load=\"name-b\"");

    Ok(())
}

#[tokio::test]
async fn child_browsing_context_navigation_noop_result_surface_matches_chromium() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-navigation-noop-result-surface"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-back-sync=\"true|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-forward-sync=\"true|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-back-committed=\"InvalidStateError|Cannot go back\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-back-finished=\"InvalidStateError|Cannot go back\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-forward-committed=\"InvalidStateError|Cannot go forward\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-forward-finished=\"InvalidStateError|Cannot go forward\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_navigation_traverse_to_noop_result_surface_matches_chromium()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url(
            "/compat/window-child-browsing-context-navigation-traverse-to-noop-result-surface",
        ))
        .await?;
    let child_url = server.url("/compat/window-child-browsing-context-target-name-a");

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-current-sync=\"true|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(&format!(
            "data-child-current-committed=\"[object NavigationHistoryEntry]|{child_url}|true|true\""
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(&format!(
            "data-child-current-finished=\"[object NavigationHistoryEntry]|{child_url}|true|true\""
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-missing-sync=\"true|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-missing-committed=\"InvalidStateError|Invalid key\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-missing-finished=\"InvalidStateError|Invalid key\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_navigation_same_document_push_result_surface_matches_chromium()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url(
            "/compat/window-child-browsing-context-navigation-same-document-push-result-surface",
        ))
        .await?;
    wait_for_body_attribute_contains(&browser, &mut page, "data-timeout", "finished:#dest:true")
        .await?;

    let child_url = server.url(
        "/compat/window-child-browsing-context-navigation-same-document-push-result-surface-child",
    );

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-location-unchanged=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-history-unchanged=\"false\"")
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(&format!(
            "data-sync=\"hash=#dest|len=2|committed=false|finished=false|history=null|entry={{&quot;step&quot;:7}}|listener=1|property=1|from={child_url}|propertyFrom={child_url}|navType=push|propertyNavType=push|order=cec:push:{child_url},popstate:#dest\""
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(&format!(
            "data-timeout=\"hash=#dest|len=2|committed=true|finished=true|history=null|entry={{&quot;step&quot;:7}}|listener=1|property=1|from={child_url}|propertyFrom={child_url}|navType=push|propertyNavType=push|order=cec:push:{child_url},popstate:#dest,cec-micro:#dest:null,committed:#dest:true,finished:#dest:true,hashchange:#dest\""
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_navigation_same_document_replace_result_surface_matches_chromium()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url(
            "/compat/window-child-browsing-context-navigation-same-document-replace-result-surface",
        ))
        .await?;
    wait_for_body_attribute_contains(&browser, &mut page, "data-timeout", "finished:#dest:true")
        .await?;

    let child_url = server.url(
        "/compat/window-child-browsing-context-navigation-same-document-replace-result-surface-child",
    );

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-location-unchanged=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-history-unchanged=\"true\"")
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(&format!(
            "data-sync=\"hash=#dest|len=1|committed=false|finished=false|history=null|entry={{&quot;step&quot;:9}}|listener=1|property=1|from={child_url}|propertyFrom={child_url}|navType=replace|propertyNavType=replace|order=cec:replace:{child_url},popstate:#dest\""
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(&format!(
            "data-timeout=\"hash=#dest|len=1|committed=true|finished=true|history=null|entry={{&quot;step&quot;:9}}|listener=1|property=1|from={child_url}|propertyFrom={child_url}|navType=replace|propertyNavType=replace|order=cec:replace:{child_url},popstate:#dest,cec-micro:#dest:null,committed:#dest:true,finished:#dest:true,hashchange:#dest\""
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_navigation_result_surface_exists_in_child_script() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page =
        browser
            .fetch(&server.url(
                "/compat/window-child-browsing-context-navigation-result-surface-in-child-script",
            ))
            .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-shape=\"committed,finished|committed,finished|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-settled=\"false,false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    let child_source_url =
        server.url("/compat/window-child-browsing-context-navigation-result-surface-source");
    let child_destination_url =
        server.url("/compat/window-child-browsing-context-navigation-result-surface-destination");
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-source-before=\"{child_source_url}||replace|true\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(&format!(
            "data-source-sync=\"false|false|{child_source_url}|1|0|{child_source_url}||replace|true|0|0\""
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(&format!(
            "data-source-timeout=\"false|false|{child_source_url}|1|0|{child_source_url}||replace|true|0|0\""
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-committed-shape=\"\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-finished-shape=\"\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-location-href=\"{}\"",
                child_destination_url
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-activation=\"{child_destination_url}|{child_source_url}|push\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-entry-state-type=\"undefined\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-currententrychange=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-property-currententrychange=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_navigation_navigate_keeps_committed_document_until_load()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(
            &server
                .url("/compat/window-child-browsing-context-navigation-pending-document-coherence"),
        )
        .await?;
    let initial_url = server.url("/compat/window-child-browsing-context-target-name-a");
    let final_url = server.url("/compat/window-child-browsing-context-delayed-child?via=navigate");

    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-document-text-after-load') === 'delayed'",
            Duration::from_secs(2),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-result-shape=\"committed,finished|committed,finished|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-same-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-same-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-document-same-immediate=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-default-view-same-immediate=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-default-view-document-same-immediate=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-location-immediate=\"{initial_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-document-url-immediate=\"{initial_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-document-location-immediate=\"{initial_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-current-entry-immediate=\"{initial_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-still-committed-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-target-hidden-from-document-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-same-pending-microtask=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-same-pending-microtask=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-document-same-pending-microtask=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-default-view-same-pending-microtask=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-default-view-document-same-pending-microtask=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-location-pending-microtask=\"{initial_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-document-url-pending-microtask=\"{initial_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-document-location-pending-microtask=\"{initial_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-current-entry-pending-microtask=\"{initial_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-still-committed-pending-microtask=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-target-hidden-from-document-pending-microtask=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-same-after-load=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-replaced-after-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-document-same-after-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-default-view-same-after-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-default-view-document-same-after-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-location-after-load=\"{final_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-document-url-after-load=\"{final_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-current-entry-after-load=\"{final_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-script-ran-after-load=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_navigation_push_result_surface_exists_in_child_script() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url(
            "/compat/window-child-browsing-context-navigation-push-result-surface-in-child-script",
        ))
        .await?;
    wait_for_body_attribute_contains(&browser, &mut page, "data-source-timeout", "replace|true")
        .await?;
    wait_for_body_attribute(&browser, &mut page, "data-dest-events", "0|0").await?;

    let child_source_url =
        server.url("/compat/window-child-browsing-context-navigation-push-result-surface-source");
    let child_destination_url = server
        .url("/compat/window-child-browsing-context-navigation-push-result-surface-destination");

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-source-before=\"{child_source_url}||replace|true\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(&format!(
            "data-source-sync=\"false|false|{child_source_url}|1|0|{child_source_url}||replace|true|0|0\""
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(&format!(
            "data-source-timeout=\"false|false|{child_source_url}|1|0|{child_source_url}||replace|true|0|0\""
        )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-committed=\"\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-finished=\"\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-dest-activation=\"{child_destination_url}|{child_source_url}|push\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-dest-current=\"{child_destination_url}|2|1\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-dest-state=\"null|undefined\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-dest-events=\"0|0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-location-href=\"{child_destination_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_reload_result_surface_exists_in_child_script() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(
            &server
                .url("/compat/window-child-browsing-context-reload-result-surface-in-child-script"),
        )
        .await?;

    let child_url = server.url("/compat/window-child-browsing-context-reload-result-surface-child");

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-count=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-shape=\"committed,finished|committed,finished|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-settled=\"false,false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-sync-snapshot=\"true|0|0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-committed-shape=\"\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-finished-shape=\"\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-child-location-href=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-activation=\"{child_url}|{child_url}|reload\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-currententrychange=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-property-currententrychange=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_navigation_reload_keeps_committed_document_until_load() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url(
            "/compat/window-child-browsing-context-navigation-reload-pending-document-coherence",
        ))
        .await?;
    let child_url = server.url("/compat/window-child-browsing-context-delayed-child?via=reload");

    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-delayed-load-count') === '2'",
            Duration::from_secs(2),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-result-shape=\"committed,finished|committed,finished|true|true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-same-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-same-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-document-same-immediate=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-default-view-same-immediate=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-default-view-document-same-immediate=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-location-immediate=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-document-url-immediate=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-document-location-immediate=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-current-entry-immediate=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-still-committed-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-reload-url-still-current-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-same-pending-microtask=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-same-pending-microtask=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-document-same-pending-microtask=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-default-view-same-pending-microtask=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-default-view-document-same-pending-microtask=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-location-pending-microtask=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-document-url-pending-microtask=\"{child_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-document-location-pending-microtask=\"{child_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-current-entry-pending-microtask=\"{child_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-still-committed-pending-microtask=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-reload-url-still-current-pending-microtask=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-delayed-load-count=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-same-after-load=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-replaced-after-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-document-same-after-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-default-view-same-after-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-default-view-document-same-after-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-location-after-load=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-document-url-after-load=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-current-entry-after-load=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-script-ran-after-load=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_location_reload_keeps_committed_document_until_load() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url(
            "/compat/window-child-browsing-context-location-reload-pending-document-coherence",
        ))
        .await?;
    let child_url =
        server.url("/compat/window-child-browsing-context-delayed-child?via=location-reload");

    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-delayed-load-count') === '2'",
            Duration::from_secs(2),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-reload-return=\"undefined\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-same-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-same-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-document-same-immediate=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-default-view-same-immediate=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-default-view-document-same-immediate=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-location-immediate=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-document-url-immediate=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-document-location-immediate=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-current-entry-immediate=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-still-committed-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-reload-url-still-current-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-same-pending-microtask=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-same-pending-microtask=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-document-same-pending-microtask=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-default-view-same-pending-microtask=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-default-view-document-same-pending-microtask=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-location-pending-microtask=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-document-url-pending-microtask=\"{child_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-document-location-pending-microtask=\"{child_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-current-entry-pending-microtask=\"{child_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-still-committed-pending-microtask=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-reload-url-still-current-pending-microtask=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-delayed-load-count=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-same-after-load=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-replaced-after-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-document-same-after-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-default-view-same-after-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-default-view-document-same-after-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-location-after-load=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-document-url-after-load=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-current-entry-after-load=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-script-ran-after-load=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_history_go_zero_keeps_committed_document_until_load() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url(
            "/compat/window-child-browsing-context-history-go-zero-pending-document-coherence",
        ))
        .await?;
    let child_url =
        server.url("/compat/window-child-browsing-context-delayed-child?via=history-go-zero");

    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-delayed-load-count') === '2'",
            Duration::from_secs(2),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-history-go-return=\"undefined\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-same-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-same-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-document-same-immediate=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-default-view-same-immediate=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-default-view-document-same-immediate=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-location-immediate=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-document-url-immediate=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-document-location-immediate=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-current-entry-immediate=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-still-committed-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-reload-url-still-current-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-same-pending-microtask=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-same-pending-microtask=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-document-same-pending-microtask=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-default-view-same-pending-microtask=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-default-view-document-same-pending-microtask=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-location-pending-microtask=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-document-url-pending-microtask=\"{child_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-document-location-pending-microtask=\"{child_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-current-entry-pending-microtask=\"{child_url}\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-still-committed-pending-microtask=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-reload-url-still-current-pending-microtask=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-delayed-load-count=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-same-after-load=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-replaced-after-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-document-same-after-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-default-view-same-after-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-default-view-document-same-after-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-location-after-load=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-document-url-after-load=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-current-entry-after-load=\"{child_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-script-ran-after-load=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_current_entry_same_document_uses_child_owner() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url(
            "/compat/window-child-browsing-context-current-entry-same-document-uses-child-owner",
        ))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-current-same-document=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_history_popstate_stays_window_local() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-history-popstate"))
        .await?;
    browser
        .wait_for_page_delay(&mut page, Duration::from_millis(250))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-location-unchanged=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-history-unchanged=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-popstate=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-history-state-sync=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-location-href-sync=\"{}#two\"",
                server.url("/compat/window-child-browsing-context-target-name-a")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-popstate-href=\"{}#one\"",
                server.url("/compat/window-child-browsing-context-target-name-a")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-location-href-timeout=\"{}#one\"",
                server.url("/compat/window-child-browsing-context-target-name-a")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-history-state-timeout=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-document-location-href-timeout=\"{}#one\"",
                server.url("/compat/window-child-browsing-context-target-name-a")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_fragment_navigation_persists_through_attribute_navigation()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-fragment-navigation-history"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-location-unchanged=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-history-unchanged=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-location-initial=\"{}\"",
                server.url("/compat/window-child-browsing-context-target-name-a?attr=1")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-history-length-initial=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-location-after-push=\"{}#frag\"",
                server.url("/compat/window-child-browsing-context-target-name-a?attr=1")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-history-length-after-push=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-current-entry-after-push=\"{}#frag\"",
                server.url("/compat/window-child-browsing-context-target-name-a?attr=1")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-history-length=\"3\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-location-after-attribute-navigation=\"{}\"",
                server.url("/compat/window-child-browsing-context-target-name-a?attr=1")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-document-location-after-attribute-navigation=\"{}\"",
                server.url("/compat/window-child-browsing-context-target-name-a?attr=1")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-current-entry-after-attribute-navigation=\"{}\"",
                server.url("/compat/window-child-browsing-context-target-name-a?attr=1")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_initial_navigation_keeps_joint_history_length_stable() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-initial-joint-history-timing"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-sync-history-length=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-load-history-length=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-load-history-length=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-entries-length=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_fragment_traversal_events_stay_window_local() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url(
            "/compat/window-child-browsing-context-fragment-traversal-events-are-window-local",
        ))
        .await?;
    browser
        .wait_for_page_delay(&mut page, Duration::from_millis(250))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-order=\"\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-sync-hash=\"#two\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(
            "data-child-order=\"child-popstate,child-popstate-microtask,child-hashchange,child-hashchange-microtask\""
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-timeout-hash=\"#one\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_location_hash_assignment_dispatches_local_popstate_and_hashchange()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url(
            "/compat/window-child-browsing-context-location-hash-assignment-dispatches-local-popstate-and-hashchange",
        ))
        .await?;
    wait_for_body_attribute(&browser, &mut page, "data-child-timeout-hash", "#frag").await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-location-unchanged=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-history-unchanged=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-order=\"\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-popstate-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-popstate-document-location-href=\"{}#frag\"",
                server.url("/compat/window-child-browsing-context-target-name-a")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-sync-hash=\"#frag\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-sync-history-length=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-sync-history-state=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(
            "data-child-order=\"child-popstate,child-popstate-microtask,child-hashchange,child-hashchange-microtask\""
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-timeout-hash=\"#frag\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-timeout-history-length=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-old=\"{}\"",
                server.url("/compat/window-child-browsing-context-target-name-a")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-new=\"{}#frag\"",
                server.url("/compat/window-child-browsing-context-target-name-a")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-current-entry-url=\"{}#frag\"",
                server.url("/compat/window-child-browsing-context-target-name-a")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-document-location-href=\"{}#frag\"",
                server.url("/compat/window-child-browsing-context-target-name-a")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_history_go_one_fragment_traversal_events_stay_window_local()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url(
            "/compat/window-child-browsing-context-history-go-one-fragment-traversal-events-are-window-local",
        ))
        .await?;
    browser
        .wait_for_page_delay(&mut page, Duration::from_millis(250))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-order=\"\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-currententrychange=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-sync-hash=\"#one\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-sync-state=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(
            "data-child-order=\"child-currententrychange,child-currententrychange-microtask:#two:2,child-popstate:2,child-popstate-microtask:2,child-hashchange\""
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(
            "data-child-property-order=\"child-property-currententrychange,child-property-currententrychange-microtask:#two:2\""
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-timeout-hash=\"#two\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-timeout-state=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-old=\"{}#one\"",
                server.url("/compat/window-child-browsing-context-target-name-a")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-new=\"{}#two\"",
                server.url("/compat/window-child-browsing-context-target-name-a")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-from-url=\"{}#one\"",
                server.url("/compat/window-child-browsing-context-target-name-a")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-property-from-url=\"{}#one\"",
                server.url("/compat/window-child-browsing-context-target-name-a")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-navigation-type=\"traverse\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-property-navigation-type=\"traverse\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_navigation_forward_result_promises_stay_window_local() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url(
            "/compat/window-child-browsing-context-navigation-forward-result-promises-are-window-local",
        ))
        .await?;
    browser
        .wait_for_page_delay(&mut page, Duration::from_millis(250))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-order=\"\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-sync-hash=\"#one\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-sync-state=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-sync-committed=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-sync-finished=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-sync-result-keys=\"committed,finished\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-sync-result-props=\"committed,finished\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-timeout-hash=\"#two\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-timeout-state=\"2\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-timeout-committed=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-timeout-finished=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-order=\"child-committed:#two,child-finished:#two\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-timeout-committed-type=\"[object NavigationHistoryEntry]\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-timeout-finished-type=\"[object NavigationHistoryEntry]\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-timeout-current-same-committed=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-timeout-current-same-finished=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn document_has_focus_reports_top_level_true_and_child_false_initially() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/document-has-focus-top-level-true-child-false"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-has-focus=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-has-focus=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_navigation_forward_dispatches_currententrychange_traverse_event_surface()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url(
            "/compat/window-child-browsing-context-navigation-forward-currententrychange-traverse-event-surface",
        ))
        .await?;
    browser
        .wait_for_page_delay(&mut page, Duration::from_millis(250))
        .await?;

    let child_base = server.url("/compat/window-child-browsing-context-target-name-a");
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-currententrychange=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-oncurrententrychange-fired=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-sync-hash=\"#one\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-sync-state=\"{&quot;n&quot;:1}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-timeout-hash=\"#two\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-timeout-state=\"{&quot;n&quot;:2}\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-child-from-url=\"{child_base}#one\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-navigation-type=\"traverse\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-order=\"currententrychange,currententrychange-microtask:#two\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_currententrychange_stays_window_local() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-currententrychange"))
        .await?;
    wait_for_body_attribute(&browser, &mut page, "data-child-currententrychange", "1").await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-top-currententrychange=\"0\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-currententrychange=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-property-currententrychange=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-currententrychange-urls=\"{}#one\"",
                server.url("/compat/window-child-browsing-context-target-name-a")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-property-currententrychange-urls=\"{}#one\"",
                server.url("/compat/window-child-browsing-context-target-name-a")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-currententrychange-from-urls=\"{}\"",
                server.url("/compat/window-child-browsing-context-target-name-a")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-property-currententrychange-from-urls=\"{}\"",
                server.url("/compat/window-child-browsing-context-target-name-a")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-currententrychange-navigation-types=\"push\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-property-currententrychange-navigation-types=\"push\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-current-entry-url=\"{}#one\"",
                server.url("/compat/window-child-browsing-context-target-name-a")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_activation_same_document_navigation_stays_initial() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url(
            "/compat/window-child-browsing-context-activation-same-document-navigation-stays-initial",
        ))
        .await?;
    wait_for_body_attribute_contains(
        &browser,
        &mut page,
        "data-child-activation-after-forward",
        "#one",
    )
    .await?;

    let child_url = server.url("/compat/window-child-browsing-context-target-name-a");
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-activation-initial=\"{child_url}||replace|{child_url}|true\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-activation-after-push=\"{child_url}||replace|{child_url}#one|true\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-activation-after-back=\"{child_url}||replace|{child_url}|true\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-child-activation-after-forward=\"{child_url}||replace|{child_url}#one|true\""
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_external_scripts_use_subresource_cookie_context() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let parent_url = server
        .url("/compat/window-child-browsing-context-external-script-cookie-parent")
        .replace("127.0.0.1", "localhost");

    let page = browser.fetch(&parent_url).await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-ready=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-cookie-seen=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-content-document=\"null\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-parent-is-top=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-length=\"3\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-parent-length=\"1\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn cross_origin_location_proxy_only_allows_href_and_replace_navigation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let parent_url = server
        .url("/compat/window-child-browsing-context-external-script-cookie-parent")
        .replace("127.0.0.1", "localhost");

    let mut page = browser.fetch(&parent_url).await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-child-ready') === 'true'",
            Duration::from_secs(2),
        )
        .await?;

    let result = evaluated_string(page.evaluate_runtime_expression_async(
        "(() => {\
          const loc = document.getElementById('child').contentWindow.location;\
          const denied = ['hash', 'search', 'pathname', 'protocol', 'host', 'hostname', 'port'].map((name) => {\
            try {\
              loc[name] = name === 'hash' ? '#blocked' : 'blocked';\
              return `${name}:set`;\
            } catch (error) {\
              return `${name}:${error && error.name}:${error instanceof DOMException}`;\
            }\
          });\
          let replaceResult;\
          try {\
            loc.replace('/compat/window-child-browsing-context-target-name-a?via=cross-origin-replace');\
            replaceResult = 'ok';\
          } catch (error) {\
            replaceResult = `${error && error.name}:${error instanceof DOMException}`;\
          }\
          let hrefResult;\
          try {\
            loc.href = '/compat/window-child-browsing-context-target-name-a?via=cross-origin-href';\
            hrefResult = 'ok';\
          } catch (error) {\
            hrefResult = `${error && error.name}:${error instanceof DOMException}`;\
          }\
          return JSON.stringify({ denied, replaceType: typeof loc.replace, replaceResult, hrefResult });\
        })()"
    ).await?);

    assert_eq!(
        result,
        Some(
            r#"{"denied":["hash:SecurityError:true","search:SecurityError:true","pathname:SecurityError:true","protocol:SecurityError:true","host:SecurityError:true","hostname:SecurityError:true","port:SecurityError:true"],"replaceType":"function","replaceResult":"ok","hrefResult":"ok"}"#
                .to_owned(),
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn cross_origin_window_proxy_exposes_standard_noop_shape() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let parent_url = server
        .url("/compat/window-child-browsing-context-external-script-cookie-parent")
        .replace("127.0.0.1", "localhost");

    let mut page = browser.fetch(&parent_url).await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-child-ready') === 'true'",
            Duration::from_secs(2),
        )
        .await?;

    let result = evaluated_string(
        page.evaluate_runtime_expression_async(
            "(() => {\
              try {\
                const win = document.getElementById('child').contentWindow;\
                let documentAccess;\
                try {\
                  void win.document;\
                  documentAccess = 'read';\
                } catch (error) {\
                  documentAccess = `${error && error.name}:${error instanceof DOMException}`;\
                }\
                const deniedWindowProbe = [\
                  'document',\
                  'frameElement',\
                  'history',\
                  'navigation',\
                  'localStorage',\
                  'sessionStorage',\
                  'indexedDB',\
                  'customElements',\
                  'navigator',\
                  'performance',\
                  'console',\
                  'screen',\
                  'visualViewport',\
                  'crypto',\
                  'caches',\
                  'clientInformation',\
                  'cookieStore',\
                  'credentialless',\
                  'crossOriginIsolated',\
                  'globalThis',\
                  'documentPictureInPicture',\
                  'fetch',\
                  'isSecureContext',\
                  'origin',\
                  'originAgentCluster',\
                  'scheduler',\
                  'speechSynthesis',\
                  'structuredClone',\
                  'trustedTypes',\
                  'setTimeout',\
                  'clearImmediate',\
                  'addEventListener',\
                  'dispatchEvent',\
                  'queueMicrotask',\
                  'requestAnimationFrame',\
                  'getComputedStyle',\
                  'getSelection',\
                  'matchMedia',\
                  'event',\
                  'onerror',\
                  'innerWidth',\
                  'innerHeight',\
                  'devicePixelRatio',\
                  'scrollX',\
                  'pageYOffset',\
                  'scrollTo',\
                  'open',\
                  'stop',\
                  'print',\
                  'find',\
                  'alert',\
                  'confirm',\
                  'prompt',\
                  'reportError',\
                  'btoa',\
                  'atob'\
                ].map((name) => {\
                  try {\
                    void win[name];\
                    return `${name}:read`;\
                  } catch (error) {\
                    return `${name}:${error && error.name}:${error instanceof DOMException}`;\
                  }\
                });\
                const restrictedMutationProbe = [\
                  ['deleteDocument', () => Reflect.deleteProperty(win, 'document')],\
                  ['deleteSetTimeout', () => Reflect.deleteProperty(win, 'setTimeout')],\
                  ['defineDocument', () => Object.defineProperty(win, 'document', {value: 1})],\
                  ['definePostMessage', () => Object.defineProperty(win, 'postMessage', {value: 1})],\
                  ['deleteLocationHref', () => Reflect.deleteProperty(win.location, 'href')],\
                  ['defineLocationHref', () => Object.defineProperty(win.location, 'href', {value: 1})]\
                ].map(([name, operation]) => {\
                  try {\
                    return `${name}:${String(operation())}`;\
                  } catch (error) {\
                    return `${name}:${error && error.name}`;\
                  }\
                });\
                const hasProbe = [\
                  'document',\
                  'setTimeout',\
                  'postMessage',\
                  'location',\
                  'self',\
                  'window',\
                  'frames',\
                  'parent',\
                  'top',\
                  'closed',\
                  'opener',\
                  'then',\
                  '__moliChildBrowsingContextHandle',\
                  '__moliCrossOriginWindowLocation',\
                  'unknownCrossOriginProbe'\
                ].map((name) => {\
                  try {\
                    return `${name}:${name in win}:${Object.prototype.hasOwnProperty.call(win, name)}`;\
                  } catch (error) {\
                    return `${name}:${error && error.name}:${error instanceof DOMException}`;\
                  }\
                });\
                const locationHasProbe = [\
                  'href',\
                  'hash',\
                  'replace',\
                  '__moliChildBrowsingContextHandle',\
                  'unknownCrossOriginProbe'\
                ].map((name) => `${name}:${name in win.location}:${Object.prototype.hasOwnProperty.call(win.location, name)}`);\
                const calls = ['blur', 'focus', 'close'].map((name) => {\
                  try {\
                    return `${name}:${String(win[name]())}`;\
                  } catch (error) {\
                    return `${name}:${error && error.name}:${error instanceof DOMException}`;\
                  }\
                });\
                const invalidNoopReceivers = ['blur', 'focus', 'close'].map((name) => {\
                  try {\
                    win[name].call({});\
                    return `${name}:ok`;\
                  } catch (error) {\
                    return `${name}:${error && error.name}:${error instanceof TypeError}`;\
                  }\
                });\
                let postMessageWindowReceiver;\
                try {\
                  win.postMessage.call(win, {type:'receiver-ok'}, '*');\
                  postMessageWindowReceiver = 'ok';\
                } catch (error) {\
                  postMessageWindowReceiver = `${error && error.name}:${error instanceof DOMException}`;\
                }\
                let postMessageInvalidReceiver;\
                try {\
                  win.postMessage.call({}, {type:'receiver-bad'}, '*');\
                  postMessageInvalidReceiver = 'ok';\
                } catch (error) {\
                  postMessageInvalidReceiver = `${error && error.name}:${error instanceof TypeError}`;\
                }\
                const ownNames = Object.getOwnPropertyNames(win).sort();\
                const ownKeys = Reflect.ownKeys(win).map(String).sort();\
                const locationOwnNames = Object.getOwnPropertyNames(win.location).sort();\
                const accessorDescriptors = [\
                  'window',\
                  'self',\
                  'location',\
                  'closed',\
                  'frames',\
                  'length',\
                  'top',\
                  'opener',\
                  'parent'\
                ].map((name) => {\
                  const descriptor = Object.getOwnPropertyDescriptor(win, name);\
                  const cachedDescriptor = Object.getOwnPropertyDescriptor(win, name);\
                  return [\
                    name,\
                    descriptor?.enumerable,\
                    descriptor?.configurable,\
                    [typeof descriptor?.get, descriptor?.get?.name ?? null, descriptor?.get?.length ?? null],\
                    [typeof descriptor?.set, descriptor?.set?.name ?? null, descriptor?.set?.length ?? null],\
                    descriptor?.get === cachedDescriptor?.get,\
                    descriptor?.set === cachedDescriptor?.set\
                  ];\
                });\
                const locationDescriptor = Object.getOwnPropertyDescriptor(win, 'location');\
                const locationHrefDescriptor = Object.getOwnPropertyDescriptor(win.location, 'href');\
                const locationHashDescriptor = Object.getOwnPropertyDescriptor(win.location, 'hash');\
                const postMessageDescriptor = Object.getOwnPropertyDescriptor(win, 'postMessage');\
                const noopDescriptors = ['blur', 'focus', 'close'].map((name) => {\
                  const descriptor = Object.getOwnPropertyDescriptor(win, name);\
                  return `${name}:${descriptor?.enumerable}:${descriptor?.configurable}:${descriptor?.writable}:${typeof descriptor?.value}:${descriptor?.value?.name}:${descriptor?.value?.length}`;\
                });\
                const locationReplaceDescriptor = Object.getOwnPropertyDescriptor(win.location, 'replace');\
                const setTimeoutDescriptor = Object.getOwnPropertyDescriptor(win, 'setTimeout');\
                let locationReplaceInvalidReceiver;\
                try {\
                  win.location.replace.call({}, '/compat/window-child-browsing-context-target-name-a?via=bad-replace-receiver');\
                  locationReplaceInvalidReceiver = 'ok';\
                } catch (error) {\
                  locationReplaceInvalidReceiver = `${error && error.name}:${error instanceof TypeError}`;\
                }\
                let locationReplaceForgedReceiver;\
                try {\
                  win.location.replace.call({__moliChildBrowsingContextHandle: 1}, '/compat/window-child-browsing-context-target-name-a?via=forged-replace-receiver');\
                  locationReplaceForgedReceiver = 'ok';\
                } catch (error) {\
                  locationReplaceForgedReceiver = `${error && error.name}:${error instanceof TypeError}`;\
                }\
                let locationHrefSetterInvalidReceiver;\
                try {\
                  locationHrefDescriptor.set.call({}, '/compat/window-child-browsing-context-target-name-a?via=bad-href-receiver');\
                  locationHrefSetterInvalidReceiver = 'ok';\
                } catch (error) {\
                  locationHrefSetterInvalidReceiver = `${error && error.name}:${error instanceof TypeError}`;\
                }\
                let locationGetterInvalidReceiver;\
                try {\
                  locationDescriptor.get.call({});\
                  locationGetterInvalidReceiver = 'ok';\
                } catch (error) {\
                  locationGetterInvalidReceiver = `${error && error.name}:${error instanceof TypeError}`;\
                }\
                const locationBeforeWindowAssign = win.location;\
                let windowLocationAssignResult;\
                try {\
                  win.location = '/compat/window-child-browsing-context-target-name-a?via=cross-origin-window-location';\
                  windowLocationAssignResult = 'ok';\
                } catch (error) {\
                  windowLocationAssignResult = `${error && error.name}:${error instanceof DOMException}`;\
                }\
                let documentDescriptorShape;\
                try {\
                  const documentDescriptor = Object.getOwnPropertyDescriptor(win, 'document');\
                  documentDescriptorShape = {\
                    enumerable: documentDescriptor?.enumerable,\
                    configurable: documentDescriptor?.configurable,\
                    getterType: typeof documentDescriptor?.get,\
                    setterType: typeof documentDescriptor?.set\
                  };\
                } catch (error) {\
                  documentDescriptorShape = { error: `${error && error.name}:${error instanceof DOMException}` };\
                }\
                return JSON.stringify({\
                  self: win.self === win,\
                  window: win.window === win,\
                  frames: win.frames === win,\
                  parent: win.parent === window,\
                  top: win.top === window,\
                  opener: win.opener === null,\
                  thenType: typeof win.then,\
                  length: win.length,\
                  closed: win.closed,\
                  blurType: typeof win.blur,\
                  focusType: typeof win.focus,\
                  closeType: typeof win.close,\
                  postMessageType: typeof win.postMessage,\
                  restrictedMutationProbe,\
                  hasProbe,\
                  locationHasProbe,\
                  calls,\
                  invalidNoopReceivers,\
                  postMessageWindowReceiver,\
                  postMessageInvalidReceiver,\
                  ownNamesLeakInternal: ownNames.some((name) => name.startsWith('__moli')),\
                  ownKeysLeakInternal: ownKeys.some((name) => name.startsWith('__moli')),\
                  locationOwnNamesLeakInternal: locationOwnNames.some((name) => name.startsWith('__moli')),\
                  accessorDescriptors,\
                  locationHrefDescriptor: {\
                    enumerable: locationHrefDescriptor?.enumerable,\
                    configurable: locationHrefDescriptor?.configurable,\
                    getterType: typeof locationHrefDescriptor?.get,\
                    setterType: typeof locationHrefDescriptor?.set\
                  },\
                  locationHashDescriptor: {\
                    enumerable: locationHashDescriptor?.enumerable,\
                    configurable: locationHashDescriptor?.configurable,\
                    getterType: typeof locationHashDescriptor?.get,\
                    setterType: typeof locationHashDescriptor?.set\
                  },\
                  postMessageDescriptor: {\
                    enumerable: postMessageDescriptor?.enumerable,\
                    configurable: postMessageDescriptor?.configurable,\
                    writable: postMessageDescriptor?.writable,\
                    valueType: typeof postMessageDescriptor?.value,\
                    valueName: postMessageDescriptor?.value?.name,\
                    valueLength: postMessageDescriptor?.value?.length\
                  },\
                  noopDescriptors,\
                  locationReplaceDescriptor: {\
                    enumerable: locationReplaceDescriptor?.enumerable,\
                    configurable: locationReplaceDescriptor?.configurable,\
                    writable: locationReplaceDescriptor?.writable,\
                    valueType: typeof locationReplaceDescriptor?.value,\
                    valueName: locationReplaceDescriptor?.value?.name,\
                  valueLength: locationReplaceDescriptor?.value?.length\
                  },\
                  locationReplaceInvalidReceiver,\
                  locationReplaceForgedReceiver,\
                  locationHrefSetterInvalidReceiver,\
                  locationGetterInvalidReceiver,\
                  setTimeoutDescriptor: {\
                    enumerable: setTimeoutDescriptor?.enumerable,\
                    configurable: setTimeoutDescriptor?.configurable,\
                    getterType: typeof setTimeoutDescriptor?.get,\
                    setterType: typeof setTimeoutDescriptor?.set\
                  },\
                  documentDescriptor: documentDescriptorShape,\
                  windowLocationAssignResult,\
                  locationStableAfterWindowAssign: win.location === locationBeforeWindowAssign,\
                  deniedWindowProbe,\
                  documentAccess\
                });\
              } catch (error) {\
                return JSON.stringify({ topError: `${error && error.name}:${error && error.message}` });\
              }\
            })()",
        )
        .await?,
    );

    assert_eq!(
        result,
        Some(
            r#"{"self":true,"window":true,"frames":true,"parent":true,"top":true,"opener":true,"thenType":"undefined","length":3,"closed":false,"blurType":"function","focusType":"function","closeType":"function","postMessageType":"function","restrictedMutationProbe":["deleteDocument:SecurityError","deleteSetTimeout:SecurityError","defineDocument:SecurityError","definePostMessage:SecurityError","deleteLocationHref:SecurityError","defineLocationHref:SecurityError"],"hasProbe":["document:true:true","setTimeout:true:true","postMessage:true:true","location:true:true","self:true:true","window:true:true","frames:true:true","parent:true:true","top:true:true","closed:true:true","opener:true:true","then:true:true","__moliChildBrowsingContextHandle:SecurityError:true","__moliCrossOriginWindowLocation:SecurityError:true","unknownCrossOriginProbe:SecurityError:true"],"locationHasProbe":["href:true:true","hash:true:true","replace:true:true","__moliChildBrowsingContextHandle:false:false","unknownCrossOriginProbe:false:false"],"calls":["blur:undefined","focus:undefined","close:undefined"],"invalidNoopReceivers":["blur:TypeError:true","focus:TypeError:true","close:TypeError:true"],"postMessageWindowReceiver":"ok","postMessageInvalidReceiver":"TypeError:true","ownNamesLeakInternal":false,"ownKeysLeakInternal":false,"locationOwnNamesLeakInternal":false,"accessorDescriptors":[["window",false,true,["function","get window",0],["undefined",null,null],true,true],["self",false,true,["function","get self",0],["undefined",null,null],true,true],["location",false,true,["function","get location",0],["function","set location",1],true,true],["closed",false,true,["function","get closed",0],["undefined",null,null],true,true],["frames",false,true,["function","get frames",0],["undefined",null,null],true,true],["length",false,true,["function","get length",0],["undefined",null,null],true,true],["top",false,true,["function","get top",0],["undefined",null,null],true,true],["opener",false,true,["function","get opener",0],["undefined",null,null],true,true],["parent",false,true,["function","get parent",0],["undefined",null,null],true,true]],"locationHrefDescriptor":{"enumerable":false,"configurable":false,"getterType":"undefined","setterType":"function"},"locationHashDescriptor":{"enumerable":false,"configurable":false,"getterType":"function","setterType":"function"},"postMessageDescriptor":{"enumerable":false,"configurable":false,"writable":false,"valueType":"function","valueName":"postMessage","valueLength":1},"noopDescriptors":["blur:false:false:false:function:blur:0","focus:false:false:false:function:focus:0","close:false:false:false:function:close:0"],"locationReplaceDescriptor":{"enumerable":false,"configurable":false,"writable":false,"valueType":"function","valueName":"replace","valueLength":1},"locationReplaceInvalidReceiver":"TypeError:true","locationReplaceForgedReceiver":"TypeError:true","locationHrefSetterInvalidReceiver":"TypeError:true","locationGetterInvalidReceiver":"TypeError:true","setTimeoutDescriptor":{"enumerable":false,"configurable":false,"getterType":"function","setterType":"function"},"documentDescriptor":{"enumerable":false,"configurable":false,"getterType":"function","setterType":"function"},"windowLocationAssignResult":"ok","locationStableAfterWindowAssign":true,"deniedWindowProbe":["document:SecurityError:true","frameElement:SecurityError:true","history:SecurityError:true","navigation:SecurityError:true","localStorage:SecurityError:true","sessionStorage:SecurityError:true","indexedDB:SecurityError:true","customElements:SecurityError:true","navigator:SecurityError:true","performance:SecurityError:true","console:SecurityError:true","screen:SecurityError:true","visualViewport:SecurityError:true","crypto:SecurityError:true","caches:SecurityError:true","clientInformation:SecurityError:true","cookieStore:SecurityError:true","credentialless:SecurityError:true","crossOriginIsolated:SecurityError:true","globalThis:SecurityError:true","documentPictureInPicture:SecurityError:true","fetch:SecurityError:true","isSecureContext:SecurityError:true","origin:SecurityError:true","originAgentCluster:SecurityError:true","scheduler:SecurityError:true","speechSynthesis:SecurityError:true","structuredClone:SecurityError:true","trustedTypes:SecurityError:true","setTimeout:SecurityError:true","clearImmediate:SecurityError:true","addEventListener:SecurityError:true","dispatchEvent:SecurityError:true","queueMicrotask:SecurityError:true","requestAnimationFrame:SecurityError:true","getComputedStyle:SecurityError:true","getSelection:SecurityError:true","matchMedia:SecurityError:true","event:SecurityError:true","onerror:SecurityError:true","innerWidth:SecurityError:true","innerHeight:SecurityError:true","devicePixelRatio:SecurityError:true","scrollX:SecurityError:true","pageYOffset:SecurityError:true","scrollTo:SecurityError:true","open:SecurityError:true","stop:SecurityError:true","print:SecurityError:true","find:SecurityError:true","alert:SecurityError:true","confirm:SecurityError:true","prompt:SecurityError:true","reportError:SecurityError:true","btoa:SecurityError:true","atob:SecurityError:true"],"documentAccess":"SecurityError:true"}"#.to_owned(),
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn cross_origin_top_window_proxy_length_tracks_top_child_lifecycle() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let parent_url = server
        .url("/compat/window-child-browsing-context-external-script-cookie-parent")
        .replace("127.0.0.1", "localhost");

    let mut page = browser.fetch(&parent_url).await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-child-ready') === 'true'",
            Duration::from_secs(2),
        )
        .await?;

    let result = evaluated_string(
        page.evaluate_runtime_expression_with_await_async(
            "(() => new Promise(resolve => {\
              const child = document.getElementById('child').contentWindow;\
              const replyType = `length-probe-${Math.random()}`;\
              window.addEventListener('message', function onMessage(event) {\
                if (!event.data || event.data.type !== replyType) {\
                  return;\
                }\
                window.removeEventListener('message', onMessage);\
                resolve(JSON.stringify({\
                  initialParentLength,\
                  initialTopLength,\
                  afterAppendParentLength,\
                  afterAppendTopLength,\
                  afterRemoveParentLength: event.data.parentLength,\
                  afterRemoveTopLength: event.data.topLength\
                }));\
              });\
              const initialParentLength = child.parent.length;\
              const initialTopLength = child.top.length;\
              const extra = document.createElement('iframe');\
              extra.srcdoc = '<p>extra</p>';\
              document.body.appendChild(extra);\
              const afterAppendParentLength = child.parent.length;\
              const afterAppendTopLength = child.top.length;\
              extra.remove();\
              child.postMessage({ type: 'report-length', replyType }, '*');\
            }))()",
            true,
        )
        .await?,
    );

    assert_eq!(
        result,
        Some(
            r#"{"initialParentLength":1,"initialTopLength":1,"afterAppendParentLength":2,"afterAppendTopLength":2,"afterRemoveParentLength":1,"afterRemoveTopLength":1}"#
                .to_owned(),
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn cross_origin_window_proxy_exposes_named_child_frames() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let parent_url = server
        .url("/compat/window-child-browsing-context-external-script-cookie-parent")
        .replace("127.0.0.1", "localhost");

    let mut page = browser.fetch(&parent_url).await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-child-ready') === 'true'",
            Duration::from_secs(2),
        )
        .await?;

    let before = evaluated_string(
        page.evaluate_runtime_expression_async(
            "(() => {\
              const win = document.getElementById('child').contentWindow;\
              const named = win.nestedNamed;\
              const indexed = win[0];\
              const descriptor = Object.getOwnPropertyDescriptor(win, 'nestedNamed');\
              const documentDescriptor = Object.getOwnPropertyDescriptor(win, 'document');\
              const focusDescriptor = Object.getOwnPropertyDescriptor(win, 'focus');\
              let documentAccess;\
              try {\
                void win.document;\
                documentAccess = 'read';\
              } catch (error) {\
                documentAccess = `${error && error.name}:${error instanceof DOMException}`;\
              }\
              return JSON.stringify({\
                hasNamed: 'nestedNamed' in win,\
                ownNamed: Object.prototype.hasOwnProperty.call(win, 'nestedNamed'),\
                hasDocumentCollision: Object.prototype.hasOwnProperty.call(win, 'document'),\
                hasFocusCollision: Object.prototype.hasOwnProperty.call(win, 'focus'),\
                sameAsIndexed: named === indexed,\
                documentDescriptor: {\
                  enumerable: documentDescriptor?.enumerable,\
                  configurable: documentDescriptor?.configurable,\
                  getterType: typeof documentDescriptor?.get,\
                  setterType: typeof documentDescriptor?.set,\
                  valueType: typeof documentDescriptor?.value\
                },\
                focusDescriptor: {\
                  enumerable: focusDescriptor?.enumerable,\
                  configurable: focusDescriptor?.configurable,\
                  writable: focusDescriptor?.writable,\
                  valueType: typeof focusDescriptor?.value\
                },\
                documentAccess,\
                focusType: typeof win.focus,\
                namedSelf: named.self === named,\
                namedWindow: named.window === named,\
                namedFrames: named.frames === named,\
                namedParent: named.parent === win,\
                namedTop: named.top === window,\
                namedLength: named.length,\
                descriptor: {\
                  enumerable: descriptor?.enumerable,\
                  configurable: descriptor?.configurable,\
                  writable: descriptor?.writable,\
                  valueType: typeof descriptor?.value\
                }\
              });\
            })()",
        )
        .await?,
    );

    assert_eq!(
        before,
        Some(
            r#"{"hasNamed":true,"ownNamed":true,"hasDocumentCollision":true,"hasFocusCollision":true,"sameAsIndexed":true,"documentDescriptor":{"enumerable":false,"configurable":false,"getterType":"function","setterType":"function","valueType":"undefined"},"focusDescriptor":{"enumerable":false,"configurable":false,"writable":false,"valueType":"function"},"documentAccess":"SecurityError:true","focusType":"function","namedSelf":true,"namedWindow":true,"namedFrames":true,"namedParent":true,"namedTop":true,"namedLength":0,"descriptor":{"enumerable":false,"configurable":true,"writable":false,"valueType":"object"}}"#
                .to_owned(),
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    let navigation = evaluated_string(
        page.evaluate_runtime_expression_async(
            "(() => {\
              const iframe = document.getElementById('child');\
              const win = iframe.contentWindow;\
              win.location.href = new URL('/compat/window-child-browsing-context-target-name-a?via=cross-origin-named-refresh', iframe.src).href;\
              return 'ok';\
            })()",
        )
        .await?,
    );
    assert_eq!(navigation, Some("ok".to_owned()));
    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-child-load-count') === '2'",
            Duration::from_secs(2),
        )
        .await?;

    let after = evaluated_string(
        page.evaluate_runtime_expression_async(
            "(() => {\
              const refreshed = document.getElementById('child').contentWindow;\
              const probe = (operation) => {\
                try {\
                  return String(operation());\
                } catch (error) {\
                  return `${error && error.name}:${error instanceof DOMException}`;\
                }\
              };\
              return JSON.stringify({\
                afterHasNamed: probe(() => 'nestedNamed' in refreshed),\
                afterOwnNamed: probe(() => Object.prototype.hasOwnProperty.call(refreshed, 'nestedNamed')),\
                afterNamedType: probe(() => typeof refreshed.nestedNamed),\
                afterLength: refreshed.length,\
                afterDocumentDescriptorGetter: typeof Object.getOwnPropertyDescriptor(refreshed, 'document')?.get,\
                afterFocusType: typeof refreshed.focus\
              });\
            })()",
        )
        .await?,
    );

    assert_eq!(
        after,
        Some(
            r#"{"afterHasNamed":"SecurityError:true","afterOwnNamed":"SecurityError:true","afterNamedType":"SecurityError:true","afterLength":0,"afterDocumentDescriptorGetter":"function","afterFocusType":"function"}"#
                .to_owned(),
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_iframe_host_load_events_fire_for_live_context_updates() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-iframe-load"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-parser-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-dynamic-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-invalid-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_window_identity_survives_navigation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-navigation-identity"))
        .await?;
    wait_for_body_attribute(&browser, &mut page, "data-document-updated", "name-a").await?;
    wait_for_body_attribute(&browser, &mut page, "data-listener-preserved", "kept").await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-same=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-location-updated=\"{}\"",
                server.url("/compat/window-child-browsing-context-target-name-a")
            )),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-updated=\"name-a\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-listener-preserved=\"kept\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_redirect_updates_window_document_and_navigation_url() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-redirect-coherence"))
        .await?;
    let final_url = server.url("/compat/window-child-browsing-context-redirect-child-final");

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-same=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-same=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-location-href=\"{final_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-window-document-url=\"{final_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-content-document-url=\"{final_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-navigation-entry-url=\"{final_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-body=\"redirect-final\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_delayed_document_load_does_not_block_parent_runtime() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-delayed-async-navigation"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-parent-continued=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-before-load=\"not-ready\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-load-fired=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-after-load=\"delayed\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-script-ran=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_pending_navigation_keeps_committed_window_document_until_load()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-pending-navigation-coherence"))
        .await?;
    let final_url = server.url("/compat/window-child-browsing-context-delayed-child");

    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-document-text-after-load') === 'delayed'",
            Duration::from_secs(2),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-same-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-same-immediate=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-document-same-immediate=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-default-view-same-immediate=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-default-view-document-same-immediate=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-location-immediate=\"about:blank\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-url-immediate=\"about:blank\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-entry-immediate=\"about:blank\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-text-immediate=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-same-pending-microtask=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-same-pending-microtask=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-document-same-pending-microtask=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-default-view-same-pending-microtask=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-default-view-document-same-pending-microtask=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-location-pending-microtask=\"about:blank\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-url-pending-microtask=\"about:blank\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-entry-pending-microtask=\"about:blank\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-text-pending-microtask=\"\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-same-after-load=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-replaced-after-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-window-document-same-after-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-document-default-view-same-after-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-default-view-document-same-after-load=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-location-after-load=\"{final_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-document-url-after-load=\"{final_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!("data-current-entry-after-load=\"{final_url}\"")),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-script-ran-after-load=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_browsing_context_delayed_external_script_does_not_block_parent_runtime() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-delayed-external-script"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-parent-continued=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-before-load=\"not-ready\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-load-fired=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-order=\"external,inline\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-inline-after-external=\"true\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_stale_document_completion_after_second_navigation_is_ignored()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-stale-async-navigation"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-current-child') === 'fast'",
            Duration::from_secs(2),
        )
        .await?;

    tokio::time::sleep(Duration::from_millis(260)).await;
    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-current-child') === 'fast'",
            Duration::from_secs(2),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-second-navigation-issued=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-child=\"fast\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-current-url=\"{}\"",
                server.url("/compat/window-child-browsing-context-stale-fast-child")
            ))
    );
    assert!(
        !page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-child=\"slow\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_browsing_context_stale_external_script_completion_after_navigation_is_ignored()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-stale-external-script"))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-current-child') === 'fast'",
            Duration::from_secs(2),
        )
        .await?;

    tokio::time::sleep(Duration::from_millis(260)).await;
    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-current-child') === 'fast'",
            Duration::from_secs(2),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-second-navigation-issued=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-current-child=\"fast\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains(&format!(
                "data-current-url=\"{}\"",
                server.url("/compat/window-child-browsing-context-stale-script-fast-child")
            ))
    );
    assert!(
        !page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("data-stale-script-ran=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_disconnected_document_completion_is_ignored() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-disconnected-async-navigation"))
        .await?;
    tokio::time::sleep(Duration::from_millis(180)).await;
    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-frame-removed') === 'true'",
            Duration::from_secs(2),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-frame-removed=\"true\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-load-fired=\"false\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-before-remove-document=\"not-ready\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_post_message_uses_target_child_origin() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-post-message-origin"))
        .await?;
    wait_for_body_attribute(&browser, &mut page, "data-delivered", "deliver").await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-delivered=\"deliver\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-blocked=\"false\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_cross_origin_post_message_reply_preserves_source_identity()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-post-message-cross-origin-reply"))
        .await?;

    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-child-loaded') === 'true'",
            Duration::from_secs(2),
        )
        .await?;
    let response_result = browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.getAttribute('data-response') === 'ok'",
            Duration::from_secs(2),
        )
        .await;
    assert!(
        response_result.is_ok(),
        "{}\nsubresources={:?}",
        page.serialize_html_async().await.unwrap(),
        page.subresource_network_records()
    );
    response_result?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-source-ok=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-source-ok=\"true\""),
        "{}",
        page.serialize_html_async().await.unwrap()
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-child-origin=\"http://127.0.0.1"),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_window_name_initializes_from_iframe_name() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-target-name"))
        .await?;

    let evaluated = page
        .evaluate_runtime_expression_async(
            "(() => {\
              window.addEventListener('message', event => {\
                if (event.data && event.data.name === 'iframe-name-from-attribute') {\
                  document.body.dataset.childWindowName = event.data.name;\
                }\
              });\
              const unnamed = document.createElement('iframe');\
              document.body.appendChild(unnamed);\
              const iframe = document.createElement('iframe');\
              iframe.name = 'iframe-name-from-attribute';\
              iframe.srcdoc = '<!doctype html><script>window.addEventListener(\"load\", () => parent.postMessage({ name: window.name }, \"*\"));<\\/script>';\
              document.body.appendChild(iframe);\
              return JSON.stringify({ unnamed: unnamed.contentWindow.name, named: iframe.contentWindow.name });\
            })()",
        )
        .await?;
    assert_eq!(
        evaluated_string(evaluated.clone()),
        Some(r#"{"unnamed":"","named":"iframe-name-from-attribute"}"#.to_owned()),
        "{evaluated}"
    );

    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.dataset.childWindowName === 'iframe-name-from-attribute'",
            Duration::from_secs(2),
        )
        .await?;

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_window_name_updates_after_child_navigation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-target-name"))
        .await?;
    let first_url = server.url("/compat/window-child-browsing-context-window-name-a");
    let second_url = server.url("/compat/window-child-browsing-context-window-name-b");

    let evaluated = page
        .evaluate_runtime_expression_async(&format!(
            "(() => {{\
              const iframe = document.createElement('iframe');\
              const observed = [];\
              iframe.onload = () => {{\
                observed.push(iframe.contentWindow.name);\
                document.body.dataset.windowNames = JSON.stringify(observed);\
                if (observed.length === 1) {{\
                  iframe.src = {second_url:?};\
                }}\
              }};\
              iframe.src = {first_url:?};\
              document.body.appendChild(iframe);\
              return 'started';\
            }})()"
        ))
        .await?;
    assert_eq!(
        evaluated_string(evaluated.clone()),
        Some("started".to_owned()),
        "{evaluated}"
    );
    let wait_result = browser
        .wait_for_script_truthy(
            &mut page,
            r#"document.body.dataset.windowNames === '["test","test3"]'"#,
            Duration::from_secs(2),
        )
        .await;
    assert!(
        wait_result.is_ok(),
        "{}\nsubresources={:?}",
        page.serialize_html_async().await.unwrap(),
        page.subresource_network_records()
    );
    wait_result?;

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn parser_child_browsing_context_window_name_updates_after_child_navigation() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-window-name-navigation"))
        .await?;

    let wait_result = browser
        .wait_for_script_truthy(
            &mut page,
            r#"document.body.dataset.windowNames === '["test","test3"]'"#,
            Duration::from_secs(2),
        )
        .await;
    assert!(
        wait_result.is_ok(),
        "{}\nsubresources={:?}",
        page.serialize_html_async().await.unwrap(),
        page.subresource_network_records()
    );
    wait_result?;

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn child_browsing_context_worker_constructor_uses_child_base_url() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let mut page = browser
        .fetch(&server.url("/compat/window-child-browsing-context-worker-relay"))
        .await?;

    browser
        .wait_for_script_truthy(
            &mut page,
            "document.body.dataset.message === 'worker-ready'",
            Duration::from_secs(2),
        )
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-message=\"worker-ready\"")
    );
    assert!(
        page.serialize_html_async().await.unwrap().contains(
            "data-worker-pathname=\"/compat/window-child-browsing-context-worker-relay/window-child-browsing-context-worker-relay-worker.js\""
        ),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}
