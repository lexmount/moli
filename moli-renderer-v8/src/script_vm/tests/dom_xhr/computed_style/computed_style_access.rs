use super::*;

#[test]
fn inspector_runtime_evaluate_uses_child_context_window_surface() {
    let mut vm = new_storage_test_vm("https://inspector-child-window-viewport.test/");
    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.style.width = '300px';
  frame.style.height = '65px';
  frame.src = 'data:text/html,<!doctype html><body>cross-origin child</body>';
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("Inspector child Window viewport setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("Inspector child Window viewport realm should materialize");
    let child_context_ptr = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("Inspector child Window viewport realm record should exist");
        &realm.context as *const v8::Global<v8::Context>
    };
    vm.with_context_scope_by_ptr(child_context_ptr, |scope, _host_ptr| {
        let global = scope.get_current_context().global(scope);
        let function = v8::Function::builder(inspector_active_child_window_scope_callback)
            .build(scope)
            .ok_or_else(|| anyhow::anyhow!("failed to create Inspector owner-scope probe"))?;
        let _ = global.define_own_property(
            scope,
            crate::util::v8str(scope, "__inspectorActiveChildWindowScope").into(),
            function.into(),
            v8::PropertyAttribute::DONT_ENUM,
        );
        Ok(())
    })
    .expect("Inspector child owner-scope probe should install");

    let messages = vm
        .dispatch_inspector_protocol_message(
            &serde_json::json!({
                "id": 41,
                "method": "Runtime.evaluate",
                "params": {
                    "contextId": child_context_id,
                    "expression": r#"(async () => {
                      await Promise.resolve();
                      return [
                        __inspectorActiveChildWindowScope(),
                        innerWidth,
                        innerHeight,
                        matchMedia('(width: 300px)').matches,
                        matchMedia('(prefers-reduced-motion: no-preference)').matches
                      ].join('|');
                    })()"#,
                    "awaitPromise": true,
                    "returnByValue": true
                }
            })
            .to_string(),
        )
        .expect("Runtime.evaluate should dispatch to the child execution context");
    let response = messages
        .iter()
        .find(|message| message["id"] == serde_json::json!(41))
        .expect("Runtime.evaluate should return a response");

    assert_eq!(
        response["result"]["result"]["value"],
        serde_json::json!("true|300|65|true|true"),
        "Inspector dispatch must establish the same child browsing-context owner scope as page script execution"
    );

    let top_result = vm
        .eval("[innerWidth, innerHeight].join('|')")
        .expect("top Window viewport should remain top-level after Inspector child evaluation");
    assert_eq!(top_result, "1920|1080");
}

#[test]
fn inspector_default_runtime_evaluate_masks_ambient_child_owner_scope() {
    let mut vm = new_storage_test_vm("https://inspector-top-window-viewport.test/");
    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.style.width = '300px';
  frame.style.height = '65px';
  frame.src = 'data:text/html,<!doctype html><body>child owner</body>';
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("Inspector top Window viewport setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("Inspector child owner realm should materialize");
    let child_handle = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("Inspector child owner realm record should exist")
        .child_handle;
    let top_context_ptr = &vm.page_default_context as *const v8::Global<v8::Context>;
    vm.with_context_scope_by_ptr(top_context_ptr, |scope, _host_ptr| {
        let _previous =
            crate::native_bridge::enter_active_child_window_scope(scope, Some(child_handle));
        Ok(())
    })
    .expect("ambient child owner scope should install");

    let messages = vm
        .dispatch_inspector_protocol_message(
            &serde_json::json!({
                "id": 42,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "[innerWidth, innerHeight].join('|')",
                    "returnByValue": true
                }
            })
            .to_string(),
        )
        .expect("default Runtime.evaluate should dispatch");
    let response = messages
        .iter()
        .find(|message| message["id"] == serde_json::json!(42))
        .expect("default Runtime.evaluate should return a response");
    assert_eq!(
        response["result"]["result"]["value"],
        serde_json::json!("1920|1080"),
        "the default Inspector realm must mask an unrelated ambient child owner"
    );

    let restored = vm
        .with_context_scope_by_ptr(top_context_ptr, |scope, _host_ptr| {
            let restored = crate::native_bridge::active_child_window_handle(scope);
            let _previous = crate::native_bridge::enter_active_child_window_scope(scope, None);
            Ok(restored)
        })
        .expect("ambient child owner scope should be inspectable");
    assert_eq!(restored, Some(child_handle));
}

#[test]
fn computed_white_space_resolves_inherited_custom_property_changes() {
    let mut vm = new_storage_test_vm("https://custom-property-white-space.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '.inner { white-space: var(--x); }';
  head.appendChild(style);
  body.innerHTML = '<div id="outer"><div id="inbetween"><div id="inner" class="inner"></div></div></div>';
  const outer = document.getElementById('outer');
  const inbetween = document.getElementById('inbetween');
  const inner = document.getElementById('inner');

  outer.style.cssText = '--x: pre';
  const inherited = getComputedStyle(inner).whiteSpace;
  inbetween.style.cssText = '--x: pre-wrap';
  const overridden = getComputedStyle(inner).whiteSpace;
  inbetween.style.cssText = '';
  outer.style.cssText = '--x: nowrap';
  const changed = getComputedStyle(inner).whiteSpace;
  return `${inherited}|${overridden}|${changed}`;
})()
"#,
        )
        .expect("white-space should resolve inherited custom property changes");

    assert_eq!(result, "pre|pre-wrap|nowrap");
}

#[test]
fn child_frame_computed_style_wrapper_reuses_target_context_between_property_reads() {
    let mut vm = new_storage_test_vm("https://child-computed-style-retention.test/");
    let document = vm.document_handle_for_test();

    let setup = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  const childDocument = frame.contentWindow.document;
  childDocument.open();
  childDocument.write('<style>body { color: rgb(0, 128, 0); background-color: rgb(1, 2, 3); }</style><body>text</body>');
  childDocument.close();
  globalThis.__childFrameComputedStyle = frame.contentWindow.getComputedStyle(childDocument.body);
  return globalThis.__childFrameComputedStyle.color;
})()
"#,
        )
        .expect("child frame computed style setup should evaluate");

    assert_eq!(setup, "rgb(0, 128, 0)");
    let generation_after_setup = vm.computed_style_cache_generation_for_document_for_test(document);

    let result = vm
        .eval(
            r#"
(() => {
  const style = globalThis.__childFrameComputedStyle;
  return [
    style.getPropertyValue('color'),
    style.getPropertyValue('background-color'),
    style.getPropertyValue('display')
  ].join('|');
})()
"#,
        )
        .expect("child frame computed style property reads should evaluate");
    let generation_after_reads = vm.computed_style_cache_generation_for_document_for_test(document);

    assert_eq!(result, "rgb(0, 128, 0)|rgb(1, 2, 3)|block");
    assert_eq!(generation_after_reads, generation_after_setup);
}

#[test]
fn child_frame_held_computed_style_tracks_iframe_render_state_changes() {
    let mut vm = new_storage_test_vm("https://child-computed-style-visibility.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  const childDocument = frame.contentWindow.document;
  childDocument.open();
  childDocument.write('<style>body { color: rgb(0, 128, 0); }</style><body>text</body>');
  childDocument.close();
  const style = frame.contentWindow.getComputedStyle(childDocument.body);
  const visible = `${style.length > 0}:${style.color}`;
  frame.style.display = 'none';
  const hidden = `${style.length}:${style.color}`;
  frame.style.display = 'block';
  const shown = `${style.length > 0}:${style.color}`;
  return `${visible}|${hidden}|${shown}`;
})()
"#,
        )
        .expect("held child frame computed style should track iframe render state");

    assert_eq!(result, "true:rgb(0, 128, 0)|0:|true:rgb(0, 128, 0)");
}

