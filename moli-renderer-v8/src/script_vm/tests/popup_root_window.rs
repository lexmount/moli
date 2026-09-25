use super::*;

fn expected_location_surface(same_origin: bool, path: &str) -> serde_json::Value {
    let read = if same_origin { "ok" } else { "SecurityError" };
    let convert = if same_origin {
        "Error:1"
    } else {
        "SecurityError:0"
    };
    let mut expected = serde_json::json!({
        "path": if same_origin { path } else { "SecurityError" },
        "toString": read,
        "assign": convert,
        "replace": "Error:1",
        "hrefInvalidURL": "SyntaxError",
        "replaceInvalidURL": "SyntaxError",
        "borrowedGetter": read,
        "borrowedSetter": convert,
        "borrowedAssign": convert,
        "borrowedToString": read,
        "prototypeIsNull": !same_origin,
        "keys": if same_origin {
            vec!["ancestorOrigins", "assign", "hash", "host", "hostname", "href", "origin",
                 "pathname", "port", "protocol", "reload", "replace", "search", "toString", "valueOf"]
        } else { vec!["href", "replace", "then"] },
    });
    for key in [
        "href",
        "origin",
        "protocol",
        "host",
        "hostname",
        "port",
        "pathname",
        "search",
        "hash",
        "ancestorOrigins",
    ] {
        expected[format!("get:{key}")] = read.into();
        let readonly = matches!(key, "origin" | "ancestorOrigins");
        expected[format!("descriptor:{key}")] = match (same_origin, readonly, key) {
            (true, true, _) => "function:undefined",
            (true, false, _) => "function:function",
            (false, _, "href") => "undefined:function",
            (false, _, _) => "SecurityError",
        }
        .into();
        if !readonly {
            expected[format!("set:{key}")] = if key == "href" { "Error:1" } else { convert }.into();
        }
    }
    expected
}

#[tokio::test]
async fn popup_location_checks_actual_window_origins_for_properties_and_borrowed_functions() {
    const PROBE: &str = include_str!("../../../tests/fixtures/popup-location-origin.js");
    const A: &str = "www.example.test";
    const B: &str = "remote.example.test";
    for hosts in [
        vec![B, B],
        vec![A, B],
        vec![A, B, B],
        vec![A, B, A],
        vec![B, A, B],
        vec![B, B, A],
        vec![A, A, A],
        vec![A, B, A, B],
    ] {
        let bodies = (0..hosts.len()).map(|index| format!(
            "<!doctype html><body><script>{PROBE}\npopupLocationFrame({}, {index});</script>",
            serde_json::json!(hosts),
        )).collect();
        let server = StaticHttpServer::spawn_with_bodies(bodies).await;
        let loader = static_http_loader([server.resolve_entry(A), server.resolve_entry(B)]);
        let opener_url = server.url_for_host(A, "/opener");
        let popup_url = server.url_for_host(hosts[0], "/popup");
        let mut vm =
            new_storage_page_task_executor_test_vm_with_loader(opener_url.as_str(), &loader);
        vm.eval(&format!(
            "globalThis.popupRootResult = null; onmessage = event => popupRootResult = event.data; \
             globalThis.rootPopup = open({:?});",
            popup_url.as_str(),
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(popupRootResult !== null)",
            "true",
            &format!("popup Location origins: {hosts:?}"),
        )
        .await;
        let result: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(popupRootResult)").unwrap()).unwrap();
        let child = *hosts.last().unwrap();
        assert_eq!(
            result,
            serde_json::json!({
                "top": expected_location_surface(child == hosts[0], "/popup"),
                "self": expected_location_surface(true, &format!("/frame-{}", hosts.len() - 1)),
                "opener": expected_location_surface(child == A, "/opener"),
                "popup": {
                    "self": expected_location_surface(true, "/popup"),
                    "opener": expected_location_surface(hosts[0] == A, "/opener"),
                },
            }),
            "{hosts:?}"
        );
        vm.eval("rootPopup.close()").unwrap();
        assert_eq!(server.finish_targets().await.len(), hosts.len());
    }
}

#[tokio::test]
async fn popup_location_uses_popup_origin_when_its_opener_is_a_child_realm() {
    const PROBE: &str = include_str!("../../../tests/fixtures/popup-location-origin.js");
    let bodies = (0..2).map(|index| format!(
        "<!doctype html><body><script>{PROBE}\npopupLocationFrame(['remote.example.test','remote.example.test'], {index});</script>",
    )).collect();
    let server = StaticHttpServer::spawn_with_bodies(bodies).await;
    let loader = static_http_loader([server.resolve_entry("remote.example.test")]);
    let opener_url = server.url_for_host("www.example.test", "/opener");
    let popup_url = server.url_for_host("remote.example.test", "/popup");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(opener_url.as_str(), &loader);
    vm.eval(&format!(
        r#"
        globalThis.popupRootResult = null;
        globalThis.frame = document.createElement('iframe'); document.body.append(frame);
        frame.contentWindow.onmessage = event => popupRootResult = event.data;
        globalThis.rootPopup = frame.contentWindow.open({:?});
    "#,
        popup_url.as_str()
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(popupRootResult !== null)",
        "true",
        "child-owned popup Location",
    )
    .await;
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(popupRootResult)").unwrap()).unwrap();
    assert_eq!(
        result,
        serde_json::json!({
            "top": expected_location_surface(true, "/popup"),
            "self": expected_location_surface(true, "/frame-1"),
            "opener": expected_location_surface(false, "blank"),
            "popup": {
                "self": expected_location_surface(true, "/popup"),
                "opener": expected_location_surface(false, "blank"),
            },
        })
    );
    vm.eval("rootPopup.close()").unwrap();
    assert_eq!(server.finish_targets().await, ["/popup", "/frame-1"]);
}

#[tokio::test]
async fn popup_location_retains_origin_domain_and_local_window_after_navigation() {
    let body = "<!doctype html><script>onload = () => opener.postMessage(location.pathname, '*');</script>";
    let server = StaticHttpServer::spawn_with_bodies(vec![body.into(), body.into()]).await;
    let loader = static_http_loader([
        server.resolve_entry("www.example.test"),
        server.resolve_entry("remote.example.test"),
    ]);
    let opener_url = server.url_for_host("www.example.test", "/opener");
    let next_url = server.url_for_host("remote.example.test", "/next");
    let initial_url = server.url_for_host("www.example.test", "/initial");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(opener_url.as_str(), &loader);
    vm.eval(&format!(
        "globalThis.nextPath = null; onmessage = e => nextPath = e.data; globalThis.popup = open({:?});",
        initial_url.as_str(),
    )).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(nextPath)",
        "/initial",
        "initial popup Document",
    )
    .await;
    assert_eq!(
        vm.eval(
            r#"
        globalThis.oldLocation = popup.location;
        globalThis.oldGetter = Object.getOwnPropertyDescriptor(oldLocation, 'href').get;
        globalThis.oldAssign = oldLocation.assign;
        globalThis.read = callback => { try { return callback(); } catch (e) { return e.name; } };
        popup.document.domain = 'example.test';
        JSON.stringify([read(() => oldLocation.href), read(() => oldGetter.call(oldLocation))]);
    "#
        )
        .unwrap(),
        r#"["SecurityError","SecurityError"]"#
    );
    assert_eq!(
        vm.eval(
            r#"
        document.domain = 'example.test';
        JSON.stringify([oldLocation.href, oldGetter.call(oldLocation)]);
    "#
        )
        .unwrap(),
        serde_json::json!([initial_url.as_str(), initial_url.as_str()]).to_string()
    );
    vm.eval(&format!(
        "globalThis.nextPath = null; onmessage = e => nextPath = e.data; popup.location.href = {:?};",
        next_url.as_str(),
    )).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(nextPath)",
        "/next",
        "popup cross-origin navigation",
    )
    .await;
    assert_eq!(
        vm.eval(
            r#"
        let conversions = 0;
        const value = { toString() { ++conversions; return '/wrong'; } };
        JSON.stringify([
            oldLocation.href,
            oldGetter.call(oldLocation),
            read(() => popup.location.href),
            read(() => oldGetter.call(popup.location)),
            read(() => oldAssign.call(popup.location, value)),
            conversions,
            read(() => { oldLocation.href = '/retired'; return 'ok'; }),
        ]);
    "#
        )
        .unwrap(),
        r#"["about:blank","about:blank","SecurityError","SecurityError","SecurityError",0,"ok"]"#
    );
    vm.eval("popup.close()").unwrap();
    assert_eq!(server.finish_targets().await, ["/initial", "/next"]);
}

