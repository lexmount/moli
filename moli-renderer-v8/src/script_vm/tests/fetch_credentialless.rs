use super::*;
use crate::cross_origin_isolation::{CrossOriginEmbedderPolicy, DocumentIsolationPolicy};
use parking_lot::Mutex;
use std::collections::BTreeSet;
use std::sync::Arc;

fn credentialless_response(
    request: &http_fixture::CapturedHttpRequest,
    origins: &Mutex<(String, String)>,
) -> String {
    let origins = origins.lock();
    let path = request.target.as_str();
    let location = match path {
        "/same-to-cross" => Some(format!("{}/cross-final", origins.1)),
        "/cross-to-same" => Some(format!("{}/same-final", origins.0)),
        "/round-trip" => Some(format!("{}/return", origins.1)),
        "/return" => Some(format!("{}/round-final", origins.0)),
        _ => None,
    };
    let cookie = path.trim_start_matches('/').replace('-', "_");
    let mut response = format!(
        "HTTP/1.1 {}\r\nContent-Type: image/gif\r\nCache-Control: no-store\r\nSet-Cookie: seen_{cookie}=1; Path=/; SameSite=Lax\r\nAccess-Control-Allow-Origin: {}\r\nAccess-Control-Allow-Credentials: true\r\nConnection: close\r\n",
        if location.is_some() {
            "302 Found"
        } else {
            "200 OK"
        },
        origins.0,
    );
    if path != "/no-corp" {
        response.push_str("Cross-Origin-Resource-Policy: cross-origin\r\n");
    }
    if let Some(location) = location {
        response.push_str(&format!("Location: {location}\r\n"));
    }
    response.push_str("Content-Length: 6\r\n\r\nGIF89a");
    response
}

#[tokio::test]
async fn window_fetch_credentialless_credentials_follow_redirect_taint() {
    let mut failures = Vec::new();
    for (name, coep, dip, credentialless) in [
        (
            "none",
            CrossOriginEmbedderPolicy::None,
            DocumentIsolationPolicy::None,
            false,
        ),
        (
            "require-corp",
            CrossOriginEmbedderPolicy::RequireCorp,
            DocumentIsolationPolicy::None,
            false,
        ),
        (
            "credentialless",
            CrossOriginEmbedderPolicy::Credentialless,
            DocumentIsolationPolicy::None,
            true,
        ),
        (
            "isolate-and-credentialless",
            CrossOriginEmbedderPolicy::None,
            DocumentIsolationPolicy::IsolateAndCredentialless,
            true,
        ),
    ] {
        let origins = Arc::new(Mutex::new((String::new(), String::new())));
        let same_origins = origins.clone();
        let same = StaticHttpServer::spawn_with_raw_responder(6, move |_, request| {
            credentialless_response(request, &same_origins)
        })
        .await;
        let cross_origins = origins.clone();
        let cross = StaticHttpServer::spawn_with_raw_responder(7, move |_, request| {
            credentialless_response(request, &cross_origins)
        })
        .await;
        let same_origin = same.base_url().origin().ascii_serialization();
        let cross_origin = cross.base_url().origin().ascii_serialization();
        *origins.lock() = (same_origin.clone(), cross_origin.clone());
        let loader = static_http_loader([]);
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(
            &format!("{same_origin}/page.html"),
            &loader,
        );
        vm.set_main_navigation_policy_container(crate::document_runtime::DocumentPolicyContainer {
            cross_origin_embedder_policy: coep,
            document_isolation_policy: dip,
            ..Default::default()
        });
        let cookies = vm
            .current_main_document_resource_loader()
            .unwrap()
            .request_client()
            .cookie_store();
        cookies
            .lock()
            .unwrap()
            .set_document_cookie(&same.base_url(), "seed=1; Path=/; SameSite=Lax");
        // The same probe exercises Request metadata and redirects in Window and Worker realms.
        vm.eval(&format!(
            "{}\nglobalThis.credentiallessResult = null; workerCredentiallessFetchProbe({same_origin:?}, {cross_origin:?}).then(result => {{ credentiallessResult = result; }}, error => {{ credentiallessResult = {{error:String(error)}}; }});",
            include_str!("../../../tests/fixtures/worker-credentialless-fetch.js"),
        )).unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(credentiallessResult !== null)",
            "true",
            "credentialless Window requests should finish",
        )
        .await;
        let payload: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(credentiallessResult)").unwrap())
                .unwrap();
        assert_eq!(payload["isolated"], false, "{name}");
        let results = payload["results"].as_array().unwrap();
        assert_eq!(results.len(), 9, "{name}: {payload}");
        for (index, result) in results.iter().enumerate() {
            assert_eq!(
                result["credentials"],
                if index < 7 { "include" } else { "omit" }
            );
            assert_eq!(
                result["mode"],
                if matches!(index, 2 | 8) {
                    "cors"
                } else {
                    "no-cors"
                }
            );
            let expected = if index == 6 && coep == CrossOriginEmbedderPolicy::RequireCorp {
                serde_json::json!({"error": "TypeError"})
            } else if matches!(index, 0 | 7) {
                serde_json::json!({"type": "basic", "status": 200})
            } else if matches!(index, 2 | 8) {
                serde_json::json!({"type": "cors", "status": 200})
            } else {
                serde_json::json!({"type": "opaque", "status": 0})
            };
            for (field, expected) in expected.as_object().unwrap() {
                if &result[field] != expected {
                    failures.push(format!("{name}, case {index}, {field}: {result}"));
                }
            }
        }
        let mut requests = same.finish().await;
        requests.extend(cross.finish().await);
        let mut expected_cookies = BTreeSet::from(["seed".to_owned()]);
        for request in requests {
            let path = request.target.as_str();
            let allowed = match path {
                "/same-omit" | "/cors-omit" => false,
                "/same" | "/cors" | "/same-to-cross" | "/round-trip" => true,
                _ => !credentialless,
            };
            let has_cookie = request
                .header_value("cookie")
                .is_some_and(|value| value.split(';').any(|cookie| cookie.trim() == "seed=1"));
            if has_cookie != allowed {
                failures.push(format!(
                    "{name}, {path}: expected Cookie={allowed}, got {has_cookie}"
                ));
            }
            if allowed {
                expected_cookies.insert(format!("seen_{}", path[1..].replace('-', "_")));
            }
        }
        let actual_cookies = cookies
            .lock()
            .unwrap()
            .cookies()
            .into_iter()
            .map(|cookie| cookie.name)
            .collect::<BTreeSet<_>>();
        if actual_cookies != expected_cookies {
            failures.push(format!(
                "{name}: cookies {actual_cookies:?}, expected {expected_cookies:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
