use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn preload_as_admission_matches_parser_and_dynamic_links() -> Result<()> {
    check_preload_admission(include_str!("../fixtures/preload-as.js"), 36).await
}

#[tokio::test(flavor = "multi_thread")]
async fn preload_type_and_media_admission_matches_parser_and_dynamic_links() -> Result<()> {
    check_preload_admission(include_str!("../fixtures/preload-type-media.js"), 119).await
}

#[tokio::test(flavor = "multi_thread")]
async fn preload_type_and_media_admission_uses_emulated_media_and_viewport() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let mut config = AppConfig::default();
    config.set_optional_resource_fetch_mask(moli_page_types::OptionalResourceFetchMask::ALL);
    let browser = Browser::new(config)?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    page.set_emulated_media_async(&moli_page_types::EmulatedMediaOverrides {
        media: Some("print".to_owned()),
        ..Default::default()
    })
    .await?;
    page.set_viewport_surface_async(Some(moli_page_types::ViewportSurface {
        inner_width: 240,
        inner_height: 600,
        ..Default::default()
    }))
    .await?;
    let fixture = format!(
        "(globalThis.preloadProbeMediaType = 'print', \
         globalThis.preloadProbeNarrowViewport = true, {})",
        include_str!("../fixtures/preload-type-media.js")
    );
    let observed = evaluate_preload_probe(&mut page, &fixture).await?;
    assert_eq!(observed["observations"].as_array().unwrap().len(), 119);
    assert_eq!(observed["failures"], serde_json::json!([]));
    assert_eq!(observed["recovered"], true, "{}", observed["failures"]);
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn preload_type_and_media_admission_uses_child_document_viewport() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    let observed = evaluate_preload_probe(
        &mut page,
        r#"(async () => {
          const frame = document.createElement('iframe');
          frame.width = '240';
          frame.src = '/static?preload-child';
          await new Promise(resolve => { frame.onload = resolve; document.body.append(frame); });
          const child = frame.contentWindow;
          const events = [];
          const loaded = new Promise(resolve => {
            for (const [name, media] of [
              ['narrow', '(max-width: 300px)'], ['wide', '(min-width: 500px)']
            ]) {
              const link = child.document.createElement('link');
              Object.assign(link, {rel: 'preload', as: 'fetch', media,
                href: new URL('/static?child-preload=' + name, location.href).href});
              link.onload = link.onerror = e => {
                events.push(name + ':' + e.type);
                if (name === 'narrow') resolve();
              };
              child.document.head.append(link);
            }
            setTimeout(resolve, 5000);
          });
          await loaded;
          await new Promise(resolve => setTimeout(resolve, 100));
          return {
            parentNarrow: matchMedia('(max-width: 300px)').matches,
            childNarrow: child.matchMedia('(max-width: 300px)').matches,
            events
          };
        })()"#,
    )
    .await?;
    assert_eq!(
        observed,
        serde_json::json!({"parentNarrow":false,"childNarrow":true,"events":["narrow:load"]})
    );
    server.shutdown().await;
    Ok(())
}

pub(super) async fn evaluate_preload_probe(
    page: &mut Page,
    fixture: &str,
) -> Result<serde_json::Value> {
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(
            &format!("({fixture}).then(JSON.stringify)"),
            true,
        ),
    )
    .await??;
    Ok(serde_json::from_str(
        result["value"]
            .as_str()
            .expect("preload admission observations"),
    )?)
}

async fn check_preload_admission(fixture: &str, case_count: usize) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let mut config = AppConfig::default();
    config.set_optional_resource_fetch_mask(moli_page_types::OptionalResourceFetchMask::ALL);
    let browser = Browser::new(config)?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    for context in ["top-dynamic", "top-parser"] {
        let observed = evaluate_preload_probe(&mut page, fixture).await?;
        assert_eq!(observed["context"], context);
        assert_eq!(
            observed["observations"].as_array().unwrap().len(),
            case_count
        );
        assert_eq!(observed["failures"], serde_json::json!([]), "{context}");
        assert_eq!(observed["recovered"], true, "{}", observed["failures"]);
        if context == "top-dynamic" {
            let mut url = url::Url::parse(&server.url("/compat/child-dynamic-markup-document"))?;
            url.query_pairs_mut()
                .append_pair("markup", observed["parserMarkup"].as_str().unwrap());
            page = browser.fetch(url.as_str()).await?;
        }
    }
    server.shutdown().await;
    Ok(())
}
