use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct RedirectCase {
    label: String,
    sites: Vec<usize>,
    credentials: Option<&'static str>,
    denied_hop: Option<usize>,
    authorization: bool,
    allowed: bool,
}

fn redirect_cases() -> Vec<RedirectCase> {
    let mut cases = Vec::new();
    for sites in [vec![0, 0], vec![0, 1], vec![1, 0], vec![1, 1], vec![1, 2]] {
        for credentials in [
            None,
            Some(":"),
            Some("user:"),
            Some(":password"),
            Some("user:password"),
        ] {
            cases.push(RedirectCase {
                label: format!("{sites:?}/{credentials:?}"),
                allowed: sites == [0, 0] || matches!(credentials, None | Some(":")),
                sites: sites.clone(),
                credentials,
                denied_hop: None,
                authorization: false,
            });
        }
    }
    for (sites, denied_hop) in [(vec![1, 0], 0), (vec![0, 1, 0], 1), (vec![1, 2], 1)] {
        cases.push(RedirectCase {
            label: format!("unauthorized hop {denied_hop} in {sites:?}"),
            sites,
            denied_hop: Some(denied_hop),
            credentials: None,
            authorization: false,
            allowed: false,
        });
    }
    for sites in [vec![0, 0], vec![0, 1, 0], vec![1, 2]] {
        cases.push(RedirectCase {
            label: format!("authorization across {sites:?}"),
            sites,
            denied_hop: None,
            credentials: None,
            authorization: true,
            allowed: true,
        });
    }
    cases
}

