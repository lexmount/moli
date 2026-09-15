use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct PreflightCase {
    label: &'static str,
    method: &'static str,
    headers: &'static [(&'static str, &'static str)],
    permissions: &'static [(&'static str, &'static str)],
    credentials: bool,
    upload_listener: bool,
    allowed: bool,
}

fn permission_cases() -> Vec<PreflightCase> {
    const DEFAULT: PreflightCase = PreflightCase {
        label: "",
        method: "PUT",
        headers: &[("X-Test", "1")],
        permissions: &[],
        credentials: false,
        upload_listener: false,
        allowed: true,
    };
    vec![
        PreflightCase {
            label: "duplicate-fields",
            permissions: &[
                ("Access-Control-Allow-Methods", "POST"),
                ("access-control-allow-methods", "PUT"),
                ("Access-Control-Allow-Headers", "x-other"),
                ("ACCESS-CONTROL-ALLOW-HEADERS", "X-Test"),
            ],
            ..DEFAULT
        },
        PreflightCase {
            label: "wildcards",
            permissions: &[
                ("Access-Control-Allow-Methods", "*"),
                ("Access-Control-Allow-Headers", "*"),
            ],
            ..DEFAULT
        },
        PreflightCase {
            label: "credentialed-method-wildcard",
            permissions: &[
                ("Access-Control-Allow-Methods", "*"),
                ("Access-Control-Allow-Headers", "X-Test"),
            ],
            credentials: true,
            allowed: false,
            ..DEFAULT
        },
        PreflightCase {
            label: "credentialed-header-wildcard",
            permissions: &[
                ("Access-Control-Allow-Methods", "PUT"),
                ("Access-Control-Allow-Headers", "*"),
            ],
            credentials: true,
            allowed: false,
            ..DEFAULT
        },
        PreflightCase {
            label: "credentialed-explicit-permissions",
            permissions: &[
                ("Access-Control-Allow-Methods", "*, PUT"),
                ("Access-Control-Allow-Headers", "*, x-test"),
            ],
            credentials: true,
            ..DEFAULT
        },
        PreflightCase {
            label: "credentialed-literal-stars",
            method: "*",
            headers: &[("*", "1")],
            permissions: &[
                ("Access-Control-Allow-Methods", "*"),
                ("Access-Control-Allow-Headers", "*"),
            ],
            credentials: true,
            ..DEFAULT
        },
        PreflightCase {
            label: "authorization-needs-explicit-permission",
            method: "POST",
            headers: &[("Authorization", "secret")],
            permissions: &[("Access-Control-Allow-Headers", "*")],
            allowed: false,
            ..DEFAULT
        },
        PreflightCase {
            label: "authorization-in-second-field",
            method: "POST",
            headers: &[("aUtHoRiZaTiOn", "secret")],
            permissions: &[
                ("Access-Control-Allow-Headers", "*"),
                ("Access-Control-Allow-Headers", "AUTHORIZATION"),
            ],
            credentials: true,
            ..DEFAULT
        },
        PreflightCase {
            label: "case-sensitive-method",
            method: "patcH",
            permissions: &[
                ("Access-Control-Allow-Methods", "patcH"),
                ("Access-Control-Allow-Headers", "x-TEST"),
            ],
            ..DEFAULT
        },
        PreflightCase {
            label: "method-case-mismatch",
            method: "patcH",
            permissions: &[
                ("Access-Control-Allow-Methods", "PATCH"),
                ("Access-Control-Allow-Headers", "x-test"),
            ],
            allowed: false,
            ..DEFAULT
        },
        PreflightCase {
            label: "empty-elements-and-http-whitespace",
            permissions: &[
                ("Access-Control-Allow-Methods", ",\t PUT, ,"),
                ("Access-Control-Allow-Headers", "\t, X-Test,\t,"),
            ],
            ..DEFAULT
        },
        PreflightCase {
            label: "missing-method-permission",
            permissions: &[("Access-Control-Allow-Headers", "x-test")],
            allowed: false,
            ..DEFAULT
        },
        PreflightCase {
            label: "safelisted-method-without-permission",
            method: "GET",
            permissions: &[("Access-Control-Allow-Headers", "x-test")],
            ..DEFAULT
        },
        PreflightCase {
            label: "malformed-method-after-matching-field",
            permissions: &[
                ("Access-Control-Allow-Methods", "PUT"),
                ("Access-Control-Allow-Methods", "Bad value"),
                ("Access-Control-Allow-Headers", "X-Test"),
            ],
            allowed: false,
            ..DEFAULT
        },
        PreflightCase {
            label: "malformed-header-after-matching-field",
            permissions: &[
                ("Access-Control-Allow-Methods", "PUT"),
                ("Access-Control-Allow-Headers", "X-Test"),
                ("Access-Control-Allow-Headers", "Bad value"),
            ],
            allowed: false,
            ..DEFAULT
        },
        PreflightCase {
            label: "safelisted-method-with-malformed-method-list",
            method: "GET",
            permissions: &[
                ("Access-Control-Allow-Methods", "Bad value"),
                ("Access-Control-Allow-Headers", "X-Test"),
            ],
            allowed: false,
            ..DEFAULT
        },
        PreflightCase {
            label: "no-unsafe-headers-with-malformed-header-list",
            headers: &[],
            permissions: &[
                ("Access-Control-Allow-Methods", "PUT"),
                ("Access-Control-Allow-Headers", "Bad value"),
            ],
            allowed: false,
            ..DEFAULT
        },
        PreflightCase {
            label: "upload-flag-with-missing-method-list",
            headers: &[],
            upload_listener: true,
            ..DEFAULT
        },
        PreflightCase {
            label: "upload-flag-with-empty-method-list",
            headers: &[],
            permissions: &[("Access-Control-Allow-Methods", "")],
            upload_listener: true,
            allowed: false,
            ..DEFAULT
        },
        PreflightCase {
            label: "credentialed-upload-with-missing-method-list",
            headers: &[],
            upload_listener: true,
            credentials: true,
            ..DEFAULT
        },
        PreflightCase {
            label: "upload-flag-still-requires-header-permission",
            upload_listener: true,
            allowed: false,
            ..DEFAULT
        },
        PreflightCase {
            label: "upload-flag-with-different-method-list",
            headers: &[],
            permissions: &[("Access-Control-Allow-Methods", "GET")],
            upload_listener: true,
            allowed: false,
            ..DEFAULT
        },
    ]
}

