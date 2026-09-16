use super::*;

const SYNTHETIC_PROBE: &str = r#"
(() => {
  const rows = {};
  const NativeErrorEvent = ErrorEvent;
  const NativeEvent = Event;
  const marker = new TypeError('marker');
  let activeEvent, trace, mode;
  let callbackGets = 0;
  const later = event => { trace.push('after'); };
  const capture = () => trace.push('capture');
  const replacement = function() { trace.push('replacement:' + arguments.length); return true; };
  const handler = function(message, filename, line, column, error) {
    trace.push({args:arguments.length, receiver:this===self,
      values:arguments.length===5 ? [message, filename, line, column, error===marker] : [message===activeEvent]});
    return mode === 'plain' ? false : true;
  };
  const before = event => {
    if (event !== activeEvent) { trace.push('reported:' + event.error.name); self.onerror=null; event.preventDefault(); return; }
    trace.push('before');
    if (mode==='replace' || mode==='object') self.onerror=replacement;
    if (mode==='clear') self.onerror=null;
    if (mode==='reactivate') { self.onerror=null; self.onerror=replacement; }
    if (mode==='stopImmediate') event.stopImmediatePropagation();
    if (mode==='stop') event.stopPropagation();
  };
  self.addEventListener('error', before);
  for (mode of ['order','replace','clear','reactivate','object','proxy','revoked-object','revoked-function','plain','forged','shadow','stopImmediate','stop','capture']) {
    trace=[];
    self.onerror=null;
    self.removeEventListener('error', later);
    self.removeEventListener('error', capture, true);
    let value=handler;
    if (mode==='object') value={handleEvent() { throw new Error('must not call handleEvent'); }};
    if (mode==='proxy') value=new Proxy(handler,{get() {callbackGets++; throw new Error('callback lookup');}});
    if (mode.startsWith('revoked-')) { const p=Proxy.revocable(mode==='revoked-object'?{}:handler,{});value=p.proxy;p.revoke(); }
    self.onerror=value;
    const identity=self.onerror===value;
    self.addEventListener('error',later);
    if(mode==='capture')self.addEventListener('error',capture,true);
    activeEvent=mode==='plain'||mode==='forged'?new NativeEvent('error',{cancelable:true}):new NativeErrorEvent('error',{message:'message',filename:'source.js',lineno:12,colno:34,error:marker,cancelable:true});
    if(mode==='forged') Object.setPrototypeOf(activeEvent,NativeErrorEvent.prototype);
    let fieldReads=0;
    if(mode==='shadow') for(const key of ['message','filename','lineno','colno','error']) Object.defineProperty(activeEvent,key,{get(){fieldReads++;throw new Error('field getter');}});
    const returned=self.dispatchEvent(activeEvent);
    rows[mode]={identity,trace,returned,fieldReads};
  }
  self.onerror=null;
  self.removeEventListener('error',before);
  self.removeEventListener('error',later);
  self.removeEventListener('error',capture,true);
  rows.callbackGets=callbackGets;
  rows.returns=[];
  for(const value of [true,false,undefined,null,0,1,'truthy',{},new Boolean(true)]) {
    self.onerror=()=>value;
    rows.returns.push([
      self.dispatchEvent(new NativeErrorEvent('error',{cancelable:true})),
      self.dispatchEvent(new NativeEvent('error',{cancelable:true}))
    ]);
  }
  self.onerror=null;
  // An ErrorEvent on an ordinary EventTarget keeps the one-argument/false-cancels rules.
  const reader=new FileReader();
  let readerArguments;
  reader.onerror=function(event){readerArguments=[arguments.length,event instanceof NativeErrorEvent,this===reader];return false;};
  rows.reader={returned:reader.dispatchEvent(new NativeErrorEvent('error',{cancelable:true})),arguments:readerArguments};
  return rows;
})()

"#;

const NATIVE_PROBE: &str = r#"
const config=__CONFIG__;
const NativeErrorEvent=ErrorEvent;
const marker=config.mode==='throw-undefined'?undefined:config.mode==='throw-null'?null:config.mode==='throw-number'?42:new TypeError('marker');
let trace=[],ctorCalls=0,fieldReads=0;
const replacement=function(message,url,line,column,error){trace.push({replacement:true,args:arguments.length,same:error===marker});return true;};
self.addEventListener('error',event=>{
  trace.push('before');
  if(config.mode==='replace'||config.mode==='object') self.onerror=replacement;
  if(config.mode==='clear') self.onerror=null;
  if(config.mode==='reactivate'){self.onerror=null;self.onerror=replacement;}
  if(config.mode==='stopImmediate'){event.preventDefault();event.stopImmediatePropagation();}
  if(config.mode==='stop') event.stopPropagation();
  if(config.mode==='shadow')for(const key of ['message','filename','lineno','colno','error'])Object.defineProperty(event,key,{get(){fieldReads++;throw new Error('field getter');}});
});
let handler=function(message,url,line,column,error){
  trace.push({handler:true,args:arguments.length,same:error===marker,receiver:this===self,message:message.includes(config.mode==='throw-undefined'?'undefined':config.mode==='throw-null'?'null':config.mode==='throw-number'?'42':'marker'),url:typeof url==='string'&&url.length>0,location:line>0&&column>0});
  return true;
};
if(config.mode==='object')handler={};
if(config.mode==='proxy')handler=new Proxy(handler,{get(){throw new Error('callback lookup');}});
self.onerror=handler;
self.addEventListener('error',event=>{
  trace.push({after:true,prevented:event.defaultPrevented,errorEvent:event instanceof NativeErrorEvent,trusted:event.isTrusted});
  event.preventDefault();
});
if(config.mode==='capture')self.addEventListener('error',()=>trace.push('capture'),true);
if(config.mode==='poison-init')Object.defineProperty(Object.prototype,'bubbles',{get(){fieldReads++;throw new Error('init getter');},configurable:true});
if(config.mode==='delete-constructor')delete self.ErrorEvent;
if(config.mode==='replace-constructor')self.ErrorEvent=function(){ctorCalls++;return new Event('error',{cancelable:true});};
function run(){
  setTimeout(()=>__FINISH__({trace,ctorCalls,fieldReads}),0);
  throw marker;
}
setTimeout(run,0);

