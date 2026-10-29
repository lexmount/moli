use super::*;

use crate::page_task_queue::RendererOwnerWakeSource;

fn take_window_message_wake(
    wake_rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::page_task_queue::RendererOwnerWake>,
) -> bool {
    while let Ok(wake) = wake_rx.try_recv() {
        if wake.source_for_test() == RendererOwnerWakeSource::WindowMessageTask {
            return true;
        }
    }
    false
}

async fn window_message_source_lifecycle(action: &str) -> anyhow::Result<String> {
    let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
    let document_url = Url::parse("https://example.com/window-message-source-lifecycle")?;
    let (mut page_vm, _resource_source, _owner_wake_rx) =
        page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
    page_vm.vm_mut().eval(
        r#"
globalThis.sourceFrame = document.createElement('iframe');
sourceFrame.id = 'message-source';
document.body.appendChild(sourceFrame);
"#,
    )?;
    materialize_child_realm_through_page_turn_for_test(&mut page_vm, "message-source")?;
    page_vm
        .vm_mut()
        .eval(&format!("globalThis.sourceAction = {action:?}"))?;
    page_vm.vm_mut().eval(
        r#"
globalThis.oldSource = sourceFrame.contentWindow;
oldSource.onmessage = new oldSource.Function(`
  parent.postMessage('prepare', '*');
  parent.postMessage('payload', '*');
`);
globalThis.sourcePayload = null;
onmessage = event => {
  if (event.data === 'prepare') {
    globalThis.sourceWasOldBeforeMutation = event.source === oldSource;
    if (sourceAction === 'document-open') {
      oldSource.document.open();
      oldSource.document.write('<!doctype html><body>replacement Document');
      oldSource.document.close();
    } else {
      sourceFrame.remove();
      if (sourceAction === 'reinsert') document.body.appendChild(sourceFrame);
    }
    globalThis.sourceWindowChanged = sourceFrame.isConnected ? sourceFrame.contentWindow !== oldSource : null;
  } else if (event.data === 'payload') {
    sourcePayload = {
      data: event.data,
      sourceIsCurrent: sourceFrame.isConnected && event.source === sourceFrame.contentWindow,
      sourceIsOld: event.source === oldSource,
      sourceIsNull: event.source === null,
      originMatches: event.origin === location.origin,
      trusted: event.isTrusted
    };
  }
};
oldSource.postMessage('go', '*');
"#,
    )?;
    for label in ["child producer", "source mutation", "accepted payload"] {
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::WindowMessage,
                    &loader,
                )
                .await?,
            "{label} should run as one selected posted-message task"
        );
    }
    page_vm
        .vm_mut()
        .eval("JSON.stringify({sourceWasOldBeforeMutation, windowChanged:sourceWindowChanged, payload:sourcePayload})")
}

#[tokio::test(flavor = "current_thread")]
async fn window_message_dictionary_getter_exception_never_admits_a_payload_task() {
    run_page_vm_async_test(async {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let document_url = Url::parse("https://example.com/window-message-conversion")?;
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let setup = page_vm.vm_mut().eval(
            r#"
globalThis.conversionMessages = [];
globalThis.conversionClones = 0;
onmessage = event => conversionMessages.push(event.data);
const conversionSentinel = {};
let conversionException = false;
try {
  postMessage({get value() { conversionClones++; return 'rejected'; }}, {
    get targetOrigin() { throw conversionSentinel; }
  });
} catch (error) {
  conversionException = error === conversionSentinel;
}
postMessage('fence', '*');
String(conversionException && conversionClones === 0);
"#,
        )?;
        assert_eq!(setup, "true");
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::WindowMessage,
                    &loader,
                )
                .await?
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify(conversionMessages)")?,
            r#"["fence"]"#
        );
        assert!(
            !page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::WindowMessage,
                    &loader,
                )
                .await?
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("a throwing options getter must not create a posted-message task");
}

