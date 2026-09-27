use super::*;

pub(super) fn preload_markup() -> String {
    format!(
        r#"<!doctype html>
        <script>globalThis.sriExecutions = 0;</script>
        <script type="importmap">{{"integrity": {{
          "./text-module.txt?bad": "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
          "./text-module.txt?preload": "{INTEGRITY}"
        }}}}</script>
        <link rel="modulepreload" as="text" href="/text-module.txt?preload"
          onload="globalThis.textPreloaded = true" onerror="globalThis.textPreloaded = false">
        <body>Text module preload"#
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn text_modules_reuse_preloads_and_enforce_cors_and_integrity() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/text-module-preload.html", servers.origin))
        .await?;
    let expression = format!(
        r#"(async () => {{
          const link = document.querySelector('link');
          if (globalThis.textPreloaded === undefined) {{
            await new Promise((resolve, reject) => {{
              link.addEventListener('load', resolve);
              link.addEventListener('error', () => reject(new Error('text preload failed')));
            }});
          }}
          if (globalThis.textPreloaded !== true) throw new Error('text preload did not load');
          const [first, second] = await Promise.all([
            import(link.href, {{with: {{type: 'text'}}}}),
            import(link.href, {{with: {{type: 'text'}}}})
          ]);
          const rejected = url => import(url, {{with: {{type: 'text'}}}})
            .then(() => 'fulfilled', error => error.name);
          const badIntegrity = await rejected('/text-module.txt?bad');
          const badCors = await rejected({cross:?} + '/text-module.txt?no-cors');
          const missing = await rejected('/missing.txt');
          const goodCors = await import({cross:?} + '/cors.js?allowed', {{with: {{type: 'text'}}}});
          return JSON.stringify([first.default, first === second, badIntegrity, badCors,
            missing, goodCors.default === first.default, sriExecutions]);
        }})()"#,
        cross = servers.cross_origin,
    );
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(&expression, true),
    )
    .await??;
    let result: serde_json::Value =
        serde_json::from_str(result["value"].as_str().unwrap_or_else(|| {
            panic!(
                "text module network results: {result}; requests={:?}",
                servers.requests.lock()
            )
        }))?;
    assert_eq!(
        result,
        serde_json::json!([SCRIPT, true, "TypeError", "TypeError", "TypeError", true, 0])
    );
    let requests = servers.requests.lock();
    for path in [
        "/text-module.txt?preload",
        "/text-module.txt?bad",
        "/text-module.txt?no-cors",
        "/missing.txt",
        "/cors.js?allowed",
    ] {
        let matching: Vec<_> = requests.iter().filter(|r| r.path == path).collect();
        assert_eq!(matching.len(), 1, "{path}: {requests:?}");
        let request = matching[0];
        assert_eq!(request.sec_fetch_dest.as_deref(), Some("text"), "{path}");
        assert_eq!(request.sec_fetch_mode.as_deref(), Some("cors"), "{path}");
    }
    Ok(())
}
