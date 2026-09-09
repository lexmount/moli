use super::*;

async fn run_rejection_task(
    vm: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
) {
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::PromiseRejection,
            loader,
        )
        .await
        .expect("promise rejection task")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn rejection_notification_rechecks_handlers_added_by_an_earlier_notification() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://promise-notification-batch.test/",
        &loader,
    );
    vm.eval(
        r#"
        globalThis.__notifications = [];
        const first = Promise.reject('first');
        const second = Promise.reject('second');
        onunhandledrejection = event => {
            __notifications.push(event.reason);
            event.preventDefault();
            if (event.promise === first) second.catch(() => {});
        };
        onrejectionhandled = () => __notifications.push('rejectionhandled');
    "#,
    )
    .unwrap();
    vm.eval("0").unwrap();
    run_rejection_task(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(__notifications)").unwrap(),
        r#"["first"]"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn main_window_unhandled_rejection_dispatches_to_main_window() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://main-promise-rejection.test/", &loader);

    vm.eval(
        r#"
globalThis.__mainPromiseRejections = [];
addEventListener("unhandledrejection", event => {
  __mainPromiseRejections.push(String(event.reason));
  event.preventDefault();
});
Promise.reject("main-owned");
"#,
    )
    .expect("main Window rejection setup should evaluate");
    vm.eval("0")
        .expect("main Window rejection checkpoint should evaluate");
    assert_eq!(
        vm.eval("JSON.stringify(__mainPromiseRejections)").unwrap(),
        "[]",
        "microtask checkpoints must leave the notification in the DOM task queue"
    );
    run_rejection_task(&mut vm, &loader).await;

    assert_eq!(
        vm.eval("JSON.stringify(__mainPromiseRejections)")
            .expect("main Window rejection result should evaluate"),
        r#"["main-owned"]"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn handler_added_during_unhandled_rejection_does_not_dispatch_rejectionhandled() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://handled-during-notification.test/",
        &loader,
    );

    vm.eval(
        r#"
globalThis.__handledDuringNotificationEvents = [];
const reason = new Error("handled-during-notification");
const rejected = Promise.reject(reason);
onunhandledrejection = event => {
  __handledDuringNotificationEvents.push(event.type);
  event.preventDefault();
  rejected.catch(value => {
    __handledDuringNotificationEvents.push(value === reason ? "handler" : "wrong-reason");
  });
};
onrejectionhandled = event => {
  __handledDuringNotificationEvents.push(event.type);
};
"#,
    )
    .expect("rejection handled during notification setup should evaluate");
    vm.eval("0")
        .expect("unhandled rejection notification checkpoint should evaluate");
    run_rejection_task(&mut vm, &loader).await;
    vm.eval("0")
        .expect("rejection handler reaction checkpoint should evaluate");

    assert_eq!(
        vm.eval("JSON.stringify(__handledDuringNotificationEvents)")
            .expect("rejection handled during notification result should evaluate"),
        r#"["unhandledrejection","handler"]"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn universal_isolated_world_rejection_uses_its_registry_backed_realm() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://isolated-promise-rejection.test/",
        &loader,
    );
    let context_id = vm
        .create_isolated_world("promise-rejection-universal", true)
        .expect("universal isolated world should be created");

    vm.eval_in_isolated_context(
        context_id,
        r#"
globalThis.__isolatedPromiseRejections = [];
addEventListener("unhandledrejection", event => {
  __isolatedPromiseRejections.push(String(event.reason));
  event.preventDefault();
});
Promise.reject("isolated-owned");
"queued"
"#,
    )
    .expect("isolated rejection setup should evaluate");
    vm.eval_in_isolated_context(context_id, "0")
        .expect("isolated rejection checkpoint should evaluate");
    run_rejection_task(&mut vm, &loader).await;

    assert_eq!(
        vm.eval_in_isolated_context(context_id, "JSON.stringify(__isolatedPromiseRejections)",)
            .expect("isolated rejection result should evaluate"),
        r#"["isolated-owned"]"#,
        "strict binding must restore the isolated realm and its Universal registry policy"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn live_child_unhandled_rejection_dispatches_only_to_child_window() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://child-promise-rejection.test/",
        &loader,
    );

    vm.eval(
        r#"
globalThis.__parentPromiseRejections = [];
addEventListener("unhandledrejection", event => {
  __parentPromiseRejections.push(String(event.reason));
  event.preventDefault();
});

const root = document.documentElement ||
  document.appendChild(document.createElement("html"));
const body = document.body || root.appendChild(document.createElement("body"));
const frame = document.createElement("iframe");
body.appendChild(frame);
globalThis.__promiseRejectionChild = frame.contentWindow;
__promiseRejectionChild.eval(`
  globalThis.__childPromiseRejections = [];
  addEventListener("unhandledrejection", event => {
    __childPromiseRejections.push(String(event.reason));
    event.preventDefault();
  });
  Promise.reject("child-owned");
`);
"#,
    )
    .expect("live child rejection setup should evaluate");
    vm.eval("0")
        .expect("live child rejection checkpoint should evaluate");
    run_rejection_task(&mut vm, &loader).await;

    assert_eq!(
        vm.eval(
            r#"JSON.stringify({
  parent: __parentPromiseRejections,
  child: __promiseRejectionChild.__childPromiseRejections
})"#,
        )
        .expect("live child rejection result should evaluate"),
        r#"{"parent":[],"child":["child-owned"]}"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn live_child_rejectionhandled_dispatches_only_to_child_window() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://child-rejection-handled.test/",
        &loader,
    );

    vm.eval(
        r#"
globalThis.__parentPromiseEvents = [];
for (const type of ["unhandledrejection", "rejectionhandled"]) {
  addEventListener(type, event => {
    __parentPromiseEvents.push(type);
    event.preventDefault();
  });
}

const root = document.documentElement ||
  document.appendChild(document.createElement("html"));
const body = document.body || root.appendChild(document.createElement("body"));
const frame = document.createElement("iframe");
body.appendChild(frame);
globalThis.__promiseHandledChild = frame.contentWindow;
__promiseHandledChild.eval(`
  globalThis.__childPromiseEvents = [];
  for (const type of ["unhandledrejection", "rejectionhandled"]) {
    addEventListener(type, event => {
      __childPromiseEvents.push(type);
      event.preventDefault();
    });
  }
  globalThis.__lateHandledPromise = Promise.reject("late-child");
`);
"#,
    )
    .expect("live child late-handler setup should evaluate");
    vm.eval("0")
        .expect("live child unhandled rejection checkpoint should evaluate");
    run_rejection_task(&mut vm, &loader).await;
    vm.eval(r#"__promiseHandledChild.__lateHandledPromise.catch(() => {})"#)
        .expect("parent realm should be able to attach the live child rejection handler");
    assert_eq!(
        vm.eval("JSON.stringify(__promiseHandledChild.__childPromiseEvents)")
            .unwrap(),
        r#"["unhandledrejection"]"#,
        "attaching the handler must only queue rejectionhandled"
    );
    run_rejection_task(&mut vm, &loader).await;

    assert_eq!(
        vm.eval(
            r#"JSON.stringify({
  parent: __parentPromiseEvents,
  child: __promiseHandledChild.__childPromiseEvents
})"#,
        )
        .expect("live child rejectionhandled result should evaluate"),
        r#"{"parent":[],"child":["unhandledrejection","rejectionhandled"]}"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn detached_child_late_handler_does_not_dispatch_rejectionhandled_to_parent_window() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://detached-rejection-handled.test/",
        &loader,
    );

    vm.eval(
        r#"
globalThis.__parentPromiseEvents = [];
for (const type of ["unhandledrejection", "rejectionhandled"]) {
  addEventListener(type, event => {
    __parentPromiseEvents.push(type);
    event.preventDefault();
  });
}

const root = document.documentElement ||
  document.appendChild(document.createElement("html"));
const body = document.body || root.appendChild(document.createElement("body"));
globalThis.__lateHandlerFrame = document.createElement("iframe");
body.appendChild(__lateHandlerFrame);
globalThis.__lateHandlerChild = __lateHandlerFrame.contentWindow;
__lateHandlerChild.eval(`
  globalThis.__childPromiseEvents = [];
  for (const type of ["unhandledrejection", "rejectionhandled"]) {
    addEventListener(type, event => {
      __childPromiseEvents.push(type);
      event.preventDefault();
    });
  }
  globalThis.__lateHandledPromise = Promise.reject("detached-late-child");
`);
"#,
    )
    .expect("detached child late-handler setup should evaluate");
    vm.eval("0")
        .expect("child unhandled rejection checkpoint should evaluate");
    run_rejection_task(&mut vm, &loader).await;
    vm.eval("__lateHandlerFrame.remove()")
        .expect("child frame removal should evaluate");
    vm.eval("__lateHandlerChild.__lateHandledPromise.catch(() => {})")
        .expect("parent realm should be able to attach the detached child rejection handler");

    assert_eq!(
        vm.eval(
            r#"JSON.stringify({
  parent: __parentPromiseEvents,
  child: __lateHandlerChild.__childPromiseEvents
})"#,
        )
        .expect("detached child rejectionhandled result should evaluate"),
        r#"{"parent":[],"child":["unhandledrejection"]}"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn detached_child_dynamic_import_rejection_is_not_reported_to_parent_window() {
    let mut vm = new_storage_page_task_executor_test_vm("https://inactive-import-rejection.test/");

    let promise_shape = vm
        .eval(
            r#"
(() => {
  globalThis.__parentPromiseRejections = [];
  addEventListener("unhandledrejection", event => {
    __parentPromiseRejections.push(String(event.reason));
    event.preventDefault();
  });

  const root = document.documentElement ||
    document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  const child = frame.contentWindow;
  child.eval(`
    globalThis.__inactivePromiseRejections = [];
    addEventListener("unhandledrejection", event => {
      __inactivePromiseRejections.push(String(event.reason));
      event.preventDefault();
    });
  `);
  frame.remove();

  globalThis.__inactiveImportChild = child;
  globalThis.__inactiveImportPromise = child.eval("import('foobar')");
  return String(
    __inactiveImportPromise !== null &&
    typeof __inactiveImportPromise.then === "function"
  );
})()
"#,
        )
        .expect("detached child dynamic import should return without throwing");
    assert_eq!(promise_shape, "true");

    for _ in 0..3 {
        vm.eval("0")
            .expect("detached child rejection checkpoint should evaluate");
    }

    assert!(
        !vm.has_ready_dom_manipulation_family_for_test(
            PageDomManipulationTestFamily::PromiseRejection,
        ),
        "no rejection notification may remain queued"
    );
    assert_eq!(
        vm.eval(
            r#"JSON.stringify({
  parent: __parentPromiseRejections,
  child: __inactiveImportChild.__inactivePromiseRejections
})"#,
        )
        .expect("detached child rejection result should evaluate"),
        r#"{"parent":[],"child":[]}"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn promise_rejection_batches_preserve_dom_fifo_across_checkpoints() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_page_task_executor_test_vm_with_loader("https://promise-fifo.test/", &loader);
    vm.eval(
        r#"
      globalThis.events = [];
      globalThis.receiver = new BroadcastChannel('promise-fifo');
      globalThis.sender = new BroadcastChannel('promise-fifo');
      receiver.onmessage = event => events.push('message:' + event.data);
      onunhandledrejection = event => {
        events.push('rejection:' + event.reason);
        event.preventDefault();
        queueMicrotask(() => events.push('microtask:' + event.reason));
      };
      sender.postMessage('first');
      Promise.reject('first');
    "#,
    )
    .unwrap();
    vm.eval("sender.postMessage('second'); Promise.reject('second');")
        .unwrap();
    assert_eq!(vm.eval("JSON.stringify(events)").unwrap(), "[]");
    for family in [
        PageDomManipulationTestFamily::BroadcastChannel,
        PageDomManipulationTestFamily::PromiseRejection,
        PageDomManipulationTestFamily::BroadcastChannel,
        PageDomManipulationTestFamily::PromiseRejection,
    ] {
        assert!(
            vm.run_one_dom_manipulation_task_executor_turn(family, &loader)
                .await
                .unwrap()
        );
    }
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["message:first","rejection:first","microtask:first","message:second","rejection:second","microtask:second"]"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn handler_added_by_an_earlier_dom_task_suppresses_rejection_notification() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://promise-late-task.test/", &loader);
    vm.eval(
        r#"
      globalThis.events = [];
      globalThis.receiver = new BroadcastChannel('promise-handler-task');
      globalThis.sender = new BroadcastChannel('promise-handler-task');
      receiver.onmessage = () => {
        events.push('task');
        rejected.catch(() => events.push('handler'));
      };
      onunhandledrejection = event => { events.push(event.type); event.preventDefault(); };
      onrejectionhandled = event => events.push(event.type);
      globalThis.rejected = Promise.reject('handled before notification');
      sender.postMessage('attach');
    "#,
    )
    .unwrap();
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::BroadcastChannel,
            &loader,
        )
        .await
        .unwrap()
    );
    run_rejection_task(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["task","handler"]"#
    );
    assert!(!vm.has_ready_dom_manipulation_family_for_test(
        PageDomManipulationTestFamily::PromiseRejection
    ));
}
