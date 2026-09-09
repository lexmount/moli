use super::*;
use crate::page_task_queue::{
    PageScriptPreparationErrorTargetEffect, RendererOwnerWakeSource,
    RendererPageDomManipulationTask, RendererPageScriptPreparationErrorTask,
};

fn take_error_task(page_vm: &mut PageVm) -> RendererPageScriptPreparationErrorTask {
    let task = page_vm
        .take_dom_manipulation_body_task_for_test(
            PageDomManipulationTestFamily::ScriptPreparationError,
        )
        .expect("queued script error task");
    let RendererPageDomManipulationTask::ScriptPreparationError(task) = task else {
        unreachable!("exact script-error family selection")
    };
    task
}

fn queue_error(page_vm: &mut PageVm, element_id: &str) -> anyhow::Result<()> {
    let element = page_vm
        .vm()
        .element_handle_by_id_for_test(element_id)
        .expect("script element");
    page_vm.vm_mut().queue_script_preparation_error(element)
}

#[tokio::test(flavor = "current_thread")]
async fn script_preparation_errors_share_dom_fifo_and_complete_listener_reactions() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page_vm, _resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(
                &loader, Url::parse("https://example.com/script-error-fifo")?,
            );
        while owner_wake_rx.try_recv().is_ok() {}
        page_vm.vm_mut().eval(r#"
globalThis.events = [];
globalThis.receiver = new BroadcastChannel('script-error-fifo');
receiver.onmessage = () => {
  events.push('broadcast');
  Promise.resolve().then(() => events.push('microtask:broadcast'));
};
globalThis.sender = new BroadcastChannel('script-error-fifo');
sender.postMessage('first');
document.body.innerHTML = '<script id="first"></script><script id="second"></script>';
for (const element of document.scripts) {
  element.onerror = () => {
    events.push(element.id);
    Promise.resolve().then(() => events.push('microtask:' + element.id));
    if (element.id === 'first') throw new Error('listener sentinel');
  };
}
onerror = message => { events.push(message.includes('listener sentinel') ? 'exception' : 'wrong-error'); return true; };
'installed'
"#)?;
        queue_error(&mut page_vm, "first")?;
        queue_error(&mut page_vm, "second")?;
        assert_eq!(page_vm.vm_mut().eval("events.join('|')")?, "");
        assert!(!page_vm.vm().has_ready_timeout(), "no timer descriptor for element errors");
        let wakes = std::iter::from_fn(|| owner_wake_rx.try_recv().ok())
            .map(|wake| wake.source_for_test()).collect::<Vec<_>>();
        assert_eq!(wakes, vec![RendererOwnerWakeSource::DomManipulationTask]);
        page_vm.vm_mut().eval("document.getElementById('first').remove(); 'detached'")?;
        assert!(page_vm.run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::DomManipulation(PageDomManipulationTestFamily::BroadcastChannel),
            &loader,
        ).await?);
        assert_eq!(page_vm.vm_mut().eval("events.join('|')")?, "broadcast|microtask:broadcast");

        let first = take_error_task(&mut page_vm);
        let outcome = page_vm.apply_selected_page_script_preparation_error_turn(first)?;
        assert_eq!(outcome.action.target_effect, PageScriptPreparationErrorTargetEffect::DispatchedToCurrentOwner);
        assert_eq!(page_vm.vm_mut().eval("events.join('|')")?, "broadcast|microtask:broadcast|first|microtask:first|exception",
            "the body retains detached elements and cleans up the listener before reporting its exception");
        page_vm.finish_selected_page_callback_task(&loader).await?;
        assert!(page_vm.run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::DomManipulation(PageDomManipulationTestFamily::ScriptPreparationError),
            &loader,
        ).await?);
        assert_eq!(page_vm.vm_mut().eval("events.join('|')")?,
            "broadcast|microtask:broadcast|first|microtask:first|exception|second|microtask:second");

        let duplicate = page_vm.apply_selected_page_script_preparation_error_turn(first)?;
        assert!(matches!(duplicate.action.target_effect,
            PageScriptPreparationErrorTargetEffect::DiscardedStaleOwner { current_owner: None }));
        assert!(!page_vm.has_ready_dom_manipulation_task_for_test());
        Ok::<_, anyhow::Error>(())
    }).await.expect("script error FIFO and checkpoint test");
}

