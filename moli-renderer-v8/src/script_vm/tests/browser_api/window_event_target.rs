use super::*;

const ROUTING_PROBE: &str = r#"(() => {
 const frames=[document.createElement('iframe'),document.createElement('iframe')];
 frames.forEach(frame=>document.body.appendChild(frame));
 const windows=[window,...frames.map(frame=>frame.contentWindow)];
 const rows=[];
 for (const operation of ['dispatchEvent','addEventListener','removeEventListener']) {
  for (let source=0;source<3;source++) for(let target=0;target<3;target++) {
   const type='borrowed-'+operation+'-'+source+'-'+target;
   const trace=[];
   const callback=function(event){trace.push({current:windows.indexOf(event.currentTarget),receiver:windows.indexOf(this)});};
   let thrown=null;
   try {
    if(operation!=='addEventListener')windows.forEach(w=>w.addEventListener(type,callback));
    const method=windows[source].EventTarget.prototype[operation];
    if(operation==='dispatchEvent')method.call(windows[target],new Event(type));
    else {
     method.call(windows[target],type,callback);
     windows.forEach(w=>w.dispatchEvent(new Event(type)));
    }
   }catch(error){thrown=error.name;}
   finally {windows.forEach(w=>w.removeEventListener(type,callback));}
   rows.push({operation,source,target,trace,thrown});
  }
 }
 frames.forEach(frame=>frame.remove());
 return rows;
})()"#;

const CONVERSION_PROBE: &str = r#"(() => {
 const rows=[];
 for(const operation of ['addEventListener','removeEventListener']) for(const phase of ['type','capture']) for(const childMethod of [false,true]) {
  const frame=document.body.appendChild(document.createElement('iframe'));
  const original=frame.contentWindow;
  const method=(childMethod?original:window).EventTarget.prototype[operation];
  const trace=[],conversions=[];let replacement=null,returned=null,thrown=null;
  const callback=function(){trace.push(this===replacement?'new-operation':this===window?'top-operation':'old-operation');};
  const replace=()=>{frame.remove();document.body.appendChild(frame);replacement=frame.contentWindow;
   if(operation==='removeEventListener')replacement.addEventListener('probe',callback);
   else replacement.addEventListener('probe',()=>trace.push('new-sentinel'));
  };
  window.addEventListener('probe',callback);original.addEventListener('probe',callback);
  const type={toString(){conversions.push('type');if(phase==='type')replace();return 'probe';}};
  const options={get capture(){conversions.push('capture');if(phase==='capture')replace();return false;}};
  try{const value=method.call(original,type,callback,options);returned=value===undefined?'undefined':value;}
  catch(error){thrown=error.name;}
  replacement.dispatchEvent(new Event('probe'));window.dispatchEvent(new Event('probe'));
  window.removeEventListener('probe',callback);frame.remove();
  rows.push({operation,phase,childMethod,returned,thrown,conversions,trace});
 }
 return rows;
})()"#;

#[test]
fn borrowed_window_event_target_methods_use_the_receiver_window() {
    let mut vm = new_parsed_test_vm(
        "https://window-eventtarget-receiver.test/",
        "<!doctype html><html><body></body></html>",
    );
    let result = vm
        .eval(&format!("JSON.stringify({ROUTING_PROBE})"))
        .unwrap();
    let actual: serde_json::Value = serde_json::from_str(&result).unwrap();
    let mut expected = Vec::new();
    for operation in ["dispatchEvent", "addEventListener", "removeEventListener"] {
        for source in 0..3 {
            for target in 0..3 {
                let targets: Vec<_> = if operation == "removeEventListener" {
                    (0..3).filter(|window| *window != target).collect()
                } else {
                    vec![target]
                };
                let trace: Vec<_> = targets
                    .into_iter()
                    .map(|window| serde_json::json!({"current": window, "receiver": window}))
                    .collect();
                expected.push(serde_json::json!({"operation":operation,"source":source,"target":target,"trace":trace,"thrown":null}));
            }
        }
    }
    assert_eq!(actual, serde_json::json!(expected));
}

#[test]
fn window_event_target_argument_conversion_cannot_retarget_a_replacement_window() {
    let mut vm = new_parsed_test_vm(
        "https://window-eventtarget-conversion.test/",
        "<!doctype html><html><body></body></html>",
    );
    let result = vm
        .eval(&format!("JSON.stringify({CONVERSION_PROBE})"))
        .unwrap();
    let actual: serde_json::Value = serde_json::from_str(&result).unwrap();
    let mut expected = Vec::new();
    for operation in ["addEventListener", "removeEventListener"] {
        for phase in ["type", "capture"] {
            for child_method in [false, true] {
                let trace = if operation == "removeEventListener" {
                    ["new-operation", "top-operation"]
                } else {
                    ["new-sentinel", "top-operation"]
                };
                expected.push(serde_json::json!({"operation":operation,"phase":phase,"childMethod":child_method,"returned":"undefined","thrown":null,"conversions":["type","capture"],"trace":trace}));
            }
        }
    }
    assert_eq!(actual, serde_json::json!(expected));
}

#[test]
fn borrowed_window_event_target_methods_check_origin_before_argument_conversion() {
    let mut vm = new_parsed_test_vm(
        "https://window-eventtarget-security.test/",
        "<!doctype html><html><body></body></html>",
    );
    vm.exec(
        r#"
        globalThis.foreignFrame = document.createElement('iframe');
        foreignFrame.src = 'data:text/html,<title>opaque</title>';
        document.body.appendChild(foreignFrame);
        "#,
        None,
    )
    .unwrap();
    vm.drain_pending_child_frame_work_for_test();
    let result = vm.eval(r#"
        (() => {
          const target = foreignFrame.contentWindow;
          const rows = [];
          for (const method of ['addEventListener', 'removeEventListener', 'dispatchEvent']) {
            let conversions = 0, name = null, realm = false;
            const type = {toString() { conversions++; return 'probe'; }};
            const options = {get capture() { conversions++; return false; }};
            try {
              if (method === 'dispatchEvent') EventTarget.prototype[method].call(target, new Event('probe'));
              else EventTarget.prototype[method].call(target, type, () => {}, options);
            } catch (error) {
              name = error.name;
              realm = error instanceof DOMException;
            }
            rows.push([method, conversions, name, realm]);
          }
          return JSON.stringify(rows);
        })()
    "#).unwrap();
    assert_eq!(
        result,
        r#"[["addEventListener",0,"SecurityError",true],["removeEventListener",0,"SecurityError",true],["dispatchEvent",0,"SecurityError",true]]"#,
    );
}
