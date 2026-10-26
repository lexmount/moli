use super::*;

async fn check_native_xhr_cases(url: &str, flows: &[&str]) {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_page_task_executor_test_vm_with_loader(url, &loader);
    vm.eval(include_str!("native_events.js")).unwrap();
    for receiver in 0..2 {
        for methods in 0..2 {
            for registration in 0..2 {
                for tamper in ["none", "getter", "constructor"] {
                    for flow in flows {
                        let request_url = match *flow {
                            "error" => "invalid-protocol://xhr-native.test/failure",
                            "post" => url,
                            _ => "data:text/plain,ok",
                        };
                        let config = serde_json::json!({
                            "receiver": receiver, "methods": methods, "registration": registration,
                            "tamper": tamper, "flow": flow, "url": request_url,
                        });
                        vm.eval(&format!(
                            "globalThis.result='pending'; checkNativeXhrEvents({config}).then(()=>{{result='ok'}},error=>{{result=String(error.stack||error)}})"
                        ))
                        .unwrap();
                        advance_page_task_executor_until_eval_equals(
                            &mut vm,
                            &loader,
                            "String(result!=='pending')",
                            "true",
                            "XHR native event completion",
                        )
                        .await;
                        assert_eq!(vm.eval("result").unwrap(), "ok", "{config}");
                    }
                }
            }
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn xhr_native_events_preserve_brands_trust_and_receiver_realm_without_author_constructors() {
    check_native_xhr_cases("https://xhr-native.test/", &["data", "error", "abort"]).await;
}

#[tokio::test(flavor = "current_thread")]
async fn xhr_native_events_include_upload_progress_in_receiver_realm() {
    let server = StaticHttpServer::spawn(24).await;
    check_native_xhr_cases(server.base_url().as_str(), &["post"]).await;
    let requests = server.finish().await;
    assert_eq!(requests.len(), 24);
    for request in requests {
        assert_eq!(request.method, "POST");
        assert_eq!(request.body, b"payload");
    }
}
