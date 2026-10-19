use super::*;

// These projection tests isolate sampling from the rendering task's layout
// publication. Only already-published geometry may be queried by a broadcast.
fn broadcast_published_resize_observations(vm: &mut StandaloneScriptVmHarness) {
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
        let host = unsafe { &*host_ptr };
        let dispatch = crate::native_bridge::OwnerDispatchScope::Top;
        let target = host
            .current_window_document_task_target_for_dispatch_scope(dispatch)
            .unwrap();
        let owner = host
            .current_window_execution_context_owner(dispatch)
            .unwrap();
        crate::context_bootstrap::broadcast_document_resize_observers(
            scope, host_ptr, target, owner, 0,
        )?;
        Ok(())
    })
    .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn resize_observer_entries_preserve_webidl_snapshots_and_callback_realms() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://resize-entries.test/");
    let count: usize = vm
        .eval(include_str!(
            "../../../tests/fixtures/resize-observer-entry-webidl.js"
        ))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(count, 12);
    for index in 0..count {
        let name = vm
            .eval(&format!("__resizeEntryChecks.start({index})"))
            .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(__resizeEntryChecks.result !== null)",
            "true",
            &name,
        )
        .await;
        let result = vm.eval("__resizeEntryChecks.result").unwrap();
        let result: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(result["total"].as_u64().unwrap() > 0, "{name}");
        assert_eq!(result["failures"], serde_json::json!([]), "{name}");
    }
}

#[test]
fn ondemand_ro_uses_published_css_box_axes_and_unzoomed_dimensions() {
    let mut vm = new_storage_html_test_vm("https://resize-published-axis.test/");
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    vm.eval(r#"
        document.body.innerHTML='<div id=target style="display:block;width:50px;height:100px;padding:0;border:2px solid;box-sizing:content-box;writing-mode:vertical-lr;zoom:2"></div>';
        window.samples=[];
    "#).unwrap();
    refresh_layout_for_test(&mut vm);
    let builds = vm.layout_pass_observability_for_test().1;
    vm.eval(
        r#"
        window.observer=new ResizeObserver(entries=>samples.push(...entries.map(e=>[
            e.contentRect.width,e.contentRect.height,
            e.contentBoxSize[0].inlineSize,e.contentBoxSize[0].blockSize,
            e.borderBoxSize[0].inlineSize,e.borderBoxSize[0].blockSize
        ])));
        observer.observe(target);
    "#,
    )
    .unwrap();
    broadcast_published_resize_observations(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify(samples)").unwrap(),
        "[[50,100,100,50,104,54]]"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);

    vm.eval("target.style.writingMode='horizontal-tb'").unwrap();
    broadcast_published_resize_observations(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify(samples)").unwrap(),
        "[[50,100,100,50,104,54]]",
        "OnDemand must continue sampling the axes frozen with its previous layout"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    refresh_layout_for_test(&mut vm);
    let builds = vm.layout_pass_observability_for_test().1;
    // Publishing a snapshot is not itself a legacy RO scheduling operation.
    // This mutation schedules its existing check without requesting layout.
    vm.eval("target.setAttribute('data-resample','yes')")
        .unwrap();
    broadcast_published_resize_observations(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify(samples)").unwrap(),
        "[[50,100,100,50,104,54],[50,100,50,100,54,104]]"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    assert_eq!(
        vm.layout_pass_observability_for_test()
            .3
            .unwrap()
            .paint_operation_count,
        0
    );
}

#[test]
fn real_ro_css_box_projection_excludes_non_replaced_inline_fragments() {
    let mut vm = new_storage_html_test_vm("https://resize-css-box-kind.test/");
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    vm.eval(r#"
        document.body.innerHTML='<span id=inline style="padding:0;border:2px solid">text</span><span id=atomic style="display:inline-block;width:50px;height:100px;padding:0;border:2px solid;box-sizing:content-box;writing-mode:vertical-lr"></span>';
        window.samples=[];
    "#).unwrap();
    refresh_layout_for_test(&mut vm);
    let builds = vm.layout_pass_observability_for_test().1;
    vm.eval(
        r#"
        window.observer=new ResizeObserver(entries=>samples.push(...entries.map(e=>[
            e.target.id,e.contentRect.width,e.contentRect.height,
            e.contentBoxSize[0].inlineSize,e.contentBoxSize[0].blockSize,
            e.borderBoxSize[0].inlineSize,e.borderBoxSize[0].blockSize
        ])));
        observer.observe(document.getElementById('inline'));
        observer.observe(document.getElementById('atomic'));
    "#,
    )
    .unwrap();
    broadcast_published_resize_observations(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify(samples)").unwrap(),
        r#"[["inline",0,0,0,0,0,0],["atomic",50,100,100,50,104,54]]"#,
        "a non-replaced inline's CSSOM fragment union is not an RO CSS box"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
}
