use super::*;

use crate::native_bridge::{
    LightweightPopupDocumentFetchTarget, LightweightPopupNavigationTaskToken,
};
use crate::types::{LoadedChildDocument, PopupDocumentLoadCompletion, PopupDocumentLoadOutcome};

fn owner_attached_popup_page_vm(
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

async fn wait_for_popup_terminal(
    queue: &mut impl crate::page_resource_completion::RendererPageResourceCompletionTestSource,
    wake_rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::page_task_queue::RendererOwnerWake>,
    label: &str,
) {
    super::child_document_completion::wait_for_page_resource_completion(queue, wake_rx, label)
        .await;
}

fn open_popup(page_vm: &mut PageVm, url: &str, target_name: &str, global_name: &str) -> u64 {
    page_vm
        .vm_mut()
        .eval(&format!(
            "globalThis[{global_name:?}] = open({url:?}, {target_name:?}); \
             String(globalThis[{global_name:?}] !== null)"
        ))
        .expect("popup navigation should start");
    page_vm
        .vm_mut()
        .take_pending_popup_activations()
        .into_iter()
        .last()
        .and_then(|activation| activation.popup_id())
        .expect("popup navigation should publish its browsing-context id")
}

fn queued_popup_target(
    queue: &mut impl crate::page_resource_completion::RendererPageResourceCompletionTestSource,
) -> LightweightPopupDocumentFetchTarget {
    let owner = queue
        .next_ready_owner()
        .expect("ready popup terminal must retain its exact owner");
    let crate::page_resource_completion::RendererPageResourceCompletionLocalOwner::PopupDocumentLoad(
        target,
    ) = owner.local_owner()
    else {
        panic!("expected popup completion owner, got {owner:?}");
    };
    target
}

fn loaded_popup_completion(
    target: LightweightPopupDocumentFetchTarget,
    final_url: &str,
    markup: &str,
) -> PopupDocumentLoadCompletion {
    PopupDocumentLoadCompletion::new(
        target,
        Ok(PopupDocumentLoadOutcome::Loaded(Box::new(
            LoadedChildDocument {
                final_url: Url::parse(final_url).expect("popup completion final URL"),
                policy_container: crate::document_runtime::DocumentPolicyContainer::default(),
                content_type: Some("text/html".to_owned()),
                character_set: "UTF-8".to_owned(),
                markup: markup.to_owned(),
                document_network: None,
            },
        ))),
    )
}

#[tokio::test(flavor = "current_thread")]
async fn production_popup_fetch_reaches_stable_typed_turn_and_commits() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/typed-popup.html",
            "HTTP/1.1 200 OK",
            "<!doctype html><script>opener.__typedPopupScript = 'ran';</script><p id='typed-popup-loaded'>typed popup</p>".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, mut queue, mut wake_rx) = owner_attached_popup_page_vm(
            &loader,
            Url::parse(&format!("{base_url}/page.html")).expect("page URL"),
        );
        let popup_url = format!("{base_url}/typed-popup.html");
        let _popup_id = open_popup(
            &mut page_vm,
            &popup_url,
            "typed-popup",
            "__typedPopup",
        );

        wait_for_popup_terminal(&mut queue, &mut wake_rx, "typed popup fetch").await;
        let target = queued_popup_target(&mut queue);
        assert_eq!(
            page_vm
                .vm()
                .current_lightweight_popup_document_fetch_target(target.load_id()),
            Some(target)
        );

        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("current popup terminal should consume one typed Page turn");
        assert_eq!(
            outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            outcome.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );
        assert_eq!(
            outcome.action.source,
            RendererOwnerResourceActivitySource::PopupDocument
        );
        assert_eq!(
            page_vm.vm_mut().eval("__typedPopup.document.getElementById('typed-popup-loaded').textContent")?,
            "typed popup"
        );
        assert_eq!(page_vm.vm_mut().eval("String(__typedPopupScript)")?, "ran");
        assert!(!page_vm.vm().has_pending_lightweight_popup_document_loads());
        assert!(!queue.has_ready_completion());

        server.await.expect("typed popup server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("production typed popup test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn popup_document_completion_body_does_not_own_the_outer_task_checkpoint() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/body-only-popup.html",
            "HTTP/1.1 200 OK",
            r#"<!doctype html><script>
opener.__popupResourceBodyEvents = ["script"];
Promise.resolve().then(() => opener.__popupResourceBodyEvents.push("microtask"));
</script>"#
                .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, mut queue, mut wake_rx) = owner_attached_popup_page_vm(
            &loader,
            Url::parse(&format!("{base_url}/page.html")).expect("page URL"),
        );
        page_vm
            .vm_mut()
            .eval("globalThis.__popupResourceBodyEvents = []; 'ready'")?;
        open_popup(
            &mut page_vm,
            &format!("{base_url}/body-only-popup.html"),
            "body-only-popup",
            "__bodyOnlyPopup",
        );

        wait_for_popup_terminal(&mut queue, &mut wake_rx, "body-only popup fetch").await;
        page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("popup resource body should consume one typed terminal");
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "JSON.stringify(globalThis.__popupResourceBodyEvents)",
                )?,
            r#"["script"]"#,
            "resource application must leave the enclosing task-end checkpoint to the selected dispatcher",
        );

        server
            .await
            .expect("body-only popup server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("popup resource body-only checkpoint witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_popup_document_completion_submits_checkpoint_without_runtime_drain() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/selected-popup.html",
            "HTTP/1.1 200 OK",
            r#"<!doctype html><script>
opener.__selectedPopupEvents = ["script"];
Promise.resolve().then(() => opener.__selectedPopupEvents.push("microtask"));
</script>"#
                .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, mut queue, mut wake_rx) = owner_attached_popup_page_vm(
            &loader,
            Url::parse(&format!("{base_url}/page.html")).expect("page URL"),
        );
        page_vm
            .vm_mut()
            .eval("globalThis.__selectedPopupEvents = []; 'ready'")?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
        open_popup(
            &mut page_vm,
            &format!("{base_url}/selected-popup.html"),
            "selected-popup",
            "__selectedPopup",
        );

        wait_for_popup_terminal(&mut queue, &mut wake_rx, "selected popup fetch").await;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ResourceCompletion,
                    &loader,
                )
                .await?,
            "popup terminal must enter the production selected dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "JSON.stringify(globalThis.__selectedPopupEvents)",
                )?,
            r#"["script","microtask"]"#,
            "the popup script reaction must run at the enclosing resource-task boundary"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "popup completion must not drain unrelated main-Document runtime residence"
        );

        server.await.expect("selected popup server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("selected popup resource checkpoint witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn popup_window_open_self_javascript_stays_in_popup_browsing_context() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/window-open-self-popup.html",
            "HTTP/1.1 200 OK",
            r#"<!doctype html><script>
window.open("javascript:window.opener.__popupSelfEvents.push(document.defaultView === window ? 'popup' : 'wrong')", "_self");
</script>"#
                .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, mut queue, mut wake_rx) = owner_attached_popup_page_vm(
            &loader,
            Url::parse(&format!("{base_url}/page.html")).expect("page URL"),
        );
        page_vm
            .vm_mut()
            .eval("globalThis.__popupSelfEvents = []; 'ready'")?;
        let popup_url = format!("{base_url}/window-open-self-popup.html");
        open_popup(
            &mut page_vm,
            &popup_url,
            "window-open-self-popup",
            "__windowOpenSelfPopup",
        );

        wait_for_popup_terminal(&mut queue, &mut wake_rx, "window.open _self popup").await;
        page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("popup document completion should consume one typed Page turn");

        assert!(
            !page_vm.vm().has_pending_location_navigation(),
            "popup window.open(..., '_self') must not navigate the main Page"
        );
        assert!(
            page_vm.vm().has_ready_timeout(),
            "popup javascript: navigation must publish its asynchronous popup task"
        );

        server
            .await
            .expect("window.open _self popup server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("popup window.open _self routing test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn popup_noopener_self_link_stays_in_popup_browsing_context() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/noopener-self-popup.html",
            "HTTP/1.1 200 OK",
            r#"<!doctype html>
<a id="self-link" rel="noopener" target="_self"
   href="javascript:window.opener.__popupNoopenerSelfEvents.push(document.defaultView === window ? 'popup' : 'wrong')">self</a>
<script>document.getElementById('self-link').click();</script>"#
                .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, mut queue, mut wake_rx) = owner_attached_popup_page_vm(
            &loader,
            Url::parse(&format!("{base_url}/page.html")).expect("page URL"),
        );
        page_vm
            .vm_mut()
            .eval("globalThis.__popupNoopenerSelfEvents = []; 'ready'")?;
        let popup_url = format!("{base_url}/noopener-self-popup.html");
        open_popup(
            &mut page_vm,
            &popup_url,
            "noopener-self-popup",
            "__noopenerSelfPopup",
        );

        wait_for_popup_terminal(&mut queue, &mut wake_rx, "noopener _self popup link").await;
        page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("popup document completion should consume one typed Page turn");

        assert!(
            !page_vm.vm().has_pending_location_navigation(),
            "a popup link targeting _self must not navigate the main Page"
        );
        assert!(
            page_vm.vm().has_ready_timeout(),
            "popup link javascript: navigation must publish its asynchronous popup task"
        );

        server
            .await
            .expect("noopener _self popup server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("popup noopener _self link routing test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn ignored_popup_response_settles_only_its_exact_current_load() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/ignored-popup.html",
            "HTTP/1.1 204 No Content",
            String::new(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, mut queue, mut wake_rx) = owner_attached_popup_page_vm(
            &loader,
            Url::parse(&format!("{base_url}/page.html")).expect("page URL"),
        );
        let popup_url = format!("{base_url}/ignored-popup.html");
        let _popup_id = open_popup(&mut page_vm, &popup_url, "ignored-popup", "__ignoredPopup");

        wait_for_popup_terminal(&mut queue, &mut wake_rx, "ignored popup response").await;
        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("ignored response should still consume its exact terminal turn");
        assert_eq!(
            outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            page_vm.vm_mut().eval("__ignoredPopup.location.href")?,
            "about:blank"
        );
        assert!(!page_vm.vm().has_pending_lightweight_popup_document_loads());

        server.await.expect("ignored popup server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("ignored typed popup test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn failed_popup_fetch_settles_only_its_exact_current_load() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&dns_failure_fetch_config())
            .expect("loader");
        let (mut page_vm, mut queue, mut wake_rx) = owner_attached_popup_page_vm(
            &loader,
            Url::parse("http://127.0.0.1/page.html").expect("page URL"),
        );
        open_popup(
            &mut page_vm,
            "http://127.0.0.1:1/unreachable-popup.html",
            "failed-popup",
            "__failedPopup",
        );

        wait_for_popup_terminal(&mut queue, &mut wake_rx, "failed popup fetch").await;
        let target = queued_popup_target(&mut queue);
        assert_eq!(
            page_vm
                .vm()
                .current_lightweight_popup_document_fetch_target(target.load_id()),
            Some(target)
        );
        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("failed current popup request should consume one typed turn");
        assert_eq!(
            outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            outcome.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );
        assert!(
            !page_vm.vm().has_pending_lightweight_popup_document_loads(),
            "transport failure must settle the exact pending popup load"
        );
        assert_eq!(
            page_vm
                .vm()
                .current_lightweight_popup_document_fetch_target(target.load_id()),
            None
        );
        assert!(!queue.has_ready_completion());
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("failed typed popup test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn queued_popup_response_is_stale_after_newer_navigation_and_cannot_commit() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/older-popup.html",
                "HTTP/1.1 200 OK",
                "<!doctype html><p id='older-popup'>older</p>".to_owned(),
                Duration::ZERO,
            ),
            (
                "/newer-popup.html",
                "HTTP/1.1 200 OK",
                "<!doctype html><p id='newer-popup'>newer</p>".to_owned(),
                Duration::ZERO,
            ),
        ])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, mut queue, mut wake_rx) = owner_attached_popup_page_vm(
            &loader,
            Url::parse(&format!("{base_url}/page.html")).expect("page URL"),
        );
        let older_url = format!("{base_url}/older-popup.html");
        let newer_url = format!("{base_url}/newer-popup.html");
        open_popup(&mut page_vm, &older_url, "reused-popup", "__reusedPopup");
        wait_for_popup_terminal(&mut queue, &mut wake_rx, "older popup response").await;
        let older_target = queued_popup_target(&mut queue);

        let reopened = page_vm.vm_mut().eval(&format!(
            "String(open({newer_url:?}, 'reused-popup') === __reusedPopup)"
        ))?;
        assert_eq!(reopened, "true");
        let _ = page_vm.vm_mut().take_pending_popup_activations();
        assert_eq!(
            page_vm
                .vm()
                .current_lightweight_popup_document_fetch_target(older_target.load_id()),
            None,
            "newer navigation must retire the older exact popup load before application"
        );

        let older_outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("older response should consume one stale-discard turn");
        assert!(matches!(
            older_outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner { .. }
        ));
        assert_eq!(
            older_outcome.action.output_effect,
            PageResourceCompletionOutputEffect::None
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(__reusedPopup.document.getElementById('older-popup'))")?,
            "null"
        );

        wait_for_popup_terminal(&mut queue, &mut wake_rx, "newer popup response").await;
        let newer_target = queued_popup_target(&mut queue);
        assert_ne!(older_target, newer_target);
        let newer_outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("newer response should retain its own application authority");
        assert_eq!(
            newer_outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__reusedPopup.document.getElementById('newer-popup').textContent")?,
            "newer"
        );

        server.await.expect("reused popup server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("same-Page stale popup navigation test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn queued_popup_response_is_stale_after_browsing_context_close() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/closed-popup.html",
            "HTTP/1.1 200 OK",
            "<!doctype html><script>opener.__closedPopupScriptRan = true;</script>".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, mut queue, mut wake_rx) = owner_attached_popup_page_vm(
            &loader,
            Url::parse(&format!("{base_url}/page.html")).expect("page URL"),
        );
        let popup_url = format!("{base_url}/closed-popup.html");
        open_popup(&mut page_vm, &popup_url, "closed-popup", "__closedPopup");
        wait_for_popup_terminal(&mut queue, &mut wake_rx, "closed popup response").await;
        let target = queued_popup_target(&mut queue);
        page_vm.vm_mut().eval("__closedPopup.close(); 'closed'")?;

        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("closed popup response should consume one stale-discard turn");
        assert!(matches!(
            outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner { .. }
        ));
        assert_eq!(
            outcome.action.output_effect,
            PageResourceCompletionOutputEffect::None
        );
        assert_eq!(
            page_vm
                .vm()
                .current_lightweight_popup_document_fetch_target(target.load_id()),
            None
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__closedPopupScriptRan)")?,
            "undefined"
        );

        server.await.expect("closed popup server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("closed popup stale terminal test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn retired_root_namespace_rejects_an_identical_popup_local_target() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/namespace-popup.html",
            "HTTP/1.1 200 OK",
            "<!doctype html><p id='namespace-current'>current</p>".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, mut queue, mut wake_rx) = owner_attached_popup_page_vm(
            &loader,
            Url::parse(&format!("{base_url}/page.html")).expect("page URL"),
        );
        let popup_url = format!("{base_url}/namespace-popup.html");
        open_popup(
            &mut page_vm,
            &popup_url,
            "namespace-popup",
            "__namespacePopup",
        );
        wait_for_popup_terminal(&mut queue, &mut wake_rx, "namespace popup response").await;
        let target = queued_popup_target(&mut queue);
        let current_root = page_vm.document_lifecycle.identity().document;
        let retired_root = current_root.successor_for_testing();
        queue.enqueue_local_for_test(RendererPageResourceCompletion::popup_document_load(
            retired_root,
            loaded_popup_completion(
                target,
                &popup_url,
                "<!doctype html><p id='retired-root-must-not-commit'>retired</p>",
            ),
        ));
        let current_envelope = queue
            .pop_front()
            .expect("production current popup terminal should remain queued")
            .1;
        let retired_envelope = queue
            .pop_front()
            .expect("retired-root popup terminal should remain queued")
            .1;
        queue.enqueue_local_for_test(retired_envelope);
        queue.enqueue_local_for_test(current_envelope);

        let stale = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("retired root terminal should consume a stale-discard turn");
        assert_eq!(
            stale.action.owner,
            RendererPageResourceCompletionOwner::popup_document_load(retired_root, target)
        );
        assert_eq!(
            stale.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(RendererPageResourceCompletionOwner::popup_document_load(
                    current_root,
                    target,
                )),
            }
        );
        assert_eq!(
            page_vm
                .vm()
                .current_lightweight_popup_document_fetch_target(target.load_id()),
            Some(target),
            "root mismatch must not settle the current PageVm's colliding local popup target"
        );

        let current = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("current root terminal should retain application authority");
        assert_eq!(
            current.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            page_vm.vm_mut().eval(
                "__namespacePopup.document.getElementById('namespace-current').textContent"
            )?,
            "current"
        );
        assert_eq!(
            page_vm.vm_mut().eval(
                "String(__namespacePopup.document.getElementById('retired-root-must-not-commit'))"
            )?,
            "null"
        );

        server.await.expect("namespace popup server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("popup root namespace collision test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn mismatched_legacy_popup_target_cannot_consume_the_current_pending_load() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/authorization-popup.html",
            "HTTP/1.1 200 OK",
            "<!doctype html><p id='authorized-popup'>authorized</p>".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, mut queue, mut wake_rx) = owner_attached_popup_page_vm(
            &loader,
            Url::parse(&format!("{base_url}/page.html")).expect("page URL"),
        );
        let popup_url = format!("{base_url}/authorization-popup.html");
        open_popup(
            &mut page_vm,
            &popup_url,
            "authorization-popup",
            "__authorizationPopup",
        );
        wait_for_popup_terminal(&mut queue, &mut wake_rx, "authorization popup response").await;
        let envelope = queue
            .pop_front()
            .expect("production popup terminal should remain queued")
            .1;
        let crate::page_resource_completion::RendererPageResourceTerminal::PopupDocumentLoad {
            completion,
        } = envelope.into_terminal()
        else {
            panic!("expected popup terminal");
        };
        let completion = *completion;
        let current_target = completion.target();
        let mismatched_task = LightweightPopupNavigationTaskToken::for_test(
            current_target.task().document_owner(),
            999,
        );
        let mismatched_target = LightweightPopupDocumentFetchTarget::for_test(
            current_target.load_id(),
            mismatched_task,
        );

        page_vm
            .vm_mut()
            .complete_popup_document_load(loaded_popup_completion(
                mismatched_target,
                &popup_url,
                "<!doctype html><p id='forged-popup'>forged</p>",
            ))?;
        assert_eq!(
            page_vm
                .vm()
                .current_lightweight_popup_document_fetch_target(current_target.load_id()),
            Some(current_target),
            "a mismatched token sharing the load id must not remove the authorized pending load"
        );

        page_vm.vm_mut().complete_popup_document_load(completion)?;
        assert_eq!(
            page_vm.vm_mut().eval(
                "__authorizationPopup.document.getElementById('authorized-popup').textContent"
            )?,
            "authorized"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(__authorizationPopup.document.getElementById('forged-popup'))")?,
            "null"
        );

        server
            .await
            .expect("authorization popup server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("popup authorization boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn stale_popup_terminals_are_fifo_and_one_per_page_turn() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let root_document = page_vm.document_lifecycle.identity().document;
        let owner = crate::native_bridge::LightweightPopupDocumentOwner::new(
            71,
            crate::native_bridge::LightweightPopupDocumentId::new(73),
        );
        let target = |load_id, navigation_id| {
            LightweightPopupDocumentFetchTarget::for_test(
                load_id,
                LightweightPopupNavigationTaskToken::for_test(owner, navigation_id),
            )
        };
        let first_target = target(75, 77);
        let second_target = target(79, 81);
        let mut queue = RendererPageNetworkingSource::new_for_test();
        queue.enqueue_local_for_test(RendererPageResourceCompletion::popup_document_load(
            root_document,
            loaded_popup_completion(
                first_target,
                "https://popup-fifo.test/first.html",
                "<!doctype html><p>first</p>",
            ),
        ));
        queue.enqueue_local_for_test(RendererPageResourceCompletion::popup_document_load(
            root_document,
            loaded_popup_completion(
                second_target,
                "https://popup-fifo.test/second.html",
                "<!doctype html><p>second</p>",
            ),
        ));

        let first = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("first stale popup terminal should consume one turn");
        assert_eq!(
            first.action.owner,
            RendererPageResourceCompletionOwner::popup_document_load(root_document, first_target,)
        );

        assert!(queue.has_ready_completion());

        let second = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("second stale popup terminal should require its own turn");
        assert_eq!(
            second.action.owner,
            RendererPageResourceCompletionOwner::popup_document_load(root_document, second_target,)
        );

        assert!(!queue.has_ready_completion());
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("popup FIFO one-turn test should run");
}
