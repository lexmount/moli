use super::service_worker_drain::drain_service_worker_test_turn;
use super::*;

const OPTIONS_PROBE: &str = r#"function checkOptions(targets) {
 const rows=[];let serial=0;
 for(const [targetName,create] of targets) for(const method of ['addEventListener','removeEventListener'])
 for(const callbackKind of ['function','null','primitive']) for(const fail of ['none','capture','once','passive','signal']) {
  const [target,eventType,cleanup]=create(++serial);
  const trace=[],marker={};let count=0,error=null;
  const listener=()=>count++;
  const callback=callbackKind==='function'?listener:callbackKind==='null'?null:42;
  if(method==='removeEventListener')target.addEventListener(eventType,listener);
  const type={toString(){trace.push('type');return eventType;}};
  const options=new Proxy({}, {
   has(_,name){trace.push('has:'+name);return true;},
   get(_,name){trace.push(String(name));if(name===fail)throw marker;return name==='signal'?undefined:false;}
  });
  try{target[method](type,callback,options);}catch(e){error=e===marker?'marker':e.name;}
  target.dispatchEvent(new Event(eventType));
  target.removeEventListener(eventType,listener);
  rows.push({target:targetName,method,callback:callbackKind,fail,trace,error,count});
  if(cleanup)cleanup();
 }
 return rows;
}
"#;

const PORT_PROBE: &str = r#"async function checkPortOptions() {
 const targets=[['MessagePort',()=>{const channel=new MessageChannel();return [channel.port1,'message',()=>{channel.port1.close();channel.port2.close();},()=>channel.port2.postMessage('probe')];}]];
 const rows=[];let serial=0;
 for(const [targetName,create] of targets) for(const method of ['addEventListener','removeEventListener'])
 for(const callbackKind of ['function','null','primitive']) for(const fail of ['none','capture','once','passive','signal']) {
  const [target,eventType,cleanup,send]=create(++serial);
  const trace=[],marker={};let count=0,error=null;
  const listener=()=>count++;
  const callback=callbackKind==='function'?listener:callbackKind==='null'?null:42;
  if(method==='removeEventListener')target.addEventListener(eventType,listener);
  const type={toString(){trace.push('type');return eventType;}};
  const options=new Proxy({}, {
   has(_,name){trace.push('has:'+name);return true;},
   get(_,name){trace.push(String(name));if(name===fail)throw marker;return name==='signal'?undefined:false;}
  });
  try{target[method](type,callback,options);}catch(e){error=e===marker?'marker':e.name;}
  await new Promise(resolve=>{target.onmessage=resolve;target.start();send();});
  target.removeEventListener(eventType,listener);
  rows.push({target:targetName,method,callback:callbackKind,fail,trace,error,count});
  if(cleanup)cleanup();
 }
 return rows;
}
"#;

const PAGE_PROBE: &str = r#"(() => {
 const frame=document.body.appendChild(document.createElement('iframe'));
 const rows=checkOptions([
 ['Window',n=>[window,'options-'+n]],
 ['child Window',n=>[frame.contentWindow,'options-'+n]],
 ['Document',n=>[document,'options-'+n]],
 ['Element',n=>[document.createElement('div'),'options-'+n]],
 ['EventTarget',n=>[new EventTarget(),'options-'+n]],
 ['FileReader',n=>[new FileReader(),'options-'+n]],
 ['AbortSignal',()=>[new AbortController().signal,'abort']]
 ]);frame.remove();return rows;
})()"#;

const WORKER_TARGETS: &str = r#"checkOptions([
 ['WorkerGlobalScope',n=>[self,'options-'+n]],
 ['EventTarget',n=>[new EventTarget(),'options-'+n]],
 ['FileReader',n=>[new FileReader(),'options-'+n]],
 ['AbortSignal',()=>[new AbortController().signal,'abort']]
])"#;

const SIGNAL_PROBE: &str = r#"(() => {
 const frame=document.body.appendChild(document.createElement('iframe'));
 const child=frame.contentWindow;
 const channel=new MessageChannel();
 const targets=[window,child,document,document.createElement('div'),new EventTarget(),new FileReader(),new AbortController().signal,channel.port1];
 const localSignal=new AbortController().signal,childSignal=new child.AbortController().signal;
 const revoked=Proxy.revocable(localSignal,{});revoked.revoke();
 const signals=[['undefined',undefined],['null',null],['number',1],['object',{}],['forged',Object.create(AbortSignal.prototype)],['proxy',new Proxy(localSignal,{})],['revoked',revoked.proxy],['local',localSignal],['child',childSignal]];
 const rows=[];
 for(let index=0;index<targets.length;index++)for(const method of ['addEventListener','removeEventListener'])for(const [kind,signal]of signals){
  const trace=[];let error=null,realm=null;
  const options=new Proxy({}, {get(_,key){trace.push(String(key));return key==='signal'?signal:false;},has(){throw new Error('must not call HasProperty');}});
  try{targets[index][method]('probe',null,options);}catch(e){error=e.name;realm=e instanceof (index===1?child:window).TypeError;}
  rows.push({index,method,kind,trace,error,realm});
 }
 channel.port1.close();channel.port2.close();frame.remove();return rows;
})()"#;

