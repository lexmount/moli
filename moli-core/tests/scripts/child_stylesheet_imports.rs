use super::*;
use anyhow::Context;
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Notify,
    task::{JoinHandle, JoinSet},
};

#[derive(Default)]
struct Asset {
    count: AtomicUsize,
    started: Notify,
    released: AtomicBool,
    release: Notify,
}

impl Asset {
    async fn wait(&self, release: bool) {
        loop {
            let notified = if release {
                self.release.notified()
            } else {
                self.started.notified()
            };
            tokio::pin!(notified);
            notified.as_mut().enable();
            if if release {
                self.released.load(Ordering::Acquire)
            } else {
                self.count.load(Ordering::Acquire) != 0
            } {
                return;
            }
            notified.await;
        }
    }
}

struct ChildImportServer {
    origin: String,
    task: JoinHandle<()>,
    assets: Arc<Mutex<HashMap<String, Arc<Asset>>>>,
}

impl Drop for ChildImportServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl ChildImportServer {
    async fn spawn() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let cross_origin = format!("http://localhost:{}", listener.local_addr()?.port());
        let assets = Arc::new(Mutex::new(HashMap::<String, Arc<Asset>>::new()));
        let server_assets = assets.clone();
        let task = tokio::spawn(async move {
            let assets = server_assets;
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((mut stream, _)) = accepted else { break };
                        let assets = assets.clone();
                        let cross_origin = cross_origin.clone();
                        connections.spawn(async move {
                            let mut request = Vec::new();
                            while !request.ends_with(b"\r\n\r\n") && request.len() < 16384 {
                                let Ok(byte) = stream.read_u8().await else { return };
                                request.push(byte);
                            }
                            let request = String::from_utf8_lossy(&request);
                            let path = request.split_whitespace().nth(1).unwrap_or("/");
                            let url = url::Url::parse(&format!("http://test{path}")).unwrap();
                            let query: HashMap<_,_> = url.query_pairs().into_owned().collect();
                            let token = query.get("token").cloned().unwrap_or_default();
                            let asset_token = if url.path().starts_with("/child-import-") && url.path().ends_with(".css") {
                                format!("{token}:{}", url.path())
                            } else { token.clone() };
                            let asset = assets.lock().entry(asset_token).or_default().clone();
                            let mut extra_headers = String::new();
                            let mut status = "200 OK";
                            if url.path().ends_with(".css") {
                                asset.count.fetch_add(1, Ordering::AcqRel);
                                asset.started.notify_waiters();
                            }
                            let (mime, body) = match url.path() {
                                "/child-import-entry.css" => {
                                    status = "302 Found";
                                    extra_headers = format!("Location: /child-import-final.css?token={token}\r\n");
                                    ("text/css", String::new())
                                }
                                "/child-import-final.css" => ("text/css", format!("@import url('/child-import-entry.css?token={token}#cycle'); :root {{ --redirect-cycle: ready; }}")),
                                "/child-import-csp-page" => {
                                    let mode = query.get("mode").map(String::as_str).unwrap_or("enforce");
                                    let kind = query.get("kind").map(String::as_str).unwrap_or("linked");
                                    let policy = "style-src 'self' 'nonce-allowed'";
                                    let meta = if mode == "meta" { format!("<meta http-equiv='Content-Security-Policy' content=\"{policy}\">") } else {
                                        extra_headers = format!("Content-Security-Policy{}: {policy}\r\n", if mode == "report" { "-Report-Only" } else { "" });
                                        String::new()
                                    };
                                    let root = format!("/child-import-csp-root.css?token={token}");
                                    let style = if kind == "inline" { format!("<style nonce='allowed'>@import url('{root}');</style>") } else { format!("<link rel='stylesheet' href='{root}'>") };
                                    ("text/html", format!(r#"<!doctype html>{meta}<script>
                                      globalThis.styleViolations = [];
                                      document.addEventListener('securitypolicyviolation', event => styleViolations.push({{
                                        target: event.target === document, directive: event.effectiveDirective,
                                        disposition: event.disposition, documentURI: event.documentURI, blockedURI: event.blockedURI
                                      }}));
                                    </script>{style}<script>globalThis.afterStylesheet = true;
                                      document.addEventListener('DOMContentLoaded', () => globalThis.stylesheetDcl = true);
                                    </script><body>CSP imports"#))
                                }
                                "/child-import-csp-root.css" => {
                                    if token.starts_with("late-") { asset.wait(true).await; }
                                    ("text/css", format!("@import url('http://127.0.0.1:{}/child-import-csp-branch.css?token={token}'); @import url('data:text/css,:root{{--csp-data:applied}}'); :root {{ --csp-root: ready; }}", cross_origin.rsplit(':').next().unwrap()))
                                }
                                "/child-import-late-page" => {
                                    let mode = query.get("mode").map(String::as_str).unwrap_or("dynamic");
                                    let cross = query.get("cross").is_some_and(|value| value == "true");
                                    let meta = "<meta id='late-policy' http-equiv='Content-Security-Policy' content=\"style-src 'self'\">";
                                    let root = format!("{}/child-import-csp-root.css?token={token}", if cross { &cross_origin } else { "" });
                                    let stylesheet = if query.get("kind").is_some_and(|kind| kind == "inline") {
                                        format!("<style>@import url('{root}');</style>")
                                    } else { format!("<link rel='stylesheet' href='{root}'>") };
                                    ("text/html", format!(r#"<!doctype html><head><script>
                                      globalThis.lateViolations = [];
                                      document.addEventListener('securitypolicyviolation', event => lateViolations.push({{
                                        target:event.target === document, directive:event.effectiveDirective, disposition:event.disposition
                                      }}));
                                    </script>{}{stylesheet}{}</head><body><script>globalThis.afterLateStylesheet = true;</script>Late CSP"#, if mode == "before" { meta } else { "" }, if mode == "parser" { meta } else { "" }))
                                }
                                "/child-import-nested-redirect.css" => ("text/css", format!("@import url('/child-import-alias.css?token={token}');")),
                                "/child-import-alias.css" => {
                                    status = "302 Found";
                                    extra_headers = format!("Location: /child-import-real.css?token={token}\r\n");
                                    ("text/css", String::new())
                                }
                                "/child-import-real.css" => ("text/css", format!("@import url('/child-import-real.css?token={token}#cycle'); :root {{ --nested-redirect: ready; }}")),
                                "/child-import-csp-branch.css" => ("text/css", format!("@import url('{}{}?token={token}'); @import url('/child-import-csp-allowed.css?token={token}');", if token.contains("redirect") { "" } else { &cross_origin }, if token.contains("redirect") { "/child-import-csp-hop.css" } else { "/child-import-csp-blocked.css" })),
                                "/child-import-csp-hop.css" => {
                                    status = "302 Found";
                                    extra_headers = format!("Location: {cross_origin}/child-import-csp-blocked.css?token={token}\r\n");
                                    ("text/css", String::new())
                                }
                                "/child-import-csp-blocked.css" => ("text/css", format!("@import url('/child-import-csp-descendant.css?token={token}'); :root {{ --csp-blocked: applied; }}")),
                                "/child-import-csp-allowed.css" => ("text/css", ":root { --csp-allowed: ready; }".to_owned()),
                                "/child-import-csp-descendant.css" => ("text/css", ":root { --csp-descendant: applied; }".to_owned()),
                                "/child-import-inline-policy" => {
                                    let mode = query.get("mode").map(String::as_str).unwrap_or("header");
                                    let policy = "style-src 'self' 'nonce-allowed'";
                                    let meta = if mode == "meta" { format!("<meta http-equiv='Content-Security-Policy' content=\"{policy}\">") } else {
                                        extra_headers = format!("Content-Security-Policy{}: {policy}\r\n", if mode == "report" { "-Report-Only" } else { "" });
                                        String::new()
                                    };
                                    let nonce = match query.get("nonce").map(String::as_str) {
                                        Some("allowed") => " nonce='allowed'",
                                        Some("wrong") => " nonce='wrong'",
                                        _ => "",
                                    };
                                    let import = if query.get("imports").is_some_and(|value| value == "true") {
                                        format!("@import url('/child-import-inline-allowed.css?token={token}');")
                                    } else { String::new() };
                                    ("text/html", format!(r#"<!doctype html><head>{meta}<script>
                                      globalThis.inlineViolations = [];
                                      document.addEventListener('securitypolicyviolation', event => inlineViolations.push({{
                                        directive:event.effectiveDirective, disposition:event.disposition,
                                        documentURI:event.documentURI, blockedURI:event.blockedURI
                                      }}));
                                    </script><style id='policy-style'{nonce}>{import}:root {{ --inline-policy: applied; }}</style>
                                    <script>globalThis.afterInlineStylesheet = true;</script></head><body>Inline policy"#))
                                }
                                "/child-import-inline-allowed.css" => ("text/css", ":root { --inline-import: applied; }".to_owned()),
                                "/child-import-parent" => {
                                    let kind = query.get("kind").map(String::as_str).unwrap_or("linked");
                                    let scenario = query.get("scenario").map(String::as_str).unwrap_or("nested");
                                    ("text/html", format!("<!doctype html><iframe src='/child-import-page?kind={kind}&amp;token={token}&amp;scenario={scenario}'></iframe>"))
                                }
                                "/child-import-page" => {
                                    let kind = query.get("kind").map(String::as_str).unwrap_or("linked");
                                    let scenario = query.get("scenario").map(String::as_str).unwrap_or("nested");
                                    let root = format!("/child-import-root.css?token={token}&scenario={scenario}");
                                    let style = if kind == "inline" { format!("<style>@import url('{root}');</style>") }
                                        else { format!("<link rel='stylesheet' href='{root}'>") };
                                    let probe = r#"try { const rootSheet = document.styleSheets[0];
                                        const root = rootSheet.href ? rootSheet : rootSheet.cssRules[0].styleSheet;
                                        const imports = root ? Array.from(root.cssRules).filter(rule => rule.type === 3) : [];
                                        globalThis.parserSheetSnapshot = {root: !!root, branches: imports.length,
                                            children: imports.map(rule => {
                                                const branch = rule.styleSheet;
                                                const leafRule = branch && branch.cssRules[0];
                                                const leaf = leafRule && leafRule.styleSheet;
                                                return {branch: !!branch, leaf: !!leaf,
                                                    owner: !!branch && branch.ownerRule === rule && branch.parentStyleSheet === root,
                                                    leafOwner: !!leaf && leaf.ownerRule === leafRule && leaf.parentStyleSheet === branch,
                                                    text: leaf ? Array.from(leaf.cssRules).find(rule => rule.type === 1)?.cssText ?? null : null};
                                            })}; } catch (error) { globalThis.parserSheetSnapshot = {error: String(error)}; } parent.postMessage({childImportProbe: true, href: location.href}, '*');"#;
                                    ("text/html", format!("<!doctype html>{style}<script>{probe}</script><body>Child imports"))
                                }
                                "/child-import-root.css" | "/child-import-branch.css" | "/child-import-leaf.css" => {
                                    let scenario = query.get("scenario").map(String::as_str).unwrap_or("nested");
                                    if scenario == "delayed" && url.path() == "/child-import-branch.css" { asset.wait(true).await; }
                                    let body = match url.path() {
                                        "/child-import-root.css" => format!("@import url('/child-import-branch.css?token={token}&scenario={scenario}'); @import url('/child-import-branch.css?token={token}&scenario={scenario}'); .root {{ color: rgb(1, 2, 3); }}"),
                                        "/child-import-branch.css" => format!("@import url('/child-import-leaf.css?token={token}&scenario={scenario}'); .branch {{ color: rgb(4, 5, 6); }}"),
                                        _ if scenario == "cycle" => format!("@import url('/child-import-root.css?token={token}&scenario={scenario}'); .leaf {{ color: rgb(7, 8, 9); }}"),
                                        _ => ".leaf { color: rgb(7, 8, 9); }".to_owned(),
                                    };
                                    ("text/css", body)
                                }

                                "/probe-started" | "/probe-release" | "/probe-stats" => {
                                    if url.path() == "/probe-started" { asset.wait(false).await; }
                                    if url.path() == "/probe-release" {
                                        asset.released.store(true, Ordering::Release);
                                        asset.release.notify_waiters();
                                    }
                                    ("application/json", serde_json::json!({"count":asset.count.load(Ordering::Acquire)}).to_string())
                                }
                                _ => ("text/html", "<!doctype html><body>preload consumption".to_owned()),
                            };
                            if url.path() == "/child-import-leaf.css"
                                && query.get("scenario").is_some_and(|value| value == "invalid") {
                                status = "404 Not Found";
                            }
                            let response = format!("HTTP/1.1 {status}\r\n{extra_headers}Content-Type: {mime}\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                            let _ = stream.write_all(response.as_bytes()).await;
                            let _ = stream.write_all(body.as_bytes()).await;
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Ok(Self {
            origin,
            task,
            assets,
        })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_parser_stylesheet_imports_finish_before_following_scripts() -> Result<()> {
    let server = ChildImportServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut failures = Vec::new();
    for kind in ["linked", "inline"] {
        for scenario in ["nested", "cycle", "invalid"] {
            let token = format!("parser-{kind}-{scenario}");
            let mut page = tokio::time::timeout(
                Duration::from_secs(30),
                browser.fetch(&format!(
                    "{}/child-import-parent?kind={kind}&token={token}&scenario={scenario}",
                    server.origin
                )),
            )
            .await??;
            let result = page.evaluate_runtime_expression_with_await_async(
            "JSON.stringify(document.querySelector('iframe').contentWindow.parserSheetSnapshot ?? {error: 'parser script did not publish a snapshot'})", true).await?;
            let value: serde_json::Value = serde_json::from_str(
                result["value"]
                    .as_str()
                    .context("child parser import snapshot")?,
            )?;
            let requests = ["root", "branch", "leaf"].map(|name| {
                server
                    .assets
                    .lock()
                    .get(&format!("{token}:/child-import-{name}.css"))
                    .map(|asset| asset.count.load(Ordering::Acquire))
                    .unwrap_or(0)
            });
            let children = value["children"].as_array().cloned().unwrap_or_default();
            let expected_text = if scenario == "invalid" {
                serde_json::Value::Null
            } else {
                serde_json::json!(".leaf { color: rgb(7, 8, 9); }")
            };
            if requests != [1, 1, 1]
                || value["root"] != true
                || value["branches"] != 2
                || children.len() != 2
                || children.iter().any(|row| {
                    row["branch"] != true
                        || row["leaf"] != true
                        || row["owner"] != true
                        || row["leafOwner"] != true
                        || row["text"] != expected_text
                })
            {
                failures.push(serde_json::json!({"kind":kind,"scenario":scenario,"requests":requests,"snapshot":value}));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "parser imports must be fetched once and installed before child scripts: {}",
        serde_json::to_string(&failures)?
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_parser_imports_hold_scripts_and_frame_load_until_the_graph_finishes() -> Result<()> {
    let server = ChildImportServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let probe = include_str!("../fixtures/child-stylesheet-imports-pending.js");
    for kind in ["linked", "inline"] {
        let mut page = browser.fetch(&server.origin).await?;
        let value = tokio::time::timeout(
            Duration::from_secs(30),
            page.evaluate_runtime_expression_with_await_async(
                &format!("({probe})({kind:?}).then(JSON.stringify)"),
                true,
            ),
        )
        .await??;
        let value: serde_json::Value =
            serde_json::from_str(value["value"].as_str().context("pending import result")?)?;
        assert_eq!(
            value["before"],
            serde_json::json!({"loaded":false,"parserMessages":0}),
            "{kind}: {value}"
        );
        assert_eq!(value["loaded"], true, "{kind}: {value}");
        assert_eq!(value["parserMessages"], 1, "{kind}: {value}");
        assert_eq!(value["snapshot"]["branches"], 2, "{kind}: {value}");
        assert!(
            value["snapshot"]["children"]
                .as_array()
                .context("loaded imports")?
                .iter()
                .all(|row| row["leaf"] == true && row["owner"] == true && row["leafOwner"] == true),
            "{kind}: {value}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_linked_import_cycle_keeps_both_root_url_identities() -> Result<()> {
    let server = ChildImportServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.origin).await?;
    let value = tokio::time::timeout(Duration::from_secs(30), page.evaluate_runtime_expression_with_await_async(r#"
      (async () => {
        const frame = document.createElement('iframe');
        const loaded = new Promise(resolve => frame.onload = resolve);
        frame.srcdoc = '<!doctype html><link rel="stylesheet" href="/child-import-entry.css?token=redirect-cycle"><script>globalThis.afterStylesheet = true;</script><body>Redirect cycle';
        document.body.append(frame);
        await loaded;
        const child = frame.contentWindow;
        const root = frame.contentDocument.styleSheets[0];
        const imported = root.cssRules[0].styleSheet;
        return JSON.stringify({href:root.href, rules:root.cssRules.length,
          importedRules:imported ? imported.cssRules.length : 0,
          style:child.getComputedStyle(frame.contentDocument.documentElement).getPropertyValue('--redirect-cycle').trim(),
          script:child.afterStylesheet === true, ready:frame.contentDocument.readyState});
      })()
    "#, true)).await??;
    let value: serde_json::Value =
        serde_json::from_str(value["value"].as_str().context("redirect cycle snapshot")?)?;
    assert_eq!(
        value,
        serde_json::json!({"href":format!("{}/child-import-entry.css?token=redirect-cycle", server.origin),"rules":2,"importedRules":0,"style":"ready","script":true,"ready":"complete"})
    );
    for name in ["entry", "final"] {
        assert_eq!(
            server
                .assets
                .lock()
                .get(&format!("redirect-cycle:/child-import-{name}.css"))
                .map(|asset| asset.count.load(Ordering::Acquire)),
            Some(1),
            "{name}: cycle must not refetch either root identity"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_parser_imports_enforce_the_child_csp_before_fetch() -> Result<()> {
    let server = ChildImportServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.origin).await?;
    let expression = r#"(async () => {
      const parentViolations = [];
      document.addEventListener('securitypolicyviolation', event => parentViolations.push(event.blockedURI));
      const meta = document.createElement('meta');
      meta.httpEquiv = 'Content-Security-Policy'; meta.content = "style-src 'none'";
      document.head.append(meta);
      const rows = [];
      for (const kind of ['linked', 'inline']) {
        for (const mode of ['enforce', 'meta', 'report']) {
          for (const hop of ['direct', 'redirect']) {
            const frame = document.createElement('iframe');
            const loaded = new Promise(resolve => frame.onload = resolve);
            const token = kind + '-' + mode + '-' + hop;
            frame.src = '/child-import-csp-page?kind=' + kind + '&mode=' + mode + '&token=' + token;
            document.body.append(frame);
            await loaded;
            const child = frame.contentWindow;
            const style = child.getComputedStyle(frame.contentDocument.documentElement);
            rows.push({token, kind, mode, hop, script:child.afterStylesheet === true,
              dcl:child.stylesheetDcl === true, ready:frame.contentDocument.readyState,
              root:style.getPropertyValue('--csp-root').trim(), allowed:style.getPropertyValue('--csp-allowed').trim(),
              blocked:style.getPropertyValue('--csp-blocked').trim(), data:style.getPropertyValue('--csp-data').trim(),
              descendant:style.getPropertyValue('--csp-descendant').trim(), violations:child.styleViolations});
            frame.remove();
          }
        }
      }
      return JSON.stringify({rows, parentViolations});
    })()"#;
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        page.evaluate_runtime_expression_with_await_async(expression, true),
    )
    .await??;
    let result: serde_json::Value =
        serde_json::from_str(result["value"].as_str().context("child CSP results")?)?;
    assert_eq!(
        result["parentViolations"],
        serde_json::json!([]),
        "child violations must stay in the child Document"
    );
    let rows = result["rows"].as_array().context("CSP rows")?;
    assert_eq!(rows.len(), 12);
    for row in rows {
        let token = row["token"].as_str().unwrap();
        let reporting = row["mode"] == "report";
        for field in ["script", "dcl"] {
            assert_eq!(row[field], true, "{row}");
        }
        assert_eq!(row["ready"], "complete", "{row}");
        for field in ["root", "allowed"] {
            assert_eq!(row[field], "ready", "{row}");
        }
        for field in ["blocked", "data", "descendant"] {
            assert_eq!(row[field], if reporting { "applied" } else { "" }, "{row}");
        }
        let violations = row["violations"].as_array().context("child violations")?;
        // A cross-origin import, a data: import, and (only when reporting)
        // the cross-origin descendant are checked against the child policy.
        assert_eq!(violations.len(), if reporting { 3 } else { 2 }, "{row}");
        for violation in violations {
            assert_eq!(violation["target"], true, "{row}");
            assert_eq!(violation["directive"], "style-src-elem", "{row}");
            assert_eq!(
                violation["disposition"],
                if reporting { "report" } else { "enforce" },
                "{row}"
            );
            assert!(
                violation["documentURI"]
                    .as_str()
                    .unwrap()
                    .contains("/child-import-csp-page?"),
                "{row}"
            );
        }
        for (name, expected) in [
            ("root", 1),
            ("branch", 1),
            ("allowed", 1),
            ("blocked", usize::from(reporting)),
            ("descendant", usize::from(reporting)),
            ("hop", usize::from(row["hop"] == "redirect")),
        ] {
            let count = server
                .assets
                .lock()
                .get(&format!("{token}:/child-import-csp-{name}.css"))
                .map_or(0, |asset| asset.count.load(Ordering::Acquire));
            assert_eq!(count, expected, "{name}: {row}");
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_srcdoc_imports_use_the_inherited_origin_for_csp_self() -> Result<()> {
    let server = ChildImportServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.origin).await?;
    let result = tokio::time::timeout(Duration::from_secs(30), page.evaluate_runtime_expression_with_await_async(r#"
      (async () => {
        const frame = document.createElement('iframe');
        const loaded = new Promise(resolve => frame.onload = resolve);
        const rootUrl = new URL('/child-import-csp-root.css?token=srcdoc-csp', location.href).href;
        frame.srcdoc = `<!doctype html><meta http-equiv="Content-Security-Policy" content="style-src 'self'">
          <base href="https://unrelated-base.invalid/">
          <link rel="stylesheet" href="${rootUrl}">
          <script>globalThis.afterStylesheet = true;</script><body>Inherited CSP origin`;
        document.body.append(frame);
        await loaded;
        const style = frame.contentWindow.getComputedStyle(frame.contentDocument.documentElement);
        return JSON.stringify({root:style.getPropertyValue('--csp-root').trim(),
          allowed:style.getPropertyValue('--csp-allowed').trim(),
          blocked:style.getPropertyValue('--csp-blocked').trim(),
          data:style.getPropertyValue('--csp-data').trim(),
          script:frame.contentWindow.afterStylesheet === true, ready:frame.contentDocument.readyState});
      })()
    "#, true)).await??;
    let result: serde_json::Value =
        serde_json::from_str(result["value"].as_str().context("srcdoc CSP result")?)?;
    assert_eq!(
        result,
        serde_json::json!({"root":"ready", "allowed":"ready", "blocked":"", "data":"", "script":true, "ready":"complete"})
    );
    assert!(
        !server
            .assets
            .lock()
            .contains_key("srcdoc-csp:/child-import-csp-blocked.css")
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_inline_style_csp_blocks_the_owner_before_import_admission() -> Result<()> {
    let server = ChildImportServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.origin).await?;
    let probe = include_str!("../fixtures/child-stylesheet-inline-csp.js");
    let expression = format!("({probe})().then(JSON.stringify)");
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        page.evaluate_runtime_expression_with_await_async(&expression, true),
    )
    .await??;
    let result: serde_json::Value =
        serde_json::from_str(result["value"].as_str().context("inline CSP result")?)?;
    assert_eq!(
        result["parentViolations"],
        serde_json::json!([]),
        "{result}"
    );
    let rows = result["rows"].as_array().context("inline CSP cases")?;
    assert_eq!(rows.len(), 8);
    for row in rows {
        let allowed = row["nonce"] == "allowed" || row["mode"] == "report";
        let imports = row["imports"] == true;
        assert_eq!(row["local"], if allowed { "applied" } else { "" }, "{row}");
        assert_eq!(
            row["imported"],
            if allowed && imports { "applied" } else { "" },
            "{row}"
        );
        assert_eq!(row["sheet"], allowed, "{row}");
        assert_eq!(row["sheetCount"], usize::from(allowed), "{row}");
        assert_eq!(row["script"], true, "{row}");
        assert_eq!(row["ready"], "complete", "{row}");
        let violations = row["violations"].as_array().context("inline violations")?;
        assert_eq!(
            violations.len(),
            usize::from(row["nonce"] != "allowed"),
            "{row}"
        );
        for violation in violations {
            assert_eq!(violation["blockedURI"], "inline", "{row}");
            assert_eq!(violation["directive"], "style-src-elem", "{row}");
            assert_eq!(
                violation["disposition"],
                if row["mode"] == "report" {
                    "report"
                } else {
                    "enforce"
                },
                "{row}"
            );
            assert!(
                violation["documentURI"]
                    .as_str()
                    .unwrap()
                    .contains("/child-import-inline-policy?"),
                "{row}"
            );
        }
        let token = row["token"].as_str().context("inline token")?;
        let count = server
            .assets
            .lock()
            .get(&format!("{token}:/child-import-inline-allowed.css"))
            .map_or(0, |asset| asset.count.load(Ordering::Acquire));
        assert_eq!(count, usize::from(allowed && imports), "{row}");
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_imports_observe_late_meta_without_rechecking_admitted_roots() -> Result<()> {
    let server = ChildImportServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.origin).await?;
    let probe = include_str!("../fixtures/child-stylesheet-late-csp.js");
    for (kind, mode, cross) in [
        ("linked", "dynamic", false),
        ("linked", "dynamic", true),
        ("linked", "parser", false),
        ("linked", "parser", true),
        ("linked", "before", true),
        ("linked", "before", false),
        ("inline", "dynamic", true),
        ("inline", "parser", true),
    ] {
        let token = format!("late-{kind}-{mode}-{cross}");
        let expression =
            format!("({probe})({mode:?}, {cross}, {token:?}, {kind:?}).then(JSON.stringify)");
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            page.evaluate_runtime_expression_with_await_async(&expression, true),
        )
        .await
        .with_context(|| format!("{kind}, {mode}, cross={cross}: stylesheet did not settle"))??;
        let result: serde_json::Value =
            serde_json::from_str(result["value"].as_str().context("late CSP snapshot")?)?;
        let root_allowed = mode != "before" || !cross;
        assert_eq!(
            result["root"],
            if root_allowed { "ready" } else { "" },
            "{mode}, cross={cross}: {result}"
        );
        assert_eq!(
            result["allowed"],
            if root_allowed { "ready" } else { "" },
            "{mode}, cross={cross}: {result}"
        );
        for field in ["blocked", "data"] {
            assert_eq!(result[field], "", "{mode}, cross={cross}: {result}");
        }
        assert_eq!(result["script"], true, "{mode}, cross={cross}: {result}");
        assert_eq!(
            result["ready"], "complete",
            "{mode}, cross={cross}: {result}"
        );
        let violations = result["violations"]
            .as_array()
            .context("late CSP violations")?;
        assert_eq!(
            violations.len(),
            if root_allowed { 2 } else { 1 },
            "{mode}, cross={cross}: {result}"
        );
        assert!(
            violations.iter().all(|event| event["target"] == true
                && event["directive"] == "style-src-elem"
                && event["disposition"] == "enforce"),
            "{result}"
        );
        let requests = server.assets.lock();
        for (name, expected) in [
            ("root", usize::from(root_allowed)),
            ("branch", usize::from(root_allowed)),
            ("allowed", usize::from(root_allowed)),
            ("blocked", 0),
        ] {
            let count = requests
                .get(&format!("{token}:/child-import-csp-{name}.css"))
                .map_or(0, |asset| asset.count.load(Ordering::Acquire));
            assert_eq!(count, expected, "{name}: {mode}, cross={cross}: {result}");
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_imports_register_each_redirect_response_before_expanding_imports() -> Result<()> {
    let server = ChildImportServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.origin).await?;
    let result = tokio::time::timeout(Duration::from_secs(30), page.evaluate_runtime_expression_with_await_async(r#"
      (async () => {
        const frame = document.createElement('iframe');
        const loaded = new Promise(resolve => frame.onload = resolve);
        frame.srcdoc = '<!doctype html><link rel="stylesheet" href="/child-import-nested-redirect.css?token=nested-redirect"><script>globalThis.afterStylesheet=true;</script><body>Nested redirect';
        document.body.append(frame);await loaded;
        const child = frame.contentWindow;
        const root = frame.contentDocument.styleSheets[0];
        const imported = root.cssRules[0].styleSheet;
        const cycle = imported.cssRules[0].styleSheet;
        return JSON.stringify({style:child.getComputedStyle(frame.contentDocument.documentElement).getPropertyValue('--nested-redirect').trim(),
          cycleRules:cycle ? cycle.cssRules.length : 0, script:child.afterStylesheet === true});
      })()
    "#, true)).await??;
    let result: serde_json::Value = serde_json::from_str(
        result["value"]
            .as_str()
            .context("nested redirect snapshot")?,
    )?;
    assert_eq!(
        result,
        serde_json::json!({"style":"ready", "cycleRules":0,"script":true})
    );
    for name in ["nested-redirect", "alias", "real"] {
        let count = server
            .assets
            .lock()
            .get(&format!("nested-redirect:/child-import-{name}.css"))
            .map_or(0, |asset| asset.count.load(Ordering::Acquire));
        assert_eq!(
            count, 1,
            "{name}: a redirected import must not fetch itself again"
        );
    }
    Ok(())
}
