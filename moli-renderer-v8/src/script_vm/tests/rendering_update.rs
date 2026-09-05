use super::{ChildFrameSemanticTurnKind, new_storage_page_task_executor_test_vm};
use crate::network::ResourceRequestClient;

#[tokio::test(flavor = "current_thread")]
async fn deferred_layout_resources_coalesce_on_the_existing_rendering_source() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://layout-resource-turn.test/");
    for _ in 0..3 {
        assert!(
            vm._context_host
                .borrow_mut()
                .queue_layout_resource_admission(Vec::new())
        );
    }
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert!(
        !vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap(),
        "repeated paused capture must not manufacture extra rendering turns"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn deferred_layout_resources_do_not_retarget_after_document_open() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://layout-resource-retirement.test/");
    let before = vm.current_main_document_task_owner().unwrap();
    let root = vm
        ._context_host
        .borrow()
        .dom_host()
        .document_element_handle()
        .unwrap();
    assert!(
        vm._context_host
            .borrow_mut()
            .queue_layout_resource_admission(vec![moli_layout::LayoutCssImageReference {
            source: root,
            resolved_url:
                "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'/>"
                    .to_owned(),
        },])
    );
    vm.eval("document.open(); document.write('<!doctype html><title>replacement</title>'); document.close(); 'replaced'").unwrap();
    assert_ne!(before, vm.current_main_document_task_owner().unwrap());
    let resources = vm.css_image_resource_observability_for_test();
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(
        vm.css_image_resource_observability_for_test(),
        resources,
        "retired capture references must not admit resources into the replacement"
    );
    assert!(
        !vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap()
    );
}

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
async fn scroll_handler_reentrancy_queues_a_new_turn_and_checkpoints_after_scrollend() {
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
        "scroll:10|scrollend:20|microtask:20",
        "one rendering update dispatches its pending event list before the host-task checkpoint"
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
        "scroll:10|scrollend:20|microtask:20|scroll:20|scrollend:20|microtask:20"
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
        "scroll|scrollend|microtask",
        "public listener errors must not suppress later pending events or the host-task checkpoint"
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

fn emulation_viewport(width: u32) -> crate::protocol_types::ViewportSurface {
    crate::protocol_types::ViewportSurface {
        inner_width: width,
        inner_height: 600,
        outer_width: 1920,
        outer_height: 1080,
        device_pixel_ratio: 1.0,
        screen_width: 1920,
        screen_height: 1080,
        screen_avail_width: 1920,
        screen_avail_height: 1040,

        ..Default::default()
    }
}

#[tokio::test(flavor = "current_thread")]
async fn environment_updates_defer_and_coalesce_resize_and_media_notifications() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://environment-turn.test/");
    vm.set_viewport_surface_for_bootstrap(Some(emulation_viewport(640)));
    vm.eval(
        r#"
        globalThis.events = [];
        globalThis.widthQuery = matchMedia('(min-width: 800px)');
        globalThis.darkQuery = matchMedia('(prefers-color-scheme: dark)');
        const record = name => events.push([name, innerWidth, visualViewport.width,
            widthQuery.matches, darkQuery.matches].join(':'));
        addEventListener('resize', () => record('window'));
        visualViewport.addEventListener('resize', () => record('visual'));
        widthQuery.addEventListener('change', () => record('width'));
        darkQuery.addEventListener('change', () => record('dark'));
        'ready'
    "#,
    )
    .unwrap();
    vm.set_viewport_surface(Some(emulation_viewport(800)))
        .unwrap();
    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides {
        color_scheme: Some("dark".to_owned()),
        ..Default::default()
    });
    vm.set_viewport_surface(Some(emulation_viewport(900)))
        .unwrap();
    assert_eq!(vm.eval("events.join('|')").unwrap(), "");
    assert_eq!(
        vm.eval(
            "[innerWidth, visualViewport.width, widthQuery.matches, darkQuery.matches].join(':')"
        )
        .unwrap(),
        "900:900:true:true"
    );
    // A list created after the change already has the current match result.
    vm.eval("matchMedia('(min-width: 800px)').addEventListener('change', () => events.push('late')); 'ready'").unwrap();
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "window:900:900:true:true|visual:900:900:true:true|width:900:900:true:true|dark:900:900:true:true"
    );
    assert!(
        !vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap()
    );
    vm.set_viewport_surface(Some(emulation_viewport(900)))
        .unwrap();
    assert!(
        !vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn environment_screen_changes_report_media_without_resizing_the_viewport() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://screen-media-turn.test/");
    vm.set_viewport_surface_for_bootstrap(Some(emulation_viewport(800)));
    vm.eval(r#"
        globalThis.events = [];
        addEventListener('resize', () => events.push('window'));
        visualViewport.addEventListener('resize', () => events.push('visual'));
        matchMedia('(device-width: 1000px)').addEventListener('change', e => events.push('screen:' + e.matches));
        matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => events.push('dark'));
        'ready'
    "#).unwrap();
    let mut surface = emulation_viewport(800);
    surface.screen_width = 1000;
    surface.screen_avail_width = 1000;
    vm.set_viewport_surface(Some(surface)).unwrap();
    assert_eq!(vm.eval("events.join('|')").unwrap(), "");
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(vm.eval("events.join('|')").unwrap(), "screen:true");
    // Returning to the original preference before the turn produces no change.
    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides {
        color_scheme: Some("dark".to_owned()),
        ..Default::default()
    });
    vm.set_emulated_media(&Default::default());
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(vm.eval("events.join('|')").unwrap(), "screen:true");
    vm.set_viewport_surface(Some(emulation_viewport(800)))
        .unwrap();
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "screen:true|screen:false"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn environment_resize_replacement_retires_the_remaining_document_events() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://environment-replacement.test/");
    vm.set_viewport_surface_for_bootstrap(Some(emulation_viewport(640)));
    vm.eval(r#"
        globalThis.events = [];
        addEventListener('resize', () => {
            events.push('resize');
            document.open(); document.write('<!doctype html><p>replacement</p>'); document.close();
        });
        visualViewport.addEventListener('resize', () => events.push('retired-visual'));
        matchMedia('(min-width: 800px)').addEventListener('change', () => events.push('retired-media'));
        document.addEventListener('visibilitychange', () => events.push('retired-visibility'));
        'ready'
    "#).unwrap();
    vm.set_viewport_surface(Some(emulation_viewport(800)))
        .unwrap();
    vm.set_document_activity(moli_page_types::DocumentActivity::new(false, false))
        .unwrap();
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(vm.eval("events.join('|')").unwrap(), "resize");
}

