use moli_test_support as support;

use anyhow::Result;
use moli_core::runtime::{
    Browser, BrowserConfig as AppConfig, FetchReadinessTimeout, FetchTimeoutPhase,
    RawDocumentPageRequired, RenderedDomWaitUntil,
};
use moli_fetch::Request;
use std::time::{Duration, Instant};
use support::FixtureServer;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use url::Url;

async fn spawn_stalled_binary_response_server() -> Result<(String, JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let mut request = [0_u8; 4096];
        let Ok(read) = stream.read(&mut request).await else {
            return;
        };
        if read == 0 {
            return;
        }
        if stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 1048576\r\nConnection: keep-alive\r\n\r\n",
            )
            .await
            .is_err()
        {
            return;
        }
        std::future::pending::<()>().await;
    });
    Ok((format!("http://{address}/stalled.bin"), server))
}

#[tokio::test]
async fn fetches_static_fixture() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.session().open(&server.url("/static")).await?;

    assert_eq!(page.status(), 200);
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("fixture static")
    );
    assert!(
        page.headers()
            .iter()
            .any(|(name, _)| name == "content-type")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn follows_redirects() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser.fetch(&server.url("/redirect")).await?;

    assert_eq!(page.status(), 200);
    assert_eq!(page.final_url().path(), "/static");
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("fixture static")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn fetch_reports_404_as_error() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let error = browser
        .fetch(&server.url("/net/upstream/xhr/404"))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("404"));

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn fetch_renders_403_challenge_document() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_allow_http_error(&server.url("/net/upstream/xhr/403-challenge"))
        .await?;
    assert_eq!(page.status(), 403);
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("forbidden challenge")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn fetch_reports_500_as_error() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let error = browser
        .fetch(&server.url("/net/upstream/xhr/500"))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("500"));

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn fetch_allow_http_error_returns_page_for_success() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_allow_http_error(&server.url("/static"))
        .await?;
    assert_eq!(page.status(), 200);
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("fixture static")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn fetch_with_wait_until_rejects_raw_document_through_page_api() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let error = browser
        .fetch_with_wait_until(
            &server.url("/net/upstream/xhr/binary"),
            RenderedDomWaitUntil::Load,
            Duration::from_secs(1),
        )
        .await
        .expect_err("the Page fetch API must reject a raw non-HTML document");

    assert_eq!(
        error.to_string(),
        "raw non-HTML document cannot be returned through the Page fetch API"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn page_fetch_rejects_raw_response_without_waiting_for_its_stalled_body() -> Result<()> {
    let (url, server) = spawn_stalled_binary_response_server().await?;
    let browser = Browser::new(AppConfig::default())?;
    let started = Instant::now();

    let error = tokio::time::timeout(
        Duration::from_secs(1),
        browser.fetch_with_wait_until(&url, RenderedDomWaitUntil::Load, Duration::from_millis(100)),
    )
    .await
    .expect("the Page API must not wait for a raw response body")
    .expect_err("the Page API must reject a raw non-HTML document");

    assert!(error.is::<RawDocumentPageRequired>());
    assert_eq!(
        error.to_string(),
        "raw non-HTML document cannot be returned through the Page fetch API"
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "raw Page rejection must not fall through to the HTTP transport timeout"
    );
    server.abort();
    Ok(())
}

#[tokio::test]
async fn raw_document_body_materialization_uses_the_fetch_deadline() -> Result<()> {
    let (url, server) = spawn_stalled_binary_response_server().await?;
    let browser = Browser::new(AppConfig::default())?;

    let error = tokio::time::timeout(
        Duration::from_secs(1),
        browser.fetch_request_document_allow_http_error_with_wait_until(
            Request::get(&url)?,
            RenderedDomWaitUntil::Load,
            Duration::from_millis(100),
        ),
    )
    .await
    .expect("raw body materialization must honor the fetch deadline")
    .expect_err("the stalled raw body must time out");
    let timeout = error
        .downcast_ref::<FetchReadinessTimeout>()
        .expect("the raw body timeout must retain its typed phase");
    assert_eq!(timeout.phase(), FetchTimeoutPhase::StreamingMainBody);

    server.abort();
    Ok(())
}

#[tokio::test]
async fn fetch_allow_http_error_renders_404_response() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let url = server.url("/net/upstream/xhr/404");

    let page = browser.fetch_allow_http_error(&url).await?;
    assert_eq!(page.status(), 404);
    assert_eq!(page.final_url().as_str(), url);
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("Not Found")
    );
    assert!(
        page.headers()
            .iter()
            .any(|(name, _)| name == "content-type")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn follows_location_replace_during_page_load() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/location-nav/replace-source"))
        .await?;

    assert_eq!(page.final_url().path(), "/location-nav/target");
    assert_eq!(page.final_url().query(), Some("from=replace"));
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("location-target=replace")
    );
    assert!(
        !page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("replace-source")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn follows_location_assign_href_and_search_setters_during_page_load() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let assign_page = browser
        .fetch(&server.url("/location-nav/assign-source"))
        .await?;
    assert_eq!(assign_page.final_url().path(), "/location-nav/target");
    assert_eq!(assign_page.final_url().query(), Some("from=assign"));
    assert!(
        assign_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("location-target=assign")
    );
    assert!(
        !assign_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("assign-source")
    );

    let href_page = browser
        .fetch(&server.url("/location-nav/href-source"))
        .await?;
    assert_eq!(href_page.final_url().path(), "/location-nav/target");
    assert_eq!(href_page.final_url().query(), Some("from=href"));
    assert!(
        href_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("location-target=href")
    );
    assert!(
        !href_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("href-source")
    );

    let pathname_page = browser
        .fetch(&server.url("/location-nav/pathname-source"))
        .await?;
    assert_eq!(
        pathname_page.final_url().path(),
        "/location-nav/pathname-target"
    );
    assert_eq!(pathname_page.final_url().query(), None);
    assert!(
        pathname_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("location-target=pathname")
    );
    assert!(
        !pathname_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("pathname-source")
    );

    let search_page = browser
        .fetch(&server.url("/location-nav/search-source"))
        .await?;
    assert_eq!(
        search_page.final_url().path(),
        "/location-nav/search-source"
    );
    assert_eq!(search_page.final_url().query(), Some("from=search"));
    assert!(
        search_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("location-target=search")
    );
    assert!(
        !search_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("search-source")
    );

    let host_page = browser
        .fetch(&server.url("/location-nav/host-source"))
        .await?;
    assert_eq!(host_page.final_url().host_str(), Some("localhost"));
    assert_eq!(host_page.final_url().path(), "/location-nav/host-source");
    assert!(
        host_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("location-target=host")
    );
    assert!(
        !host_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("host-source")
    );

    let hostname_page = browser
        .fetch(&server.url("/location-nav/hostname-source"))
        .await?;
    assert_eq!(hostname_page.final_url().host_str(), Some("localhost"));
    assert_eq!(
        hostname_page.final_url().path(),
        "/location-nav/hostname-source"
    );
    assert!(
        hostname_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("location-target=hostname")
    );
    assert!(
        !hostname_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("hostname-source")
    );

    let target_server = FixtureServer::spawn().await?;
    let target_port = fixture_url_port(&target_server.url("/static"));
    let port_page = browser
        .fetch(&server.url(&format!(
            "/location-nav/port-source?targetPort={target_port}"
        )))
        .await?;
    let expected_query = format!("targetPort={target_port}");
    assert_eq!(port_page.final_url().port(), Some(target_port));
    assert_eq!(port_page.final_url().path(), "/location-nav/port-source");
    assert_eq!(port_page.final_url().query(), Some(expected_query.as_str()));
    assert!(
        port_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("location-target=port")
    );
    assert!(
        !port_page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("port-source")
    );
    target_server.shutdown().await;

    server.shutdown().await;
    Ok(())
}

