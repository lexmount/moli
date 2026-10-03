use super::*;
use crate::RendererPopupActivationSource;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn popup_request_context_keeps_the_accepted_source_url_and_referrer_policy() {
    let runtime = JsRuntime::initialize();
    runtime
        .browser_context_runtime()
        .delegate_auxiliary_document_responses_to_browser();
    let (tx, mut rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(tx);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let source = url::Url::parse("https://source.test/accepted?query=value#fragment").unwrap();
    let mut page = create_test_html_page_with_navigation_dispatch(
        &runtime,
        &loader,
        source.clone(),
        "<!doctype html><meta name=referrer content=origin><body><a id=link href='https://destination.test/link' target=link rel=noreferrer>link</a>",
        RendererTopLevelNavigationDispatch::DelegateToBrowser,
    )
    .await;
    rx.drain();
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "open('https://destination.test/child','child');open('https://destination.test/private','private','noreferrer');document.getElementById('link').click();document.querySelector('meta').content='unsafe-url';history.replaceState(null,'','/later');true".to_owned(),
            await_promise: false,
        })
        .await
        .unwrap();
    assert_eq!(renderer_json_value(reply), Some(serde_json::json!(true)));
    let publications = rx.drain();
    let activations = popup_activations_for_page(&publications, &page);
    assert_eq!(activations.len(), 3);
    for (activation, (expected_policy, expected_referrer)) in activations.iter().zip([
        ("origin", "https://source.test/"),
        ("no-referrer", ""),
        ("no-referrer", ""),
    ]) {
        let initiator = activation.navigation_initiator().unwrap();
        assert_eq!(initiator.url(), &source);
        assert_eq!(
            initiator.request_metadata().referrer_policy.as_deref(),
            Some(expected_policy)
        );
        assert_eq!(
            initiator.document_referrer(&url::Url::parse(activation.url()).unwrap()),
            expected_referrer
        );
    }
    drop(activations);
    drop(publications);
    page.close_async().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nested_srcdoc_popup_requests_use_the_ancestor_document_referrer_url() {
    let runtime = JsRuntime::initialize();
    runtime
        .browser_context_runtime()
        .delegate_auxiliary_document_responses_to_browser();
    let (tx, mut rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(tx);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let source = url::Url::parse("https://source.test/ancestor?accepted=true#fragment").unwrap();
    let mut page = create_test_html_page_with_navigation_dispatch(
        &runtime,
        &loader,
        source.clone(),
        "<!doctype html><base href='https://different.test/base/'><body></body>",
        RendererTopLevelNavigationDispatch::DelegateToBrowser,
    )
    .await;
    rx.drain();
    let inner = "<!doctype html><meta name=referrer content=origin><a id=link href='https://destination.test/link' target=link rel=opener>link</a><script>open('https://destination.test/child','child');document.getElementById('link').click();</script>";
    let outer = format!(
        "<!doctype html><body><script>const frame=document.createElement('iframe');frame.srcdoc={};document.body.append(frame);</script>",
        serde_json::to_string(inner)
            .unwrap()
            .replace("</script>", "<\\/script>")
    );
    page.run_async_command(RendererPageCommand::EvaluateExpression {
        expression: format!(
            "const frame=document.createElement('iframe');frame.srcdoc={};document.body.append(frame);true",
            serde_json::to_string(&outer).unwrap()
        ),
        await_promise: false,
    })
    .await
    .unwrap();
    page.run_async_command(
        RendererPageCommand::CompleteChildFrameLifecycleWorkBestEffort {
            timeout_ms: 2_000,
            loader: loader.clone(),
        },
    )
    .await
    .unwrap();
    let publications = rx.drain();
    let activations = popup_activations_for_page(&publications, &page);
    assert_eq!(activations.len(), 2);
    for activation in &activations {
        let initiator = activation.navigation_initiator().unwrap();
        assert_eq!(initiator.url(), &source);
        assert_eq!(
            initiator.origin().ascii_serialization(),
            "https://source.test"
        );
        assert_eq!(
            initiator.document_referrer(&url::Url::parse(activation.url()).unwrap()),
            "https://source.test/"
        );
    }
    drop(activations);
    drop(publications);
    page.close_async().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opaque_popup_initiators_do_not_send_a_referrer_from_their_http_url() {
    let runtime = JsRuntime::initialize();
    runtime
        .browser_context_runtime()
        .delegate_auxiliary_document_responses_to_browser();
    let (tx, mut rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(tx);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let source = url::Url::parse("https://source.test/private?secret=value").unwrap();
    let (mut page, _, _, _, _) = runtime.create_html_page_from_response(
        source.clone(), source.clone(), None, false, 0, 200,
        vec![
            ("content-type".to_owned(), b"text/html".to_vec()),
            ("content-security-policy".to_owned(), b"sandbox allow-scripts allow-popups allow-popups-to-escape-sandbox".to_vec()),
        ], &loader, crate::RendererWebStorageHandles::ephemeral(),
        "<!doctype html><meta name=referrer content=unsafe-url><a id=link target=link rel=opener href='https://source.test/link'>link</a>".to_owned(),
        crate::RendererDocumentOptions::default(),
    ).await.unwrap();
    rx.drain();
    page.run_async_command(RendererPageCommand::EvaluateExpression {
        expression:
            "open('https://source.test/child','child');document.getElementById('link').click();true"
                .to_owned(),
        await_promise: false,
    })
    .await
    .unwrap();
    let publications = rx.drain();
    let activations = popup_activations_for_page(&publications, &page);
    assert_eq!(activations.len(), 2);
    for activation in &activations {
        let initiator = activation.navigation_initiator().unwrap();
        assert_eq!(initiator.url(), &source);
        assert!(initiator.origin().is_opaque());
        assert_eq!(
            initiator.request_metadata().referrer_policy.as_deref(),
            Some("no-referrer")
        );
        assert_eq!(
            initiator.document_referrer(&url::Url::parse(activation.url()).unwrap()),
            ""
        );
    }
    drop(activations);
    drop(publications);
    page.close_async().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn selected_navigation_referrer_precedes_parser_and_document_start_scripts() {
    let runtime = JsRuntime::initialize();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let url = url::Url::parse("https://destination.test/child").unwrap();
    for referrer in ["https://source.test/accepted", ""] {
        let (mut page, _, _, _, _) = runtime
            .create_html_page_from_response(
                url.clone(),
                url.clone(),
                None,
                false,
                0,
                200,
                vec![
                    ("content-type".to_owned(), b"text/html".to_vec()),
                    ("referrer-policy".to_owned(), b"no-referrer".to_vec()),
                ],
                &loader,
                crate::RendererWebStorageHandles::ephemeral(),
                "<!doctype html><script>window.parserReferrer=document.referrer</script>"
                    .to_owned(),
                crate::RendererDocumentOptions {
                    main_document_commit: Some(crate::RendererMainDocumentCommit {
                        frame_id: "referrer-frame".to_owned(),
                        loader_id: "referrer-loader".to_owned(),
                        url: url.to_string(),
                        unreachable_url: None,
                        security_origin: "https://destination.test".to_owned(),
                        secure_context_type: "Secure".to_owned(),
                        document_referrer: Some(referrer.to_owned()),
                        timestamp: 0.0,
                        session_history_position: None,
                        browsing_context_group: None,
                    }),
                    document_start_scripts: vec![crate::DocumentStartScript {
                        registry_key: None,
                        devtools_session: None,
                        source: "window.startReferrer=document.referrer".to_owned(),
                        world_name: None,
                        has_bidi_channel_argument: false,
                        bidi_channel_handoffs: Vec::new(),
                    }],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let (reply, _) = page
            .run_async_command(RendererPageCommand::EvaluateExpression {
                expression: "JSON.stringify([document.referrer,startReferrer,parserReferrer])"
                    .to_owned(),
                await_promise: false,
            })
            .await
            .unwrap();
        assert_eq!(
            renderer_json_value(reply),
            Some(serde_json::Value::String(
                serde_json::json!([referrer, referrer, referrer]).to_string()
            ))
        );
        page.close_async().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn noopener_initial_pages_keep_creator_security_and_fresh_session_storage() {
    let runtime = JsRuntime::initialize();
    runtime
        .browser_context_runtime()
        .delegate_auxiliary_document_responses_to_browser();
    let (tx, mut rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(tx);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for (origin, secure) in [("https://source.test", true), ("http://source.test", false)] {
        let source_url = url::Url::parse(&format!("{origin}/creator")).unwrap();
        let storage = crate::RendererWebStorageHandles::ephemeral();
        let (mut parent, _, _, _, _) = runtime
            .start_create_html_page_from_response(
                runtime.reserve_page_for_creation(),
                source_url.clone(),
                source_url,
                None,
                false,
                0,
                200,
                vec![("content-type".to_owned(), b"text/html".to_vec())],
                &loader,
                storage.clone(),
                "<!doctype html><body>creator</body>".to_owned(),
                None,
                RendererTopLevelNavigationDispatch::DelegateToBrowser,
                crate::RendererDocumentOptions::default(),
            )
            .unwrap()
            .await_ready()
            .await
            .unwrap();
        for (href, features) in [
            ("", "noopener"),
            ("about:blank", "noopener"),
            ("", "noreferrer"),
            ("about:blank", "noreferrer"),
        ] {
            rx.drain();
            let (reply, _) = parent.run_async_command(RendererPageCommand::EvaluateExpression {
                expression: format!("localStorage.setItem('shared','creator');sessionStorage.setItem('private','creator');open({href:?},'detached',{features:?})===null"),
                await_promise: false,
            }).await.unwrap();
            assert_eq!(renderer_json_value(reply), Some(serde_json::json!(true)));
            let publications = rx.drain();
            let activations = popup_activations_for_page(&publications, &parent);
            assert_eq!(activations.len(), 1);
            let activation = &activations[0];
            assert!(matches!(
                activation.source(),
                RendererPopupActivationSource::Window {
                    exposes_opener: false,
                    ..
                }
            ));
            let pending = activation
                .pending_auxiliary_page()
                .expect("noopener has a concrete initial Page");
            let crate::RendererPopupActivationParts {
                session_storage_store: session_storage,
                initial_empty_document_storage_key: storage_key,
                ..
            } = activation.clone().into_parts();
            let blank = url::Url::parse("about:blank").unwrap();
            let (mut child, _, _, _, _) = runtime
                .start_create_html_page_from_response(
                    pending.page_reservation(),
                    blank.clone(),
                    blank,
                    None,
                    false,
                    0,
                    200,
                    vec![("content-type".to_owned(), b"text/html".to_vec())],
                    &loader,
                    crate::RendererWebStorageHandles::new(
                        storage.local_storage(),
                        session_storage.unwrap(),
                    ),
                    "<!doctype html><body></body>".to_owned(),
                    storage_key,
                    RendererTopLevelNavigationDispatch::DelegateToBrowser,
                    crate::RendererDocumentOptions::default(),
                )
                .unwrap()
                .await_ready()
                .await
                .unwrap();
            let (reply, _) = child.run_async_command(RendererPageCommand::EvaluateExpression {
                expression: "JSON.stringify([location.href,origin,isSecureContext,opener===null,document.baseURI,localStorage.getItem('shared'),sessionStorage.getItem('private'),document.referrer])".to_owned(),
                await_promise: false,
            }).await.unwrap();
            assert_eq!(
                renderer_json_value(reply),
                Some(serde_json::Value::String(
                    serde_json::json!([
                        "about:blank",
                        origin,
                        secure,
                        true,
                        "about:blank",
                        "creator",
                        null,
                        if features == "noreferrer" {
                            String::new()
                        } else {
                            format!("{origin}/")
                        }
                    ])
                    .to_string()
                )),
                "{origin}, {href:?}, {features}"
            );
            let detached_group = activation.browsing_context_name().unwrap().group();
            rx.drain();
            parent
                .run_async_command(RendererPageCommand::EvaluateExpression {
                    expression: "open('', 'detached');true".to_owned(),
                    await_promise: false,
                })
                .await
                .unwrap();
            let related = rx.drain();
            let related_activations = popup_activations_for_page(&related, &parent);
            assert_eq!(related_activations.len(), 1);
            assert_ne!(
                related_activations[0]
                    .browsing_context_name()
                    .unwrap()
                    .group(),
                detached_group
            );
            drop(related_activations);
            drop(related);
            child.close_async().await.unwrap();
        }
        parent.close_async().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auxiliary_blank_pages_use_the_inherited_security_origin_for_document_domain() {
    let runtime = JsRuntime::initialize();
    runtime
        .browser_context_runtime()
        .delegate_auxiliary_document_responses_to_browser();
    let (tx, mut rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(tx);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for (origin, domain) in [
        ("https://sub.example.test", "sub.example.test"),
        ("http://127.0.0.1:8000", "127.0.0.1"),
        ("http://[::1]:8000", "::1"),
    ] {
        let mut parent = create_test_html_page_with_navigation_dispatch(
            &runtime,
            &loader,
            url::Url::parse(&format!("{origin}/creator")).unwrap(),
            "<!doctype html><body>creator</body>",
            RendererTopLevelNavigationDispatch::DelegateToBrowser,
        )
        .await;
        rx.drain();
        parent
            .run_async_command(RendererPageCommand::EvaluateExpression {
                expression: "open('about:blank','child');true".to_owned(),
                await_promise: false,
            })
            .await
            .unwrap();
        let publications = rx.drain();
        let activations = popup_activations_for_page(&publications, &parent);
        assert_eq!(activations.len(), 1);
        let pending = activations[0].pending_auxiliary_page().unwrap();
        let crate::RendererPopupActivationParts {
            session_storage_store: session_storage,
            initial_empty_document_storage_key: storage_key,
            ..
        } = activations[0].clone().into_parts();
        let blank = url::Url::parse("about:blank").unwrap();
        let (mut child, _, _, _, _) = runtime
            .start_create_html_page_from_response(
                pending.page_reservation(),
                blank.clone(),
                blank,
                None,
                false,
                0,
                200,
                vec![("content-type".to_owned(), b"text/html".to_vec())],
                &loader,
                crate::RendererWebStorageHandles::new(
                    crate::new_shared_web_storage_store(),
                    session_storage.unwrap(),
                ),
                "<!doctype html><body></body>".to_owned(),
                storage_key,
                RendererTopLevelNavigationDispatch::DelegateToBrowser,
                crate::RendererDocumentOptions::default(),
            )
            .unwrap()
            .await_ready()
            .await
            .unwrap();
        let (reply, _) = child.run_async_command(RendererPageCommand::EvaluateExpression {
            expression: format!("(()=>{{const initial=document.domain;document.domain={domain:?};let invalid;try{{document.domain='different.invalid';invalid='accepted'}}catch(e){{invalid=e.name}}return JSON.stringify([initial,document.domain,origin,invalid])}})()"),
            await_promise: false,
        }).await.unwrap();
        assert_eq!(
            renderer_json_value(reply),
            Some(serde_json::Value::String(
                serde_json::json!([domain, domain, origin, "SecurityError"]).to_string()
            ))
        );
        child.close_async().await.unwrap();
        parent.close_async().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abandoning_auxiliary_adoption_retires_only_the_reserved_page() {
    let runtime = JsRuntime::initialize();
    runtime
        .browser_context_runtime()
        .delegate_auxiliary_document_responses_to_browser();
    let (tx, mut rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(tx);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = create_test_html_page_with_navigation_dispatch(
        &runtime,
        &loader,
        url::Url::parse("https://example.test/creator").unwrap(),
        "<!doctype html><body>creator</body>",
        RendererTopLevelNavigationDispatch::DelegateToBrowser,
    )
    .await;
    rx.drain();
    let baseline = runtime.document_isolate_accounting_for_diagnostics();
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "window.p=open('about:blank','pending'); p.closed".to_owned(),
            await_promise: false,
        })
        .await
        .unwrap();
    assert_eq!(renderer_json_value(reply), Some(serde_json::json!(false)));
    let publications = rx.drain();
    let activations = popup_activations_for_page(&publications, &page);
    assert_eq!(activations.len(), 1);
    let pending = activations[0].pending_auxiliary_page().unwrap();
    let retained = pending.clone();
    drop(activations);
    drop(publications);
    drop(pending);
    assert_eq!(
        runtime
            .document_isolate_accounting_for_diagnostics()
            .reserved,
        baseline.reserved + 1
    );
    // A cloned protocol capability keeps the one initial Page alive. Only
    // the final drop cancels its owner-local reservation.
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                "[p.closed,p.document.body.textContent,document.body.textContent].join('|')"
                    .to_owned(),
            await_promise: false,
        })
        .await
        .unwrap();
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("false||creator"))
    );
    drop(retained);
    tokio::time::timeout(Duration::from_secs(5), async {
        while runtime
            .document_isolate_accounting_for_diagnostics()
            .reserved
            != baseline.reserved
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("abandoned initial Page must release its isolate reservation");
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "p.closed && document.body.textContent === 'creator'".to_owned(),
            await_promise: false,
        })
        .await
        .unwrap();
    assert_eq!(renderer_json_value(reply), Some(serde_json::json!(true)));
    assert_eq!(runtime.renderer_page_count_for_testing(), 1);
    page.close_async().await.unwrap();
}