#[test]
fn child_frame_held_computed_style_tracks_iframe_viewport_changes() {
    let mut vm = new_storage_test_vm("https://child-computed-style-viewport.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  frame.style.width = '100px';
  (document.body || document.documentElement || document).appendChild(frame);
  const childDocument = frame.contentWindow.document;
  childDocument.open();
  childDocument.write('<style>body { color: rgb(255, 0, 0); } @media all and (min-width: 150px) { body { color: rgb(0, 128, 0); } }</style><body>text</body>');
  childDocument.close();
  const style = frame.contentWindow.getComputedStyle(childDocument.body);
  const before = style.color;
  frame.style.width = '200px';
  const after = style.color;
  return `${before}|${after}`;
})()
"#,
        )
        .expect("held child frame computed style should track iframe viewport changes");

    assert_eq!(result, "rgb(255, 0, 0)|rgb(0, 128, 0)");
}

#[test]
fn removed_iframe_clears_child_document_computed_style_cache() {
    let mut vm = new_storage_test_vm("https://child-computed-style-removal.test/");
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const body = document.body || document.documentElement || document;
  globalThis.__styleCacheFrame = document.createElement('iframe');
  globalThis.__styleCacheFrame.id = 'style-cache-reattach-frame';
  body.appendChild(globalThis.__styleCacheFrame);
  const childDocument = globalThis.__styleCacheFrame.contentWindow.document;
  childDocument.open();
  childDocument.write('<style>body { color: rgb(0, 128, 0); }</style><body>text</body>');
  childDocument.close();
  return globalThis.__styleCacheFrame.contentWindow.getComputedStyle(childDocument.body).color;
})()
"#,
        )
        .expect("child frame computed style setup should evaluate");

    assert_eq!(initial, "rgb(0, 128, 0)");
    let retired_child_document =
        child_document_handle_for_frame_id(&vm, "style-cache-reattach-frame");
    assert!(vm.document_style_world_is_active_for_test(retired_child_document));
    assert!(vm.computed_style_cache_entry_count_for_document_for_test(document) > 0);

    let removed = vm
        .eval(
            r#"
(() => {
  globalThis.__styleCacheFrame.remove();
  return String(globalThis.__styleCacheFrame.contentWindow === null);
})()
"#,
        )
        .expect("child frame removal should evaluate");

    assert_eq!(removed, "true");
    assert!(!vm.document_style_world_is_active_for_test(retired_child_document));
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );

    let reattached = vm
        .eval(
            r#"
(() => {
  (document.body || document.documentElement || document).appendChild(globalThis.__styleCacheFrame);
  const childDocument = globalThis.__styleCacheFrame.contentWindow.document;
  childDocument.open();
  childDocument.write('<style>body { color: rgb(1, 2, 3); }</style><body>text</body>');
  childDocument.close();
  const color = globalThis.__styleCacheFrame.contentWindow.getComputedStyle(childDocument.body).color;
  delete globalThis.__styleCacheFrame;
  return color;
})()
"#,
        )
        .expect("reattached child frame computed style should evaluate");

    assert_eq!(reattached, "rgb(1, 2, 3)");
    let child_document = child_document_handle_for_frame_id(&vm, "style-cache-reattach-frame");
    assert!(computed_style_cache_entry_count_for_document(&vm, child_document) > 0);
}

#[test]
fn held_child_frame_computed_style_is_empty_after_iframe_removal() {
    let mut vm = new_storage_test_vm("https://held-child-computed-style-removal.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const body = document.body || document.documentElement || document;
  const frame = document.createElement('iframe');
  body.appendChild(frame);
  let childDocument = frame.contentWindow.document;
  childDocument.open();
  childDocument.write('<style>body { color: rgb(0, 128, 0); }</style><body>text</body>');
  childDocument.close();

  const held = frame.contentWindow.getComputedStyle(childDocument.body);
  const before = `${held.length > 0}:${held.color}`;
  frame.remove();
  const removed = `${held.length}:${held.color}`;

  body.appendChild(frame);
  childDocument = frame.contentWindow.document;
  childDocument.open();
  childDocument.write('<style>body { color: rgb(1, 2, 3); }</style><body>next</body>');
  childDocument.close();
  const oldHeld = `${held.length}:${held.color}`;
  const fresh = frame.contentWindow.getComputedStyle(childDocument.body).color;
  return `${before}|${removed}|${oldHeld}|${fresh}`;
})()
"#,
        )
        .expect("held child frame computed style removal should evaluate");

    assert_eq!(result, "true:rgb(0, 128, 0)|0:|0:|rgb(1, 2, 3)");
}

#[test]
fn held_child_frame_computed_style_is_empty_after_srcdoc_navigation() {
    let mut vm = new_storage_test_vm("https://held-child-computed-style-srcdoc.test/");
    let document = vm.document_handle_for_test();

    vm.eval(
        r#"
(() => {
  const body = document.body || document.documentElement || document;
  const frame = document.createElement('iframe');
  frame.id = 'held-srcdoc-frame';
  frame.srcdoc = '<style>body { color: rgb(0, 128, 0); }</style><body>first</body>';
  body.appendChild(frame);
})()
"#,
    )
    .expect("first child srcdoc navigation should queue");
    assert!(
        vm.run_next_child_navigation_commit_body_for_test()
            .expect("typed child navigation-commit body should succeed")
            .is_some(),
        "the first srcdoc Document must commit on its own owner turn"
    );

    let before_replacement = vm
        .eval(
            r#"
(() => {
  const frame = document.getElementById('held-srcdoc-frame');
  const firstDocument = frame.contentDocument;
  const held = frame.contentWindow.getComputedStyle(firstDocument.body);
  globalThis.__heldSrcdocFirstDocument = firstDocument;
  globalThis.__heldSrcdocComputedStyle = held;
  const before = `${held.length > 0}:${held.color}`;

  let symbolError = 'none';
  try {
    frame.srcdoc = Symbol('srcdoc');
  } catch (error) {
    symbolError = error.name;
  }
  const afterFailedSrcdoc =
    `${symbolError}:${held.length > 0}:${held.color}:${frame.contentDocument === firstDocument}`;

  let srcSymbolError = 'none';
  try {
    frame.src = Symbol('src');
  } catch (error) {
    srcSymbolError = error.name;
  }
  const afterFailedSrc =
    `${srcSymbolError}:${held.length > 0}:${held.color}:${frame.contentDocument === firstDocument}`;

  frame.srcdoc = '<style>body { color: rgb(1, 2, 3); }</style><body>second</body>';
  return `${before}|${afterFailedSrcdoc}|${afterFailedSrc}`;
})()
"#,
        )
        .expect("held child frame pre-replacement state should evaluate");
    assert!(
        vm.run_next_child_navigation_commit_body_for_test()
            .expect("typed child navigation-commit body should succeed")
            .is_some(),
        "the replacement srcdoc Document must commit on a later owner turn"
    );
    let after_replacement = vm
        .eval(
            r#"
(() => {
  const frame = document.getElementById('held-srcdoc-frame');
  const held = globalThis.__heldSrcdocComputedStyle;
  const firstDocument = globalThis.__heldSrcdocFirstDocument;
  const oldHeld = `${held.length}:${held.color}`;
  const secondDocument = frame.contentDocument;
  const fresh = frame.contentWindow.getComputedStyle(secondDocument.body).color;
  return `${oldHeld}|${fresh}|${firstDocument === secondDocument}`;
})()
"#,
        )
        .expect("held child frame post-replacement state should evaluate");

    assert_eq!(
        format!("{before_replacement}|{after_replacement}"),
        "true:rgb(0, 128, 0)|TypeError:true:rgb(0, 128, 0):true|TypeError:true:rgb(0, 128, 0):true|0:|rgb(1, 2, 3)|false"
    );
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
}