fn fixture_url_port(url: &str) -> u16 {
    Url::parse(url)
        .expect("fixture URL should parse")
        .port()
        .expect("fixture URL should include an explicit port")
}

#[tokio::test]
async fn follows_location_reload_during_page_load() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/location-nav/reload-source"))
        .await?;

    assert_eq!(page.final_url().path(), "/location-nav/reload-source");
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("location-reload=done")
    );
    assert!(
        !page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("reload-source")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn follows_same_href_location_assignment_after_script_cookie_challenge() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/location-nav/same-href-cookie-challenge"))
        .await?;

    assert_eq!(
        page.final_url().path(),
        "/location-nav/same-href-cookie-challenge"
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("same-href-cookie-challenge=done")
    );
    assert!(
        !page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("same-href-cookie-challenge=source")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn follows_chained_location_navigation_during_page_load() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/location-nav/chain-source"))
        .await?;

    assert_eq!(page.final_url().path(), "/location-nav/target");
    assert_eq!(page.final_url().query(), Some("from=chain-mid"));
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("location-target=chain-mid")
    );
    assert!(
        !page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("chain-source")
    );
    assert!(
        !page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("<main id=\"mid\">chain-mid</main>")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn follows_timeout_chained_location_navigation_during_network_idle_wait() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/location-nav/chain-timeout-source"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(page.final_url().path(), "/location-nav/target");
    assert_eq!(page.final_url().query(), Some("from=chain-mid"));
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("location-target=chain-mid")
    );
    assert!(
        !page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("chain-timeout-source")
    );
    assert!(
        !page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("chain-source")
    );
    assert!(
        !page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("<main id=\"mid\">chain-mid</main>")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn follows_timeout_chained_location_navigation_during_domstable_wait() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/location-nav/chain-timeout-source"),
            RenderedDomWaitUntil::DomStable,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(page.final_url().path(), "/location-nav/target");
    assert_eq!(page.final_url().query(), Some("from=chain-mid"));
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("location-target=chain-mid")
    );
    assert!(
        !page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("chain-timeout-source")
    );
    assert!(
        !page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("chain-source")
    );
    assert!(
        !page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("<main id=\"mid\">chain-mid</main>")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn rejects_location_navigation_loop_during_page_load() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let error = match browser.fetch(&server.url("/location-nav/loop-a")).await {
        Ok(_) => panic!("location navigation loop should fail"),
        Err(error) => error,
    };
    let error_message = format!("{error:#}");
    assert!(
        error_message.contains("too many chained location navigations"),
        "unexpected error: {error:#}"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn follows_async_location_search_during_network_idle_wait() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/location-nav/search-async-source"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(page.final_url().path(), "/location-nav/search-async-source");
    assert_eq!(page.final_url().query(), Some("from=search-async"));
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("location-target=search-async")
    );
    assert!(
        !page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("search-async-source")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn follows_async_location_search_during_domstable_wait() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/location-nav/search-async-source"),
            RenderedDomWaitUntil::DomStable,
            Duration::from_secs(5),
        )
        .await?;

    assert_eq!(page.final_url().path(), "/location-nav/search-async-source");
    assert_eq!(page.final_url().query(), Some("from=search-async"));
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("location-target=search-async")
    );
    assert!(
        !page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("search-async-source")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn rejects_timeout_location_navigation_loop_during_network_idle_wait() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let error = match browser
        .fetch_with_wait_until(
            &server.url("/location-nav/loop-timeout-source"),
            RenderedDomWaitUntil::NetworkIdle,
            Duration::from_secs(20),
        )
        .await
    {
        Ok(_) => panic!("location navigation loop should fail during networkidle wait"),
        Err(error) => error,
    };
    let error_message = format!("{error:#}");
    assert!(
        error_message.contains("too many chained location navigations"),
        "unexpected error: {error:#}"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn rejects_timeout_location_navigation_loop_during_domstable_wait() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let error = match browser
        .fetch_with_wait_until(
            &server.url("/location-nav/loop-timeout-source"),
            RenderedDomWaitUntil::DomStable,
            Duration::from_secs(20),
        )
        .await
    {
        Ok(_) => panic!("location navigation loop should fail during domstable wait"),
        Err(error) => error,
    };
    let error_message = format!("{error:#}");
    assert!(
        error_message.contains("too many chained location navigations"),
        "unexpected error: {error:#}"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn follows_post_parse_timeout_location_assign_during_page_load() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch_with_wait_until(
            &server.url("/location-nav/assign-post-parse-timeout-source"),
            RenderedDomWaitUntil::DomStable,
            Duration::from_secs(2),
        )
        .await?;

    assert_eq!(page.final_url().path(), "/location-nav/target");
    assert_eq!(
        page.final_url().query(),
        Some("from=assign-post-parse-timeout")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("location-target=assign-post-parse-timeout")
    );
    assert!(
        !page
            .serialize_html_async()
            .await
            .unwrap()
            .contains("assign-post-parse-timeout-source"),
        "{}",
        page.serialize_html_async().await.unwrap()
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn repeated_date_locale_formatting_does_not_crash_fetch() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/date-locale-bomb"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-ok=\"1\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn date_locale_methods_return_stable_values_and_invalid_date() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;

    let page = browser
        .fetch(&server.url("/compat/date-locale-details"))
        .await?;

    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-locale-string=\"3/24/2024, 4:05:06 PM\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-locale-date=\"3/24/2024\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-locale-time=\"4:05:06 PM\"")
    );
    assert!(
        page.serialize_html_async()
            .await
            .unwrap()
            .contains("data-invalid=\"Invalid Date\"")
    );

    server.shutdown().await;
    Ok(())
}
