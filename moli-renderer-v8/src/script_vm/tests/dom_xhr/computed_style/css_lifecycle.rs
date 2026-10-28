use super::*;

mod animation_lists;
mod computed_keyframes;
mod document_batch;
mod effect_conversion;
mod initial_observation;
mod playback_contracts;
mod playback_updates;
mod projection_contracts;
mod reentrant_transitions;
mod rendered_lifetime;
mod review_boundaries;
mod review_contracts;
mod sampling_contexts;
mod shared_instances;
mod signed_delays;
mod style_triggers;
mod transition_timing;

#[tokio::test]
async fn css_lifecycle_infinite_animation_with_positive_delay_does_not_overflow() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/infinite-delay",
        "<html><head></head><body></body></html>",
        &loader,
    );
    vm.eval(
        r#"
        document.head.innerHTML = `<style>
          @keyframes spin { from { left: 0px; } to { left: 10px; } }
          .spin { position: relative; animation: spin 1s linear 0.1s infinite; }
        </style>`;
        document.body.innerHTML = '<div id="box" class="spin"></div>';
    "#,
    )
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.eval("document.body.className = 'unrelated'").unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("getComputedStyle(document.getElementById('box')).animationIterationCount")
            .unwrap(),
        "infinite"
    );
}

#[tokio::test]
async fn css_lifecycle_scans_keep_parent_and_child_document_state_independent() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/documents",
        "<html><head></head><body><iframe id=child></iframe></body></html>",
        &loader,
    );
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 64)
        .await
        .unwrap();
    vm.eval(
        r#"
        globalThis.events = [];
        const child = document.getElementById('child').contentDocument;
        const css = `@keyframes move { from { left: 0px; } to { left: 10px; } }
          .box { position: relative; width: 50px; transition: width 60s; }
          .active { width: 200px; animation: move 60s infinite; }`;
        for (const [doc, label] of [[document, 'top'], [child, 'child']]) {
          doc.head.appendChild(doc.createElement('style')).textContent = css;
          const box = doc.body.appendChild(doc.createElement('div'));
          box.id = 'box'; box.className = 'box';
          for (const type of ['animationstart','transitionrun']) {
            box.addEventListener(type, e => events.push(label + ':' + e.type));
          }
        }
    "#,
    )
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 64)
        .await
        .unwrap();
    vm.eval(r#"
        document.getElementById('box').className = 'box active';
        document.getElementById('child').contentDocument.getElementById('box').className = 'box active';
    "#).unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 64)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("events.slice().sort().join('|')").unwrap(),
        "child:animationstart|child:transitionrun|top:animationstart|top:transitionrun"
    );
    vm.eval("document.body.className = 'unrelated'").unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 64)
        .await
        .unwrap();
    assert_eq!(vm.eval("events.length").unwrap(), "4");
}

#[tokio::test]
async fn css_lifecycle_listener_and_style_scan_order_has_one_start_and_live_record() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for listener_first in [false, true] {
        let mut vm = new_parsed_page_task_executor_test_vm(
            "https://css-lifecycle.test/listener-order",
            "<html><head></head><body></body></html>",
            &loader,
        );
        vm.eval(
            r#"
            document.head.innerHTML = `<style>
              @keyframes move { from { left: 0px; } to { left: 10px; } }
              .active { position: relative; animation: move 60s; }
            </style>`;
            document.body.innerHTML = '<div id="box"></div>';
            globalThis.events = [];
            globalThis.listen = () => {
              for (const type of ['animationstart','animationcancel']) {
                document.getElementById('box').addEventListener(type, e => events.push(e.type));
              }
            };
        "#,
        )
        .unwrap();
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
            .await
            .unwrap();
        let change = "document.getElementById('box').className = 'active'";
        if listener_first {
            vm.eval("listen()").unwrap();
        }
        vm.eval(change).unwrap();
        if !listener_first {
            vm.eval("listen()").unwrap();
        }
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
            .await
            .unwrap();
        assert_eq!(
            vm.eval("events.join('|')").unwrap(),
            "animationstart",
            "listener_first={listener_first}"
        );
        vm.eval("document.getElementById('box').className = ''")
            .unwrap();
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
            .await
            .unwrap();
        assert_eq!(
            vm.eval("events.join('|')").unwrap(),
            "animationstart|animationcancel",
            "listener_first={listener_first}"
        );
    }
}

