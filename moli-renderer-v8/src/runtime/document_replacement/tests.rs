use super::*;
use crate::devtools::pause::RendererInspectorPauseExitReason;

fn replacement(pause: &RendererInspectorPauseBridge) -> RendererDocumentReplacement {
    RendererDocumentReplacement::new(pause.clone(), moli_fetch::FetchCancelHandle::new())
}

fn assert_debugging_enabled(pause: &RendererInspectorPauseBridge) {
    assert_eq!(pause.wait_for_pause_work(|| Some(42)), Ok(42));
}

fn assert_replacement_exit(pause: &RendererInspectorPauseBridge) {
    assert_eq!(
        pause.wait_for_pause_work(|| Some(42)),
        Err(RendererInspectorPauseExitReason::DocumentReplacement)
    );
}

#[test]
fn capturing_and_dropping_replacement_inputs_never_disturbs_the_old_document() {
    let pause = RendererInspectorPauseBridge::default();
    let input = replacement(&pause);
    assert_debugging_enabled(&pause);
    drop(input.clone());
    assert_debugging_enabled(&pause);
    drop(input);
    assert_debugging_enabled(&pause);
}

#[test]
fn canceled_input_cannot_start_replacement_even_if_captured_before_cancellation() {
    let pause = RendererInspectorPauseBridge::default();
    let input = replacement(&pause);
    let retained = input.clone();
    input.cancellation.cancel();
    assert!(input.begin().is_err());
    assert!(retained.begin().is_err());
    assert_debugging_enabled(&pause);
}

#[test]
fn scope_drop_restores_debugging_without_waiting_for_retained_inputs() {
    let pause = RendererInspectorPauseBridge::default();
    let input = replacement(&pause);
    let scope = input.clone().begin().unwrap();
    assert_replacement_exit(&pause);
    drop(scope);
    assert_debugging_enabled(&pause);
    drop(input);
    assert_debugging_enabled(&pause);
}

#[test]
fn command_and_prepared_handle_share_one_scope_until_both_release_it() {
    let pause = RendererInspectorPauseBridge::default();
    let command = replacement(&pause).begin().unwrap();
    let handle = Arc::clone(&command);
    drop(command);
    assert_replacement_exit(&pause);
    drop(handle);
    assert_debugging_enabled(&pause);
}

#[test]
fn overlapping_preparations_cannot_finish_each_others_scope() {
    for finish_first in [true, false] {
        let pause = RendererInspectorPauseBridge::default();
        let first = replacement(&pause).begin().unwrap();
        let second = replacement(&pause).begin().unwrap();
        let (finished, pending) = if finish_first {
            (first, second)
        } else {
            (second, first)
        };
        drop(finished);
        assert_replacement_exit(&pause);
        drop(pending);
        assert_debugging_enabled(&pause);
    }
}

#[test]
fn replacement_is_document_local() {
    let first = RendererInspectorPauseBridge::default();
    let second = RendererInspectorPauseBridge::default();
    let scope = replacement(&first).begin().unwrap();
    assert_replacement_exit(&first);
    assert_debugging_enabled(&second);
    drop(scope);
    assert_debugging_enabled(&first);
}

#[test]
fn observed_exit_reason_survives_the_end_of_replacement() {
    let pause = RendererInspectorPauseBridge::default();
    let scope = replacement(&pause).begin().unwrap();
    let reason = pause.wait_for_pause_work(|| Some(42));
    drop(scope);
    assert_eq!(
        reason,
        Err(RendererInspectorPauseExitReason::DocumentReplacement)
    );
    assert_debugging_enabled(&pause);
}

#[test]
fn target_closure_takes_precedence_and_prevents_new_replacement() {
    let pause = RendererInspectorPauseBridge::default();
    let scope = replacement(&pause).begin().unwrap();
    pause.close_target();
    assert!(replacement(&pause).begin().is_err());
    drop(scope);
    assert_eq!(
        pause.wait_for_pause_work(|| Some(42)),
        Err(RendererInspectorPauseExitReason::TargetClosed)
    );
}

#[test]
fn preparation_wakes_the_pause_loop_and_covers_repeated_pauses() {
    let pause = RendererInspectorPauseBridge::default();
    let waiter_pause = pause.clone();
    let (waiting_tx, waiting_rx) = std::sync::mpsc::channel();
    let (exited_tx, exited_rx) = std::sync::mpsc::channel();
    let waiter = std::thread::spawn(move || {
        let mut waiting_tx = Some(waiting_tx);
        exited_tx
            .send(waiter_pause.wait_for_pause_work(|| {
                if let Some(tx) = waiting_tx.take() {
                    tx.send(()).unwrap();
                }
                None::<()>
            }))
            .unwrap();
    });
    let waiting = waiting_rx.recv_timeout(std::time::Duration::from_secs(30));
    let scope = replacement(&pause).begin().unwrap();
    let exited = exited_rx.recv_timeout(std::time::Duration::from_secs(30));
    if exited.is_err() {
        pause.close_target();
    }
    waiter.join().unwrap();
    waiting.expect("waiter must reach the pause loop before document preparation");
    assert_eq!(
        exited.expect("document preparation must wake the pause loop"),
        Err(RendererInspectorPauseExitReason::DocumentReplacement)
    );
    assert_replacement_exit(&pause);
    assert_replacement_exit(&pause);
    drop(scope);
    assert_debugging_enabled(&pause);
}

