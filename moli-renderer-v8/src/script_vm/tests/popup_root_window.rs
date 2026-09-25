use super::*;

#[tokio::test]
async fn initial_popup_aliases_its_creators_document_domain_in_both_directions() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://www.example.test/opener",
        &loader,
    );
    assert_eq!(
        vm.eval(
            r#"
            const popup = open();
            popup.document.body.textContent = 'popup';
            const initialDomain = popup.document.domain;
            const frame = document.createElement('iframe');
            document.body.append(frame);
            const childPopup = frame.contentWindow.open();
            document.domain = document.domain;
            const before = [popup.document.domain, childPopup.document.domain];
            popup.document.domain = 'example.test';
            JSON.stringify([initialDomain, ...before, document.domain, frame.contentWindow.document.domain,
                popup.document.domain, childPopup.document.domain, popup.document.body.textContent]);
            "#,
        ).unwrap(),
        r#"["www.example.test","www.example.test","www.example.test","example.test","example.test","example.test","example.test","popup"]"#
    );
}

#[tokio::test]
async fn popup_window_promise_reactions_keep_their_origin_without_granting_it_to_the_opener() {
    const SOURCE: &str = include_str!("../../../tests/fixtures/popup-window-async.js");
    let server = StaticHttpServer::spawn_with_bodies(vec![format!(
        "<!doctype html><title>popup</title><script>{SOURCE}</script>"
    )])
    .await;
    let loader = static_http_loader([
        server.resolve_entry("www.example.test"),
        server.resolve_entry("remote.example.test"),
    ]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        server.url_for_host("www.example.test", "/opener").as_str(),
        &loader,
    );
    vm.eval(&format!(
        r#"
        globalThis.popupMessages = [];
        onmessage = event => {{
            popupMessages.push(event.data);
            Promise.resolve().then(() => {{
                let result;
                try {{ result = popup.document.title; }} catch (error) {{ result = error.name; }}
                popupMessages.push({{kind: "main-" + event.data.kind, result}});
            }});
        }};
        globalThis.popup = open({:?}); true;
    "#,
        server
            .url_for_host("remote.example.test", "/popup")
            .as_str()
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(popupMessages.length)",
        "6",
        "popup Promise reactions",
    )
    .await;
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify(popupMessages.sort((a, b) => a.kind.localeCompare(b.kind)))")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        result,
        serde_json::json!([
            {"kind":"await", "title":"popup"},
            {"kind":"catch", "title":"popup"},
            {"kind":"main-await", "result":"SecurityError"},
            {"kind":"main-catch", "result":"SecurityError"},
            {"kind":"main-then", "result":"SecurityError"},
            {"kind":"then", "title":"popup", "self":true},
        ])
    );

    let popup_id = vm._context_host.borrow().open_lightweight_popup_ids()[0];
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        fn run_body(scope: &mut v8::PinScope<'_, '_>, source: &str) {
            let source = crate::util::v8_string(scope, source).unwrap();
            let script = v8::Script::compile(scope, source, None).unwrap();
            crate::script_execution::execute_compiled_script(scope, script).unwrap();
        }
        run_body(
            scope,
            r#"
            globalThis.retainedPopupReaction = []; globalThis.newWindowReaction = [];
            globalThis.popupNavigationError = null;
            Promise.resolve().then(() => {
                popup.location.href = "about:blank";
            }).catch(error => { popupNavigationError = error.name; });
            "#,
        );
        let owner = crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id);
        let previous = owner.enter(scope);
        // Keep this job queued while a new LocalWindow commits. Its origin
        // must remain that of the old popup, even after the proxy becomes
        // same-origin with the opener.
        run_body(
            scope,
            r#"Promise.resolve().then(() => {
            try { retainedPopupReaction.push(popup.document.URL); }
            catch (error) { retainedPopupReaction.push(error.name); }
        });"#,
        );
        owner.restore(scope, previous);
        run_body(
            scope,
            r#"
            Promise.resolve().then(() => newWindowReaction.push(popup.document.URL));
        "#,
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([retainedPopupReaction, newWindowReaction, popupNavigationError])")
            .unwrap(),
        r#"[["SecurityError"],["about:blank"],null]"#
    );
    assert_eq!(server.finish_targets().await.len(), 1);
}

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

