use super::*;

fn enqueue_post_parse_work(page_vm: &mut PageVm, work: PostParseLifecycleWork) {
    page_vm
        .vm_mut()
        .document_runtime
        .enqueue_main_document_runtime_lifecycle_work_for_test(work);
}

async fn run_selected_post_parse_work(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
) -> anyhow::Result<()> {
    assert!(
        page_vm
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MainDocumentRuntime(
                    PageMainDocumentRuntimeActionKind::PostParseWork,
                ),
                loader,
            )
            .await?,
        "one exact main-Document post-parse task must be ready"
    );
    Ok(())
}

fn csp_violation_task(
    page_vm: &PageVm,
) -> crate::page_task_queue::ContentSecurityPolicyViolationEventTask {
    let document_url = page_vm
        .vm()
        .document_runtime
        .host_document()
        .url()
        .to_string();
    crate::page_task_queue::ContentSecurityPolicyViolationEventTask::new(
        page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main Document owner"),
        crate::content_security_policy::ContentSecurityPolicyUrlViolation {
            effective_directive: "script-src",
            blocked_uri: "https://blocked-post-parse.test/script.js".to_owned(),
            document_uri: document_url,
            original_policy: "script-src 'none'".to_owned(),
            disposition: crate::content_security_policy::ContentSecurityPolicyDisposition::Enforce,
            report_uri_endpoints: Vec::new(),
            report_to_endpoints: Vec::new(),
            sample: String::new(),
            source_file: String::new(),
            line_number: 0,
            column_number: 0,
        },
    )
}

#[tokio::test(flavor = "current_thread")]
async fn post_parse_callback_body_retains_reactions_for_selected_completion() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().eval(
            r#"
globalThis.__postParseBodyOrder = [];
document.addEventListener("securitypolicyviolation", () => {
  __postParseBodyOrder.push("callback");
  queueMicrotask(() => __postParseBodyOrder.push("callback:microtask"));
}, { once: true });
"installed"
"#,
        )?;
        page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
            r#"
