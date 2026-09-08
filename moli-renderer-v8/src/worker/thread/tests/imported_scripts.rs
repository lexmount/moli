use super::*;

#[tokio::test]
async fn worker_importscripts_trusted_types_reports_keep_script_and_document_locations_separate() {
    ensure_v8();
    let source = "self.violatePolicy = () => {\n  try { trustedTypes.createPolicy('forbidden'); } catch {}\n};\nself.violateSink = () => {\n  try { setTimeout('blocked code'); } catch {}\n};\nviolatePolicy();\nviolateSink();";
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/imported/violations.js",
        "HTTP/1.1 200 OK",
        "text/javascript",
        source.into(),
        Duration::ZERO,
    )])
    .await;
    let worker_url = format!("{base_url}/worker/main.js");
    let options = WorkerSpawnOptions::new(
        r#"
        const violations = [];
        addEventListener('securitypolicyviolation', event => violations.push([
            event.effectiveDirective, event.documentURI, event.sourceFile,
            event.lineNumber, event.columnNumber > 0
        ]));
        const setup = trustedTypes.createPolicy('bootstrap', { createScriptURL: s => s });
        importScripts(setup.createScriptURL('../imported/violations.js'));
        setTimeout(() => {
            violatePolicy(); violateSink();
            postMessage(violations); close();
        }, 0);
        "#
        .into(),
        worker_url.clone(),
    )
    .with_content_security_policies(vec![
        "require-trusted-types-for 'script'; trusted-types bootstrap".into(),
    ]);
    let mut handle = spawn_test_worker_with_options(options);
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let actual: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    let script_url = format!("{base_url}/imported/violations.js");
    assert_eq!(
        actual,
        serde_json::json!([
            ["trusted-types", worker_url, script_url, 2, true],
            ["require-trusted-types-for", worker_url, script_url, 5, true],
            ["trusted-types", worker_url, script_url, 2, true],
            ["require-trusted-types-for", worker_url, script_url, 5, true],
        ])
    );
    server.await.unwrap();
}

#[tokio::test]
async fn worker_importscripts_trusted_types_reports_do_not_expose_redirect_targets() {
    ensure_v8();
    let (foreign_url, foreign_server) = spawn_path_response_http_server(vec![(
        "/private-user/violations.js?credential=hidden",
        "HTTP/1.1 200 OK",
        "text/javascript",
        "setTimeout(() => {\n  try { trustedTypes.createPolicy('forbidden'); } catch {}\n  postMessage(violations); close();\n}, 0);".into(),
        Duration::ZERO,
    )])
    .await;
    let redirect = format!(
        "HTTP/1.1 302 Found\r\nLocation: {foreign_url}/private-user/violations.js?credential=hidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let (base_url, server) = spawn_raw_path_response_http_server(vec![(
        "/worker/redirect.js",
        redirect,
        Duration::ZERO,
    )])
    .await;
    let worker_url = format!("{base_url}/worker/main.js");
    let options = WorkerSpawnOptions::new(
        r#"
        const violations = [];
        addEventListener('securitypolicyviolation', event => violations.push([
            event.disposition, event.documentURI, event.sourceFile,
            event.lineNumber, event.columnNumber > 0
        ]));
        importScripts('./redirect.js');
        "#
        .into(),
        worker_url.clone(),
    )
    .with_content_security_policies(vec!["trusted-types 'none'".into()])
    .with_content_security_report_only_policies(vec!["trusted-types 'none'".into()]);
    let mut handle = spawn_test_worker_with_options(options);
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let actual: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    let request_url = format!("{base_url}/worker/redirect.js");
    assert_eq!(
        actual,
        serde_json::json!([
            ["enforce", worker_url, request_url, 2, true],
            ["report", worker_url, request_url, 2, true],
        ])
    );
    server.await.unwrap();
    foreign_server.await.unwrap();
}