#[tokio::test(flavor = "current_thread")]
async fn retired_target_window_messages_prepare_without_admitting_a_task() {
    run_page_vm_async_test(async {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let document_url = Url::parse("https://example.com/retired-window-message-prepare")?;
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(r#"
globalThis.targetFrame = document.createElement('iframe');
targetFrame.id = 'retired-message-target'; document.body.appendChild(targetFrame);
"#)?;
        materialize_child_realm_through_page_turn_for_test(&mut page_vm, "retired-message-target")?;
        let result = page_vm.vm_mut().eval(r#"
const oldTarget = targetFrame.contentWindow;
const retainedPostMessage = oldTarget.postMessage;
targetFrame.remove();
const buffer = new ArrayBuffer(8), channel = new MessageChannel();
const log = [];
let error = null;
try {
  Reflect.apply(retainedPostMessage, oldTarget, [{get value() {log.push('payload'); return 7;}}, {
    get transfer() {log.push('transfer'); return [channel.port2, buffer];},
    get targetOrigin() {log.push('origin'); return 'not a URL';}
  }]);
} catch (caught) {error = caught.name;}
const marker = {};
let sameException = false;
try {Reflect.apply(retainedPostMessage, oldTarget, [0, {get transfer() {throw marker;}}]);}
catch (caught) {sameException = caught === marker;}
let portUsable = false;
try {const moved = structuredClone(channel.port2, {transfer:[channel.port2]}); moved.close(); portUsable = true;}
catch (_) {}
channel.port1.close(); channel.port2.close();
globalThis.preparedMessages = [];
onmessage = event => preparedMessages.push(event.data);
postMessage('fence', '*');
JSON.stringify({log, error, bytes:buffer.byteLength, portUsable, sameException});
"#)?;
        assert_eq!(result, r#"{"log":["transfer","origin","payload"],"error":null,"bytes":0,"portUsable":true,"sameException":true}"#);
        assert!(page_vm.run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::WindowMessage, &loader,
        ).await?);
        assert_eq!(page_vm.vm_mut().eval("JSON.stringify(preparedMessages)")?, r#"["fence"]"#);
        assert!(!page_vm.run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::WindowMessage, &loader,
        ).await?);
        Ok::<_, anyhow::Error>(())
    }).await.expect("a retired target must still prepare data without a message task");
}

#[tokio::test(flavor = "current_thread")]
async fn window_message_source_does_not_rebind_to_a_reinserted_iframe() {
    run_page_vm_async_test(async {
        let result = window_message_source_lifecycle("reinsert").await?;
        let result: serde_json::Value = serde_json::from_str(&result)?;
        assert_eq!(result["sourceWasOldBeforeMutation"], true);
        assert_eq!(result["windowChanged"], true);
        assert_eq!(result["payload"]["data"], "payload");
        assert_eq!(result["payload"]["originMatches"], true);
        assert_eq!(result["payload"]["trusted"], true);
        assert_eq!(result["payload"]["sourceIsCurrent"], false);
        assert_eq!(result["payload"]["sourceIsOld"], true);
        assert_eq!(result["payload"]["sourceIsNull"], false);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("queued messages must not impersonate a successor source Window");
}

#[tokio::test(flavor = "current_thread")]
async fn window_message_retains_a_removed_source_window_until_delivery() {
    run_page_vm_async_test(async {
        let result = window_message_source_lifecycle("remove").await?;
        assert_eq!(result, r#"{"sourceWasOldBeforeMutation":true,"windowChanged":null,"payload":{"data":"payload","sourceIsCurrent":false,"sourceIsOld":true,"sourceIsNull":false,"originMatches":true,"trusted":true}}"#);
        Ok::<_, anyhow::Error>(())
    }).await.expect("retiring the source must not erase the accepted WindowProxy");
}

#[tokio::test(flavor = "current_thread")]
async fn window_message_retains_its_incumbent_source_before_argument_side_effects() {
    run_page_vm_async_test(async {
        for stage in ["transfer", "targetOrigin", "iterator", "payload"] {
            for action in ["remove", "reinsert"] {
                let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
                let document_url = Url::parse("https://example.com/window-source-arguments")?;
                let (mut page_vm, _resource_source, _owner_wake_rx) =
                    page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
                page_vm.vm_mut().eval(r#"
globalThis.sourceFrame = document.createElement('iframe');
sourceFrame.id = 'source-conversion'; document.body.appendChild(sourceFrame);
"#)?;
                materialize_child_realm_through_page_turn_for_test(&mut page_vm, "source-conversion")?;
                page_vm.vm_mut().eval(&format!("globalThis.sourceStage = {stage:?}; globalThis.sourceAction = {action:?};"))?;
                page_vm.vm_mut().eval(r#"
globalThis.oldSource = sourceFrame.contentWindow;
globalThis.sourceBuffer = new oldSource.ArrayBuffer(8);
globalThis.sourceMutations = 0;
globalThis.sourcePayload = null;
onmessage = event => {
  sourcePayload = {value: event.data.value, bytes: event.data.buffer.byteLength,
    sourceIsOld: event.source === oldSource, sourceIsNull: event.source === null,
    sourceIsCurrent: sourceFrame.isConnected && event.source === sourceFrame.contentWindow,
    originMatches: event.origin === location.origin, trusted: event.isTrusted, ports: event.ports.length};
};
oldSource.__sourceControls = {stage: sourceStage, buffer: sourceBuffer, mutate() {
  sourceMutations++;
  sourceFrame.remove();
  if (sourceAction === 'reinsert') document.body.appendChild(sourceFrame);
}};
oldSource.onmessage = new oldSource.Function(`
  const {stage, buffer, mutate} = __sourceControls;
  const options = {
    get transfer() {
      if (stage === 'transfer') mutate();
      if (stage === 'iterator') return {[Symbol.iterator]() {
        let yielded = false;
        return {next() {
          if (yielded) return {done: true};
          yielded = true; mutate(); return {done: false, value: buffer};
        }};
      }};
      return [buffer];
    },
    get targetOrigin() { if (stage === 'targetOrigin') mutate(); return '*'; }
  };
  parent.postMessage({buffer, get value() {
    if (stage === 'payload') mutate(); return 7;
  }}, options);
`);
oldSource.postMessage('go', '*');
"#)?;
                for label in ["child producer", "accepted payload"] {
                    assert!(page_vm.run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::WindowMessage, &loader,
                    ).await?, "{stage}/{action}: {label}");
                }
                assert_eq!(page_vm.vm_mut().eval("JSON.stringify(sourcePayload)")?, r#"{"value":7,"bytes":8,"sourceIsOld":true,"sourceIsNull":false,"sourceIsCurrent":false,"originMatches":true,"trusted":true,"ports":0}"#, "{stage}/{action}");
                assert_eq!(page_vm.vm_mut().eval("sourceMutations + ':' + sourceBuffer.byteLength")?, "1:0", "{stage}/{action}");
            }
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("source capture must precede every author-code conversion boundary");
}

#[tokio::test(flavor = "current_thread")]
async fn window_message_source_survives_document_open_in_the_same_window() {
    run_page_vm_async_test(async {
        let result = window_message_source_lifecycle("document-open").await?;
        assert_eq!(
            result,
            r#"{"sourceWasOldBeforeMutation":true,"windowChanged":false,"payload":{"data":"payload","sourceIsCurrent":true,"sourceIsOld":true,"sourceIsNull":false,"originMatches":true,"trusted":true}}"#
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("replacing a Document must retain the source's LocalWindow identity");
}

#[tokio::test(flavor = "current_thread")]
async fn window_message_uses_typed_one_turn_execution_and_not_the_legacy_wait_driver() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/window-message-owner-turn").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__typedWindowMessageEvents = [];
  addEventListener("message", event => {
    __typedWindowMessageEvents.push("message:" + event.data);
    Promise.resolve().then(() => {
      __typedWindowMessageEvents.push("microtask:" + event.data);
    });
  });
  postMessage("first", "*");
  postMessage("second", "*");
})()
"#,
        )?;

        assert_eq!(page_vm.vm().ms_to_next_timeout(), None);
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__typedWindowMessageEvents.join('|')")?,
            "",
            "a timer-deadline observation must not consume a migrated Window.postMessage task"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::WindowMessage,
                    &loader
                )
                .await?,
            "the first real producer task should consume one selected typed turn"
        );

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__typedWindowMessageEvents.join('|')")?,
            "message:first|microtask:first",
            "one selected turn must checkpoint its microtasks without draining the next message"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::WindowMessage,
                    &loader
                )
                .await?,
            "the second producer task should remain for the next selected turn"
        );

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__typedWindowMessageEvents.join('|')")?,
            "message:first|microtask:first|message:second|microtask:second"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Window.postMessage typed one-turn test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn window_message_completion_syncs_a_microtask_created_child_after_the_checkpoint() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/window-message-microtask-child").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__windowMessageChildOrder = [];
  onmessage = () => {
    __windowMessageChildOrder.push("callback");
    Promise.resolve().then(() => {
      __windowMessageChildOrder.push("microtask");
      const frame = document.createElement("iframe");
      frame.id = "window-message-microtask-child";
      frame.srcdoc = "<!doctype html><body>child</body>";
      document.body.appendChild(frame);
    });
  };
  postMessage("create-child", "*");
})()
"#,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WindowMessage, &loader)
                .await?,
            "the exact Window.postMessage task should run through the selected dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__windowMessageChildOrder.join('|')")?,
            "callback|microtask",
            "the agent checkpoint must run before callback completion synchronizes child records"
        );
        assert!(
            page_vm.vm().has_pending_child_navigation_commit_for_test(),
            "a srcdoc frame created by the reaction must publish its typed navigation commit during post-checkpoint child synchronization"
        );
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(crate::frame_owner_model::ChildFrameSemanticTurnKind::NavigationCommit),
            "the microtask-created srcdoc frame must remain a concrete later Page task"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Window.postMessage post-checkpoint child synchronization test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn window_message_retains_its_local_window_target_across_document_open() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/window-message-document-open").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let before_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main Document owner should exist");
        page_vm.vm_mut().eval(
            r#"
postMessage("queued-before-document-open", "*");
document.open();
document.close();
globalThis.__documentOpenTypedMessages = [];
onmessage = event => __documentOpenTypedMessages.push(event.data);
"queued"
"#,
        )?;
        let after_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("document.open replacement owner should exist");
        assert_ne!(before_document, after_document);
        assert_eq!(
            before_document.local_window_id, after_document.local_window_id,
            "document.open replaces the Document but retains its LocalWindow task target"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::WindowMessage,
                    &loader
                )
                .await?,
            "the pre-document.open task should remain scheduler-visible"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__documentOpenTypedMessages.join('|')")?,
            "queued-before-document-open"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Window.postMessage document.open ownership test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn retiring_a_ready_local_window_readmits_its_stale_stable_task() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/window-message-retirement").unwrap();
        let (mut page_vm, _resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
const frame = document.createElement("iframe");
frame.id = "retired-window-message-target";
document.body.appendChild(frame);
"created"
"#,
        )?;
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "retired-window-message-target",
        )?;
        while owner_wake_rx.try_recv().is_ok() {}

        page_vm.vm_mut().eval(
            r#"
document.getElementById("retired-window-message-target")
  .contentWindow.postMessage("must-not-run", "*");
"queued"
"#,
        )?;
        assert!(
            take_window_message_wake(&mut owner_wake_rx),
            "the original empty-to-nonempty transition should publish one typed wake"
        );
        assert!(!take_window_message_wake(&mut owner_wake_rx));

        page_vm.vm_mut().eval(
            r#"
document.getElementById("retired-window-message-target").remove();
"retired"
"#,
        )?;
        assert!(
            take_window_message_wake(&mut owner_wake_rx),
            "retiring the local payload must readmit the already-ready stable source"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::WindowMessage,
                    &loader
                )
                .await?,
            "the retired target's stable task should remain drainable"
        );
        assert!(
            !page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::WindowMessage,
                    &loader
                )
                .await?,
            "retirement reconsideration must not create a duplicate task"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Window.postMessage retirement readmission test should run");
}

