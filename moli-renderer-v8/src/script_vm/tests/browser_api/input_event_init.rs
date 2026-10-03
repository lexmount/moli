use super::*;

#[test]
fn input_event_node_dispatch_releases_ranges_after_all_listeners() {
    let mut vm = new_storage_html_test_vm("https://input-range-dispatch.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/input-range-dispatch.js"
        ))
        .unwrap(),
        "true"
    );
}

#[test]
fn retained_input_event_does_not_root_live_ranges_after_node_dispatch() {
    let mut vm = new_storage_html_test_vm("https://input-range-gc.test/");
    assert!(vm._context_host.borrow_mut().live_ranges_is_empty());
    vm.eval(
        r#"
      const text=document.createTextNode('abc');document.body.append(text);
      globalThis.retainedInputRangeEvent=new InputEvent('beforeinput',{targetRanges:[
        new StaticRange({startContainer:text,startOffset:0,endContainer:text,endOffset:2})
      ]});
      globalThis.retainedInputRangeSnapshots=retainedInputRangeEvent.getTargetRanges();
    "#,
    )
    .unwrap();
    assert!(!vm._context_host.borrow_mut().live_ranges_is_empty());
    vm.eval("document.body.dispatchEvent(retainedInputRangeEvent)")
        .unwrap();
    for _ in 0..2 {
        vm.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
    }
    assert!(
        vm._context_host.borrow_mut().live_ranges_is_empty(),
        "retaining the event and static snapshots must not retain its internal live Range"
    );
    assert_eq!(vm.eval("retainedInputRangeEvent instanceof InputEvent && retainedInputRangeEvent.getTargetRanges().length===0 && retainedInputRangeSnapshots[0].endOffset===2").unwrap(), "true");
}

#[test]
fn input_event_live_ranges_validate_and_normalize_native_boundaries() {
    let mut vm = new_storage_html_test_vm("https://input-range-boundaries.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/input-range-boundaries.js"
        ))
        .unwrap(),
        "true"
    );
}

#[test]
fn input_and_touch_dictionaries_convert_before_native_sequences_and_range_state() {
    let mut vm = new_storage_html_test_vm("https://input-touch-dictionary.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/input-touch-dictionary.js"
        ))
        .unwrap(),
        "true"
    );
    assert_eq!(vm.eval(r#"(() => {
        const raw = '\ud800\0\udc00';
        if (new InputEvent('audit', {inputType: raw}).inputType !== raw) throw Error('legacy inputType UTF16');
        for (const sentinel of [{}, null, undefined, 7]) {
            const reads = [];
            let caught = Symbol();
            try {new CapturedMouseEvent('audit', {
                get bubbles() {reads.push('bubbles');return false;},
                get cancelable() {reads.push('cancelable');return false;},
                get composed() {reads.push('composed');return false;},
                get surfaceX() {reads.push('surfaceX');throw sentinel;},
                get surfaceY() {throw Error('must stop');}
            });} catch(e) {caught=e;}
            if (caught !== sentinel || reads.join() !== 'bubbles,cancelable,composed,surfaceX') throw Error('CapturedMouse abrupt completion');
        }
        return true;
    })()"#).unwrap(), "true");
}

#[tokio::test]
async fn input_event_dictionary_ranges_and_prototypes_use_native_conversion_and_brands() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://input-event-init.test/",
        &loader,
    );
    vm.eval(
        r#"
        if (!document.documentElement) document.appendChild(document.createElement('html'));
        if (!document.body) document.documentElement.appendChild(document.createElement('body'));
        const frame = document.body.appendChild(document.createElement('iframe'));
        frame.id = 'child'; frame.srcdoc = '<head></head><body></body>'; void frame.contentWindow;
        'ready'
    "#,
    )
    .unwrap();
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::RealmMaterialization,
            &loader
        )
        .await
        .unwrap()
    );
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .unwrap();
    let source = format!(
        "{}; globalThis.__uiEventResults = __inputEventProbe([globalThis, document.getElementById('child').contentWindow]); __uiEventResults.complete",
        include_str!("input_event_init.js"),
    );
    assert_eq!(vm.eval(&source).unwrap(), "true", "{}",
        vm.eval("JSON.stringify({errors:__uiEventResults.errors,failures:__uiEventResults.rows.filter(row=>Object.values(row.checks).some(value=>value!==true))})").unwrap());
}
