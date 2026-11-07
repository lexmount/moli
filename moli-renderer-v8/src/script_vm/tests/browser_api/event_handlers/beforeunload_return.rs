use super::*;

#[tokio::test(flavor = "current_thread")]
async fn beforeunload_return_conversion_keeps_the_callback_incumbent() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for registration in ["attribute", "idl"] {
        let mut vm =
            new_storage_page_task_executor_test_vm("https://beforeunload-return-incumbent.test/");
        vm.eval(
            r#"
const root = document.documentElement || document.appendChild(document.createElement('html'));
const body = document.body || root.appendChild(document.createElement('body'));
globalThis.frameLoaded = false;
const frame = document.createElement('iframe');
frame.onload = () => { frameLoaded = true; };
body.appendChild(frame);
"#,
        )
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(frameLoaded)",
            "true",
            "finish the child's initial document load before registering callbacks",
        )
        .await;
        vm.eval(&format!(
            r#"
const child = frame.contentWindow;
child.name = 'iframe';
child.__callbackCalls = 0;
globalThis.messages = [];
addEventListener('message', event => messages.push([
  event.data, event.source === child, event.source === window, event.source.name
]));
if ('{registration}' === 'attribute') {{
  child.document.body.setAttribute('onbeforeunload',
    "__callbackCalls++; return {{toString:parent.postMessage.bind(parent,'coerced','*')}};");
}} else {{
  child.eval("onbeforeunload = () => {{ __callbackCalls++; return {{toString:parent.postMessage.bind(parent,'coerced','*')}}; }}");
}}
child.dispatchEvent(new child.Event('beforeunload', {{cancelable:true}}));
"#
        ))
        .expect("invoke the child callback from a parent script");
        assert_eq!(vm.eval("String(child.__callbackCalls)").unwrap(), "1");
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(messages.length)",
            "1",
            "deliver the message queued during return conversion",
        )
        .await;
        assert_eq!(
            vm.eval("JSON.stringify(messages)").unwrap(),
            r#"[["coerced",true,false,"iframe"]]"#,
            "{registration}"
        );
    }
}

#[test]
fn beforeunload_return_conversion_precedes_callback_cleanup_microtasks() {
    let mut vm = new_storage_test_vm("https://beforeunload-return-order.test/");
    vm.eval(
        r#"
globalThis.steps = [];
onbeforeunload = event => {
  globalThis.seenEvent = event;
  steps.push('handler');
  queueMicrotask(() => steps.push(['microtask', window.event === event, event.returnValue]));
  return {toString() { steps.push(['convert', window.event === event]); return 'leave'; }};
};
"#,
    )
    .unwrap();
    vm.with_default_context_scope(|scope, _| {
        let window = scope.get_current_context().global(scope);
        crate::context_bootstrap::dispatch_beforeunload_for_runtime_owner(scope, window);
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify({steps, value:seenEvent.returnValue, canceled:seenEvent.defaultPrevented})")
            .unwrap(),
        r#"{"steps":["handler",["convert",true],["microtask",true,""]],"value":"leave","canceled":true}"#
    );
}

#[test]
fn beforeunload_conversion_exceptions_report_after_callback_cleanup() {
    let mut vm = new_storage_test_vm("https://beforeunload-return-error.test/");
    vm.eval(
        r#"
globalThis.steps = [];
const marker = new Error('conversion');
addEventListener('error', event => {
  steps.push(['error', event.error === marker, window.event === event]);
  event.preventDefault();
});
onbeforeunload = event => {
  globalThis.seenEvent = event;
  steps.push('handler');
  queueMicrotask(() => steps.push('microtask'));
  return {toString() { steps.push('convert'); throw marker; }};
};
"#,
    )
    .unwrap();
    vm.with_default_context_scope(|scope, _| {
        let window = scope.get_current_context().global(scope);
        crate::context_bootstrap::dispatch_beforeunload_for_runtime_owner(scope, window);
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify({steps, value:seenEvent.returnValue, canceled:seenEvent.defaultPrevented})")
            .unwrap(),
        r#"{"steps":["handler","convert","microtask",["error",true,true]],"value":"","canceled":false}"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn beforeunload_message_source_attaches_to_the_current_document_before_property_access() {
    const HOST: &str = "beforeunload-source.localhost";
    const OTHER: &str = "other-source.localhost";
    for cross_origin in [false, true] {
        let source = r#"<!doctype html><script>window.name = 'iframe';</script>
<body onbeforeunload="return {toString:parent.postMessage.bind(parent,'retiring','*')};">source"#;
        let mut bodies = vec![source.to_owned()];
        if cross_origin {
            bodies.push("<!doctype html><body>cross-origin replacement".to_owned());
        }
        let server = StaticHttpServer::spawn_with_bodies(bodies).await;
        let parent_url = server.url_for_host(HOST, "/parent");
        let source_url = server.url_for_host(HOST, "/source");
        let destination = if cross_origin {
            server.url_for_host(OTHER, "/replacement").to_string()
        } else {
            "about:blank".to_owned()
        };
        let loader = static_http_loader([server.resolve_entry(HOST), server.resolve_entry(OTHER)]);
        let mut vm =
            new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
        vm.eval(&format!(
            r#"
const root = document.documentElement || document.appendChild(document.createElement('html'));
const body = document.body || root.appendChild(document.createElement('body'));
globalThis.loads = 0;
globalThis.sourceMessages = [];
const frame = document.createElement('iframe');
frame.onload = () => loads++;
frame.src = {source_url};
body.appendChild(frame);
addEventListener('message', event => {{
  if (event.data !== 'retiring') return;
  const result = {{origin:event.origin, trusted:event.isTrusted}};
  // These property reads must precede the contentWindow identity check.
  try {{ result.name = event.source.name; }} catch (_) {{ result.blocked = true; }}
  try {{ result.url = event.source.location.href; }} catch (_) {{ result.locationBlocked = true; }}
  result.sameProxy = event.source === frame.contentWindow;
  sourceMessages.push(result);
}});
"#,
            source_url = serde_json::to_string(source_url.as_str()).unwrap(),
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(loads)",
            "1",
            "load the source document before navigating it",
        )
        .await;
        vm.eval(&format!(
            "frame.src = {};",
            serde_json::to_string(&destination).unwrap()
        ))
        .unwrap();
        vm.drain_ready_child_frame_task_executor_turns_for_setup(&loader, 128)
            .await
            .unwrap();
        if cross_origin {
            wait_for_one_page_resource_completion_executor_test_turn(
                &mut vm,
                "replacement document response",
            )
            .await;
            vm.drain_ready_child_frame_task_executor_turns_for_setup(&loader, 128)
                .await
                .unwrap();
        }
        assert_eq!(vm.eval("String(loads)").unwrap(), "2");
        assert!(
            vm.run_one_window_message_executor_turn(&loader)
                .await
                .unwrap()
        );
        let result: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(sourceMessages)").unwrap()).unwrap();
        let expected = if cross_origin {
            serde_json::json!([{"origin":parent_url.origin().ascii_serialization(), "trusted":true,
                "blocked":true, "locationBlocked":true, "sameProxy":true}])
        } else {
            serde_json::json!([{"origin":parent_url.origin().ascii_serialization(), "trusted":true,
                "name":"iframe", "url":"about:blank", "sameProxy":true}])
        };
        assert_eq!(result, expected, "cross_origin={cross_origin}");
        assert_eq!(
            server.finish().await.len(),
            if cross_origin { 2 } else { 1 }
        );
    }
}
