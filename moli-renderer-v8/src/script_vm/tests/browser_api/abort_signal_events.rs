use super::*;

const PROBE: &str = include_str!("abort_signal_events.js");

#[test]
fn aborted_signals_release_native_roots_and_preserve_live_reason_identity() {
    let mut vm = new_parsed_test_vm("https://abort-signal-gc.test/", "<!doctype html><body>");
    vm.eval(r#"(() => {
      globalThis.abortWeakGraphs=[];
      for(const mode of ['controller','static','dependent']) (() => {
        const reason={};
        const controller=new AbortController();
        const source=controller.signal;
        const signal=mode==='static'?AbortSignal.abort(reason):mode==='dependent'?AbortSignal.any([source]):source;
        reason.controller=controller;reason.source=source;reason.signal=signal;
        controller.abort(reason);
        abortWeakGraphs.push([controller,source,signal,reason].map(value=>new WeakRef(value)));
      })();
      const controller=new AbortController(),reason={controller};
      controller.abort(reason);
      globalThis.retainedAbortSignal=controller.signal;
      globalThis.retainedAbortReason=new WeakRef(reason);
    })()"#).unwrap();
    vm.renderer_document_isolate
        .clone()
        .with_entered_renderer_document_isolate(|isolate| {
            isolate.clear_kept_objects();
            isolate.low_memory_notification();
            Ok(())
        })
        .unwrap();
    assert_eq!(vm.eval(r#"JSON.stringify([
      abortWeakGraphs.every(graph=>graph.every(ref=>ref.deref()===undefined)),
      retainedAbortSignal.aborted,
      retainedAbortSignal.reason===retainedAbortReason.deref(),
      AbortSignal.any([retainedAbortSignal]).reason===retainedAbortSignal.reason,
      (()=>{try{retainedAbortSignal.throwIfAborted();}catch(error){return error===retainedAbortSignal.reason;}})()
    ])"#).unwrap(), "[true,true,true,true,true]");
}

fn run_probe(expression: &str) -> serde_json::Value {
    let mut vm = new_parsed_test_vm("https://abort-signal-events.test/", "<!doctype html><body>");
    let result = vm
        .eval(&format!("{PROBE}\nJSON.stringify({expression})"))
        .unwrap();
    serde_json::from_str(&result).unwrap()
}

#[test]
fn abort_signal_shares_event_target_listeners_across_realms() {
    let result = run_probe(
        r#"(() => {
        const frame = document.body.appendChild(document.createElement('iframe'));
        const realm = frame.contentWindow;
        try {
            return [abortSignalEventTargetProbe(),
                abortSignalEventTargetProbe(realm, EventTarget.prototype),
                abortSignalEventTargetProbe(window, realm.EventTarget.prototype)];
        } finally { frame.remove(); }
    })()"#,
    );
    let rows = result.as_array().unwrap();
    assert_eq!(rows.len(), 3);
    for row in rows {
        assert_eq!(row["failures"], serde_json::json!([]), "{row}");
        assert_eq!(row["scenarios"].as_array().unwrap().len(), 6);
    }
}

#[test]
fn abort_signal_generated_receiver_checks_use_callee_realm_before_conversion() {
    let result = run_probe(
        r#"(() => {
        const frame = document.body.appendChild(document.createElement('iframe'));
        try { return [abortSignalReceiverProbe(), abortSignalReceiverProbe(frame.contentWindow)]; }
        finally { frame.remove(); }
    })()"#,
    );
    for row in result.as_array().unwrap() {
        assert_eq!(row["failures"], serde_json::json!([]), "{row}");
        assert_eq!(row["checks"], 40);
    }
}

#[test]
fn abort_signal_retained_target_dispatch_uses_callback_realm_lifetime() {
    let result = run_probe("abortSignalLifetimeProbe()");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(result["calls"], 2);
}
