use super::*;

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
    vm.eval(
        "__resizeEntryChecks.resumeAfterLayout = null;
         __resizeEntryChecks.publishLayout = () => new Promise(resolve => {
           __resizeEntryChecks.resumeAfterLayout = resolve;
         });",
    )
    .unwrap();
    for index in 0..count {
        let name = vm
            .eval(&format!("__resizeEntryChecks.start({index})"))
            .unwrap();
        let mut publications = 0;
        loop {
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(__resizeEntryChecks.result !== null || __resizeEntryChecks.resumeAfterLayout !== null)",
                "true",
                &name,
            )
            .await;
            if vm.eval("__resizeEntryChecks.result !== null").unwrap() == "true" {
                break;
            }
            publications += 1;
            assert!(publications <= 2, "{name}: unexpected layout request");
            vm.publish_layout_for_test().unwrap();
            vm.eval(
                "(() => {
                   const resume = __resizeEntryChecks.resumeAfterLayout;
                   __resizeEntryChecks.resumeAfterLayout = null;
                   resume();
                 })()",
            )
            .unwrap();
        }
        let result = vm.eval("__resizeEntryChecks.result").unwrap();
        let result: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(result["total"].as_u64().unwrap() > 0, "{name}");
        assert_eq!(result["failures"], serde_json::json!([]), "{name}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn resize_observer_microtask_can_disconnect_the_next_observer() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://resize-callback-order.test/");
    vm.eval(
        r#"
      globalThis.order = [];
      const target = document.body.appendChild(document.createElement('div'));
      target.style.cssText = 'width:50px;height:20px';
      const first = new ResizeObserver(() => {
        order.push('first');
        Promise.resolve().then(() => { order.push('microtask'); second.disconnect(); });
      });
      const second = new ResizeObserver(() => order.push('second'));
      first.observe(target); second.observe(target);
    "#,
    )
    .unwrap();
    vm.run_one_rendering_update_executor_turn(&loader)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(order)").unwrap(),
        r#"["first","microtask"]"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn resize_observer_uses_flattened_shadow_and_slot_depth() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for slotted in [false, true] {
        let mut vm = new_storage_page_task_executor_test_vm("https://resize-flat-tree.test/");
        vm.eval(&format!(r#"
          globalThis.widths = []; globalThis.errors = [];
          addEventListener('error', e => errors.push(e.message));
          const host = document.body.appendChild(document.createElement('div'));
          host.style.cssText = 'display:block;width:100px;height:100px';
          const root = host.attachShadow({{mode:'open'}});
          const wrapper = root.appendChild(document.createElement('div'));
          const child = document.createElement('div');
          child.style.cssText = 'width:50px;height:20px';
          if ({slotted}) {{ wrapper.appendChild(document.createElement('slot')); host.appendChild(child); }}
          else wrapper.appendChild(child);
          const observer = new ResizeObserver(entries => {{
            for (const entry of entries) if (entry.target === child) {{
              widths.push(entry.contentRect.width);
              if (entry.contentRect.width === 50) child.style.width = '80px';
            }}
          }});
          observer.observe(host); observer.observe(child);
        "#)).unwrap();
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap();
        assert_eq!(
            vm.eval("JSON.stringify([widths,errors])").unwrap(),
            "[[50,80],[]]",
            "slotted={slotted}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn resize_observer_finishes_deep_finite_updates_without_a_loop_error() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://resize-deep-updates.test/");
    vm.eval(
        r#"
      globalThis.delivered = []; globalThis.errors = [];
      addEventListener('error', e => errors.push(e.message));
      const nodes = []; let parent = document.body;
      for (let i = 0; i < 40; i++) {
        const node = parent.appendChild(document.createElement('div'));
        node.style.cssText = 'width:100px;height:20px'; nodes.push(node); parent = node;
      }
      const observer = new ResizeObserver(entries => {
        for (const entry of entries) {
          const index = nodes.indexOf(entry.target);
          if (index === 0 || entry.contentRect.width === 101) {
            delivered.push(index);
            if (index + 1 < nodes.length) nodes[index + 1].style.width = '101px';
          }
        }
      });
      nodes.forEach(node => observer.observe(node));
    "#,
    )
    .unwrap();
    vm.run_one_rendering_update_executor_turn(&loader)
        .await
        .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(delivered.length)",
        "40",
        "finite deep resize notifications must continue through queued rendering work",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify([delivered.length,errors])")
            .unwrap(),
        "[40,[]]"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn resize_observer_finite_skipped_changes_reach_the_next_rendering_turn() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for register_sibling in [false, true] {
        let mut vm = new_storage_page_task_executor_test_vm("https://resize-next-turn.test/");
        vm.eval(&format!(
            r#"
          globalThis.widths = []; globalThis.errors = [];
          addEventListener('error', e => errors.push(e.message));
          const node = document.body.appendChild(document.createElement('div'));
          node.style.cssText = 'width:50px;height:20px';
          const sibling = document.body.appendChild(document.createElement('div'));
          sibling.style.cssText = 'width:70px;height:20px';
          const observer = new ResizeObserver(entries => {{
            widths.push(entries[0].contentRect.width);
            if (widths.length === 1) {{
              if ({register_sibling}) {{
                observer.disconnect();
                const next = new ResizeObserver(entries => {{
                  widths.push(entries[0].contentRect.width); next.disconnect();
                }});
                next.observe(sibling);
              }} else node.style.width = '60px';
            }} else observer.disconnect();
          }});
          observer.observe(node);
        "#
        ))
        .unwrap();
        assert!(
            vm.run_one_rendering_update_executor_turn(&loader)
                .await
                .unwrap()
        );
        assert_eq!(
            vm.eval("JSON.stringify([widths,errors.length])").unwrap(),
            "[[50],1]"
        );
        assert!(
            vm.run_one_rendering_update_executor_turn(&loader)
                .await
                .unwrap(),
            "skipped changes must schedule a later rendering opportunity without external activity"
        );
        assert_eq!(
            vm.eval("JSON.stringify([widths,errors.length])").unwrap(),
            if register_sibling {
                "[[50,70],1]"
            } else {
                "[[50,60],1]"
            },
        );
        assert!(
            !vm.run_one_rendering_update_executor_turn(&loader)
                .await
                .unwrap(),
            "finite changes must converge after disconnect"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn resize_observer_gathers_all_active_targets_before_callbacks() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for existing_c in [false, true] {
        let mut vm = new_storage_page_task_executor_test_vm("https://resize-gather.test/");
        vm.eval(&format!(
            r#"
          globalThis.events = []; globalThis.phase = {};
          addEventListener('error', () => events.push('error'));
          const [a,b,c] = ['a','b','c'].map(id => {{
            const node = document.body.appendChild(document.createElement('div'));
            node.id = id; node.style.cssText = 'width:50px;height:20px'; return node;
          }});
          const first = new ResizeObserver(() => {{
            if (phase === 0) return;
            events.push('first');
            if ({existing_c}) c.style.width = '70px'; else second.observe(c);
            first.disconnect();
          }});
          const second = new ResizeObserver(entries => {{
            if (phase === 0) return;
            events.push('second:' + entries.map(entry => entry.target.id).join(','));
            if (entries.some(entry => entry.target === c)) second.disconnect();
          }});
          first.observe(a); second.observe(b);
          if ({existing_c}) second.observe(c);
        "#,
            if existing_c { 0 } else { 1 }
        ))
        .unwrap();
        if existing_c {
            vm.run_one_rendering_update_executor_turn(&loader)
                .await
                .unwrap();
            vm.eval("phase = 1; events = []; a.style.width = '60px'; b.style.width = '60px';")
                .unwrap();
            vm.publish_layout_for_test().unwrap();
        }
        assert!(
            vm.run_one_rendering_update_executor_turn(&loader)
                .await
                .unwrap()
        );
        assert_eq!(
            vm.eval("JSON.stringify(events)").unwrap(),
            r#"["first","second:b","error"]"#,
            "a later observer's current batch must exclude targets activated by an earlier callback"
        );
        assert!(
            vm.run_one_rendering_update_executor_turn(&loader)
                .await
                .unwrap()
        );
        assert_eq!(
            vm.eval("JSON.stringify(events)").unwrap(),
            r#"["first","second:b","error","second:c"]"#
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn resize_observer_same_depth_feedback_still_reports_a_loop_error() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://resize-feedback.test/");
    vm.eval(
        r#"
      globalThis.deliveries = 0; globalThis.errors = [];
      addEventListener('error', e => errors.push(e.message));
      const node = document.body.appendChild(document.createElement('div'));
      node.style.cssText = 'width:50px;height:20px';
      new ResizeObserver(entries => {
        deliveries++; node.style.width = (entries[0].contentRect.width + 1) + 'px';
      }).observe(node);
    "#,
    )
    .unwrap();
    vm.run_one_rendering_update_executor_turn(&loader)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([deliveries,errors])").unwrap(),
        r#"[1,["ResizeObserver loop completed with undelivered notifications."]]"#
    );
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(
        vm.eval("JSON.stringify([deliveries,errors.length])")
            .unwrap(),
        "[2,2]",
        "ongoing feedback must keep yielding between rendering opportunities"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn resize_observer_deep_growth_yields_without_masking_real_feedback() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for feedback in [false, true] {
        let mut vm = new_storage_page_task_executor_test_vm("https://resize-work-budget.test/");
        vm.eval(&format!(r#"
          globalThis.created = 1; globalThis.errors = [];
          addEventListener('error', e => errors.push(e.message));
          const root = document.body.appendChild(document.createElement('div'));
          root.style.cssText = 'width:100px;height:20px';
          const observer = new ResizeObserver(entries => {{
            for (const entry of entries) if ({feedback} && entry.target === root)
              root.style.width = (entry.contentRect.width + 1) + 'px';
            if (created < 120) {{
              const child = entries[entries.length - 1].target.appendChild(document.createElement('div'));
              child.style.cssText = 'width:100px;height:20px'; created++;
              observer.observe(child);
            }}
          }});
          observer.observe(root);
        "#)).unwrap();
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap();
        assert_eq!(
            vm.eval("created > 1 && created < 120").unwrap(),
            "true",
            "one rendering source task must yield before consuming unbounded deep work"
        );
        assert_eq!(
            vm.eval("String(errors.length)").unwrap(),
            "0",
            "loop errors are decided after the whole broadcast converges"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn resize_observer_continuation_preserves_depth_until_broadcast_settles() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for disconnect in [false, true] {
        let mut vm = new_storage_page_task_executor_test_vm("https://resize-continuation.test/");
        vm.eval(&format!(r#"
          globalThis.rootCalls = 0; globalThis.created = 0; globalThis.errors = [];
          addEventListener('error', e => errors.push(e.message));
          const root = document.body.appendChild(document.createElement('div'));
          root.style.cssText = 'width:100px;height:20px';
          const observer = new ResizeObserver(entries => {{
            for (const entry of entries) if (entry.target === root) {{
              rootCalls++; root.style.width = '101px';
            }}
            if (created < 40) {{
              const child = entries[entries.length - 1].target.appendChild(document.createElement('div'));
              child.style.cssText = 'width:100px;height:20px'; created++;
              observer.observe(child);
            }} else if ({disconnect}) observer.disconnect();
          }});
          observer.observe(root);
        "#)).unwrap();
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap();
        assert_eq!(
            vm.eval("String(errors.length)").unwrap(),
            "0",
            "a yielded broadcast is not finished"
        );
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(created)",
            "40",
            "deep resize continuation",
        )
        .await;
        // Drain the final notification that may disconnect the observer.
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            if disconnect {
                "String(rootCalls === 1 && errors.length === 0)"
            } else {
                "String(errors.length === 1)"
            },
            "true",
            "resize broadcast settlement",
        )
        .await;
        assert_eq!(vm.eval("String(rootCalls)").unwrap(), "1");
        assert_eq!(
            vm.eval("String(errors.length)").unwrap(),
            if disconnect { "0" } else { "1" }
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn resize_observer_later_callback_can_handle_an_earlier_rejection() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for handled in [false, true] {
        let mut vm = new_storage_page_task_executor_test_vm("https://resize-rejections.test/");
        vm.eval(&format!(r#"
          globalThis.events = []; globalThis.rejected = null;
          addEventListener('unhandledrejection', e => {{ events.push('unhandled'); e.preventDefault(); }});
          addEventListener('rejectionhandled', () => events.push('handled'));
          const target = document.body.appendChild(document.createElement('div'));
          target.style.cssText = 'width:50px;height:20px';
          new ResizeObserver(() => {{ rejected = Promise.reject('observer rejection'); }}).observe(target);
          new ResizeObserver(() => {{ if ({handled}) rejected.catch(() => {{}}); }}).observe(target);
        "#)).unwrap();
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap();
        assert_eq!(
            vm.eval("JSON.stringify(events)").unwrap(),
            if handled { "[]" } else { r#"["unhandled"]"# }
        );
    }
}
