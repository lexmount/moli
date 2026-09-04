use super::*;

#[tokio::test]
async fn same_origin_nested_children_share_window_security_token_without_top_access() {
    let (root_url, server) = spawn_nested_same_origin_window_access_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let root_url_parsed = Url::parse(&root_url).expect("root child URL");
    let top_url = format!(
        "http://localhost:{}/top.html",
        root_url_parsed
            .port()
            .expect("root child URL should carry a port")
    );
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&top_url, &loader);
    let root_url_literal = serde_json::to_string(&root_url).expect("serialize root child URL");

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__nestedOriginResult = null;
  const frame = document.createElement("iframe");
  frame.src = {root_url_literal};
  addEventListener("message", event => {{
    globalThis.__nestedOriginResult = {{
      data: event.data,
      sourceIsRoot: event.source === frame.contentWindow
    }};
  }});
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__nestedOriginRootFrame = frame;
  return "queued";
}})()
"#
    ))
    .expect("nested same-origin child setup should evaluate");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__nestedOriginResult !== null)",
        "true",
        "nested same-origin child result",
    )
    .await;

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__nestedOriginResult)")
            .expect("nested origin result should evaluate"),
        r#"{"data":{"marker":"nested","parentIsRoot":true,"topDenied":true,"wasmModule":true},"sourceIsRoot":true}"#
    );
    let requests = server.await.expect("nested origin server should finish");
    assert_eq!(requests.len(), 2);
}

#[tokio::test]
async fn queued_window_message_does_not_rebind_to_replacement_child_local_window() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://window-message-local-window-owner.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__windowMessageOwnerEvents = [];
  addEventListener("message", event => {
    __windowMessageOwnerEvents.push(String(event.data));
  });
  const frame = document.createElement("iframe");
  frame.srcdoc = `<!doctype html><script>
    addEventListener("message", event => {
      parent.postMessage("old-local-window-received:" + event.data, "*");
    });
    parent.postMessage("initial-ready", "*");
  <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__windowMessageOwnerFrame = frame;
  return "queued";
})()
"#,
    )
    .expect("initial child window-message owner setup should evaluate");
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("initial child setup should use the selected-task dispatcher");
    for _ in 0..8 {
        if vm
            .eval("String(__windowMessageOwnerEvents.includes('initial-ready'))")
            .expect("initial child readiness should evaluate")
            == "true"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("initial child readiness should advance");
    }

    vm.eval(
        r#"
(() => {
  __windowMessageOwnerFrame.contentWindow.postMessage("stale-target", "*");
  __windowMessageOwnerFrame.srcdoc = `<!doctype html><script>
    addEventListener("message", event => {
      parent.postMessage("replacement-local-window-received:" + event.data, "*");
    });
    parent.postMessage("replacement-ready", "*");
  <\/script>`;
  return "navigating";
})()
"#,
    )
    .expect("queued message plus child replacement should evaluate");
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            crate::frame_owner_model::ChildFrameSemanticTurnKind::NavigationCommit,
            &loader,
        )
        .await
        .expect("replacement child navigation commit should use the selected-task dispatcher"),
        "replacement child navigation commit should be runnable before the stale message"
    );
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("replacement child setup should use the selected-task dispatcher");

    for _ in 0..12 {
        let events = vm
            .eval("JSON.stringify(__windowMessageOwnerEvents)")
            .expect("window-message owner events should evaluate");
        if events.contains("replacement-ready") {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("child setup should use the selected-task dispatcher");
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("child replacement should advance");
    }
    for _ in 0..4 {
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("stale window-message timer should drain");
    }

    assert_eq!(
        vm.eval("JSON.stringify(__windowMessageOwnerEvents)")
            .expect("window-message owner result should evaluate"),
        r#"["initial-ready","replacement-ready"]"#
    );
}

#[tokio::test]
async fn queued_window_message_survives_same_local_window_document_open() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://window-message-document-open.test/",
        &loader,
    );
    let before_owner = vm
        .current_main_document_task_owner()
        .expect("main window-message owner should exist");

    assert_eq!(
        vm.eval(
            r#"
postMessage("before-document-open", "*");
document.open();
document.close();
globalThis.__documentOpenWindowMessages = [];
onmessage = event => __documentOpenWindowMessages.push(event.data);
"queued"
"#,
        )
        .expect("queued postMessage plus document.open should evaluate"),
        "queued"
    );
    let after_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main window-message owner should exist");
    assert_ne!(after_owner, before_owner);
    assert_eq!(
        after_owner.local_window_id, before_owner.local_window_id,
        "document.open must retain the window-message target LocalWindow"
    );

    for _ in 0..4 {
        if vm
            .eval("__documentOpenWindowMessages.length")
            .expect("document.open window-message count should evaluate")
            == "1"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("document.open window message should advance");
    }
    assert_eq!(
        vm.eval("__documentOpenWindowMessages.join('|')")
            .expect("document.open window-message result should evaluate"),
        "before-document-open"
    );
}

#[tokio::test]
async fn window_timer_survives_same_local_window_document_open() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://timer-document-open.test/", &loader);
    let before_owner = vm
        .current_main_document_task_owner()
        .expect("main timer owner should exist");

    assert_eq!(
        vm.eval(
            r#"
globalThis.__documentOpenTimerEvents = [];
setTimeout(() => __documentOpenTimerEvents.push("preserved"), 0);
document.open();
document.write("<!doctype html><title>replacement</title>");
document.close();
"queued"
"#,
        )
        .expect("queued timer plus document.open should evaluate"),
        "queued"
    );
    let after_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main timer owner should exist");
    assert_ne!(after_owner.document_id, before_owner.document_id);
    assert_eq!(after_owner.local_window_id, before_owner.local_window_id);

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("preserved LocalWindow timer should drain");
    assert_eq!(
        vm.eval("__documentOpenTimerEvents.join('|')")
            .expect("document.open timer result should evaluate"),
        "preserved"
    );
}

#[tokio::test]
async fn child_navigation_retires_old_local_window_timers() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://timer-local-window-owner.test/", &loader);

    vm.eval(
        r#"
globalThis.__childTimerOwnerEvents = [];
const frame = document.createElement("iframe");
frame.srcdoc = `<!doctype html><script>
  parent.__childTimerOwnerEvents.push("old-ready");
  setTimeout(() => parent.__childTimerOwnerEvents.push("stale-timer"), 0);
<\/script>`;
(document.body || document.documentElement || document).appendChild(frame);
globalThis.__childTimerOwnerFrame = frame;
"queued"
"#,
    )
    .expect("old child timer setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    assert_eq!(
        vm.eval("__childTimerOwnerEvents.join('|')")
            .expect("old child timer setup result should evaluate"),
        "old-ready"
    );

    vm.eval(
        r#"
__childTimerOwnerFrame.srcdoc = `<!doctype html><script>
  parent.__childTimerOwnerEvents.push("replacement-ready");
<\/script>`;
"navigating"
"#,
    )
    .expect("child timer replacement should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("retired child timer should leave the timer queue quiescent");

    assert_eq!(
        vm.eval("__childTimerOwnerEvents.join('|')")
            .expect("child timer replacement result should evaluate"),
        "old-ready|replacement-ready"
    );
}

#[tokio::test]
async fn child_realm_retirement_cancels_callback_targeting_live_top_window() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_test_vm_with_loader("https://timer-callback-relevant-realm.test/", &loader);

    vm.eval(
        r#"
globalThis.__timerRelevantRealmEvents = [];
const frame = document.createElement("iframe");
frame.srcdoc = `<!doctype html><script>
  parent.__timerRelevantRealmEvents.push("old-ready");
  parent.setTimeout(
    () => parent.__timerRelevantRealmEvents.push("stale-callback-realm"),
    0
  );
