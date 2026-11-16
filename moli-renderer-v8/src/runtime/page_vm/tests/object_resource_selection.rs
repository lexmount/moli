use super::*;
use crate::page_task_queue::RendererPageDomManipulationTask;

fn select_object_error(
    page_vm: &mut PageVm,
) -> crate::page_task_queue::RendererPageObjectResourceSelectionTask {
    let task = page_vm
        .take_dom_manipulation_body_task_for_test(
            PageDomManipulationTestFamily::ObjectResourceSelection,
        )
        .expect("an invalid object resource queues a DOM selection task");
    let RendererPageDomManipulationTask::ObjectResourceSelection(task) = task else {
        unreachable!("exact object resource-selection task")
    };
    task
}

#[tokio::test(flavor = "current_thread")]
async fn invalid_object_resource_selection_uses_dom_fifo_and_intrinsic_events() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page_vm, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://object-error.test/fifo")?,
        );
        page_vm.vm_mut().eval(
            r#"
          globalThis.events = [];
          globalThis.details = [];
          globalThis.intrinsicEvent = Event;
          globalThis.receiver = new BroadcastChannel('object-error-fifo');
          receiver.onmessage = () => {
            events.push('broadcast');
            Promise.resolve().then(() => events.push('microtask:broadcast'));
          };
          globalThis.sender = new BroadcastChannel('object-error-fifo');
          sender.postMessage('first');
        "#,
        )?;
        page_vm.vm_mut().exec_runtime_turn(
            r#"
          Event = function() {throw Error('author Event constructor');};
          globalThis.object = document.createElement('object');
          object.data = 'http://test:test';
          object.onerror = event => {
            events.push('error');
            details.push(event instanceof intrinsicEvent, event.isTrusted,
              !event.bubbles, !event.cancelable, event.target === object,
              object.contentWindow === null, object.contentDocument === null);
            Promise.resolve().then(() => events.push('microtask:error'));
          };
          object.onload = () => events.push('wrong-load');
          document.body.append(object);
        "#,
            None,
        )?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(frames.length) + ':' + events.join('|')")?,
            "0:"
        );
        assert!(
            !page_vm.vm().has_ready_timeout(),
            "selection errors do not use a timer"
        );
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::BroadcastChannel
                    ),
                    &loader,
                )
                .await?
        );
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::ObjectResourceSelection
                    ),
                    &loader,
                )
                .await?
        );
        assert_eq!(
            page_vm.vm_mut().eval("events.join('|')")?,
            "broadcast|microtask:broadcast|error|microtask:error"
        );
        assert_eq!(
            page_vm.vm_mut().eval("JSON.stringify(details)")?,
            "[true,true,true,true,true,true,true]"
        );
        page_vm.vm_mut().exec_runtime_turn(
            "void object.contentWindow; void object.contentDocument;",
            None,
        )?;
        assert!(
            !page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::ObjectResourceSelection
                    ),
                    &loader,
                )
                .await?,
            "wrapper resynchronization must not repeat a completed selection"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("object selection FIFO and trusted error event");
}

#[tokio::test(flavor = "current_thread")]
async fn invalid_object_resource_selection_cancels_obsolete_attributes_and_detachments() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page_vm, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://object-error.test/replacement")?,
        );
        page_vm.vm_mut().exec_runtime_turn(
            r#"
          globalThis.events = [];
          globalThis.object = document.createElement('object');
          object.onerror = () => events.push(object.getAttribute('data'));
          object.data = 'http://test:test';
          document.body.append(object);
          object.remove();
          object.data = 'http://[';
          document.body.append(object);
        "#,
            None,
        )?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::ObjectResourceSelection
                    ),
                    &loader,
                )
                .await?
        );
        assert_eq!(page_vm.vm_mut().eval("events.join('|')")?, "http://[");
        assert!(!page_vm.has_ready_dom_manipulation_task_for_test());
        page_vm
            .vm_mut()
            .exec_runtime_turn("object.data = 'http://test:test'; object.remove();", None)?;
        assert!(
            !page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::ObjectResourceSelection
                    ),
                    &loader,
                )
                .await?
        );
        assert_eq!(page_vm.vm_mut().eval("events.join('|')")?, "http://[");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("obsolete object selection retirement");
}

#[tokio::test(flavor = "current_thread")]
async fn invalid_object_resource_selection_does_not_enter_a_replacement_document() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page_vm, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader, Url::parse("https://object-error.test/document")?,
        );
        page_vm.vm_mut().exec_runtime_turn(r#"
          globalThis.events = [];
          const object = document.createElement('object');
          object.data = 'http://test:test';
          object.onerror = () => events.push('retired');
          document.body.append(object);
        "#, None)?;
        let task = select_object_error(&mut page_vm);
        let previous = page_vm.vm().current_main_document_task_owner().unwrap();
        page_vm.vm_mut().eval("document.open(); document.write('<!doctype html><body>replacement</body>'); document.close();")?;
        assert_ne!(previous, page_vm.vm().current_main_document_task_owner().unwrap());
        page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
            "globalThis.checkpoints = 0; Promise.resolve().then(() => checkpoints++); 'queued'",
        )?;
        page_vm.run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
            RendererPageDomManipulationTask::ObjectResourceSelection(task), &loader,
        ).await?;
        assert_eq!(page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
            "JSON.stringify([events, checkpoints])",
        )?, "[[],0]");
        Ok::<_, anyhow::Error>(())
    }).await.expect("object selection keeps its exact Document");
}
