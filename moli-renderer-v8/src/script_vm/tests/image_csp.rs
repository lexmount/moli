use super::*;
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct ImageCspServer {
    origin: String,
    cross_origin: String,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
    port: u16,
}

impl ImageCspServer {
    async fn spawn() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let origin = format!("http://image-csp.test:{port}");
        let cross_origin = format!("http://cross-image-csp.test:{port}");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let redirect_origin = cross_origin.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 2048];
                while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                    let count = socket.read(&mut buffer).await.unwrap();
                    if count == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                }
                let request = String::from_utf8_lossy(&bytes);
                let target = request.split_ascii_whitespace().nth(1).unwrap();
                captured.lock().push(target.to_owned());
                let (status, headers, body): (&str, String, &[u8]) = if target.contains("redirect")
                {
                    let destination = if target.contains("cross") {
                        format!("{redirect_origin}/pixel.gif")
                    } else {
                        "/pixel.gif".to_owned()
                    };
                    ("302 Found", format!("Location: {destination}\r\n"), b"")
                } else if target == "/child.html" {
                    (
                        "200 OK",
                        "Content-Type: text/html\r\nContent-Security-Policy: img-src 'self'\r\n"
                            .to_owned(),
                        br#"<!doctype html><body><script>
                        addEventListener('securitypolicyviolation', event => parent.postMessage(event.documentURI, '*'));
                        for (const [name, origin] of [
                          ['child', location.origin],
                          ['parent', location.origin.replace('cross-image-csp', 'image-csp')],
                        ]) {
                          const image = new Image();
                          image.onload = () => parent.postMessage(name + ':load', '*');
                          image.onerror = () => parent.postMessage(name + ':error', '*');
                          image.src = origin + '/pixel.gif';
                          document.body.append(image);
                        }
                        </script>"#,
                    )
                } else {
                    (
                        "200 OK",
                        "Content-Type: image/gif\r\n".to_owned(),
                        b"GIF89a\x01\0\x01\0\x80\0\0\0\0\0\xff\xff\xff!\xf9\x04\x01\0\0\0\0,\0\0\0\0\x01\0\x01\0\0\x02\x02D\x01\0;",
                    )
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(head.as_bytes()).await.unwrap();
                socket.write_all(body).await.unwrap();
            }
        });
        Self {
            origin,
            cross_origin,
            requests,
            task,
            port,
        }
    }

    fn loader(&self) -> crate::network::ResourceRequestClientOwner {
        let loader = static_http_loader([
            format!("image-csp.test:{}:127.0.0.1", self.port),
            format!("cross-image-csp.test:{}:127.0.0.1", self.port),
        ]);
        loader.set_image_fetch_enabled(true);
        loader
    }
}

impl Drop for ImageCspServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn loaded_child(
    vm: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
    setup: &str,
) {
    vm.eval(&format!(
        r#"
        globalThis.frameLoaded = false;
        globalThis.frame = document.createElement('iframe');
        frame.onload = () => {{ frameLoaded = true; }};
        {setup}
        document.body.append(frame);
    "#
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        vm,
        loader,
        "String(frameLoaded)",
        "true",
        "child document must load",
    )
    .await;
}

fn set_header_policy(
    vm: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    document_url: &str,
    policy: &str,
    report_only: bool,
) {
    let header = if report_only {
        "Content-Security-Policy-Report-Only"
    } else {
        "Content-Security-Policy"
    };
    vm.set_main_navigation_policy_container(
        crate::document_runtime::DocumentPolicyContainer::from_navigation_response_headers(
            &[(header.to_owned(), policy.as_bytes().to_vec())],
            &Url::parse(document_url).unwrap(),
        ),
    );
}

#[tokio::test]
async fn image_csp_redirects_check_final_url_with_child_policy_and_report_owner() {
    for (cross, report_only, path_source) in [
        (false, false, false),
        (true, false, false),
        (true, true, false),
        (false, false, true),
    ] {
        let server = ImageCspServer::spawn().await;
        let loader = server.loader();
        let document_url = format!("{}/page.html", server.origin);
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document_url, &loader);
        let policy = if path_source {
            format!("img-src {}/allowed/", server.origin)
        } else {
            "img-src 'self'".to_owned()
        };
        set_header_policy(&mut vm, &document_url, &policy, report_only);
        loaded_child(&mut vm, &loader, "frame.srcdoc = '<!doctype html><body>';").await;
        let path = if cross {
            "/allowed/redirect-cross"
        } else {
            "/allowed/redirect"
        };
        vm.eval(&format!(r#"
            globalThis.events = [];
            addEventListener('securitypolicyviolation', () => events.push('parent-report'));
            frame.contentWindow.addEventListener('securitypolicyviolation', event => {{
                events.push([event.disposition, event.effectiveDirective, event.documentURI, event.blockedURI].join('|'));
            }});
            globalThis.imageResult = 'pending';
            const image = frame.contentDocument.createElement('img');
            image.onload = () => {{ imageResult = 'load'; }};
            image.onerror = () => {{ imageResult = 'error'; }};
            image.src = '{}{path}';
            frame.contentDocument.body.append(image);
        "#, server.origin)).unwrap();
        let expected = if cross && !report_only {
            "error"
        } else {
            "load"
        };
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "imageResult",
            expected,
            "redirected image CSP result",
        )
        .await;
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
            .await
            .unwrap();
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm);
        assert_eq!(
            vm.eval("JSON.stringify(events)").unwrap(),
            if cross {
                let disposition = if report_only { "report" } else { "enforce" };
                serde_json::json!([format!(
                    "{disposition}|img-src|about|{}/pixel.gif",
                    server.cross_origin
                )])
                .to_string()
            } else {
                "[]".to_owned()
            }
        );
        // This is a response check: both HTTP hops complete, but a forbidden
        // final response must never be published as a successful image.
        assert_eq!(*server.requests.lock(), [path, "/pixel.gif"]);
    }
}