#[test]
fn repeated_iframe_navigation_retires_each_previous_document_style_world() {
    let mut vm = new_storage_test_vm("https://child-style-world-retirement.test/");
    let main_document = vm.document_handle_for_test();

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.id = 'style-world-churn-frame';
  frame.srcdoc = '<style>body { color: rgb(0, 128, 0); }</style><body data-generation="0">first</body>';
  (document.body || document.documentElement || document).appendChild(frame);
  getComputedStyle(document.documentElement).display;
})()
"#,
    )
    .expect("initial child style-world navigation should queue");
    assert!(
        vm.run_next_child_navigation_commit_body_for_test()
            .expect("initial child navigation-commit body should succeed")
            .is_some()
    );
    assert_eq!(
        vm.eval(
            r#"
(() => {
  const frame = document.getElementById('style-world-churn-frame');
  globalThis.__heldChurnStyle = frame.contentWindow.getComputedStyle(frame.contentDocument.body);
  return globalThis.__heldChurnStyle.color;
})()
"#,
        )
        .expect("initial child style should compute"),
        "rgb(0, 128, 0)"
    );

    let mut previous_document = child_document_handle_for_frame_id(&vm, "style-world-churn-frame");
    assert!(vm.document_style_world_is_active_for_test(main_document));
    assert!(vm.document_style_world_is_active_for_test(previous_document));
    let steady_active_worlds = vm.active_document_style_world_count_for_test();
    assert_eq!(steady_active_worlds, 2);

    for generation in 1..=8 {
        let navigation = format!(
            r#"
(() => {{
  const frame = document.getElementById('style-world-churn-frame');
  globalThis.__heldChurnStyle = frame.contentWindow.getComputedStyle(frame.contentDocument.body);
  frame.srcdoc = '<body data-generation="{generation}">next</body>';
}})()
"#
        );
        vm.eval(&navigation)
            .expect("replacement child style-world navigation should queue");
        assert!(
            vm.run_next_child_navigation_commit_body_for_test()
                .expect("replacement child navigation-commit body should succeed")
                .is_some()
        );

        let current_document = child_document_handle_for_frame_id(&vm, "style-world-churn-frame");
        assert_ne!(current_document, previous_document);
        assert!(
            !vm.document_style_world_is_active_for_test(previous_document),
            "the previous child Document must leave the active style-world map at commit"
        );
        assert_eq!(
            vm.eval(
                r#"
(() => {
  const frame = document.getElementById('style-world-churn-frame');
  const held = globalThis.__heldChurnStyle;
  const stale = `${held.length}:${held.color}`;
  const fresh = frame.contentWindow.getComputedStyle(frame.contentDocument.body).display;
  return `${stale}|${fresh}|${frame.contentDocument.body.dataset.generation}`;
})()
"#,
            )
            .expect("stale and current child styles should remain distinguishable"),
            format!("0:|block|{generation}")
        );
        assert!(
            !vm.document_style_world_is_active_for_test(previous_document),
            "reading a held stale declaration must not recreate its retired style world"
        );
        assert!(vm.document_style_world_is_active_for_test(current_document));
        assert_eq!(
            vm.active_document_style_world_count_for_test(),
            steady_active_worlds,
            "iframe navigation churn must keep heavyweight style worlds bounded"
        );
        previous_document = current_document;
    }
}

#[test]
fn computed_style_custom_property_names_use_iframe_viewport() {
    let mut vm = new_storage_test_vm("https://child-custom-property-names.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  frame.style.width = '100px';
  (document.body || document.documentElement || document).appendChild(frame);
  const childDocument = frame.contentWindow.document;
  childDocument.open();
  childDocument.write('<style>@media all and (min-width: 150px) { body { --wide-name: yes; } }</style><body>text</body>');
  childDocument.close();
  const hasName = () => {
    const style = getComputedStyle(childDocument.body);
    return Array.from({ length: style.length }, (_, index) => style.item(index)).includes('--wide-name');
  };
  const before = hasName();
  frame.style.width = '200px';
  const after = hasName();
  return `${before}|${after}`;
})()
"#,
        )
        .expect("computed custom property names should use iframe viewport");

    assert_eq!(result, "false|true");
}

#[test]
fn computed_font_family_serializes_css_fonts_generic_functions() {
    let mut vm = new_storage_test_vm("https://font-family-generic-functions.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  (document.body || document.documentElement || document).appendChild(target);
  const computed = (value) => {
    target.style.fontFamily = value;
    return getComputedStyle(target).fontFamily;
  };
  return [
    computed('generic(fangsong)'),
    computed('-webkit-generic(fangsong)'),
    computed('"Times New Roman"'),
    computed('"34J"')
  ].join('|');
})()
"#,
        )
        .expect("computed font-family serialization should evaluate");

    assert_eq!(
        result,
        r#"generic(fangsong)|-webkit-generic(fangsong)|"Times New Roman"|"34J""#
    );
}

