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