async fn prepare(
    runtime: &crate::JsRuntime,
    reservation: impl Into<crate::RendererDocumentPreparationTarget>,
) -> anyhow::Result<crate::PreparedRendererDocument> {
    let loader = crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default())?;
    let url = url::Url::parse("https://replacement.test/").unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        runtime.prepare_streaming_raw_document_from_external_body(
            reservation,
            url.clone(),
            url,
            None,
            false,
            0,
            Vec::new(),
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            crate::ExternalRawDocumentBodyStream::from_bytes(
                b"<!doctype html><title>new</title>".to_vec(),
            ),
            false,
            crate::PageVmInitStage::Load,
            crate::RendererReplyBoundary::Stage,
            crate::RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            crate::RendererNavigationReplyPolicy::FollowBeforeReply,
            None,
            None,
            crate::RendererDocumentOptions::default(),
        ),
    )
    .await?
}

async fn owner_barrier(runtime: &crate::JsRuntime) {
    let pending = runtime.enqueue_owner_command_probe_for_testing().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(30), pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

fn replacement_target(
    page: &crate::RendererPageHandle,
    pause: &RendererInspectorPauseBridge,
) -> RendererPageReplacementTarget {
    let mut target = page.document_replacement_target(moli_fetch::FetchCancelHandle::new());
    target.replacement = replacement(pause);
    target
}

#[tokio::test]
async fn renderer_prepare_keeps_scope_until_cancel_or_commit() {
    let runtime = crate::JsRuntime::initialize();
    let mut page = live_page_for_replacement_target_test(&runtime).await;
    let pause = RendererInspectorPauseBridge::default();
    for commit in [false, true] {
        let target = replacement_target(&page, &pause);
        let prepared = prepare(&runtime, target.clone()).await.unwrap();
        assert_replacement_exit(&pause);
        if commit {
            let permit = prepared.issue_commit_permit();
            let replacement = prepared.commit_page_replacement(permit).await.unwrap();
            page.adopt_document_replacement(replacement).unwrap();
        } else {
            prepared.cancel().await.unwrap();
        }
        assert_debugging_enabled(&pause);
        drop(target);
    }
    page.close_async().await.unwrap();
}

#[tokio::test]
async fn failed_renderer_prepare_releases_scope() {
    let runtime = crate::JsRuntime::initialize();
    let other_runtime = crate::JsRuntime::initialize();
    let mut page = live_page_for_replacement_target_test(&other_runtime).await;
    let pause = RendererInspectorPauseBridge::default();
    let result = prepare(&runtime, replacement_target(&page, &pause)).await;
    assert!(result.is_err(), "another owner's Page must be rejected");
    assert_debugging_enabled(&pause);
    page.close_async().await.unwrap();
}

#[tokio::test]
async fn rejected_renderer_command_releases_scope() {
    let runtime = crate::JsRuntime::initialize();
    let page = live_page_for_replacement_target_test(&runtime).await;
    let pause = RendererInspectorPauseBridge::default();
    runtime.close_owner_command_admission_for_testing();
    assert!(
        prepare(&runtime, replacement_target(&page, &pause))
            .await
            .is_err()
    );
    assert_debugging_enabled(&pause);
}

#[tokio::test]
async fn dropping_prepared_handle_retires_owner_scope() {
    let runtime = crate::JsRuntime::initialize();
    let mut page = live_page_for_replacement_target_test(&runtime).await;
    let pause = RendererInspectorPauseBridge::default();
    let prepared = prepare(&runtime, replacement_target(&page, &pause))
        .await
        .unwrap();
    assert_replacement_exit(&pause);
    drop(prepared);
    owner_barrier(&runtime).await;
    assert_debugging_enabled(&pause);
    assert_eq!(
        runtime
            .document_isolate_accounting_for_diagnostics()
            .reserved,
        0
    );
    page.close_async().await.unwrap();
}

#[tokio::test]
async fn dropping_inflight_prepare_retires_queued_document_and_scope() {
    let runtime = crate::JsRuntime::initialize();
    let mut page = live_page_for_replacement_target_test(&runtime).await;
    let pause = RendererInspectorPauseBridge::default();
    // The owner can finish both commands during the first poll. Hold reservation
    // dispatch so this test reaches the in-flight state before allowing progress.
    let (reservation_entered, reservation_release) =
        runtime.install_owner_command_dispatch_gate_for_testing();
    let mut pending = Box::pin(prepare(&runtime, replacement_target(&page, &pause)));
    assert!(futures_util::poll!(&mut pending).is_pending());
    reservation_entered
        .recv_timeout(std::time::Duration::from_secs(30))
        .unwrap();
    reservation_release.send(()).unwrap();
    // Finish reservation first, then hold the actual prepare command. The
    // reservation-cancellation test below covers cancellation before this point.
    owner_barrier(&runtime).await;
    let (entered, release) = runtime.install_owner_command_dispatch_gate_for_testing();
    assert!(futures_util::poll!(&mut pending).is_pending());
    let reached_gate = entered.recv_timeout(std::time::Duration::from_secs(30));
    assert_replacement_exit(&pause);
    drop(pending);
    // The queued command still owns the scope until owner-side cancellation.
    assert_replacement_exit(&pause);
    let released = release.send(());
    reached_gate.unwrap();
    released.unwrap();
    owner_barrier(&runtime).await;
    assert_debugging_enabled(&pause);
    assert_eq!(
        runtime
            .document_isolate_accounting_for_diagnostics()
            .reserved,
        0
    );
    page.close_async().await.unwrap();
}

async fn live_page_for_replacement_target_test(
    runtime: &crate::JsRuntime,
) -> crate::RendererPageHandle {
    let loader =
        crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let url = url::Url::parse("https://replacement.test/initial").unwrap();
    let (page, _, _, _, _) = runtime
        .create_streaming_raw_page_from_external_body(
            url.clone(),
            url,
            None,
            false,
            0,
            Vec::new(),
            200,
            vec![("content-type".into(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::ephemeral(),
            crate::ExternalRawDocumentBodyStream::from_bytes(
                b"<!doctype html><p>original</p>".to_vec(),
            ),
            false,
            crate::PageVmInitStage::Load,
            crate::RendererReplyBoundary::Stage,
            crate::RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
            crate::RendererNavigationReplyPolicy::FollowBeforeReply,
            None,
            None,
            crate::RendererDocumentOptions::default(),
        )
        .await
        .unwrap();
    page
}

#[tokio::test]
async fn existing_page_target_is_inert_until_response_preparation() {
    let runtime = crate::JsRuntime::initialize();
    let mut page = live_page_for_replacement_target_test(&runtime).await;
    let pause = RendererInspectorPauseBridge::default();
    let mut target = page.document_replacement_target(moli_fetch::FetchCancelHandle::new());
    target.replacement = replacement(&pause);
    assert_debugging_enabled(&pause);
    drop(target.clone());
    assert_debugging_enabled(&pause);
    let prepared = prepare(&runtime, target).await.unwrap();
    assert_replacement_exit(&pause);
    assert_eq!(prepared.token().page_id(), page.renderer_page_id());
    prepared.cancel().await.unwrap();
    assert_debugging_enabled(&pause);
    page.close_async().await.unwrap();
}

#[tokio::test]
async fn canceled_existing_page_target_never_reserves_or_wakes_the_document() {
    let runtime = crate::JsRuntime::initialize();
    let mut page = live_page_for_replacement_target_test(&runtime).await;
    let pause = RendererInspectorPauseBridge::default();
    let cancellation = moli_fetch::FetchCancelHandle::new();
    let mut target = page.document_replacement_target(cancellation.clone());
    target.replacement = RendererDocumentReplacement::new(pause.clone(), cancellation.clone());
    cancellation.cancel();
    let result = prepare(&runtime, target).await;
    assert!(matches!(result, Err(error) if error.is::<moli_fetch::FetchCancelled>()));
    assert_debugging_enabled(&pause);
    assert_eq!(
        runtime
            .document_isolate_accounting_for_diagnostics()
            .reserved,
        0
    );
    assert_eq!(runtime.renderer_owner_handle().len(), 1);
    page.close_async().await.unwrap();
}

#[tokio::test]
async fn abandoned_existing_page_reservation_releases_its_exact_pause_scope() {
    let runtime = crate::JsRuntime::initialize();
    let mut page = live_page_for_replacement_target_test(&runtime).await;
    let pause = RendererInspectorPauseBridge::default();
    let mut target = page.document_replacement_target(moli_fetch::FetchCancelHandle::new());
    target.replacement = replacement(&pause);
    let (entered, release) = runtime.install_owner_command_dispatch_gate_for_testing();
    let mut pending = Box::pin(prepare(&runtime, target));
    assert!(futures_util::poll!(&mut pending).is_pending());
    let reached = entered.recv_timeout(std::time::Duration::from_secs(30));
    assert_replacement_exit(&pause);
    drop(pending);
    assert_replacement_exit(&pause);
    release.send(()).unwrap();
    reached.unwrap();
    owner_barrier(&runtime).await;
    assert_debugging_enabled(&pause);
    assert_eq!(
        runtime
            .document_isolate_accounting_for_diagnostics()
            .reserved,
        0
    );
    let replacement = prepare(
        &runtime,
        page.document_replacement_target(moli_fetch::FetchCancelHandle::new()),
    )
    .await
    .unwrap();
    replacement.cancel().await.unwrap();
    page.close_async().await.unwrap();
}