#[test]
fn computed_width_preserves_values_without_used_width() {
    let mut vm = new_storage_test_vm("https://computed-width-no-used-value.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  body.innerHTML = `
    <div style="width: 200px">
      <span id="inline"></span>
      <div id="none" style="display:none"></div>
      <div id="contents" style="display:contents"></div>
    </div>`;
  return ['inline', 'none', 'contents'].map((id) => {
    const target = document.getElementById(id);
    target.style.minWidth = '10px';
    target.style.maxWidth = '50px';
    return ['10%', '1px', '60px'].map((width) => {
      target.style.width = width;
      return getComputedStyle(target).width;
    }).join('/');
  }).join('|');
})()
"#,
        )
        .expect("computed width should preserve values without a used width");

    assert_eq!(result, "10%/1px/60px|10%/1px/60px|10%/1px/60px");
}

#[test]
fn box_metrics_use_computed_display_and_real_used_sizes() {
    let mut vm = new_storage_test_vm("https://box-metrics-computed-style.test/");

    vm.eval(
        r#"
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = `
    .test div { width: 50px; height: 30px; }
    #hidden:lang(xx) { display: none; }
    #matched[lang|='es'] { width: 80px; height: 40px; }
  `;
  head.appendChild(style);
  body.innerHTML = `
    <p id="hidden" lang="xx">hidden</p>
    <div class="test">
      <div id="matched" lang="es-MX"></div>
      <div id="unmatched" lang="mx-es"></div>
    </div>
  `;
  const hidden = document.getElementById('hidden');
  const matched = document.getElementById('matched');
  const unmatched = document.getElementById('unmatched');
"#,
    )
    .expect("prepare geometry fixture");
    publish_layout_for_test(&mut vm);
    let result = vm
        .eval(
            r#"[
    getComputedStyle(hidden).display,
    hidden.offsetWidth,
    hidden.offsetHeight,
    hidden.getClientRects().length,
    getComputedStyle(matched).width,
    matched.offsetWidth,
    getComputedStyle(matched).height,
    matched.offsetHeight,
    matched.getBoundingClientRect().width,
    matched.getBoundingClientRect().height,
    matched.getClientRects().length,
    getComputedStyle(unmatched).width,
    unmatched.offsetWidth,
    getComputedStyle(unmatched).height,
    unmatched.offsetHeight,
    unmatched.getBoundingClientRect().width
  ].join('|')"#,
        )
        .expect("box metrics should consume simple computed style facts");

    assert_eq!(
        result,
        "none|0|0|0|80px|80|40px|40|80|40|1|50px|50|30px|30|50"
    );
}

#[test]
fn box_metric_reads_published_nested_geometry_without_updating_styles() {
    let mut vm = new_storage_test_vm("https://box-metric-style-snapshot.test/");

    vm.eval(
        r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '#outer { width: 200px; } #target { width: 50%; }';
  head.appendChild(style);
  const outer = document.createElement('div');
  outer.id = 'outer';
  let parent = outer;
  for (let index = 0; index < 64; index++) {
    const layer = document.createElement('div');
    parent.appendChild(layer);
    parent = layer;
  }
  const target = document.createElement('div');
  target.id = 'target';
  parent.appendChild(target);
  body.appendChild(outer);
  return 'ready';
})()
"#,
    )
    .expect("nested box-metric fixture should initialize");
    publish_layout_for_test(&mut vm);
    let update_materializations_before = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();

    let result = vm
        .eval("String(document.getElementById('target').offsetWidth)")
        .expect("box metric should resolve a nested percentage width");

    assert_eq!(result, "100");
    let update_materializations = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test()
        .saturating_sub(update_materializations_before);
    assert_eq!(
        update_materializations, 0,
        "a geometry read must not materialize a style-world update"
    );
}

#[test]
fn unrelated_inline_property_read_skips_logical_inset_direction_resolution() {
    let mut vm = new_storage_test_vm("https://inline-logical-inset-gate.test/");

    vm.eval(
        r#"
const html = document.documentElement || document.appendChild(document.createElement('html'));
const body = document.body || html.appendChild(document.createElement('body'));
const target = document.createElement('div');
target.id = 'target';
target.style.color = 'red';
body.appendChild(target);
"#,
    )
    .expect("inline-style fixture should initialize");
    let update_materializations_before = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();

    let result = vm
        .eval("document.getElementById('target').style.fontSize")
        .expect("missing inline font-size should remain readable");

    assert_eq!(result, "");
    let update_materializations = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test()
        .saturating_sub(update_materializations_before);
    assert_eq!(
        update_materializations, 0,
        "a non-logical inline property must not resolve writing mode or direction"
    );
}

#[test]
fn box_metrics_round_fractional_computed_px_values() {
    let mut vm = new_storage_test_vm("https://box-metrics-fractional-style.test/");

    vm.eval(
        r#"
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = `
    #fractional { width: 79.6px; height: 30.6px; }
  `;
  head.appendChild(style);
  body.innerHTML = `<div id="fractional"></div>`;
  const fractional = document.getElementById('fractional');
"#,
    )
    .expect("prepare geometry fixture");
    publish_layout_for_test(&mut vm);
    let result = vm
        .eval(
            r#"[
    fractional.offsetWidth,
    fractional.clientWidth,
    fractional.scrollWidth,
    fractional.offsetHeight,
    fractional.clientHeight,
    fractional.scrollHeight,
    fractional.getBoundingClientRect().width,
    fractional.getBoundingClientRect().height
  ].join('|')"#,
        )
        .expect("box metrics should round fractional computed px values");

    assert_eq!(result, "80|80|80|31|31|31|79.59375|30.59375");
}

#[test]
fn layout_resolves_a_deep_percentage_width_chain_to_zero() {
    let mut vm = new_storage_test_vm("https://mock-geometry-inline-width-chain.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  body.innerHTML = '';
  let parent = body;
  for (let i = 0; i < 40; i++) {
    const next = document.createElement('div');
    next.style.width = '50%';
    parent.appendChild(next);
    parent = next;
  }
  return String(parent.offsetWidth);
})()
"#,
        )
        .expect("layout inline percentage width probe should evaluate");

    assert_eq!(result, "0");
}

#[test]
fn document_open_replacement_clears_inline_style_state() {
    let mut vm = new_storage_test_vm("https://document-open-style-state.test/");

    vm.eval("const fixtureRoot = document.documentElement || document.appendChild(document.createElement('html')); const fixtureBody = document.body || fixtureRoot.appendChild(document.createElement('body')); fixtureBody.innerHTML = '<div id=before style=display:block>before</div>'").expect("prepare replacement fixture");
    publish_layout_for_test(&mut vm);
    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));

  const before = document.getElementById('before');
  before.style.display = 'block';
  const beforeStyle = before.style;
  const beforeComputed = getComputedStyle(before);
  const warmed = `${getComputedStyle(before).display}:${before.getClientRects().length}`;
  document.open();
  document.write("<!doctype html><html><body><div id='after' style='display:none'>after</div></body></html>");
  document.close();
  const after = document.getElementById('after');
  const afterStyle = after.style;
  const afterComputed = getComputedStyle(after);
  return `${warmed}|${before === after}:${beforeStyle === afterStyle}:${beforeComputed === afterComputed}|${afterStyle.display}:${afterComputed.display}:${after.getClientRects().length}:${after.offsetWidth}`;
})()
"#,
        )
        .expect("document replacement inline style state should evaluate");

    assert_eq!(result, "block:1|false:false:false|none:none:0:0");
}

#[test]
fn isolated_document_open_replacement_clears_inline_style_state() {
    let mut vm = new_storage_test_vm("https://isolated-document-open-style-state.test/");
    let context_id = vm
        .create_isolated_world("playwright-utility-replacement", false)
        .expect("isolated world should be created");

    vm.eval("const fixtureRoot = document.documentElement || document.appendChild(document.createElement('html')); const fixtureBody = document.body || fixtureRoot.appendChild(document.createElement('body')); fixtureBody.innerHTML = '<div id=before style=display:block>before</div>'").expect("prepare replacement fixture");
    publish_layout_for_test(&mut vm);
    let result = vm
        .eval_in_isolated_context(
            context_id,
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));

  const before = document.getElementById('before');
  before.style.display = 'block';
  const beforeStyle = before.style;
  const beforeComputed = getComputedStyle(before);
  const warmed = `${beforeComputed.display}:${before.getClientRects().length}`;
  document.open();
  console.debug('--moli-set-content--');
  document.write("<!doctype html><html><body><div id='after' style='display:none'>after</div></body></html>");
  document.close();
  const after = document.getElementById('after');
  const afterStyle = after.style;
  const afterComputed = getComputedStyle(after);
  return `${warmed}|${before === after}:${beforeStyle === afterStyle}:${beforeComputed === afterComputed}|${afterStyle.display}:${afterComputed.display}:${after.getClientRects().length}:${after.offsetWidth}`;
})()
"#,
        )
        .expect("isolated document replacement inline style state should evaluate");

    assert_eq!(result, "block:1|false:false:false|none:none:0:0");
}

#[tokio::test]
async fn document_open_replacement_clears_timer_mutated_inline_style_state() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm("https://document-open-timer-style-state.test/");

    vm.exec(
        r#"
const html = document.documentElement || document.appendChild(document.createElement('html'));
const body = document.body || html.appendChild(document.createElement('body'));
body.innerHTML = "<div id='before' style='display:none'>before</div>";
setTimeout(() => { document.getElementById('before').style.display = 'block'; }, 0);
"#,
        None,
    )
    .expect("initial document replacement should run");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("timer style mutation should drain");

    publish_layout_for_test(&mut vm);
    let warmed = vm
        .eval(
            r#"
(() => {
  const before = document.getElementById('before');
  return `${before.style.display}:${getComputedStyle(before).display}:${before.getClientRects().length}`;
})()
"#,
        )
        .expect("timer-mutated style should be observable");
    assert_eq!(warmed, "block:block:1");

    let result = vm
        .eval(
            r#"
(() => {
  document.open();
  document.write("<!doctype html><html><body><div id='after' style='display:none'>after</div></body></html>");
  document.close();
  const after = document.getElementById('after');
  return `${after.getAttribute('style')}:${after.style.display}:${getComputedStyle(after).display}:${after.getClientRects().length}:${after.offsetWidth}`;
})()
"#,
        )
        .expect("replacement after timer style mutation should evaluate");

    assert_eq!(result, "display:none:none:none:0:0");
}

#[test]
fn computed_display_treats_hidden_attribute_as_none() {
    let mut vm = new_storage_test_vm("https://hidden-computed-display.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.createElement('div');
  host.id = 'host';
  host.hidden = true;
  host.style.display = 'block';
  const child = document.createElement('span');
  child.id = 'child';
  host.appendChild(child);
  (document.body || document.documentElement || document).appendChild(host);
  return [
    host.hidden,
    getComputedStyle(host).display,
    getComputedStyle(child).display,
    host.getClientRects().length,
    host.offsetWidth
  ].join('|');
})()
"#,
        )
        .expect("hidden attribute should influence computed display");

    assert_eq!(result, "true|none|inline|0|0");
}

#[test]
fn computed_style_distinguishes_hidden_and_until_found_states() {
    let mut vm = new_storage_test_vm("https://hidden-until-found-computed-style.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  (document.body || document.documentElement || document).appendChild(target);
  const read = () => [
    getComputedStyle(target).display,
    getComputedStyle(target).contentVisibility
  ].join(':');
  const values = [read()];
  for (const value of ['', 'asdf', 'until-found', 'UNTIL-FOUND', 'UnTiL-FoUnD', '0']) {
    target.setAttribute('hidden', value);
    values.push(read());
  }
  target.setAttribute('hidden', 'until-found');
  target.style.contentVisibility = 'visible';
  values.push(`${target.style.contentVisibility}/${read()}`);
  target.style.removeProperty('content-visibility');
  values.push(read());
  target.removeAttribute('hidden');
  target.style.contentVisibility = 'hidden';
  values.push(`${target.style.contentVisibility}/${read()}`);
  target.style.contentVisibility = 'bogus';
  values.push(`${target.style.contentVisibility}/${read()}`);
  return values.join('|');
})()
"#,
        )
        .expect("hidden presentation states should affect computed style");

    assert_eq!(
        result,
        "block:visible|none:visible|none:visible|block:hidden|block:hidden|block:hidden|none:visible|visible/block:visible|block:hidden|hidden/block:hidden|hidden/block:hidden"
    );
}

#[test]
fn detached_nested_iframe_window_get_computed_style_uses_iframe_width() {
    let mut vm = new_storage_test_vm("https://nested-computed-width.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const appendTarget = document.body || document.documentElement || document;
  const outer = document.createElement('iframe');
  outer.setAttribute('width', '100');
  appendTarget.appendChild(outer);
  const outerDocument = outer.contentWindow.document;
  outerDocument.open();
  outerDocument.write('<body style="margin:0"><iframe id="inner" style="width:100%"></iframe>');
  outerDocument.close();
  const innerWindow = outerDocument.querySelector('#inner').contentWindow;
  const innerDocument = innerWindow.document;
  innerDocument.open();
  innerDocument.write('<body style="margin:0"><div style="width:100%"></div>');
  innerDocument.close();
  const target = innerDocument.querySelector('div');
  const descriptor = Object.getOwnPropertyDescriptor(innerWindow, 'getComputedStyle');
  const value = descriptor && descriptor.value;
  const shape = [
    typeof value,
    value && value.name,
    value && value.length,
    descriptor && descriptor.enumerable,
    descriptor && descriptor.configurable,
    descriptor && descriptor.writable,
    /\[native code\]/.test(String(value))
  ].join(':');
  const before = innerWindow.getComputedStyle(target).width;
  outer.setAttribute('width', '200');
  const after = innerWindow.getComputedStyle(target).width;
  return `${before}|${after}|${shape}`;
})()
"#,
        )
        .expect("nested detached iframe window should expose getComputedStyle");

    assert_eq!(
        result,
        "100px|200px|function:getComputedStyle:1:true:true:true:true"
    );
}

#[test]
fn detached_iframe_computed_style_keeps_pseudo_and_target_identity() {
    let mut vm = new_storage_test_vm("https://detached-iframe-computed-pseudo.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const detachedDocument = document.implementation.createHTMLDocument('');
  const frame = detachedDocument.createElement('iframe');
  frame.setAttribute('width', '220');
  frame.srcdoc = `
    <style>
      body { color: rgb(255, 0, 0); background-color: rgb(1, 2, 3); }
      body::before { content: "x"; color: rgb(0, 128, 0); }
      @media all and (min-width: 200px) {
        body::after { content: "y"; color: rgb(0, 0, 255); }
      }
    </style>
    <body>text</body>`;
  detachedDocument.body.appendChild(frame);

  const childDocument = frame.contentDocument;
  const target = childDocument.body;
  const origin = frame.contentWindow.getComputedStyle(target);
  const before = frame.contentWindow.getComputedStyle(target, '::before');
  const after = frame.contentWindow.getComputedStyle(target, '::after');
  const highlight = frame.contentWindow.getComputedStyle(target, '::highlight(name)');
  const topOrigin = getComputedStyle(target);
  return [
    origin.color,
    origin.backgroundColor,
    before.color,
    after.color,
    highlight.length > 200,
    highlight.color,
    topOrigin.length,
    origin.color,
    origin.backgroundColor,
    before.color,
    after.color,
    highlight.length > 200
  ].join('|');
})()
"#,
        )
        .expect("detached iframe computed pseudo style should evaluate");

    assert_eq!(
        result,
        "rgb(255, 0, 0)|rgb(1, 2, 3)|rgb(0, 128, 0)|rgb(0, 0, 255)|true|rgb(255, 0, 0)|0|rgb(255, 0, 0)|rgb(1, 2, 3)|rgb(0, 128, 0)|rgb(0, 0, 255)|true"
    );
}

#[test]
fn held_detached_iframe_computed_style_uses_target_owner_sources_after_adoption() {
    let mut vm = new_storage_test_vm("https://detached-iframe-held-owner-source.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));

  CSS.registerProperty({
    name: '--held-length',
    syntax: '<length>',
    initialValue: '33px',
    inherits: false
  });
  const activeStyle = document.createElement('style');
  activeStyle.textContent = `
    #cross-doc-target {
      color: rgb(1, 2, 3);
      background-color: rgb(7, 8, 9);
      width: var(--held-length);
    }
  `;
  head.appendChild(activeStyle);

  const detachedDocument = document.implementation.createHTMLDocument('');
  const frame = detachedDocument.createElement('iframe');
  frame.srcdoc = `
    <style>
      #cross-doc-target {
        color: rgb(255, 0, 0);
        background-color: rgb(5, 6, 7);
        width: 77px;
      }
    </style>
    <body><div id="cross-doc-target">x</div></body>`;
  detachedDocument.body.appendChild(frame);

  const childWindow = frame.contentWindow;
  const childDocument = frame.contentDocument;

  const target = childDocument.getElementById('cross-doc-target');
  const held = childWindow.getComputedStyle(target);
  const before = [held.color, held.backgroundColor, held.width].join(',');

  body.appendChild(target);
  const after = [
    target.ownerDocument === document,
    held.color,
    held.backgroundColor,
    held.width,
    getComputedStyle(target).color,
    getComputedStyle(target).width
  ].join(',');
  return `${before}|${after}`;
})()
"#,
        )
        .expect("held detached iframe computed style should use target owner sources");

    assert_eq!(
        result,
        "rgb(255, 0, 0),rgb(5, 6, 7),77px|true,rgb(1, 2, 3),rgb(7, 8, 9),33px,rgb(1, 2, 3),33px"
    );
}

#[test]
fn held_detached_iframe_custom_property_animation_uses_target_owner_registry_after_adoption() {
    let mut vm = new_storage_test_vm("https://detached-iframe-held-custom-animation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));

  CSS.registerProperty({
    name: '--held-animated-length',
    syntax: '<length>',
    initialValue: '100px',
    inherits: false
  });
  const activeStyle = document.createElement('style');
  activeStyle.textContent = `
    @keyframes heldPulse {
      from { --held-animated-length: 10px; }
      to { --held-animated-length: 30px; }
    }
    #held-animation-target {
      --held-animated-length: 0px;
      animation: heldPulse 10s -5s linear paused;
    }
  `;
  head.appendChild(activeStyle);

  const detachedDocument = document.implementation.createHTMLDocument('');
  const frame = detachedDocument.createElement('iframe');
  frame.srcdoc = '<body><div id="held-animation-target">x</div></body>';
  detachedDocument.body.appendChild(frame);

  const childWindow = frame.contentWindow;
  const childDocument = childWindow.document;

  const target = childDocument.getElementById('held-animation-target');
  const held = childWindow.getComputedStyle(target);

  body.appendChild(target);
  return [
    target.ownerDocument === document,
    held.getPropertyValue('--held-animated-length'),
    getComputedStyle(target).getPropertyValue('--held-animated-length')
  ].join('|');
})()
"#,
        )
        .expect("held detached iframe custom property animation should use target owner registry");

    assert_eq!(result, "true|20px|20px");
}

#[test]
fn detached_iframe_attribute_mutation_targets_computed_style_cache() {
    let mut vm = new_storage_test_vm("https://detached-iframe-attribute-style-cache.test/");

    let before = vm
        .eval(
            r#"
(() => {
  const detachedDocument = document.implementation.createHTMLDocument('');
  const frame = detachedDocument.createElement('iframe');
  frame.id = 'detached-attribute-style-frame';
  frame.srcdoc = `
    <style>
      #detached-attribute-outside { color: rgb(1, 2, 3); }
      #detached-attribute-target { color: rgb(7, 8, 9); }
      #detached-attribute-target[data-state="active"] { color: rgb(4, 5, 6); }
    </style>
    <body>
      <section><div id="detached-attribute-outside">outside</div></section>
      <section><div id="detached-attribute-target">target</div></section>
    </body>`;
  detachedDocument.body.appendChild(frame);

  const childDocument = frame.contentDocument;
  globalThis.__detachedAttributeStyleFrame = frame;
  globalThis.__detachedAttributeStyleOutside = childDocument.getElementById('detached-attribute-outside');
  globalThis.__detachedAttributeStyleTarget = childDocument.getElementById('detached-attribute-target');

  const outside = frame.contentWindow.getComputedStyle(globalThis.__detachedAttributeStyleOutside);
  const target = frame.contentWindow.getComputedStyle(globalThis.__detachedAttributeStyleTarget);
  globalThis.__detachedAttributeStyleComputed = target;
  return `${outside.color}|${target.color}`;
})()
"#,
        )
        .expect("detached iframe attribute style cache setup should evaluate");

    assert_eq!(before, "rgb(1, 2, 3)|rgb(7, 8, 9)");
    let child_document = owner_document_handle_for_element_id(&vm, "detached-attribute-target");
    assert!(computed_style_cache_entry_count_for_document(&vm, child_document) > 0);

    let mutation_result = vm.eval(
        r#"
(() => {
  globalThis.__detachedAttributeStyleTarget.setAttribute('data-state', 'active');
  return [
    globalThis.__detachedAttributeStyleTarget.getAttribute('data-state'),
    globalThis.__detachedAttributeStyleTarget.hasAttribute('data-state'),
    globalThis.__detachedAttributeStyleFrame.contentDocument.querySelector('#detached-attribute-target[data-state="active"]') ===
      globalThis.__detachedAttributeStyleTarget
  ].join('|');
})()
"#,
    )
    .expect("detached iframe attribute mutation should evaluate");
    assert_eq!(mutation_result, "active|true|true");

    let after = vm
        .eval(
            r#"
(() => {
  const frame = globalThis.__detachedAttributeStyleFrame;
  const outside = frame.contentWindow.getComputedStyle(globalThis.__detachedAttributeStyleOutside);
  const target = frame.contentWindow.getComputedStyle(globalThis.__detachedAttributeStyleTarget);
  const heldTarget = globalThis.__detachedAttributeStyleComputed;
  delete globalThis.__detachedAttributeStyleFrame;
  delete globalThis.__detachedAttributeStyleOutside;
  delete globalThis.__detachedAttributeStyleTarget;
  delete globalThis.__detachedAttributeStyleComputed;
  return `${outside.color}|${target.color}|${heldTarget.color}`;
})()
"#,
        )
        .expect("detached iframe attribute style cache mutation should evaluate");

    assert_eq!(after, "rgb(1, 2, 3)|rgb(4, 5, 6)|rgb(4, 5, 6)");
    assert!(computed_style_cache_entry_count_for_document(&vm, child_document) > 0);
}

#[test]
fn held_detached_iframe_computed_style_is_empty_after_srcdoc_navigation() {
    let mut vm = new_storage_test_vm("https://detached-iframe-srcdoc-computed-style.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const detachedDocument = document.implementation.createHTMLDocument('');
  const frame = detachedDocument.createElement('iframe');
  frame.id = 'detached-srcdoc-navigation-frame';
  frame.srcdoc = '<style>body { color: rgb(0, 128, 0); }</style><body>first</body>';
  detachedDocument.body.appendChild(frame);
  const firstDocument = frame.contentDocument;
  const held = frame.contentWindow.getComputedStyle(firstDocument.body);
  const before = `${held.length > 0}:${held.color}`;

  frame.srcdoc = '<style>body { color: rgb(1, 2, 3); }</style><body>second</body>';
  const oldHeld = `${held.length}:${held.color}`;
  const secondDocument = frame.contentDocument;
  const secondHeld = frame.contentWindow.getComputedStyle(secondDocument.body);
  const secondFresh = secondHeld.color;

  let symbolError = 'none';
  try {
    frame.srcdoc = Symbol('srcdoc');
  } catch (error) {
    symbolError = error.name;
  }
  const afterFailedSrcdoc = `${symbolError}:${secondHeld.length > 200}:${secondHeld.color}:${frame.contentDocument === secondDocument}`;

  frame.removeAttribute('srcdoc');
  const secondOldHeld = `${secondHeld.length}:${secondHeld.color}`;
  const blankDocument = frame.contentDocument;
  blankDocument.body.id = 'detached-srcdoc-final-body';
  const blankFresh = frame.contentWindow.getComputedStyle(blankDocument.body).color;
  return [
    before,
    oldHeld,
    secondFresh,
    firstDocument === secondDocument,
    afterFailedSrcdoc,
    secondOldHeld,
    blankFresh,
    secondDocument === blankDocument
  ].join('|');
})()
"#,
        )
        .expect("held detached iframe computed style srcdoc navigation should evaluate");

    assert_eq!(
        result,
        "true:rgb(0, 128, 0)|0:|rgb(1, 2, 3)|false|TypeError:true:rgb(1, 2, 3):true|0:|rgb(0, 0, 0)|false"
    );
    let child_document = owner_document_handle_for_element_id(&vm, "detached-srcdoc-final-body");
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, child_document),
        1
    );
}

#[test]
fn held_detached_iframe_computed_style_is_empty_after_src_navigation() {
    let mut vm = new_storage_test_vm("https://detached-iframe-src-computed-style.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const detachedDocument = document.implementation.createHTMLDocument('');
  const frame = detachedDocument.createElement('iframe');
  frame.id = 'detached-src-navigation-frame';
  const first = 'data:text/html;charset=utf-8;base64,PGJvZHkgc3R5bGU9ImNvbG9yOiByZ2IoMCwgMTI4LCAwKSI+Zmlyc3Q8L2JvZHk+';
  const second = 'data:text/html;charset=utf-8;base64,PGJvZHkgc3R5bGU9ImNvbG9yOiByZ2IoMSwgMiwgMykiPnNlY29uZDwvYm9keT4=';
  const third = 'data:text/html;charset=utf-8;base64,PGJvZHkgc3R5bGU9ImNvbG9yOiByZ2IoNCwgNSwgNikiPnRoaXJkPC9ib2R5Pg==';
  frame.src = first;
  detachedDocument.body.appendChild(frame);
  const firstDocument = frame.contentDocument;
  const held = frame.contentWindow.getComputedStyle(firstDocument.body);
  const before = `${held.length > 0}:${held.color}`;

  frame.src = second;
  const oldHeld = `${held.length}:${held.color}`;
  const secondDocument = frame.contentDocument;
  const secondHeld = frame.contentWindow.getComputedStyle(secondDocument.body);
  const secondFresh = secondHeld.color;

  frame.setAttribute('src', third);
  const secondOldHeld = `${secondHeld.length}:${secondHeld.color}`;
  const thirdDocument = frame.contentDocument;
  const thirdHeld = frame.contentWindow.getComputedStyle(thirdDocument.body);
  const thirdFresh = thirdHeld.color;

  let symbolError = 'none';
  try {
    frame.src = Symbol('src');
  } catch (error) {
    symbolError = error.name;
  }
  const afterFailedSrc = `${symbolError}:${thirdHeld.length > 200}:${thirdHeld.color}:${frame.contentDocument === thirdDocument}`;

  frame.removeAttribute('src');
  const thirdOldHeld = `${thirdHeld.length}:${thirdHeld.color}`;
  const blankDocument = frame.contentDocument;
  blankDocument.body.id = 'detached-src-final-body';
  const blankFresh = frame.contentWindow.getComputedStyle(blankDocument.body).color;
  return [
    before,
    oldHeld,
    secondFresh,
    firstDocument === secondDocument,
    secondOldHeld,
    thirdFresh,
    secondDocument === thirdDocument,
    afterFailedSrc,
    thirdOldHeld,
    blankFresh,
    thirdDocument === blankDocument
  ].join('|');
})()
"#,
        )
        .expect("held detached iframe computed style src navigation should evaluate");

    assert_eq!(
        result,
        "true:rgb(0, 128, 0)|0:|rgb(1, 2, 3)|false|0:|rgb(4, 5, 6)|false|TypeError:true:rgb(4, 5, 6):true|0:|rgb(0, 0, 0)|false"
    );
    let child_document = owner_document_handle_for_element_id(&vm, "detached-src-final-body");
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, child_document),
        1
    );
}

#[test]
fn computed_line_height_resolves_numbers_and_percentages() {
    let mut vm = new_storage_test_vm("https://computed-line-height.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = 'div { font-size: 16px; }';
  (document.documentElement || document.body || document).appendChild(style);
  const appendTarget = document.body || document.documentElement || document;
  const values = ['normal', '1', '10px', '10%'];
  return values.map(value => {
    const target = document.createElement('div');
    target.style.lineHeight = value;
    appendTarget.appendChild(target);
    return getComputedStyle(target).lineHeight;
  }).join('|');
})()
"#,
        )
        .expect("computed line-height should resolve numbers and percentages");

    assert_eq!(result, "normal|16px|10px|1.6px");
}

#[test]
fn computed_horizontal_auto_margins_resolve_to_pixels() {
    let mut vm = new_storage_test_vm("https://computed-auto-margin.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = `
    x { display: block; position: relative; width: 60px; }
    y { display: block; width: 40px; margin: auto; }
    #target { position: absolute; left: 0; right: 0; }`;
  (document.documentElement || document.body || document).appendChild(style);
  const wrapper = document.createElement('x');
  const target = document.createElement('y');
  target.id = 'target';
  wrapper.appendChild(target);
  (document.body || document.documentElement || document).appendChild(wrapper);
  const computed = getComputedStyle(target);
  return [
    computed.marginLeft,
    computed.marginRight,
    computed.left,
    computed.right
  ].join('|');
})()
"#,
        )
        .expect("computed auto margins should resolve against containing block width");

    assert_eq!(result, "10px|10px|0px|0px");
}