async fn check_cors_redirects(worker: bool, api: &str) {
    let cases = std::sync::Arc::new(redirect_cases());
    let mut listeners = Vec::new();
    let mut origins = Vec::new();
    for _ in 0..3 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        origins.push(format!("http://{}", listener.local_addr().unwrap()));
        listeners.push(listener);
    }
    let server_cases = cases.clone();
    let server_origins = origins.clone();
    let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let mut observed = Vec::new();
        loop {
            let (site, mut socket) = tokio::select! {
                accepted = listeners[0].accept() => (0, accepted.unwrap().0),
                accepted = listeners[1].accept() => (1, accepted.unwrap().0),
                accepted = listeners[2].accept() => (2, accepted.unwrap().0),
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
            let mut line = head.lines().next().unwrap().split_whitespace();
            let method = line.next().unwrap();
            let path = line.next().unwrap();
            let parts = path
                .trim_start_matches('/')
                .split('/')
                .map(|part| part.parse::<usize>().unwrap())
                .collect::<Vec<_>>();
            let [status, index, hop] = parts[..] else {
                panic!("unexpected path {path}")
            };
            let case = &server_cases[index];
            assert_eq!(case.sites[hop], site);
            let header = |name: &str| {
                head.lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(field, _)| field.eq_ignore_ascii_case(name))
                    .map(|(_, value)| value.trim().to_owned())
            };
            let origin = header("origin");
            let authorization = header("authorization");
            observed.push((
                status,
                index,
                hop,
                method.to_owned(),
                origin.clone(),
                authorization,
            ));
            let mut response = format!(
                "HTTP/1.1 {} Response\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\nCache-Control: no-store\r\n",
                if method != "OPTIONS" && hop + 1 < case.sites.len() {
                    status
                } else {
                    200
                }
            );
            // Even forbidden destinations opt in. A rejected redirect must not
            // be explained by a later response's missing permission headers.
            if method == "OPTIONS" || case.denied_hop != Some(hop) {
                response.push_str(&format!(
                    "Access-Control-Allow-Origin: {}\r\nAccess-Control-Allow-Credentials: true\r\n",
                    origin.as_deref().unwrap_or(&server_origins[0])
                ));
            }
            if method == "OPTIONS" {
                response.push_str("Access-Control-Allow-Methods: GET\r\nAccess-Control-Allow-Headers: authorization\r\nAccess-Control-Max-Age: 0\r\n");
            } else if hop + 1 < case.sites.len() {
                let mut destination = url::Url::parse(&format!(
                    "{}/{status}/{index}/{}",
                    server_origins[case.sites[hop + 1]],
                    hop + 1
                ))
                .unwrap();
                if hop + 2 == case.sites.len()
                    && let Some(credentials) = case.credentials
                {
                    let (username, password) = credentials.split_once(':').unwrap();
                    destination.set_username(username).unwrap();
                    destination.set_password(Some(password)).unwrap();
                }
                response.push_str(&format!("Location: {destination}\r\n"));
            }
            response.push_str("\r\nok");
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        observed
    });
    let mut config = moli_fetch::FetchConfig::default();
    config.set_http_no_proxy(Some("*".to_owned()));
    let loader = ResourceRequestClient::new(&config).unwrap();
    let mut vm =
        new_page_task_executor_test_vm_with_loader(&format!("{}/page", origins[0]), &loader);
    let inputs = cases.iter().map(|case| serde_json::json!({"label": case.label, "site": case.sites[0], "authorization": case.authorization})).collect::<Vec<_>>();
    let probe = format!(
        r#"
        (async () => {{
            const cases = {}, origins = {}, api = {api:?}, results = [];
            for (const status of [301, 302, 303, 307, 308]) {{
                for (const [index, item] of cases.entries()) {{
                    const url = origins[item.site] + '/' + status + '/' + index + '/0';
                    let allowed = false;
                    if (api === 'fetch') {{
                        let response;
                        try {{
                            response = await fetch(url, {{headers: item.authorization ? {{Authorization: 'Bearer author'}} : {{}}}});
                        }} catch (error) {{ if (!(error instanceof TypeError)) throw error; }}
                        if (response) {{
                            if (response.status !== 200 || !response.redirected || await response.text() !== 'ok') throw new Error('Unexpected redirect response');
                            allowed = true;
                        }}
                    }} else {{
                        const xhr = new XMLHttpRequest();
                        const done = new Promise(resolve => xhr.onloadend = resolve);
                        xhr.open('GET', url, api !== 'sync-xhr');
                        if (item.authorization) xhr.setRequestHeader('Authorization', 'Bearer author');
                        try {{
                            xhr.send();
                            if (api !== 'sync-xhr') await done;
                            allowed = xhr.status === 200 && xhr.responseText === 'ok';
                        }} catch (error) {{ if (api !== 'sync-xhr' || error.name !== 'NetworkError') throw error; }}
                        if (!allowed && (xhr.status !== 0 || xhr.responseText !== '' || xhr.getAllResponseHeaders() !== '')) throw new Error('Rejected XHR exposed response');
                    }}
                    results.push([status, item.label, allowed]);
                }}
            }}
            return JSON.stringify(results);
        }})()
    "#,
        serde_json::to_string(&inputs).unwrap(),
        serde_json::to_string(&origins).unwrap()
    );
    let script = if worker {
        let worker_script = format!(
            "Promise.resolve().then(() => {probe}).then(value => {{ postMessage(value); close(); }}, error => {{ postMessage(String(error.stack || error)); close(); }});"
        );
        format!(
            r#"
            globalThis.corsRedirectResult = 'pending';
            const worker = new Worker(URL.createObjectURL(new Blob([{}], {{type: 'text/javascript'}})));
            worker.onmessage = event => {{ corsRedirectResult = event.data; }};
            worker.onerror = event => {{ corsRedirectResult = event.message; event.preventDefault(); }};
        "#,
            serde_json::to_string(&worker_script).unwrap()
        )
    } else {
        format!(
            "globalThis.corsRedirectResult = 'pending'; Promise.resolve().then(() => {probe}).then(value => {{ corsRedirectResult = value; }}, error => {{ corsRedirectResult = String(error.stack || error); }});"
        )
    };
    vm.eval(&script).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while vm.eval("corsRedirectResult === 'pending'").unwrap() == "true" {
            wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
                .await
                .unwrap();
        }
    })
    .await
    .expect("CORS redirect matrix should finish");
    stop_tx.send(()).unwrap();
    let observed = server.await.unwrap();
    let expected = [301, 302, 303, 307, 308]
        .into_iter()
        .flat_map(|status| {
            cases
                .iter()
                .map(move |case| (status, &case.label, case.allowed))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        vm.eval("corsRedirectResult").unwrap(),
        serde_json::to_string(&expected).unwrap(),
        "worker={worker}, api={api}"
    );
    let mut expected_requests = Vec::new();
    for status in [301, 302, 303, 307, 308] {
        for (index, case) in cases.iter().enumerate() {
            let hops = if case.allowed {
                case.sites.len()
            } else if let Some(hop) = case.denied_hop {
                hop + 1
            } else {
                case.sites.len() - 1
            };
            let mut tainted = false;
            let mut authorization = case.authorization;
            for hop in 0..hops {
                let site = case.sites[hop];
                if hop > 0 && case.sites[hop - 1] != site {
                    tainted |= case.sites[hop - 1] != 0;
                    authorization = false;
                }
                if authorization && (site != 0 || tainted) {
                    expected_requests.push((status, index, hop, "OPTIONS".to_owned()));
                }
                expected_requests.push((status, index, hop, "GET".to_owned()));
                for record in observed
                    .iter()
                    .filter(|record| record.0 == status && record.1 == index && record.2 == hop)
                {
                    if site != 0 || tainted {
                        assert_eq!(
                            record.4.as_deref(),
                            Some(if tainted { "null" } else { origins[0].as_str() }),
                            "request Origin for {} at hop {hop}",
                            case.label
                        );
                    }
                    if case.authorization {
                        assert_eq!(
                            record.5.as_deref(),
                            (authorization && record.3 != "OPTIONS").then_some("Bearer author"),
                            "Authorization for {} at hop {hop}",
                            case.label
                        );
                    }
                }
            }
        }
    }
    assert_eq!(
        observed
            .into_iter()
            .map(|(status, index, hop, method, _, _)| (status, index, hop, method))
            .collect::<Vec<_>>(),
        expected_requests,
        "forbidden redirect destinations must not receive a request: worker={worker}, api={api}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cors_redirect_policy_reaches_window_fetch() {
    check_cors_redirects(false, "fetch").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cors_redirect_policy_reaches_worker_fetch() {
    check_cors_redirects(true, "fetch").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cors_redirect_policy_reaches_window_xhr() {
    check_cors_redirects(false, "xhr").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cors_redirect_policy_reaches_worker_xhr() {
    check_cors_redirects(true, "xhr").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cors_redirect_policy_reaches_window_sync_xhr() {
    check_cors_redirects(false, "sync-xhr").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cors_redirect_policy_reaches_worker_sync_xhr() {
    check_cors_redirects(true, "sync-xhr").await;
}
