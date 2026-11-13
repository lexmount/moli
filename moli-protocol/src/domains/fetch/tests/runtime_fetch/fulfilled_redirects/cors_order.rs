use super::super::native_xhr::run_cdp_command_with_network_decisions;
use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn error_mode_checks_cors_before_redirect_rejection() {
    let (mut ctx, _, page_server) = fixture().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().fallback(any(
                |method: axum::http::Method, uri: axum::http::Uri| async move {
                    let options = method == axum::http::Method::OPTIONS;
                    let mut response = axum::http::Response::builder()
                        .status(if options {
                            204
                        } else {
                            uri.path()[1..].parse::<u16>().unwrap()
                        })
                        .header("Content-Type", "text/plain")
                        .header("Access-Control-Allow-Methods", "GET")
                        .header("Access-Control-Allow-Headers", "X-Probe");
                    if !options && uri.query().unwrap().contains("location=1") {
                        response = response.header("Location", "/unused-target");
                    }
                    if options || uri.query().unwrap().contains("cors=1") {
                        response = response.header("Access-Control-Allow-Origin", "*");
                    }
                    response
                        .body(axum::body::Body::from(if options { "" } else { "body" }))
                        .unwrap()
                },
            )),
        )
        .await
        .unwrap();
    });
    let mut index = 0;
    for stage in [None, Some("Request"), Some("Response")] {
        command(&mut ctx, 52_000, "Fetch.disable", json!({})).await;
        if let Some(stage) = stage {
            command(
                &mut ctx,
                52_001,
                "Fetch.enable",
                json!({"patterns":[{
                    "urlPattern":"*", "resourceType":"Fetch", "requestStage":stage
                }]}),
            )
            .await;
        }
        for status in [301, 302, 303, 307, 308] {
            for cors in [false, true] {
                for location in [false, true] {
                    let id = 52_010 + index * 2;
                    index += 1;
                    // A response-stage replacement starts with a permitted network
                    // response, then supplies the headers being tested.
                    let wire_cors = cors || stage == Some("Response");
                    let url = format!(
                        "{origin}/{status}?cors={}&location={}&case={index}",
                        u8::from(wire_cors),
                        u8::from(location)
                    );
                    ctx.sent.clear();
                    let mut response_headers = if cors {
                        vec![json!({"name":"Access-Control-Allow-Origin","value":"*"})]
                    } else {
                        vec![]
                    };
                    if location {
                        response_headers.push(json!({"name":"Location","value":"/unused-target"}));
                    }
                    let pauses = run_cdp_command_with_network_decisions(
                    &mut ctx,
                    json!({"id":id,"sessionId":"SID-1","method":"Runtime.evaluate","params":{
                        "expression":format!("fetch({url:?},{{redirect:'error',headers:{{'X-Probe':'yes'}}}}).then(r=>({{status:r.status}}),e=>({{error:e.name,message:e.message}}))"),
                        "awaitPromise":true,"returnByValue":true
                    }}),
                    |pause| json!({"id":id+1,"sessionId":"SID-1","method":"Fetch.fulfillRequest","params":{
                        "requestId":pause["params"]["requestId"],"responseCode":status,"body":"Ym9keQ==",
                        "responseHeaders":response_headers.clone()
                    }}),
                ).await;
                    let response = take_response_by_id(&mut ctx, id);
                    let value = &response["result"]["result"]["value"];
                    if cors && stage.is_some() && !location {
                        assert_eq!(
                            value["status"], status,
                            "{stage:?}/{status}/location={location}: {value}"
                        );
                    } else {
                        assert_eq!(
                            value["error"], "TypeError",
                            "{stage:?}/{status}/{cors}/location={location}: {response}"
                        );
                        let message = value["message"].as_str().unwrap();
                        assert!(
                            if cors {
                                message.contains("redirect")
                            } else {
                                message.contains("CORS")
                            },
                            "{stage:?}/{status}/{cors}/location={location}: {value}"
                        );
                    }
                    assert_eq!(pauses.len(), usize::from(stage.is_some()));
                    let network_id = ctx
                        .sent
                        .iter()
                        .find(|event| {
                            event["method"] == "Network.requestWillBeSent"
                                && event["params"]["request"]["url"] == url
                                && event["params"]["request"]["method"] == "GET"
                        })
                        .expect("announced fetch request")["params"]["requestId"]
                        .clone();
                    wait_until_scheduler_message(&mut ctx, "CORS/error-mode terminal", |event| {
                        event["method"]
                            == if cors && stage.is_some() && !location {
                                "Network.loadingFinished"
                            } else {
                                "Network.loadingFailed"
                            }
                            && event["params"]["requestId"] == network_id
                    })
                    .await;
                    let terminals = ctx
                        .sent
                        .iter()
                        .filter(|event| {
                            matches!(
                                event["method"].as_str(),
                                Some("Network.loadingFailed" | "Network.loadingFinished")
                            ) && event["params"]["requestId"] == network_id
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(terminals.len(), 1, "{terminals:?}");
                    if !cors {
                        assert_eq!(terminals[0]["params"]["errorText"], "net::ERR_FAILED");
                    }
                }
            }
        }
    }
    server.abort();
    page_server.abort();
}