#[tokio::test]
async fn image_csp_header_self_survives_nested_local_documents_and_base_changes() {
    for srcdoc in [false, true] {
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let document_url = "https://image-csp-creator.test/page.html";
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(document_url, &loader);
        set_header_policy(&mut vm, document_url, "img-src 'self'", false);
        loaded_child(
            &mut vm,
            &loader,
            if srcdoc {
                "frame.srcdoc = '<!doctype html><body>';"
            } else {
                "frame.src = 'about:blank';"
            },
        )
        .await;
        vm.eval(
            r#"
            const base = frame.contentDocument.createElement('base');
            base.href = 'https://unrelated-base.test/';
            frame.contentDocument.head.append(base);
            globalThis.nestedLoaded = false;
            globalThis.nested = frame.contentDocument.createElement('iframe');
            nested.onload = () => { nestedLoaded = true; };
            nested.srcdoc = '<!doctype html><body>';
            frame.contentDocument.body.append(nested);
        "#,
        )
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(nestedLoaded)",
            "true",
            "nested local document must load",
        )
        .await;
        vm.eval(
            r#"
            globalThis.events = [];
            nested.contentWindow.addEventListener('securitypolicyviolation', event => {
                events.push(event.effectiveDirective + ':' + event.documentURI);
            });
            for (const [name, url] of [
                ['self', 'https://image-csp-creator.test/pixel.gif'],
                ['base', 'https://unrelated-base.test/pixel.gif'],
            ]) {
                const image = nested.contentDocument.createElement('img');
                image.onload = () => events.push(name + ':load');
                image.onerror = () => events.push(name + ':error');
                image.src = url;
                nested.contentDocument.body.append(image);
            }
        "#,
        )
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(events.filter(x => /:(load|error)$/.test(x)).length)",
            "2",
            "nested image terminals",
        )
        .await;
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
            .await
            .unwrap();
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm);
        assert_eq!(
            vm.eval("events.sort().join('|')").unwrap(),
            "base:error|img-src:about|self:load"
        );
    }
}

#[tokio::test]
async fn image_csp_network_child_uses_its_own_policy_source_after_navigation() {
    let server = ImageCspServer::spawn().await;
    let loader = server.loader();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        &format!("{}/page.html", server.origin),
        &loader,
    );
    vm.eval(
        "globalThis.events = []; addEventListener('message', event => events.push(event.data));",
    )
    .unwrap();
    loaded_child(
        &mut vm,
        &loader,
        &format!("frame.src = '{}/child.html';", server.cross_origin),
    )
    .await;
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(events.filter(x => /:(load|error)$/.test(x)).length)",
        "2",
        "network child image terminals",
    )
    .await;
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm);
    assert_eq!(
        vm.eval("events.sort().join('|')").unwrap(),
        format!("child:load|{}/child.html|parent:error", server.cross_origin)
    );
    assert_eq!(*server.requests.lock(), ["/child.html", "/pixel.gif"]);
}

#[tokio::test]
async fn image_csp_rechecks_cached_redirect_response_for_the_requesting_document() {
    for report_only in [false, true] {
        let server = ImageCspServer::spawn().await;
        let loader = server.loader();
        let document_url = format!("{}/page.html", server.origin);
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document_url, &loader);
        vm.eval(&format!(
            r#"
            globalThis.warmResult = 'pending';
            globalThis.warmImage = new Image();
            warmImage.onload = () => {{ warmResult = 'load'; }};
            warmImage.onerror = () => {{ warmResult = 'error'; }};
            warmImage.src = '{}/allowed/redirect-cross';
            document.body.append(warmImage);
        "#,
            server.origin
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "warmResult",
            "load",
            "unrestricted redirect must warm the image cache",
        )
        .await;
        set_header_policy(&mut vm, &document_url, "img-src 'self'", report_only);
        loaded_child(&mut vm, &loader, "frame.srcdoc = '<!doctype html><body>';").await;
        vm.eval(&format!(
            r#"
            globalThis.events = [];
            frame.contentWindow.addEventListener('securitypolicyviolation', event => {{
                events.push(event.disposition + '|' + event.documentURI + '|' + event.blockedURI);
            }});
            globalThis.imageResult = 'pending';
            const image = frame.contentDocument.createElement('img');
            image.onload = () => {{ imageResult = 'load'; }};
            image.onerror = () => {{ imageResult = 'error'; }};
            image.src = '{}/allowed/redirect-cross';
            frame.contentDocument.body.append(image);
        "#,
            server.origin
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "imageResult",
            if report_only { "load" } else { "error" },
            "cached redirect must honor the new owner's policy",
        )
        .await;
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
            .await
            .unwrap();
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm);
        assert_eq!(
            vm.eval("events.join(',')").unwrap(),
            format!(
                "{}|about|{}/pixel.gif",
                if report_only { "report" } else { "enforce" },
                server.cross_origin,
            )
        );
        assert_eq!(
            *server.requests.lock(),
            ["/allowed/redirect-cross", "/pixel.gif"]
        );
    }
}
