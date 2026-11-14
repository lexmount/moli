use super::native_xhr::run_cdp_command_with_network_decisions;
use super::*;
use axum::{body::Bytes, http::Uri};
use parking_lot::Mutex;

#[derive(Clone, Copy, Debug)]
enum Api {
    Fetch,
    AsyncXhr,
    SyncXhr,
}

impl Api {
    fn resource_type(self) -> &'static str {
        if matches!(self, Self::Fetch) {
            "Fetch"
        } else {
            "XHR"
        }
    }

    fn expression(self, url: &str, method: &str, headers: Value) -> String {
        let body = if method == "POST" {
            "'abc'"
        } else {
            "undefined"
        };
        match self {
            Self::Fetch => format!(
                "fetch({url:?},{{method:{method:?},headers:{headers},body:{body}}}).then(r=>r.text()).catch(e=>'ERROR:'+e.name)"
            ),
            Self::AsyncXhr | Self::SyncXhr => format!(
                "new Promise(resolve=>{{const x=new XMLHttpRequest();x.open({method:?},{url:?},{asynchronous});for(const [k,v] of Object.entries({headers}))x.setRequestHeader(k,v);x.onload=()=>resolve(x.responseText);x.onerror=()=>resolve('ERROR:NetworkError');try{{x.send({body});}}catch(e){{resolve('ERROR:'+e.name);}}}})",
                asynchronous = matches!(self, Self::AsyncXhr)
            ),
        }
    }
}

const APIS: [Api; 3] = [Api::Fetch, Api::AsyncXhr, Api::SyncXhr];

struct Fixture {
    origin: String,
    peer: String,
    requests: Arc<Mutex<Vec<Value>>>,
    servers: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for server in &self.servers {
            server.abort();
        }
    }
}

impl Fixture {
    async fn new() -> Self {
        let mut listeners = Vec::new();
        for _ in 0..3 {
            listeners.push(TcpListener::bind("127.0.0.1:0").await.unwrap());
        }
        let origins = listeners
            .iter()
            .map(|listener| format!("http://{}", listener.local_addr().unwrap()))
            .collect::<Vec<_>>();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let peer = origins[1].clone();
        let last = origins[2].clone();
        let app = Router::new().fallback(any(move |method: Method, uri: Uri, headers: HeaderMap, body: Bytes| {
            let recorded = recorded.clone();
            let peer = peer.clone();
            let last = last.clone();
            async move {
                let header = |name| headers.get(name).and_then(|value| value.to_str().ok());
                let record = json!({"path":uri.path(),"method":method.as_str(),"contentType":header("content-type"),"contentLength":header("content-length"),"origin":header("origin"),"authorization":header("authorization"),"keep":header("x-keep"),"preflight":header("access-control-request-headers"),"body":String::from_utf8_lossy(&body)});
                recorded.lock().push(record.clone());
                let mut response = if method == Method::OPTIONS {
                    StatusCode::NO_CONTENT.into_response()
                } else if uri.path() == "/page" {
                    axum::response::Html("<!doctype html>ready").into_response()
                } else if let Some(status) = uri.path().strip_prefix("/start/") {
                    (StatusCode::from_u16(status.parse().unwrap()).unwrap(), [("location", "/target")], "").into_response()
                } else if uri.path() == "/auth-start" {
                    (StatusCode::FOUND, [("location", format!("{peer}/auth-next"))], "").into_response()
                } else if uri.path() == "/auth-next" {
                    (StatusCode::FOUND, [("location", format!("{last}/target"))], "").into_response()
                } else {
                    axum::Json(record).into_response()
                };
                for (name,value) in [("access-control-allow-origin","*"),("access-control-allow-methods","GET, POST"),("access-control-allow-headers","authorization, content-type, x-keep")] {
                    response.headers_mut().insert(name,value.parse().unwrap());
                }
                response
            }
        }));
        let servers = listeners
            .into_iter()
            .map(|listener| {
                let app = app.clone();
                tokio::spawn(async move {
                    axum::serve(listener, app).await.unwrap();
                })
            })
            .collect();
        Self {
            origin: origins[0].clone(),
            peer: origins[1].clone(),
            requests,
            servers,
        }
    }

    async fn context(&self, api: Api) -> TestContext {
        let mut ctx = TestContext::new();
        with_loaded_http_document(&mut ctx, &format!("{}/page", self.origin), "SID-1", "TID-1")
            .await;
        enable_runtime_async(&mut ctx, "SID-1", 47000).await;
        ctx.process_async(json!({"id":47001,"sessionId":"SID-1","method":"Fetch.enable","params":{"patterns":[{"urlPattern":"*","resourceType":api.resource_type()}]}})).await;
        ctx.expect_result(47001, json!({}), Some("SID-1"));
        self.requests.lock().clear();
        ctx
    }
}

