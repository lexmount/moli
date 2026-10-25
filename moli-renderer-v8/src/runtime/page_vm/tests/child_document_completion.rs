use super::*;

use crate::frame_owner_model::{
    ChildDocumentNavigationFetchTarget, DocumentId, FrameDocumentTaskOwner, FrameRequestId,
    FrameSchedulerLaneId, LocalWindowId,
};
use crate::types::{
    ChildDocumentLoadCompletion, ChildDocumentLoadNetworkAttribution, ChildDocumentLoadOutcome,
    LoadedChildDocument,
};

fn owner_attached_page_vm(
    loader: &crate::network::ResourceRequestClient,
    document_url: Url,
) -> (
    PageVm,
    crate::page_task_queue::RendererPageResourceCompletionTestSource,
    tokio::sync::mpsc::UnboundedReceiver<crate::page_task_queue::RendererOwnerWake>,
) {
    let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
    let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(
        wake_tx,
        crate::runtime::RendererPageToken::new_for_testing(PageId::new_for_testing(1)),
    );
    let hooks = PageVmRuntimeHooks::standalone_with_owner_wake_without_owner_reservation_for_test(
        owner_wake,
    );
    let page_vm =
        test_page_vm_with_loader_document_url_and_hooks(loader, Vec::new(), document_url, hooks);
    let queue = page_vm.page_resource_completion_queue();
    (page_vm, queue, wake_rx)
}