async fn check_preflight_permissions(worker: bool) {
    for api in ["fetch", "xhr", "sync-xhr"] {
        let cases = std::sync::Arc::new(
            permission_cases()
                .into_iter()
                .filter(|case| api != "fetch" || !case.upload_listener)
                .collect::<Vec<_>>(),
        );
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
                let index: usize = request_line
                    .next()
                    .unwrap()
                    .trim_start_matches('/')
                    .parse()
                    .unwrap();
                let length = head
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                socket.read_exact(&mut vec![0; length]).await.unwrap();
                let mut response = "HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: http://origin.test\r\nAccess-Control-Allow-Credentials: true\r\nAccess-Control-Max-Age: 0\r\nContent-Length: 2\r\nConnection: close\r\n".to_owned();
                if method == "OPTIONS" {
                    for (name, value) in server_cases[index].permissions {
                        response.push_str(&format!("{name}: {value}\r\n"));
                    }
                    let lower_head = head.to_ascii_lowercase();
                    assert!(
                        !lower_head.contains("\r\nauthorization:"),
                        "OPTIONS must not carry credentials"
                    );
                    assert!(lower_head.contains(&format!(
                        "\r\naccess-control-request-method: {}\r\n",
                        server_cases[index].method.to_ascii_lowercase()
                    )));
                }
                observed.push((index, method));
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
                serde_json::json!({
                    "label": case.label, "method": case.method, "headers": case.headers,
                    "credentials": case.credentials, "upload": case.upload_listener,
                })
            })
            .collect::<Vec<_>>();
        let probe = format!(
            r#"
            (async () => {{
                const cases = {}, results = [];
                const api = {api:?}, base = {base:?};
                for (const [index, item] of cases.entries()) {{
                    const url = base + index;
                    let allowed = false;
                    if (api === 'fetch') {{
                        try {{
                            const response = await fetch(url, {{method: item.method, headers: item.headers, credentials: item.credentials ? 'include' : 'omit'}});
                            allowed = response.status === 200 && await response.text() === 'ok';
                        }} catch (error) {{ if (!(error instanceof TypeError)) throw error; }}
                    }} else {{
                        const xhr = new XMLHttpRequest();
                        const done = new Promise(resolve => xhr.onloadend = resolve);
                        xhr.open(item.method, url, api !== 'sync-xhr');
                        xhr.withCredentials = item.credentials;
                        for (const [name, value] of item.headers) xhr.setRequestHeader(name, value);
                        if (item.upload) xhr.upload.onprogress = () => {{}};
                        try {{
                            xhr.send('payload');
                            if (api !== 'sync-xhr') await done;
                            allowed = xhr.status === 200 && xhr.responseText === 'ok';
                        }} catch (error) {{ if (api !== 'sync-xhr' || error.name !== 'NetworkError') throw error; }}
                    }}
                    results.push([item.label, allowed]);
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
                globalThis.preflightResult = 'pending';
                const worker = new Worker(URL.createObjectURL(new Blob([{}], {{type: 'text/javascript'}})));
                worker.onmessage = event => {{ preflightResult = event.data; }};
                worker.onerror = event => {{ preflightResult = event.message; event.preventDefault(); }};
            "#,
                serde_json::to_string(&worker_script).unwrap()
            )
        } else {
            format!(
                "globalThis.preflightResult = 'pending'; Promise.resolve().then(() => {probe}).then(value => {{ preflightResult = value; }}, error => {{ preflightResult = String(error.stack || error); }});"
            )
        };
        vm.eval(&script).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while vm.eval("preflightResult === 'pending'").unwrap() == "true" {
                wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
                    .await
                    .unwrap();
            }
        })
        .await
        .expect("preflight permission matrix should finish");
        stop_tx.send(()).unwrap();
        let observed = server.await.unwrap();
        let expected = cases
            .iter()
            .map(|case| (case.label, case.allowed))
            .collect::<Vec<_>>();
        assert_eq!(
            vm.eval("preflightResult").unwrap(),
            serde_json::to_string(&expected).unwrap(),
            "worker={worker}, api={api}"
        );
        let expected_requests = cases
            .iter()
            .enumerate()
            .flat_map(|(index, case)| {
                std::iter::once((index, "OPTIONS".to_owned()))
                    .chain(case.allowed.then(|| (index, case.method.to_owned())))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            observed, expected_requests,
            "only approved requests may reach transport: worker={worker}, api={api}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cors_preflight_permissions_reach_window_fetch_and_xhr() {
    check_preflight_permissions(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cors_preflight_permissions_reach_worker_fetch_and_xhr() {
    check_preflight_permissions(true).await;
}