fn continue_request(pause: &Value, headers: Option<Vec<Value>>, method: Option<&str>) -> Value {
    let mut params = json!({"requestId":pause["params"]["requestId"]});
    if let Some(headers) = headers {
        params["headers"] = json!(headers);
    }
    if let Some(method) = method {
        params["method"] = json!(method);
    }
    json!({"id":47003,"sessionId":"SID-1","method":"Fetch.continueRequest","params":params})
}

fn echo_headers(pause: &Value) -> Vec<Value> {
    pause["params"]["request"]["headers"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, value)| json!({"name":name,"value":value}))
        .collect()
}

async fn request(
    ctx: &mut TestContext,
    expression: String,
    decide: impl FnMut(&Value) -> Value,
) -> (Value, Vec<Value>) {
    let pauses=run_cdp_command_with_network_decisions(ctx,json!({"id":47002,"sessionId":"SID-1","method":"Runtime.evaluate","params":{"expression":expression,"awaitPromise":true,"returnByValue":true}}),decide).await;
    let result = take_response_by_id(ctx, 47002);
    assert!(
        result.get("error").is_none() && result["result"].get("exceptionDetails").is_none(),
        "{result}"
    );
    (result["result"]["result"]["value"].clone(), pauses)
}

async fn assert_terminal(ctx: &mut TestContext, pauses: &[Value], terminal: &str) {
    let id = pauses[0]["params"]["networkId"].clone();
    assert!(id.as_str().is_some_and(|id| !id.is_empty()));
    for pause in pauses {
        assert_eq!(pause["params"]["networkId"], id);
    }
    wait_until_scheduler_message(ctx, "intercepted request terminal", |message| {
        matches!(
            message["method"].as_str(),
            Some("Network.loadingFailed" | "Network.loadingFinished")
        ) && message["params"]["requestId"] == id
    })
    .await;
    let terminals = ctx
        .sent
        .iter()
        .filter(|message| {
            matches!(
                message["method"].as_str(),
                Some("Network.loadingFailed" | "Network.loadingFinished")
            ) && message["params"]["requestId"] == id
        })
        .collect::<Vec<_>>();
    assert_eq!(terminals.len(), 1, "{terminals:?}");
    assert_eq!(terminals[0]["method"], terminal);
}