<\/script>`;
(document.body || document.documentElement || document).appendChild(frame);
globalThis.__timerRelevantRealmFrame = frame;
"queued"
"#,
    )
    .expect("child callback-realm timer setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    vm.eval(
        r#"
__timerRelevantRealmFrame.srcdoc = `<!doctype html><script>
  parent.__timerRelevantRealmEvents.push("replacement-ready");
<\/script>`;
"navigating"
"#,
    )
    .expect("child callback-realm replacement should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("destroyed callback-realm timer should leave the queue quiescent");

    assert_eq!(
        vm.eval("__timerRelevantRealmEvents.join('|')")
            .expect("child callback-realm timer result should evaluate"),
        "old-ready|replacement-ready"
    );
}

#[tokio::test]
async fn child_window_timer_accepts_before_target_realm_materialization() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_test_vm_with_loader("https://timer-pending-child-realm.test/", &loader);

    assert_eq!(
        vm.eval(
            r#"
globalThis.__pendingRealmTimerEvents = [];
const frame = document.createElement("iframe");
frame.srcdoc = "<!doctype html><title>child</title>";
(document.body || document.documentElement || document).appendChild(frame);
const childWindow = frame.contentWindow;
const timerId = childWindow.setTimeout(
  () => __pendingRealmTimerEvents.push("fired"),
  0
);
const sourceTimerId = childWindow.setTimeout(
  "parent.__pendingRealmTimerEvents.push('source-fired')",
  0
);
String(timerId > 0 && sourceTimerId > 0)
"#,
        )
        .expect("pre-materialization child timer acceptance should evaluate"),
        "true"
    );

    vm.drain_pending_child_frame_work_for_test();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("materialized child timer should drain");
    assert_eq!(
        vm.eval("__pendingRealmTimerEvents.join('|')")
            .expect("pre-materialization child timer result should evaluate"),
        "fired|source-fired"
    );
}

#[tokio::test]
async fn initial_empty_child_timer_survives_superseded_precommit_navigation() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_test_vm_with_loader("https://timer-pending-child-replacement.test/", &loader);

    assert_eq!(
        vm.eval(
            r#"
globalThis.__initialEmptyTimerEvents = [];
globalThis.__initialEmptyMainWindow = window;
globalThis.__initialEmptyMainDocument = document;
const frame = document.createElement("iframe");
frame.srcdoc = "<!doctype html><title>superseded child</title>";
(document.body || document.documentElement || document).appendChild(frame);
const initialWindow = frame.contentWindow;
const initialDocument = initialWindow.document;
initialWindow.__initialEmptyMarker = "preserved";
initialWindow.addEventListener(
  "initial-empty-transition",
  () => __initialEmptyTimerEvents.push("preserved-listener")
);
initialDocument.addEventListener(
  "initial-empty-document-transition",
  () => __initialEmptyTimerEvents.push("stale-document-listener")
);
const timerId = initialWindow.setTimeout(
  () => __initialEmptyTimerEvents.push("preserved-timer"),
  0
);
frame.srcdoc = `<!doctype html><script>
  parent.__initialEmptyTimerEvents.push(
    "committed-ready:" + window.__initialEmptyMarker
  );
  window.dispatchEvent(new Event("initial-empty-transition"));
  document.dispatchEvent(new Event("initial-empty-document-transition"));
<\/script>`;
String(timerId > 0)
"#,
        )
        .expect("initial-empty timer setup should evaluate"),
        "true"
    );

    vm.drain_pending_child_frame_work_for_test();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("preserved initial-empty timer should drain");
    assert_eq!(
        vm.eval("__initialEmptyTimerEvents.join('|')")
            .expect("initial-empty timer result should evaluate"),
        "committed-ready:preserved|preserved-listener|preserved-timer"
    );
    assert_eq!(
        vm.eval(
            "String(window === __initialEmptyMainWindow && document === __initialEmptyMainDocument)"
        )
        .expect("main identity after child initial-empty transition should evaluate"),
        "true",
        "child initial-empty reuse must not replace the main Window or Document"
    );
}

#[tokio::test]
async fn initial_empty_document_domain_prevents_local_window_reuse() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://sub.initial-empty-domain.test/", &loader);

    assert_eq!(
        vm.eval(
            r#"
globalThis.__initialEmptyDomainEvents = [];
const frame = document.createElement("iframe");
frame.srcdoc = "<!doctype html><title>superseded child</title>";
(document.body || document.documentElement || document).appendChild(frame);
const initialWindow = frame.contentWindow;
initialWindow.__initialEmptyDomainMarker = "must-not-survive";
const timerId = initialWindow.setTimeout(
  () => __initialEmptyDomainEvents.push("stale-timer"),
  0
);
initialWindow.document.domain = "initial-empty-domain.test";
frame.srcdoc = `<!doctype html><script>
  parent.__initialEmptyDomainEvents.push(
    "committed-ready:" + String(window.__initialEmptyDomainMarker)
  );
<\/script>`;
String(timerId > 0)
"#,
        )
        .expect("document.domain transition setup should evaluate"),
        "true"
    );

    vm.drain_pending_child_frame_work_for_test();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("retired initial-empty LocalWindow timer should leave the queue quiescent");
    assert_eq!(
        vm.eval("__initialEmptyDomainEvents.join('|')")
            .expect("document.domain transition result should evaluate"),
        "committed-ready:undefined"
    );
}

#[tokio::test]
async fn window_clear_timer_is_scoped_to_its_local_window() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://timer-clear-owner.test/", &loader);

    assert_eq!(
        vm.eval(
            r#"
globalThis.__scopedTimerEvents = [];
const popup = open("about:blank");
const popupTimer = popup.setTimeout(() => __scopedTimerEvents.push("popup"), 1);
clearTimeout(popupTimer);
const popupSourceTimer = popup.setTimeout(
  "globalThis.__scopedTimerEvents.push('popup-source')",
  1
);
clearTimeout(popupSourceTimer);
const topTimer = setTimeout(() => __scopedTimerEvents.push("top"), 1);
popup.clearTimeout(topTimer);
const cancelledPopupTimer = popup.setTimeout(
  () => __scopedTimerEvents.push("cancel-failed"),
  1
);
popup.clearTimeout(cancelledPopupTimer);
"queued"
"#,
        )
        .expect("cross-LocalWindow timer cancellation setup should evaluate"),
        "queued"
    );

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("scoped LocalWindow timers should drain");
    assert_eq!(
        vm.eval("JSON.stringify(__scopedTimerEvents.sort())")
            .expect("scoped timer result should evaluate"),
        r#"["popup","popup-source","top"]"#
    );
}

#[tokio::test]
async fn lightweight_popup_close_retires_its_local_window_timers() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://timer-popup-close.test/", &loader);

    vm.eval(
        r#"
globalThis.__closedPopupTimerEvents = [];
const popup = open("about:blank");
popup.setTimeout(() => __closedPopupTimerEvents.push("stale-popup-timer"), 0);
popup.close();
"closed"
"#,
    )
    .expect("popup timer close setup should evaluate");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(popup.opener === null)",
        "true",
        "queued popup definite close",
    )
    .await;
    assert_eq!(
        vm.eval("__closedPopupTimerEvents.length")
            .expect("closed popup timer result should evaluate"),
        "0"
    );
}

#[tokio::test]
async fn child_interval_does_not_reschedule_after_local_window_navigation() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_test_vm_with_loader("https://timer-interval-navigation.test/", &loader);

    vm.eval(
        r#"
globalThis.__childIntervalOwnerEvents = [];
const frame = document.createElement("iframe");
frame.srcdoc = `<!doctype html><script>
  setInterval(() => {
    parent.__childIntervalOwnerEvents.push("tick");
    parent.__childIntervalOwnerFrame.srcdoc =
      "<!doctype html><script>parent.__childIntervalOwnerEvents.push('replacement-ready');<\\/script>";
  }, 0);
