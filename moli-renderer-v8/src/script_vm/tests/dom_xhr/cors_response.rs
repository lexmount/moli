use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct CorsResponseCase {
    label: &'static str,
    origins: &'static [&'static str],
    credentials: &'static [&'static str],
    include_credentials: bool,
    allowed: bool,
}

fn response_cases() -> Vec<CorsResponseCase> {
    const DEFAULT: CorsResponseCase = CorsResponseCase {
        label: "",
        origins: &["http://origin.test"],
        credentials: &["true"],
        include_credentials: false,
        allowed: true,
    };
    vec![
        CorsResponseCase {
            label: "single-origin",
            ..DEFAULT
        },
        CorsResponseCase {
            label: "wildcard",
            origins: &["*"],
            ..DEFAULT
        },
        CorsResponseCase {
            label: "credentialed-origin",
            include_credentials: true,
            ..DEFAULT
        },
        CorsResponseCase {
            label: "http-whitespace",
            origins: &[" \thttp://origin.test\t "],
            credentials: &["\ttrue "],
            include_credentials: true,
            ..DEFAULT
        },
        CorsResponseCase {
            label: "missing-origin",
            origins: &[],
            allowed: false,
            ..DEFAULT
        },
        CorsResponseCase {
            label: "duplicate-origin",
            origins: &["http://origin.test", "http://origin.test"],
            allowed: false,
            ..DEFAULT
        },
        CorsResponseCase {
            label: "wildcard-leading-duplicate",
            origins: &["*", "http://wrong.test", "*"],
            allowed: false,
            ..DEFAULT
        },
        CorsResponseCase {
            label: "origin-trailing-empty",
            origins: &["http://origin.test", ""],
            allowed: false,
            ..DEFAULT
        },
        CorsResponseCase {
            label: "wildcard-trailing-empty",
            origins: &["*", ""],
            allowed: false,
            ..DEFAULT
        },
        CorsResponseCase {
            label: "origin-leading-empty",
            origins: &["", "http://origin.test"],
            allowed: false,
            ..DEFAULT
        },
        CorsResponseCase {
            label: "combined-origins",
            origins: &["http://origin.test, http://origin.test"],
            allowed: false,
            ..DEFAULT
        },
        CorsResponseCase {
            label: "credentialed-wildcard",
            origins: &["*"],
            include_credentials: true,
            allowed: false,
            ..DEFAULT
        },
        CorsResponseCase {
            label: "duplicate-credentials",
            credentials: &["true", "true"],
            include_credentials: true,
            allowed: false,
            ..DEFAULT
        },
        CorsResponseCase {
            label: "credentials-trailing-false",
            credentials: &["true", "false"],
            include_credentials: true,
            allowed: false,
            ..DEFAULT
        },
        CorsResponseCase {
            label: "credentials-trailing-empty",
            credentials: &["true", ""],
            include_credentials: true,
            allowed: false,
            ..DEFAULT
        },
        CorsResponseCase {
            label: "missing-credentials",
            credentials: &[],
            include_credentials: true,
            allowed: false,
            ..DEFAULT
        },
        CorsResponseCase {
            label: "uppercase-credentials",
            credentials: &["True"],
            include_credentials: true,
            allowed: false,
            ..DEFAULT
        },
        CorsResponseCase {
            label: "unused-duplicate-credentials",
            credentials: &["true", "false", ""],
            ..DEFAULT
        },
        CorsResponseCase {
            label: "unused-invalid-credentials",
            credentials: &["True"],
            ..DEFAULT
        },
    ]
}

