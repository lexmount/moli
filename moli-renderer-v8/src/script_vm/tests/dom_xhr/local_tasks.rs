use super::*;

#[tokio::test(flavor = "current_thread")]
async fn window_xhr_local_delivery_follows_microtasks_and_allows_cancellation() {
    for url in [
        "data:text/plain,ok",
        "invalid-protocol://xhr-local.test/failure",
        "http://xhr-local.test:25/failure",
        "blob:https://xhr-local.test/missing",
    ] {
        for action in ["none", "abort", "reopen"] {
            let loader = static_http_loader(std::iter::empty::<String>());
            let mut vm =
                new_page_task_executor_test_vm_with_loader("https://xhr-local.test/", &loader);
            vm.eval(&format!(
                r#"
globalThis.result = 'pending';
globalThis.events = [];
const xhr = new XMLHttpRequest();
for (const type of ['load','error','abort','loadend']) {{
  xhr.addEventListener(type, event => {{
    if (!(event instanceof ProgressEvent) || event.target !== xhr)
      throw new Error('native XHR progress event required');
    events.push(type + ':' + xhr.readyState);
  }});
}}
xhr.open('GET', {url});
xhr.send();
events.push('returned:' + xhr.readyState);
queueMicrotask(() => {{
  events.push('microtask:' + xhr.readyState);
  if ({action} === 'abort') xhr.abort();
  if ({action} === 'reopen') xhr.open('GET','data:text/plain,reopened');
  Promise.resolve().then(() => events.push('promise:' + xhr.readyState));
  const barrier = new XMLHttpRequest();
  barrier.open('GET','data:text/plain,barrier');
  barrier.onloadend = () => {{
    result = JSON.stringify({{events,status:xhr.status,readyState:xhr.readyState,text:xhr.responseText}});
  }};
  barrier.send();
}});
"#,
                url = serde_json::to_string(url).unwrap(),
                action = serde_json::to_string(action).unwrap(),
            ))
            .unwrap();
            assert_eq!(
                vm.eval(
                    "events.some(event => event.startsWith('load:') || event.startsWith('error:'))"
                )
                .unwrap(),
                "false",
                "{url} / {action}: delivery must remain a task after the microtask checkpoint"
            );
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(result !== 'pending')",
                "true",
                "local XHR networking task barrier",
            )
            .await;
            let result: serde_json::Value =
                serde_json::from_str(&vm.eval("result").unwrap()).unwrap();
            let success = url.starts_with("data:");
            let expected_events = match action {
                "abort" => vec![
                    "returned:1",
                    "microtask:1",
                    "abort:4",
                    "loadend:4",
                    "promise:0",
                ],
                "reopen" => vec!["returned:1", "microtask:1", "promise:1"],
                _ => vec![
                    "returned:1",
                    "microtask:1",
                    "promise:1",
                    if success { "load:4" } else { "error:4" },
                    "loadend:4",
                ],
            };
            assert_eq!(
                result["events"],
                serde_json::json!(expected_events),
                "{url} / {action}"
            );
            assert_eq!(
                result["readyState"],
                match action {
                    "abort" => 0,
                    "reopen" => 1,
                    _ => 4,
                }
            );
            assert_eq!(
                result["status"],
                if success && action == "none" { 200 } else { 0 }
            );
            assert_eq!(
                result["text"],
                if success && action == "none" {
                    "ok"
                } else {
                    ""
                }
            );
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn window_xhr_local_delivery_uses_receiver_realm_and_document_lifetime() {
    for receiver_child in [false, true] {
        for methods_child in [false, true] {
            for remove in [false, true] {
                let loader = static_http_loader(std::iter::empty::<String>());
                let mut vm = new_page_task_executor_test_vm_with_loader(
                    "https://xhr-task-owner.test/",
                    &loader,
                );
                vm.eval(&format!(
                    r#"
globalThis.result = 'pending';
(async () => {{
  const frame = document.createElement('iframe');
  const loaded = new Promise(resolve => frame.onload = resolve);
  frame.srcdoc = '<!doctype html><body>XHR owner</body>';
  document.body.append(frame);
  await loaded;
  const receiver = {receiver_child} ? frame.contentWindow : self;
  const methods = ({methods_child} ? frame.contentWindow : self).XMLHttpRequest.prototype;
  const xhr = new receiver.XMLHttpRequest();
  const EventConstructor = receiver.ProgressEvent;
  const events = [];
  let afterMicrotask = false;
  xhr.onload = event => events.push({{
    native:event instanceof EventConstructor, afterMicrotask, text:xhr.responseText
  }});
  methods.open.call(xhr, 'GET', 'data:text/plain,owned', true);
  methods.send.call(xhr);
  queueMicrotask(() => {{ afterMicrotask = true; }});
  if ({remove}) frame.remove();
  const barrier = new XMLHttpRequest();
  barrier.open('GET','data:text/plain,barrier');
  barrier.onload = () => {{
    result = JSON.stringify(events);
    frame.remove();
  }};
  barrier.send();
}})().catch(error => {{ result = String(error.stack || error); }});
"#
                ))
                .unwrap();
                advance_page_task_executor_until_eval_equals(
                    &mut vm,
                    &loader,
                    "String(result !== 'pending')",
                    "true",
                    "cross-realm XHR networking task barrier",
                )
                .await;
                let result: serde_json::Value =
                    serde_json::from_str(&vm.eval("result").unwrap()).unwrap();
                assert_eq!(
                    result,
                    if receiver_child && remove {
                        serde_json::json!([])
                    } else {
                        serde_json::json!([{"native":true,"afterMicrotask":true,"text":"owned"}])
                    },
                    "receiver_child={receiver_child}, methods_child={methods_child}, remove={remove}"
                );
            }
        }
    }
}
