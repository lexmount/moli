use super::{ChildFrameSemanticTurnKind, new_storage_page_task_executor_test_vm};
use crate::network::ResourceRequestClient;

#[tokio::test(flavor = "current_thread")]
async fn window_scroll_coalesces_into_one_rendering_update_without_a_timer() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm("https://scroll-rendering-update.test/");

    vm.eval(
        r#"
globalThis.__scrollLog = [];
document.addEventListener("scroll", () => __scrollLog.push("scroll:" + scrollY));
document.addEventListener("scrollend", () => __scrollLog.push("scrollend:" + scrollY));
scrollTo(0, 10);
scrollTo(0, 20);
scrollTo(0, 20);
"queued"
"#,
    )
    .expect("scroll producers should run");

    assert!(
        !vm.has_ready_timeout(),
        "Document scroll events must not manufacture a PageTimer descriptor"
    );
    assert_eq!(
        vm.eval("__scrollLog.join('|')")
            .expect("pre-turn log should be readable"),
        "",
        "scroll events must remain deferred until the rendering turn"
    );
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .expect("rendering update should run"),
        "coalesced scrolls should retain one production source task"
    );
    assert_eq!(
        vm.eval("__scrollLog.join('|')")
            .expect("post-turn log should be readable"),
        "scroll:20|scrollend:20"
    );
    assert!(
        !vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .expect("empty rendering source should remain usable"),
        "three synchronous scrolls of one Document must coalesce to one update"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn animation_and_scroll_share_rendering_fifo_but_consume_one_task_per_turn() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm("https://animation-scroll-rendering-fifo.test/");

    vm.eval(
        r#"
const root = document.documentElement || document.appendChild(document.createElement("html"));
const head = document.head || root.appendChild(document.createElement("head"));
const body = document.body || root.appendChild(document.createElement("body"));
head.appendChild(document.createElement("style")).textContent = `
  @keyframes pulse { from { left: 0px; } to { left: 10px; } }
  #animated { position: relative; animation: pulse 1s linear; }
`;
body.innerHTML = `<div id="animated"></div>`;
globalThis.__renderingLog = [];
document.getElementById("animated").addEventListener(
  "animationstart",
  () => __renderingLog.push("animation")
);
document.addEventListener("scroll", () => __renderingLog.push("scroll"));
document.addEventListener("scrollend", () => __renderingLog.push("scrollend"));
scrollTo(0, 12);
"queued"
"#,
    )
    .expect("animation and scroll producers should run");

    assert!(
        !vm.has_ready_timeout(),
        "neither rendering operation may manufacture a PageTimer"
    );
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .expect("animation rendering task should run first")
    );
    assert_eq!(
        vm.eval("__renderingLog.join('|')")
            .expect("first-turn rendering log should be readable"),
        "animation",
        "one selected rendering task must not drain the following scroll task"
    );
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .expect("scroll rendering task should run second")
    );
    assert_eq!(
        vm.eval("__renderingLog.join('|')")
            .expect("second-turn rendering log should be readable"),
        "animation|scroll|scrollend"
    );
    assert!(
        !vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .expect("drained rendering source should remain usable")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn scroll_handler_reentrancy_queues_a_new_turn_and_cleans_up_before_scrollend() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm("https://scroll-reentrant-update.test/");

    vm.eval(
        r#"
globalThis.__scrollLog = [];
globalThis.__didReenterScroll = false;
document.addEventListener("scroll", () => {
  __scrollLog.push("scroll:" + scrollY);
  Promise.resolve().then(() => __scrollLog.push("microtask:" + scrollY));
  if (!__didReenterScroll) {
    __didReenterScroll = true;
    scrollTo(0, 20);
  }
});
document.addEventListener("scrollend", () => __scrollLog.push("scrollend:" + scrollY));
scrollTo(0, 10);
"queued"
"#,
    )
    .expect("reentrant scroll fixture should initialize");

    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .expect("first rendering update should run")
    );
    assert_eq!(
        vm.eval("__scrollLog.join('|')")
            .expect("first-turn log should be readable"),
        "scroll:10|microtask:20|scrollend:20",
        "scroll callback cleanup precedes the next pending scrollend event"
    );

    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .expect("reentrant rendering update should run"),
        "scrolling from a handler must become a distinct subsequent turn"
    );
    assert_eq!(
        vm.eval("__scrollLog.join('|')")
            .expect("second-turn log should be readable"),
        "scroll:10|microtask:20|scrollend:20|scroll:20|microtask:20|scrollend:20"
    );
    assert!(!vm.has_ready_timeout());
}

