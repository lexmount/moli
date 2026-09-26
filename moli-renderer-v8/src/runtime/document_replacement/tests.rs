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
    reservation: crate::RendererPageReservationToken,
    replacement: Option<RendererDocumentReplacement>,
) -> anyhow::Result<crate::PreparedRendererDocument> {
    let loader = crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default())?;
    let url = url::Url::parse("https://replacement.test/").unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        runtime.prepare_streaming_raw_document_from_external_body(
            reservation,
            replacement,
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

#[tokio::test]
async fn renderer_prepare_keeps_scope_until_cancel_or_commit() {
    let runtime = crate::JsRuntime::initialize();
    let pause = RendererInspectorPauseBridge::default();
    for commit in [false, true] {
        let input = replacement(&pause);
        let prepared = prepare(
            &runtime,
            runtime.reserve_page_for_creation(),
            Some(input.clone()),
        )
        .await
        .unwrap();
        assert_replacement_exit(&pause);
        if commit {
            let permit = prepared.issue_commit_permit();
            let (mut page, _, _, _, _) = prepared.commit(permit).await.unwrap();
            page.close_async().await.unwrap();
        } else {
            prepared.cancel().await.unwrap();
        }
        assert_debugging_enabled(&pause);
        drop(input);
    }
}

#[tokio::test]
async fn canceled_replacement_is_rejected_before_renderer_admission() {
    let runtime = crate::JsRuntime::initialize();
    let (output_tx, mut output_rx) = crate::runtime::renderer_output_transport_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let reservation = runtime.reserve_page_for_creation();
    let pause = RendererInspectorPauseBridge::default();
    let input = replacement(&pause);
    input.cancellation.cancel();
    let result = prepare(&runtime, reservation, Some(input)).await;
    assert!(
        matches!(result, Err(error) if error.to_string() == moli_fetch::NET_ERR_ABORTED_ERROR_TEXT)
    );
    assert_debugging_enabled(&pause);
    assert!(matches!(
        output_rx.try_recv(),
        Ok(crate::runtime::RendererOutputTransportMessage::PageReservationReleased {
            owner_local_host_id, page_id,
        }) if owner_local_host_id == reservation.local_host_id() && page_id == reservation.page_id()
    ));
    assert!(matches!(
        output_rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
    assert_eq!(
        runtime
            .document_isolate_accounting_for_diagnostics()
            .created,
        0
    );
}

#[tokio::test]
async fn failed_renderer_prepare_releases_scope() {
    let runtime = crate::JsRuntime::initialize();
    let other_runtime = crate::JsRuntime::initialize();
    let pause = RendererInspectorPauseBridge::default();
    let result = prepare(
        &runtime,
        other_runtime.reserve_page_for_creation(),
        Some(replacement(&pause)),
    )
    .await;
    assert!(
        result.is_err(),
        "another owner's reservation must be rejected"
    );
    assert_debugging_enabled(&pause);
}

#[tokio::test]
async fn rejected_renderer_command_releases_scope() {
    let runtime = crate::JsRuntime::initialize();
    let pause = RendererInspectorPauseBridge::default();
    runtime.close_owner_command_admission_for_testing();
    assert!(
        prepare(
            &runtime,
            runtime.reserve_page_for_creation(),
            Some(replacement(&pause))
        )
        .await
        .is_err()
    );
    assert_debugging_enabled(&pause);
}

#[tokio::test]
async fn dropping_prepared_handle_retires_owner_scope() {
    let runtime = crate::JsRuntime::initialize();
    let pause = RendererInspectorPauseBridge::default();
    let prepared = prepare(
        &runtime,
        runtime.reserve_page_for_creation(),
        Some(replacement(&pause)),
    )
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
}

#[tokio::test]
async fn dropping_inflight_prepare_retires_queued_document_and_scope() {
    let runtime = crate::JsRuntime::initialize();
    let pause = RendererInspectorPauseBridge::default();
    let (entered, release) = runtime.install_owner_command_dispatch_gate_for_testing();
    let mut pending = Box::pin(prepare(
        &runtime,
        runtime.reserve_page_for_creation(),
        Some(replacement(&pause)),
    ));
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
}