pub(super) async fn wait_for_css_events(
    vm: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
    expression: &str,
    expected: &str,
) {
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if vm.eval(expression).unwrap() == expected {
                break;
            }
            if !vm
                .run_one_oldest_ready_page_task_executor_turn(loader)
                .await
                .unwrap()
            {
                assert!(vm.wait_for_task_executor_work_arrival().await);
            }
        }
    })
    .await;
    if result.is_err() {
        panic!(
            "CSS deadline did not publish work; query {expression:?}, expected {expected:?}, actual {:?}; events {:?}",
            vm.eval(expression),
            vm.eval("typeof events==='undefined'?'missing':JSON.stringify(events)")
        );
    }
}

#[tokio::test]
async fn css_lifecycle_static_page_dispatches_delayed_animation_and_transition_boundaries() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/deadlines",
        "<html><head></head><body></body></html>",
        &loader,
    );
    vm.eval(r#"
        document.head.innerHTML = `<style>
          @keyframes move { from { left: 0px; } to { left: 10px; } }
          #box { opacity: 0; transition: opacity 0.04s 0.04s; }
          #box.active { opacity: 1; animation: move 0.04s linear 0.04s 2; }
        </style>`;
        document.body.innerHTML = '<div id="box"></div>';
        document.body.className = 'baseline';
        globalThis.events = [];
        for (const type of ['animationstart','animationend','transitionrun','transitionstart','transitionend']) {
          document.getElementById('box').addEventListener(type, e => events.push([e.type,e.elapsedTime]));
        }
    "#).unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.eval("document.getElementById('box').className = 'active'")
        .unwrap();
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(
        vm.eval("events.map(e => e[0]).join('|')").unwrap(),
        "transitionrun"
    );
    assert!(
        !vm.has_ready_timeout(),
        "CSS timing must not create a JavaScript timer"
    );
    wait_for_css_events(&mut vm, &loader, "events.length.toString()", "5").await;
    let mut events: Vec<(String, f64)> =
        serde_json::from_str(&vm.eval("JSON.stringify(events)").unwrap()).unwrap();
    events.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        events.iter().map(|e| e.0.as_str()).collect::<Vec<_>>(),
        [
            "animationend",
            "animationstart",
            "transitionend",
            "transitionrun",
            "transitionstart"
        ]
    );
    for (kind, elapsed) in events {
        let expected = match kind.as_str() {
            "animationend" => 0.08,
            "transitionend" => 0.04,
            _ => 0.0,
        };
        assert!((elapsed - expected).abs() < 1e-6, "{kind}: {elapsed}");
    }
    assert!(
        !vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap(),
        "completed effects must leave no periodic wake"
    );
}

#[tokio::test]
async fn css_lifecycle_animation_name_round_trip_cancels_and_restarts() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/names",
        "<html><head></head><body></body></html>",
        &loader,
    );
    vm.eval(r#"
        document.head.innerHTML = `<style>
          @keyframes a { from { left: 0px; } to { left: 10px; } }
          @keyframes b { from { left: 0px; } to { left: 20px; } }
          #box { position: relative; animation: a 60s infinite; }
          #box.b { animation-name: b; }
        </style>`;
        document.body.innerHTML = '<div id="box"></div>';
        globalThis.events = [];
        for (const type of ['animationstart','animationcancel']) {
          document.getElementById('box').addEventListener(type, e => events.push(e.type + ':' + e.animationName));
        }
    "#).unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.eval("document.getElementById('box').className = 'b'")
        .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.eval("document.getElementById('box').className = ''")
        .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.eval("document.body.className = 'unrelated'").unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "animationstart:a|animationcancel:a|animationstart:b|animationcancel:b|animationstart:a"
    );
}