async fn check_response_fields(worker: bool) {
    for api in ["fetch", "xhr", "sync-xhr"] {
        let cases = std::sync::Arc::new(response_cases());
        let opaque_index = cases
            .iter()
            .position(|case| case.label == "duplicate-origin")
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let server_cases = cases.clone();
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let mut observed = Vec::new();
            loop {
                let mut socket = tokio::select! {
                    accepted = listener.accept() => accepted.unwrap().0,
                    _ = &mut stop_rx => break,
                };
                let mut head = Vec::new();
                let mut byte = [0; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    assert!(head.len() < 8192);
                    assert_eq!(socket.read(&mut byte).await.unwrap(), 1);
                    head.push(byte[0]);
                }
                let head = String::from_utf8(head).unwrap();
                let mut request_line = head.lines().next().unwrap().split_whitespace();
                let method = request_line.next().unwrap().to_owned();
                let (phase, index) = request_line
                    .next()
                    .unwrap()
                    .trim_start_matches('/')
                    .split_once('/')
                    .unwrap();
                let index: usize = index.parse().unwrap();
                let case = &server_cases[index];
                let use_case_fields = match phase {
                    "simple" | "opaque" => true,
                    "preflight" => method == "OPTIONS",
                    "after-preflight" => method != "OPTIONS",
                    _ => panic!("unexpected phase: {phase}"),
                };
                let mut response = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\nX-Secret: hidden\r\n".to_owned();
                let (origins, credentials) = if use_case_fields {
                    (case.origins, case.credentials)
                } else {
                    (&["http://origin.test"][..], &["true"][..])
                };
                for (name, values) in [
                    ("Access-Control-Allow-Origin", origins),
                    ("Access-Control-Allow-Credentials", credentials),
                ] {
                    for (index, value) in values.iter().enumerate() {
                        let name = if index == 0 {
                            name.to_owned()
                        } else {
                            name.to_ascii_lowercase()
                        };
                        response.push_str(&format!("{name}: {value}\r\n"));
                    }
                }
                if method == "OPTIONS" {
                    response.push_str(
                        "Access-Control-Allow-Methods: PUT\r\nAccess-Control-Max-Age: 0\r\n",
                    );
                    assert!(
                        head.to_ascii_lowercase()
                            .contains("\r\naccess-control-request-method: put\r\n")
                    );
                }
                observed.push((phase.to_owned(), index, method));
                response.push_str("\r\nok");
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            observed
        });
        let mut config = moli_fetch::FetchConfig::default();
        config.set_http_no_proxy(Some("*".to_owned()));
        let loader = ResourceRequestClient::new(&config).unwrap();
        let mut vm = new_page_task_executor_test_vm_with_loader("http://origin.test/page", &loader);
        let inputs = cases
            .iter()
            .map(|case| {
                serde_json::json!({"label": case.label, "credentials": case.include_credentials})
            })
            .collect::<Vec<_>>();
        let probe = format!(
            r#"
            (async () => {{
                const cases = {}, results = [];
                const api = {api:?}, base = {base:?};
                for (const phase of ['simple', 'preflight', 'after-preflight']) {{
                    const method = phase === 'simple' ? 'GET' : 'PUT';
                    for (const [index, item] of cases.entries()) {{
                        const url = base + phase + '/' + index;
                        let allowed = false;
                        if (api === 'fetch') {{
                            let response;
                            try {{
                                response = await fetch(url, {{method, credentials: item.credentials ? 'include' : 'omit'}});
                            }} catch (error) {{ if (!(error instanceof TypeError)) throw error; }}
                            if (response) {{
                                if (response.type !== 'cors' || response.status !== 200 || await response.text() !== 'ok') {{
                                    throw new Error('Unexpected Fetch response: ' + phase + '/' + item.label);
                                }}
                                allowed = true;
                            }}
                        }} else {{
                            const xhr = new XMLHttpRequest();
                            const done = new Promise(resolve => xhr.onloadend = resolve);
                            xhr.open(method, url, api !== 'sync-xhr');
                            xhr.withCredentials = item.credentials;
                            try {{
                                xhr.send();
                                if (api !== 'sync-xhr') await done;
                                allowed = xhr.status === 200 && xhr.responseText === 'ok';
                            }} catch (error) {{ if (api !== 'sync-xhr' || error.name !== 'NetworkError') throw error; }}
                            if (!allowed && (xhr.status !== 0 || xhr.responseText !== '' || xhr.getAllResponseHeaders() !== '')) {{
                                throw new Error('Rejected XHR exposed its response: ' + phase + '/' + item.label);
                            }}
                        }}
                        results.push([phase, item.label, allowed]);
                    }}
                }}
                if (api === 'fetch') {{
                    const response = await fetch(base + 'opaque/{opaque_index}', {{mode: 'no-cors'}});
                    if (response.type !== 'opaque' || response.status !== 0 || await response.text() !== '' || response.headers.get('X-Secret') !== null) {{
                        throw new Error('no-cors must preserve the opaque response');
                    }}
                }}
                return JSON.stringify(results);
            }})()
        "#,
            serde_json::to_string(&inputs).unwrap()
        );
        let script = if worker {
            let worker_script = format!(
                "Promise.resolve().then(() => {probe}).then(value => {{ postMessage(value); close(); }}, error => {{ postMessage(String(error.stack || error)); close(); }});"
            );
            format!(
                r#"
                globalThis.corsFieldsResult = 'pending';
                const worker = new Worker(URL.createObjectURL(new Blob([{}], {{type: 'text/javascript'}})));
                worker.onmessage = event => {{ corsFieldsResult = event.data; }};
                worker.onerror = event => {{ corsFieldsResult = event.message; event.preventDefault(); }};
            "#,
                serde_json::to_string(&worker_script).unwrap()
            )
        } else {
            format!(
                "globalThis.corsFieldsResult = 'pending'; Promise.resolve().then(() => {probe}).then(value => {{ corsFieldsResult = value; }}, error => {{ corsFieldsResult = String(error.stack || error); }});"
            )
        };
        vm.eval(&script).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while vm.eval("corsFieldsResult === 'pending'").unwrap() == "true" {
                wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
                    .await
                    .unwrap();
            }
        })
        .await
        .expect("CORS response field matrix should finish");
        stop_tx.send(()).unwrap();
        let observed = server.await.unwrap();
        let mut expected = Vec::new();
        let mut expected_requests = Vec::new();
        for phase in ["simple", "preflight", "after-preflight"] {
            for (index, case) in cases.iter().enumerate() {
                expected.push((phase, case.label, case.allowed));
                if phase != "simple" {
                    expected_requests.push((phase.to_owned(), index, "OPTIONS".to_owned()));
                }
                if phase != "preflight" || case.allowed {
                    expected_requests.push((
                        phase.to_owned(),
                        index,
                        if phase == "simple" { "GET" } else { "PUT" }.to_owned(),
                    ));
                }
            }
        }
        if api == "fetch" {
            expected_requests.push(("opaque".to_owned(), opaque_index, "GET".to_owned()));
        }
        assert_eq!(
            vm.eval("corsFieldsResult").unwrap(),
            serde_json::to_string(&expected).unwrap(),
            "worker={worker}, api={api}"
        );
        assert_eq!(
            observed, expected_requests,
            "failed preflights must prevent the actual request: worker={worker}, api={api}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cors_response_fields_reach_window_fetch_and_xhr() {
    check_response_fields(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cors_response_fields_reach_worker_fetch_and_xhr() {
    check_response_fields(true).await;
}
