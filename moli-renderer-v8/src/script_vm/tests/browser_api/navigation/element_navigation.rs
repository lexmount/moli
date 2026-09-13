use super::*;

#[test]
fn anchor_navigation_fires_once_when_click_is_borrowed_from_child_realm() {
    for target in ["", "_sElF", "_ToP", "_PARENT"] {
        for use_child_realm in [true, false] {
            for cancel_top in [false, true] {
                let mut vm = new_parsed_test_vm(
                    "https://anchor-navigation-target.test/source",
                    "<!doctype html><body></body>",
                );
                let description = format!(
                    "target={target:?} child_realm={use_child_realm} cancel_top={cancel_top}"
                );
                let result = vm
                    .eval(&format!(
                        r#"
                        (() => {{
                            const seen = [];
                            const frame = document.createElement('iframe');
                            document.body.appendChild(frame);
                            frame.srcdoc = '<body></body>';
                            frame.contentWindow.navigation.onnavigate = event => {{
                                seen.push('child');
                                event.preventDefault();
                            }};
                            const anchor = document.createElement('a');
                            anchor.href = 'https://anchor-navigation-target.test/destination';
                            anchor.target = {target:?};
                            document.body.appendChild(anchor);
                            navigation.onnavigate = event => {{
                                seen.push([
                                    'top',
                                    event.target === navigation,
                                    event.currentTarget === navigation,
                                    event.sourceElement === anchor,
                                    event.destination.url
                                ]);
                                event.signal.onabort = () => seen.push('top-abort');
                                if ({cancel_top}) event.preventDefault();
                            }};
                            navigation.onnavigateerror = () => seen.push('top-error');
                            const realm = {use_child_realm} ? frame.contentWindow : window;
                            if ({use_child_realm} && realm.HTMLElement === HTMLElement)
                                throw new Error('click must use the child realm');
                            realm.HTMLElement.prototype.click.call(anchor);
                            return JSON.stringify(seen);
                        }})()
                        "#,
                    ))
                    .expect(&description);
                let expected_url = "https://anchor-navigation-target.test/destination";
                let mut expected = vec![serde_json::json!(["top", true, true, true, expected_url])];
                if cancel_top {
                    expected.extend([
                        serde_json::json!("top-abort"),
                        serde_json::json!("top-error"),
                    ]);
                }
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&result).unwrap(),
                    serde_json::json!(expected),
                    "{description}"
                );
                let pending = vm.take_pending_location_navigation_with_seed();
                if cancel_top {
                    assert!(pending.is_none(), "{description}");
                } else {
                    assert_eq!(
                        pending.expect(&description).url.as_str(),
                        expected_url,
                        "{description}"
                    );
                }
            }
        }
    }
}