#[tokio::test]
async fn css_lifecycle_transition_can_be_enabled_with_the_target_value() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/new-transition",
        "<html><head></head><body></body></html>",
        &loader,
    );
    vm.eval(r#"
        document.head.innerHTML = `<style>
          #box { opacity: 0; }
          #box.active { opacity: 1; transition: all 60s; }
        </style>`;
        document.body.innerHTML = '<div id="box"></div>';
        document.body.className = 'baseline';
        globalThis.events = [];
        for (const type of ['transitionrun','transitionstart']) {
          document.getElementById('box').addEventListener(type, e => events.push(e.type + ':' + e.propertyName));
        }
    "#).unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.eval("document.getElementById('box').className = 'active'")
        .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "transitionrun:opacity|transitionstart:opacity"
    );
    vm.eval("document.body.className = 'another-unrelated-class'")
        .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    assert_eq!(vm.eval("events.length").unwrap(), "2");
}

#[tokio::test]
async fn css_lifecycle_removed_transition_property_cancels_without_cancelling_retained_property() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/property-removal",
        "<html><head></head><body></body></html>",
        &loader,
    );
    vm.eval(r#"
        document.head.innerHTML = `<style>
          #box { width: 50px; opacity: 0; transition: width 60s, opacity 60s; }
          #box.active { width: 200px; opacity: 1; }
        </style>`;
        document.body.innerHTML = '<div id="box"></div>';
        document.body.className = 'baseline';
        globalThis.events = [];
        for (const type of ['transitionrun','transitionstart','transitioncancel']) {
          document.getElementById('box').addEventListener(type, e => events.push(e.type + ':' + e.propertyName));
        }
    "#).unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.eval("document.getElementById('box').className = 'active'")
        .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.eval("document.getElementById('box').style.transitionProperty = 'opacity'")
        .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("events.filter(e => e.startsWith('transitioncancel')).join('|')")
            .unwrap(),
        "transitioncancel:width"
    );
    vm.eval("document.body.className = 'unrelated'").unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("events.filter(e => e.startsWith('transitioncancel')).join('|')")
            .unwrap(),
        "transitioncancel:width"
    );
    // The before-change style must keep advancing while width is excluded.
    // Re-admitting width without changing its value must not restart it.
    vm.eval("document.getElementById('box').style.width = '300px'")
        .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.eval("document.getElementById('box').style.transitionProperty = 'width, opacity'")
        .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("events.filter(e => e === 'transitionrun:width').length")
            .unwrap(),
        "1"
    );
    vm.eval("document.getElementById('box').style.transitionProperty = 'none'")
        .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("events.filter(e => e.startsWith('transitioncancel')).join('|')")
            .unwrap(),
        "transitioncancel:width|transitioncancel:opacity"
    );
}