#[tokio::test(flavor = "current_thread")]
async fn throwing_scroll_handler_does_not_abort_scrollend_or_the_task_checkpoint() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm("https://scroll-listener-error.test/");

    vm.eval(
        r#"
globalThis.__scrollErrorLog = [];
document.addEventListener("scroll", () => {
  __scrollErrorLog.push("scroll");
  Promise.resolve().then(() => __scrollErrorLog.push("microtask"));
  throw new Error("expected scroll listener failure");
});
document.addEventListener("scrollend", () => __scrollErrorLog.push("scrollend"));
scrollTo(0, 10);
"queued"
"#,
    )
    .expect("throwing scroll-listener fixture should initialize");

    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .expect("listener failure must not abort the rendering turn")
    );
    assert_eq!(
        vm.eval("__scrollErrorLog.join('|')")
            .expect("listener-error log should remain readable"),
        "scroll|microtask|scrollend",
        "public listener errors must not suppress callback cleanup or later pending events"
    );
    assert!(!vm.has_ready_timeout());
}

#[tokio::test(flavor = "current_thread")]
async fn scroll_handler_document_replacement_does_not_retarget_pending_scrollend() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm("https://scroll-handler-replacement.test/");

    vm.eval(
        r#"
globalThis.__scrollReplacementLog = [];
document.addEventListener("scroll", () => {
  __scrollReplacementLog.push("retired-scroll");
  document.open();
  document.write(`<!doctype html><script>
    document.addEventListener("scrollend", () => {
      globalThis.__scrollReplacementLog.push("replacement-scrollend");
    });
  <\/script>`);
  document.close();
});
document.addEventListener("scrollend", () => {
  __scrollReplacementLog.push("retired-scrollend");
});
scrollTo(0, 10);
"queued"
"#,
    )
    .expect("scroll replacement fixture should initialize");

    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .expect("rendering update should dispatch the retired Document scroll")
    );
    assert_eq!(
        vm.eval("__scrollReplacementLog.join('|')")
            .expect("replacement scroll log should remain readable"),
        "retired-scroll",
        "the old pending scrollend entry must not target either retired or replacement Document"
    );
    assert!(!vm.has_ready_timeout());
}

#[tokio::test(flavor = "current_thread")]
async fn unchanged_window_scroll_position_queues_neither_rendering_work_nor_timer() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm("https://scroll-no-change.test/");

    vm.eval(
        r#"
globalThis.__scrollEvents = 0;
document.addEventListener("scroll", () => __scrollEvents++);
document.addEventListener("scrollend", () => __scrollEvents++);
scrollTo(0, 0);
"unchanged"
"#,
    )
    .expect("unchanged scroll should evaluate");

    assert!(!vm.has_ready_timeout());
    assert!(
        !vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .expect("empty rendering source should remain usable")
    );
    assert_eq!(
        vm.eval("String(__scrollEvents)")
            .expect("event count should be readable"),
        "0"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn child_document_scroll_dispatches_in_its_exact_default_context() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm("https://child-scroll-rendering-update.test/");

    vm.eval(
        r#"
globalThis.__topScrollEvents = 0;
document.addEventListener("scroll", () => __topScrollEvents++);
document.addEventListener("scrollend", () => __topScrollEvents++);
const root = document.documentElement || document.appendChild(document.createElement("html"));
const body = document.body || root.appendChild(document.createElement("body"));
const frame = document.createElement("iframe");
body.appendChild(frame);
void frame.contentWindow;
"child-created"
"#,
    )
    .expect("initial child Document should be created");
    for turn in [
        ChildFrameSemanticTurnKind::NavigationCommit,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        ChildFrameSemanticTurnKind::HostLoad,
    ] {
        assert!(
            !vm.run_one_child_frame_task_executor_turn(turn, &loader)
                .await
                .expect("initial about:blank child task probe should succeed"),
            "the synchronous initial about:blank child must not leave {turn:?} work"
        );
    }
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::RealmMaterialization,
            &loader,
        )
        .await
        .expect("child realm materialization turn should succeed"),
        "child Window exposure should retain one production realm-materialization task"
    );
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child scroll default context should be materialized");

    vm.eval_in_child_default_context(
        child_context_id,
        r#"
globalThis.__childScrollLog = [];
document.addEventListener("scroll", () => __childScrollLog.push("scroll:" + scrollY));
document.addEventListener("scrollend", () => __childScrollLog.push("scrollend:" + scrollY));
scrollTo(0, 15);
"queued-child"
"#,
    )
    .expect("child scroll should enter the rendering source");

    assert!(!vm.has_ready_timeout());
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .expect("child rendering update should run")
    );
    assert_eq!(
        vm.eval_in_child_default_context(child_context_id, "__childScrollLog.join('|')")
            .expect("child scroll log should remain readable"),
        "scroll:15|scrollend:15"
    );
    assert_eq!(
        vm.eval("String(__topScrollEvents)")
            .expect("top Document scroll count should remain readable"),
        "0",
        "a child rendering update must not retarget its events to the top Document"
    );
}