#[tokio::test(flavor = "current_thread")]
async fn environment_media_and_visibility_changes_reach_existing_child_documents() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://child-environment.test/");
    vm.eval(r#"
        const root = document.documentElement || document.appendChild(document.createElement('html'));
        const body = document.body || root.appendChild(document.createElement('body'));
        const frame = document.createElement('iframe'); body.appendChild(frame); void frame.contentWindow;
        'ready'
    "#).unwrap();
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::RealmMaterialization,
            &loader
        )
        .await
        .unwrap()
    );
    let child_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .next()
        .unwrap()
        .context_id;
    vm.eval_in_child_default_context(child_id, r#"
        globalThis.events = [];
        matchMedia('(prefers-color-scheme: dark)').addEventListener('change', e => events.push('dark:' + e.matches));
        document.addEventListener('visibilitychange', () => events.push('hidden:' + document.hidden));
        'ready'
    "#).unwrap();
    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides {
        color_scheme: Some("dark".to_owned()),
        ..Default::default()
    });
    vm.set_document_activity(moli_page_types::DocumentActivity::new(false, false))
        .unwrap();
    assert_eq!(
        vm.eval_in_child_default_context(child_id, "events.join('|')")
            .unwrap(),
        ""
    );
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(
        vm.eval_in_child_default_context(child_id, "events.join('|')")
            .unwrap(),
        "dark:true|hidden:true"
    );
}