#[tokio::test(flavor = "multi_thread")]
async fn replayed_upload_headers_follow_the_same_rules_for_fetch_and_native_xhr() {
    let fixture = Fixture::new().await;
    for api in APIS {
        for status in [301, 302, 303, 307, 308] {
            for repeat_method in [false, true] {
                let mut ctx = fixture.context(api).await;
                let mut original = None;
                let (result, pauses) = request(
                    &mut ctx,
                    api.expression(
                        &format!("{}/start/{status}", fixture.origin),
                        "POST",
                        json!({"Content-Type":"text/plain","X-Keep":"yes"}),
                    ),
                    |pause| {
                        if pause["params"]["request"]["url"]
                            .as_str()
                            .unwrap()
                            .contains("/start/")
                        {
                            original = Some(echo_headers(pause));
                            continue_request(pause, None, None)
                        } else {
                            continue_request(
                                pause,
                                original.clone(),
                                repeat_method.then(|| {
                                    pause["params"]["request"]["method"].as_str().unwrap()
                                }),
                            )
                        }
                    },
                )
                .await;
                assert_eq!(pauses.len(), 2, "{api:?} {status}");
                let result: Value = serde_json::from_str(result.as_str().unwrap()).unwrap();
                let keep_body = matches!(status, 307 | 308);
                assert_eq!(
                    result["method"],
                    if keep_body { "POST" } else { "GET" },
                    "{api:?} {status}"
                );
                assert_eq!(
                    result["contentType"],
                    if keep_body {
                        json!("text/plain")
                    } else {
                        Value::Null
                    },
                    "{api:?} {status}"
                );
                assert_eq!(
                    result["contentLength"],
                    if keep_body { json!("3") } else { Value::Null },
                    "{api:?} {status}"
                );
                assert_eq!(result["body"], if keep_body { "abc" } else { "" });
                assert_eq!(result["keep"], "yes");
                assert_eq!(
                    result["origin"],
                    if keep_body {
                        json!(fixture.origin)
                    } else {
                        Value::Null
                    }
                );
                assert_terminal(&mut ctx, &pauses, "Network.loadingFinished").await;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn redirected_fetch_and_xhr_accept_explicit_new_content_type() {
    let fixture = Fixture::new().await;
    for api in APIS {
        let mut ctx = fixture.context(api).await;
        let (result, pauses) = request(
            &mut ctx,
            api.expression(
                &format!("{}/start/302", fixture.origin),
                "POST",
                json!({"Content-Type":"text/plain"}),
            ),
            |pause| {
                let redirected = pause["params"]["request"]["method"] == "GET";
                continue_request(
                    pause,
                    redirected
                        .then(|| vec![json!({"name":"Content-Type","value":"application/x-new"})]),
                    None,
                )
            },
        )
        .await;
        let result: Value = serde_json::from_str(result.as_str().unwrap()).unwrap();
        assert_eq!(result["method"], "GET", "{api:?}");
        assert_eq!(result["contentType"], "application/x-new", "{api:?}");
        assert_eq!(result["body"], "");
        assert_terminal(&mut ctx, &pauses, "Network.loadingFinished").await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_and_native_xhr_project_origin_without_granting_forbidden_preflight_headers() {
    let fixture = Fixture::new().await;
    for api in APIS {
        for denied in [false, true] {
            let mut ctx = fixture.context(api).await;
            let mut headers = json!({"Content-Type":"text/plain","X-Keep":"yes"});
            if denied {
                headers["X-Denied"] = json!("no");
            }
            let (result, pauses) = request(
                &mut ctx,
                api.expression(&format!("{}/target", fixture.peer), "POST", headers),
                |pause| {
                    assert!(
                        pause["params"]["request"]["headers"]
                            .as_object()
                            .unwrap()
                            .iter()
                            .any(|(name, value)| name.eq_ignore_ascii_case("origin")
                                && value == &fixture.origin),
                        "{api:?}: {pause}"
                    );
                    continue_request(pause, Some(echo_headers(pause)), None)
                },
            )
            .await;
            assert_eq!(pauses.len(), 1);
            let records = fixture.requests.lock().clone();
            let preflight = records
                .iter()
                .find(|r| r["method"] == "OPTIONS")
                .expect("author X-Keep requires preflight");
            let names = preflight["preflight"]
                .as_str()
                .unwrap()
                .split(',')
                .map(str::trim)
                .collect::<Vec<_>>();
            assert!(names.contains(&"x-keep"));
            assert!(!names.contains(&"origin"), "{preflight}");
            if denied {
                assert!(
                    result.as_str().unwrap().starts_with("ERROR:"),
                    "{api:?}: {result}"
                );
                assert!(!records.iter().any(|r| r["method"] == "POST"));
            } else {
                let result: Value = serde_json::from_str(result.as_str().unwrap()).unwrap();
                assert_eq!(result["origin"], fixture.origin, "{api:?}");
                assert_eq!(result["body"], "abc");
            }
            assert_terminal(
                &mut ctx,
                &pauses,
                if denied {
                    "Network.loadingFailed"
                } else {
                    "Network.loadingFinished"
                },
            )
            .await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_and_native_xhr_authorization_override_stays_with_its_redirect_hop() {
    let fixture = Fixture::new().await;
    for api in APIS {
        let mut ctx = fixture.context(api).await;
        let (result, pauses) = request(
            &mut ctx,
            api.expression(
                &format!("{}/auth-start", fixture.origin),
                "GET",
                json!({"Authorization":"author-secret"}),
            ),
            |pause| {
                let redirected =
                    pause["params"]["request"]["url"] == format!("{}/auth-next", fixture.peer);
                continue_request(
                    pause,
                    redirected.then(|| {
                        vec![json!({"name":"Authorization","value":"interceptor-secret"})]
                    }),
                    None,
                )
            },
        )
        .await;
        assert_eq!(pauses.len(), 3);
        let result: Value = serde_json::from_str(result.as_str().unwrap()).unwrap();
        assert_eq!(result["authorization"], Value::Null, "{api:?}");
        let actual = {
            let records = fixture.requests.lock();
            records
                .iter()
                .filter(|r| r["method"] != "OPTIONS")
                .map(|r| r["authorization"].clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            actual,
            [
                json!("author-secret"),
                json!("interceptor-secret"),
                Value::Null
            ],
            "{api:?}"
        );
        assert_terminal(&mut ctx, &pauses, "Network.loadingFinished").await;
    }
}