#[test]
fn computed_insets_absolutize_font_relative_lengths() {
    let mut vm = new_storage_test_vm("https://computed-inset-em.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  target.style.cssText = 'position: static; font-size: 10px; top: 1em; right: 2em; bottom: 3em; left: 4em';
  (document.body || document.documentElement || document).appendChild(target);
  const computed = getComputedStyle(target);
  return [
    computed.top,
    computed.right,
    computed.bottom,
    computed.left
  ].join('|');
})()
"#,
        )
        .expect("computed insets should resolve font-relative lengths");

    assert_eq!(result, "10px|20px|30px|40px");
}

#[test]
fn computed_positioned_insets_resolve_percentages_against_containing_block() {
    let mut vm = new_storage_test_vm("https://computed-inset-percent.test/");

    vm.eval(
        r#"
  const wrapper = document.createElement('div');
  wrapper.style.cssText = 'height: 20px; width: 40px; padding: 1px 2px';
  const target = document.createElement('div');
  target.style.cssText = 'position: relative; top: 10%; left: calc(25% - 2px)';
  wrapper.appendChild(target);
  (document.body || document.documentElement || document).appendChild(wrapper);
  const computed = getComputedStyle(target);
"#,
    )
    .expect("prepare geometry fixture");
    publish_layout_for_test(&mut vm);
    let result = vm
        .eval(r#"`${computed.top}|${computed.left}`"#)
        .expect("computed positioned insets should resolve percentages");

    assert_eq!(result, "2px|8px");
}

#[test]
fn computed_relative_auto_insets_resolve_against_opposite_side() {
    let mut vm = new_storage_test_vm("https://computed-relative-auto-inset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  target.style.cssText = 'position: relative; top: auto; right: auto; bottom: 3px; left: 2px';
  (document.body || document.documentElement || document).appendChild(target);
  const computed = getComputedStyle(target);
  return `${computed.top}|${computed.right}|${computed.bottom}|${computed.left}`;
})()
"#,
        )
        .expect("computed relative auto insets should resolve against the opposite side");

    assert_eq!(result, "-3px|-2px|3px|2px");
}