<\/script>`;
(document.body || document.documentElement || document).appendChild(frame);
globalThis.__childIntervalOwnerFrame = frame;
"queued"
"#,
    )
    .expect("child interval navigation setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    assert!(
        vm.run_next_due_timer_callback_for_test(&loader)
            .await
            .expect("first child interval turn should run")
    );
    vm.drain_pending_child_frame_work_for_test();
    let events_after_replacement = vm
        .eval("__childIntervalOwnerEvents.join('|')")
        .expect("child interval replacement state should evaluate");
    assert_eq!(events_after_replacement, "tick|replacement-ready");
    vm.advance_timers_until_deadline_for_test_with_deadline(
        &loader,
        std::time::Instant::now() + std::time::Duration::from_millis(20),
    )
    .await
    .expect("retired child interval queue should stay quiescent");

    assert_eq!(
        vm.eval("__childIntervalOwnerEvents.join('|')")
            .expect("child interval navigation result should evaluate"),
        "tick|replacement-ready"
    );
}

#[tokio::test]
async fn queued_window_message_survives_source_local_window_retirement() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://window-message-source-retirement.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__sourceRetirementEvents = [];
  const frame = document.createElement("iframe");
  frame.srcdoc = `<!doctype html><script>
    addEventListener("message", event => {
      parent.postMessage("from-retiring-source:" + event.data, "*");
      parent.__sourceRetirementFrame.srcdoc =
        "<!doctype html><script>parent.postMessage('replacement-ready', '*');<\\/script>";
    });
    parent.postMessage("initial-ready", "*");
  <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__sourceRetirementFrame = frame;
  addEventListener("message", event => {
    __sourceRetirementEvents.push({
      data: String(event.data),
      sourceIsStableProxy: event.source === frame.contentWindow
    });
  });
  return "queued";
})()
"#,
    )
    .expect("source-retirement window-message setup should evaluate");
    for _ in 0..128 {
        if !vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("initial child setup should use the selected-task dispatcher")
        {
            break;
        }
    }
    for _ in 0..8 {
        if vm
            .eval("String(__sourceRetirementEvents.some(event => event.data === 'initial-ready'))")
            .expect("initial source-retirement readiness should evaluate")
            == "true"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("initial source-retirement readiness should advance");
    }

    vm.eval("__sourceRetirementFrame.contentWindow.postMessage('go', '*')")
        .expect("message to retiring source should queue");
    assert!(
        vm.run_one_window_message_executor_turn(&loader)
            .await
            .expect("target child message should run")
    );
    for _ in 0..128 {
        if !vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("replacement child setup should use the selected-task dispatcher")
        {
            break;
        }
    }

    for _ in 0..8 {
        if vm
            .eval(
                "String(__sourceRetirementEvents.some(event => event.data === 'replacement-ready'))",
            )
            .expect("replacement source readiness should evaluate")
            == "true"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("source-retirement messages should advance");
    }

    assert_eq!(
        vm.eval("JSON.stringify(__sourceRetirementEvents)")
            .expect("source-retirement result should evaluate"),
        r#"[{"data":"initial-ready","sourceIsStableProxy":true},{"data":"from-retiring-source:go","sourceIsStableProxy":true},{"data":"replacement-ready","sourceIsStableProxy":true}]"#
    );
}

#[tokio::test]
async fn child_post_message_reply_is_delivered_after_sender_installs_late_listener() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://child-postmessage-late-listener.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__lateChildReplyMessages = [];
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <script>
      addEventListener("message", event => {
        parent.postMessage("reply:" + event.data, "*");
      });
    </` + `script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__lateChildReplyFrame = frame;
  return "ready";
})()
"#,
    )
    .expect("late listener child setup should evaluate");
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("child setup should use the selected-task dispatcher");

    vm.eval(
        r#"
(() => {
  __lateChildReplyFrame.contentWindow.postMessage("go", "*");
  addEventListener("message", event => {
    __lateChildReplyMessages.push({
      data: event.data,
      sourceIsChild: event.source === __lateChildReplyFrame.contentWindow
    });
  });
  return "posted";
})()
"#,
    )
    .expect("late listener postMessage should evaluate");

    for _ in 0..4 {
        if vm
            .eval("String(globalThis.__lateChildReplyMessages.length)")
            .expect("late listener message length should evaluate")
            == "1"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("late listener reply should advance");
    }

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__lateChildReplyMessages)")
            .expect("late listener messages should evaluate"),
        r#"[{"data":"reply:go","sourceIsChild":true}]"#
    );
}

#[tokio::test]
async fn storage_events_preserve_wpt_dom_string_utf16_units() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://storage-event-domstring-units.test/page",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  localStorage.clear();
  globalThis.__storageEvents = [];
  const frame = document.createElement('iframe');
  frame.srcdoc = `<body onstorage="
    const units = value => value === null ? null : Array.from({ length: value.length }, (_, index) => value.charCodeAt(index));
    parent.__storageEvents.push({
      key: units(event.key),
      oldValue: units(event.oldValue),
      newValue: units(event.newValue),
      storageArea: event.storageArea === localStorage
    });
  "></body>`;
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("storage event UTF-16 setup should evaluate");
    drain_pending_page_child_frame_work_for_test(&mut vm).await;

    assert_eq!(
        vm.eval(
            r#"
(() => {
const key = String.fromCharCode(0xD800);
const value = String.fromCharCode(0xDC00);
localStorage.setItem(key, value);
return __storageEvents.length;
})()
"#
        )
        .expect("first surrogate storage mutation should evaluate"),
        "0"
    );
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::StorageEvent,
            &loader,
        )
        .await
        .expect("selected dispatcher should advance first surrogate storage event")
    );

    vm.eval(
        r#"
(() => {
const key = String.fromCharCode(0xD800);
const value = String.fromCharCode(0xD83C, 0xDF4D);
localStorage.setItem(key, value);
})()
"#,
    )
    .expect("second surrogate storage mutation should evaluate");
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::StorageEvent,
            &loader,
        )
        .await
        .expect("selected dispatcher should advance second surrogate storage event")
    );

    assert_eq!(
        vm.eval("JSON.stringify(__storageEvents)")
            .expect("storage surrogate event result should evaluate"),
        r#"[{"key":[55296],"oldValue":null,"newValue":[56320],"storageArea":true},{"key":[55296],"oldValue":[56320],"newValue":[55356,57165],"storageArea":true}]"#
    );
}

