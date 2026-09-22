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
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/retired-child.html",
            "HTTP/1.1 200 OK",
            "<!doctype html><p id='must-not-commit'>retired response</p>".to_owned(),
            Duration::from_millis(100),
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
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
        wait_for_page_resource_completion(&mut queue, &mut wake_rx, "retired child fetch").await;
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
        assert!(
            page_vm
                .take_completed_child_frame_navigation_loads()
                .is_empty(),
            "stale response must not synthesize a child navigation commit/lifecycle snapshot"
        );

        server.await.expect("retired child server should finish");
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
                final_url: Url::parse(request_url).expect("final URL"),
                policy_container: crate::document_runtime::DocumentPolicyContainer::default(),
                content_type: Some("text/html".to_owned()),
                character_set: "UTF-8".to_owned(),
                markup: "<!doctype html>".to_owned(),
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