#[test]
fn computed_absolute_auto_insets_resolve_against_containing_block() {
    let mut vm = new_storage_test_vm("https://computed-absolute-auto-inset.test/");

    vm.eval(
        r#"
  const wrapper = document.createElement('div');
  wrapper.style.cssText = 'position: relative; height: 20px; width: 40px';
  const target = document.createElement('div');
  target.style.cssText = 'position: absolute; top: auto; right: 4px; bottom: 3px; left: auto';
  wrapper.appendChild(target);
  (document.body || document.documentElement || document).appendChild(wrapper);
  const computed = getComputedStyle(target);
"#,
    )
    .expect("prepare geometry fixture");
    publish_layout_for_test(&mut vm);
    let result = vm
        .eval(r#"`${computed.top}|${computed.right}|${computed.bottom}|${computed.left}`"#)
        .expect("computed absolute auto insets should resolve against the containing block");

    assert_eq!(result, "17px|4px|3px|36px");
}

#[test]
fn computed_absolute_grid_inset_left_defaults_to_zero() {
    let mut vm = new_storage_test_vm("https://computed-absolute-grid-inset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) document.appendChild(document.createElement('html'));
  if (!document.head) document.documentElement.appendChild(document.createElement('head'));
  if (!document.body) document.documentElement.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = `
    span { display: grid; grid-template-columns: 100px 100px; }
    span { position: absolute; grid-column: 2; }`;
  document.head.appendChild(style);
  document.body.innerHTML = '<span>abc<span id="target">def</span></span>';
  return getComputedStyle(document.getElementById('target')).left;
})()
"#,
        )
        .expect("absolute grid item computed left should resolve to zero");

    assert_eq!(result, "0px");
}