fn assert_option_rows(value: &serde_json::Value, expected_rows: usize) {
    let rows = value.as_array().expect("options probe must return rows");
    assert_eq!(rows.len(), expected_rows);
    for row in rows {
        let add = row["method"] == "addEventListener";
        let primitive = row["callback"] == "primitive";
        let callable = row["callback"] == "function";
        let fail = row["fail"].as_str().unwrap();
        let members: &[&str] = if add {
            &["capture", "once", "passive", "signal"]
        } else {
            &["capture"]
        };
        let mut trace = vec!["type"];
        let mut error = None;
        if primitive {
            error = Some("TypeError");
        } else {
            for member in members {
                trace.push(member);
                if *member == fail {
                    error = Some("marker");
                    break;
                }
            }
        }
        let count = if add {
            usize::from(callable && error.is_none())
        } else {
            usize::from(!callable || error.is_some())
        };
        assert_eq!(row["trace"], serde_json::json!(trace), "{row}");
        assert_eq!(row["error"], serde_json::json!(error), "{row}");
        assert_eq!(row["count"], serde_json::json!(count), "{row}");
    }
}

#[test]
fn event_listener_options_convert_in_order_and_stop_before_mutation() {
    let mut vm = new_parsed_test_vm(
        "https://event-listener-options.test/",
        "<!doctype html><html><body></body></html>",
    );
    let value = vm
        .eval(&format!("{OPTIONS_PROBE}\nJSON.stringify({PAGE_PROBE})"))
        .unwrap();
    assert_option_rows(&serde_json::from_str(&value).unwrap(), 210);
}

#[test]
fn event_listener_options_validate_signal_even_for_null_callbacks() {
    let mut vm = new_parsed_test_vm(
        "https://event-listener-signal-conversion.test/",
        "<!doctype html><html><body></body></html>",
    );
    let value = vm.eval(&format!("JSON.stringify({SIGNAL_PROBE})")).unwrap();
    let rows: serde_json::Value = serde_json::from_str(&value).unwrap();
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 144);
    for row in rows {
        let add = row["method"] == "addEventListener";
        let valid = matches!(
            row["kind"].as_str().unwrap(),
            "undefined" | "local" | "child"
        );
        let expected = if add && !valid {
            serde_json::json!("TypeError")
        } else {
            serde_json::Value::Null
        };
        assert_eq!(row["error"], expected, "{row}");
        assert_eq!(
            row["realm"],
            if expected.is_null() {
                serde_json::Value::Null
            } else {
                serde_json::json!(true)
            },
            "{row}"
        );
        assert_eq!(
            row["trace"],
            if add {
                serde_json::json!(["capture", "once", "passive", "signal"])
            } else {
                serde_json::json!(["capture"])
            },
            "{row}"
        );
    }
}

#[tokio::test]
async fn worker_event_listener_options_share_conversion_and_exception_semantics() {
    for shared in [false, true] {
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let (mut vm, browser_context_runtime) =
            new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
                "https://worker-listener-options.test/",
                &loader,
            );
        let worker_source = if shared {
            format!(
                "{OPTIONS_PROBE}\n{PORT_PROBE}\nonconnect=async e=>e.ports[0].postMessage({{options:{WORKER_TARGETS},ports:await checkPortOptions()}});"
            )
        } else {
            format!(
                "{OPTIONS_PROBE}\n{PORT_PROBE}\n(async()=>postMessage({{options:{WORKER_TARGETS},ports:await checkPortOptions()}}))();"
            )
        };
        let worker_source = serde_json::to_string(&worker_source).unwrap();
        let constructor = if shared { "SharedWorker" } else { "Worker" };
        let port = if shared { "worker.port" } else { "worker" };
        let cleanup = if shared {
            "port.close()"
        } else {
            "worker.terminate()"
        };
        vm.eval(&format!(r#"
            const url=URL.createObjectURL(new Blob([{worker_source}],{{type:'text/javascript'}}));
            const worker=new {constructor}(url),port={port};
            port.onmessage=e=>{{globalThis.optionsResult=e.data;{cleanup};URL.revokeObjectURL(url);}};
            worker.onerror=e=>{{globalThis.optionsResult=String(e.message);}};
        "#)).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while vm.eval("globalThis.optionsResult !== undefined").unwrap() != "true" {
                browser_context_runtime.drain_shared_worker_service_lane();
                drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
            }
        })
        .await
        .expect("worker options probe should settle");
        let value = vm.eval("JSON.stringify(globalThis.optionsResult)").unwrap();
        let value: serde_json::Value = serde_json::from_str(&value).unwrap();
        assert_option_rows(&value["options"], 120);
        assert_option_rows(&value["ports"], 30);
    }
}

#[tokio::test]
async fn message_port_options_errors_preserve_actual_message_listeners() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            "https://message-port-options.test/",
            &loader,
        );
    vm.eval(&format!("{PORT_PROBE}\ncheckPortOptions().then(value=>globalThis.portOptionsResult=value,error=>globalThis.portOptionsResult=String(error));")).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while vm
            .eval("globalThis.portOptionsResult !== undefined")
            .unwrap()
            != "true"
        {
            drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
        }
    })
    .await
    .expect("message port options probe should settle");
    let value = vm
        .eval("JSON.stringify(globalThis.portOptionsResult)")
        .unwrap();
    assert_option_rows(&serde_json::from_str(&value).unwrap(), 30);
}
