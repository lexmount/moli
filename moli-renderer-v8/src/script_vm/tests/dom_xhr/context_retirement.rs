use super::*;

#[test]
fn window_xhr_context_lifecycle_does_not_root_unused_instances() {
    let mut vm = new_storage_test_vm("https://xhr-context-gc.test/");
    vm.eval(
        "globalThis.keptXhr=new XMLHttpRequest();globalThis.weakXhr=new WeakRef(new XMLHttpRequest());",
    )
    .unwrap();
    for _ in 0..2 {
        vm.renderer_document_isolate
            .clone()
            .with_entered_renderer_document_isolate(|isolate| {
                isolate.clear_kept_objects();
                isolate.low_memory_notification();
                Ok(())
            })
            .unwrap();
    }
    assert_eq!(vm.eval("weakXhr.deref()===undefined").unwrap(), "true");
    // Collection of one observer must not detach another live instance.
    vm.eval("keptXhr.open('GET','data:text/plain,live')")
        .unwrap();
    assert_eq!(vm.eval("String(keptXhr.readyState)").unwrap(), "1");
}

#[tokio::test(flavor = "current_thread")]
async fn window_xhr_context_retirement_disposes_instances_without_changing_ready_state() {
    for stage in ["unsent", "opened", "local-pending", "done"] {
        for transition in ["remove", "navigate", "document-open"] {
            // Local response and navigation tasks can race before navigation
            // commits. The held network response controls cover that case.
            if stage == "local-pending" && transition == "navigate" {
                continue;
            }
            for abort in [false, true] {
                let loader = static_http_loader(std::iter::empty::<String>());
                let mut vm = new_page_task_executor_test_vm_with_loader(
                    "https://xhr-context-retirement.test/",
                    &loader,
                );
                let source = r#"
globalThis.result='pending';
(async()=>{
  const stage=STAGE, transition=TRANSITION, abort=ABORT;
  const frame=document.createElement('iframe');
  let loaded=new Promise(resolve=>frame.onload=resolve);
  frame.srcdoc='<!doctype html><body>old</body>';document.body.append(frame);await loaded;
  const xhr=new frame.contentWindow.XMLHttpRequest(),events=[];
  const snapshot=()=>[xhr.readyState,xhr.status,xhr.responseText,xhr.statusText,xhr.responseURL,xhr.getAllResponseHeaders()];
  for(const type of ['load','error','abort','loadend']) xhr.addEventListener(type,()=>events.push([type,xhr.readyState]));
  if(stage!=='unsent') xhr.open('GET','data:text/plain,retained');
  if(stage==='local-pending') xhr.send();
  if(stage==='done') await new Promise((resolve,reject)=>{xhr.onloadend=resolve;xhr.onerror=reject;xhr.send()});
  events.length=0;
  if(transition==='remove') frame.remove();
  if(transition==='navigate') {loaded=new Promise(resolve=>frame.onload=resolve);frame.srcdoc='<!doctype html><body>new</body>';await loaded}
  if(transition==='document-open') {const doc=frame.contentDocument;doc.open();doc.write('<!doctype html><body>same Window</body>');doc.close()}
  const retired=snapshot();
  await Promise.resolve();
  if(abort) xhr.abort();
  const afterAbort=snapshot();
  const barrier=new XMLHttpRequest();barrier.open('GET','data:text/plain,barrier');
  await new Promise(resolve=>{barrier.onloadend=resolve;barrier.send()});
  result=JSON.stringify({retired,afterAbort,final:snapshot(),events});
  frame.remove();
})().catch(error=>{result=String(error.stack||error)});
"#
                .replace("STAGE", &serde_json::to_string(stage).unwrap())
                .replace("TRANSITION", &serde_json::to_string(transition).unwrap())
                .replace("ABORT", if abort { "true" } else { "false" });
                vm.eval(&source).unwrap();
                advance_page_task_executor_until_eval_equals(
                    &mut vm,
                    &loader,
                    "String(result!=='pending')",
                    "true",
                    "Window XHR context retirement and parent networking barrier",
                )
                .await;
                let result: serde_json::Value =
                    serde_json::from_str(&vm.eval("result").unwrap()).unwrap();
                let state = match stage {
                    "unsent" => 0,
                    "done" => 4,
                    _ => 1,
                };
                let retained = serde_json::json!([
                    4,
                    200,
                    "retained",
                    "OK",
                    "data:text/plain,retained",
                    "content-type: text/plain\r\n"
                ]);
                let empty = |state| serde_json::json!([state, 0, "", "", "", ""]);
                let same_window = transition == "document-open";
                assert_eq!(
                    result["retired"],
                    if stage == "done" && same_window {
                        retained.clone()
                    } else {
                        empty(state)
                    },
                    "{stage}/{transition}/abort={abort}: retirement snapshot"
                );
                let after_state =
                    if abort && (stage == "done" || (same_window && stage == "local-pending")) {
                        0
                    } else {
                        state
                    };
                assert_eq!(
                    result["afterAbort"],
                    if stage == "done" && same_window && !abort {
                        retained.clone()
                    } else {
                        empty(after_state)
                    },
                    "{stage}/{transition}/abort={abort}: microtask snapshot"
                );
                let completes = same_window && stage == "local-pending" && !abort;
                assert_eq!(
                    result["final"],
                    if completes || (same_window && stage == "done" && !abort) {
                        retained.clone()
                    } else {
                        empty(after_state)
                    },
                    "{stage}/{transition}/abort={abort}: final snapshot"
                );
                assert_eq!(
                    result["events"],
                    if completes {
                        serde_json::json!([["load", 4], ["loadend", 4]])
                    } else if same_window && stage == "local-pending" && abort {
                        serde_json::json!([["abort", 4], ["loadend", 4]])
                    } else {
                        serde_json::json!([])
                    },
                    "{stage}/{transition}/abort={abort}: terminal events"
                );
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn window_xhr_context_retirement_clears_partial_response_and_suppresses_later_events() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for transition in ["remove", "navigate", "document-open"] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                head.push(socket.read_u8().await.unwrap());
                assert!(head.len() < 65536);
            }
            assert!(head.starts_with(b"GET /held HTTP/1.1\r\n"));
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nX-Content-Type-Options: nosniff\r\nContent-Length: 4099\r\nConnection: close\r\n\r\n").await.unwrap();
            socket.write_all(&[b'o'; 4096]).await.unwrap();
            // Keep the response withheld while the VM and loader remain live.
            let mut byte = [0];
            let result =
                tokio::time::timeout(std::time::Duration::from_secs(5), socket.read(&mut byte))
                    .await
                    .expect("retirement/abort must close the old transport before fixture cleanup");
            assert!(
                matches!(result, Ok(0))
                    || result.as_ref().is_err_and(|error| matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
                    ))
            );
        });
        let mut config = moli_fetch::FetchConfig::default();
        config.set_http_no_proxy(Some("*".to_owned()));
        let loader = ResourceRequestClient::new(&config).unwrap();
        let mut vm = new_page_task_executor_test_vm_with_loader(&format!("{origin}/page"), &loader);
        vm.eval(
            r#"
globalThis.frame=document.createElement('iframe');globalThis.oldXhr=null;
(async()=>{
 const loaded=new Promise(resolve=>frame.onload=resolve);
 frame.srcdoc='<!doctype html><body>child</body>';document.body.append(frame);await loaded;
 oldXhr=new frame.contentWindow.XMLHttpRequest();oldXhr.open('GET','/held');oldXhr.send();
})();
"#,
        )
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(oldXhr?.readyState===3 && oldXhr.responseText.length===4096)",
            "true",
            "actual child XHR partial response before context transition",
        )
        .await;
        vm.eval(&r#"
globalThis.result='pending';
(async()=>{
 const events=[];
 for(const type of ['readystatechange','load','error','abort','loadend']) oldXhr.addEventListener(type,()=>events.push(type));
 const transition=TRANSITION;
 if(transition==='remove') frame.remove();
 if(transition==='navigate') {const loaded=new Promise(resolve=>frame.onload=resolve);frame.srcdoc='<!doctype html><body>replacement</body>';await loaded}
 if(transition==='document-open') {const doc=frame.contentDocument;doc.open();doc.write('<!doctype html><body>same Window</body>');doc.close()}
 const retired=[oldXhr.readyState,oldXhr.status,oldXhr.responseText.length,oldXhr.getResponseHeader('Content-Length')];
 await Promise.resolve();oldXhr.abort();
 const barrier=new XMLHttpRequest();barrier.open('GET','data:text/plain,parent');
 await new Promise(resolve=>{barrier.onloadend=resolve;barrier.send()});
 result=JSON.stringify({retired,events,final:[oldXhr.readyState,oldXhr.status,oldXhr.responseText,oldXhr.responseURL],parent:[barrier.status,barrier.responseText]});
})().catch(error=>{result=String(error.stack||error)});
"#.replace("TRANSITION", &serde_json::to_string(transition).unwrap())).unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(result!=='pending')",
            "true",
            "retired streaming XHR and independent parent networking barrier",
        )
        .await;
        let result: serde_json::Value = serde_json::from_str(&vm.eval("result").unwrap()).unwrap();
        assert_eq!(
            result["retired"],
            if transition == "document-open" {
                serde_json::json!([3, 200, 4096, "4099"])
            } else {
                serde_json::json!([3, 0, 0, null])
            },
            "{transition}"
        );
        assert_eq!(
            result["events"],
            if transition == "document-open" {
                serde_json::json!(["readystatechange", "abort", "loadend"])
            } else {
                serde_json::json!([])
            },
            "{transition}"
        );
        assert_eq!(
            result["final"],
            serde_json::json!([0, 0, "", ""]),
            "{transition}"
        );
        assert_eq!(
            result["parent"],
            serde_json::json!([200, "parent"]),
            "{transition}"
        );
        server.await.unwrap();
    }
}