#[tokio::test(flavor = "current_thread")]
async fn script_preparation_errors_retire_with_the_exact_document_without_checkpointing() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(
                &loader, Url::parse("https://example.com/script-error-stale")?,
            );
        page_vm.vm_mut().eval(r#"
globalThis.events = [];
document.body.innerHTML = '<script id="retired"></script>';
document.getElementById('retired').onerror = () => events.push('retired');
'installed'
"#)?;
        queue_error(&mut page_vm, "retired")?;
        let task = take_error_task(&mut page_vm);
        let previous = page_vm.vm().current_main_document_task_owner().unwrap();
        page_vm.vm_mut().eval(r#"
document.open(); document.write('<!doctype html><body>replacement</body>'); document.close(); 'replaced'
"#)?;
        let current = page_vm.vm().current_main_document_task_owner().unwrap();
        assert_ne!(previous, current);
        assert_eq!(previous.local_window_id, current.local_window_id);
        page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
            "globalThis.checkpoints = 0; Promise.resolve().then(() => checkpoints++); 'queued'",
        )?;
        page_vm.run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
            RendererPageDomManipulationTask::ScriptPreparationError(task), &loader,
        ).await?;
        assert_eq!(page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
            "JSON.stringify([events, checkpoints])",
        )?, "[[],0]", "a stale event must not run or checkpoint the replacement document");
        assert!(!page_vm.vm_mut().discard_stale_script_preparation_error_task(task.task_id()),
            "stale payload was already retired");
        Ok::<_, anyhow::Error>(())
    }).await.expect("script error exact-document test");
}

#[test]
fn script_preparation_errors_preserve_reused_ids_across_page_vm_replacement() {
    run_page_vm_large_stack_async_test("script-error-page-vm-replacement", || async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/replacement.html",
            "HTTP/1.1 200 OK",
            "<!doctype html><body>replacement</body>".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse(&format!("{base_url}/initial.html")).unwrap(),
            );
        let local_executor = page_vm.local_executor.clone();
        local_executor.run(async move {
            let mut page_vm = page_vm;
            page_vm.vm_mut().eval(
                "document.body.innerHTML = '<script id=retired></script>'; 'installed'",
            )?;
            queue_error(&mut page_vm, "retired")?;
            let retired = take_error_task(&mut page_vm);
            let replacement_url = format!("{base_url}/replacement.html");
            page_vm.vm_mut().eval(&format!("location.href = {replacement_url:?}; 'navigating'"))?;
            let mut lifecycle = None;
            let navigation = page_vm.follow_pending_location_navigation_one_turn_async(
                &mut lifecycle, PageVmInitStage::Load,
            ).await?;
            assert!(matches!(navigation,
                crate::runtime::PageVmFollowNavigationTurnOutcome::Completed
                | crate::runtime::PageVmFollowNavigationTurnOutcome::PostParseLifecycle { .. }));
            page_vm.vm_mut().eval(r#"
globalThis.events = [];
document.body.innerHTML = '<script id=current></script>';
document.getElementById('current').onerror = () => events.push('current');
'installed'
"#)?;
            queue_error(&mut page_vm, "current")?;
            let current = take_error_task(&mut page_vm);
            assert_eq!(retired.task_id(), current.task_id(), "the two Hosts naturally reuse their first task id");
            assert_ne!(retired.owner().root_document(), current.owner().root_document());
            let stale = page_vm.apply_selected_page_script_preparation_error_turn(retired)?;
            assert!(matches!(stale.action.target_effect,
                PageScriptPreparationErrorTargetEffect::DiscardedStaleOwner { .. }));
            page_vm.run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                RendererPageDomManipulationTask::ScriptPreparationError(current), &loader,
            ).await?;
            assert_eq!(page_vm.vm_mut().eval("events.join('|')")?, "current",
                "a foreign root must not retire the replacement Host's reused id");
            Ok::<_, anyhow::Error>(())
        }).await.expect("script error PageVm namespace test");
        server.await.expect("replacement server");
    });
}