#[test]
fn child_navigation_to_top_fires_once_and_honors_cancel_or_intercept() {
    for (depth, target) in [(1, "_top"), (1, "_PARENT"), (2, "_ToP")] {
        for api in ["click", "open"] {
            for use_child_realm in [true, false] {
                for action in ["allow", "cancel", "intercept"] {
                    let mut vm = new_parsed_test_vm(
                        "https://anchor-navigation-target.test/source",
                        "<!doctype html><body></body>",
                    );
                    let description = format!(
                        "depth={depth} target={target} api={api} child_realm={use_child_realm} action={action}"
                    );
                    let result = vm
                    .eval(&format!(
                        r#"
                        (() => {{
                            const seen = globalThis.__lmTargetNavigationLog = [];
                            const from = navigation.currentEntry;
                            let source = document;
                            for (let i = 0; i < {depth}; ++i) {{
                                const frame = source.createElement('iframe');
                                source.body.appendChild(frame);
                                frame.srcdoc = '<body></body>';
                                source = frame.contentDocument;
                                frame.contentWindow.navigation.onnavigate = event => {{
                                    seen.push('child:' + i);
                                    event.preventDefault();
                                }};
                            }}
                            const anchor = source.createElement('a');
                            anchor.href = 'https://anchor-navigation-target.test/destination';
                            anchor.target = {target:?};
                            source.body.appendChild(anchor);
                            const initialHistoryLength = history.length;
                            navigation.onnavigate = event => {{
                                seen.push([
                                    'top',
                                    event instanceof NavigateEvent,
                                    event.target === navigation,
                                    event.currentTarget === navigation,
                                    event.sourceElement === ({api:?} === 'click' ? anchor : null),
                                    event.canIntercept,
                                    event.destination.url
                                ]);
                                event.signal.onabort = () => seen.push('top-abort');
                                if ({action:?} === 'cancel') event.preventDefault();
                                if ({action:?} === 'intercept') event.intercept({{
                                    handler() {{
                                        seen.push([
                                            'handler',
                                            location.href,
                                            navigation.currentEntry.url,
                                            navigation.transition instanceof NavigationTransition,
                                            navigation.transition.from === from,
                                            navigation.transition.navigationType,
                                            history.length === initialHistoryLength + 1
                                        ]);
                                        return Promise.resolve().then(() => seen.push('handler-complete'));
                                    }}
                                }});
                            }};
                            navigation.onnavigateerror = () => seen.push('top-error');
                            navigation.onnavigatesuccess = () => seen.push('top-success');
                            const realm = {use_child_realm} ? source.defaultView : window;
                            if ({api:?} === 'click') realm.HTMLElement.prototype.click.call(anchor);
                            else realm.open.call(source.defaultView, anchor.href, anchor.target);
                            return JSON.stringify(seen);
                        }})()
                        "#,
                    ))
                    .expect(&description);
                    let expected_url = "https://anchor-navigation-target.test/destination";
                    let mut expected = vec![serde_json::json!([
                        "top",
                        true,
                        true,
                        true,
                        true,
                        true,
                        expected_url
                    ])];
                    if action == "cancel" {
                        expected.extend([
                            serde_json::json!("top-abort"),
                            serde_json::json!("top-error"),
                        ]);
                    } else if action == "intercept" {
                        expected.push(serde_json::json!([
                            "handler",
                            expected_url,
                            expected_url,
                            true,
                            true,
                            "push",
                            true
                        ]));
                    }
                    assert_eq!(
                        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
                        serde_json::json!(expected),
                        "{description}"
                    );
                    if action == "intercept" {
                        expected.extend([
                            serde_json::json!("handler-complete"),
                            serde_json::json!("top-success"),
                        ]);
                    }
                    if action != "allow" {
                        let settled = vm
                            .eval(
                                "JSON.stringify({ log: globalThis.__lmTargetNavigationLog, \
                                 href: location.href, entryUrl: navigation.currentEntry.url, \
                                 transition: navigation.transition })",
                            )
                            .expect(&description);
                        let committed_url = if action == "intercept" {
                            expected_url
                        } else {
                            "https://anchor-navigation-target.test/source"
                        };
                        assert_eq!(
                            serde_json::from_str::<serde_json::Value>(&settled).unwrap(),
                            serde_json::json!({
                                "log": expected,
                                "href": committed_url,
                                "entryUrl": committed_url,
                                "transition": null
                            }),
                            "{description}"
                        );
                    }
                    let pending = vm.take_pending_location_navigation_with_seed();
                    if action == "allow" {
                        assert_eq!(
                            pending.expect(&description).url.as_str(),
                            expected_url,
                            "{description}"
                        );
                    } else {
                        assert!(pending.is_none(), "{description}");
                    }
                }
            }
        }
    }
}