queueMicrotask(() => __postParseBodyOrder.push("preexisting"));
"queued"
"#,
        )?;
        let task = csp_violation_task(&page_vm);
        enqueue_post_parse_work(
            &mut page_vm,
            PostParseLifecycleWork::DispatchContentSecurityPolicyViolation(task),
        );

        let outcome = page_vm
            .run_page_main_document_runtime_body_for_test(&loader)
            .await?
            .expect("CSP work must produce one exact post-parse body turn");
        assert_eq!(
            outcome.action.kind(),
            PageMainDocumentRuntimeActionKind::PostParseWork
        );
        assert_eq!(
            outcome.action.target_effect(),
            PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "__postParseBodyOrder.join('|')",
            )?,
            "callback",
            "body execution must neither run the old BeforeTask checkpoint nor end the selected task"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("post-parse body-only witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_post_parse_callbacks_share_one_post_body_completion_boundary() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().eval(
            r#"
globalThis.__postParseCallbackOrder = [];
document.addEventListener("securitypolicyviolation", () => {
  __postParseCallbackOrder.push("csp");
  queueMicrotask(() => __postParseCallbackOrder.push("csp:microtask"));
}, { once: true });
const scriptTarget = document.createElement("script");
scriptTarget.id = "post-parse-script-event-target";
scriptTarget.type = "application/json";
scriptTarget.addEventListener("load", () => {
  __postParseCallbackOrder.push("script");
  queueMicrotask(() => __postParseCallbackOrder.push("script:microtask"));
}, { once: true });
document.body.appendChild(scriptTarget);
window.addEventListener("error", () => {
  __postParseCallbackOrder.push("window-error");
  queueMicrotask(() => __postParseCallbackOrder.push("window-error:microtask"));
}, { once: true });
"installed"
"#,
        )?;
        let script_node = page_vm
            .vm()
            .document_runtime
            .dom_host()
            .element_handle_by_id("post-parse-script-event-target")
            .expect("script event target");
        let script_handle = page_vm
            .vm_mut()
            .document_runtime
            .bind_runtime_owned_script_handle_for_node(
                script_node,
                "post-parse-script-event-handle",
            );

        page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
            "queueMicrotask(() => __postParseCallbackOrder.push('pre-csp')); 'queued'",
        )?;
        let csp_task = csp_violation_task(&page_vm);
        enqueue_post_parse_work(
            &mut page_vm,
            PostParseLifecycleWork::DispatchContentSecurityPolicyViolation(csp_task),
        );
        run_selected_post_parse_work(&mut page_vm, &loader).await?;
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "__postParseCallbackOrder.join('|')",
            )?,
            "csp|pre-csp|csp:microtask",
            "CSP body must precede the selected task checkpoint"
        );

        page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
            "queueMicrotask(() => __postParseCallbackOrder.push('pre-script')); 'queued'",
        )?;
        enqueue_post_parse_work(
            &mut page_vm,
            PostParseLifecycleWork::DispatchScriptEvent(crate::host::ScriptEventTask::new(
                crate::host::ScriptEventKind::Load,
                script_handle,
            )),
        );
        run_selected_post_parse_work(&mut page_vm, &loader).await?;
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "__postParseCallbackOrder.join('|')",
            )?,
            "csp|pre-csp|csp:microtask|script|pre-script|script:microtask",
            "script terminal body must precede the selected task checkpoint"
        );

        page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
            "queueMicrotask(() => __postParseCallbackOrder.push('pre-window-error')); 'queued'",
        )?;
        enqueue_post_parse_work(
            &mut page_vm,
            PostParseLifecycleWork::ReportWindowScriptFailure(
                crate::page_task_queue::WindowScriptFailureReportTask::new(
                    "post-parse failure",
                    Some("https://post-parse.test/failure.js".to_owned()),
                ),
            ),
        );
        run_selected_post_parse_work(&mut page_vm, &loader).await?;
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "__postParseCallbackOrder.join('|')",
            )?,
            "csp|pre-csp|csp:microtask|script|pre-script|script:microtask|window-error|pre-window-error|window-error:microtask",
            "window error body must precede the selected task checkpoint"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("post-parse callback completion witnesses should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_post_parse_state_tasks_checkpoint_then_commit_exact_effects() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm();
        page_vm
            .vm_mut()
            .eval("globalThis.__postParseStateCheckpoints = []; 'installed'")?;

        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                "queueMicrotask(() => __postParseStateCheckpoints.push('stylesheet')); 'queued'",
            )?;
        enqueue_post_parse_work(
            &mut page_vm,
            PostParseLifecycleWork::SeedDocumentOwnedBlockingStylesheets(Vec::new()),
        );
        run_selected_post_parse_work(&mut page_vm, &loader).await?;

        page_vm
            .vm_mut()
            .document_runtime
            .set_document_ready_state(crate::dom::native::DocumentReadyState::Complete);
        let run_count = page_vm.report.runs.len();
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                "queueMicrotask(() => __postParseStateCheckpoints.push('run')); 'queued'",
            )?;
        enqueue_post_parse_work(
            &mut page_vm,
            PostParseLifecycleWork::RecordDocumentScriptRun {
                position: 7,
                run: detached_test_run(),
            },
        );
        run_selected_post_parse_work(&mut page_vm, &loader).await?;
        assert_eq!(page_vm.report.runs.len(), run_count + 1);
        assert_eq!(
            page_vm.vm().document_runtime.host_document().ready_state(),
            crate::dom::native::DocumentReadyState::Loading,
            "run-record body must retain its ready-state transition"
        );

        let owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("state task owner");
        let lease = page_vm
            .vm_mut()
            .accept_main_document_script_load_delay_binding(
                owner,
                crate::frame_owner_model::MainDocumentScriptLoadDelayKind::Classic,
            )
            .expect("state task must acquire an exact load-delay lease");
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                "queueMicrotask(() => __postParseStateCheckpoints.push('lease')); 'queued'",
            )?;
        enqueue_post_parse_work(
            &mut page_vm,
            PostParseLifecycleWork::SettleMainDocumentScriptLoadDelay(lease),
        );
        run_selected_post_parse_work(&mut page_vm, &loader).await?;
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(false),
            "the exact lease must settle once in the selected body"
        );

        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                "queueMicrotask(() => __postParseStateCheckpoints.push('recheck')); 'queued'",
            )?;
        assert!(
            page_vm
                .vm_mut()
                .document_runtime
                .enqueue_main_document_completion_recheck(owner),
            "completion recheck must publish through its deduplicated production route"
        );
        run_selected_post_parse_work(&mut page_vm, &loader).await?;

        let detached_count = page_vm.report.runs.len();
        page_vm
            .vm_mut()
            .eval_without_microtask_checkpoint_for_test(
                "queueMicrotask(() => __postParseStateCheckpoints.push('detached')); 'queued'",
            )?;
        enqueue_post_parse_work(
            &mut page_vm,
            PostParseLifecycleWork::RecordDetachedPostParseRuns(vec![detached_test_run()]),
        );
        run_selected_post_parse_work(&mut page_vm, &loader).await?;
        assert_eq!(page_vm.report.runs.len(), detached_count + 1);

        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__postParseStateCheckpoints.join('|')",
                )?,
            "stylesheet|run|lease|recheck|detached",
            "every current state task must own its selected task checkpoint"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("post-parse state completion witnesses should run");
}

