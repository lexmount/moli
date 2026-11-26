use super::*;

#[test]
fn parser_classic_abort_preserves_cleanup_context() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async {
        let _timeout = crate::v8_execution_watchdog::V8ExecutionWatchdog::override_timeout_for_test(
            crate::v8_execution_watchdog::V8ExecutionWatchdogKind::ScriptTurn,
            std::time::Duration::from_millis(500),
        );
        let mut page_vm = parse_phase_one_html_into_page_vm_for_test(
            r#"<!doctype html><script>
globalThis.events=[];
function note(value) {
  events.push(value);
  queueMicrotask(() => {
    events.push(value + ':micro');
    if (value === 'start') events.push('currentScript:' + (document.currentScript?.id || 'null'));
  });
}
onerror=()=>{note('window:error');return true;};
</script><script id="run" onload="note('element:load')" onerror="note('element:error')">
note('start');for(;;){}
</script><script>note('after');</script>"#,
        )
        .await;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify(events)")
                .expect("recovered isolate"),
            r#"["start","start:micro","currentScript:run","after","after:micro"]"#,
            "aborted evaluation must clean up before the parser continuation"
        );
        let diagnostics = page_vm
            .vm_mut()
            .runtime_observable_lifecycle_errors_for_testing();
        assert!(
            diagnostics
                .iter()
                .any(|message| message.contains("script execution exceeded")),
            "the original watchdog failure must remain visible: {diagnostics:?}"
        );
        assert!(
            diagnostics
                .iter()
                .all(|message| !message.contains("parser-boundary lifecycle")),
            "parser failure must not enter the ordinary lifecycle queue: {diagnostics:?}"
        );
    }));
}

#[test]
fn parser_classic_exception_keeps_original_window_error_and_cleanup_order() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async {
        let mut page_vm = parse_phase_one_html_into_page_vm_for_test(
            r#"<!doctype html><script>
globalThis.events=[];
globalThis.originalError=new Error('original');
function note(value) {
  events.push(value);
  queueMicrotask(() => {
    events.push(value + ':micro');
    if (value === 'start') events.push('currentScript:' + (document.currentScript?.id || 'null'));
  });
}
onerror=(_message,_source,_line,_column,error)=>{
  note('window:error:' + (error === originalError));return true;
};
</script><script id="run">
note('start');throw originalError;
</script><script>note('after');</script>"#,
        ).await;
        assert_eq!(
            page_vm.vm_mut().eval("JSON.stringify(events)").expect("exception result"),
            r#"["start","window:error:true","start:micro","currentScript:run","window:error:true:micro","after","after:micro"]"#,
            "JavaScript exceptions must retain their identity and report exactly once"
        );
    }));
}