#[tokio::test]
async fn repeated_iframe_src_assignment_reloads_child_storage_event_handler() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://storage-events-reload.test/page",
        &loader,
    );

    let setup = vm
        .eval(
            r#"
(() => {
  localStorage.clear();
  globalThis.__storageEvents = [];
  globalThis.__storageChildLoads = 0;
  const childMarkup = `<!doctype html><body onstorage="
    parent.__storageEvents.push([
      event.key,
      event.oldValue === null,
      event.newValue,
      event.url,
      event.storageArea === localStorage
    ].join('|'));
  "></body>`;
  const childUrl = URL.createObjectURL(new Blob([childMarkup], { type: "text/html" }));
  const frame = document.createElement("iframe");
  frame.onload = () => { __storageChildLoads += 1; };
  frame.src = childUrl;
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__storageFrame = frame;
  globalThis.__storageChildUrl = childUrl;
  return "queued";
})()
"#,
        )
        .expect("initial child storage event frame setup should evaluate");
    assert_eq!(setup, "queued");
    drain_pending_page_child_frame_work_for_test(&mut vm).await;
    assert_eq!(
        vm.eval("__storageChildLoads")
            .expect("initial child storage frame load count should evaluate"),
        "1"
    );

    let before_reload = vm
        .eval("__storageFrame.src = __storageChildUrl; __storageChildLoads")
        .expect("same-src child reload should evaluate");
    assert_eq!(
        before_reload, "1",
        "same-src assignment should queue a new navigation instead of firing load synchronously"
    );
    drain_pending_page_child_frame_work_for_test(&mut vm).await;
    assert_eq!(
        vm.eval("__storageChildLoads")
            .expect("same-src child reload count should evaluate"),
        "2",
        "outside an active load event, assigning the same iframe src must reload"
    );

    assert_eq!(
        vm.eval("localStorage.setItem('k', 'v'); __storageEvents.length")
            .expect("storage mutation after child reload should evaluate"),
        "0",
        "storage events must remain queued after the reload"
    );
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::StorageEvent,
            &loader,
        )
        .await
        .expect("selected dispatcher should advance reloaded child storage event")
    );
    assert_eq!(
        vm.eval("__storageEvents.join('||')")
            .expect("reloaded child storage event result should evaluate"),
        "k|true|v|https://storage-events-reload.test/page|true"
    );
}

#[test]
fn document_open_preserves_message_ports_owned_by_main_local_window() {
    let mut vm = new_storage_test_vm("https://message-port-document-open.test/");

    vm.eval(
        r#"
(() => {
  const channel = new MessageChannel();
  globalThis.__documentOpenPort1 = channel.port1;
  globalThis.__documentOpenPort2 = channel.port2;
  return "created";
})()
"#,
    )
    .expect("main MessagePort pair should be created");

    let before_owner = vm
        ._context_host
        .borrow()
        .current_main_document_task_owner()
        .expect("main document owner should exist");
    let before_ports = vm
        ._context_host
        .borrow()
        .message_port_execution_context_owners_for_test();
    assert_eq!(before_ports.len(), 2);
    assert!(before_ports.iter().all(|(_, owner, _)| {
        *owner
            == crate::native_bridge::WindowExecutionContextOwner::Frame(
                before_owner.local_window_id,
            )
    }));

    vm.eval(
        r#"
document.open();
document.write("<!doctype html><title>replacement</title>");
document.close();
"opened"
"#,
    )
    .expect("document.open replacement should evaluate");

    let after_owner = vm
        ._context_host
        .borrow()
        .current_main_document_task_owner()
        .expect("replacement main document owner should exist");
    assert_eq!(after_owner.local_window_id, before_owner.local_window_id);
    assert_ne!(after_owner.document_id, before_owner.document_id);
    assert_eq!(
        vm._context_host
            .borrow()
            .message_port_execution_context_owners_for_test(),
        before_ports,
        "document.open must not retire or rebind LocalWindow-owned MessagePorts"
    );
}

#[tokio::test]
async fn transferred_child_message_port_rehomes_and_retires_with_local_window() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://message-port-local-window-owner.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__messagePortOwnerEvents = [];
  addEventListener("message", event => {
    __messagePortOwnerEvents.push(String(event.data));
  });
  const channel = new MessageChannel();
  globalThis.__messagePortOwnerLocalPort = channel.port1;
  globalThis.__messagePortOwnerTransferredPort = channel.port2;
  const frame = document.createElement("iframe");
  frame.srcdoc = `<!doctype html><script>
    addEventListener("message", event => {
      globalThis.__ownedTransferredPort = event.ports[0];
      __ownedTransferredPort.start();
      parent.postMessage("port-bound", "*");
    });
    parent.postMessage("child-ready", "*");
  <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__messagePortOwnerFrame = frame;
  return "queued";
})()
"#,
    )
    .expect("MessagePort execution-context owner setup should evaluate");

    let main_local_window_id = vm
        ._context_host
        .borrow()
        .current_main_document_task_owner()
        .expect("main document owner should exist")
        .local_window_id;
    let main_owner = crate::native_bridge::WindowExecutionContextOwner::Frame(main_local_window_id);
    let initial_owners = vm
        ._context_host
        .borrow()
        .message_port_execution_context_owners_for_test();
    assert_eq!(initial_owners.len(), 2);
    assert!(
        initial_owners
            .iter()
            .all(|(_, owner, _)| *owner == main_owner)
    );

    for _ in 0..8 {
        if vm
            .eval("String(__messagePortOwnerEvents.includes('child-ready'))")
            .expect("child-ready state should evaluate")
            == "true"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("child setup should use the selected-task dispatcher");
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should advance child readiness");
    }
    assert_eq!(
        vm.eval("String(__messagePortOwnerEvents.includes('child-ready'))")
            .expect("child-ready completion should evaluate"),
        "true"
    );

    vm.eval(
        r#"
__messagePortOwnerFrame.contentWindow.postMessage(
  "bind-port",
  "*",
  [__messagePortOwnerTransferredPort]
);
"transferred"
"#,
    )
    .expect("MessagePort transfer should evaluate");
    for _ in 0..8 {
        if vm
            .eval("String(__messagePortOwnerEvents.includes('port-bound'))")
            .expect("port-bound state should evaluate")
            == "true"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("child setup should use the selected-task dispatcher");
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should advance MessagePort transfer");
    }
    assert_eq!(
        vm.eval("String(__messagePortOwnerEvents.includes('port-bound'))")
            .expect("port-bound completion should evaluate"),
        "true"
    );

    let transferred_owners = vm
        ._context_host
        .borrow()
        .message_port_execution_context_owners_for_test();
    assert_eq!(transferred_owners.len(), 2);
    assert_eq!(
        transferred_owners
            .iter()
            .filter(|(_, owner, _)| *owner == main_owner)
            .count(),
        1
    );
    let child_owner = transferred_owners
        .iter()
        .find_map(|(_, owner, _)| (*owner != main_owner).then_some(*owner))
        .expect("transferred endpoint should be owned by the child LocalWindow");
    assert!(matches!(
        child_owner,
        crate::native_bridge::WindowExecutionContextOwner::Frame(_)
    ));

    vm.eval(
        r#"
__messagePortOwnerFrame.srcdoc =
  "<!doctype html><script>parent.postMessage('replacement-ready', '*');<\/script>";
"navigating"
"#,
    )
    .expect("child replacement should evaluate");
    for _ in 0..8 {
        if vm
            .eval("String(__messagePortOwnerEvents.includes('replacement-ready'))")
            .expect("replacement-ready state should evaluate")
            == "true"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("child setup should use the selected-task dispatcher");
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should advance child replacement");
    }
    assert_eq!(
        vm.eval("String(__messagePortOwnerEvents.includes('replacement-ready'))")
            .expect("replacement-ready completion should evaluate"),
        "true"
    );

    let replacement_owners = vm
        ._context_host
        .borrow()
        .message_port_execution_context_owners_for_test();
    assert_eq!(replacement_owners.len(), 1);
    assert_eq!(replacement_owners[0].1, main_owner);
    assert!(
        replacement_owners
            .iter()
            .all(|(_, owner, _)| *owner != child_owner),
        "navigation must actively retire the old child LocalWindow endpoint"
    );
}

#[tokio::test]
async fn stale_child_message_port_does_not_dispatch_to_reused_iframe_generation() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://message-port-child-generation.test/",
        &loader,
    );

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__messagePortGenerationEvents = [];
  addEventListener("message", event => {
    __messagePortGenerationEvents.push("window:" + event.data + ":" + event.origin);
  });

  const frame = document.createElement("iframe");
  frame.srcdoc = `<!doctype html><script>
    const channel = new MessageChannel();
    channel.port2.onmessage = () => {
      parent.postMessage("stale-port-handler-ran", "*");
    };
    parent.__staleChildPort = channel.port1;
    parent.postMessage("first-ready", "*");
  <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__messagePortGenerationFrame = frame;
  return "queued";
})()
"#,
        )
        .expect("stale child MessagePort setup should evaluate");
    assert_eq!(setup, "queued");

    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("child setup should use the selected-task dispatcher");
    for _ in 0..4 {
        if vm
            .eval(
                r#"String(globalThis.__messagePortGenerationEvents.includes(
  "window:first-ready:null"
))"#,
            )
            .expect("first child ready state should evaluate")
            == "true"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should observe first child ready");
    }

    vm.eval(
        r#"
globalThis.__messagePortGenerationEvents = [];
__messagePortGenerationFrame.srcdoc = "<!doctype html><script>parent.postMessage('second-ready', '*');<\/script>";
"#,
    )
    .expect("second child navigation should evaluate");
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("child setup should use the selected-task dispatcher");
    for _ in 0..4 {
        if vm
            .eval(
                r#"String(globalThis.__messagePortGenerationEvents.includes(
  "window:second-ready:https://message-port-child-generation.test"
))"#,
            )
            .expect("second child ready state should evaluate")
            == "true"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should observe second child ready");
    }

    vm.eval("__staleChildPort.postMessage('should-not-dispatch')")
        .expect("posting to stale child MessagePort should evaluate");
    for _ in 0..4 {
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should flush stale child MessagePort wake");
    }

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__messagePortGenerationEvents)")
            .expect("stale child MessagePort events should evaluate"),
        r#"["window:second-ready:https://message-port-child-generation.test"]"#
    );
}