#[tokio::test]
async fn popup_window_keeps_canonical_identity_and_cross_origin_access_boundaries() {
    const PROBE: &str = include_str!("../../../tests/fixtures/popup-window-access.js");
    const A: &str = "www.example.test";
    const B: &str = "remote.example.test";
    for has_same_origin_child in [false, true] {
        let mut bodies = vec![format!(
            r#"<!doctype html><body><iframe name="related-target"></iframe><script>
            if ({has_same_origin_child}) {{
                const child = document.createElement('iframe');
                const url = new URL('/child', location.href);
                url.hostname = '{A}';
                child.name = 'related-target'; child.src = url.href;
                document.body.append(child);
            }}
            onload = () => opener.postMessage('ready', '*');
            </script>"#,
        )];
        if has_same_origin_child {
            bodies.push("<!doctype html><body>child".to_owned());
        }
        let server = StaticHttpServer::spawn_with_bodies(bodies).await;
        let loader = static_http_loader([server.resolve_entry(A), server.resolve_entry(B)]);
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(
            server.url_for_host(A, "/opener").as_str(),
            &loader,
        );
        assert_eq!(
            vm.eval(&format!(
                "globalThis.ready = false; onmessage = event => ready = event.data === 'ready'; \
             globalThis.rootPopup = open({:?}, 'related-page'); String(rootPopup.document.URL);",
                server.url_for_host(B, "/popup").as_str(),
            ))
            .unwrap(),
            "about:blank"
        );
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(ready)",
            "true",
            "cross-origin popup projection",
        )
        .await;
        vm.eval(PROBE).unwrap();
        let child_index = usize::from(has_same_origin_child);
        let actual: serde_json::Value = serde_json::from_str(
            &vm.eval(&format!(
                "JSON.stringify(inspectPopupWindow(rootPopup, {child_index}))",
            ))
            .unwrap(),
        )
        .unwrap();
        let expected: Vec<_> = ["popup", "parent", "top"].map(|path| serde_json::json!({
            "path": path, "identity": true,
            "document": "SecurityError", "name": "SecurityError", "location": "SecurityError",
            "expando": "SecurityError", "descriptor": "SecurityError", "set": "SecurityError",
            "define": "SecurityError", "has": "SecurityError", "del": "SecurityError",
            "closed": false, "self": true, "roots": true, "length": child_index + 1,
            "opener": true, "postMessage": "function", "postMessageStable": true,
            "postMessageRealm": true, "close": "function", "prototypeIsNull": true,
            "then": "undefined", "iterator": "SecurityError",
        })).into();
        assert_eq!(
            actual,
            serde_json::json!(expected),
            "same-origin child: {has_same_origin_child}"
        );
        assert_eq!(
            vm.eval("rootPopup.close(); String(rootPopup.closed)")
                .unwrap(),
            "true"
        );
        assert_eq!(server.finish_targets().await.len(), child_index + 1);
    }
}

#[tokio::test]
async fn popup_window_rechecks_borrowed_accessors_after_navigation_and_close() {
    let server = StaticHttpServer::spawn_with_bodies(vec![
        "<!doctype html><script>onload = () => opener.postMessage('ready', '*')</script>"
            .to_owned(),
    ])
    .await;
    let loader = static_http_loader([server.resolve_entry("remote.example.test")]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        server.url_for_host("www.example.test", "/opener").as_str(),
        &loader,
    );
    assert_eq!(
        vm.eval(&format!(
            r#"
        globalThis.ready = false; onmessage = event => ready = event.data === 'ready';
        globalThis.popup = open({:?}, 'popup-transition');
        globalThis.originalPopup = popup;
        globalThis.originalName = Object.getOwnPropertyDescriptor(popup, 'name');
        globalThis.originalOpener = Object.getOwnPropertyDescriptor(popup, 'opener');
        popup.document.URL;
    "#,
            server
                .url_for_host("remote.example.test", "/popup")
                .as_str()
        ))
        .unwrap(),
        "about:blank"
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(ready)",
        "true",
        "popup cross-origin navigation",
    )
    .await;
    assert_eq!(
        vm.eval(
            r#"(() => {
        const read = f => { try { return f(); } catch (error) { return error.name; } };
        let conversions = 0;
        const value = {toString() { conversions++; return 'renamed'; }};
        globalThis.foreignLocation = popup.location;
        return JSON.stringify([
            popup === originalPopup,
            read(() => popup.document.URL),
            read(() => originalName.get.call(popup)),
            read(() => originalName.set.call(popup, value)),
            conversions,
            read(() => originalOpener.set.call(popup, null)),
            popup.opener === window,
        ]);
    })()"#
        )
        .unwrap(),
        r#"[true,"SecurityError","SecurityError","SecurityError",0,"SecurityError",true]"#
    );
    vm.eval("popup.location = 'about:blank'; true").unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm, &loader,
        "(() => { try { return String(popup.document.URL === 'about:blank'); } catch (_) { return 'false'; } })()",
        "true", "popup returns to same origin",
    ).await;
    assert_eq!(
        vm.eval(
            r#"JSON.stringify([
        popup === originalPopup, popup.document.URL,
        originalName.get.call(popup), popup.location !== foreignLocation,
    ])"#
        )
        .unwrap(),
        r#"[true,"about:blank","popup-transition",true]"#
    );
    vm.eval("popup.close(); true").unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(popup.closed && popup.parent === null && popup.top === null)",
        "true",
        "closed popup browsing context",
    )
    .await;
    assert_eq!(vm.eval("JSON.stringify([popup === originalPopup, popup.closed, popup.length, popup.self === popup])").unwrap(), "[true,true,0,true]");
    assert_eq!(server.finish_targets().await.len(), 1);
}
