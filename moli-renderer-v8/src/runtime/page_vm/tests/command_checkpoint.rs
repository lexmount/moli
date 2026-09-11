use super::*;
use crate::{
    PendingSubresourceContinueOutcome, RendererPageCommand, RendererPageReply,
    RendererSetDocumentContentResult, types::PendingSubresourceContinueEvent,
};

#[test]
fn passive_console_snapshot_does_not_checkpoint_the_page_agent() {
    let mut page_vm = test_page_vm();
    page_vm
        .vm_mut()
        .eval_without_microtask_checkpoint_for_test(
            r#"
globalThis.__passiveSnapshotCheckpoint = 0;
Promise.resolve().then(() => __passiveSnapshotCheckpoint += 1);
"queued"
"#,
        )
        .expect("passive snapshot checkpoint witness should queue a reaction");

    let _ = page_vm
        .vm_mut()
        .snapshot_console_messages_with_context()
        .expect("console snapshot should read the current realm slots");

    assert_eq!(
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test("String(__passiveSnapshotCheckpoint)",)
            .expect("passive snapshot marker should remain readable"),
        "0",
        "a passive console snapshot must not become a Page-agent checkpoint authority",
    );
}

#[test]
fn passive_timezone_surface_sync_does_not_checkpoint_the_page_agent() {
    let mut page_vm = test_page_vm();
    page_vm
        .vm_mut()
        .eval_without_microtask_checkpoint_for_test(
            r#"
globalThis.__passiveTimezoneCheckpoint = 0;
Promise.resolve().then(() => __passiveTimezoneCheckpoint += 1);
"queued"
"#,
        )
        .expect("timezone checkpoint witness should queue a reaction");

    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
    environment
        .set_timezone(Some("UTC"))
        .expect("timezone surface should update without executing Page work");

    assert_eq!(
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test("String(__passiveTimezoneCheckpoint)",)
            .expect("timezone checkpoint marker should remain readable"),
        "0",
        "a passive runtime-surface update must not become a Page-agent checkpoint authority",
    );
}

#[test]
fn file_input_command_completes_listener_microtasks_before_returning() {
    let mut page_vm = test_page_vm();
    page_vm
        .vm_mut()
        .eval(
            r#"
const input = document.createElement("input");
input.id = "checkpoint-file-input";
input.type = "file";
document.body.appendChild(input);
globalThis.__fileInputCommandCheckpoint = [];
input.addEventListener("change", () => {
  __fileInputCommandCheckpoint.push("listener");
  Promise.resolve().then(() => __fileInputCommandCheckpoint.push("microtask"));
});
"ready"
"#,
        )
        .expect("file-input command checkpoint fixture should initialize");
    let input = page_vm
        .vm()
        .document_runtime
        .get_element_by_id("checkpoint-file-input")
        .expect("file input should exist");
    let backend_node_id = page_vm
        .renderer_backend_node_id_for_live_handle(input)
        .expect("file input should have a renderer backend node binding");

    assert_eq!(
        page_vm
            .set_file_input_files_for_backend_node_id(
                backend_node_id,
                vec![crate::dom::native::SelectedFile {
                    bytes: b"checkpoint".to_vec(),
                    mime_type: "text/plain".to_owned(),
                    name: "checkpoint.txt".to_owned(),
                    last_modified: 1.0,
                }],
                false,
            )
            .expect("file-input command should complete"),
        Some(true),
    );

    assert_eq!(
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test("__fileInputCommandCheckpoint.join('|')",)
            .expect("file-input command checkpoint log should remain readable"),
        "listener|microtask",
        "the JS-capable input command must own an explicit command-end checkpoint",
    );
}

#[test]
fn insert_text_command_completes_listener_microtasks_before_returning() {
    let mut page_vm = test_page_vm();
    page_vm
        .vm_mut()
        .eval(
            r#"
const input = document.createElement("input");
input.id = "checkpoint-text-input";
document.body.appendChild(input);
input.focus();
globalThis.__insertTextCommandCheckpoint = [];
input.addEventListener("input", () => {
  __insertTextCommandCheckpoint.push("listener");
  Promise.resolve().then(() => __insertTextCommandCheckpoint.push("microtask"));
});
"ready"
"#,
        )
        .expect("insert-text command checkpoint fixture should initialize");

    assert!(
        page_vm
            .insert_text_into_active_control("checkpoint")
            .expect("insert-text command should complete")
    );

    assert_eq!(
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test("__insertTextCommandCheckpoint.join('|')",)
            .expect("insert-text command checkpoint log should remain readable"),
        "listener|microtask",
        "the JS-capable insert-text command must own an explicit command-end checkpoint",
    );
}

#[tokio::test(flavor = "current_thread")]
async fn set_document_content_command_completes_mutation_observers_before_returning() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm_with_root_frame_id("checkpoint-root-frame");
        page_vm.vm_mut().eval(
            r#"
globalThis.__setContentCheckpoint = [];
new MutationObserver(records => {
  __setContentCheckpoint.push(records.length);
}).observe(document, { childList: true, subtree: true });
"ready"
"#,
        )?;
        let frame_id = page_vm
            .vm()
            .root_frame_id()
            .expect("test Page should expose its root frame")
            .to_owned();

        let reply = page_vm
            .dispatch_renderer_page_command_async(RendererPageCommand::SetDocumentContent {
                frame_id,
                html: "<main>after</main>".to_owned(),
            })
            .await?;
        assert!(matches!(
            reply,
            RendererPageReply::SetDocumentContentResult(RendererSetDocumentContentResult::Updated)
        ));
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(__setContentCheckpoint.length > 0)",
                )?,
            "true",
            "the SetContent owner turn must settle MutationObserver callbacks before replying",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("setDocumentContent command checkpoint should complete");
}