"#;

fn expected_synthetic_onerror() -> serde_json::Value {
    let handler =
        serde_json::json!({"args":5,"receiver":true,"values":["message","source.js",12,34,true]});
    let ordinary = serde_json::json!({"args":1,"receiver":true,"values":[true]});
    let mut expected = serde_json::Map::new();
    for (mode, trace, returned) in [
        (
            "order",
            serde_json::json!(["before", handler, "after"]),
            false,
        ),
        (
            "replace",
            serde_json::json!(["before", "replacement:5", "after"]),
            false,
        ),
        ("clear", serde_json::json!(["before", "after"]), true),
        ("reactivate", serde_json::json!(["before", "after"]), true),
        (
            "object",
            serde_json::json!(["before", "replacement:5", "after"]),
            false,
        ),
        (
            "proxy",
            serde_json::json!(["before", handler, "after"]),
            false,
        ),
        (
            "revoked-object",
            serde_json::json!(["before", "after"]),
            true,
        ),
        (
            "revoked-function",
            serde_json::json!(["before", "reported:TypeError", "after", "after"]),
            true,
        ),
        (
            "plain",
            serde_json::json!(["before", ordinary, "after"]),
            false,
        ),
        (
            "forged",
            serde_json::json!(["before", ordinary, "after"]),
            true,
        ),
        (
            "shadow",
            serde_json::json!(["before", handler, "after"]),
            false,
        ),
        ("stopImmediate", serde_json::json!(["before"]), true),
        (
            "stop",
            serde_json::json!(["before", handler, "after"]),
            false,
        ),
        (
            "capture",
            serde_json::json!(["capture", "before", handler, "after"]),
            false,
        ),
    ] {
        expected.insert(
            mode.into(),
            serde_json::json!({"identity":true,"trace":trace,"returned":returned,"fieldReads":0}),
        );
    }
    expected.insert("callbackGets".into(), 0.into());
    expected.insert(
        "returns".into(),
        serde_json::json!([
            [false, true],
            [true, false],
            [true, true],
            [true, true],
            [true, true],
            [true, true],
            [true, true],
            [true, true],
            [true, true]
        ]),
    );
    expected.insert(
        "reader".into(),
        serde_json::json!({"returned":false,"arguments":[1,true,true]}),
    );
    serde_json::Value::Object(expected)
}

#[tokio::test]
async fn worker_onerror_synthetic_dispatch_preserves_order_arguments_and_cancellation() {
    ensure_v8();
    let mut handle = spawn_worker(
        format!("postMessage({SYNTHETIC_PROBE});close();"),
        "https://worker-onerror.test/synthetic.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("worker onerror timed out")
        .expect("worker channel closed");
    let actual: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(actual, expected_synthetic_onerror());
}

#[tokio::test]
async fn worker_onerror_native_errors_use_ordered_handlers_and_intrinsic_event_data() {
    ensure_v8();
    for kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
        for mode in [
            "order",
            "replace",
            "clear",
            "reactivate",
            "object",
            "proxy",
            "stopImmediate",
            "stop",
            "shadow",
            "delete-constructor",
            "replace-constructor",
            "capture",
            "poison-init",
            "throw-undefined",
            "throw-null",
            "throw-number",
        ] {
            let source = NATIVE_PROBE
                .replace("__CONFIG__", &serde_json::json!({"mode":mode}).to_string())
                .replace("__FINISH__", "(value => { postMessage(value); close(); })");
            let mut handle = spawn_test_worker_with_options(
                WorkerSpawnOptions::new(source, "https://worker-onerror.test/native.js".into())
                    .with_script_kind(kind),
            );
            let message = timeout(TIMEOUT, handle.recv())
                .await
                .unwrap_or_else(|_| panic!("{mode}: worker onerror timed out"))
                .expect("worker channel closed");
            let actual: serde_json::Value =
                serde_json::from_str(&expect_post_json(message)).unwrap();
            let mut trace = Vec::new();
            if mode == "capture" {
                trace.push(serde_json::json!("capture"));
            }
            trace.push(serde_json::json!("before"));
            if mode != "stopImmediate" {
                if matches!(mode, "replace" | "object") {
                    trace.push(serde_json::json!({"replacement":true,"args":5,"same":true}));
                } else if !matches!(mode, "clear" | "reactivate") {
                    trace.push(serde_json::json!({"handler":true,"args":5,"same":true,"receiver":true,"message":true,"url":true,"location":true}));
                }
                trace.push(serde_json::json!({"after":true,"prevented":!matches!(mode,"clear"|"reactivate"),"errorEvent":true,"trusted":true}));
            }
            assert_eq!(
                actual,
                serde_json::json!({"trace":trace,"ctorCalls":0,"fieldReads":0}),
                "{kind:?}/{mode}"
            );
        }
    }
}