#[tokio::test]
async fn css_lifecycle_document_replacement_cancels_old_deadlines() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for replace_from_listener in [false, true] {
        let mut vm = new_parsed_page_task_executor_test_vm(
            "https://css-lifecycle.test/replacement",
            "<html><head></head><body></body></html>",
            &loader,
        );
        let old_owner = vm
            ._context_host
            .borrow()
            .current_main_document_task_owner()
            .unwrap();
        vm.eval(&format!(
            "globalThis.replaceFromListener = {replace_from_listener}"
        ))
        .unwrap();
        vm.eval(r#"
            globalThis.events = [];
            globalThis.replaceDocument = () => {
              document.open();
              document.write('<html><head><style>@keyframes b { from { left: 0px; } to { left: 20px; } } #fresh { position: relative; animation: b 0.1s; }</style></head><body><div id="fresh"></div></body></html>');
              document.close();
              const fresh = document.getElementById('fresh');
              for (const type of ['animationstart','animationend']) fresh.addEventListener(type, e => events.push(e.animationName + ':' + e.type));
            };
            document.head.innerHTML = '<style>@keyframes a { from { left: 0px; } to { left: 10px; } } #box { position: relative; animation: a 0.05s; animation-delay: ' + (replaceFromListener ? '0s' : '60s') + '; }</style>';
            document.body.innerHTML = '<div id="box"></div>';
            const box = document.getElementById('box');
            box.addEventListener('animationstart', e => {
              events.push('a:' + e.type);
              if (replaceFromListener) replaceDocument();
            });
            box.addEventListener('animationend', e => events.push('a:' + e.type));
        "#).unwrap();
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 64)
            .await
            .unwrap();
        if !replace_from_listener {
            {
                let host = vm._context_host.borrow();
                assert!(
                    crate::native_bridge::element::next_animation_transition_deadline(
                        &host,
                        crate::native_bridge::WindowDocumentOwner::Frame(old_owner)
                    )
                    .is_some()
                );
            }
            vm.eval("replaceDocument()").unwrap();
        }
        let expected = if replace_from_listener {
            "a:animationstart|b:animationstart|b:animationend"
        } else {
            "b:animationstart|b:animationend"
        };
        wait_for_css_events(&mut vm, &loader, "events.join('|')", expected).await;
        let host = vm._context_host.borrow();
        assert_ne!(host.current_main_document_task_owner(), Some(old_owner));
        assert!(
            host.animation_transition_state()
                .document(crate::native_bridge::WindowDocumentOwner::Frame(old_owner))
                .is_none()
        );
    }
}

#[tokio::test]
async fn css_lifecycle_iframe_and_parent_deadlines_progress_without_mutations() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/child-deadlines",
        "<html><head></head><body><iframe id=child></iframe></body></html>",
        &loader,
    );
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 64)
        .await
        .unwrap();
    vm.eval(r#"
        globalThis.events = [];
        const child = document.getElementById('child').contentDocument;
        for (const [doc, label] of [[document,'top'],[child,'child']]) {
          doc.head.appendChild(doc.createElement('style')).textContent = '@keyframes move { from { left: 0px; } to { left: 10px; } } .box { position: relative; animation: move 0.04s linear 0.04s; }';
          const box = doc.body.appendChild(doc.createElement('div'));
          box.className = 'box';
          for (const type of ['animationstart','animationend']) box.addEventListener(type, e => events.push(label + ':' + e.type));
        }
    "#).unwrap();
    wait_for_css_events(&mut vm, &loader, "events.length.toString()", "4").await;
    assert_eq!(
        vm.eval("events.slice().sort().join('|')").unwrap(),
        "child:animationend|child:animationstart|top:animationend|top:animationstart"
    );
}

#[tokio::test]
async fn css_lifecycle_completed_animation_is_remembered_until_removed() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/completed",
        r#"<html><head><style>
          @keyframes a { from { left: 0px; } to { left: 10px; } }
          #box { position: relative; }
          #box.active { animation: a 0.02s; }
        </style></head><body><div id=box></div></body></html>"#,
        &loader,
    );
    vm.eval(
        r#"
        globalThis.events = [];
        for (const type of ['animationstart', 'animationend', 'animationcancel']) {
          box.addEventListener(type, e => events.push(e.type));
        }
    "#,
    )
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.eval("box.className = 'active'").unwrap();
    wait_for_css_events(
        &mut vm,
        &loader,
        "events.join('|')",
        "animationstart|animationend",
    )
    .await;
    vm.eval("document.body.className = 'unrelated'; box.style.color = 'red'")
        .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "animationstart|animationend"
    );
    vm.eval("box.className = ''").unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "animationstart|animationend"
    );
    vm.eval("box.className = 'active'").unwrap();
    wait_for_css_events(
        &mut vm,
        &loader,
        "events.join('|')",
        "animationstart|animationend|animationstart|animationend",
    )
    .await;
}