#[tokio::test]
async fn popup_root_window_preserves_nested_parent_top_identity_and_origin_checks() {
    const PROBE: &str = include_str!("../../../tests/fixtures/popup-root-window.js");
    const A: &str = "www.example.test";
    const B: &str = "remote.example.test";
    for hosts in [
        vec![B, B],
        vec![A, B],
        vec![A, B, B],
        vec![A, B, A],
        vec![B, A, B],
        vec![B, B, A],
        vec![A, A, A],
        vec![A, B, A, B],
    ] {
        let bodies = (0..hosts.len())
            .map(|index| {
                format!(
                    "<!doctype html><body><script>{PROBE}\npopupRootFrame({}, {index});</script>",
                    serde_json::json!(hosts),
                )
            })
            .collect();
        let server = StaticHttpServer::spawn_with_bodies(bodies).await;
        let loader = static_http_loader([server.resolve_entry(A), server.resolve_entry(B)]);
        let opener_url = server.url_for_host(A, "/opener");
        let popup_url = server.url_for_host(hosts[0], "/popup");
        let mut vm =
            new_storage_page_task_executor_test_vm_with_loader(opener_url.as_str(), &loader);
        vm.eval(&format!(
            "globalThis.popupRootResult = null;\n\
             onmessage = event => popupRootResult = event.data;\n\
             globalThis.rootPopup = open({:?});",
            popup_url.as_str(),
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(popupRootResult !== null)",
            "true",
            &format!("nested popup roots: {hosts:?}"),
        )
        .await;
        let result: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(popupRootResult)").unwrap()).unwrap();
        let child = hosts.last().unwrap();
        let parent_path = if hosts.len() == 2 {
            "/popup".to_owned()
        } else {
            format!("/frame-{}", hosts.len() - 2)
        };
        let top_path = if *child == hosts[0] {
            "/popup"
        } else {
            "SecurityError"
        };
        assert_eq!(
            result,
            serde_json::json!({
                "topHasOpener": true,
                "topIsParent": hosts.len() == 2,
                "parentTopIsTop": true,
                "topParentIsTop": true,
                "topSelfIsTop": true,
                "topDocumentPath": top_path,
                "parentDocumentPath": if *child == hosts[hosts.len() - 2] {
                    parent_path.as_str()
                } else {
                    "SecurityError"
                },
                "openerDocumentPath": if *child == A { "/opener" } else { "SecurityError" },
            }),
            "{hosts:?}",
        );
        vm.eval("rootPopup.close()").unwrap();
        assert_eq!(server.finish_targets().await.len(), hosts.len());
    }
}
