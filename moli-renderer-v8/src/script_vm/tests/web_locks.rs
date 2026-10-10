use super::*;
use crate::ensure_v8_for_test as ensure_v8;
use std::time::{Duration, Instant};

#[tokio::test(flavor = "current_thread")]
async fn web_locks_window_runs_native_queue_abort_steal_and_utf16_contract() {
    ensure_v8();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = crate::runtime::PageVmTaskExecutorTestHarness::new(
        url::Url::parse("https://locks.test/").unwrap(),
        &loader,
    );
    vm.set_storage_bucket_store(crate::context_bootstrap::new_shared_storage_bucket_store());
    vm.eval(&format!("const run={};run().then(value=>globalThis.result=value,error=>globalThis.result=String(error));", include_str!("../../worker/thread/tests/web_locks.js"))).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while vm.eval("typeof result").unwrap() == "undefined" {
        vm.run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .unwrap();
        assert!(Instant::now() < deadline, "Web Locks contract timed out");
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(vm.eval("result").unwrap(), "complete");
}

#[tokio::test(flavor = "current_thread")]
async fn web_locks_page_teardown_releases_a_grant_before_its_callback_runs() {
    ensure_v8();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let store = crate::context_bootstrap::new_shared_storage_bucket_store();
    {
        let mut vm = crate::runtime::PageVmTaskExecutorTestHarness::new(
            url::Url::parse("https://locks.test/").unwrap(),
            &loader,
        );
        vm.set_storage_bucket_store(store.clone());
        vm.eval("navigator.locks.request('retired',()=>new Promise(()=>{}));")
            .unwrap();
    }
    let mut vm = crate::runtime::PageVmTaskExecutorTestHarness::new(
        url::Url::parse("https://locks.test/").unwrap(),
        &loader,
    );
    vm.set_storage_bucket_store(store);
    vm.eval("navigator.locks.query().then(snapshot=>globalThis.result=snapshot.held.length+snapshot.pending.length);").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while vm.eval("typeof result").unwrap() == "undefined" {
        vm.run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .unwrap();
        assert!(Instant::now() < deadline, "retired client query timed out");
    }
    assert_eq!(vm.eval("result").unwrap(), "0");
}

#[tokio::test(flavor = "current_thread")]
async fn web_locks_borrowed_navigator_getter_keeps_the_child_owner_and_retires_it() {
    ensure_v8();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = crate::runtime::PageVmTaskExecutorTestHarness::new(
        url::Url::parse("https://locks.test/").unwrap(),
        &loader,
    );
    vm.set_storage_bucket_store(crate::context_bootstrap::new_shared_storage_bucket_store());
    vm.eval(
        r#"
        (async () => {
          const check = (value, message) => { if (!value) throw new Error(message); };
          const getter = Object.getOwnPropertyDescriptor(Navigator.prototype, 'locks').get;
          const frame = document.createElement('iframe');
          document.body.append(frame);
          const other = frame.contentWindow;
          const manager = getter.call(other.navigator);
          check(manager === other.navigator.locks, 'borrowed getter SameObject');
          let started;
          const entered = new Promise(resolve => { started = resolve; });
          manager.request('child-owner', () => {
            started();
            return new Promise(() => {});
          });
          await entered;
          const before = await navigator.locks.query();
          check(before.held.length === 1 && before.held[0].name === 'child-owner',
                'same-origin child shares the storage bucket');
          frame.remove();
          const after = await navigator.locks.query();
          check(after.held.length === 0 && after.pending.length === 0,
                'removing child releases its held lock');
          const error = await LockManager.prototype.query.call(manager).then(
            () => null, error => error);
          check(error instanceof DOMException && error.name === 'InvalidStateError',
                'retained manager cannot use a retired child');
          const lateFrame = document.createElement('iframe');
          document.body.append(lateFrame);
          const retainedNavigator = lateFrame.contentWindow.navigator;
          lateFrame.remove();
          const lateManager = getter.call(retainedNavigator);
          check(typeof lateManager.request === 'function', 'inactive getter returns a manager');
          const lateError = await lateManager.request('late', () => {}).then(
            () => null, error => error);
          check(lateError.name === 'InvalidStateError', 'inactive manager rejects requests');
          globalThis.result = 'complete';
        })().catch(error => { globalThis.result = error.stack || String(error); });
        "#,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while vm.eval("typeof result").unwrap() == "undefined" {
        vm.run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .unwrap();
        assert!(
            Instant::now() < deadline,
            "child Web Locks contract timed out"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(vm.eval("result").unwrap(), "complete");
}
