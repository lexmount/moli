use super::*;

async fn check_runtime_modulepreloads(child: bool) -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/runtime-modulepreload.html", servers.origin))
        .await?;
    let probe = format!(
        r#"(async () => {{
          if (document.readyState !== 'complete')
            await new Promise(resolve => addEventListener('load', resolve, {{once:true}}));
          const check = (value, message) => {{ if (!value) throw new Error(message); }};
          const preload = (href, as, integrity) => new Promise((resolve, reject) => {{
            const link = document.createElement('link');
            link.rel = 'modulepreload'; link.as = as; link.href = href;
            if (integrity !== undefined) link.integrity = integrity;
            const timeout = setTimeout(() => reject(new Error('no preload event: ' + href)), 3000);
            const done = event => {{ clearTimeout(timeout); resolve(event.type); }};
            link.onload = done; link.onerror = done;
            document.head.append(link);
          }});
          const paths = ['/echo-origin.js?runtime', '/module.json?runtime',
            '/style.css?runtime', '/text-module.txt?runtime'];
          const types = ['script', 'json', 'style', 'text'];
          const events = await Promise.all(paths.map((path, i) => preload(path, types[i])));
          check(events.every(event => event === 'load'), 'preload load events: ' + events);
          check(!globalThis.sriExecutions, 'preload evaluated JavaScript');
          const js = await import(paths[0]);
          check(js === await import(paths[0]), 'module namespace identity');
          check(sriExecutions === 1, 'JavaScript must evaluate once');
          check((await import(paths[1], {{with:{{type:'json'}}}})).default.answer === 42, 'JSON');
          check((await import(paths[2], {{with:{{type:'css'}}}})).default instanceof CSSStyleSheet, 'CSS');
          check((await import(paths[3], {{with:{{type:'text'}}}})).default === {script}, 'text');
          check(await preload(paths[0], 'script') === 'load', 'already fetched module event');
          const failures = await Promise.all([
            preload('/missing.js?runtime', 'script'),
            preload({cross} + '/script.js?runtime', 'script'),
            preload('/echo-origin.js?bad-integrity', 'script', 'sha384-AAAA'),
            preload('/echo-origin.js?map-integrity', 'script'),
            preload('/echo-origin.js?invalid-as', 'image')
          ]);
          check(failures.every(event => event === 'error'), 'preload errors: ' + failures);
          return true;
        }})()"#,
        script = serde_json::to_string(SCRIPT)?,
        cross = serde_json::to_string(&servers.cross_origin)?,
    );
    let expression = if child {
        format!(
            r#"(async () => {{
                const frame = document.createElement('iframe');
                frame.src = '/runtime-modulepreload.html?child';
                await new Promise(resolve => {{ frame.onload = resolve; document.body.append(frame); }});
                return frame.contentWindow.eval({});
            }})()"#,
            serde_json::to_string(&probe)?
        )
    } else {
        probe
    };
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(&expression, true),
    )
    .await??;
    assert_eq!(
        result["value"],
        true,
        "child={child}: {result}; requests={:?}",
        servers.requests.lock()
    );
    let requests = servers.requests.lock();
    for (path, destination) in [
        ("/echo-origin.js?runtime", "script"),
        ("/module.json?runtime", "json"),
        ("/style.css?runtime", "style"),
        ("/text-module.txt?runtime", "text"),
        ("/missing.js?runtime", "script"),
        ("/script.js?runtime", "script"),
        ("/echo-origin.js?bad-integrity", "script"),
        ("/echo-origin.js?map-integrity", "script"),
    ] {
        let matching: Vec<_> = requests
            .iter()
            .filter(|request| request.path == path)
            .collect();
        assert_eq!(matching.len(), 1, "child={child}, {path}: {requests:?}");
        assert_eq!(
            matching[0].sec_fetch_dest.as_deref(),
            Some(destination),
            "{path}"
        );
        assert_eq!(
            matching[0].sec_fetch_mode.as_deref(),
            Some("cors"),
            "{path}"
        );
    }
    assert!(
        !requests
            .iter()
            .any(|request| request.path.contains("invalid-as"))
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_modulepreloads_fetch_after_load_and_reuse_the_main_module_map() -> Result<()> {
    check_runtime_modulepreloads(false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_modulepreloads_fetch_after_load_and_reuse_the_child_module_map() -> Result<()> {
    check_runtime_modulepreloads(true).await
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_modulepreloads_keep_document_maps_and_retire_queued_child_starts() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/page.html", servers.origin))
        .await?;
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(
            r#"(async () => {
              if (document.readyState !== 'complete')
                await new Promise(resolve => addEventListener('load', resolve, {once:true}));
              const preload = (doc, path) => new Promise((resolve, reject) => {
                const link = doc.createElement('link');
                link.rel = 'modulepreload'; link.href = path;
                const timeout = setTimeout(() => reject(new Error('no preload event: ' + path)), 3000);
                link.onload = () => { clearTimeout(timeout); resolve(); };
                link.onerror = () => { clearTimeout(timeout); reject(new Error('preload error: ' + path)); };
                doc.head.append(link);
              });
              const path = '/echo-origin.js?separate-maps';
              await preload(document, path);
              const frame = document.createElement('iframe');
              frame.src = '/page.html?child';
              await new Promise(resolve => { frame.onload = resolve; document.body.append(frame); });
              const child = frame.contentDocument;
              await preload(child, path);
              let retiredEvents = 0;
              const oldLink = child.createElement('link');
              oldLink.rel = 'modulepreload'; oldLink.href = '/echo-origin.js?retired-child';
              oldLink.onload = oldLink.onerror = () => retiredEvents++;
              child.head.append(oldLink);
              // Retire the captured Document before its queued start can run.
              child.open(); child.write('<!doctype html><head></head><body>replacement'); child.close();
              await preload(child, '/echo-origin.js?replacement-child');
              if (retiredEvents !== 0) throw new Error('retired child link received an event');
              document.open(); document.write('<!doctype html><head></head><body>replacement'); document.close();
              await preload(document, path);
              if (globalThis.sriExecutions) throw new Error('preload executed');
              await import(path);
              if (sriExecutions !== 1) throw new Error('replacement module evaluation');
              return true;
            })()"#,
            true,
        ),
    ).await??;
    assert_eq!(
        result["value"],
        true,
        "{result}; requests={:?}",
        servers.requests.lock()
    );
    let requests = servers.requests.lock();
    for (path, expected) in [
        // document.open replaces the tree but retains the Window's module map.
        ("/echo-origin.js?separate-maps", 2),
        ("/echo-origin.js?retired-child", 0),
        ("/echo-origin.js?replacement-child", 1),
    ] {
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == path)
                .count(),
            expected,
            "{path}: {requests:?}"
        );
    }
    Ok(())
}