#[tokio::test(flavor = "current_thread")]
async fn fetch_interception_command_completes_window_reactions_without_runtime_drain() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm
            .vm_mut()
            .set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
        page_vm.vm_mut().eval(
            r#"
globalThis.__fetchCommandState = "pending";
fetch("https://command-checkpoint.test/resource").then(
  () => { globalThis.__fetchCommandState = "fulfilled"; },
  () => { globalThis.__fetchCommandState = "rejected"; }
);
"queued"
"#,
        )?;
        let pending = page_vm.vm_mut().take_pending_subresource_fetch_infos();
        assert_eq!(
            pending.len(),
            1,
            "the fixture should pause one Window Fetch"
        );

        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__fetchCommandCheckpoint = 0;
Promise.resolve().then(() => __fetchCommandCheckpoint += 1);
"reaction queued"
"#,
            )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        let reply = page_vm
            .dispatch_renderer_page_command_async(
                RendererPageCommand::FailPendingSubresourceFetch {
                    internal_id: pending[0].internal_id,
                    error_text: "command checkpoint witness".to_owned(),
                },
            )
            .await?;
        assert!(matches!(reply, RendererPageReply::Unit));
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__fetchCommandState + ':' + __fetchCommandCheckpoint",
                )?,
            "rejected:1",
            "a Window-entering Fetch command must settle its Promise reactions before replying",
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "command completion must not acquire generic runtime-drain authority",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Fetch-interception command checkpoint witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn offline_fetch_continue_publishes_completed_after_window_command_checkpoint() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm
            .vm_mut()
            .set_fetch_subresource_interception(true, Some(SubresourceResourceType::Fetch));
        page_vm.vm_mut().eval(
            r#"
globalThis.__offlineContinueState = "pending";
fetch("https://offline-command-checkpoint.test/resource").then(
  () => { globalThis.__offlineContinueState = "fulfilled"; },
  () => { globalThis.__offlineContinueState = "rejected"; }
);
"queued"
"#,
        )?;
        let pending = page_vm.vm_mut().take_pending_subresource_fetch_infos();
        assert_eq!(
            pending.len(),
            1,
            "the fixture should pause one Window Fetch"
        );

        let reply = page_vm
            .dispatch_renderer_page_command_async(RendererPageCommand::SetNetworkOffline(true))
            .await?;
        assert!(matches!(reply, RendererPageReply::Unit));
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__offlineContinueCheckpoint = 0;
Promise.resolve().then(() => __offlineContinueCheckpoint += 1);
"reaction queued"
"#,
            )?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        let reply = page_vm
            .dispatch_renderer_page_command_async(
                RendererPageCommand::ContinuePendingSubresourceFetch {
                    internal_id: pending[0].internal_id,
                    url: None,
                    method: None,
                    body: None,
                    headers: None,
                    intercept_response: false,
                    handle_auth_requests: false,
                },
            )
            .await?;
        assert!(matches!(
            reply,
            RendererPageReply::PendingSubresourceContinueOutcome(
                PendingSubresourceContinueOutcome::Started
            )
        ));
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__offlineContinueState + ':' + __offlineContinueCheckpoint",
                )?,
            "rejected:1",
            "offline continuation must complete its Window reactions before replying",
        );

        // This is a standalone PageVm fixture with no renderer output
        // transport. Inspect its local producer fallback directly; production
        // Page commands must not expose a destructive drain parallel to the
        // concrete output stream.
        let events = page_vm.vm_mut().take_pending_subresource_continue_events();
        assert_eq!(
            events,
            vec![PendingSubresourceContinueEvent::Completed {
                internal_id: pending[0].internal_id,
            }],
            "the command must publish its Completed output after consuming command completion",
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "command completion must not acquire generic runtime-drain authority",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("offline Fetch-continuation command witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn rejected_fetch_interception_command_without_window_body_does_not_checkpoint() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                r#"
globalThis.__rejectedFetchCommandCheckpoint = 0;
Promise.resolve().then(() => __rejectedFetchCommandCheckpoint += 1);
"reaction queued"
"#,
            )?;

        let error = match page_vm
            .dispatch_renderer_page_command_async(
                RendererPageCommand::FailPendingSubresourceFetch {
                    internal_id: u64::MAX,
                    error_text: "missing request".to_owned(),
                },
            )
            .await
        {
            Ok(_) => panic!("an unknown interception request should be rejected"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("unknown pending subresource fetch"),
            "the command should preserve its exact domain error: {error:#}",
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "String(__rejectedFetchCommandCheckpoint)",
                )?,
            "0",
            "a command rejected before entering a Window realm must not advance Page microtasks",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("rejected Fetch-interception command witness should run");
}
