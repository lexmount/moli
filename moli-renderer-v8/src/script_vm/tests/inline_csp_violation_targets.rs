use super::*;

#[test]
fn csp_source_location_is_absent_outside_javascript_execution() {
    let mut vm = new_storage_test_vm("https://inline-csp-stack.test/");
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        assert_eq!(
            v8::StackTrace::current_stack_trace(scope, 1)
                .unwrap()
                .get_frame_count(),
            0
        );
        assert!(crate::content_security_policy::current_script_violation_location(scope).is_none());
        Ok(())
    })
    .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn inline_csp_violations_follow_exact_documents_and_dispatch_time_targets() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for url in [
        "http://localhost/",
        "https://localhost/",
        "data:text/html,<body>",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(url, &loader);
        vm.eval("document.body.innerHTML='<iframe></iframe>'")
            .unwrap();
        vm.eval(include_str!(
            "../../../tests/fixtures/inline-csp-violation-targets.js"
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(globalThis.__uiEventResults?.complete)",
            "true",
            "inline CSP violations must run through exact-owner Page tasks",
        )
        .await;
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
                .unwrap(),
            "[]",
            "{url}"
        );
        assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "309", "{url}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn inline_csp_report_only_preserves_execution_and_reports_each_policy_as_a_task() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://inline-csp-report.test/",
        &loader,
    );
    vm.set_response_content_security_report_only_policies(&[
        "script-src-attr 'none' 'report-sample'".to_owned(),
        "script-src 'none'".to_owned(),
    ]);
    vm.eval(
        r#"
      globalThis.events = [];
      globalThis.trace = [];
      globalThis.executed = 0;
      const button = document.createElement('button');
      document.body.appendChild(button);
      button.addEventListener('securitypolicyviolation', event => {
        events.push([event.target === button, event.disposition,
          event.effectiveDirective, event.sample]);
        trace.push('violation');
        queueMicrotask(() => trace.push('checkpoint'));
      });
      button.setAttribute('onclick', 'globalThis.executed++');
      button.click();
      button.click();
      Promise.resolve().then(() => trace.push('initial checkpoint'));
    "#,
    )
    .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(trace)").unwrap(),
        r#"["initial checkpoint"]"#
    );
    assert_eq!(vm.eval("executed").unwrap(), "2");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(events.length)",
        "2",
        "each report-only policy must publish one native task",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"[[true,"report","script-src-attr","globalThis.executed++"],[true,"report","script-src-attr",""]]"#
    );
    assert_eq!(
        vm.eval("JSON.stringify(trace)").unwrap(),
        r#"["initial checkpoint","violation","checkpoint","violation","checkpoint"]"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn inline_csp_violation_tasks_retire_with_their_child_and_popup_documents() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://inline-csp-retire.test/",
        &loader,
    );
    vm.eval(
        r#"
      globalThis.delivered = [];
      const frame = document.createElement('iframe');
      document.body.appendChild(frame);
      const popup = open();
      for (const owner of [frame.contentWindow, popup]) {
        const meta = owner.document.createElement('meta');
        meta.httpEquiv = 'Content-Security-Policy';
        meta.content = "script-src-attr 'none'";
        owner.document.head.appendChild(meta);
        owner.document.addEventListener('securitypolicyviolation', () => delivered.push('stale'));
        const button = owner.document.createElement('button');
        owner.document.body.appendChild(button);
        button.setAttribute('onclick', 'void 0');
        void button.onclick;
      }
      frame.remove();
      popup.close();
    "#,
    )
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .unwrap();
    assert_eq!(vm.eval("JSON.stringify(delivered)").unwrap(), "[]");
}