#[test]
fn window_message_rejects_a_real_page_vm_replacement_task_id_collision() {
    run_page_vm_large_stack_async_test("window-message-page-vm-replacement-collision", || async {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/replacement.html",
            "HTTP/1.1 200 OK",
            "<!doctype html><body>replacement</body>".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/initial.html")).unwrap();
        let (page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(
                    r#"
globalThis.__retiredWindowMessageEvents = [];
onmessage = event => __retiredWindowMessageEvents.push(event.data);
postMessage("retired", "*");
"queued"
"#,
                )?;
                let retired_root = page_vm.document_lifecycle.identity().document;

                let replacement_url = format!("{base_url}/replacement.html");
                page_vm
                    .vm_mut()
                    .eval(&format!("location.href = {replacement_url:?}; 'queued'"))?;
                let mut pending_document_lifecycle_turn = None;
                let navigation = page_vm
                    .follow_pending_location_navigation_one_turn_async(
                        &mut pending_document_lifecycle_turn,
                        PageVmInitStage::Load,
                    )
                    .await?;
                assert!(matches!(
                    navigation,
                    crate::runtime::PageVmFollowNavigationTurnOutcome::Completed
                        | crate::runtime::PageVmFollowNavigationTurnOutcome::PostParseLifecycle {
                            ..
                        }
                ));

                let current_root = page_vm.document_lifecycle.identity().document;
                assert_ne!(retired_root, current_root);
                page_vm.vm_mut().eval(
                    r#"
globalThis.__replacementWindowMessageEvents = [];
onmessage = event => __replacementWindowMessageEvents.push(event.data);
postMessage("current", "*");
"queued"
"#,
                )?;

                assert!(
                    page_vm
                        .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WindowMessage, &loader)
                        .await?,
                    "the retired PageVm task should consume one stale selected turn"
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__replacementWindowMessageEvents.join('|')")?,
                    "",
                    "the stale root task must not dispatch or remove the replacement Host payload"
                );

                assert!(
                    page_vm
                        .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WindowMessage, &loader)
                        .await?,
                    "the replacement task must survive the stale local-id collision"
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__replacementWindowMessageEvents.join('|')")?,
                    "current",
                    "discarding the old root task must not remove the replacement Host payload"
                );
                Ok::<_, anyhow::Error>(())
            })
            .await
            .expect("Window.postMessage PageVm replacement should run through the typed executor");
        server
            .await
            .expect("Window.postMessage replacement server should finish");
    });
}