#[test]
fn computed_grid_column_auto_and_unset_resolve_to_auto() {
    let mut vm = new_storage_html_test_vm("https://computed-grid-column-auto.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const unset = document.createElement('div');
  const auto = document.createElement('div');
  const positioned = document.createElement('div');
  auto.style.cssText = 'grid-column: auto';
  positioned.style.cssText = 'grid-column: 2';
  const body = document.body || document.documentElement || document;
  body.appendChild(unset);
  body.appendChild(auto);
  body.appendChild(positioned);
  const value = (element) => {
    const style = getComputedStyle(element);
    return `${style.getPropertyValue('grid-column-start')}/${style.getPropertyValue('grid-column-end')}`;
  };
  return [value(unset), value(auto), value(positioned)].join('|');
})()
"#,
        )
        .expect("computed grid-column auto/unset values should resolve");

    assert_eq!(result, "auto/auto|auto/auto|2/auto");
}

#[test]
fn computed_absolute_logical_inline_insets_resolve_physical_sides() {
    let mut vm = new_storage_test_vm("https://computed-logical-inline-inset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) document.appendChild(document.createElement('html'));
  if (!document.head) document.documentElement.appendChild(document.createElement('head'));
  if (!document.body) document.documentElement.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = `
    .ifc { position: relative; font: 20px/1 Ahem; }
    .relpos { position: relative; }
    .target { position: absolute; width: 5em; height: 1em; top: 1em; }
    .fix-start { inset-inline-start: 0; }
    .fix-end { inset-inline-end: 0; }`;
  document.head.appendChild(style);
  document.body.innerHTML = `
    <div class="ifc">
      Lorem
      <span class="relpos">
        ipsum dolor
        <div class="target fix-start" id="start"></div>
        <div class="target fix-end" id="end"></div>
      </span>
      sit amet
    </div>`;
  const start = getComputedStyle(document.getElementById('start'));
  const end = getComputedStyle(document.getElementById('end'));
  return `${start.left}|${start.right}|${end.left}|${end.right}`;
})()
"#,
        )
        .expect("logical inline insets should resolve physical left and right");

    assert_eq!(result, "0px|140px|140px|0px");
}