#[tokio::test]
async fn child_storage_mutations_queue_events_to_parent_window() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://storage-child.test/page",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  sessionStorage.clear();
  globalThis.__storageEvents = [];
  addEventListener('storage', event => {
    __storageEvents.push({
      key: event.key,
      oldValue: event.oldValue,
      newValue: event.newValue,
      storageArea: event.storageArea === sessionStorage,
      instance: event instanceof StorageEvent,
      tag: Object.prototype.toString.call(event)
    });
  });
  const frame = document.createElement('iframe');
  frame.srcdoc = `<script>sessionStorage.setItem('child', '1');<\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  return __storageEvents.length;
})()
"#,
    )
    .expect("child storage setup should evaluate");
    drain_pending_page_child_frame_work_for_test(&mut vm).await;
    assert_eq!(
        vm.eval("__storageEvents.length")
            .expect("child storage event should still be queued"),
        "0"
    );

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::StorageEvent,
            &loader,
        )
        .await
        .expect("selected dispatcher should advance child-origin storage event")
    );
    assert_eq!(
        vm.eval("JSON.stringify(__storageEvents)")
            .expect("child storage event result should evaluate"),
        r#"[{"key":"child","oldValue":null,"newValue":"1","storageArea":true,"instance":true,"tag":"[object StorageEvent]"}]"#
    );
}

#[tokio::test]
async fn opaque_origin_frames_reject_web_storage_access() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://opaque-storage.test/page.html",
        &loader,
    );

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__opaqueStorageMessages = [];
  addEventListener("message", event => {
    __opaqueStorageMessages.push(String(event.data));
  });

  const createFrame = id => {
    const frame = document.createElement("iframe");
    frame.setAttribute("sandbox", "allow-scripts");
    frame.srcdoc = `<script>
      const outcome = callback => {
        try {
          callback();
          return "resolved";
        } catch (error) {
          return error && error.name;
        }
      };
      parent.postMessage([
        "${id}",
        outcome(() => localStorage),
        outcome(() => sessionStorage),
        location.origin
      ].join(":"), "*");
    <\/script>`;
    return frame;
  };

  const host = document.body || document.documentElement || document;
  host.appendChild(createFrame("left"));
  host.appendChild(createFrame("right"));
  return "queued";
})()
"#,
        )
        .expect("opaque WebStorage setup should evaluate");
    assert_eq!(setup, "queued");

    for _ in 0..8 {
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
            .await
            .expect("child setup should use the selected-task dispatcher");
        let message_count = vm
            .eval(r#"String(__opaqueStorageMessages.length)"#)
            .expect("opaque WebStorage message count should evaluate");
        if message_count == "2" {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("opaque WebStorage child load should advance");
    }
    for _ in 0..4 {
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("opaque WebStorage storage event drain should advance");
    }

    let result = vm
        .eval("JSON.stringify(__opaqueStorageMessages.sort())")
        .expect("opaque WebStorage messages should evaluate");
    assert_eq!(
        result,
        r#"["left:SecurityError:SecurityError:null","right:SecurityError:SecurityError:null"]"#
    );
}

#[test]
fn top_web_storage_receiver_ignores_ambient_opaque_child_owner() {
    let mut vm = new_storage_test_vm("https://top-web-storage-owner.test/page.html");
    vm.eval(
        r#"
(() => {
  localStorage.clear();
  localStorage.setItem("top-key", "top-value");
  const frame = document.createElement("iframe");
  frame.setAttribute("sandbox", "allow-scripts");
  frame.srcdoc = "<!doctype html><script>void localStorage;<\/script>";
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("top WebStorage owner setup should evaluate");

    let child_handle = vm
        ._context_host
        .borrow()
        .child_browsing_context_handles_in_document_order()[0];
    assert!(
        vm._context_host
            .borrow()
            .child_browsing_context_has_opaque_origin(child_handle),
        "sandboxed child should have an opaque storage origin"
    );

    let top_context_ptr = &vm.page_default_runtime.context as *const v8::Global<v8::Context>;
    vm.with_context_scope_by_ptr(top_context_ptr, |scope, _host_ptr| {
        let _previous =
            crate::native_bridge::enter_active_child_window_scope(scope, Some(child_handle));
        Ok(())
    })
    .expect("ambient opaque child owner should install");

    let result = vm.eval("localStorage.getItem('top-key')");
    vm.with_context_scope_by_ptr(top_context_ptr, |scope, _host_ptr| {
        let _previous = crate::native_bridge::enter_active_child_window_scope(scope, None);
        Ok(())
    })
    .expect("ambient opaque child owner should clear");

    assert_eq!(
        result.expect("top localStorage should use the top Window receiver"),
        "top-value"
    );
}

#[tokio::test]
async fn third_party_web_storage_is_partitioned_by_top_level_site() {
    let (child_origin, server) = spawn_web_storage_partition_child_server(3).await;
    let child_url = format!("{child_origin}/partition-child.html");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let web_storage = crate::RendererWebStorageHandles::ephemeral();

    let first_a = run_web_storage_partition_probe(
        "http://top-a-webstorage-partition.test/page.html",
        &child_url,
        "a1",
        &loader,
        &web_storage,
    )
    .await;
    assert_eq!(
        first_a,
        r#"{"label":"a1","beforeLocal":null,"beforeSession":null,"afterLocal":"a1","afterSession":"a1"}"#
    );

    let first_b = run_web_storage_partition_probe(
        "http://top-b-webstorage-partition.test/page.html",
        &child_url,
        "b1",
        &loader,
        &web_storage,
    )
    .await;
    assert_eq!(
        first_b,
        r#"{"label":"b1","beforeLocal":null,"beforeSession":null,"afterLocal":"b1","afterSession":"b1"}"#
    );

    let second_a = run_web_storage_partition_probe(
        "http://top-a-webstorage-partition.test/second.html",
        &child_url,
        "a2",
        &loader,
        &web_storage,
    )
    .await;
    assert_eq!(
        second_a,
        r#"{"label":"a2","beforeLocal":"a1","beforeSession":"a1","afterLocal":"a2","afterSession":"a2"}"#
    );

    let requests = server.await.expect("partition child server should finish");
    assert_eq!(requests.len(), 3);
    assert!(
        requests
            .iter()
            .all(|request| request.starts_with("GET /partition-child.html?label=")),
        "unexpected child frame requests: {requests:?}"
    );
}

#[tokio::test]
async fn third_party_about_blank_popup_does_not_reuse_opener_or_first_party_storage_area() {
    let (child_origin, server) = spawn_about_blank_popup_storage_child_server().await;
    let child_url = format!("{child_origin}/popup-child.html");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "http://top-about-blank-popup-partition.test/page.html",
        &loader,
    );

    let child_url_literal = serde_json::to_string(&child_url).expect("child url should serialize");
    vm.eval(&format!(
        r#"
(() => {{
  localStorage.clear();
  localStorage.setItem("popup-scope", "top");
  globalThis.__aboutBlankPopupStorageMessage = null;
  addEventListener("message", event => {{
    globalThis.__aboutBlankPopupStorageMessage = String(event.data);
  }});
  const frame = document.createElement("iframe");
  frame.src = {child_url_literal};
  (document.body || document.documentElement || document).appendChild(frame);
  return "queued";
}})()
"#
    ))
    .expect("about:blank popup partition setup should evaluate");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__aboutBlankPopupStorageMessage !== null)",
        "true",
        "about:blank popup partition result",
    )
    .await;

    assert_eq!(
        vm.eval("globalThis.__aboutBlankPopupStorageMessage || 'missing'")
            .expect("about:blank popup message should evaluate"),
        r#"{"popupBefore":null,"popupAfter":"popup-first-party","childAfter":"child-partition","opener":true}"#
    );
    assert_eq!(
        vm.eval("localStorage.getItem('popup-scope')")
            .expect("top localStorage should evaluate"),
        "top"
    );

    let mut child_first_party_vm =
        new_storage_test_vm_with_loader(&format!("{child_origin}/first-party.html"), &loader);
    assert_eq!(
        child_first_party_vm
            .eval("localStorage.getItem('popup-scope')")
            .expect("child first-party localStorage should evaluate"),
        "null"
    );

    let requests = server.await.expect("popup child server should finish");
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].starts_with("GET /popup-child.html "),
        "unexpected popup child request: {:?}",
        requests[0]
    );
}

#[tokio::test]
async fn document_domain_exact_self_assignment_keeps_storage_event_delivery() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://www.example.com/document-domain-page",
        &loader,
    );

    let initial = vm
        .eval(
            r#"
(() => {
  localStorage.clear();
  globalThis.__documentDomainStorageEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "document-domain-storage-child";
  (document.body || document.documentElement || document).appendChild(frame);
  return frame.contentWindow.location.href;
})()
"#,
        )
        .expect("document.domain storage event iframe setup should evaluate");
    assert_eq!(initial, "about:blank");
    drain_pending_page_child_frame_work_for_test(&mut vm).await;

    let setup = vm
        .eval(
            r#"
(() => {
  const frame = document.getElementById("document-domain-storage-child");
  frame.contentWindow.addEventListener("storage", event => {
    __documentDomainStorageEvents.push({
      key: event.key,
      oldValue: event.oldValue,
      newValue: event.newValue,
      storageArea: event.storageArea === frame.contentWindow.localStorage,
      childDomain: frame.contentDocument.domain
    });
  });
  let childDomain;
  try {
    frame.contentDocument.domain = document.domain;
    childDomain = frame.contentDocument.domain;
  } catch (error) {
    childDomain = `${error.name}:${error.code}`;
  }
  localStorage.setItem("test", "test");
  return JSON.stringify({
    parentDomain: document.domain,
    childDomain,
    eventCount: __documentDomainStorageEvents.length
  });
})()
"#,
        )
        .expect("document.domain storage event mutation should evaluate");
    assert_eq!(
        setup,
        r#"{"parentDomain":"www.example.com","childDomain":"www.example.com","eventCount":0}"#,
        "document.domain exact-self assignment must succeed before the queued event fires"
    );

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::StorageEvent,
            &loader,
        )
        .await
        .expect("selected dispatcher should advance document.domain storage event")
    );
    assert_eq!(
        vm.eval("JSON.stringify(__documentDomainStorageEvents)")
            .expect("document.domain storage event result should evaluate"),
        r#"[{"key":"test","oldValue":null,"newValue":"test","storageArea":true,"childDomain":"www.example.com"}]"#
    );
}

#[tokio::test]
async fn child_window_post_message_dispatches_a_trusted_message_event() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://window-message-trust.test/",
        &loader,
    );

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__windowMessageTrust = [];
  const frame = document.createElement("iframe");
  addEventListener("message", event => {
    __windowMessageTrust.push({
      data: event.data,
      trusted: event.isTrusted,
      sourceIsChild: event.source !== null && event.source === frame.contentWindow
    });
  });
  dispatchEvent(new MessageEvent("message", { data: "synthetic" }));
  frame.srcdoc = `<script>parent.postMessage("child", "*");<\/script>`;
  (document.body || document.documentElement).appendChild(frame);
  return String(__windowMessageTrust.length);
})()
"#,
        )
        .expect("window message trust setup should evaluate");
    assert_eq!(setup, "1", "synthetic dispatch should remain synchronous");

    for _ in 0..128 {
        if vm
            .eval("String(__windowMessageTrust.length)")
            .expect("window message trust length should evaluate")
            == "2"
        {
            break;
        }
        if !vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("child postMessage setup should advance")
        {
            break;
        }
    }

    assert_eq!(
        vm.eval("JSON.stringify(__windowMessageTrust)")
            .expect("window message trust result should evaluate"),
        r#"[{"data":"synthetic","trusted":false,"sourceIsChild":false},{"data":"child","trusted":true,"sourceIsChild":true}]"#
    );
}

#[tokio::test]
async fn window_post_message_validates_and_normalizes_target_origin() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://message-target.test/path",
        &loader,
    );

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__messageEvents = [];
  onmessage = event => {
    __messageEvents.push({
      data: event.data,
      origin: event.origin,
      ports: event.ports.length
    });
  };
  const probe = callback => {
    try {
      callback();
      return "no-throw";
    } catch (error) {
      return `${error.name}:${error.code}`;
    }
  };
  const badHost = probe(() => postMessage("", "http://foo bar", []));
  const relative = probe(() => postMessage("", "example.org", []));
  postMessage(["ok"], location.protocol + "//" + location.host + "/", []);
  postMessage({fromOptions: true}, {targetOrigin: location.origin});
  return `${badHost}|${relative}|${__messageEvents.length}`;
})()
"#,
        )
        .expect("window postMessage targetOrigin setup should evaluate");

    assert_eq!(result, "SyntaxError:12|SyntaxError:12|0");
    for _ in 0..6 {
        if vm
            .eval("__messageEvents.length")
            .expect("queued window message count should evaluate")
            == "2"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should drain queued window message");
    }
    assert_eq!(
        vm.eval("JSON.stringify(__messageEvents)")
            .expect("queued window message should evaluate"),
        r#"[{"data":["ok"],"origin":"https://message-target.test","ports":0},{"data":{"fromOptions":true},"origin":"https://message-target.test","ports":0}]"#
    );
}

#[tokio::test]
async fn window_post_message_allows_promise_continuations_between_events() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://message-order.test/path",
        &loader,
    );

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__messageOrder = [];
  let waitingFor = "initial";
  addEventListener("message", event => {
    if (!waitingFor) {
      __messageOrder.push("unexpected:" + event.data);
      return;
    }
    __messageOrder.push("message:" + event.data + ":" + waitingFor);
    waitingFor = "";
    Promise.resolve().then(() => {
      __messageOrder.push("microtask-after:" + event.data);
      if (event.data !== "third") {
        waitingFor = "after-" + event.data;
      }
    });
  });
  postMessage("first", "*");
  postMessage("second", "*");
  postMessage("third", "*");
  return String(__messageOrder.length);
})()
"#,
        )
        .expect("window postMessage ordering setup should evaluate");

    assert_eq!(result, "0");
    for _ in 0..10 {
        let order = vm
            .eval("JSON.stringify(__messageOrder)")
            .expect("window postMessage order should evaluate while waiting");
        if order
            == r#"["message:first:initial","microtask-after:first","message:second:after-first","microtask-after:second","message:third:after-second","microtask-after:third"]"#
        {
            break;
        }
        let _ = vm
            .run_one_window_message_executor_turn(&loader)
            .await
            .expect("wait driver should drain ordered window messages");
    }
    assert_eq!(
        vm.eval("JSON.stringify(__messageOrder)")
            .expect("window postMessage order should evaluate"),
        r#"["message:first:initial","microtask-after:first","message:second:after-first","microtask-after:second","message:third:after-second","microtask-after:third"]"#
    );
}

#[tokio::test]
async fn window_post_message_uses_an_independent_one_message_per_turn_task_source() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://message-task-source.test/path",
        &loader,
    );

    assert_eq!(
        vm.eval(
            r#"
globalThis.__postedMessageTaskEvents = [];
addEventListener("message", event => {
  __postedMessageTaskEvents.push("message:" + event.data);
  Promise.resolve().then(() => {
    __postedMessageTaskEvents.push("microtask:" + event.data);
  });
});
postMessage("first", "*");
postMessage("second", "*");
JSON.stringify(__postedMessageTaskEvents)
"#,
        )
        .expect("posted-message task-source setup should evaluate"),
        "[]"
    );
    assert!(vm.has_ready_window_message_task());
    assert!(
        !vm.has_ready_timeout(),
        "postMessage must not create a synthetic timer task"
    );

    assert!(
        vm.run_one_window_message_executor_turn(&loader)
            .await
            .expect("first posted-message task should run")
    );
    assert_eq!(
        vm.eval("JSON.stringify(__postedMessageTaskEvents)")
            .expect("first posted-message checkpoint result should evaluate"),
        r#"["message:first","microtask:first"]"#
    );
    assert!(
        vm.has_ready_window_message_task(),
        "the remaining message must publish exactly one continuation"
    );
    assert!(!vm.has_ready_timeout());

    assert!(
        vm.run_one_window_message_executor_turn(&loader)
            .await
            .expect("second posted-message task should run")
    );
    assert_eq!(
        vm.eval("JSON.stringify(__postedMessageTaskEvents)")
            .expect("second posted-message result should evaluate"),
        r#"["message:first","microtask:first","message:second","microtask:second"]"#
    );
    assert!(!vm.has_ready_window_message_task());
    assert!(!vm.has_ready_timeout());
}

#[tokio::test]
async fn window_post_message_array_second_argument_uses_options_defaults_without_transfer() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://message-array-options.test/",
        &loader,
    );

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__messageEvents = [];
  const channel = new MessageChannel();
  onmessage = event => {
    __messageEvents.push({
      data: event.data,
      origin: event.origin,
      ports: event.ports.length
    });
  };
  try {
    postMessage({fromRawArray: true}, [channel.port2]);
  } catch (error) {
    return `${error.name}:${error.code}`;
  }
  return String(__messageEvents.length);
})()
"#,
        )
        .expect("window postMessage raw-array options setup should evaluate");

    assert_eq!(result, "0");
    let _ = vm
        .run_one_oldest_ready_page_task_executor_turn(&loader)
        .await
        .expect("wait driver should drain raw-array window message");
    assert_eq!(
        vm.eval("JSON.stringify(__messageEvents)")
            .expect("raw-array window message should evaluate"),
        r#"[{"data":{"fromRawArray":true},"origin":"https://message-array-options.test","ports":0}]"#
    );
}

#[test]
fn lightweight_blank_popup_document_domain_uses_its_inherited_origin() {
    let mut vm = new_storage_test_vm("https://sub.example.test/creator");
    assert_eq!(
        vm.eval(
            r#"(()=>{
        const popup=open('about:blank','child');const doc=popup.document;
        const initial=doc.domain;doc.domain='sub.example.test';
        return JSON.stringify([initial,doc.domain]);
    })()"#
        )
        .unwrap(),
        r#"["sub.example.test","sub.example.test"]"#
    );
}

#[test]
fn opaque_top_document_domain_cannot_use_the_document_urls_host() {
    let mut vm = new_storage_test_vm("data:text/html,opaque");
    assert_eq!(
        vm.eval(
            r#"(()=>{
        const initial=document.domain;let result;
        try{document.domain='example.test';result='accepted'}catch(e){result=e.name}
        return JSON.stringify([initial,result]);
    })()"#
        )
        .unwrap(),
        r#"["","SecurityError"]"#
    );
}

#[test]
fn document_domain_setter_records_explicit_parent_domain() {
    let mut vm = new_storage_test_vm("https://www.example.com/path");

    let result = vm
        .eval(
            r#"
(() => {
  const initial = document.domain;
  document.domain = "example.com";
  const relaxed = document.domain;
  let invalid;
  try {
    document.domain = "other.example";
    invalid = "no-throw";
  } catch (error) {
    invalid = `${error.name}:${error instanceof DOMException}:${error.code}`;
  }
  return `${initial}|${relaxed}|${invalid}`;
})()
"#,
        )
        .expect("document.domain setter probe should evaluate");

    assert_eq!(result, "www.example.com|example.com|SecurityError:true:18");
}

#[test]
fn csp_sandbox_disallows_top_document_domain_setter() {
    let mut vm = new_storage_test_vm("https://www.example.com/path");
    vm.set_response_content_security_policies(&["sandbox allow-scripts".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  try {
    document.domain = document.domain;
    return "no-throw";
  } catch (error) {
    return `${error.name}:${error instanceof DOMException}:${error.code}`;
  }
})()
"#,
        )
        .expect("CSP sandbox document.domain probe should evaluate");

    assert_eq!(result, "SecurityError:true:18");
}

#[test]
fn csp_sandbox_allow_same_origin_disallows_top_document_domain_setter() {
    let mut vm = new_storage_test_vm("https://www.example.com/path");
    vm.set_response_content_security_policies(&[
        "sandbox allow-scripts allow-same-origin".to_owned()
    ]);

    let result = vm
        .eval(
            r#"
(() => {
  try {
    const initial = document.domain;
    document.domain = document.domain;
    return `${initial}|${document.domain}`;
  } catch (error) {
    return `${error.name}:${error instanceof DOMException}:${error.code}`;
  }
})()
"#,
        )
        .expect("CSP allow-same-origin document.domain probe should evaluate");

    assert_eq!(result, "SecurityError:true:18");
}

#[tokio::test]
async fn child_response_csp_sandbox_disallows_document_domain_setter() {
    let (child_url, server) = spawn_child_response_csp_sandbox_document_domain_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let child_url_parsed = Url::parse(&child_url).expect("child url");
    let document_url = format!(
        "http://127.0.0.1:{}/parent.html",
        child_url_parsed
            .port()
            .expect("child url should carry a port")
    );
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document_url, &loader);
    let child_url_literal = serde_json::to_string(&child_url).expect("serialize child url");

    let setup = vm
        .eval(&format!(
            r#"
(() => {{
  const frame = document.createElement("iframe");
  frame.src = {child_url_literal};
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__responseCspSandboxFrame = frame;
  return "queued";
}})()
"#
        ))
        .expect("child response CSP sandbox setup should evaluate");
    assert_eq!(setup, "queued");
    vm.drain_ready_child_frame_task_executor_turns_for_setup(&loader, 256)
        .await
        .expect("child response CSP setup should use the selected-task dispatcher");

    wait_for_one_page_resource_completion_selected_task_executor_test_turn(
        &mut vm,
        &loader,
        "child response CSP sandbox completion",
    )
    .await;
    vm.drain_ready_child_frame_task_executor_turns_for_setup(&loader, 256)
        .await
        .expect("child response CSP lifecycle should use the selected-task dispatcher");
    let requests = server
        .await
        .expect("child response CSP sandbox server should finish");
    assert_eq!(requests.len(), 1);

    let result = vm
        .eval(
            r#"
(() => {
  const frame = globalThis.__responseCspSandboxFrame;
  const ChildDOMException = frame.contentWindow.DOMException;
  const document = frame.contentDocument;
  try {
    const initial = document.domain;
    document.domain = document.domain;
    return `${initial}|${document.domain}`;
  } catch (error) {
    return `${error.name}:${error instanceof DOMException}:${error instanceof ChildDOMException}:${error.code}`;
  }
})()
"#,
        )
        .expect("child response CSP sandbox document.domain probe should evaluate");

    assert_eq!(result, "SecurityError:false:true:18");
}

#[test]
fn iframe_sandbox_allow_same_origin_disallows_document_domain_after_document_open() {
    let mut vm = new_storage_test_vm("https://www.example.com/path");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement("iframe");
  frame.setAttribute("sandbox", "allow-scripts allow-same-origin");
  (document.body || document.documentElement || document).appendChild(frame);
  frame.contentDocument.open();
  frame.contentDocument.write("<!doctype html><title>child</title>");
  frame.contentDocument.close();
  const ChildDOMException = frame.contentWindow.DOMException;
  try {
    const initial = frame.contentDocument.domain;
    frame.contentDocument.domain = frame.contentDocument.domain;
    return `${initial}|${frame.contentDocument.domain}`;
  } catch (error) {
    return `${error.name}:${error instanceof DOMException}:${error instanceof ChildDOMException}:${error.code}`;
  }
})()
"#,
        )
        .expect("sandbox allow-same-origin document.domain probe should evaluate");

    assert_eq!(result, "SecurityError:false:true:18");
}

#[tokio::test(flavor = "current_thread")]
async fn csp_sandbox_top_origin_blocks_tuple_documents_and_preserves_allow_same_origin() {
    for opaque in [false, true] {
        let server = StaticHttpServer::spawn(1).await;
        let base = server.base_url();
        let source = base.join("source.html").unwrap();
        let destination = base.join("common/blank.html").unwrap();
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(source.as_str(), &loader);
        vm.set_main_navigation_policy_container(crate::document_runtime::DocumentPolicyContainer {
            response_content_security_policies: vec![if opaque {
                "sandbox allow-scripts".into()
            } else {
                "sandbox allow-scripts allow-same-origin".into()
            }],
            ..Default::default()
        });
        vm.eval(r#"
            if (!document.documentElement) document.appendChild(document.createElement('html'));
            if (!document.body) document.documentElement.appendChild(document.createElement('body'));
            globalThis.sandboxFrame=document.createElement('iframe');
            globalThis.initialSandboxFrameReady=false;
            sandboxFrame.onload=()=>initialSandboxFrameReady=true;
            document.body.appendChild(sandboxFrame);
        "#).unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "initialSandboxFrameReady",
            "true",
            "initial sandbox document load",
        )
        .await;
        assert_eq!(
            vm.eval("sandboxFrame.contentDocument !== null").unwrap(),
            if opaque { "false" } else { "true" },
            "creation sandbox flags determine the initial document origin"
        );
        vm.eval(&format!(
            r#"
            globalThis.sandboxTupleResult='pending';
            sandboxFrame.onload=()=>{{
                let access;
                try {{ access=typeof sandboxFrame.contentWindow.fetch; }}
                catch(error) {{ access=error.name; }}
                sandboxTupleResult=String(sandboxFrame.contentDocument===null)+'|'+access;
            }};
            sandboxFrame.src={url:?};
        "#,
            url = destination.as_str()
        ))
        .unwrap();
        let expected = if opaque {
            "true|SecurityError"
        } else {
            "false|function"
        };
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "sandboxTupleResult",
            expected,
            "sandboxed top document access after child commit",
        )
        .await;
        {
            let host = vm._context_host.borrow();
            assert_eq!(
                host.main_default_world_security_token_key().is_none(),
                opaque
            );
            assert_eq!(
                host.main_isolated_world_security_token_key().is_none(),
                opaque
            );
        }
        assert_eq!(server.finish_targets().await, vec!["/common/blank.html"]);
    }
}
