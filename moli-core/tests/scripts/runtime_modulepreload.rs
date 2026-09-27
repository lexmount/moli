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

async fn check_module_script_consumers(child: bool) -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/page.html", servers.origin))
        .await?;
    let probe = r#"(async () => {
      const check = (value, message) => { if (!value) throw new Error(message); };
      const attach = (tag, url, attributes = {}) => new Promise((resolve, reject) => {
        const element = document.createElement(tag);
        if (tag === 'link') { element.rel = 'modulepreload'; element.href = url; }
        else { element.type = 'module'; element.src = url; }
        Object.assign(element, attributes);
        const timeout = setTimeout(() => reject(new Error('no event: ' + url)), 3000);
        element.onload = element.onerror = event => { clearTimeout(timeout); resolve(event.type); };
        document.head.append(element);
      });
      for (const [name, warmup, ordered] of [
        ['preloaded', 'preload', false],
        ['ordered', 'preload', true],
        ['credentials', 'preload', false],
        ['integrity', 'preload', false],
        ['imported', 'import', false],
        ['evaluated', 'script', false],
        ['concurrent', 'concurrent-preload', false],
        ['cold', 'none', false]
      ]) {
        const url = '/echo-origin.js?consumer-' + name;
        const before = globalThis.sriExecutions || 0;
        const pending = [];
        if (warmup === 'preload') {
          check(await attach('link', url, {crossOrigin: 'anonymous'}) === 'load', name + ' preload');
          check((globalThis.sriExecutions || 0) === before, name + ' premature evaluation');
        } else if (warmup === 'import') await import(url);
        else if (warmup === 'script') check(await attach('script', url) === 'load', name + ' first script');
        else if (warmup === 'concurrent-preload') pending.push(attach('link', url));
        const attributes = {async: !ordered};
        if (name === 'credentials') attributes.crossOrigin = 'use-credentials';
        // The already-fetched module is reused before new fetch options are examined.
        if (name === 'integrity') attributes.integrity = 'sha384-AAAA';
        pending.push(attach('script', url, attributes), attach('script', url, attributes));
        check((await Promise.all(pending)).every(event => event === 'load'), name + ' terminal events');
        check(sriExecutions === before + 1, name + ' must evaluate exactly once');
        const namespace = await import(url);
        check(namespace === await import(url), name + ' namespace identity');
        check(sriExecutions === before + 1, name + ' import must reuse evaluation');
      }
      // Fetch policy must still be enforced on roots that are absent from the map.
      check(await attach('script', '/echo-origin.js?consumer-bad-sri', {integrity:'sha384-AAAA'}) === 'error', 'fresh SRI');
      check(await attach('script', '/missing.js?consumer-missing') === 'error', 'fresh 404');
      const beforeClassic = sriExecutions;
      for (let i = 0; i < 2; i++)
        check(await attach('script', '/echo-origin.js?consumer-classic', {type:''}) === 'load', 'classic load');
      check(sriExecutions === beforeClassic + 2, 'classic scripts must each evaluate');
      return true;
    })()"#;
    let expression = if child {
        format!(
            r#"(async () => {{
                const frame = document.createElement('iframe');
                frame.src = '/page.html?child';
                await new Promise(resolve => {{ frame.onload = resolve; document.body.append(frame); }});
                return frame.contentWindow.eval({});
            }})()"#,
            serde_json::to_string(probe)?
        )
    } else {
        probe.to_owned()
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
    for name in [
        "preloaded",
        "ordered",
        "credentials",
        "integrity",
        "imported",
        "evaluated",
        "concurrent",
        "cold",
        "bad-sri",
        "missing",
        "classic",
    ] {
        let path = if name == "missing" {
            "/missing.js?consumer-missing".to_owned()
        } else {
            format!("/echo-origin.js?consumer-{name}")
        };
        let matching: Vec<_> = requests
            .iter()
            .filter(|request| request.path == path)
            .collect();
        assert_eq!(
            matching.len(),
            if name == "classic" { 2 } else { 1 },
            "{path}: {requests:?}"
        );
        assert_eq!(
            matching[0].sec_fetch_dest.as_deref(),
            Some("script"),
            "{path}"
        );
        assert_eq!(
            matching[0].sec_fetch_mode.as_deref(),
            Some(if name == "classic" { "no-cors" } else { "cors" }),
            "{path}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_script_consumers_share_the_main_module_map() -> Result<()> {
    check_module_script_consumers(false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn module_script_consumers_share_the_child_module_map() -> Result<()> {
    check_module_script_consumers(true).await
}

#[tokio::test(flavor = "multi_thread")]
async fn module_script_map_reservations_preserve_service_worker_responses() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/page.html", servers.origin))
        .await?;
    let cases = serde_json::json!([
        {"name":"synthetic", "src":"/sw-default.js", "type":"module", "integrity":INTEGRITY, "expected":"load"},
        {"name":"cors", "src":"/sw-cors.js", "type":"module", "integrity":INTEGRITY, "expected":"load"},
        {"name":"opaque", "src":"/sw-opaque.js", "type":"module", "expected":"error"}
    ]);
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(&dynamic_probe(&cases, true), true),
    )
    .await??;
    assert_integrity_results(result, &cases);
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        page.evaluate_runtime_expression_with_await_async(
            r#"(async () => {
                const before = sriExecutions;
                await import('/sw-default.js');
                const script = document.createElement('script');
                script.type = 'module'; script.src = '/sw-default.js';
                await new Promise((resolve, reject) => {
                    script.onload = resolve; script.onerror = reject; document.head.append(script);
                });
                return sriExecutions === before;
            })()"#,
            true,
        ),
    )
    .await??;
    assert_eq!(result["value"], true, "{result}");
    assert!(
        !servers
            .requests
            .lock()
            .iter()
            .any(|request| request.path.starts_with("/sw-")),
        "intercepted roots must not reach the network"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_script_fetch_settles_the_retained_map_after_document_open() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/page.html", servers.origin))
        .await?;
    let result = tokio::time::timeout(Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(r#"(async () => {
          const url = '/echo-origin.js?consumer-open';
          globalThis.retiredModuleEvents = 0;
          const script = document.createElement('script'); script.type = 'module'; script.src = url;
          script.onload = script.onerror = () => retiredModuleEvents++;
          document.head.append(script);
          await fetch('/module-response-started');
          document.open(); document.write('<!doctype html><head></head><body>replacement'); document.close();
          const imported = import(url);
          await fetch('/module-response-release');
          await imported;
          return JSON.stringify({executions: sriExecutions, retiredEvents: retiredModuleEvents,
            entries: performance.getEntriesByName(new URL(url, location.href).href).length});
        })()"#, true),
    ).await??;
    assert!(result["value"].is_string(), "{result}");
    let actual: serde_json::Value = serde_json::from_str(result["value"].as_str().unwrap())?;
    assert_eq!(
        actual,
        serde_json::json!({"executions":1,"retiredEvents":0,"entries":0})
    );
    assert_eq!(
        servers
            .requests
            .lock()
            .iter()
            .filter(|request| request.path == "/echo-origin.js?consumer-open")
            .count(),
        1
    );
    Ok(())
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
