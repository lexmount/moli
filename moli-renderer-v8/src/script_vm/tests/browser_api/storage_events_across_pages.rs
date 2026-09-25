use super::*;

const OBSERVE: &str = r#"
globalThis.storageEvents = [];
function recordStorage(event) {
  this.storageEvents.push({key:event.key, old:event.oldValue, value:event.newValue,
    url:event.url, local:event.storageArea === this.localStorage,
    trusted:event.isTrusted, instance:event instanceof this.StorageEvent});
}
addEventListener('storage', recordStorage);
globalThis.storageFrame = document.createElement('iframe');
document.body.append(storageFrame);
storageFrame.contentWindow.storageEvents = [];
storageFrame.contentWindow.addEventListener('storage', recordStorage);
"#;

async fn drain_storage(
    vm: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
) -> usize {
    let mut count = 0;
    while vm
        .run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::StorageEvent,
            loader,
        )
        .await
        .unwrap()
    {
        count += 1;
        assert!(
            count < 64,
            "storage events must have a bounded delivery count"
        );
    }
    count
}

#[tokio::test]
async fn local_storage_events_cross_pages_with_matching_origin_and_partition() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let storage = crate::RendererWebStorageHandles::ephemeral();
    let other_storage = crate::RendererWebStorageHandles::new(
        storage.local_storage(),
        crate::new_shared_web_storage_store(),
    );
    let mut source = new_storage_page_task_executor_test_vm_with_loader(
        "https://storage-events.test/source",
        &loader,
    );
    let mut target = new_storage_page_task_executor_test_vm_with_loader(
        "https://storage-events.test/target",
        &loader,
    );
    let mut cross_origin =
        new_storage_page_task_executor_test_vm_with_loader("https://other-storage.test/", &loader);
    let mut isolated = new_storage_page_task_executor_test_vm_with_loader(
        "https://storage-events.test/isolated",
        &loader,
    );
    source.set_web_storage_handles(&storage);
    target.set_web_storage_handles(&other_storage);
    cross_origin.set_web_storage_handles(&other_storage);
    for vm in [&mut source, &mut target, &mut cross_origin, &mut isolated] {
        vm.eval(OBSERVE).unwrap();
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 100)
            .await
            .unwrap();
    }
    source
        .eval(
            r#"
localStorage.setItem('k', 'one');
localStorage.setItem('k', 'one');
localStorage.setItem('k', 'two');
localStorage.removeItem('k');
localStorage.removeItem('k');
localStorage.setItem('x', 'value');
localStorage.clear(); localStorage.clear();
localStorage.setItem('\ud800', '\udc00');
"#,
        )
        .unwrap();
    assert_eq!(target.eval("storageEvents.length").unwrap(), "0");
    assert_eq!(drain_storage(&mut target, &loader).await, 12);
    assert_eq!(drain_storage(&mut source, &loader).await, 6);
    assert_eq!(drain_storage(&mut cross_origin, &loader).await, 0);
    assert_eq!(drain_storage(&mut isolated, &loader).await, 0);
    assert_eq!(source.eval("storageEvents.length").unwrap(), "0");
    assert_eq!(
        target
            .eval(
                r#"JSON.stringify([window, storageFrame.contentWindow].map(w => {
  const expected = [['k',null,'one'],['k','one','two'],['k','two',null],
                    ['x',null,'value'],[null,null,null],['\ud800',null,'\udc00']];
  return JSON.stringify(w.storageEvents.map(e=>[e.key,e.old,e.value])) === JSON.stringify(expected)
    && w.storageEvents.every(e=>e.local && e.trusted && e.instance
      && e.url === 'https://storage-events.test/source');
}))"#
            )
            .unwrap(),
        "[true,true]"
    );

    source
        .eval("sessionStorage.setItem('session-key', 'private')")
        .unwrap();
    assert_eq!(drain_storage(&mut target, &loader).await, 0);
    assert_eq!(drain_storage(&mut source, &loader).await, 1);
    assert_eq!(
        source
            .eval("storageFrame.contentWindow.storageEvents.at(-1).local")
            .unwrap(),
        "false"
    );
}

#[tokio::test]
async fn remote_storage_events_keep_exact_recipients_through_frame_replacement() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let storage = crate::RendererWebStorageHandles::ephemeral();
    let mut source = new_storage_page_task_executor_test_vm_with_loader(
        "https://storage-events.test/source",
        &loader,
    );
    let mut target = new_storage_page_task_executor_test_vm_with_loader(
        "https://storage-events.test/target",
        &loader,
    );
    source.set_web_storage_handles(&storage);
    target.set_web_storage_handles(&storage);
    target.eval(OBSERVE).unwrap();
    target
        .drain_ready_page_task_executor_turns_for_setup(&loader, 100)
        .await
        .unwrap();
    source
        .eval("localStorage.setItem('before', 'replacement')")
        .unwrap();
    target
        .eval(
            r#"
const retired = storageFrame.contentWindow;
storageFrame.remove();
storageFrame = document.createElement('iframe'); document.body.append(storageFrame);
storageFrame.contentWindow.storageEvents = [];
storageFrame.contentWindow.addEventListener('storage', recordStorage);
"#,
        )
        .unwrap();
    // Drain only the storage family: new child setup must not redefine the
    // captured recipient of the already queued event.
    assert_eq!(drain_storage(&mut target, &loader).await, 2);
    assert_eq!(target.eval("JSON.stringify([storageEvents.length,retired.storageEvents.length,storageFrame.contentWindow.storageEvents.length])").unwrap(), "[1,0,0]");
    source
        .eval("localStorage.setItem('after', 'replacement')")
        .unwrap();
    assert_eq!(drain_storage(&mut target, &loader).await, 2);
    assert_eq!(target.eval("JSON.stringify([storageEvents.map(e=>e.key),storageFrame.contentWindow.storageEvents.map(e=>e.key)])").unwrap(), r#"[["before","after"],["after"]]"#);

    target.set_web_storage_handles(&crate::RendererWebStorageHandles::ephemeral());
    source
        .eval("localStorage.setItem('partition', 'isolated')")
        .unwrap();
    assert_eq!(drain_storage(&mut target, &loader).await, 0);
    target.set_web_storage_handles(&storage);
    source
        .eval("localStorage.setItem('partition', 'restored')")
        .unwrap();
    assert_eq!(drain_storage(&mut target, &loader).await, 2);
}

#[tokio::test]
async fn remote_storage_events_survive_document_open_in_the_same_window() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let storage = crate::RendererWebStorageHandles::ephemeral();
    let mut source = new_storage_page_task_executor_test_vm_with_loader(
        "https://storage-events.test/source",
        &loader,
    );
    let mut target = new_storage_page_task_executor_test_vm_with_loader(
        "https://storage-events.test/target",
        &loader,
    );
    source.set_web_storage_handles(&storage);
    target.set_web_storage_handles(&storage);
    source
        .eval("localStorage.setItem('queued', 'before-open')")
        .unwrap();
    target
        .eval(
            r#"
document.open(); document.write('<!doctype html><body>new document'); document.close();
globalThis.eventsAfterOpen = [];
addEventListener('storage', event => eventsAfterOpen.push(event.key + ':' + event.newValue));
"#,
        )
        .unwrap();
    assert_eq!(drain_storage(&mut target, &loader).await, 1);
    assert_eq!(
        target.eval("eventsAfterOpen.join(',')").unwrap(),
        "queued:before-open"
    );
}
