use super::*;
use crate::page_task_queue::RendererPageDomManipulationTask;

#[tokio::test(flavor = "current_thread")]
async fn promise_rejection_tasks_retire_with_their_exact_document() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        for handled in [false, true] {
            let (mut page_vm, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader, Url::parse("https://example.com/stale-rejection")?,
            );
            page_vm.vm_mut().eval(r#"
              globalThis.events = [];
              onunhandledrejection = event => { events.push(event.type); event.preventDefault(); };
              onrejectionhandled = event => events.push(event.type);
              globalThis.rejected = Promise.reject('retired document');
            "#)?;
            if handled {
                assert!(page_vm.run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(PageDomManipulationTestFamily::PromiseRejection),
                    &loader,
                ).await?);
                page_vm.vm_mut().eval("rejected.catch(() => {});")?;
            }
            let task = page_vm.take_dom_manipulation_body_task_for_test(
                PageDomManipulationTestFamily::PromiseRejection,
            ).expect("queued rejection event");
            let RendererPageDomManipulationTask::PromiseRejection(rejection) = &task else {
                unreachable!("rejection task selector")
            };
            let task_id = rejection.task_id();
            let previous = page_vm.vm().current_main_document_task_owner().unwrap();
            page_vm.vm_mut().eval(r#"
              document.open(); document.write('<!doctype html><body>replacement</body>'); document.close();
            "#)?;
            let current = page_vm.vm().current_main_document_task_owner().unwrap();
            assert_ne!(previous, current);
            page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                "globalThis.checkpoints = 0; Promise.resolve().then(() => checkpoints++);",
            )?;
            page_vm.run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(task, &loader).await?;
            assert_eq!(page_vm.vm_mut().eval_without_microtask_checkpoint_for_test("String(checkpoints)")?, "0",
                "a retired notification must not checkpoint the replacement document");
            assert_eq!(page_vm.vm_mut().eval("String(events.length)")?, if handled { "1" } else { "0" });
            assert!(!page_vm.vm_mut().discard_stale_promise_rejection_task(task_id));
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("promise rejection exact-document retirement");
}
