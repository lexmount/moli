use super::*;

#[tokio::test(flavor = "current_thread")]
async fn service_worker_queries_share_dom_fifo_and_discard_retired_documents() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://example.com/registration-query")?,
            );
        page_vm.vm_mut().eval(r#"
            globalThis.__queryEvents = [];
            globalThis.__queryReceiver = new BroadcastChannel('registration-query-fifo');
            __queryReceiver.onmessage = () => __queryEvents.push('broadcast');
            globalThis.__querySender = new BroadcastChannel('registration-query-fifo');
            __querySender.postMessage('first');
            navigator.serviceWorker.getRegistration().then(value => {
                __queryEvents.push('one:' + (value === undefined));
                queueMicrotask(() => __queryEvents.push('microtask:one'));
            });
            navigator.serviceWorker.getRegistrations().then(value => {
                __queryEvents.push('all:' + (value.length === 0 && Object.isFrozen(value)));
            });
            __queryEvents.push('sync');
        "#)?;
        assert_eq!(page_vm.vm_mut().eval("__queryEvents.join('|')")?, "sync");
        for (family, expected) in [
            (PageDomManipulationTestFamily::BroadcastChannel, "sync|broadcast"),
            (PageDomManipulationTestFamily::ServiceWorkerQuery, "sync|broadcast|one:true|microtask:one"),
            (PageDomManipulationTestFamily::ServiceWorkerQuery, "sync|broadcast|one:true|microtask:one|all:true"),
        ] {
            assert!(page_vm.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::DomManipulation(family), &loader,
            ).await?);
            assert_eq!(page_vm.vm_mut().eval("__queryEvents.join('|')")?, expected);
        }
        page_vm.vm_mut().eval(r#"
            globalThis.__staleQueryObserved = false;
            const frame = document.createElement('iframe');
            frame.id = 'retired-query-owner';
            document.body.appendChild(frame);
        "#)?;
        let child = materialize_only_child_realm_execution_context_through_page_turn_for_test(
            &mut page_vm, "retired-query-owner",
        )?;
        page_vm.vm_mut().eval_in_child_default_context(child, r#"
            navigator.serviceWorker.getRegistration().then(() => { parent.__staleQueryObserved = true; });
        "#)?;
        let claimed = page_vm.claim_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::DomManipulation(PageDomManipulationTestFamily::ServiceWorkerQuery),
        ).expect("query must reside in the DOM-manipulation source");
        page_vm.vm_mut().eval("document.getElementById('retired-query-owner').remove();")?;
        page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(r#"
            globalThis.__unrelatedQueryCheckpoint = 0;
            Promise.resolve().then(() => { __unrelatedQueryCheckpoint += 1; });
        "#)?;
        page_vm.run_claimed_selected_page_task_for_test(claimed, &loader).await?;
        assert_eq!(page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
            "String(__staleQueryObserved) + ':' + __unrelatedQueryCheckpoint",
        )?, "false:0");
        Ok::<_, anyhow::Error>(())
    }).await.expect("registration queries should use exact Document task ownership");
}