#[tokio::test]
async fn worker_importscripts_cross_origin_respects_corp_and_coep() {
    ensure_v8();
    for (require_corp, corp, expected) in [
        (false, "same-origin", r#"["NetworkError",false]"#),
        (true, "", r#"["NetworkError",false]"#),
        (true, "cross-origin", r#"["ok",true]"#),
    ] {
        let body = "self.loaded = true;";
        let corp_header = if corp.is_empty() {
            String::new()
        } else {
            format!("Cross-Origin-Resource-Policy: {corp}\r\n")
        };
        let response = format!(
            "HTTP/1.1 200 OK\r\n{corp_header}Content-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let (foreign_url, server) =
            spawn_raw_path_response_http_server(vec![("/foreign.js", response, Duration::ZERO)])
                .await;
        let foreign = serde_json::to_string(&format!("{foreign_url}/foreign.js")).unwrap();
        let policy_context = crate::types::SubresourcePolicyContext {
            cross_origin_embedder_policy: if require_corp {
                crate::cross_origin_isolation::CrossOriginEmbedderPolicy::RequireCorp
            } else {
                crate::cross_origin_isolation::CrossOriginEmbedderPolicy::None
            },
            ..Default::default()
        };
        let options = WorkerSpawnOptions::new(
            format!(
                r#"
            let outcome = 'ok';
            try {{ importScripts({foreign}); }} catch (error) {{ outcome = error.name; }}
            postMessage([outcome, self.loaded === true]); close();
            "#
            ),
            "http://127.0.0.1/worker/main.js".into(),
        )
        .with_policy_context(policy_context);
        let mut handle = spawn_test_worker_with_options(options);
        let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
        assert_eq!(
            expect_post_json(message),
            expected,
            "require_corp={require_corp}, corp={corp}"
        );
        server.await.unwrap();
    }
}

#[tokio::test]
async fn worker_importscripts_redirects_check_csp_and_ignore_redirected_paths() {
    ensure_v8();
    for (report_only, allow_foreign, expected) in [
        (false, false, r#"["NetworkError",false,["enforce"]]"#),
        (true, false, r#"["ok",true,["report"]]"#),
        (false, true, r#"["ok",true,[]]"#),
    ] {
        let (foreign_url, foreign_server) = spawn_path_response_http_server(vec![(
            "/redirect-target/foreign.js",
            "HTTP/1.1 200 OK",
            "text/javascript",
            "self.loaded = true;".into(),
            Duration::ZERO,
        )])
        .await;
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {foreign_url}/redirect-target/foreign.js\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let (worker_url, server) = spawn_raw_path_response_http_server(vec![(
            "/worker/redirect.js",
            response,
            Duration::ZERO,
        )])
        .await;
        let policy = if allow_foreign {
            format!("script-src 'self' {foreign_url}/only-before-redirect/")
        } else {
            "script-src 'self'".to_owned()
        };
        let mut options = WorkerSpawnOptions::new(
            r#"
            const violations = [];
            addEventListener('securitypolicyviolation', event => violations.push(event.disposition));
            let outcome = 'ok';
            try { importScripts('./redirect.js'); } catch (error) { outcome = error.name; }
            postMessage([outcome, self.loaded === true, violations]); close();
            "#.into(), format!("{worker_url}/worker/main.js"),
        );
        options = if report_only {
            options.with_content_security_report_only_policies(vec![policy])
        } else {
            options.with_content_security_policies(vec![policy])
        };
        let mut handle = spawn_test_worker_with_options(options);
        let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
        assert_eq!(
            expect_post_json(message),
            expected,
            "report={report_only}, allow={allow_foreign}"
        );
        server.await.unwrap();
        foreign_server.await.unwrap();
    }
}

#[tokio::test]
async fn worker_importscripts_cross_origin_does_not_bypass_module_cors() {
    ensure_v8();
    let (foreign_url, server) = spawn_path_response_http_server(vec![
        ("/foreign.js", "HTTP/1.1 200 OK", "text/javascript",
         "self.result = import(self.foreignModule).then(() => 'unexpected', error => error.name);".into(), Duration::ZERO),
        ("/foreign-module.js", "HTTP/1.1 200 OK", "text/javascript",
         "export const value = 'private module';".into(), Duration::ZERO),
    ]).await;
    let script = serde_json::to_string(&format!("{foreign_url}/foreign.js")).unwrap();
    let module = serde_json::to_string(&format!("{foreign_url}/foreign-module.js")).unwrap();
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        self.foreignModule = {module};
        try {{
            importScripts({script});
            result.then(value => {{ postMessage(value); close(); }});
        }} catch (error) {{ postMessage('importScripts:' + error.name); close(); }}
        "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        worker_test_request_client(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    assert_eq!(expect_post_json(message), r#""TypeError""#);
    server.await.unwrap();
}

#[tokio::test]
async fn worker_importscripts_cross_origin_redirect_chain_stays_muted_after_returning() {
    ensure_v8();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let worker_url = format!("http://{}", listener.local_addr().unwrap());
    let redirect = format!(
        "HTTP/1.1 302 Found\r\nLocation: {worker_url}/worker/creator.js\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let (foreign_url, foreign_server) =
        spawn_raw_path_response_http_server(vec![("/return.js", redirect, Duration::ZERO)]).await;
    let server = tokio::spawn(async move {
        let mut paths = Vec::new();
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_http_request_head(&mut stream).await.unwrap();
            let path = request
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            paths.push(path.to_owned());
            let (status, extra, body) = match path {
                "/worker/redirect.js" => (
                    "302 Found",
                    format!("Location: {foreign_url}/return.js\r\n"),
                    "",
                ),
                "/worker/creator.js" => (
                    "200 OK",
                    String::new(),
                    "self.result = import('./leaf.js').then(() => 'unexpected', error => error.name);",
                ),
                "/worker/leaf.js" => (
                    "200 OK",
                    String::new(),
                    "export const value = 'unexpected';",
                ),
                "/done" => ("200 OK", String::new(), "done"),
                _ => panic!("unexpected importScripts request {path}"),
            };
            let response = format!(
                "HTTP/1.1 {status}\r\n{extra}Content-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            if path == "/done" {
                return paths;
            }
        }
    });
    let mut handle = spawn_worker_with_request_client(
        r#"
        try {
            importScripts('./redirect.js');
            result.then(async value => { await fetch('/done'); postMessage(value); close(); });
        } catch (error) { postMessage('importScripts:' + error.name); close(); }
        "#
        .into(),
        format!("{worker_url}/worker/main.js"),
        worker_test_request_client(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    assert_eq!(expect_post_json(message), r#""TypeError""#);
    assert_eq!(
        server.await.unwrap(),
        ["/worker/redirect.js", "/worker/creator.js", "/done"]
    );
    foreign_server.await.unwrap();
}

#[tokio::test]
async fn worker_importscripts_keeps_settings_base_separate_from_dynamic_import_base() {
    ensure_v8();
    let (base_url, server) = spawn_path_response_http_server(vec![
        (
            "/imported/creator.js",
            "HTTP/1.1 200 OK",
            "text/javascript",
            "importScripts('./nested.js'); self.importLater = () => import('./leaf.js');".into(),
            Duration::ZERO,
        ),
        (
            "/worker/nested.js",
            "HTTP/1.1 200 OK",
            "text/javascript",
            "self.nested = 'worker';".into(),
            Duration::ZERO,
        ),
        (
            "/imported/leaf.js",
            "HTTP/1.1 200 OK",
            "text/javascript",
            "export const value = 'imported';".into(),
            Duration::ZERO,
        ),
    ])
    .await;
    let mut handle = spawn_worker_with_request_client(
        r#"
        try {
            importScripts('../imported/creator.js');
            importLater().then(module => {
                postMessage({nested, value: module.value}); close();
            }, error => { postMessage({error: error.name}); close(); });
        } catch (error) { postMessage({error: error.name}); close(); }
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        worker_test_request_client(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    assert_eq!(
        expect_post_json(message),
        r#"{"nested":"worker","value":"imported"}"#
    );
    server.await.unwrap();
}

#[tokio::test]
async fn worker_importscripts_cross_origin_sanitizes_import_base_but_not_settings_origin() {
    ensure_v8();
    let (worker_url, worker_server) = spawn_path_response_http_server(vec![
        (
            "/worker/nested.js",
            "HTTP/1.1 200 OK",
            "text/javascript",
            "self.nested = 'worker';".into(),
            Duration::ZERO,
        ),
        (
            "/worker/leaf.js",
            "HTTP/1.1 200 OK",
            "text/javascript",
            "export const value = 'worker-module';".into(),
            Duration::ZERO,
        ),
    ])
    .await;
    let absolute = serde_json::to_string(&format!("{worker_url}/worker/leaf.js")).unwrap();
    let source = format!(
        r#"
        importScripts('./nested.js');
        self.relativeImport = import('./leaf.js').then(() => 'unexpected', error => error.name);
        self.absoluteImport = import({absolute}).then(module => module.value);
        self.importLater = () => import('./later.js').then(() => 'unexpected', error => error.name);
    "#
    );
    let (foreign_url, foreign_server) = spawn_path_response_http_server(vec![(
        "/foreign/creator.js",
        "HTTP/1.1 200 OK",
        "text/javascript",
        source,
        Duration::ZERO,
    )])
    .await;
    let foreign = serde_json::to_string(&format!("{foreign_url}/foreign/creator.js")).unwrap();
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        try {{
            importScripts({foreign});
            Promise.all([relativeImport, absoluteImport, importLater()]).then(values => {{
                postMessage({{nested, values}}); close();
            }}, error => {{ postMessage({{error: error.name}}); close(); }});
        }} catch (error) {{ postMessage({{error: error.name}}); close(); }}
        "#
        ),
        format!("{worker_url}/worker/main.js"),
        worker_test_request_client(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    assert_eq!(
        expect_post_json(message),
        r#"{"nested":"worker","values":["TypeError","worker-module","TypeError"]}"#
    );
    foreign_server.await.unwrap();
    worker_server.await.unwrap();
}

#[tokio::test]
async fn worker_importscripts_cross_origin_async_errors_are_muted() {
    ensure_v8();
    let (foreign_url, server) = spawn_path_response_http_server(vec![(
        "/foreign.js",
        "HTTP/1.1 200 OK",
        "text/javascript",
        "setTimeout(() => { throw new Error('private message'); }, 0);".into(),
        Duration::ZERO,
    )])
    .await;
    let foreign = serde_json::to_string(&format!("{foreign_url}/foreign.js")).unwrap();
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        addEventListener('error', event => {{
            postMessage({{message: event.message, filename: event.filename,
                line: event.lineno, column: event.colno, errorIsNull: event.error === null}});
            event.preventDefault(); close();
        }});
        importScripts({foreign});
        "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        worker_test_request_client(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    assert_eq!(
        expect_post_json(message),
        r#"{"message":"Script error.","filename":"","line":0,"column":0,"errorIsNull":true}"#
    );
    server.await.unwrap();
}

#[tokio::test]
async fn worker_importscripts_cross_origin_rejections_are_not_reported() {
    ensure_v8();
    let (foreign_url, server) = spawn_path_response_http_server(vec![
        ("/foreign.js", "HTTP/1.1 200 OK", "text/javascript",
         "Promise.reject('private immediate'); setTimeout(() => Promise.reject('private timer'), 0);".into(), Duration::ZERO),
    ]).await;
    let foreign = serde_json::to_string(&format!("{foreign_url}/foreign.js")).unwrap();
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
        const reasons = [];
        addEventListener('unhandledrejection', event => {{
            reasons.push(event.reason); event.preventDefault();
        }});
        try {{ importScripts({foreign}); }} catch (error) {{ reasons.push(error.name); }}
        Promise.reject('public');
        setTimeout(() => {{ postMessage(reasons); close(); }}, 0);
        "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        worker_test_request_client(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    assert_eq!(expect_post_json(message), r#"["public"]"#);
    server.await.unwrap();
}
