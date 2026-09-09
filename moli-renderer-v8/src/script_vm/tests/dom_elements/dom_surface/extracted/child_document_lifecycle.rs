use super::*;

#[tokio::test]
async fn element_matches_delegates_loaded_child_document_elements() {
    let mut vm = new_storage_test_vm("https://matches-loaded-child.test/#parent");
    vm.eval(
        r##"
(() => {
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <body>
      <div id="universal"><address id="address"><code id="code"></code></address></div>
      <p id="nth"><em id="em1"></em><strong></strong><em id="em2"></em><strong></strong><em id="em3"></em></p>
      <fieldset disabled><input id="disabledInput"></fieldset>
      <div id="target"></div>
    </body>
  `;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"##,
    )
    .expect("loaded child matches setup should evaluate");
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "loaded child matches document")
        .await;

    let result = vm
        .eval(
            r##"
(() => {
  const doc = document.querySelector("iframe").contentDocument;
  doc.defaultView.history.replaceState(null, "", "about:srcdoc#target");
  const code = doc.getElementById("code");
  return [
    code.ownerDocument === doc,
    doc.querySelectorAll("*").length,
    doc.querySelector("#code") === code,
    code.matches("code"),
    code.matches("#code"),
    code.matches("address code"),
    code.matches("*"),
    code.matches("#universal > * > *"),
    doc.getElementById("em3").matches("#nth em:nth-of-type(3)"),
    doc.getElementById("disabledInput").matches(":disabled"),
    doc.getElementById("target").matches("#target"),
    document.querySelector("iframe").contentDocument.URL.endsWith("#target"),
    document.querySelector(":target") === null,
    doc.querySelector(":target") === doc.getElementById("target"),
    doc.getElementById("target").matches(":target")
  ].join("|");
})()
"##,
        )
        .expect("loaded child matches probe should evaluate");

    assert_eq!(
        result,
        "true|15|true|true|true|true|true|true|true|true|true|true|true|true|true"
    );
}
#[tokio::test]
async fn loaded_child_document_top_level_children_remain_removable_from_parent_realm() {
    let mut vm = new_storage_test_vm("https://child-document-top-level-remove.test/");
    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  frame.id = "restore-frame";
  frame.srcdoc = `<!doctype html><html><head></head><body><p>child</p></body></html>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child top-level removal setup should evaluate");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "loaded child top-level removal document",
    )
    .await;

    let frame_handle = vm
        .document_runtime
        .dom_host()
        .element_handle_by_id("restore-frame")
        .expect("iframe should have a native handle");
    let child_document = vm
        ._context_host
        .borrow()
        .child_browsing_context_document_handle(frame_handle)
        .expect("iframe should have a child document handle");
    let child_handles = vm
        .document_runtime
        .dom_host()
        .child_handles(child_document)
        .collect::<Vec<_>>();
    assert!(
        child_handles.iter().all(|child| {
            vm.document_runtime
                .dom_host()
                .node(*child)
                .and_then(Node::parent_node)
                == Some(child_document)
        }),
        "native child document top-level children should belong to the child document"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.getElementById("restore-frame");
  const referenceDoc = document.implementation.createHTMLDocument("reference");
  referenceDoc.removeChild(referenceDoc.documentElement);
  referenceDoc.appendChild(frame.contentDocument.documentElement.cloneNode(true));

  const doc = frame.contentDocument;
  const before = [
    doc === frame.contentWindow.document,
    doc.firstChild.parentNode === doc,
    doc.lastChild.parentNode === doc,
    doc.documentElement && doc.lastChild.isSameNode(doc.documentElement)
  ];

  while (frame.contentDocument.firstChild &&
         frame.contentDocument.firstChild.nodeType != Node.DOCUMENT_TYPE_NODE) {
    frame.contentDocument.removeChild(frame.contentDocument.firstChild);
  }
  while (frame.contentDocument.lastChild &&
         frame.contentDocument.lastChild.nodeType != Node.DOCUMENT_TYPE_NODE) {
    frame.contentDocument.removeChild(frame.contentDocument.lastChild);
  }
  if (!frame.contentDocument.firstChild) {
    frame.contentDocument.appendChild(
      frame.contentDocument.implementation.createDocumentType("html", "", "")
    );
  }
  const appended = frame.contentDocument.appendChild(
    referenceDoc.documentElement.cloneNode(true)
  );

  return JSON.stringify({
    before,
    first: doc.firstChild && doc.firstChild.nodeName,
    last: doc.lastChild && doc.lastChild.nodeName,
    appendedParent: appended.parentNode === doc,
    documentElementIsAppended: doc.documentElement === appended,
    bodyText: doc.body && doc.body.textContent
  });
})()
"#,
        )
        .expect("child document WPT-style restore should evaluate");

    assert_eq!(
        result,
        r#"{"before":[true,true,true,true],"first":"html","last":"HTML","appendedParent":true,"documentElementIsAppended":true,"bodyText":"child"}"#
    );
}
#[tokio::test]
async fn iframe_javascript_url_string_completion_replaces_child_document() {
    let mut vm = new_storage_test_vm("https://iframe-javascript-url.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__javascriptUrlStringCompletionLoadCount = 0;
  const frame = document.createElement('iframe');
  frame.src = "javascript:document.contentType";
  frame.onload = () => {
    globalThis.__javascriptUrlStringCompletionLoadCount++;
  };
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("iframe javascript URL setup should evaluate");
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "HostLoad must not commit the pending javascript URL navigation"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "NavigationCommit should enqueue javascript URL execution"
    );
    let before_execution_loads = vm
        .eval("String(globalThis.__javascriptUrlStringCompletionLoadCount)")
        .expect("javascript URL pre-execution load count should evaluate");
    assert_eq!(before_execution_loads, "0");
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "javascript URL should execute on DocumentScriptReady",
    )
    .await;
    for transition in ["DOMContentLoaded", "complete"] {
        expect_child_frame_task_source_after_realm_prerequisite(
            &mut vm,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            &format!("javascript URL replacement should run {transition} before HostLoad"),
        )
        .await;
    }
    assert_eq!(
        vm.eval("typeof document.querySelector('iframe').onload")
            .expect("replacement load handler retention should evaluate"),
        "function",
        "navigation must retain the handler registered on the parent-owned frame element"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "javascript URL string-completion replacement should load on HostLoad"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.querySelector('iframe');
  const doc = frame.contentDocument;
    return [
    doc !== null,
    doc && doc.contentType,
    doc && doc.documentElement.textContent,
    doc && doc.URL.startsWith('javascript:'),
    globalThis.__javascriptUrlStringCompletionLoadCount
  ].join('|');
})()
"#,
        )
        .expect("iframe javascript URL document should evaluate");

    assert_eq!(result, "true|text/html|text/html|false|1");
}
#[tokio::test]
async fn iframe_javascript_url_non_string_completion_does_not_replace_child_document() {
    let mut vm = new_storage_test_vm("https://iframe-javascript-url-non-string.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__javascriptUrlNonStringLoadCount = 0;
  const frame = document.createElement('iframe');
  frame.src = "javascript:42";
  frame.onload = () => {
    globalThis.__javascriptUrlNonStringLoadCount++;
  };
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("iframe javascript URL non-string setup should evaluate");
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "HostLoad must not commit the pending javascript URL navigation"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "NavigationCommit should enqueue javascript URL execution"
    );
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "javascript URL should execute on DocumentScriptReady",
    )
    .await;
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "non-string javascript URL completion should expose the already-complete initial document load"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.querySelector('iframe');
  const doc = frame.contentDocument;
  return [
    doc !== null,
    doc && doc.contentType,
    doc && doc.documentElement.textContent,
    doc && doc.URL.startsWith('javascript:'),
    globalThis.__javascriptUrlNonStringLoadCount
  ].join('|');
})()
"#,
        )
        .expect("iframe javascript URL non-string document should evaluate");

    assert_eq!(result, "true|text/html||false|1");
}
#[tokio::test]
async fn iframe_javascript_url_exception_does_not_replace_child_document_or_load() {
    let mut vm = new_storage_test_vm("https://iframe-javascript-url-exception.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__javascriptUrlExceptionLoadCount = 0;
  const frame = document.createElement('iframe');
  frame.src = "javascript:throw new Error('boom')";
  frame.onload = () => {
    globalThis.__javascriptUrlExceptionLoadCount++;
  };
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("iframe javascript URL exception setup should evaluate");
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "HostLoad must not commit the pending javascript URL navigation"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "NavigationCommit should enqueue javascript URL execution"
    );
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "javascript URL exception should execute on DocumentScriptReady after realm materialization",
    )
    .await;
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "failed javascript URL execution should not synthesize a load"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.querySelector('iframe');
  const doc = frame.contentDocument;
  return [
    doc !== null,
    doc && doc.contentType,
    doc && doc.documentElement.textContent,
    doc && doc.URL.startsWith('javascript:'),
    globalThis.__javascriptUrlExceptionLoadCount
  ].join('|');
})()
"#,
        )
        .expect("iframe javascript URL exception document should evaluate");

    assert_eq!(result, "true|text/html||false|0");
}
#[tokio::test]
async fn child_frame_load_dispatches_document_readiness_before_pageshow() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-lifecycle.test/page.html", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childLifecycleEvents = [];
  globalThis.childStarted = () => {
    const frame = document.getElementById("child");
    frame.contentDocument.addEventListener("DOMContentLoaded", () => {
      __childLifecycleEvents.push("domcontentloaded");
    });
    frame.contentDocument.onreadystatechange = () => {
      __childLifecycleEvents.push(`readystate${frame.contentDocument.readyState}`);
    };
    frame.onload = () => __childLifecycleEvents.push("frameload");
    frame.contentWindow.onpageshow = event => {
      __childLifecycleEvents.push(
        `pageshow:${event.persisted === false}:${'persisted' in event}`
      );
    };
  };
  const frame = document.createElement("iframe");
  frame.id = "child";
  frame.srcdoc = "<head><script>top.childStarted()</" + "script></head><body>child</body>";
  (document.body || document.documentElement || document).appendChild(frame);
  return "ready";
})()
"#,
    )
    .expect("frame setup");
    vm.drain_pending_child_frame_work_for_test();

    let events = vm
        .eval("globalThis.__childLifecycleEvents.join('|')")
        .expect("child lifecycle events should evaluate");

    assert_eq!(
        events,
        "readystateinteractive|domcontentloaded|readystatecomplete|frameload|pageshow:true:true"
    );
}
#[tokio::test]
async fn child_document_open_during_pageshow_does_not_redispatch_pageshow() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader(
        "https://child-document-open-pageshow.test/page.html",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__childDocumentOpenPageshowEvents = [];
  globalThis.__replaceChildDuringPageshow = () => {
    __childDocumentOpenPageshowEvents.push("old-pageshow");
    const frame = document.getElementById("child");
    frame.contentDocument.open();
    frame.contentDocument.write(`
      <script>
        onpageshow = () => parent.__childDocumentOpenPageshowEvents.push("new-pageshow");
      <\/script>
      <p>replacement</p>`);
    frame.contentDocument.close();
  };
  const frame = document.createElement("iframe");
  frame.id = "child";
  frame.srcdoc = `
    <script>
      onpageshow = () => parent.__replaceChildDuringPageshow();
    <\/script>
    <p>original</p>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child document.open during pageshow setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    assert_eq!(
        vm.eval("__childDocumentOpenPageshowEvents.join('|')")
            .expect("child document.open pageshow events should evaluate"),
        "old-pageshow"
    );
}
#[tokio::test]
async fn child_pageshow_replacement_stops_old_frame_finish_and_protocol_output() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-pageshow-phases.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childPageshowPhaseEvents = [];
  globalThis.__childPageshowPhaseReplacement = `
    <script>
      addEventListener("load", () => parent.__childPageshowPhaseEvents.push("new-window"));
      addEventListener("pageshow", () => parent.__childPageshowPhaseEvents.push("new-pageshow"));
    <\/script>
    <body data-version="new">new</body>`;
  const frame = document.createElement("iframe");
  frame.id = "pageshow-phase-frame";
  frame.onload = () => {
    __childPageshowPhaseEvents.push(`frame:${frame.contentDocument.body.dataset.version}`);
  };
  frame.srcdoc = `
    <script>
      addEventListener("load", () => {
        window.__oldWindowLoadInvoked = true;
        parent.__childPageshowPhaseEvents.push("old-window");
      });
      addEventListener("pageshow", () => {
        window.__oldWindowPageshowInvoked = true;
        parent.__childPageshowPhaseEvents.push("old-pageshow");
        parent.document.getElementById("pageshow-phase-frame").srcdoc =
          parent.__childPageshowPhaseReplacement;
      });
    <\/script>
    <body data-version="old">old</body>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child pageshow-phase replacement setup should evaluate");

    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "old child pageshow-phase srcdoc should commit before parser work",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "old pageshow replacement handler should install",
    )
    .await;
    let child_handle = vm
        .document_runtime
        .dom_host()
        .element_handle_by_id("pageshow-phase-frame")
        .expect("pageshow-phase iframe should remain in the main document");
    {
        let context_host = vm._context_host.borrow();
        let load_callback_identities =
            context_host.child_window_event_callback_identities_for_test(child_handle, "load");
        assert_eq!(
            load_callback_identities.len(),
            1,
            "old Window load listener must be registered against the reused LocalWindow"
        );
        let pageshow_callback_identities =
            context_host.child_window_event_callback_identities_for_test(child_handle, "pageshow");
        assert_eq!(
            pageshow_callback_identities.len(),
            1,
            "old Window pageshow listener must be registered against the reused LocalWindow"
        );
        assert!(
            load_callback_identities
                .iter()
                .chain(&pageshow_callback_identities)
                .filter_map(|(relevant, _)| *relevant)
                .all(|identity| context_host
                    .window_execution_context_identity_is_current(identity)),
            "old Window lifecycle listeners must retain current callback relevant realms"
        );
    }
    for context in ["old DOMContentLoaded", "old complete"] {
        expect_child_frame_task_source_after_realm_prerequisite(
            &mut vm,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            context,
        )
        .await;
    }
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::HostLoad,
        "old pageshow should run before frame finish",
    )
    .await;
    assert_eq!(
        vm.eval(
            "String(document.getElementById('pageshow-phase-frame').contentWindow.__oldWindowLoadInvoked === true && document.getElementById('pageshow-phase-frame').contentWindow.__oldWindowPageshowInvoked === true)"
        )
        .expect("old child Window lifecycle invocation markers should evaluate"),
        "true",
        "HostLoad must invoke old Window load and pageshow before replacement"
    );
    assert_eq!(
        vm.eval("__childPageshowPhaseEvents.join('|')")
            .expect("old pageshow-phase trace should evaluate"),
        "old-window|frame:old|old-pageshow"
    );
    assert_eq!(
        vm.completed_child_frame_navigation_load_count(),
        0,
        "replacement in pageshow must suppress old frame/protocol completion"
    );

    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "pageshow replacement should commit on its navigation source",
    )
    .await;
    expect_one_child_frame_task_source(
        &mut vm,
        ChildFrameSemanticTurnKind::RealmMaterialization,
        "pageshow replacement realm materialization must survive stale context retirement",
    )
    .await;
    expect_one_child_frame_task_source(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "replacement pageshow handlers should install",
    )
    .await;
    for context in ["replacement DOMContentLoaded", "replacement complete"] {
        expect_child_frame_task_source_after_realm_prerequisite(
            &mut vm,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            context,
        )
        .await;
    }
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::HostLoad,
        "replacement pageshow delivery should finish independently",
    )
    .await;

    assert_eq!(
        vm.eval("__childPageshowPhaseEvents.join('|')")
            .expect("replacement pageshow-phase trace should evaluate"),
        "old-window|frame:old|old-pageshow|new-window|frame:new|new-pageshow"
    );
    assert_eq!(vm.completed_child_frame_navigation_load_count(), 1);
}
#[tokio::test]
async fn child_domcontentloaded_uses_one_document_to_window_event_path() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-dcl-path.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childDclPath = [];
  globalThis.__parentDclLeak = 0;
  addEventListener("DOMContentLoaded", () => {
    globalThis.__parentDclLeak += 1;
  });
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <script>
      let firstDclEvent;
      addEventListener("DOMContentLoaded", event => {
        firstDclEvent = event;
        const path = event.composedPath();
        parent.__childDclPath.push([
          "window-capture",
          event.target === document,
          event.currentTarget === window,
          event.eventPhase,
          path[0] === document,
          path[1] === window
        ].join(":"));
      }, true);
      document.addEventListener("DOMContentLoaded", event => {
        parent.__childDclPath.push([
          "document",
          event === firstDclEvent,
          event.target === document,
          event.currentTarget === document,
          event.eventPhase
        ].join(":"));
      });
      addEventListener("DOMContentLoaded", event => {
        parent.__childDclPath.push([
          "window-bubble",
          event === firstDclEvent,
          event.target === document,
          event.currentTarget === window,
          event.eventPhase
        ].join(":"));
      });
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child DOMContentLoaded event-path setup should evaluate");

    vm.drain_pending_child_frame_work_for_test();

    assert_eq!(
        vm.eval("globalThis.__childDclPath.join('|')")
            .expect("child DOMContentLoaded event path should evaluate"),
        "window-capture:true:true:1:true:true|document:true:true:true:2|window-bubble:true:true:true:3"
    );
    assert_eq!(
        vm.eval("String(globalThis.__parentDclLeak)")
            .expect("parent DOMContentLoaded leak count should evaluate"),
        "0",
        "a child Document event path must not reuse the parent Window target"
    );
}
#[tokio::test]
async fn child_domcontentloaded_document_stop_propagation_blocks_window_bubble() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-dcl-stop.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childDclStop = [];
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <script>
      addEventListener("DOMContentLoaded", () => {
        parent.__childDclStop.push("window-capture");
      }, true);
      document.addEventListener("DOMContentLoaded", event => {
        parent.__childDclStop.push("document");
        event.stopPropagation();
      });
      addEventListener("DOMContentLoaded", () => {
        parent.__childDclStop.push("window-bubble");
      });
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child DOMContentLoaded stop-propagation setup should evaluate");

    vm.drain_pending_child_frame_work_for_test();

    assert_eq!(
        vm.eval("globalThis.__childDclStop.join('|')")
            .expect("child DOMContentLoaded stop-propagation result should evaluate"),
        "window-capture|document"
    );
}
#[tokio::test]
async fn child_static_media_delays_complete_and_iframe_load_until_loadeddata() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://child-media-delay.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__childMediaDelayEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "media-frame";
  frame.onload = () => __childMediaDelayEvents.push("frame-load");
  frame.srcdoc = `
    <video id="clip" src="data:video/webm;base64,AA=="></video>
    <script>
      document.addEventListener("DOMContentLoaded", () => {
        parent.__childMediaDelayEvents.push("dcl");
      });
      const clip = document.getElementById("clip");
      for (const type of ["loadstart", "loadedmetadata", "loadeddata", "canplay"]) {
        clip.addEventListener(type, () => {
          parent.__childMediaDelayEvents.push(type);
        });
      }
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child static media setup should evaluate");

    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child static media srcdoc should commit before parser work",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child media parser script should run before lifecycle",
    )
    .await;
    assert_eq!(
        vm.eval("document.querySelector('iframe').contentDocument.readyState")
            .expect("child readiness"),
        "interactive",
        "parser EOF must apply interactive synchronously"
    );
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "child media document should dispatch DOMContentLoaded",
    )
    .await;
    assert!(
        !vm.run_one_child_frame_task_executor_turn(ChildFrameSemanticTurnKind::HostLoad, &loader,)
            .await
            .expect("blocked child HostLoad probe should succeed"),
        "the media token must keep HostLoad unavailable"
    );
    assert_eq!(
        vm.eval(
            "JSON.stringify({ events: __childMediaDelayEvents, readyState: document.getElementById('media-frame').contentDocument.readyState })"
        )
        .expect("blocked child media lifecycle should evaluate"),
        r#"{"events":["dcl"],"readyState":"interactive"}"#
    );

    run_next_page_media_element_event_for_test(&mut vm, &loader, "child media loadstart turn")
        .await;
    run_next_page_media_element_event_for_test(&mut vm, &loader, "child media loadedmetadata turn")
        .await;
    assert!(
        !vm.run_one_child_frame_task_executor_turn(ChildFrameSemanticTurnKind::HostLoad, &loader,)
            .await
            .expect("metadata child HostLoad probe should succeed"),
        "metadata must not release the media delay"
    );
    run_next_page_media_element_event_for_test(&mut vm, &loader, "child media loadeddata turn")
        .await;
    assert_eq!(
        vm.eval("globalThis.__childMediaDelayEvents.join('|')")
            .expect("child media loadeddata trace should evaluate"),
        "dcl|loadstart|loadedmetadata|loadeddata",
        "loadeddata must dispatch before it exposes complete work"
    );

    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "loadeddata should queue a later complete lifecycle turn",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::HostLoad,
        "completed media document should load on a still later HostLoad turn",
    )
    .await;
    assert_eq!(
        vm.eval(
            "JSON.stringify({ events: __childMediaDelayEvents, readyState: document.getElementById('media-frame').contentDocument.readyState })"
        )
        .expect("released child media lifecycle should evaluate"),
        r#"{"events":["dcl","loadstart","loadedmetadata","loadeddata","frame-load"],"readyState":"complete"}"#
    );
    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "non-blocking child media canplay turn",
    )
    .await;
}
#[tokio::test]
async fn child_media_network_failure_releases_lifecycle_before_later_host_load() {
    let (media_url, request_rx, release_tx, server) = spawn_gated_media_resource_server(404).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader
        .set_optional_resource_fetch_mask(crate::protocol_types::OptionalResourceFetchMask::AUDIO);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        &media_url.replace("/media", "/page"),
        &loader,
    );

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__childFailedMediaEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "failed-media-frame";
  frame.onload = () => __childFailedMediaEvents.push("frame-load");
  frame.srcdoc = `
    <audio id="failed-media" src={media_url:?}></audio>
    <script>
      const media = document.getElementById("failed-media");
      for (const type of ["loadstart", "loadedmetadata", "loadeddata", "canplay", "error"]) {{
        media.addEventListener(type, () => parent.__childFailedMediaEvents.push(type));
      }}
      document.addEventListener("DOMContentLoaded", () => {{
        parent.__childFailedMediaEvents.push("dcl");
      }});
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
}})()
"#
    ))
    .expect("child failed media setup should evaluate");
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child failed media srcdoc should commit before parser work",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child media parser script should install listeners",
    )
    .await;
    assert_eq!(
        vm.eval("document.querySelector('iframe').contentDocument.readyState")
            .expect("child readiness"),
        "interactive",
        "parser EOF must apply interactive synchronously"
    );
    let request = request_rx.await.expect("child media request should arrive");
    assert!(
        request
            .to_ascii_lowercase()
            .contains("sec-fetch-dest: audio")
    );
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "child failed media document should dispatch DOMContentLoaded",
    )
    .await;
    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "child failed media loadstart turn",
    )
    .await;
    assert!(
        !vm.run_one_child_frame_task_executor_turn(ChildFrameSemanticTurnKind::HostLoad, &loader,)
            .await
            .expect("pending-media child HostLoad probe should succeed"),
        "the pending media request must keep child HostLoad unavailable"
    );

    release_tx
        .send(())
        .expect("release child failed media response");
    wait_for_one_page_resource_completion_executor_test_turn(
        &mut vm,
        "child media network failure",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childFailedMediaEvents.join('|')")
            .expect("child media completion trace should evaluate"),
        "dcl|loadstart",
        "resource completion must not inline-dispatch media error or iframe load"
    );

    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "child failed media error owner turn",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childFailedMediaEvents.join('|')")
            .expect("child media error trace should evaluate"),
        "dcl|loadstart|error"
    );
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "media error should expose complete only on a later lifecycle turn",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::HostLoad,
        "completed failed-media document should dispatch iframe load later",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childFailedMediaEvents.join('|')")
            .expect("child media HostLoad trace should evaluate"),
        "dcl|loadstart|error|frame-load"
    );
    server.await.expect("child media server should finish");
}
#[tokio::test]
async fn child_image_network_failure_releases_lifecycle_before_later_host_load() {
    let (image_url, request_rx, release_tx, server) = spawn_gated_image_resource_server(404).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        &image_url.replace("/image.png", "/page"),
        &loader,
    );

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__childFailedImageEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "failed-image-frame";
  frame.onload = () => __childFailedImageEvents.push("frame-load");
  frame.srcdoc = `
    <img id="failed-image" src={image_url:?}>
    <script>
      const image = document.getElementById("failed-image");
      image.onload = () => parent.__childFailedImageEvents.push("load");
      image.onerror = () => parent.__childFailedImageEvents.push("error");
      document.addEventListener("DOMContentLoaded", () => {{
        parent.__childFailedImageEvents.push("dcl");
      }});
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
}})()
"#
    ))
    .expect("child failed image setup should evaluate");
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child failed image srcdoc should commit before parser work",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child image parser script should install listeners",
    )
    .await;
    assert_eq!(
        vm.eval("document.querySelector('iframe').contentDocument.readyState")
            .expect("child readiness"),
        "interactive",
        "parser EOF must apply interactive synchronously"
    );
    let request = request_rx.await.expect("child image request should arrive");
    assert!(
        request
            .to_ascii_lowercase()
            .contains("sec-fetch-dest: image")
    );
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "child image document should dispatch DOMContentLoaded",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childFailedImageEvents.join('|')")
            .expect("child image pre-terminal trace"),
        "dcl"
    );
    let child_image = (0..vm.document_runtime.dom_host().dom().nodes().len())
        .map(crate::document_runtime::DomHandle::new)
        .find(|handle| {
            vm.document_runtime
                .dom_host()
                .node(*handle)
                .and_then(crate::dom::native::Node::as_element)
                .is_some_and(|element| element.attribute("id") == Some("failed-image"))
        })
        .expect("child image handle");
    assert!(
        !vm.run_one_child_frame_task_executor_turn(ChildFrameSemanticTurnKind::HostLoad, &loader)
            .await
            .expect("pending-image child HostLoad probe should succeed"),
        "the pending image request must keep child HostLoad unavailable"
    );
    let child_pending_before_detach = vm
        ._context_host
        .borrow()
        .pending_image_load_event(child_image)
        .expect("connected child image sequence");
    vm.eval(
        "document.getElementById('failed-image-frame').contentDocument.getElementById('failed-image').remove()",
    )
    .expect("detaching the pending child image should evaluate");
    let detached_pending = vm
        ._context_host
        .borrow()
        .pending_image_load_event(child_image)
        .expect("detached child image sequence");
    assert_eq!(detached_pending.id(), child_pending_before_detach.id());
    assert_eq!(
        detached_pending.network_request_id(),
        child_pending_before_detach.network_request_id()
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_image_load_event_is_current(child_image, &detached_pending),
        "same-document detach must preserve current image ownership"
    );
    assert!(
        !vm.run_one_child_frame_task_executor_turn(ChildFrameSemanticTurnKind::HostLoad, &loader)
            .await
            .expect("detached-image child HostLoad probe should succeed"),
        "same-document image removal must preserve the child document delay until the event"
    );

    release_tx
        .send(())
        .expect("release child failed image response");
    wait_for_one_page_resource_completion_executor_test_turn(
        &mut vm,
        "child image network failure",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childFailedImageEvents.join('|')")
            .expect("child image completion trace"),
        "dcl",
        "resource completion must not inline-dispatch image error or iframe load"
    );

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ImageLoadEvent,
            &loader,
        )
        .await
        .expect("child image error selected task should run"),
        "child image failure should enqueue one DOM-manipulation turn"
    );
    assert!(
        !vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ImageLoadEvent,
            &loader,
        )
        .await
        .expect("child image error source should become idle"),
        "one failed image terminal must produce exactly one selected event task"
    );
    assert_eq!(
        vm.eval("globalThis.__childFailedImageEvents.join('|')")
            .expect("child image error trace"),
        "dcl|error"
    );
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "image error should expose complete only on a later lifecycle turn",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::HostLoad,
        "completed failed-image document should dispatch iframe load later",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childFailedImageEvents.join('|')")
            .expect("child image HostLoad trace"),
        "dcl|error|frame-load"
    );
    server.await.expect("child image server should finish");
}
#[tokio::test]
async fn child_dynamic_media_accepted_during_dcl_delays_later_load_turns() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://child-dynamic-media.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__childDynamicMediaEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "dynamic-media-frame";
  frame.onload = () => __childDynamicMediaEvents.push("frame-load");
  frame.srcdoc = `<body><script>
    document.addEventListener("DOMContentLoaded", () => {
      parent.__childDynamicMediaEvents.push("dcl");
      const clip = document.createElement("video");
      clip.addEventListener("loadeddata", () => {
        parent.__childDynamicMediaEvents.push("loadeddata");
      });
      document.body.appendChild(clip);
      clip.src = "data:video/webm;base64,AQ==";
    });
  <\/script></body>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child dynamic media setup should evaluate");

    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child dynamic media srcdoc should commit before parser work",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "dynamic media parser script should install its DCL producer",
    )
    .await;
    assert_eq!(
        vm.eval("document.querySelector('iframe').contentDocument.readyState")
            .expect("child readiness"),
        "interactive",
        "parser EOF must apply interactive synchronously"
    );
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "DCL should accept dynamic media before complete is prepared",
    )
    .await;
    assert!(
        !vm.run_one_child_frame_task_executor_turn(ChildFrameSemanticTurnKind::HostLoad, &loader,)
            .await
            .expect("dynamic-media child HostLoad probe should succeed"),
        "DCL-inserted media must block HostLoad"
    );

    run_next_page_media_element_event_for_test(&mut vm, &loader, "dynamic media loadstart turn")
        .await;
    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "dynamic media loadedmetadata turn",
    )
    .await;
    run_next_page_media_element_event_for_test(&mut vm, &loader, "dynamic media loadeddata turn")
        .await;
    assert_eq!(
        vm.eval("globalThis.__childDynamicMediaEvents.join('|')")
            .expect("dynamic media terminal trace should evaluate"),
        "dcl|loadeddata"
    );
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "dynamic media terminal should expose complete later",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::HostLoad,
        "dynamic media completion should expose iframe load later",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childDynamicMediaEvents.join('|')")
            .expect("dynamic media load trace should evaluate"),
        "dcl|loadeddata|frame-load"
    );
}
#[tokio::test]
async fn child_static_text_track_starts_at_interactive_without_own_load_token() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://child-static-track.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__childStaticTrackEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "track-frame";
  frame.onload = () => __childStaticTrackEvents.push("frame-load");
  frame.srcdoc = `
    <video><track id="captions" default src="data:text/vtt,WEBVTT%0A%0A00%3A00%3A00.000%20--%3E%2000%3A00%3A01.000%0Ahello"></video>
    <script>
      document.getElementById("captions").addEventListener("load", () => {
        parent.__childStaticTrackEvents.push("track:" + document.readyState);
      });
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child static text-track setup should evaluate");

    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child static text-track srcdoc should commit before parser work",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child track parser script should install its listener",
    )
    .await;
    assert_eq!(
        vm.eval("document.querySelector('iframe').contentDocument.readyState")
            .expect("child readiness"),
        "interactive",
        "parser EOF must apply interactive synchronously"
    );
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::TextTrackDefaultMode,
            &loader,
        )
        .await
        .expect("child default text-track mode owner turn"),
        "child default track should use the shared DOM-manipulation source"
    );
    let frame = vm
        .document_runtime
        .get_element_by_id("track-frame")
        .expect("text-track child frame handle");
    {
        let context_host = vm._context_host.borrow();
        let snapshot = context_host
            .frame_owner_current_child_snapshot(frame)
            .expect("text-track child owner snapshot");
        let track = context_host
            .dom_host()
            .element_handle_by_id_in_subtree(snapshot.document_handle, "captions")
            .expect("child text-track handle");
        let pending = context_host
            .pending_text_track_load_sequence(track)
            .expect("child text-track sequence");
        let expected_owner = crate::frame_owner_model::FrameDocumentTaskOwner::new(
            snapshot.scheduler_lane_id,
            snapshot.local_window_id,
            snapshot.document_id,
        );
        assert_eq!(
            pending.target(),
            crate::native_bridge::WindowDocumentTaskTarget::new(
                crate::native_bridge::WindowDocumentOwner::Frame(expected_owner),
                crate::native_bridge::OwnerDispatchScope::Child(frame),
            )
        );
    }
    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("child track load-start networking turn")
    );
    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("child track terminal networking turn")
    );
    assert_eq!(
        vm.eval("globalThis.__childStaticTrackEvents.join('|')")
            .expect("child track interactive trace should evaluate"),
        "track:interactive",
        "track scheduling must not wait for iframe/window load"
    );

    for (source, transition) in [
        (
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            "DOMContentLoaded",
        ),
        (ChildFrameSemanticTurnKind::DocumentLifecycle, "complete"),
        (ChildFrameSemanticTurnKind::HostLoad, "iframe load"),
    ] {
        expect_page_child_frame_task_source_after_realm_prerequisite(
            &mut vm,
            &loader,
            source,
            &format!("track document should later advance through {transition}"),
        )
        .await;
    }
    assert_eq!(
        vm.eval("globalThis.__childStaticTrackEvents.join('|')")
            .expect("child track load trace should evaluate"),
        "track:interactive|frame-load"
    );
}
#[tokio::test]
async fn child_document_replacement_retires_media_sequence_and_delay() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-media-replace.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childMediaReplacementEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "media-replacement-frame";
  frame.onload = () => {
    __childMediaReplacementEvents.push(
      "frame-load:" + frame.contentDocument.getElementById("document-marker").textContent
    );
  };
  frame.srcdoc = `
    <video id="old-media" src="data:video/webm;base64,AA=="></video>
    <script>
      document.getElementById("old-media").addEventListener("loadstart", () => {
        parent.__childMediaReplacementEvents.push("stale-media-loadstart");
      });
      document.addEventListener("DOMContentLoaded", () => {
        parent.__childMediaReplacementEvents.push("first-dcl");
      });
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child media replacement setup should evaluate");

    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "first child media srcdoc should commit before parser work",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "first media document parser script should run",
    )
    .await;
    assert_eq!(
        vm.eval("document.querySelector('iframe').contentDocument.readyState")
            .expect("child readiness"),
        "interactive",
        "parser EOF must apply interactive synchronously"
    );
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "first media document should dispatch DOMContentLoaded",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childMediaReplacementEvents.join('|')")
            .expect("first media document trace should evaluate"),
        "first-dcl"
    );

    vm.eval(
        r#"
(() => {
  const frame = document.getElementById("media-replacement-frame");
  frame.srcdoc = `<body><p id="document-marker">second</p><script>
    document.addEventListener("DOMContentLoaded", () => {
      parent.__childMediaReplacementEvents.push("second-dcl");
    });
  <\/script></body>`;
})()
"#,
    )
    .expect("replacement child document should queue");
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "replacement should retire the old media owner before commit",
    )
    .await;
    expect_one_child_frame_task_source(
        &mut vm,
        ChildFrameSemanticTurnKind::RealmMaterialization,
        "media replacement realm materialization must survive stale context retirement",
    )
    .await;
    expect_one_child_frame_task_source(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "replacement parser script should run",
    )
    .await;
    for (source, transition) in [
        (
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            "DOMContentLoaded",
        ),
        (ChildFrameSemanticTurnKind::DocumentLifecycle, "complete"),
        (ChildFrameSemanticTurnKind::HostLoad, "iframe load"),
    ] {
        expect_child_frame_task_source_after_realm_prerequisite(
            &mut vm,
            source,
            &format!("replacement should advance through {transition}"),
        )
        .await;
    }

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("retired child media callbacks should remain harmless");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        None,
        "retired media callbacks must not create replacement lifecycle work"
    );
    assert_eq!(
        vm.eval("globalThis.__childMediaReplacementEvents.join('|')")
            .expect("replacement child media result should evaluate"),
        "first-dcl|second-dcl|frame-load:second"
    );
}
#[tokio::test]
async fn moving_pending_child_media_restarts_under_the_new_document_owner() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://child-media-move.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__movedChildMediaEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "moving-media-frame";
  frame.onload = () => __movedChildMediaEvents.push("frame-load");
  frame.srcdoc = `
    <video id="moving-media" src="data:video/webm;base64,AA=="></video>
    <script>
      document.addEventListener("DOMContentLoaded", () => {
        parent.__movedChildMediaEvents.push("child-dcl");
      });
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("moving child media setup should evaluate");
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "moving child media srcdoc should commit before parser work",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "moving media parser script should run",
    )
    .await;
    assert_eq!(
        vm.eval("document.querySelector('iframe').contentDocument.readyState")
            .expect("child readiness"),
        "interactive",
        "parser EOF must apply interactive synchronously"
    );
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "moving media document should dispatch DOMContentLoaded",
    )
    .await;
    let frame = vm
        .document_runtime
        .get_element_by_id("moving-media-frame")
        .expect("moving media frame handle");
    let media = {
        let context_host = vm._context_host.borrow();
        let child_document = context_host
            .child_browsing_context_document_handle(frame)
            .expect("moving media child document");
        context_host
            .dom_host()
            .element_handle_by_id_in_subtree(child_document, "moving-media")
            .expect("moving media handle")
    };
    let child_sequence = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("child-owned media sequence");
    assert!(matches!(
        child_sequence.owner(),
        crate::native_bridge::PendingMediaLoadOwner::Child { .. }
    ));

    vm.eval(
        r#"
(() => {
  const frame = document.getElementById("moving-media-frame");
  const media = document.adoptNode(
    frame.contentDocument.getElementById("moving-media")
  );
  media.addEventListener("loadeddata", () => {
    __movedChildMediaEvents.push("moved-loadeddata");
  });
  (document.body || document.documentElement || document).appendChild(media);
})()
"#,
    )
    .expect("pending child media should move to the parent document");
    let main_sequence = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("moved media should restart under the parent owner");
    assert_ne!(child_sequence.id(), main_sequence.id());
    assert!(matches!(
        main_sequence.owner(),
        crate::native_bridge::PendingMediaLoadOwner::Main { .. }
    ));

    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "moving media away should release child complete later",
    )
    .await;
    expect_page_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::HostLoad,
        "the child iframe should load after its media owner moves away",
    )
    .await;
    assert!(
        !vm.has_ready_timeout(),
        "moving a pending media element must not create a synthetic Page timer"
    );
    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "stale child media loadstart turn",
    )
    .await;
    for phase in ["loadstart", "loadedmetadata", "loadeddata", "canplay"] {
        run_next_page_media_element_event_for_test(
            &mut vm,
            &loader,
            &format!("moved main media {phase} turn"),
        )
        .await;
    }
    assert_eq!(
        vm.eval("globalThis.__movedChildMediaEvents.join('|')")
            .expect("moved child media trace should evaluate"),
        "child-dcl|frame-load|moved-loadeddata",
        "the stale child callback must not consume the restarted main sequence"
    );
}
#[tokio::test]
async fn child_image_event_delay_blocks_complete_and_host_load_until_terminal() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-image-delay.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childImageDelayEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "image-frame";
  frame.onload = () => __childImageDelayEvents.push("frame-load");
  frame.srcdoc = `
    <img id="hero" src="image.png">
    <script>
      document.addEventListener("DOMContentLoaded", () => {
        parent.__childImageDelayEvents.push("dcl");
      });
      document.getElementById("hero").addEventListener("load", event => {
        parent.__childImageDelayEvents.push(
          "image-load:" + (event instanceof Event) + ":" + (event instanceof parent.Event)
        );
      });
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child image load-delay setup should evaluate");

    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child image-delay srcdoc should commit before parser work",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child image parser script should run before lifecycle",
    )
    .await;
    assert_eq!(
        vm.eval("document.querySelector('iframe').contentDocument.readyState")
            .expect("child readiness"),
        "interactive",
        "parser EOF must apply interactive synchronously"
    );
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "child image document should dispatch DOMContentLoaded",
    )
    .await;
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "the pending image token must prevent HostLoad delivery"
    );
    assert_eq!(
        vm.eval(
            "JSON.stringify({ events: __childImageDelayEvents, readyState: document.getElementById('image-frame').contentDocument.readyState })"
        )
        .expect("blocked child image lifecycle should evaluate"),
        r#"{"events":["dcl"],"readyState":"interactive"}"#,
        "image acceptance must allow DCL but block complete and iframe load"
    );

    assert_eq!(
        drain_image_load_event_bodies_for_test(&mut vm),
        1,
        "child image terminal should enqueue one DOM-manipulation turn"
    );
    assert_eq!(
        vm.eval("globalThis.__childImageDelayEvents.join('|')")
            .expect("child image terminal should evaluate"),
        "dcl|image-load:true:false",
        "the image event task must only enqueue the later lifecycle follow-up"
    );

    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "image terminal should queue a later complete lifecycle turn",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::HostLoad,
        "completed image document should load on a still later HostLoad turn",
    )
    .await;
    assert_eq!(
        vm.eval(
            "JSON.stringify({ events: __childImageDelayEvents, readyState: document.getElementById('image-frame').contentDocument.readyState })"
        )
        .expect("released child image lifecycle should evaluate"),
        r#"{"events":["dcl","image-load:true:false","frame-load"],"readyState":"complete"}"#
    );
}
#[tokio::test]
async fn child_document_replacement_cancels_stale_image_event_and_delay() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-image-replace.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childImageReplacementEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "replacement-frame";
  frame.onload = () => {
    __childImageReplacementEvents.push(
      "frame-load:" + frame.contentDocument.getElementById("document-marker").textContent
    );
  };
  frame.srcdoc = `
    <img id="old-image" src="old.png">
    <script>
      document.getElementById("old-image").addEventListener("load", () => {
        parent.__childImageReplacementEvents.push("stale-image-load");
      });
      document.addEventListener("DOMContentLoaded", () => {
        parent.__childImageReplacementEvents.push("first-dcl");
      });
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child image replacement setup should evaluate");

    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "first child image srcdoc should commit before parser work",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "first image document parser script should run",
    )
    .await;
    assert_eq!(
        vm.eval("document.querySelector('iframe').contentDocument.readyState")
            .expect("child readiness"),
        "interactive",
        "parser EOF must apply interactive synchronously"
    );
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "first image document should dispatch DOMContentLoaded",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childImageReplacementEvents.join('|')")
            .expect("first child image document should evaluate"),
        "first-dcl"
    );

    vm.eval(
        r#"
(() => {
  const frame = document.getElementById("replacement-frame");
  frame.srcdoc = `<body><p id="document-marker">second</p><script>
    document.addEventListener("DOMContentLoaded", () => {
      parent.__childImageReplacementEvents.push("second-dcl");
    });
  <\/script></body>`;
})()
"#,
    )
    .expect("replacement child document should queue");
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "replacement should commit on its navigation turn",
    )
    .await;
    expect_one_child_frame_task_source(
        &mut vm,
        ChildFrameSemanticTurnKind::RealmMaterialization,
        "image replacement realm materialization must survive stale context retirement",
    )
    .await;
    expect_one_child_frame_task_source(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "replacement parser script should run",
    )
    .await;
    assert!(
        vm.apply_next_image_load_event_body_for_test()
            .expect("stale image DOM task"),
        "the earlier image task must retire at the shared DOM FIFO head"
    );
    assert_eq!(
        vm.eval("__childImageReplacementEvents.join('|')")
            .expect("retired image trace"),
        "first-dcl",
        "the stale image task must not dispatch into the replacement"
    );

    for (source, transition) in [
        (
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            "DOMContentLoaded",
        ),
        (ChildFrameSemanticTurnKind::DocumentLifecycle, "complete"),
        (ChildFrameSemanticTurnKind::HostLoad, "iframe load"),
    ] {
        expect_child_frame_task_source_after_realm_prerequisite(
            &mut vm,
            source,
            &format!("replacement should advance through {transition}"),
        )
        .await;
    }

    let _ = drain_image_load_event_bodies_for_test(&mut vm);
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        None,
        "the canceled image task must not create replacement lifecycle work"
    );
    assert_eq!(
        vm.eval("globalThis.__childImageReplacementEvents.join('|')")
            .expect("replacement child image result should evaluate"),
        "first-dcl|second-dcl|frame-load:second",
        "replacement must retire the old image task/event/token without blocking the new load"
    );
}
#[tokio::test]
async fn moving_pending_child_image_rebinds_event_without_consuming_new_request() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-image-move.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__movedChildImageEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "moving-image-frame";
  frame.onload = () => __movedChildImageEvents.push("frame-load");
  frame.srcdoc = `
    <img id="moving-image" src="moving.png">
    <script>
      document.addEventListener("DOMContentLoaded", () => {
        parent.__movedChildImageEvents.push("child-dcl");
      });
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("moving child image setup should evaluate");
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "moving child image srcdoc should commit before parser work",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "moving image parser script should run",
    )
    .await;
    assert_eq!(
        vm.eval("document.querySelector('iframe').contentDocument.readyState")
            .expect("child readiness"),
        "interactive",
        "parser EOF must apply interactive synchronously"
    );
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "moving image document should dispatch DOMContentLoaded",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__movedChildImageEvents.join('|')")
            .expect("blocked moving child image state should evaluate"),
        "child-dcl"
    );

    vm.eval(
        r#"
(() => {
  const frame = document.getElementById("moving-image-frame");
  const image = document.adoptNode(
    frame.contentDocument.getElementById("moving-image")
  );
  image.addEventListener("load", () => {
    __movedChildImageEvents.push("moved-image-load");
  });
  (document.body || document.documentElement || document).appendChild(image);
})()
"#,
    )
    .expect("pending child image should move to the parent document");
    assert!(
        vm.apply_next_image_load_event_body_for_test()
            .expect("old image DOM task"),
        "the old image task must retire before the later complete task at the shared FIFO head"
    );
    assert_eq!(
        vm.eval("__movedChildImageEvents.join('|')")
            .expect("old image trace"),
        "child-dcl",
        "retiring the old image task must leave the rebound request pending"
    );
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "moving the image should release complete on a later lifecycle turn",
    )
    .await;
    assert!(
        !vm.has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad),
        "the rebound image event must keep its earlier position in the shared DOM FIFO"
    );
    assert!(
        vm.apply_next_image_load_event_body_for_test()
            .expect("rebound image DOM task"),
        "the new image request must deliver before the later HostLoad task"
    );
    assert_eq!(
        vm.eval("__movedChildImageEvents.join('|')")
            .expect("rebound image trace"),
        "child-dcl|moved-image-load",
        "the rebound image request must deliver once without running the later child load"
    );
    assert!(
        vm.has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad),
        "the child load must now be at the DOM FIFO head"
    );
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::HostLoad,
        "the former image owner should load on a later HostLoad turn",
    )
    .await;
    assert_eq!(
        drain_image_load_event_bodies_for_test(&mut vm),
        0,
        "the old and rebound image tasks must both have been consumed exactly once"
    );
    assert_eq!(
        vm.eval("globalThis.__movedChildImageEvents.join('|')")
            .expect("moved image event result should evaluate"),
        "child-dcl|moved-image-load|frame-load",
        "the stale child task must not consume or duplicate the rebound image request"
    );
}
#[test]
fn window_pageshow_uses_original_page_transition_event() {
    let mut vm = new_storage_test_vm("https://window-pageshow-event.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__pageshowShape = "missing";
  window.PageTransitionEvent = function PageTransitionEvent() {
    throw new Error("page replacement should not be invoked");
  };
  addEventListener("pageshow", event => {
    __pageshowShape = [
      event.type,
      event.persisted === false,
      'persisted' in event,
      event.bubbles,
      event.cancelable,
      event.isTrusted
    ].join(':');
  });
  return "ready";
})()
"#,
    )
    .expect("pageshow setup should evaluate");
    vm.dispatch_window_load_event()
        .expect("window load should dispatch");

    let shape = vm
        .eval("globalThis.__pageshowShape")
        .expect("pageshow shape should evaluate");
    assert_eq!(shape, "pageshow:true:true:true:true:true");
}
#[tokio::test]
async fn detached_frame_tree_snapshot_reuses_each_child_document_scripting_policy() {
    let mut enabled_vm = new_storage_test_vm("https://enabled-detached-frame-tree-noscript.test/");

    enabled_vm
        .eval(
            r#"
(() => {
  const root = document.body || document.documentElement || document;
  const frame = document.createElement("iframe");
  frame.id = "enabled";
  frame.srcdoc =
    '<noscript><base href="https://enabled-fallback-base.test/"><iframe id="enabled-fallback" name="enabled-fallback"></iframe></noscript>';
  root.appendChild(frame);
})()
"#,
        )
        .expect("scripting-enabled detached frame-tree fixture should evaluate");
    expect_one_child_frame_task_source(
        &mut enabled_vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "enabled srcdoc commit",
    )
    .await;
    enabled_vm
        .eval("document.getElementById('enabled').setAttribute('sandbox', 'allow-same-origin')")
        .expect("tightening the owner sandbox after commit should evaluate");

    let enabled_handle = enabled_vm
        .document_runtime
        .dom_host()
        .element_handle_by_id("enabled")
        .expect("scripting-enabled iframe owner");
    assert_ne!(
        enabled_vm
            ._context_host
            .borrow()
            .child_browsing_context_base_url(enabled_handle)
            .as_ref()
            .map(Url::as_str),
        Some("https://enabled-fallback-base.test/"),
        "scripting-enabled noscript contents must not install a fallback base element"
    );
    let enabled_tree = enabled_vm.child_browsing_context_frame_tree_snapshot_for_protocol();
    assert_eq!(enabled_tree.len(), 1);
    assert!(
        enabled_tree[0].child_frames.is_empty(),
        "scripting-enabled noscript contents must remain text in snapshot fallback parsing"
    );

    let mut sandboxed_vm =
        new_storage_test_vm("https://sandboxed-detached-frame-tree-noscript.test/");
    sandboxed_vm
        .eval(
            r#"
(() => {
  const root = document.body || document.documentElement || document;
  const frame = document.createElement("iframe");
  frame.id = "sandboxed";
  frame.setAttribute("sandbox", "allow-same-origin");
  frame.srcdoc =
    '<noscript><base href="https://sandboxed-fallback-base.test/"><iframe id="sandboxed-fallback" name="sandboxed-fallback"></iframe></noscript>';
  root.appendChild(frame);
})()
"#,
        )
        .expect("sandboxed detached frame-tree fixture should evaluate");
    expect_one_child_frame_task_source(
        &mut sandboxed_vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "sandboxed srcdoc commit",
    )
    .await;
    sandboxed_vm
        .eval("document.getElementById('sandboxed').removeAttribute('sandbox')")
        .expect("loosening the owner sandbox after commit should evaluate");

    let sandboxed_handle = sandboxed_vm
        .document_runtime
        .dom_host()
        .element_handle_by_id("sandboxed")
        .expect("sandboxed iframe owner");
    assert_eq!(
        sandboxed_vm
            ._context_host
            .borrow()
            .child_browsing_context_base_url(sandboxed_handle)
            .as_ref()
            .map(Url::as_str),
        Some("https://sandboxed-fallback-base.test/"),
        "script-disallowed noscript contents must contribute their base element"
    );
    let sandboxed_tree = sandboxed_vm.child_browsing_context_frame_tree_snapshot_for_protocol();
    assert_eq!(sandboxed_tree.len(), 1);
    assert_eq!(
        sandboxed_tree[0]
            .child_frames
            .iter()
            .map(|frame| frame.owner_element_id.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("sandboxed-fallback")],
        "script-disallowed noscript contents must remain markup in snapshot fallback parsing"
    );
    let nested_frame_id = sandboxed_tree[0].child_frames[0].frame_id.clone();
    assert!(
        sandboxed_vm
            .child_browsing_context_document_snapshot_by_frame_id(&nested_frame_id)
            .is_some(),
        "frame-id lookup must use the same scripting-disabled snapshot parser"
    );
}
#[tokio::test]
async fn child_document_open_in_dom_content_loaded_yields_to_timer() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_test_vm_with_loader("https://child-document-open-dcl-timer.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childOpenDclEvents = [];
  const frame = document.createElement('iframe');
  frame.srcdoc = `
    <script>
      addEventListener('DOMContentLoaded', function() {
        parent.__childOpenDclEvents.push('dcl');
        document.open();
        setTimeout(function() {
          parent.__childOpenDclEvents.push('timer');
        }, 0);
        document.close();
      });
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("child DOMContentLoaded document.open setup should evaluate");

    vm.drain_pending_child_frame_work_for_test();
    assert_eq!(
        vm.eval("globalThis.__childOpenDclEvents.join('|')")
            .expect("child DCL log should evaluate"),
        "dcl"
    );

    assert!(
        vm.run_next_due_timer_callback_for_test(&loader)
            .await
            .expect("timer turn should advance child DCL timer")
    );
    assert_eq!(
        vm.eval("globalThis.__childOpenDclEvents.join('|')")
            .expect("child DCL timer result should evaluate"),
        "dcl|timer"
    );
}
#[tokio::test]
async fn child_document_close_without_defer_queues_replacement_domcontentloaded() {
    let mut vm = new_storage_test_vm("https://child-document-close-dcl.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__childCloseLifecycleEvents = [];
  const frame = document.createElement('iframe');
  frame.srcdoc = `
    <script>
      document.addEventListener('DOMContentLoaded', function() {
        if (window.__replacementStarted) return;
        window.__replacementStarted = true;
        document.open();
        document.addEventListener('DOMContentLoaded', function() {
          parent.__childCloseLifecycleEvents.push('replacement-dcl');
        });
        document.write('<p>replacement</p>');
        document.close();
        parent.__childCloseLifecycleEvents.push('after-close');
      });
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("child document.close lifecycle setup should evaluate");

    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child srcdoc should install its exact document owner before parser work",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child parser script should install the original DCL handler",
    )
    .await;
    assert_eq!(
        vm.eval("document.querySelector('iframe').contentDocument.readyState")
            .expect("child readiness"),
        "interactive",
        "parser EOF must apply interactive synchronously"
    );
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "original child DCL should synchronously finish the replacement parser",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childCloseLifecycleEvents.join('|')")
            .expect("child document.close return trace should evaluate"),
        "after-close",
        "replacement DOMContentLoaded must not dispatch inline from document.close"
    );

    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "replacement DOMContentLoaded should remain a later lifecycle source turn",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childCloseLifecycleEvents.join('|')")
            .expect("replacement DCL trace should evaluate"),
        "after-close|replacement-dcl"
    );
}
#[test]
fn inherited_child_document_domain_mutation_updates_aliased_window_security_token() {
    let mut vm = new_storage_test_vm("https://www.example.com/page.html");

    vm.exec(
        r#"
const frame = document.createElement("iframe");
frame.srcdoc = "<!doctype html><body>same-origin child</body>";
(document.body || document.documentElement || document).appendChild(frame);
globalThis.__domainTokenFrame = frame;
"#,
        None,
    )
    .expect("document.domain token setup should run");
    vm.drain_pending_child_frame_work_for_test();
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("same-origin child realm should materialize");

    assert_eq!(
        vm.eval("__domainTokenFrame.contentWindow.document === __domainTokenFrame.contentDocument")
            .expect("initial child document access should evaluate"),
        "true"
    );
    assert_eq!(
        vm.eval_in_child_default_context(
            child_context_id,
            "document.domain = document.domain; document.domain",
        )
        .expect("child document.domain mutation should evaluate"),
        "www.example.com"
    );

    assert_eq!(
        vm.eval(
            "__domainTokenFrame.contentWindow.document === __domainTokenFrame.contentDocument",
        )
        .expect("aliased post-domain child access should evaluate"),
        "true"
    );
    assert_eq!(
        vm.eval("document.domain")
            .expect("aliased parent document.domain should evaluate"),
        "www.example.com"
    );
}
#[test]
fn blob_child_document_accepts_html_mime_parameters() {
    let mut vm = new_storage_test_vm("https://blob-child-html-mime.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const blob = new Blob(
    ['<!DOCTYPE html><body><span id="value">ok</span></body>'],
    { type: 'text/html;charset=utf-8' }
  );
  const frame = document.createElement('iframe');
  frame.src = URL.createObjectURL(blob);
  (document.body || document.documentElement || document).appendChild(frame);
  return frame.contentDocument !== null;
})()
"#,
        )
        .expect("blob child document should materialize");

    assert_eq!(result, "true");
}
