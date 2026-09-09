use super::*;

fn markup_url(server: &FixtureServer, markup: &str) -> String {
    let mut url = url::Url::parse(&server.url("/compat/child-dynamic-markup-document")).unwrap();
    url.query_pairs_mut().append_pair("markup", markup);
    url.into()
}

fn module_url(source: &str) -> String {
    format!(
        "data:text/javascript,{}",
        url::form_urlencoded::byte_serialize(source.as_bytes())
            .collect::<String>()
            .replace('+', "%20")
    )
}

async fn child_module_load_precedes_delayed_write(variant: &str) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    for timer_owner in ["parent", "window"] {
        let write = r#"
          parent.events.push('timer:' + document.readyState);
          document.write('replacement');
          document.close();
          parent.events.push('write');
          parent.finishWrite();
        "#;
        let source = if variant == "tla" {
            format!("await new Promise(resolve => {timer_owner}.setTimeout(resolve, 0)); {write}")
        } else {
            format!("{timer_owner}.setTimeout(() => {{ {write} }}, 0);")
        };
        let script = match variant {
            "import" => format!(
                "<script type=module>import {};</script>",
                serde_json::to_string(&module_url(&source))?
            ),
            "external" => format!(
                "<script type=module src=\"{}\"></script>",
                module_url(&source)
            ),
            _ => format!("<script type=module>{source}</script>"),
        };
        let child = markup_url(
            &server,
            &format!("<!doctype html><head>{script}</head><body>original"),
        );
        let parent = format!(
            r#"<!doctype html><body><script>
              window.events = [];
              window.writeDone = new Promise(resolve => window.finishWrite = resolve);
              const frame = document.createElement('iframe');
              frame.id = 'target';
              frame.onload = () => events.push('load:' + frame.contentDocument.body.textContent);
              frame.src = {};
              document.body.append(frame);
            </script>"#,
            serde_json::to_string(&child)?
        );
        let mut page = browser.fetch(&markup_url(&server, &parent)).await?;
        let result = page.evaluate_runtime_expression_with_await_async(
            "writeDone.then(() => JSON.stringify({events, body: document.getElementById('target').contentDocument.body.textContent}))",
            true,
        ).await?;
        let result: serde_json::Value =
            serde_json::from_str(result["value"].as_str().expect("delayed write observation"))?;
        let events = result["events"].as_array().expect("child event sequence");
        assert_eq!(
            events.iter().take(3).collect::<Vec<_>>(),
            vec![
                &serde_json::json!("load:original"),
                &serde_json::json!("timer:complete"),
                &serde_json::json!("write")
            ],
            "variant={variant}, timer_owner={timer_owner}, events={events:?}"
        );
        assert_eq!(
            result["body"], "replacement",
            "variant={variant}, timer_owner={timer_owner}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_inline_module_load_precedes_delayed_document_write() -> Result<()> {
    child_module_load_precedes_delayed_write("inline").await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_imported_module_load_precedes_delayed_document_write() -> Result<()> {
    child_module_load_precedes_delayed_write("import").await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_external_module_load_precedes_delayed_document_write() -> Result<()> {
    child_module_load_precedes_delayed_write("external").await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_module_load_precedes_timer_that_resumes_top_level_await() -> Result<()> {
    child_module_load_precedes_delayed_write("tla").await
}

const WRITE_ATTEMPTS: &str = r#"
  for (const method of ['write', 'writeln']) {
    let converted = false;
    const result = document[method]({toString() {
      converted = true;
      return '<p id="written">written</p>';
    }});
    writeLog.push({method, converted, undefinedResult: result === undefined,
      original: !!document.getElementById('original'),
      written: !!document.getElementById('written'),
      currentScriptNull: document.currentScript === null});
  }
  document.close();
"#;

async fn module_document_write_matrix(child: bool) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let imported = serde_json::to_string(&module_url(WRITE_ATTEMPTS))?;
    let variants = [
        ("sync", WRITE_ATTEMPTS.to_owned()),
        (
            "microtask",
            format!("Promise.resolve().then(() => {{ {WRITE_ATTEMPTS} }});"),
        ),
        (
            "tla-immediate",
            format!("await Promise.resolve(); {WRITE_ATTEMPTS}"),
        ),
        ("static-import", format!("import {imported};")),
        ("external", WRITE_ATTEMPTS.to_owned()),
        (
            "throw",
            format!("{WRITE_ATTEMPTS} throw new Error('module write probe');"),
        ),
    ];
    let mut results = Vec::new();
    for (name, source) in variants {
        let script = if name == "external" {
            format!(
                "<script type=module src=\"{}\"></script>",
                module_url(&source)
            )
        } else {
            format!("<script type=module>{source}</script>")
        };
        let markup = format!(
            "<!doctype html><body><p id=original>original</p><script>window.writeLog = [];</script>{script}"
        );
        let source_url = markup_url(&server, &markup);
        let url = if child {
            markup_url(
                &server,
                &format!(
                    "<!doctype html><iframe id=target src=\"{}\"></iframe>",
                    source_url.replace('&', "&amp;")
                ),
            )
        } else {
            source_url
        };
        let mut page = browser.fetch(&url).await?;
        let observed = page
            .evaluate_runtime_expression_with_await_async(
                r#"(() => {
              const target = document.getElementById('target')?.contentWindow || window;
              const duringModule = target.writeLog;
              // The initial module evaluation, its cleanup checkpoint and any
              // exception handling have ended. This later write must work.
              target.document.write('<p id="late">late</p>');
              target.document.close();
              return JSON.stringify({duringModule,
                laterWrite: target.document.getElementById('late')?.textContent});
            })()"#,
                true,
            )
            .await?;
        let observed: serde_json::Value =
            serde_json::from_str(observed["value"].as_str().expect("module write probe"))?;
        results.push(serde_json::json!({"name": name, "observed": observed}));
    }
    server.shutdown().await;
    let expected_log = ["write", "writeln"].map(|method| {
        serde_json::json!({"method": method, "converted": true, "undefinedResult": true,
            "original": true, "written": false, "currentScriptNull": true})
    });
    let expected: Vec<_> = [
        "sync",
        "microtask",
        "tla-immediate",
        "static-import",
        "external",
        "throw",
    ]
    .map(|name| {
        serde_json::json!({"name": name, "observed": {
            "duringModule": expected_log, "laterWrite": "late"
        }})
    })
    .into();
    assert_eq!(results, expected, "child={child}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn root_module_scripts_ignore_destructive_writes_through_cleanup() -> Result<()> {
    module_document_write_matrix(false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn child_module_scripts_ignore_destructive_writes_through_cleanup() -> Result<()> {
    module_document_write_matrix(true).await
}

#[tokio::test(flavor = "multi_thread")]
async fn modules_can_explicitly_open_a_document_stream() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    for child in [false, true] {
        let source_url = markup_url(
            &server,
            &format!(
                "<!doctype html><body><p id=original>original</p><script>window.writeLog=[];</script><script type=module>document.open();{WRITE_ATTEMPTS}</script>"
            ),
        );
        let url = if child {
            markup_url(
                &server,
                &format!(
                    "<!doctype html><iframe id=target src=\"{}\"></iframe>",
                    source_url.replace('&', "&amp;")
                ),
            )
        } else {
            source_url
        };
        let mut page = browser.fetch(&url).await?;
        let observed = page
            .evaluate_runtime_expression_with_await_async(
                r#"(() => {
              const target = document.getElementById('target')?.contentWindow || window;
              return JSON.stringify({log: target.writeLog,
                body: target.document.body.textContent});
            })()"#,
                true,
            )
            .await?;
        let observed: serde_json::Value =
            serde_json::from_str(observed["value"].as_str().expect("explicit stream"))?;
        let expected_log = ["write", "writeln"].map(|method| {
            serde_json::json!({"method": method, "converted": true, "undefinedResult": true,
                "original": false, "written": true, "currentScriptNull": true})
        });
        assert_eq!(
            observed,
            serde_json::json!({"log": expected_log,
            "body": "writtenwritten\n"}),
            "child={child}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_write_guard_is_released_before_pending_tla_continues() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    for child in [false, true] {
        let source_url = markup_url(
            &server,
            r#"<!doctype html><body><p id=original>original</p>
          <script type=module>
            window.moduleFinished = new Promise(resolve => window.finishModule = resolve);
            await new Promise(resolve => window.resumeModule = resolve);
            document.write('<p id="later">later</p>');
            document.close();
            finishModule();
          </script>"#,
        );
        let url = if child {
            markup_url(
                &server,
                &format!(
                    "<!doctype html><iframe id=target src=\"{}\"></iframe>",
                    source_url.replace('&', "&amp;")
                ),
            )
        } else {
            source_url
        };
        let mut page = browser
            .fetch_with_wait_until(&url, RenderedDomWaitUntil::Load, Duration::from_secs(5))
            .await?;
        let observed = page
            .evaluate_runtime_expression_with_await_async(
                r#"(async () => {
              const target = document.getElementById('target')?.contentWindow || window;
              const original = !!target.document.getElementById('original');
              const finished = target.moduleFinished;
              target.resumeModule();
              await finished;
              return JSON.stringify({original,
                later: target.document.getElementById('later')?.textContent});
            })()"#,
                true,
            )
            .await?;
        let observed: serde_json::Value =
            serde_json::from_str(observed["value"].as_str().expect("pending TLA write"))?;
        assert_eq!(
            observed,
            serde_json::json!({"original": true, "later": "later"}),
            "child={child}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn module_write_guard_follows_the_document_across_callback_realms() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let child_url = markup_url(
        &server,
        r#"<!doctype html><body><p id=original>original</p>
      <script type=module>parent.tryWrites(document);</script>"#,
    );
    let parent = format!(
        r#"<!doctype html><body><iframe id=other></iframe>
      <script>
        window.tryWrites = protectedDocument => {{
          protectedDocument.write('<p id="blocked">blocked</p>');
          protectedDocument.close();
          const other = document.getElementById('other').contentDocument;
          other.write('<p id="allowed">allowed</p>');
          other.close();
          window.writeResult = {{protected: !!protectedDocument.getElementById('original'),
            blocked: !!protectedDocument.getElementById('blocked'),
            other: other.getElementById('allowed')?.textContent}};
        }};
      </script><iframe src="{}"></iframe>"#,
        child_url.replace('&', "&amp;")
    );
    let mut page = browser.fetch(&markup_url(&server, &parent)).await?;
    let observed = page
        .evaluate_runtime_expression_with_await_async("JSON.stringify(writeResult)", true)
        .await?;
    server.shutdown().await;
    let observed: serde_json::Value =
        serde_json::from_str(observed["value"].as_str().expect("cross-realm writes"))?;
    assert_eq!(
        observed,
        serde_json::json!({"protected": true, "blocked": false, "other": "allowed"})
    );
    Ok(())
}