#[test]
fn hyperlink_top_navigation_checks_source_document_sandbox_across_realms() {
    for (depth, target) in [(1, "_top"), (1, "_PARENT"), (2, "_ToP")] {
        for source_is_child in [true, false] {
            for allow_top_navigation in [false, true] {
                for use_child_realm in [true, false] {
                    for intercept in [false, true] {
                        let mut vm = new_parsed_test_vm(
                            "https://anchor-navigation-sandbox.test/source",
                            "<!doctype html><body></body>",
                        );
                        let description = format!(
                            "depth={depth} target={target} source_is_child={source_is_child} \
                             allow_top_navigation={allow_top_navigation} \
                             child_realm={use_child_realm} intercept={intercept}"
                        );
                        vm.eval(&format!(
                            r#"
                            (() => {{
                                const log = [];
                                let childDocument = document;
                                for (let i = 0; i < {depth}; ++i) {{
                                    const frame = childDocument.createElement('iframe');
                                    frame.sandbox = 'allow-scripts allow-same-origin' +
                                        ({allow_top_navigation} ? ' allow-top-navigation' : '');
                                    frame.srcdoc = '<body></body>';
                                    childDocument.body.appendChild(frame);
                                    childDocument = frame.contentDocument;
                                    frame.contentWindow.navigation.onnavigate = () => log.push('child');
                                }}
                                const source = {source_is_child} ? childDocument : document;
                                const anchor = source.createElement('a');
                                anchor.href = 'https://anchor-navigation-sandbox.test/destination';
                                anchor.target = {target:?};
                                source.body.appendChild(anchor);
                                const from = navigation.currentEntry;
                                const initialHistoryLength = history.length;
                                navigation.onnavigate = event => {{
                                    log.push('navigate');
                                    if ({intercept}) event.intercept({{
                                        handler() {{
                                            log.push([
                                                'handler',
                                                location.href,
                                                navigation.currentEntry.url,
                                                navigation.transition instanceof NavigationTransition,
                                                navigation.transition.from === from,
                                                navigation.transition.navigationType
                                            ]);
                                            return Promise.resolve().then(() => log.push('handler-complete'));
                                        }}
                                    }});
                                }};
                                navigation.oncurrententrychange = () => log.push('currententrychange');
                                navigation.onnavigatesuccess = () => log.push('success');
                                navigation.onnavigateerror = () => log.push('error');
                                globalThis.__lmSandboxLinkSnapshot = () => ({{
                                    log,
                                    href: location.href,
                                    entryUrl: navigation.currentEntry.url,
                                    entryUnchanged: navigation.currentEntry === from,
                                    historyDelta: history.length - initialHistoryLength,
                                    transition: navigation.transition
                                }});
                                const realm = {use_child_realm} ? childDocument.defaultView : window;
                                realm.HTMLElement.prototype.click.call(anchor);
                            }})()
                            "#,
                        ))
                        .expect(&description);
                        let allowed = !source_is_child || allow_top_navigation;
                        let committed = allowed && intercept;
                        let expected_url = "https://anchor-navigation-sandbox.test/destination";
                        let mut expected_log = Vec::new();
                        if allowed {
                            expected_log.push(serde_json::json!("navigate"));
                        }
                        if committed {
                            expected_log.extend([
                                serde_json::json!("currententrychange"),
                                serde_json::json!([
                                    "handler",
                                    expected_url,
                                    expected_url,
                                    true,
                                    true,
                                    "push"
                                ]),
                                serde_json::json!("handler-complete"),
                                serde_json::json!("success"),
                            ]);
                        }
                        let committed_url = if committed {
                            expected_url
                        } else {
                            "https://anchor-navigation-sandbox.test/source"
                        };
                        let result = vm
                            .eval("JSON.stringify(__lmSandboxLinkSnapshot())")
                            .expect(&description);
                        assert_eq!(
                            serde_json::from_str::<serde_json::Value>(&result).unwrap(),
                            serde_json::json!({
                                "log": expected_log,
                                "href": committed_url,
                                "entryUrl": committed_url,
                                "entryUnchanged": !committed,
                                "historyDelta": if committed { 1 } else { 0 },
                                "transition": null
                            }),
                            "{description}"
                        );
                        let pending = vm.take_pending_location_navigation_with_seed();
                        if allowed && !intercept {
                            assert_eq!(
                                pending.expect(&description).url.as_str(),
                                expected_url,
                                "{description}"
                            );
                        } else {
                            assert!(pending.is_none(), "{description}");
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn form_navigation_events_use_resolved_top_window_across_realms() {
    for (depth, target) in [(1, "_top"), (1, "_PARENT"), (2, "_ToP")] {
        for method in ["get", "post"] {
            for api in ["submit", "requestSubmit"] {
                for use_child_realm in [true, false] {
                    for cancel_top in [true, false] {
                        let mut vm = new_parsed_test_vm(
                            "https://form-navigation-target.test/source",
                            "<!doctype html><body></body>",
                        );
                        let description = format!(
                            "depth={depth} target={target} method={method} api={api} \
                             child_realm={use_child_realm} cancel_top={cancel_top}"
                        );
                        let result = vm
                            .eval(&format!(
                                r#"
                                (() => {{
                                    const seen = [];
                                    let source = document;
                                    for (let i = 0; i < {depth}; ++i) {{
                                        const frame = source.createElement('iframe');
                                        source.body.appendChild(frame);
                                        frame.srcdoc = '<body></body>';
                                        source = frame.contentDocument;
                                        frame.contentWindow.navigation.onnavigate = event => {{
                                            seen.push('child:' + i);
                                            event.preventDefault();
                                        }};
                                        frame.contentWindow.navigation.onnavigateerror = () => seen.push('child-error');
                                    }}
                                    const form = source.createElement('form');
                                    form.target = {target:?};
                                    form.method = {method:?};
                                    form.action = 'https://form-navigation-target.test/submitted';
                                    form.innerHTML = '<input name="value" value="b">';
                                    source.body.appendChild(form);
                                    navigation.onnavigate = event => {{
                                        seen.push([
                                            'top',
                                            event.target === navigation,
                                            event.currentTarget === navigation,
                                            event.sourceElement === form,
                                            event.cancelable,
                                            event.navigationType,
                                            event.destination.url,
                                            {method:?} === 'get' ? event.formData === null : event.formData.get('value') === 'b',
                                            event instanceof NavigateEvent,
                                            {method:?} === 'get' || event.formData instanceof FormData,
                                            {method:?} === 'get' || !(event.formData instanceof source.defaultView.FormData)
                                        ]);
                                        event.signal.onabort = () => seen.push('top-abort');
                                        if ({cancel_top}) event.preventDefault();
                                    }};
                                    navigation.onnavigateerror = () => seen.push('top-error');
                                    const realm = {use_child_realm} ? source.defaultView : window;
                                    if ({use_child_realm} && realm.HTMLFormElement === HTMLFormElement)
                                        throw new Error('submission must use the child realm');
                                    realm.HTMLFormElement.prototype[{api:?}].call(form);
                                    return JSON.stringify(seen);
                                }})()
                                "#,
                            ))
                            .expect(&description);
                        let expected_url = if method == "get" {
                            "https://form-navigation-target.test/submitted?value=b"
                        } else {
                            "https://form-navigation-target.test/submitted"
                        };
                        let mut expected = vec![serde_json::json!([
                            "top",
                            true,
                            true,
                            true,
                            true,
                            "replace",
                            expected_url,
                            true,
                            true,
                            true,
                            true
                        ])];
                        if cancel_top {
                            expected.extend([
                                serde_json::json!("top-abort"),
                                serde_json::json!("top-error"),
                            ]);
                        }
                        assert_eq!(
                            serde_json::from_str::<serde_json::Value>(&result).unwrap(),
                            serde_json::json!(expected),
                            "{description}"
                        );
                        let pending = vm.take_pending_location_navigation_with_seed();
                        if cancel_top {
                            assert!(pending.is_none(), "{description}");
                        } else {
                            let pending = pending.expect(&description);
                            assert_eq!(pending.url.as_str(), expected_url, "{description}");
                            assert_eq!(
                                pending.request_method,
                                method.to_ascii_uppercase(),
                                "{description}"
                            );
                            assert_eq!(
                                pending.request_body.as_deref(),
                                (method == "post").then_some(b"value=b".as_slice()),
                                "{description}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn post_form_navigation_uses_target_intrinsic_form_data_constructor() {
    for (depth, target) in [(1, "_top"), (1, "_self"), (2, "_parent"), (1, "receiver")] {
        for api in ["submit", "requestSubmit"] {
            for use_child_realm in [true, false] {
                let mut vm = new_parsed_test_vm(
                    "https://form-navigation-target.test/source",
                    "<!doctype html><body></body>",
                );
                let description = format!(
                    "depth={depth} target={target} api={api} child_realm={use_child_realm}"
                );
                let result = vm
                .eval(&format!(
                    r#"
                    (() => {{
                        const seen = [];
                        let source = document;
                        for (let i = 0; i < {depth}; ++i) {{
                            const frame = source.createElement('iframe');
                            source.body.appendChild(frame);
                            frame.srcdoc = '<body></body>';
                            source = frame.contentDocument;
                        }}
                        const child = source.defaultView;
                        let targetWindow = {target:?} === '_self' ? child : child.parent;
                        if ({target:?} === 'receiver') {{
                            const receiver = document.createElement('iframe');
                            receiver.name = 'receiver';
                            document.body.appendChild(receiver);
                            receiver.srcdoc = '<body></body>';
                            targetWindow = receiver.contentWindow;
                        }}
                        const TargetFormData = targetWindow.FormData;
                        const ChildFormData = child.FormData;
                        const form = child.document.createElement('form');
                        form.method = 'post';
                        form.target = {target:?};
                        form.action = 'https://form-navigation-target.test/submitted';
                        form.innerHTML = '<input name="value" value="b">';
                        child.document.body.appendChild(form);
                        let constructorReads = 0;
                        form.onformdata = event => {{
                            event.formData.append('added', 'from-event');
                            Object.defineProperty(targetWindow, 'FormData', {{
                                configurable: true,
                                get() {{
                                    ++constructorReads;
                                    throw new Error('navigation must use the intrinsic FormData');
                                }}
                            }});
                        }};
                        targetWindow.navigation.onnavigate = event => {{
                            seen.push([
                                event instanceof targetWindow.NavigateEvent,
                                event.formData instanceof TargetFormData,
                                (event.formData instanceof ChildFormData) === (targetWindow === child),
                                event.formData.get('value'),
                                event.formData.get('added')
                            ]);
                            if (targetWindow !== window) event.preventDefault();
                        }};
                        const realm = {use_child_realm} ? child : window;
                        realm.HTMLFormElement.prototype[{api:?}].call(form);
                        return JSON.stringify([seen, constructorReads]);
                    }})()
                    "#,
                ))
                .expect(&description);
                assert_eq!(
                    result, r#"[[[true,true,true,"b","from-event"]],0]"#,
                    "{description}"
                );
                let pending = vm.take_pending_location_navigation_with_seed();
                if target == "_top" {
                    let pending = pending.expect(&description);
                    assert_eq!(pending.request_method, "POST", "{description}");
                    assert_eq!(
                        pending.request_body.as_deref(),
                        Some(b"value=b&added=from-event".as_slice()),
                        "{description}"
                    );
                } else {
                    assert!(pending.is_none(), "{description}");
                }
            }
        }
    }
}

#[tokio::test]
async fn named_element_navigation_prefers_its_source_frame_over_duplicate_names() {
    for action in ["anchor", "submit", "requestSubmit"] {
        for name in ["initial", "renamed", "shadowed"] {
            let server =
                StaticHttpServer::spawn_with_bodies(vec!["<!doctype html><body>selected".into()])
                    .await;
            let base = server.base_url().origin().ascii_serialization();
            let loader = static_http_loader([]);
            let mut vm = new_storage_page_task_executor_test_vm_with_loader(
                &format!("{base}/parent"),
                &loader,
            );
            vm.eval(&format!(
                r#"
                const earlier = document.createElement('iframe'); earlier.name = 'same';
                const source = document.createElement('iframe');
                source.name = {name:?} === 'renamed' ? 'old-name' : 'same';
                document.body.append(earlier, source);
                const childDocument = source.contentDocument;
                const nested = childDocument.createElement('iframe'); nested.name = 'same';
                childDocument.body.append(nested);
                globalThis.originalDocument = childDocument;
                globalThis.nameReads = 0;
                if ({name:?} === 'renamed') source.contentWindow.name = 'same';
                if ({name:?} === 'shadowed') Object.defineProperty(source.contentWindow, 'name', {{
                    get() {{ ++nameReads; throw new Error('script-visible name was read'); }}
                }});
                globalThis.namedAccessIsChild = source.contentWindow.same === nested.contentWindow;
            "#
            ))
            .unwrap();
            vm.drain_ready_page_task_executor_turns_for_setup(&loader, 100)
                .await
                .unwrap();
            vm.eval(&format!(
                r#"
                const element = childDocument.createElement({action:?} === 'anchor' ? 'a' : 'form');
                element.target = 'same';
                if ({action:?} === 'anchor') element.href = '/selected?from=anchor';
                else {{
                    element.action = '/selected';
                    element.innerHTML = '<input name=from value="{action}">';
                }}
                childDocument.body.append(element);
                if ({action:?} === 'anchor') element.click();
                else element[{action:?}]();
            "#
            ))
            .unwrap();
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(source.contentDocument.body?.textContent === 'selected')",
                "true",
                &format!("{action}, {name}"),
            )
            .await;
            assert_eq!(
                vm.eval(
                    "JSON.stringify([source.contentDocument !== originalDocument, \
                     source.contentWindow.length, earlier.contentWindow.location.href, \
                     namedAccessIsChild, nameReads])",
                )
                .unwrap(),
                r#"[true,0,"about:blank",true,0]"#,
                "{action}, {name}",
            );
            assert_eq!(
                server.finish_targets().await,
                [format!("/selected?from={action}")],
                "{action}, {name}",
            );
        }
    }
}

#[test]
fn form_action_scheme_selects_query_mutation_or_post_resource() {
    for action in [
        "http://form-action.test/target?original=1#fragment",
        "https://form-action.test/target?original=1#fragment",
        "ftp://form-action.test/target?original=1#fragment",
        "data:text/plain,payload?original=1#fragment",
        "javascript:void('original?query#fragment')",
    ] {
        for method in ["get", "post"] {
            for use_submitter in [false, true] {
                let mut vm =
                    new_storage_page_task_executor_test_vm("https://form-action.test/source");
                vm.eval(&format!(
                    r#"
                    globalThis.formDataEvents = 0;
                    const form = document.createElement('form');
                    form.innerHTML = '<input name="value" value="a b"><button>Submit</button>';
                    form.onformdata = event => {{
                        ++formDataEvents;
                        event.formData.append('from', 'event');
                    }};
                    document.body.append(form);
                    if ({use_submitter}) {{
                        form.action = '/wrong';
                        form.method = {method:?} === 'get' ? 'post' : 'get';
                        const button = form.querySelector('button');
                        button.formAction = {action:?}; button.formMethod = {method:?};
                        form.requestSubmit(button);
                    }} else {{
                        form.action = {action:?}; form.method = {method:?}; form.submit();
                    }}
                    form.action = '/changed'; form.method = 'dialog';
                    form.querySelector('input').value = 'changed';
                "#
                ))
                .unwrap();
                assert_eq!(vm.eval("formDataEvents").unwrap(), "1");
                let request = vm.take_pending_location_navigation_with_seed().unwrap();
                let mut expected_url = Url::parse(action).unwrap();
                let mutates_query =
                    method == "get" && matches!(expected_url.scheme(), "http" | "https" | "data");
                let posts_body =
                    method == "post" && matches!(expected_url.scheme(), "http" | "https");
                if mutates_query {
                    expected_url.set_query(Some("value=a+b&from=event"));
                }
                assert_eq!(
                    request.url, expected_url,
                    "{action}/{method}/{use_submitter}"
                );
                assert_eq!(
                    request.request_method,
                    if posts_body { "POST" } else { "GET" }
                );
                assert_eq!(
                    request.request_body.as_deref(),
                    posts_body.then_some(b"value=a+b&from=event".as_slice())
                );
                assert_eq!(
                    request.request_headers,
                    if posts_body {
                        vec![(
                            "Content-Type".to_owned(),
                            "application/x-www-form-urlencoded".to_owned(),
                        )]
                    } else {
                        vec![]
                    }
                );
            }
        }
    }
}

#[tokio::test]
async fn javascript_form_actions_execute_without_serializing_the_entry_list() {
    for method in ["get", "post"] {
        for action in [
            "javascript:parent.formResult='no query';void 0",
            "javascript:parent.formResult='?original#fragment';void 0",
        ] {
            let loader = static_http_loader([]);
            let mut vm = new_storage_page_task_executor_test_vm_with_loader(
                "https://form-action.test/source",
                &loader,
            );
            vm.eval(
                r#"
                globalThis.frame = document.createElement('iframe'); frame.name = 'target';
                document.body.append(frame);
            "#,
            )
            .unwrap();
            vm.drain_ready_page_task_executor_turns_for_setup(&loader, 100)
                .await
                .unwrap();
            vm.eval(&format!(
                r#"
                globalThis.formResult = null;
                globalThis.formDataEvents = 0;
                const form = document.createElement('form');
                form.innerHTML = '<input name="value" value="ignored">';
                form.method = {method:?}; form.action = {action:?}; form.target = 'target';
                form.onformdata = event => {{
                    ++formDataEvents; event.formData.append('event', 'ignored');
                }};
                document.body.append(form); form.submit();
            "#
            ))
            .unwrap();
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(formResult !== null)",
                "true",
                &format!("{method}: {action}"),
            )
            .await;
            assert_eq!(
                vm.eval("formResult").unwrap(),
                if action.contains('?') {
                    "?original#fragment"
                } else {
                    "no query"
                }
            );
            assert_eq!(vm.eval("formDataEvents").unwrap(), "1");
            assert_eq!(
                vm.eval("frame.contentWindow.location.href").unwrap(),
                "about:blank"
            );
        }
    }
}

#[test]
fn location_assign_before_complete_load_respects_user_activation() {
    for activated in [false, true] {
        let mut vm = new_storage_test_vm("https://location-before-load.test/source");
        if activated {
            vm._context_host
                .borrow_mut()
                .begin_protocol_user_gesture_activation();
        }
        let navigation_type = vm
            .eval(
                r#"
          window.observedNavigationType = null;
          navigation.addEventListener('navigate', e => observedNavigationType = e.navigationType);
          location.assign('/destination');
          observedNavigationType
        "#,
            )
            .expect("Location navigation must expose its resolved history behavior");
        if activated {
            vm._context_host
                .borrow_mut()
                .end_protocol_user_gesture_activation();
        }
        assert_eq!(navigation_type, if activated { "push" } else { "replace" });
        let pending = vm.take_pending_location_navigation_with_seed().unwrap();
        let seed = pending.entry_seed.unwrap();
        let from = seed.activation.as_ref().unwrap().from.as_ref().unwrap();
        assert_eq!(
            seed.current_index,
            from.history_index + u32::from(activated)
        );
        assert_eq!(
            seed.entries
                .iter()
                .any(|entry| entry.url == "https://location-before-load.test/source"),
            activated
        );
        assert_eq!(
            seed.entries.last().unwrap().url,
            "https://location-before-load.test/destination"
        );
    }
}

#[test]
fn form_target_blank_reloads_rel_opener_policy_for_each_submission() {
    for (rel, expected_exposes_opener) in [
        ("", false),
        ("opener", true),
        ("noopener", false),
        ("opener noopener", false),
        ("opener noreferrer", false),
    ] {
        let mut vm = new_storage_test_vm("https://example.com/page.html");
        vm.eval(&format!(
            r#"
(() => {{
  const html = document.createElement("html");
  const body = document.createElement("body");
  html.appendChild(body);
  document.appendChild(html);
  const form = document.createElement("form");
  form.action = "/submitted";
  form.target = "_BLANK";
  form.rel = {rel:?};
  body.appendChild(form);
  form.submit();
}})()
"#
        ))
        .expect("target=_blank form submission should evaluate");

        let activations = vm.take_pending_popup_activations();
        assert_eq!(
            activations.len(),
            1,
            "rel={rel:?} should produce one auxiliary browsing-context action"
        );
        let crate::RendererPopupActivationSource::Window { exposes_opener, .. } =
            activations[0].source()
        else {
            panic!("form submission must retain its exact Window source");
        };
        assert_eq!(
            exposes_opener, &expected_exposes_opener,
            "rel={rel:?} opener policy"
        );
        assert_eq!(
            activations[0].disposition(),
            crate::RendererPopupDisposition::Foreground,
            "target=_blank form submission should select its new surface"
        );
    }
}

#[tokio::test]
async fn form_target_blank_preserves_source_referrer_and_relations() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    for child_document in [false, true] {
        for submitter in ["form", "button", "input"] {
            for (rel, has_opener, has_referrer) in [
                ("", false, true),
                ("noopener", false, true),
                ("noreferrer", false, false),
                ("opener", true, true),
                ("noopener noreferrer", false, false),
                ("noreferrer opener", false, false),
                ("opener noopener", false, true),
            ] {
                let (popup_url, server) = super::misc::spawn_lightweight_popup_html_responses(
                    "form popup relation server",
                    "form popup relation",
                    "Cache-Control: no-store",
                    r#"<!doctype html><script>
                    if (location.pathname !== "/child.html") {
                      new BroadcastChannel("form-popup-relations").postMessage({
                        hasOpener: opener !== null,
                        referrer: document.referrer,
                        openerPath: opener === null ? null : opener.location.pathname
                      });
                      window.close();
                    }
                    </script>"#,
                    if child_document { 2 } else { 1 },
                )
                .await;
                let parent_url = format!(
                    "{}?source=top#fragment",
                    popup_url.replace("/popup.html", "/parent.html")
                );
                let child_url = format!(
                    "{}?source=child#fragment",
                    popup_url.replace("/popup.html", "/child.html")
                );
                let mut vm = new_broadcast_channel_page_test_vm_with_loader(&parent_url, &loader);
                vm.eval(&format!(
                    r#"
                    globalThis.results = [];
                    globalThis.channel = new BroadcastChannel("form-popup-relations");
                    channel.onmessage = event => results.push(event.data);
                    const html = document.createElement("html");
                    const body = document.createElement("body");
                    html.appendChild(body);
                    document.appendChild(html);
                    if ({child_document}) {{
                      globalThis.frame = document.createElement("iframe");
                      frame.src = {child_url:?};
                      body.appendChild(frame);
                    }}
                    'created'
                    "#,
                ))
                .expect("form source document should be created");
                if child_document {
                    advance_page_task_executor_until_eval_equals(
                        &mut vm,
                        &loader,
                        &format!("String(frame.contentDocument?.URL === {child_url:?} && frame.contentDocument.readyState === 'complete')"),
                        "true",
                        "form source iframe should load",
                    )
                    .await;
                }
                vm.eval(&format!(
                    r#"
                    const owner = {child_document} ? frame.contentDocument : document;
                    const form = owner.createElement("form");
                    form.action = {popup_url:?};
                    form.rel = {rel:?};
                    owner.body.appendChild(form);
                    if ({submitter:?} === "form") {{
                      form.target = "_BLANK";
                      form.submit();
                    }} else {{
                      const control = owner.createElement({submitter:?});
                      control.type = "submit";
                      control.formTarget = "_blank";
                      form.appendChild(control);
                      control.click();
                    }}
                    'submitted'
                    "#,
                ))
                .expect("form popup submission should evaluate");
                advance_page_task_executor_until_eval_equals(
                    &mut vm,
                    &loader,
                    "String(results.length)",
                    "1",
                    "form popup should report its loaded document relations",
                )
                .await;
                let actual: serde_json::Value = serde_json::from_str(
                    &vm.eval("JSON.stringify(results[0])").expect("popup result"),
                )
                .expect("popup JSON");
                let source_path = if child_document {
                    "/child.html"
                } else {
                    "/parent.html"
                };
                let referrer = if has_referrer {
                    format!(
                        "{}?source={}",
                        popup_url.replace("/popup.html", source_path),
                        if child_document { "child" } else { "top" }
                    )
                } else {
                    String::new()
                };
                assert_eq!(
                    actual,
                    serde_json::json!({
                        "hasOpener": has_opener,
                        "referrer": referrer,
                        "openerPath": has_opener.then_some(source_path),
                    }),
                    "child={child_document}, submitter={submitter}, rel={rel:?}",
                );
                server.await.expect("form popup server should finish");
            }
        }
    }
}

#[tokio::test]
async fn popup_navigation_applies_referrer_policy_and_link_overrides() {
    const SOURCE: &str = "http://referrer-source.test/page.html?source=1#fragment";
    const FULL: &str = "http://referrer-source.test/page.html?source=1";
    const ORIGIN: &str = "http://referrer-source.test/";
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    for (tag, document_policy, element_policy, expected) in [
        ("form", None, None, ORIGIN),
        ("form", Some("origin"), None, ORIGIN),
        ("form", Some("no-referrer"), None, ""),
        ("form", Some("same-origin"), None, ""),
        ("form", Some("unsafe-url"), None, FULL),
        ("form", Some("no-referrer"), Some("unsafe-url"), ""),
        ("a", Some("no-referrer"), Some("unsafe-url"), FULL),
        ("area", Some("unsafe-url"), Some("no-referrer"), ""),
        ("a", Some("origin"), Some("invalid-policy"), ORIGIN),
    ] {
        let (popup_url, server) = super::misc::spawn_lightweight_popup_html_responses(
            "popup referrer policy server",
            "popup referrer policy",
            "Cache-Control: no-store",
            r#"<!doctype html><script>
            opener.postMessage(document.referrer, "*");
            window.close();
            </script>"#,
            1,
        )
        .await;
        let mut vm = new_broadcast_channel_page_test_vm_with_loader(SOURCE, &loader);
        vm.set_response_referrer_policy(document_policy.map(str::to_owned));
        vm.eval(&format!(
            r#"
            globalThis.results = [];
            onmessage = event => results.push(event.data);
            const html = document.createElement("html");
            const body = document.createElement("body");
            html.appendChild(body);
            document.appendChild(html);
            const element = document.createElement({tag:?});
            element.target = "_blank";
            element.rel = "opener";
            element.setAttribute("referrerpolicy", {element_policy:?});
            body.appendChild(element);
            if ({tag:?} === "form") {{
              element.action = {popup_url:?};
              element.submit();
            }} else {{
              element.href = {popup_url:?};
              element.click();
            }}
            'submitted'
            "#,
            element_policy = element_policy.unwrap_or_default(),
        ))
        .expect("cross-origin popup should be submitted");
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(results.length)",
            "1",
            "cross-origin popup should report its referrer",
        )
        .await;
        assert_eq!(
            vm.eval("results[0]").expect("reported referrer"),
            expected,
            "tag={tag}, document policy={document_policy:?}, element policy={element_policy:?}",
        );
        server
            .await
            .expect("popup referrer policy server should finish");
    }
}

#[tokio::test]
async fn hyperlink_target_blank_reloads_rel_opener_policy_for_each_activation() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_broadcast_channel_page_test_vm_with_loader("https://example.com/page.html", &loader);

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__hyperlinkPopupResults = [];
  globalThis.__hyperlinkPopupChannel = new BroadcastChannel("hyperlink-rel-policy");
  __hyperlinkPopupChannel.onmessage = event => __hyperlinkPopupResults.push(event.data);
  globalThis.__hyperlinkPopupUrl = label => URL.createObjectURL(new Blob([`
    <!doctype html>
    <script>
      new BroadcastChannel("hyperlink-rel-policy").postMessage({
        label: ${JSON.stringify(label)},
        hasOpener: opener !== null,
        referrer: document.referrer
      });
      window.close();
    <\/script>
  `], { type: "text/html" }));
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || html.appendChild(document.createElement("body"));
  globalThis.__hyperlink = document.createElement("a");
  __hyperlink.target = "_blank";
  __hyperlink.rel = "noopener";
  __hyperlink.href = __hyperlinkPopupUrl("anchor-noopener");
  body.appendChild(__hyperlink);
  __hyperlink.click();
  return String(__hyperlinkPopupResults.length);
})()
"#,
        )
        .expect("anchor noopener popup setup should evaluate");
    assert_eq!(setup, "0");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__hyperlinkPopupResults.length)",
        "1",
        "anchor noopener popup should load",
    )
    .await;

    vm.eval(
        r#"
__hyperlink.rel = "opener";
__hyperlink.href = __hyperlinkPopupUrl("anchor-opener");
__hyperlink.click();
"#,
    )
    .expect("anchor opener popup should schedule");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__hyperlinkPopupResults.length)",
        "2",
        "anchor opener popup should load",
    )
    .await;

    vm.eval(
        r#"
globalThis.__hyperlink = document.createElement("area");
__hyperlink.target = "_blank";
__hyperlink.rel = "noreferrer";
__hyperlink.href = __hyperlinkPopupUrl("area-noreferrer");
document.body.appendChild(__hyperlink);
__hyperlink.click();
"#,
    )
    .expect("area noreferrer popup should schedule");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__hyperlinkPopupResults.length)",
        "3",
        "area noreferrer popup should load",
    )
    .await;

    vm.eval(
        r#"
__hyperlink.rel = "opener";
__hyperlink.href = __hyperlinkPopupUrl("area-opener");
__hyperlink.click();
"#,
    )
    .expect("area opener popup should schedule");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__hyperlinkPopupResults.length)",
        "4",
        "area opener popup should load",
    )
    .await;

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__hyperlinkPopupResults)")
            .expect("hyperlink popup relation results should evaluate"),
        r#"[{"label":"anchor-noopener","hasOpener":false,"referrer":"https://example.com/page.html"},{"label":"anchor-opener","hasOpener":true,"referrer":"https://example.com/page.html"},{"label":"area-noreferrer","hasOpener":false,"referrer":""},{"label":"area-opener","hasOpener":true,"referrer":"https://example.com/page.html"}]"#
    );
}