#[tokio::test(flavor = "current_thread")]
async fn stale_claimed_post_parse_task_does_not_checkpoint_or_report_in_replacement_document() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm();
        let original_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("original owner");
        let report_count = page_vm.report.runs.len();
        enqueue_post_parse_work(
            &mut page_vm,
            PostParseLifecycleWork::RecordDetachedPostParseRuns(vec![detached_test_run()]),
        );
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MainDocumentRuntime(
                    PageMainDocumentRuntimeActionKind::PostParseWork,
                ),
            )
            .expect("old Document post-parse task must be claimable");

        page_vm.vm_mut().eval(
            "document.open(); document.write('<!doctype html><body>replacement</body>'); document.close(); 'replaced'",
        )?;
        let replacement_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement owner");
        assert_ne!(replacement_owner, original_owner);
        page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
            r#"
globalThis.__stalePostParseCheckpoint = "pending";
queueMicrotask(() => { __stalePostParseCheckpoint = "wrong"; });
"queued"
"#,
        )?;

        page_vm
            .run_claimed_selected_page_task_for_test(claimed, &loader)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "globalThis.__stalePostParseCheckpoint",
            )?,
            "pending",
            "stale work must not manufacture a checkpoint in the replacement realm"
        );
        assert_eq!(
            page_vm.report.runs.len(),
            report_count,
            "stale detached-run work must not publish its old-Document report"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale post-parse claim witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn applied_post_parse_callback_finishes_before_returning_its_document_open_replacement() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm();
        let original_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("original callback owner");
        page_vm.vm_mut().eval(
            r#"
globalThis.__postParseReplacementOrder = [];
window.addEventListener("error", () => {
  __postParseReplacementOrder.push("callback");
  document.open();
  document.write("<!doctype html><body>replacement</body>");
  document.close();
  queueMicrotask(() => __postParseReplacementOrder.push("microtask"));
}, { once: true });
"installed"
"#,
        )?;
        enqueue_post_parse_work(
            &mut page_vm,
            PostParseLifecycleWork::ReportWindowScriptFailure(
                crate::page_task_queue::WindowScriptFailureReportTask::new(
                    "replace during callback",
                    None,
                ),
            ),
        );

        run_selected_post_parse_work(&mut page_vm, &loader).await?;
        let replacement_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement callback owner");
        assert_ne!(replacement_owner, original_owner);
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__postParseReplacementOrder.join('|')",
                )?,
            "callback|microtask",
            "an applied callback remains responsible for its task-end checkpoint after document.open"
        );
        assert!(
            page_vm
                .vm()
                .snapshot_live_document()
                .serialize_document()
                .contains("replacement"),
            "callback completion must reconcile the replacement Document before returning"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("post-parse callback document.open witness should run");
}

#[tokio::test(flavor = "current_thread")]
async fn post_parse_callback_completion_publishes_but_does_not_execute_runtime_successor() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm();
        page_vm
            .vm_mut()
            .document_runtime
            .note_dom_content_loaded_dispatched();
        page_vm.vm_mut().eval(
            r#"
globalThis.__postParseRuntimeBoundary = [];
window.addEventListener("error", () => {
  __postParseRuntimeBoundary.push("callback");
}, { once: true });
"installed"
"#,
        )?;
        enqueue_post_parse_work(
            &mut page_vm,
            PostParseLifecycleWork::ReportWindowScriptFailure(
                crate::page_task_queue::WindowScriptFailureReportTask::new(
                    "runtime boundary",
                    None,
                ),
            ),
        );
        page_vm
            .vm_mut()
            .enqueue_test_ready_runtime_script_followup();

        run_selected_post_parse_work(&mut page_vm, &loader).await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__postParseRuntimeBoundary.join('|')",
                )?,
            "callback"
        );
        assert!(
            page_vm.vm_mut().has_runnable_runtime_script_work_now(),
            "callback completion must not synchronously execute generic runtime work"
        );
        assert!(
            has_ready_runtime_script_continuation_for_test(&page_vm),
            "task completion may publish one typed runtime successor for later arbitration"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("post-parse runtime successor boundary witness should run");
}