pub(super) async fn wait_for_page_resource_completion(
    queue: &mut impl crate::page_resource_completion::RendererPageResourceCompletionTestSource,
    wake_rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::page_task_queue::RendererOwnerWake>,
    label: &str,
) {
    if queue.has_ready_completion() {
        return;
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while !queue.has_ready_completion() {
            wake_rx
                .recv()
                .await
                .unwrap_or_else(|| panic!("{label} owner wake route closed before its terminal"));
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{label} did not produce a terminal before timeout"));
}

async fn start_external_child_document_load(
    page_vm: &mut PageVm,
    frame_id: &str,
    child_url: &str,
) -> (
    crate::document_runtime::DomHandle,
    ChildDocumentNavigationFetchTarget,
) {
    page_vm
        .vm_mut()
        .eval(&format!(
            r#"
{{
const frame = document.createElement("iframe");
frame.id = {frame_id:?};
frame.src = {child_url:?};
document.body.appendChild(frame);
}}
"#
        ))
        .expect("external child fixture should be created");
    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
        page_vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "external child navigation start",
    )
    .await;
    let handle = page_vm
        .vm()
        .element_handle_by_id_for_test(frame_id)
        .expect("external child fixture should retain its native handle");
    let target = page_vm
        .vm()
        .current_child_document_navigation_fetch_target(handle)
        .expect("started external child fetch should expose its exact navigation target");
    (handle, target)
}

#[tokio::test(flavor = "current_thread")]
async fn production_child_document_fetch_reaches_stable_typed_turn_and_commits() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/typed-child.html",
            "HTTP/1.1 200 OK",
            "<!doctype html><p id='typed-child-loaded'>typed child</p>".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, mut queue, mut wake_rx) = owner_attached_page_vm(
            &loader,
            Url::parse(&format!("{base_url}/page.html")).expect("page URL"),
        );
        let root_document = page_vm.document_lifecycle.identity().document;
        let child_url = format!("{base_url}/typed-child.html");
        let (handle, target) =
            start_external_child_document_load(&mut page_vm, "typed-child-frame", &child_url).await;

        wait_for_page_resource_completion(&mut queue, &mut wake_rx, "typed child fetch").await;
        assert_eq!(
            queue.next_ready_owner(),
            Some(
                RendererPageResourceCompletionOwner::child_document_navigation(
                    root_document,
                    target,
                )
            ),
            "the stable queue owner must come entirely from the producer-captured exact target"
        );

        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("current child terminal should consume one typed Page turn");
        assert_eq!(
            outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            outcome.action.source,
            RendererOwnerResourceActivitySource::ChildDocument
        );
        assert_eq!(
            page_vm
                .vm()
                .current_child_document_navigation_fetch_target(handle),
            None,
            "application must settle the exact in-flight request"
        );
        assert_eq!(
            page_vm.vm_mut().eval(
                "document.getElementById('typed-child-frame').contentDocument\
                 .getElementById('typed-child-loaded').textContent"
            )?,
            "typed child"
        );
        assert!(page_vm.take_completed_child_document_networks().is_empty());
        assert!(!queue.has_ready_completion());

        server.await.expect("typed child server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("production typed child-document test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn null_body_child_navigation_reports_network_without_replacing_document() {
    run_page_vm_async_test(async move {
        for (status, status_line, tao_status_line) in [
            (204, "HTTP/1.1 204 No Content", "HTTP/1.1 204 No Content\r\nTiming-Allow-Origin: *"),
            (205, "HTTP/1.1 205 Reset Content", "HTTP/1.1 205 Reset Content\r\nTiming-Allow-Origin: *"),
        ] {
            for same_origin in [true, false] {
                for tao in [false, true] {
                    let (base_url, server) = spawn_path_response_http_server(vec![(
                        "/no-document",
                        if tao { tao_status_line } else { status_line },
                        String::new(),
                        Duration::ZERO,
                    )])
                    .await;
                    let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
                        .expect("loader");
                    let page_url = if same_origin {
                        format!("{base_url}/page.html")
                    } else {
                        "https://embedding-origin.test/page.html".to_owned()
                    };
                    let (mut page_vm, mut queue, mut wake_rx) =
                        owner_attached_page_vm(&loader, Url::parse(&page_url)?);
                    let child_url = format!("{base_url}/no-document");
                    let (handle, _) = start_external_child_document_load(
                        &mut page_vm, "no-document-frame", &child_url,
                    )
                    .await;
                    page_vm.vm_mut().eval(r#"
                        globalThis.__retainedFrame = document.getElementById('no-document-frame');
                        globalThis.__retainedDocument = __retainedFrame.contentDocument;
                        globalThis.__retainedWindow = __retainedFrame.contentWindow;
                        globalThis.__retainedURL = __retainedDocument.URL;
                        globalThis.__retainedHistoryLength = __retainedWindow.history.length;
                        globalThis.__unexpectedFrameEvent = false;
                        __retainedFrame.onload = __retainedFrame.onerror = () => {
                            __unexpectedFrameEvent = true;
                        };
                    "#)?;
                    let _ = page_vm.take_completed_child_frame_navigation_loads();
                    wait_for_page_resource_completion(&mut queue, &mut wake_rx, "null-body child fetch").await;
                    let outcome = page_vm
                        .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
                        .expect("null-body terminal must consume its authorized turn");
                    assert_eq!(outcome.action.document_effect,
                        PageResourceCompletionDocumentEffect::AppliedToCurrentOwner);
                    assert_eq!(page_vm.vm().current_child_document_navigation_fetch_target(handle), None);
                    assert_eq!(page_vm.vm_mut().eval(r#"String(
                        __retainedFrame.contentDocument === __retainedDocument &&
                        __retainedFrame.contentWindow === __retainedWindow &&
                        __retainedDocument.URL === __retainedURL &&
                        __retainedWindow.history.length === __retainedHistoryLength &&
                        !__unexpectedFrameEvent
                    )"#)?, "true", "a completed fetch must not commit a Document, history entry, or load event");
                    assert!(page_vm.take_completed_child_frame_navigation_loads().is_empty());
                    let network = page_vm.take_completed_child_document_networks();
                    assert_eq!(network.len(), 1, "the real response must still reach Network exactly once");
                    assert_eq!(network[0].snapshot.status, status);
                    assert_eq!(network[0].snapshot.request_url, child_url);
                    assert!(network[0].snapshot.resource_timing_response.is_some());
                    let timing: serde_json::Value = serde_json::from_str(&page_vm.vm_mut().eval(
                        &format!("JSON.stringify(performance.getEntriesByName({child_url:?}, 'resource').map(entry => entry.toJSON()))")
                    )?)?;
                    let entries = timing.as_array().expect("Resource Timing JSON array");
                    assert_eq!(entries.len(), usize::from(same_origin || tao));
                    if let Some(entry) = entries.first() {
                        assert_eq!(entry["initiatorType"], "iframe");
                        assert_eq!(entry["responseStatus"], if same_origin { status } else { 0 });
                        assert_eq!(entry["contentType"], "");
                        assert_eq!(entry["contentEncoding"], "");
                        assert_eq!(entry["encodedBodySize"], 0);
                        assert_eq!(entry["decodedBodySize"], 0);
                        assert_eq!(entry["transferSize"], 300);
                        assert_eq!(entry["nextHopProtocol"], "http/1.1");
                        assert!(entry["requestStart"].as_f64().unwrap() >= entry["fetchStart"].as_f64().unwrap());
                        assert!(entry["responseStart"].as_f64().unwrap() >= entry["requestStart"].as_f64().unwrap());
                        assert!(entry["responseEnd"].as_f64().unwrap() >= entry["responseStart"].as_f64().unwrap());
                    }
                    assert!(!queue.has_ready_completion());
                    server.await.expect("null-body response server should finish");
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("null-body navigation test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn required_csp_header_retains_the_admitted_attribute_across_owner_refreshes() {
    run_page_vm_async_test(async move {
        for refresh in ["name", "window"] {
            let (base_url, server) = spawn_path_response_http_server(vec![(
                "/csp-snapshot",
                "HTTP/1.1 204 No Content\r\nAllow-CSP-From: *",
                String::new(),
                Duration::ZERO,
            )])
            .await;
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
                .expect("loader");
            let (mut page_vm, mut queue, mut wake_rx) =
                owner_attached_page_vm(&loader, Url::parse(&format!("{base_url}/page.html"))?);
            let child_url = format!("{base_url}/csp-snapshot");
            page_vm.vm_mut().eval(&format!(
                r#"
                globalThis.__cspFrame = document.createElement('iframe');
                __cspFrame.id = 'request-csp-frame';
                __cspFrame.csp = "script-src 'unsafe-inline'; policy-before-source";
                __cspFrame.src = {child_url:?};
                document.body.appendChild(__cspFrame);
                __cspFrame.csp = "script-src 'unsafe-inline'; policy-after-source";
                if ({refresh:?} === 'name') __cspFrame.name = 'refreshed-owner-name';
                else void __cspFrame.contentWindow;
            "#
            ))?;
            run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                &mut page_vm,
                ChildFrameSemanticTurnKind::NavigationCommit,
                "attribute policy snapshot request start",
            )
            .await;
            wait_for_page_resource_completion(&mut queue, &mut wake_rx, "CSP snapshot fetch").await;
            page_vm
                .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
                .expect("snapshot response should apply to its exact navigation");
            let network = page_vm.take_completed_child_document_networks();
            assert_eq!(network.len(), 1);
            let requirement: Vec<_> = network[0]
                .snapshot
                .request_headers
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case("Sec-Required-CSP"))
                .map(|(_, value)| value.as_str())
                .collect();
            assert_eq!(
                requirement,
                ["script-src 'unsafe-inline'; policy-before-source"]
            );
            assert_eq!(
                page_vm.vm_mut().eval("__cspFrame.csp")?,
                "script-src 'unsafe-inline'; policy-after-source",
                "capturing request policy must not change DOM reflection"
            );
            assert!(!queue.has_ready_completion());
            server
                .await
                .expect("CSP snapshot response server should finish");
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("CSP navigation target snapshot test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn child_document_response_frame_ancestors_gates_commit() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/allowed-child.html",
                "HTTP/1.1 200 OK\r\nContent-Security-Policy: frame-ancestors *",
                "<!doctype html><p id='allowed-child'>allowed</p>".to_owned(),
                Duration::ZERO,
            ),
            (
                "/blocked-child.html",
                "HTTP/1.1 200 OK\r\nContent-Security-Policy: frame-ancestors 'none'",
                "<!doctype html><script>parent.__blockedChildExecuted = true</script>\
                 <p id='must-not-commit'>blocked</p>"
                    .to_owned(),
                Duration::ZERO,
            ),
        ])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, mut queue, mut wake_rx) = owner_attached_page_vm(
            &loader,
            Url::parse(&format!("{base_url}/page.html")).expect("page URL"),
        );

        let allowed_url = format!("{base_url}/allowed-child.html");
        start_external_child_document_load(&mut page_vm, "frame-ancestors-allowed", &allowed_url)
            .await;
        wait_for_page_resource_completion(&mut queue, &mut wake_rx, "allowed child fetch").await;
        page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("allowed child terminal should apply");
        assert_eq!(
            page_vm.vm_mut().eval(
                "document.getElementById('frame-ancestors-allowed').contentDocument\
                 .getElementById('allowed-child').textContent"
            )?,
            "allowed"
        );

        page_vm
            .vm_mut()
            .eval("globalThis.__blockedChildExecuted = false")?;
        let blocked_url = format!("{base_url}/blocked-child.html");
        start_external_child_document_load(&mut page_vm, "frame-ancestors-blocked", &blocked_url)
            .await;
        wait_for_page_resource_completion(&mut queue, &mut wake_rx, "blocked child fetch").await;
        page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("blocked child terminal should settle");
        assert_eq!(
            page_vm.vm_mut().eval(
                r#"(() => {
                    const frame = document.getElementById("frame-ancestors-blocked");
                    try {
                        void frame.contentWindow.location.href;
                        return "accessible";
                    } catch (_) {
                        return "blocked";
                    }
                })()"#
            )?,
            "blocked",
            "a blocked response must leave no ancestor-accessible child Document"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__blockedChildExecuted)")?,
            "false",
            "blocked response script must never execute"
        );

        server
            .await
            .expect("frame-ancestors child server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("frame-ancestors response gate test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn required_csp_rejection_commits_an_opaque_document_without_rewriting_network_facts() {
    run_page_vm_async_test(async move {
        for (status, status_line) in [
            (200, "HTTP/1.1 200 OK\r\nTiming-Allow-Origin: *"),
            (204, "HTTP/1.1 204 No Content\r\nTiming-Allow-Origin: *"),
            (205, "HTTP/1.1 205 Reset Content\r\nTiming-Allow-Origin: *"),
        ] {
            let body = "<!doctype html><script>parent.__rejectedBodyRan = true</script>";
            let (base_url, server) = spawn_path_response_http_server(vec![
                ("/reject", status_line, body.to_owned(), Duration::ZERO),
                ("/accepted", "HTTP/1.1 200 OK", "<!doctype html><p id='accepted'>accepted</p>".to_owned(), Duration::ZERO),
            ]).await;
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
            let (mut page_vm, mut queue, mut wake_rx) = owner_attached_page_vm(
                &loader, Url::parse(&format!("{base_url}/page.html"))?,
            );
            let rejected_url = format!("{base_url}/reject");
            page_vm.vm_mut().eval(&format!(r#"
                globalThis.__rejectedBodyRan = false;
                globalThis.__blockedLoads = 0;
                globalThis.__policyFrame = document.createElement('iframe');
                __policyFrame.id = 'required-policy-frame';
                __policyFrame.csp = "default-src 'none'";
                __policyFrame.onload = () => {{ __blockedLoads++; }};
                __policyFrame.src = {rejected_url:?};
                document.body.appendChild(__policyFrame);
                globalThis.__policyProxy = __policyFrame.contentWindow;
            "#))?;
            run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                &mut page_vm, ChildFrameSemanticTurnKind::NavigationCommit, "required policy rejection start",
            ).await;
            wait_for_page_resource_completion(&mut queue, &mut wake_rx, "required policy response").await;
            page_vm.apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
                .expect("the blocked response must still apply to its current owner");
            run_child_interactive_domcontentloaded_then_host_load_for_wait(&mut page_vm, "blocked error Document load").await;
            assert_eq!(page_vm.vm_mut().eval(r#"(() => {
                if (__policyFrame.contentDocument !== null || __policyFrame.contentWindow !== __policyProxy || __rejectedBodyRan || __blockedLoads !== 1) return false;
                try { void __policyProxy.location.href; } catch (e) { return e.name === 'SecurityError'; }
                return false;
            })()"#)?, "true", "an inert opaque error Document must replace the child while retaining WindowProxy identity and dispatching load");
            assert!(page_vm.take_completed_child_document_networks().is_empty(),
                "the committed error Document carries its response in the navigation projection, not the historical-only network queue");
            let navigations = page_vm.take_completed_child_frame_navigation_loads();
            assert_eq!(navigations.len(), 1);
            assert!(navigations[0].security_origin_opaque);
            let network = navigations[0].document_network.as_ref().expect("blocked navigation must retain its real response");
            assert_eq!(network.status, status);
            assert_eq!(network.request_url, rejected_url);
            assert!(network.resource_timing_allowed && network.resource_timing_body_visible);
            assert!(network.resource_timing_response.is_some());
            let expected_body = if status == 200 { body.as_bytes() } else { b"" };
            assert_eq!(network.response_body.as_ref().unwrap().try_bytes()?.as_ref(), expected_body);
            let entries: serde_json::Value = serde_json::from_str(&page_vm.vm_mut().eval(
                &format!("JSON.stringify(performance.getEntriesByName({rejected_url:?}, 'resource').map(e => e.toJSON()))"),
            )?)?;
            let entries = entries.as_array().unwrap();
            assert_eq!(entries.len(), 1);
            let entry = &entries[0];
            for name in ["responseStatus", "requestStart", "responseStart", "transferSize", "encodedBodySize", "decodedBodySize"] {
                assert_eq!(entry[name], 0, "blocked Document {name} must be hidden even with same-origin/TAO permission");
            }
            assert_eq!(entry["nextHopProtocol"], "");
            assert!(entry["responseEnd"].as_f64().unwrap() >= entry["fetchStart"].as_f64().unwrap());
            let accepted_url = format!("{base_url}/accepted");
            page_vm.vm_mut().eval(&format!("__policyFrame.removeAttribute('csp'); __policyFrame.src = {accepted_url:?};"))?;
            let retired_navigation = page_vm.run_child_navigation_commit_body_for_test()?
                .expect("the superseded navigation retains its exact stale-discard turn");
            assert!(matches!(retired_navigation.action.target_effect,
                crate::page_task_queue::PageChildNavigationCommitTargetEffect::DiscardedStaleOwner { current_owner: Some(_) }));
            let replacement_navigation = page_vm.run_child_navigation_commit_body_for_test()?
                .expect("the replacement navigation retains its own current-owner turn");
            assert!(matches!(replacement_navigation.action.target_effect,
                crate::page_task_queue::PageChildNavigationCommitTargetEffect::AppliedToCurrentOwner));
            let handle = page_vm.vm().element_handle_by_id_for_test("required-policy-frame").unwrap();
            assert!(page_vm.vm().current_child_document_navigation_fetch_target(handle).is_some(),
                "navigation from HTTP {status} error Document must start a real request");
            wait_for_page_resource_completion(&mut queue, &mut wake_rx, &format!("response after HTTP {status} error Document")).await;
            page_vm.apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?.expect("replacement response");
            run_child_interactive_domcontentloaded_then_host_load_for_wait(&mut page_vm, "accepted replacement Document load").await;
            assert_eq!(page_vm.vm_mut().eval("String(__policyFrame.contentWindow === __policyProxy && __policyProxy.document.getElementById('accepted').textContent === 'accepted' && __blockedLoads === 2)")?, "true", "a later accepted navigation must restore its own tuple origin, not inherit the error Document's opaque origin");
            server.await.expect("policy response server should finish");
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("required CSP blocked Document lifecycle test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn failed_child_document_fetch_is_applied_only_to_its_exact_current_request() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&dns_failure_fetch_config())
            .expect("loader");
        let (mut page_vm, mut queue, mut wake_rx) = owner_attached_page_vm(
            &loader,
            Url::parse("http://127.0.0.1/page.html").expect("page URL"),
        );
        let (handle, target) = start_external_child_document_load(
            &mut page_vm,
            "failed-child-frame",
            "http://127.0.0.1:1/unreachable.html",
        )
        .await;

        wait_for_page_resource_completion(&mut queue, &mut wake_rx, "failed child fetch").await;
        assert_eq!(
            queue.next_ready_owner(),
            Some(
                RendererPageResourceCompletionOwner::child_document_navigation(
                    page_vm.document_lifecycle.identity().document,
                    target,
                )
            )
        );
        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("failed current child request should still consume one typed turn");
        assert_eq!(
            outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            page_vm
                .vm()
                .current_child_document_navigation_fetch_target(handle),
            None
        );
        assert!(
            page_vm.take_completed_child_document_networks().is_empty(),
            "a transport error without a response must not invent a historical response fact"
        );
        assert!(!queue.has_ready_completion());
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("failed typed child-document test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn response_for_replaced_child_document_is_historical_network_only() {
    run_page_vm_async_test(async move {
        for (status, status_line) in [
            (200, "HTTP/1.1 200 OK"),
            (204, "HTTP/1.1 204 No Content"),
            (205, "HTTP/1.1 205 Reset Content"),
        ] {
            let (base_url, server) = spawn_path_response_http_server(vec![(
                "/retired-child.html",
                status_line,
                if status == 200 {
                    "<!doctype html><p id='must-not-commit'>retired response</p>".to_owned()
                } else {
                    String::new()
                },
                Duration::from_millis(100),
            )])
            .await;
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
                .expect("loader");
            let (mut page_vm, mut queue, mut wake_rx) = owner_attached_page_vm(
                &loader,
                Url::parse(&format!("{base_url}/page.html")).expect("page URL"),
            );
            let root_document = page_vm.document_lifecycle.identity().document;
            let retired_url = format!("{base_url}/retired-child.html");
            let (handle, retired_target) = start_external_child_document_load(
                &mut page_vm,
                "replacement-child-frame",
                &retired_url,
            )
            .await;

            page_vm.vm_mut().eval(
                "document.getElementById('replacement-child-frame').srcdoc = \
             \"<p id='replacement-child'>replacement</p>\";",
            )?;
            run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                &mut page_vm,
                ChildFrameSemanticTurnKind::NavigationCommit,
                "replacement srcdoc commit",
            )
            .await;
            assert_ne!(
                page_vm
                    .vm()
                    .current_child_document_navigation_fetch_target(handle),
                Some(retired_target),
                "replacement must retire the exact fetch target before its response arrives"
            );
            let _ = page_vm.take_completed_child_frame_navigation_loads();
            while wake_rx.try_recv().is_ok() {}
            wait_for_page_resource_completion(&mut queue, &mut wake_rx, "retired child fetch")
                .await;
            let activity_epoch_before = page_vm.vm().subresource_activity_epoch();

            let outcome = page_vm
                .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
                .expect("retired response should consume one stale-discard turn");
            assert_eq!(
                outcome.action.owner,
                RendererPageResourceCompletionOwner::child_document_navigation(
                    root_document,
                    retired_target,
                )
            );
            assert!(matches!(
                outcome.action.document_effect,
                PageResourceCompletionDocumentEffect::DiscardedStaleOwner { .. }
            ));
            assert_eq!(
                page_vm.vm().subresource_activity_epoch(),
                activity_epoch_before,
                "historical child response must not become replacement Document activity"
            );
            assert_eq!(
                page_vm.vm_mut().eval(
                    "document.getElementById('replacement-child-frame').contentDocument\
                 .getElementById('replacement-child').textContent"
                )?,
                "replacement"
            );
            assert_eq!(
                page_vm.vm_mut().eval(
                    "String(document.getElementById('replacement-child-frame').contentDocument\
                 .getElementById('must-not-commit'))"
                )?,
                "null"
            );
            let historical = page_vm.take_completed_child_document_networks();
            assert_eq!(historical.len(), 1);
            assert_eq!(historical[0].snapshot.request_url, retired_url);
            assert_eq!(historical[0].snapshot.status, status);
            assert_eq!(
                page_vm.vm_mut().eval(&format!(
                    "String(performance.getEntriesByName({retired_url:?}, 'resource').length)"
                ))?,
                "0",
                "a stale fetch cannot add timing to the replacement owner"
            );
            assert!(
                page_vm
                    .take_completed_child_frame_navigation_loads()
                    .is_empty(),
                "stale response must not synthesize a child navigation commit/lifecycle snapshot"
            );

            server.await.expect("retired child server should finish");
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("same-Page stale child-document test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn stale_same_root_terminal_does_not_settle_newer_exact_navigation() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_concurrent_path_response_http_server(vec![
            (
                "/older-child.html",
                "HTTP/1.1 200 OK",
                "<!doctype html><p id='older-child'>older</p>".to_owned(),
                Duration::from_millis(50),
            ),
            (
                "/newer-child.html",
                "HTTP/1.1 200 OK",
                "<!doctype html><p id='newer-child'>newer</p>".to_owned(),
                Duration::from_millis(150),
            ),
        ])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, mut queue, mut wake_rx) = owner_attached_page_vm(
            &loader,
            Url::parse(&format!("{base_url}/page.html")).expect("page URL"),
        );
        let root_document = page_vm.document_lifecycle.identity().document;
        let older_url = format!("{base_url}/older-child.html");
        let newer_url = format!("{base_url}/newer-child.html");
        let (handle, older_target) = start_external_child_document_load(
            &mut page_vm,
            "same-root-navigation-frame",
            &older_url,
        )
        .await;
        page_vm.vm_mut().eval(&format!(
            "document.getElementById('same-root-navigation-frame').src = {newer_url:?};"
        ))?;
        let stale_navigation_outcome = page_vm
            .run_child_navigation_commit_body_for_test()?
            .expect("superseded child navigation should retain one stale-discard turn");
        assert!(matches!(
            stale_navigation_outcome.action.target_effect,
            crate::page_task_queue::PageChildNavigationCommitTargetEffect::DiscardedStaleOwner {
                current_owner: Some(_)
            }
        ));
        let newer_navigation_outcome = page_vm
            .run_child_navigation_commit_body_for_test()?
            .expect("newer exact child navigation should retain its own typed turn");
        assert!(matches!(
            newer_navigation_outcome.action.target_effect,
            crate::page_task_queue::PageChildNavigationCommitTargetEffect::AppliedToCurrentOwner
        ));
        let newer_target = page_vm
            .vm()
            .current_child_document_navigation_fetch_target(handle)
            .expect("newer navigation must expose its exact request target");
        assert_ne!(older_target, newer_target);
        while wake_rx.try_recv().is_ok() {}
        wait_for_page_resource_completion(&mut queue, &mut wake_rx, "older child response").await;
        let activity_epoch_before_older_terminal = page_vm.vm().subresource_activity_epoch();

        let older_outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("older response should consume one stale-discard turn");
        assert_eq!(
            older_outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(
                    RendererPageResourceCompletionOwner::child_document_navigation(
                        root_document,
                        newer_target,
                    ),
                ),
            }
        );
        assert_eq!(
            page_vm
                .vm()
                .current_child_document_navigation_fetch_target(handle),
            Some(newer_target),
            "same-root stale cleanup must match the entire target before settling state"
        );
        assert_eq!(
            page_vm.vm().subresource_activity_epoch(),
            activity_epoch_before_older_terminal
        );
        let historical = page_vm.take_completed_child_document_networks();
        assert_eq!(historical.len(), 1);
        assert_eq!(historical[0].snapshot.request_url, older_url);

        wait_for_page_resource_completion(&mut queue, &mut wake_rx, "newer child response").await;
        let newer_outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("newer response should retain its own application authority");
        assert_eq!(
            newer_outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            page_vm.vm_mut().eval(
                "document.getElementById('same-root-navigation-frame').contentDocument\
                 .getElementById('newer-child').textContent"
            )?,
            "newer"
        );

        server
            .await
            .expect("same-root navigation server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("same-root exact child navigation test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn nested_stale_child_response_retains_producer_captured_parent_frame() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/nested-retired-child.html",
            "HTTP/1.1 200 OK",
            "<!doctype html><p>retired nested response</p>".to_owned(),
            Duration::from_millis(100),
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, mut queue, mut wake_rx) = owner_attached_page_vm(
            &loader,
            Url::parse(&format!("{base_url}/page.html")).expect("page URL"),
        );
        page_vm.vm_mut().eval(
            r#"
const outer = document.createElement("iframe");
outer.id = "nested-owner-frame";
outer.srcdoc = "<!doctype html><body></body>";
document.body.appendChild(outer);
"#,
        )?;
        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
            &mut page_vm,
            ChildFrameSemanticTurnKind::NavigationCommit,
            "outer nested-owner commit",
        )
        .await;
        let outer_attachments = page_vm.take_pending_child_frame_tree_events();
        assert_eq!(outer_attachments.len(), 1);
        let outer_frame_id = match &outer_attachments[0] {
            crate::protocol_types::ChildFrameTreeEventSnapshot::Attached(attachment) => {
                attachment.frame_id.clone()
            }
            crate::protocol_types::ChildFrameTreeEventSnapshot::Detached(_) => {
                panic!("outer child should first produce an attachment")
            }
        };
        let nested_url = format!("{base_url}/nested-retired-child.html");
        page_vm.vm_mut().eval(&format!(
            r#"
const nested = document.getElementById("nested-owner-frame")
  .contentDocument.createElement("iframe");
nested.id = "nested-network-frame";
nested.src = {nested_url:?};
document.getElementById("nested-owner-frame").contentDocument.body.appendChild(nested);
"#
        ))?;
        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
            &mut page_vm,
            ChildFrameSemanticTurnKind::NavigationCommit,
            "nested external navigation start",
        )
        .await;
        page_vm.vm_mut().eval(
            r#"
document.getElementById("nested-owner-frame").contentDocument
  .getElementById("nested-network-frame").srcdoc = "<p>nested replacement</p>";
"#,
        )?;
        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
            &mut page_vm,
            ChildFrameSemanticTurnKind::NavigationCommit,
            "nested response replacement",
        )
        .await;
        let _ = page_vm.take_completed_child_frame_navigation_loads();
        while wake_rx.try_recv().is_ok() {}
        wait_for_page_resource_completion(&mut queue, &mut wake_rx, "nested stale fetch").await;

        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("nested stale terminal should consume one turn");
        assert!(matches!(
            outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner { .. }
        ));
        let historical = page_vm.take_completed_child_document_networks();
        assert_eq!(historical.len(), 1);
        assert_eq!(historical[0].snapshot.request_url, nested_url);
        assert_eq!(
            historical[0].parent_frame_id.as_deref(),
            Some(outer_frame_id.as_str()),
            "Network attribution must retain the producer-time nested parent, not rediscover it after replacement"
        );
        assert_ne!(historical[0].frame_id, outer_frame_id);
        assert!(page_vm.take_completed_child_frame_navigation_loads().is_empty());

        server.await.expect("nested stale child server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("nested child-document attribution test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn unload_navigation_supersedes_authorized_terminal_during_application() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/unload-race.html",
            "HTTP/1.1 200 OK",
            "<!doctype html><p id='must-not-win'>old terminal</p>".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, mut queue, mut wake_rx) = owner_attached_page_vm(
            &loader,
            Url::parse(&format!("{base_url}/page.html")).expect("page URL"),
        );
        page_vm.vm_mut().eval(
            r#"
const frame = document.createElement("iframe");
frame.id = "unload-race-frame";
frame.srcdoc = "<p>initial</p>";
document.body.appendChild(frame);
"#,
        )?;
        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
            &mut page_vm,
            ChildFrameSemanticTurnKind::NavigationCommit,
            "initial srcdoc commit",
        )
        .await;
        run_child_interactive_domcontentloaded_then_host_load_for_wait(
            &mut page_vm,
            "initial srcdoc lifecycle",
        )
        .await;
        let target_url = format!("{base_url}/unload-race.html");
        page_vm.vm_mut().eval(&format!(
            r#"
const raceFrame = document.getElementById("unload-race-frame");
globalThis.__unloadRaceFired = false;
raceFrame.contentWindow.addEventListener("unload", () => {{
  globalThis.__unloadRaceFired = true;
  raceFrame.srcdoc = "<p id='unload-winner'>unload winner</p>";
}});
raceFrame.src = {target_url:?};
raceFrame.removeAttribute("srcdoc");
"#
        ))?;
        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
            &mut page_vm,
            ChildFrameSemanticTurnKind::NavigationCommit,
            "network navigation start",
        )
        .await;
        while wake_rx.try_recv().is_ok() {}
        wait_for_page_resource_completion(&mut queue, &mut wake_rx, "unload-race fetch").await;

        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("authorized terminal should consume one bounded turn");
        let unload_fired = page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test("String(globalThis.__unloadRaceFired)")?;
        assert!(
            matches!(
                outcome.action.document_effect,
                PageResourceCompletionDocumentEffect::SupersededDuringApplication { .. }
            ),
            "unload handler fired={unload_fired}; outcome={outcome:?}"
        );
        assert_eq!(
            outcome.action.body_activity,
            PageResourceCompletionBodyActivity::PageCodeOrEventDispatchAttempted,
            "the superseding unload callback must remain visible to task completion",
        );
        assert_eq!(
            outcome.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired,
            "the completed response remains historical after unload supersedes its commit"
        );
        let historical = page_vm.take_completed_child_document_networks();
        assert_eq!(historical.len(), 1);
        assert_eq!(historical[0].snapshot.request_url, target_url);

        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
            &mut page_vm,
            ChildFrameSemanticTurnKind::NavigationCommit,
            "unload handler replacement commit",
        )
        .await;
        assert_eq!(
            page_vm.vm_mut().eval(
                "document.getElementById('unload-race-frame').contentDocument\
                 .getElementById('unload-winner').textContent"
            )?,
            "unload winner"
        );
        assert_eq!(
            page_vm.vm_mut().eval(
                "String(document.getElementById('unload-race-frame').contentDocument\
                 .getElementById('must-not-win'))"
            )?,
            "null"
        );

        server
            .await
            .expect("unload-race child server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("unload reentrancy child-document test should run");
}

pub(super) fn stale_loaded_completion(
    target: ChildDocumentNavigationFetchTarget,
    frame_id: &str,
    request_url: &str,
) -> ChildDocumentLoadCompletion {
    let loader_id = format!("TEST-CHILD-LOADER-{}", target.load_id());
    ChildDocumentLoadCompletion::new(
        target,
        ChildDocumentLoadNetworkAttribution::new(frame_id.to_owned(), None, loader_id),
        Ok(ChildDocumentLoadOutcome::Loaded(Box::new(
            LoadedChildDocument {
                response_status: None,
                response_links: Default::default(),
                final_url: Url::parse(request_url).expect("final URL"),
                policy_container: crate::document_runtime::DocumentPolicyContainer::default(),
                content_type: Some("text/html".to_owned()),
                character_set: "UTF-8".to_owned(),
                content: "<!doctype html>".to_owned().into(),
                document_network: Some(crate::protocol_types::ChildFrameDocumentNetworkSnapshot {
                    request_url: request_url.to_owned(),
                    request_method: "GET".to_owned(),
                    request_headers: Vec::new(),
                    final_url: request_url.to_owned(),
                    status: 200,
                    response_headers: Vec::new(),
                    encoded_data_length: 0,
                    response_body: None,
                    from_cache: false,
                    cache_state: Default::default(),
                    preload_state: Default::default(),
                    resource_timing_body_visible: false,
                    resource_timing_response: None,
                    resource_timing_allowed: false,
                    resource_timing_redirected: false,
                    resource_timing_protocol: None,
                }),
            },
        ))),
    )
}

#[tokio::test(flavor = "current_thread")]
async fn child_document_terminals_are_fifo_and_one_per_page_turn() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let root_document = page_vm.document_lifecycle.identity().document;
        let owner = FrameDocumentTaskOwner::new(
            FrameSchedulerLaneId(901),
            LocalWindowId(902),
            DocumentId(903),
        );
        let target = |load_id| {
            ChildDocumentNavigationFetchTarget::for_test(
                crate::dom::native::NativeNodeId::new(904),
                owner,
                load_id,
                FrameRequestId(load_id),
            )
        };
        let mut queue = RendererPageNetworkingSource::new_for_test();
        queue.enqueue_local_for_test(RendererPageResourceCompletion::child_document_load(
            root_document,
            stale_loaded_completion(
                target(1),
                "fifo-child-frame",
                "https://fifo-child.test/first.html",
            ),
        ));
        queue.enqueue_local_for_test(RendererPageResourceCompletion::child_document_load(
            root_document,
            stale_loaded_completion(
                target(2),
                "fifo-child-frame",
                "https://fifo-child.test/second.html",
            ),
        ));
        assert_eq!(
            queue.next_ready_owner(),
            Some(
                RendererPageResourceCompletionOwner::child_document_navigation(
                    root_document,
                    target(1),
                )
            )
        );

        page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("first terminal should consume one turn");

        let first_network = page_vm.take_completed_child_document_networks();
        assert_eq!(first_network.len(), 1);
        assert_eq!(
            first_network[0].snapshot.request_url,
            "https://fifo-child.test/first.html"
        );
        assert!(queue.has_ready_completion());

        page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("second terminal should require its own turn");

        let second_network = page_vm.take_completed_child_document_networks();
        assert_eq!(second_network.len(), 1);
        assert_eq!(
            second_network[0].snapshot.request_url,
            "https://fifo-child.test/second.html"
        );
        assert!(!queue.has_ready_completion());
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child-document FIFO test should run");
}