#[test]
fn computed_style_property_names_follow_cssom_order() {
    let mut vm = new_storage_test_vm("https://computed-style-order.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const styleElement = document.createElement('style');
  styleElement.textContent = `
    #target {
      background-position-x: 25%;
      grid-auto-columns: 17px;
      object-fit: cover;
      overflow-wrap: anywhere;
      pointer-events: none;
      white-space-collapse: preserve;
    }
  `;
  (document.head || document.documentElement || document).appendChild(styleElement);
  const target = document.createElement('div');
  target.id = 'target';
  target.style.setProperty('--zeta-token', 'zeta');
  target.style.setProperty('--alpha-token', 'alpha');
  (document.body || document.documentElement || document).appendChild(target);
  const style = getComputedStyle(target);
  const properties = Array.from(style);
  const sorted = properties.slice().sort((left, right) => {
    const segment = name => name.startsWith('--') ? 2 : name.startsWith('-') ? 1 : 0;
    if (segment(left) !== segment(right)) {
      return segment(left) - segment(right);
    }
    return left < right ? -1 : left > right ? 1 : 0;
  });
  const values = Object.fromEntries([
    'background-position-x',
    'grid-auto-columns',
    'object-fit',
    'overflow-wrap',
    'pointer-events',
    'white-space-collapse',
  ].map(name => [name, style.getPropertyValue(name)]));
  return JSON.stringify({
    count: properties.length,
    sorted: properties.join('\n') === sorted.join('\n'),
    unique: new Set(properties).size === properties.length,
    indexed: properties.every((name, index) => style[index] === name && style.item(index) === name),
    outOfRange: style.item(style.length),
    includesDirection: properties.includes('direction'),
    includesUnicodeBidi: properties.includes('unicode-bidi'),
    excludesShorthands: !properties.includes('margin') &&
      !properties.includes('mask') && !properties.includes('padding-block'),
    customOrder: properties.indexOf('--alpha-token') < properties.indexOf('--zeta-token'),
    values,
  });
})()
"#,
        )
        .expect("computed style property names should be enumerable");

    let result: serde_json::Value = serde_json::from_str(&result).expect("valid JSON summary");
    assert!(result["count"].as_u64().is_some_and(|count| count >= 268));
    for key in [
        "sorted",
        "unique",
        "indexed",
        "includesDirection",
        "includesUnicodeBidi",
        "excludesShorthands",
        "customOrder",
    ] {
        assert_eq!(
            result[key],
            serde_json::json!(true),
            "failed invariant {key}"
        );
    }
    assert_eq!(result["outOfRange"], serde_json::json!(""));
    assert_eq!(result["values"]["background-position-x"], "25%");
    assert_eq!(result["values"]["grid-auto-columns"], "17px");
    assert_eq!(result["values"]["object-fit"], "cover");
    assert_eq!(result["values"]["overflow-wrap"], "anywhere");
    assert_eq!(result["values"]["pointer-events"], "none");
    assert_eq!(result["values"]["white-space-collapse"], "preserve");
}
