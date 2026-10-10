use super::*;

#[test]
fn child_frame_focus_selector_invalidation_uses_child_document_world() {
    let mut vm = new_storage_test_vm("https://child-focus-style-cache.test/");

    let setup = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const activeStyle = document.createElement('style');
  activeStyle.textContent = '#active-focus-cache { color: rgb(1, 2, 3); }';
  head.appendChild(activeStyle);
  const active = document.createElement('div');
  active.id = 'active-focus-cache';
  body.appendChild(active);
  globalThis.__childFocusActiveStyle = getComputedStyle(active);

  const frame = document.createElement('iframe');
  frame.id = 'focus-child-frame';
  body.appendChild(frame);
  const childWindow = frame.contentWindow;
  const childDocument = childWindow.document;
  childDocument.open();
  childDocument.write(`
    <style>
      #sibling { color: rgb(4, 5, 6); }
      #target:focus + #sibling { color: rgb(7, 8, 9); }
    </style>
    <body>
      <button id="target">target</button>
      <span id="sibling">sibling</span>
    </body>
  `);
  childDocument.close();
  globalThis.__childFocusFrame = frame;
  globalThis.__childFocusTarget = childDocument.getElementById('target');
  globalThis.__childFocusSiblingStyle =
    childWindow.getComputedStyle(childDocument.getElementById('sibling'));

  return [
    globalThis.__childFocusActiveStyle.color,
    globalThis.__childFocusSiblingStyle.color
  ].join('|');
})()
"#,
        )
        .expect("child frame focus style setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)|rgb(4, 5, 6)");
    let active_document = vm.document_runtime.dom_host().document_handle();
    let child_document = child_document_handle_for_frame_id(&vm, "focus-child-frame");
    let active_cache_before = computed_style_cache_entry_count_for_document(&vm, active_document);
    assert!(active_cache_before > 0);
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, child_document),
        1
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_style_invalidation_work_item_count_for_document_for_test(active_document),
        0
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_style_invalidation_work_item_count_for_document_for_test(child_document),
        0
    );

    let focused = vm
        .eval(
            r#"
(() => {
  globalThis.__childFocusTarget.focus();
  const childDocument = globalThis.__childFocusFrame.contentDocument;
  const result = [
    childDocument.activeElement === globalThis.__childFocusTarget,
    globalThis.__childFocusSiblingStyle.color,
    globalThis.__childFocusActiveStyle.color
  ].join('|');
  delete globalThis.__childFocusFrame;
  delete globalThis.__childFocusTarget;
  delete globalThis.__childFocusSiblingStyle;
  delete globalThis.__childFocusActiveStyle;
  return result;
})()
"#,
        )
        .expect("child frame focus style mutation should evaluate");

    assert_eq!(focused, "true|rgb(7, 8, 9)|rgb(1, 2, 3)");
    assert!(
        computed_style_cache_entry_count_for_document(&vm, active_document) >= active_cache_before,
        "child focus invalidation must not evict the active document cache; the focus reveal layout may fill additional entries"
    );
    assert!(
        computed_style_cache_entry_count_for_document(&vm, child_document) > 0,
        "child focus invalidation should keep style work in the child document world"
    );
}

#[test]
fn popup_focus_selector_invalidation_uses_popup_document_world() {
    let mut vm = new_storage_test_vm("https://popup-focus-style-cache.test/");

    let setup = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const activeStyle = document.createElement('style');
  activeStyle.textContent = '#active-popup-focus-cache { color: rgb(1, 2, 3); }';
  head.appendChild(activeStyle);
  const active = document.createElement('div');
  active.id = 'active-popup-focus-cache';
  body.appendChild(active);
  globalThis.__popupFocusActiveStyle = getComputedStyle(active);

  const popup = open('about:blank');
  globalThis.__popupFocusWindow = popup;
  const popupRoot = popup.document.documentElement ||
    popup.document.appendChild(popup.document.createElement('html'));
  const popupHead = popup.document.head ||
    popupRoot.appendChild(popup.document.createElement('head'));
  const popupBody = popup.document.body ||
    popupRoot.appendChild(popup.document.createElement('body'));
  const style = popup.document.createElement('style');
  style.textContent = [
    '#popup-focus-sibling { color: rgb(4, 5, 6); }',
    '#popup-focus-target:focus + #popup-focus-sibling { color: rgb(7, 8, 9); }'
  ].join('\n');
  popupHead.appendChild(style);
  const target = popup.document.createElement('button');
  target.id = 'popup-focus-target';
  const sibling = popup.document.createElement('span');
  sibling.id = 'popup-focus-sibling';
  popupBody.append(target, sibling);
  globalThis.__popupFocusTarget = target;
  globalThis.__popupFocusSiblingStyle = popup.getComputedStyle(sibling);

  return [
    globalThis.__popupFocusActiveStyle.color,
    globalThis.__popupFocusSiblingStyle.color
  ].join('|');
})()
"#,
        )
        .expect("popup focus style setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)|rgb(4, 5, 6)");
    let active_document = vm.document_handle_for_test();
    let popup_document = owner_document_handle_for_element_id(&vm, "popup-focus-target");
    assert_ne!(popup_document, active_document);
    let active_cache_before = computed_style_cache_entry_count_for_document(&vm, active_document);
    assert!(active_cache_before > 0);
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, popup_document),
        1
    );

    let focused = vm
        .eval(
            r#"
(() => {
  globalThis.__popupFocusTarget.focus();
  const result = [
    __popupFocusWindow.document.activeElement === globalThis.__popupFocusTarget,
    globalThis.__popupFocusSiblingStyle.color
  ].join('|');
  delete globalThis.__popupFocusWindow;
  delete globalThis.__popupFocusTarget;
  delete globalThis.__popupFocusSiblingStyle;
  delete globalThis.__popupFocusActiveStyle;
  return result;
})()
"#,
        )
        .expect("popup focus style mutation should evaluate");

    assert_eq!(focused, "true|rgb(7, 8, 9)");
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, active_document),
        active_cache_before,
        "popup focus invalidation should not clear active document cache"
    );
    assert!(
        computed_style_cache_entry_count_for_document(&vm, popup_document) > 0,
        "popup focus invalidation should keep style work in the popup document world"
    );
}

#[test]
fn isolated_world_focus_selector_invalidation_uses_root_document_world() {
    let mut vm = new_storage_test_vm("https://isolated-focus-style-cache.test/");

    let setup = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = [
    '#isolated-focus-sibling { color: rgb(4, 5, 6); }',
    '#isolated-focus-target:focus + #isolated-focus-sibling { color: rgb(7, 8, 9); }'
  ].join('\n');
  head.appendChild(style);
  const target = document.createElement('button');
  target.id = 'isolated-focus-target';
  const sibling = document.createElement('span');
  sibling.id = 'isolated-focus-sibling';
  body.append(target, sibling);
  globalThis.__isolatedFocusSiblingStyle = getComputedStyle(sibling);
  return globalThis.__isolatedFocusSiblingStyle.color;
})()
"#,
        )
        .expect("isolated focus style setup should evaluate");

    assert_eq!(setup, "rgb(4, 5, 6)");
    let document = vm.document_handle_for_test();
    let generation_before_focus =
        vm.computed_style_cache_generation_for_document_for_test(document);
    let context_id = vm
        .create_isolated_world("style-focus-test", false)
        .expect("isolated world should be created");
    let focused = vm
        .eval_in_isolated_context(
            context_id,
            r#"
(() => {
  document.getElementById('isolated-focus-target').focus();
  return String(document.activeElement === document.getElementById('isolated-focus-target'));
})()
"#,
        )
        .expect("isolated focus mutation should evaluate");

    assert_eq!(focused, "true");
    let resolved = vm
        .eval(
            r#"
(() => {
  const result = globalThis.__isolatedFocusSiblingStyle.color;
  delete globalThis.__isolatedFocusSiblingStyle;
  return result;
})()
"#,
        )
        .expect("default world held style should see isolated focus invalidation");

    assert_eq!(resolved, "rgb(7, 8, 9)");
    assert_eq!(
        vm.computed_style_cache_generation_for_document_for_test(document),
        generation_before_focus,
        "isolated focus invalidation should not bump the retained style generation"
    );
    assert!(computed_style_cache_entry_count_for_document(&vm, document) > 0);
}

#[test]
fn style_text_character_data_mutation_updates_computed_style() {
    let mut vm = new_storage_test_vm("https://style-text-character-data.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  const text = document.createTextNode('#target { color: rgb(255, 0, 0); }');
  style.appendChild(text);
  head.appendChild(style);

  const target = document.createElement('div');
  target.id = 'target';
  body.appendChild(target);
  const before = getComputedStyle(target).color;

  text.data = '#target { color: rgb(0, 128, 0); }';
  const after = getComputedStyle(target).color;
  return `${before}|${after}`;
})()
"#,
        )
        .expect("style text character data mutation should evaluate");

    assert_eq!(result, "rgb(255, 0, 0)|rgb(0, 128, 0)");
}

#[test]
fn nested_container_and_scope_rules_track_outer_selector_text_mutation() {
    let mut vm = new_storage_test_vm("https://nested-container-scope-selector-text.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  head.appendChild(style);
  const main = document.createElement('main');
  main.setAttribute('style', 'container-type: size; width: 50px; height: 50px');
  main.innerHTML = '<div class="a"><div class="x"></div></div><div class="b"><div class="x"></div></div>';
  body.appendChild(main);
  const ax = main.querySelector('.a > .x');
  const bx = main.querySelector('.b > .x');
  function run(cssText) {
    style.textContent = cssText;
    const before = [getComputedStyle(ax).zIndex, getComputedStyle(bx).zIndex].join(',');
    style.sheet.cssRules[0].selectorText = '.b';
    const after = [getComputedStyle(ax).zIndex, getComputedStyle(bx).zIndex].join(',');
    style.textContent = '';
    return `${before}>${after}`;
  }
  return [
    run('.a { @container (width) { & .x { z-index: 1; } } }'),
    run('.a { @scope (&) { & .x { z-index: 1; } } }')
  ].join('|');
})()
"#,
        )
        .expect("nested container and scope selectorText mutation should evaluate");

    assert_eq!(result, "1,auto>auto,1|1,auto>auto,1");
}

#[test]
fn container_style_queries_resolve_direct_typed_attr_values() {
    let mut vm = new_storage_test_vm("https://container-style-query-attr.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = `
    @container style(1px < attr(data-size type(<length>)) < 10px) {
      #target { --direct:true; }
    }
    @container style(1px < attr(data-missing type(<length>), 5px) < 10px) {
      #target { --fallback:true; }
    }
  `;
  head.appendChild(style);
  const container = document.createElement('main');
  container.setAttribute('data-size', '5px');
  container.innerHTML = '<div id="target"></div>';
  body.appendChild(container);
  const computed = getComputedStyle(container.firstElementChild);
  return [
    style.sheet.cssRules.length,
    computed.getPropertyValue('--direct'),
    computed.getPropertyValue('--fallback')
  ].join('|');
})()
"#,
        )
        .expect("container style query direct typed attr values should evaluate");

    assert_eq!(result, "2|true|true");
}

#[test]
fn implicit_scope_root_for_owner_stylesheet_matches_parent_element() {
    let mut vm = new_storage_test_vm("https://implicit-scope-root.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const main = document.createElement('main');
  main.innerHTML = '<div class="a"><style>@scope { z-index: 1; }</style></div>';
  body.appendChild(main);
  return getComputedStyle(main.querySelector('.a')).zIndex;
})()
"#,
        )
        .expect("implicit @scope root computed style should evaluate");

    assert_eq!(result, "1");
}

#[test]
fn computed_style_is_empty_for_disconnected_shadow_tree_elements() {
    let mut vm = new_storage_test_vm("https://computed-style-disconnected-shadow.test/");
    let document = vm.document_handle_for_test();

    let connected = vm
        .eval(
            r#"
(() => {
  globalThis.__shadowCacheRoot =
    document.documentElement || document.appendChild(document.createElement('html'));
  globalThis.__shadowCacheHost = document.createElement('div');
  globalThis.__shadowCacheRoot.append(globalThis.__shadowCacheHost);
  const shadow = globalThis.__shadowCacheHost.attachShadow({ mode: 'open' });
  globalThis.__shadowCacheTarget = document.createElement('span');
  globalThis.__shadowCacheTarget.className = 'target';
  shadow.append(globalThis.__shadowCacheTarget);
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.target { color: green; }');
  shadow.adoptedStyleSheets = [sheet];
  return getComputedStyle(globalThis.__shadowCacheTarget).color;
})()
"#,
        )
        .expect("connected shadow tree computed style should evaluate");

    assert_eq!(connected, "rgb(0, 128, 0)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(document),
        1
    );

    let disconnected = vm
        .eval(
            r#"
(() => {
  globalThis.__shadowCacheHost.remove();
  const disconnectedStyle = getComputedStyle(globalThis.__shadowCacheTarget);
  const disconnected = [
    disconnectedStyle.color,
    disconnectedStyle.length
  ].join(',');
  return disconnected;
})()
"#,
        )
        .expect("disconnected shadow tree computed style should evaluate");

    assert_eq!(disconnected, ",0");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(document),
        1
    );

    let reconnected = vm
        .eval(
            r#"
(() => {
  globalThis.__shadowCacheRoot.append(globalThis.__shadowCacheHost);
  const reconnected = getComputedStyle(globalThis.__shadowCacheTarget).color;
  delete globalThis.__shadowCacheTarget;
  delete globalThis.__shadowCacheHost;
  delete globalThis.__shadowCacheRoot;
  return reconnected;
})()
"#,
        )
        .expect("reconnected shadow tree computed style should evaluate");

    assert_eq!(reconnected, "rgb(0, 128, 0)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
}

#[test]
fn computed_style_reuses_retained_system_across_connected_shadow_roots() {
    let mut vm = new_storage_test_vm("https://computed-style-shadow-retained-cache.test/");
    let document = vm.document_handle_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));

  function makeTarget(name, css) {
    const host = document.createElement('section');
    body.appendChild(host);
    const shadow = host.attachShadow({ mode: 'open' });
    const target = document.createElement('span');
    target.className = 'target';
    shadow.appendChild(target);
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(css);
    shadow.adoptedStyleSheets = [sheet];
    return target;
  }

  globalThis.__shadowRetainedTargetA =
    makeTarget('a', '.target { color: rgb(1, 2, 3); }');
  globalThis.__shadowRetainedTargetB =
    makeTarget('b', '.target { color: rgb(4, 5, 6); }');

  const a1 = getComputedStyle(globalThis.__shadowRetainedTargetA).color;
  const b1 = getComputedStyle(globalThis.__shadowRetainedTargetB).color;
  const a2 = getComputedStyle(globalThis.__shadowRetainedTargetA).color;
  const b2 = getComputedStyle(globalThis.__shadowRetainedTargetB).color;
  return [a1, b1, a2, b2].join('|');
})()
"#,
        )
        .expect("connected shadow root computed styles should evaluate");

    assert_eq!(
        result,
        "rgb(1, 2, 3)|rgb(4, 5, 6)|rgb(1, 2, 3)|rgb(4, 5, 6)"
    );
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
}

#[test]
fn implicit_move_to_detached_parent_removes_retained_shadow_scope() {
    let mut vm = new_storage_test_vm("https://implicit-shadow-detach-style-world.test/");
    let document = vm.document_handle_for_test();

    assert_eq!(
        vm.eval(
            r#"(() => {
              const root = document.documentElement ||
                document.appendChild(document.createElement('html'));
              const body = document.body || root.appendChild(document.createElement('body'));
              const host = document.createElement('section');
              host.id = 'movable-shadow-host';
              body.append(host);
              const shadow = host.attachShadow({ mode: 'open' });
              const target = document.createElement('span');
              target.id = 'target';
              shadow.append(target);
              const sheet = new CSSStyleSheet();
              sheet.replaceSync('#target { color: rgb(1, 2, 3); }');
              shadow.adoptedStyleSheets = [sheet];
              globalThis.__movableShadowHost = host;
              globalThis.__detachedShadowParent = document.createElement('div');
              return getComputedStyle(target).color;
            })()"#,
        )
        .expect("connected adopted stylesheet should resolve"),
        "rgb(1, 2, 3)",
    );
    let shadow_root = {
        let context = vm._context_host.borrow();
        let host = context.dom_host();
        let shadow_host = host
            .element_handle_by_id("movable-shadow-host")
            .expect("shadow host handle");
        host.shadow_root_handle(shadow_host)
            .expect("shadow root handle")
    };
    assert!(
        vm.retained_shadow_scope_flush_count_for_document_for_test(document, shadow_root)
            .is_some(),
        "the initial observation must install the adopted ShadowRoot scope",
    );
    let updates_before_move = vm.retained_style_system_update_count_for_document_for_test(document);

    vm.eval(
        r#"(() => {
          __detachedShadowParent.appendChild(__movableShadowHost);
          return getComputedStyle(document.body).display;
        })()"#,
    )
    .expect("the old Document should observe the implicit removal");

    assert_eq!(
        vm.retained_shadow_scope_flush_count_for_document_for_test(document, shadow_root),
        None,
        "the old Document must discard the departed adopted ShadowRoot scope",
    );
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(document),
        updates_before_move + 1,
        "the next old-Document observation must apply one TreeScope update",
    );
}

#[test]
fn implicit_cross_document_move_transfers_retained_shadow_scope() {
    let mut vm = new_storage_test_vm("https://implicit-shadow-cross-document.test/");
    let parent_document = vm.document_handle_for_test();
    assert_eq!(
        vm.eval(
            r#"(() => {
              const html = document.documentElement ||
                document.appendChild(document.createElement('html'));
              const body = document.body || html.appendChild(document.createElement('body'));
              const frame = document.createElement('iframe');
              frame.id = 'shadow-move-frame';
              body.append(frame);
              frame.contentDocument.open();
              frame.contentDocument.write('<!doctype html><body></body>');
              frame.contentDocument.close();

              const host = document.createElement('section');
              host.id = 'cross-document-shadow-host';
              body.append(host);
              const shadow = host.attachShadow({ mode: 'open' });
              shadow.innerHTML = `
                <style>#target { color: rgb(9, 8, 7); }</style>
                <span id="target"></span>`;
              globalThis.__crossDocumentShadowHost = host;
              globalThis.__crossDocumentShadowTarget = shadow.getElementById('target');
              return getComputedStyle(__crossDocumentShadowTarget).color;
            })()"#,
        )
        .expect("parent shadow style should initialize"),
        "rgb(9, 8, 7)",
    );
    let child_document = child_document_handle_for_frame_id(&vm, "shadow-move-frame");
    let shadow_root = {
        let host = vm.document_runtime.dom_host();
        let shadow_host = host
            .element_handle_by_id("cross-document-shadow-host")
            .expect("cross-document shadow host handle");
        host.shadow_root_handle(shadow_host)
            .expect("cross-document shadow root handle")
    };
    let parent_identity = vm.retained_stylist_identity_for_document_for_test(parent_document);
    assert!(
        vm.retained_shadow_scope_flush_count_for_document_for_test(parent_document, shadow_root)
            .is_some()
    );

    assert_eq!(
        vm.eval(
            r#"(() => {
              const frame = document.getElementById('shadow-move-frame');
              frame.contentDocument.body.appendChild(__crossDocumentShadowHost);
              const parentProbe = getComputedStyle(document.body).display;
              const childColor = frame.contentWindow
                .getComputedStyle(__crossDocumentShadowTarget).color;
              return [parentProbe, childColor].join('|');
            })()"#,
        )
        .expect("cross-document shadow move should evaluate"),
        "block|rgb(9, 8, 7)",
    );
    assert_eq!(
        vm.retained_stylist_identity_for_document_for_test(parent_document),
        parent_identity,
        "the old Document world must update in place",
    );
    assert_eq!(
        vm.retained_shadow_scope_flush_count_for_document_for_test(parent_document, shadow_root),
        None,
        "the old Document must release the moved ShadowRoot scope",
    );
    assert!(
        vm.retained_shadow_scope_flush_count_for_document_for_test(child_document, shadow_root)
            .is_some(),
        "the destination Document must install the moved ShadowRoot scope",
    );
}

#[test]
fn child_layout_uses_complete_stable_tree_scope_universe() {
    let mut vm = new_storage_test_vm("https://child-tree-scope-universe.test/");

    vm.eval(
        r#"
(() => {
  const body = document.body || document.documentElement || document;
  const frame = document.createElement('iframe');
  frame.id = 'tree-scope-universe-frame';
  frame.style.cssText = 'width: 320px; height: 240px; border: 0';
  body.appendChild(frame);

  const childDocument = frame.contentDocument;
  childDocument.open();
  childDocument.write('<style>body { color: rgb(4, 5, 6); }</style><body><main>child</main></body>');
  childDocument.close();
  for (let index = 0; index < 32; index += 1) {
    const host = childDocument.createElement('section');
    childDocument.body.appendChild(host);
    const shadow = host.attachShadow({mode: 'open'});
    shadow.append(`empty-${index}`);
    if (index % 8 === 0) {
      const style = childDocument.createElement('style');
      style.textContent = ':host { display: block; }';
      shadow.prepend(style);
    }
  }
})()
"#,
    )
    .expect("child TreeScope universe fixture should initialize");

    let child_document = child_document_handle_for_frame_id(&vm, "tree-scope-universe-frame");
    let rebuilds_before =
        vm.retained_style_system_rebuild_count_for_document_for_test(child_document);
    let update_materializations_before = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    let full_snapshots_before = vm
        ._context_host
        .borrow()
        .style_world_full_snapshots_for_test();

    vm.screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
        .expect("child TreeScope universe paint layout should succeed")
        .expect("the fixture should have a layout root");

    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(child_document)
            .saturating_sub(rebuilds_before),
        1,
        "one child-document layout must build one retained style system",
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .style_world_update_materializations_for_test()
            .saturating_sub(update_materializations_before),
        2,
        "the paint layout must materialize exactly one style-world update per document",
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .style_world_full_snapshots_for_test()
            .saturating_sub(full_snapshots_before),
        2,
        "the first paint layout must materialize one full snapshot per document",
    );
}

#[test]
fn child_tree_scope_membership_update_does_not_touch_parent_style_world() {
    let mut vm = new_storage_test_vm("https://tree-scope-document-isolation.test/");
    assert_eq!(
        vm.eval(
            r#"
const parentStyle = document.createElement('style');
parentStyle.textContent = '#parent-target { color: rgb(1, 2, 3); }';
const root = document.documentElement || document.appendChild(document.createElement('html'));
const head = document.head || root.appendChild(document.createElement('head'));
const body = document.body || root.appendChild(document.createElement('body'));
head.appendChild(parentStyle);
const parentTarget = document.createElement('div');
parentTarget.id = 'parent-target';
body.appendChild(parentTarget);
const frame = document.createElement('iframe');
frame.id = 'tree-scope-isolation-frame';
body.appendChild(frame);
const childDocument = frame.contentDocument;
childDocument.open();
childDocument.write('<style>#child-target { color: rgb(4, 5, 6); }</style><body><div id=child-target></div></body>');
childDocument.close();
JSON.stringify([
  getComputedStyle(parentTarget).color,
  frame.contentWindow.getComputedStyle(childDocument.getElementById('child-target')).color,
]);
"#,
        )
        .expect("parent and child style worlds should initialize"),
        r#"["rgb(1, 2, 3)","rgb(4, 5, 6)"]"#
    );

    let parent_document = vm.document_handle_for_test();
    let child_document = child_document_handle_for_frame_id(&vm, "tree-scope-isolation-frame");
    let parent_identity = vm.retained_stylist_identity_for_document_for_test(parent_document);
    let child_identity = vm.retained_stylist_identity_for_document_for_test(child_document);
    let parent_updates =
        vm.retained_style_system_update_count_for_document_for_test(parent_document);
    let child_updates = vm.retained_style_system_update_count_for_document_for_test(child_document);
    let shadow_materializations = vm
        ._context_host
        .borrow()
        .style_world_shadow_scope_materializations_for_test();

    assert_eq!(
        vm.eval(
            r#"
const childHost = document.getElementById('tree-scope-isolation-frame')
  .contentDocument.createElement('section');
document.getElementById('tree-scope-isolation-frame')
  .contentDocument.body.appendChild(childHost);
childHost.attachShadow({ mode: 'open' });
getComputedStyle(document.getElementById('parent-target')).color;
"#,
        )
        .expect("a parent-only observation should ignore the child TreeScope change"),
        "rgb(1, 2, 3)"
    );
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(parent_document),
        parent_updates
    );
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(child_document),
        child_updates
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .style_world_shadow_scope_materializations_for_test(),
        shadow_materializations,
        "the child scope must stay lazy until the child Document is observed"
    );

    assert_eq!(
        vm.eval(
            r#"
const observedFrame = document.getElementById('tree-scope-isolation-frame');
observedFrame.contentWindow.getComputedStyle(
  observedFrame.contentDocument.getElementById('child-target')
).color;
"#,
        )
        .expect("the child observation should reconcile its own TreeScopes"),
        "rgb(4, 5, 6)"
    );
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(parent_document),
        parent_updates,
        "observing the child must leave the parent world untouched"
    );
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(child_document),
        child_updates + 1
    );
    assert_eq!(
        vm.retained_stylist_identity_for_document_for_test(parent_document),
        parent_identity
    );
    assert_eq!(
        vm.retained_stylist_identity_for_document_for_test(child_document),
        child_identity,
        "TreeScope membership changes must update the child Stylist in place"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .style_world_shadow_scope_materializations_for_test(),
        shadow_materializations + 1
    );
}

#[test]
fn empty_shadow_root_universe_updates_once_per_dom_mutation() {
    let mut vm = new_storage_test_vm("https://empty-shadow-universe-mutation.test/");
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const light = document.createElement('main');
  light.id = 'universe-light';
  body.appendChild(light);
  return getComputedStyle(light).display;
})()
"#,
        )
        .expect("initial light-tree style should evaluate");
    assert_eq!(initial, "block");
    let rebuilds_after_initial =
        vm.retained_style_system_rebuild_count_for_document_for_test(document);
    let updates_after_initial =
        vm.retained_style_system_update_count_for_document_for_test(document);

    let attached = vm
        .eval(
            r#"
(() => {
  const host = document.createElement('section');
  host.id = 'universe-host';
  document.body.appendChild(host);
  const shadow = host.attachShadow({mode: 'open'});
  const target = document.createElement('span');
  target.id = 'universe-shadow-target';
  shadow.appendChild(target);
  return [
    getComputedStyle(document.getElementById('universe-light')).display,
    getComputedStyle(target).display,
    getComputedStyle(document.getElementById('universe-light')).display
  ].join('|');
})()
"#,
        )
        .expect("styles after empty Shadow Root attachment should evaluate");
    assert_eq!(attached, "block|inline|block");
    let rebuilds_after_attach =
        vm.retained_style_system_rebuild_count_for_document_for_test(document);
    assert_eq!(
        rebuilds_after_attach.saturating_sub(rebuilds_after_initial),
        0,
        "attaching an empty Shadow Root must preserve the Document Stylist",
    );
    let updates_after_attach =
        vm.retained_style_system_update_count_for_document_for_test(document);
    assert_eq!(
        updates_after_attach.saturating_sub(updates_after_initial),
        1,
        "attaching one empty Shadow Root must update the retained TreeScope universe exactly once",
    );

    let removed = vm
        .eval(
            r#"
(() => {
  document.getElementById('universe-host').remove();
  const light = document.getElementById('universe-light');
  return [getComputedStyle(light).display, getComputedStyle(light).display].join('|');
})()
"#,
        )
        .expect("styles after empty Shadow Root removal should evaluate");
    assert_eq!(removed, "block|block");
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(document)
            .saturating_sub(rebuilds_after_attach),
        0,
        "disconnecting an empty Shadow Root must preserve the Document Stylist",
    );
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(document)
            .saturating_sub(updates_after_attach),
        1,
        "disconnecting one empty Shadow Root must update the retained TreeScope universe exactly once",
    );
}

#[test]
fn computed_style_is_empty_for_detached_and_non_flat_tree_elements() {
    let mut vm = new_storage_test_vm("https://computed-style-non-flat-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const summarize = style => `${style.length}:${style.color}`;

  const detached = document.createElement('div');

  const host = document.createElement('div');
  host.innerHTML = '<div id="non-slotted"><span id="non-slotted-descendant"></span></div>';
  body.appendChild(host);
  host.attachShadow({ mode: 'open' });

  const detachedHost = document.createElement('div');
  const detachedShadow = detachedHost.attachShadow({ mode: 'open' });
  detachedShadow.innerHTML = '<span id="detached-shadow-descendant"></span>';

  const frame = document.createElement('iframe');
  frame.style.display = 'none';
  body.appendChild(frame);
  const childDocument = frame.contentDocument;

  return [
    summarize(getComputedStyle(detached)),
    summarize(getComputedStyle(document.getElementById('non-slotted'))),
    summarize(getComputedStyle(document.getElementById('non-slotted-descendant'))),
    summarize(getComputedStyle(detachedShadow.getElementById('detached-shadow-descendant'))),
    summarize(getComputedStyle(childDocument.documentElement)),
    summarize(frame.contentWindow.getComputedStyle(childDocument.documentElement))
  ].join('|');
})()
"#,
        )
        .expect("non-flat tree computed styles should evaluate");

    assert_eq!(result, "0:|0:|0:|0:|0:|0:");
}

#[test]
fn computed_style_is_empty_for_detached_document_elements() {
    let mut vm = new_storage_test_vm("https://computed-style-detached-document.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = 'div { color: rgb(0, 128, 0); background-image: url(active.png); }';
  (document.head || document.documentElement || document).appendChild(style);

  const parsed = new DOMParser().parseFromString(
    '<style>div { color: rgb(255, 0, 0); background-image: url(detached.png); }</style><div id="target">x</div>',
    'text/html'
  );
  const target = parsed.getElementById('target');
  const computed = getComputedStyle(target);
  return [
    computed.length,
    computed.color,
    computed.backgroundImage
  ].join('|');
})()
"#,
        )
        .expect("detached document computed style probe should evaluate");

    assert_eq!(result, "0||");
}

#[test]
fn computed_style_wrapper_refreshes_empty_context_after_tree_mutation() {
    let mut vm = new_storage_test_vm("https://computed-style-wrapper-refresh.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '.target { color: rgb(0, 128, 0); }';
  head.appendChild(style);

  const target = document.createElement('div');
  target.className = 'target';
  const computed = getComputedStyle(target);
  const before = `${computed.length}:${computed.color}`;
  body.appendChild(target);
  const connected = `${computed.length > 0}:${computed.color}`;
  target.remove();
  const removed = `${computed.length}:${computed.color}`;
  return `${before}|${connected}|${removed}`;
})()
"#,
        )
        .expect("computed style wrapper refresh should evaluate");

    assert_eq!(result, "0:|true:rgb(0, 128, 0)|0:");
}

#[test]
fn computed_style_wrapper_reflects_constructed_stylesheet_mutations() {
    let mut vm = new_storage_test_vm("https://constructed-sheet-computed-wrapper-refresh.test/");
    let document = vm.document_handle_for_test();
    crate::style_engine::reset_author_source_text_parse_count_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const target = document.createElement('div');
  target.id = 'target';
  body.appendChild(target);
  globalThis.__computedSheet = new CSSStyleSheet();
  globalThis.__computedSheet.replaceSync('#target { color: rgb(0, 128, 0); }');
  document.adoptedStyleSheets = [globalThis.__computedSheet];
  globalThis.__computedStyle = getComputedStyle(target);
  return globalThis.__computedStyle.color;
})()
"#,
        )
        .expect("constructed stylesheet computed wrapper setup should evaluate");

    assert_eq!(initial, "rgb(0, 128, 0)");
    assert_eq!(
        crate::style_engine::author_source_text_parse_count_for_test(),
        0,
        "constructed stylesheet installation must reuse its parsed Stylo stylesheet",
    );
    assert!(vm.computed_style_cache_entry_count_for_document_for_test(document) > 0);
    let generation_after_setup = vm.computed_style_cache_generation_for_document_for_test(document);
    crate::live_stylesheet::reset_live_stylesheet_css_text_projection_count_for_test();
    crate::style_engine::reset_live_stylesheet_update_counts_for_test();

    let mutated = vm
        .eval(
            r#"
(() => {
  const states = [];
  globalThis.__computedSheet.replaceSync('#target { color: rgb(1, 2, 3); }');
  states.push(globalThis.__computedStyle.color);
  globalThis.__computedSheet.insertRule(
    '#target { color: rgb(4, 5, 6); }',
    globalThis.__computedSheet.cssRules.length
  );
  states.push(globalThis.__computedStyle.color);
  globalThis.__computedSheet.deleteRule(1);
  states.push(globalThis.__computedStyle.color);
  globalThis.__computedSheet.disabled = true;
  states.push(globalThis.__computedStyle.color);
  globalThis.__computedSheet.disabled = false;
  states.push(globalThis.__computedStyle.color);
  delete globalThis.__computedStyle;
  delete globalThis.__computedSheet;
  return states.join('|');
})()
"#,
        )
        .expect("constructed stylesheet computed wrapper mutations should evaluate");

    assert_eq!(
        mutated,
        "rgb(1, 2, 3)|rgb(4, 5, 6)|rgb(1, 2, 3)|rgb(0, 0, 0)|rgb(1, 2, 3)"
    );
    assert_eq!(
        crate::style_engine::author_source_text_parse_count_for_test(),
        0,
        "constructed stylesheet mutations must not rebuild an author stylesheet from cssText",
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_css_text_projection_count_for_test(),
        0,
        "constructed stylesheet mutations must not serialize a compatibility source snapshot",
    );
    assert_eq!(
        vm.computed_style_cache_generation_for_document_for_test(document),
        generation_after_setup
    );
    assert!(vm.computed_style_cache_entry_count_for_document_for_test(document) > 0);
    assert_eq!(
        crate::style_engine::exact_rule_change_notification_count_for_test(),
        2,
        "insertRule and deleteRule should use exact Stylo rule notifications",
    );
    assert_eq!(
        crate::style_engine::full_cascade_update_fallback_count_for_test(),
        3,
        "whole-sheet replacement and disabled toggles should remain explicit full-update fallbacks",
    );
}

#[test]
fn one_constructed_stylesheet_mutation_reaches_document_and_all_shadow_adopters() {
    let mut vm = new_storage_test_vm("https://constructed-sheet-all-adopters.test/");
    crate::style_engine::reset_author_source_text_parse_count_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const documentTarget = body.appendChild(document.createElement('span'));
  documentTarget.className = 'shared-target';

  function makeShadowTarget() {
    const host = body.appendChild(document.createElement('section'));
    const shadow = host.attachShadow({ mode: 'open' });
    const target = shadow.appendChild(document.createElement('span'));
    target.className = 'shared-target';
    return { shadow, target };
  }

  const first = makeShadowTarget();
  const second = makeShadowTarget();
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.shared-target { color: rgb(1, 2, 3); }');
  document.adoptedStyleSheets = [sheet];
  first.shadow.adoptedStyleSheets = [sheet];
  second.shadow.adoptedStyleSheets = [sheet];

  globalThis.__allAdopterSheet = sheet;
  globalThis.__allAdopterStyles = [
    getComputedStyle(documentTarget),
    getComputedStyle(first.target),
    getComputedStyle(second.target)
  ];
  return globalThis.__allAdopterStyles.map(style => style.color).join('|');
})()
"#,
        )
        .expect("shared constructed stylesheet setup should evaluate");

    assert_eq!(initial, "rgb(1, 2, 3)|rgb(1, 2, 3)|rgb(1, 2, 3)");
    crate::live_stylesheet::reset_live_stylesheet_css_text_projection_count_for_test();
    crate::style_engine::reset_live_stylesheet_update_counts_for_test();

    let mutated = vm
        .eval(
            r#"
(() => {
  const states = [];
  globalThis.__allAdopterSheet.cssRules[0].style.color = 'rgb(4, 5, 6)';
  states.push(globalThis.__allAdopterStyles.map(style => style.color).join(','));
  globalThis.__allAdopterSheet.insertRule(
    '.shared-target { color: rgb(7, 8, 9); }',
    globalThis.__allAdopterSheet.cssRules.length
  );
  states.push(globalThis.__allAdopterStyles.map(style => style.color).join(','));
  globalThis.__allAdopterSheet.deleteRule(0);
  states.push(globalThis.__allAdopterStyles.map(style => style.color).join(','));
  return states.join('|');
})()
"#,
        )
        .expect("all constructed stylesheet adopters should observe mutations");

    assert_eq!(
        mutated,
        "rgb(4, 5, 6),rgb(4, 5, 6),rgb(4, 5, 6)|rgb(7, 8, 9),rgb(7, 8, 9),rgb(7, 8, 9)|rgb(7, 8, 9),rgb(7, 8, 9),rgb(7, 8, 9)"
    );
    assert_eq!(
        crate::style_engine::author_source_text_parse_count_for_test(),
        0,
        "all adopter clients must consume the shared parsed stylesheet",
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_css_text_projection_count_for_test(),
        0,
        "all adopter notifications must stay on the parsed stylesheet path",
    );
    assert_eq!(
        crate::style_engine::exact_rule_change_notification_count_for_test(),
        9,
        "three exact CSSOM mutations must be delivered independently to the Document and two ShadowRoot installations",
    );
    assert_eq!(
        crate::style_engine::full_cascade_update_fallback_count_for_test(),
        0,
        "ordinary rule/declaration mutations must not dirty an entire author scope",
    );
}

#[test]
fn live_stylesheet_rule_journal_batches_multiple_mutations_until_observation() {
    let mut vm = new_storage_test_vm("https://constructed-sheet-rule-journal.test/");

    assert_eq!(
        vm.eval(
            r#"
const target = document.createElement('div');
target.className = 'journal-target';
(document.body || document.documentElement || document).appendChild(target);
globalThis.__journalSheet = new CSSStyleSheet();
globalThis.__journalSheet.replaceSync('.journal-target { color: rgb(1, 2, 3); }');
document.adoptedStyleSheets = [globalThis.__journalSheet];
getComputedStyle(target).color;
"#,
        )
        .expect("rule journal setup should evaluate"),
        "rgb(1, 2, 3)"
    );
    crate::style_engine::reset_live_stylesheet_update_counts_for_test();

    assert_eq!(
        vm.eval(
            r#"
__journalSheet.cssRules[0].style.color = 'rgb(4, 5, 6)';
__journalSheet.insertRule('.journal-target { color: rgb(7, 8, 9); }', 1);
__journalSheet.deleteRule(0);
getComputedStyle(target).color;
"#,
        )
        .expect("batched rule journal mutations should evaluate"),
        "rgb(7, 8, 9)"
    );
    assert_eq!(
        crate::style_engine::exact_rule_change_notification_count_for_test(),
        3,
        "all generations since the last observation must be replayed in order",
    );
    assert_eq!(
        crate::style_engine::full_cascade_update_fallback_count_for_test(),
        0,
    );
}

#[test]
fn nested_live_rule_journal_preserves_grouping_rule_ancestors() {
    let mut vm = new_storage_test_vm("https://nested-live-rule-journal.test/");

    assert_eq!(
        vm.eval(
            r#"
const style = document.createElement('style');
style.textContent = '@media all { .nested-journal-target { color: rgb(1, 2, 3); } }';
(document.head || document.documentElement || document).appendChild(style);
const target = document.createElement('div');
target.className = 'nested-journal-target';
(document.body || document.documentElement || document).appendChild(target);
globalThis.__nestedJournalRule = style.sheet.cssRules[0].cssRules[0];
getComputedStyle(target).color;
"#,
        )
        .expect("nested rule journal setup should evaluate"),
        "rgb(1, 2, 3)"
    );
    crate::style_engine::reset_live_stylesheet_update_counts_for_test();

    assert_eq!(
        vm.eval(
            r#"
__nestedJournalRule.style.color = 'rgb(4, 5, 6)';
getComputedStyle(document.querySelector('.nested-journal-target')).color;
"#,
        )
        .expect("nested declaration mutation should evaluate"),
        "rgb(4, 5, 6)"
    );
    assert_eq!(
        crate::style_engine::exact_rule_change_notification_count_for_test(),
        1,
    );
    assert_eq!(
        crate::style_engine::full_cascade_update_fallback_count_for_test(),
        0,
        "a nested rule with a retained @media ancestor must remain exactly expressible",
    );
}

#[test]
fn nested_declaration_mutations_invalidate_their_ancestor_style_selector() {
    let mut vm = new_storage_test_vm("https://nested-declaration-invalidation.test/");

    assert_eq!(
        vm.eval(
            r#"
const style = document.createElement('style');
style.textContent = 'div { z-index: 1; &.test { } }';
(document.head || document.documentElement || document).appendChild(style);
const target = document.createElement('div');
target.className = 'test';
(document.body || document.documentElement || document).appendChild(target);
globalThis.__nestedDeclarationRule = style.sheet.cssRules[0];
globalThis.__nestedDeclarationTarget = target;
getComputedStyle(target).zIndex;
"#,
        )
        .expect("nested declaration invalidation setup should evaluate"),
        "1"
    );
    crate::style_engine::reset_live_stylesheet_update_counts_for_test();

    assert_eq!(
        vm.eval(
            r#"
const rule = globalThis.__nestedDeclarationRule;
const mutationTarget = globalThis.__nestedDeclarationTarget;
const states = [];

rule.insertRule('z-index: 3;', 0);
const declarations = rule.cssRules[0];
states.push(declarations instanceof CSSNestedDeclarations);
states.push(getComputedStyle(mutationTarget).zIndex);

declarations.style.zIndex = '4';
states.push(getComputedStyle(mutationTarget).zIndex);

rule.deleteRule(0);
states.push(getComputedStyle(mutationTarget).zIndex);

rule.insertRule('@media all { a { } }', 1);
const media = rule.cssRules[1];
media.insertRule('z-index: 5;', 0);
states.push(media.cssRules[0] instanceof CSSNestedDeclarations);
states.push(getComputedStyle(mutationTarget).zIndex);

media.deleteRule(0);
states.push(getComputedStyle(mutationTarget).zIndex);
states.join('|');
"#,
        )
        .expect("nested declaration mutations should invalidate ancestor selectors"),
        "true|3|4|1|true|5|1"
    );
    assert_eq!(
        crate::style_engine::exact_rule_change_notification_count_for_test(),
        6,
        "nested declaration changes should remain exact journal updates",
    );
    assert_eq!(
        crate::style_engine::full_cascade_update_fallback_count_for_test(),
        0,
        "nested declaration changes must not rebuild an entire stylesheet",
    );
}

#[test]
fn computed_style_wrapper_reflects_style_element_media_mutations() {
    let mut vm = new_storage_test_vm("https://style-media-computed-wrapper-refresh.test/");
    let document = vm.document_handle_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const target = document.createElement('div');
  target.className = 'target';
  body.appendChild(target);

  const style = document.createElement('style');
  style.media = 'print';
  style.textContent = '.target { color: rgb(1, 2, 3); }';
  head.appendChild(style);

  const sheetMedia = style.sheet.media;
  const computed = getComputedStyle(target);
  const initial = computed.color;
  const initialMedia = sheetMedia.mediaText;
  style.media = 'screen';
  const active = computed.color;
  const activeMedia = sheetMedia.mediaText;
  style.media = 'print';
  const inactive = computed.color;
  const inactiveMedia = sheetMedia.mediaText;
  style.removeAttribute('media');
  const restored = computed.color;
  const restoredMedia = sheetMedia.mediaText;
  return [
    initial,
    active,
    inactive,
    restored,
    initialMedia,
    activeMedia,
    inactiveMedia,
    restoredMedia,
  ].join('|');
})()
"#,
        )
        .expect("style media computed wrapper refresh should evaluate");

    assert_eq!(
        result,
        "rgb(0, 0, 0)|rgb(1, 2, 3)|rgb(0, 0, 0)|rgb(1, 2, 3)|print|screen|print|"
    );
    assert!(vm.computed_style_cache_entry_count_for_document_for_test(document) > 0);
}

#[test]
fn shadow_style_media_mutations_update_only_the_retained_tree_scope() {
    let mut vm = new_storage_test_vm("https://shadow-style-media-refresh.test/");
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const body = document.body || document.documentElement || document;
  const host = document.createElement('div');
  body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open' });
  const style = document.createElement('style');
  style.media = 'print';
  style.textContent = '.target { color: rgb(4, 5, 6); }';
  const target = document.createElement('div');
  target.className = 'target';
  shadow.append(style, target);
  globalThis.__shadowMediaStyle = style;
  globalThis.__shadowMediaComputed = getComputedStyle(target);
  return __shadowMediaComputed.color;
})()
"#,
        )
        .expect("shadow style media fixture should evaluate");

    assert_eq!(initial, "rgb(0, 0, 0)");
    let stylist_identity = vm.retained_stylist_identity_for_document_for_test(document);
    let rebuilds = vm.retained_style_system_rebuild_count_for_document_for_test(document);
    let updates = vm.retained_style_system_update_count_for_document_for_test(document);

    let result = vm
        .eval(
            r#"
(() => {
  __shadowMediaStyle.media = 'screen';
  const active = __shadowMediaComputed.color;
  __shadowMediaStyle.media = 'print';
  const inactive = __shadowMediaComputed.color;
  __shadowMediaStyle.removeAttribute('media');
  const restored = __shadowMediaComputed.color;
  return [active, inactive, restored].join('|');
})()
"#,
        )
        .expect("shadow style media mutations should evaluate");

    assert_eq!(result, "rgb(4, 5, 6)|rgb(0, 0, 0)|rgb(4, 5, 6)");
    assert_eq!(
        vm.retained_stylist_identity_for_document_for_test(document),
        stylist_identity,
        "ShadowRoot media changes must retain the document Stylist"
    );
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(document),
        rebuilds,
        "ShadowRoot media changes must not rebuild the style world"
    );
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(document),
        updates + 3,
        "each observed media mutation must update the retained ShadowRoot scope once"
    );
}

#[test]
fn computed_style_wrapper_reflects_emulated_media_changes() {
    let mut vm = new_storage_test_vm("https://emulated-media-computed-style.test/");

    let initial = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const target = document.createElement('div');
  target.className = 'target';
  body.appendChild(target);

  const style = document.createElement('style');
  style.textContent = `
    .target { color: rgb(0, 128, 0); background-color: rgb(255, 255, 255); }
    @media print { .target { color: rgb(1, 2, 3); } }
    @media (prefers-color-scheme: dark) { .target { background-color: rgb(4, 5, 6); } }
  `;
  head.appendChild(style);

  globalThis.__emulatedMediaComputedStyle = getComputedStyle(target);
  return `${globalThis.__emulatedMediaComputedStyle.color}|${globalThis.__emulatedMediaComputedStyle.backgroundColor}`;
})()
"#,
        )
        .expect("emulated media computed style setup should evaluate");

    assert_eq!(initial, "rgb(0, 128, 0)|rgb(255, 255, 255)");

    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides {
        media: Some("print".to_owned()),
        color_scheme: Some("dark".to_owned()),
        ..Default::default()
    });
    let emulated = vm
        .eval(
            r#"
`${globalThis.__emulatedMediaComputedStyle.color}|${globalThis.__emulatedMediaComputedStyle.backgroundColor}`
"#,
        )
        .expect("emulated media computed style should refresh");

    assert_eq!(emulated, "rgb(1, 2, 3)|rgb(4, 5, 6)");

    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides::default());
    let restored = vm
        .eval(
            r#"
`${globalThis.__emulatedMediaComputedStyle.color}|${globalThis.__emulatedMediaComputedStyle.backgroundColor}`
"#,
        )
        .expect("restored emulated media computed style should refresh");

    assert_eq!(restored, "rgb(0, 128, 0)|rgb(255, 255, 255)");
}

#[test]
fn computed_style_wrapper_reflects_shadow_and_child_constructed_stylesheet_mutations() {
    let mut vm =
        new_storage_test_vm("https://constructed-sheet-cross-context-wrapper-refresh.test/");
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));

  const host = document.createElement('section');
  body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open' });
  const shadowTarget = document.createElement('span');
  shadowTarget.id = 'shadow-target';
  shadow.appendChild(shadowTarget);
  globalThis.__shadowComputedSheet = new CSSStyleSheet();
  globalThis.__shadowComputedSheet.replaceSync('#shadow-target { color: rgb(0, 128, 0); }');
  shadow.adoptedStyleSheets = [globalThis.__shadowComputedSheet];
  globalThis.__shadowComputedStyle = getComputedStyle(shadowTarget);

  const frame = document.createElement('iframe');
  frame.id = 'constructed-sheet-frame';
  body.appendChild(frame);
  const childWindow = frame.contentWindow;
  const childDocument = childWindow.document;
  childDocument.open();
  childDocument.write('<body><span id="child-target">child</span></body>');
  childDocument.close();
  globalThis.__childComputedSheet = new childWindow.CSSStyleSheet();
  globalThis.__childComputedSheet.replaceSync('#child-target { color: rgb(0, 0, 255); }');
  childDocument.adoptedStyleSheets = [globalThis.__childComputedSheet];
  globalThis.__childComputedStyle =
    childWindow.getComputedStyle(childDocument.getElementById('child-target'));

  return [
    globalThis.__shadowComputedStyle.color,
    globalThis.__childComputedStyle.color
  ].join('|');
})()
"#,
        )
        .expect("cross-context constructed stylesheet setup should evaluate");

    assert_eq!(initial, "rgb(0, 128, 0)|rgb(0, 0, 255)");
    let child_document = child_document_handle_for_frame_id(&vm, "constructed-sheet-frame");
    let document_generation_after_setup =
        vm.computed_style_cache_generation_for_document_for_test(document);
    let child_generation_after_setup =
        vm.computed_style_cache_generation_for_document_for_test(child_document);

    let mutated = vm
        .eval(
            r#"
(() => {
  const states = [];
  globalThis.__shadowComputedSheet.replaceSync('#shadow-target { color: rgb(1, 2, 3); }');
  states.push(globalThis.__shadowComputedStyle.color);
  globalThis.__shadowComputedSheet.disabled = true;
  states.push(globalThis.__shadowComputedStyle.color);
  globalThis.__shadowComputedSheet.disabled = false;
  states.push(globalThis.__shadowComputedStyle.color);

  globalThis.__childComputedSheet.replaceSync('#child-target { color: rgb(4, 5, 6); }');
  states.push(globalThis.__childComputedStyle.color);
  globalThis.__childComputedSheet.disabled = true;
  states.push(globalThis.__childComputedStyle.color);
  globalThis.__childComputedSheet.disabled = false;
  states.push(globalThis.__childComputedStyle.color);

  delete globalThis.__shadowComputedStyle;
  delete globalThis.__shadowComputedSheet;
  delete globalThis.__childComputedStyle;
  delete globalThis.__childComputedSheet;
  return states.join('|');
})()
"#,
        )
        .expect("cross-context constructed stylesheet mutations should evaluate");

    assert_eq!(
        mutated,
        "rgb(1, 2, 3)|rgb(0, 0, 0)|rgb(1, 2, 3)|rgb(4, 5, 6)|rgb(0, 0, 0)|rgb(4, 5, 6)"
    );
    assert_eq!(
        vm.computed_style_cache_generation_for_document_for_test(document),
        document_generation_after_setup,
        "shadow stylesheet mutation should use scoped cache invalidation",
    );
    assert_eq!(
        vm.computed_style_cache_generation_for_document_for_test(child_document),
        child_generation_after_setup,
        "child-document stylesheet mutation should not clear the whole document cache",
    );
}

#[test]
fn child_frame_shadow_adopted_stylesheet_change_uses_child_document_world() {
    let mut vm = new_storage_test_vm("https://child-shadow-adopted-cache.test/");

    let setup = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));

  const activeStyle = document.createElement('style');
  activeStyle.textContent = '#active-shadow-adopted-cache { color: rgb(1, 2, 3); }';
  head.appendChild(activeStyle);
  const active = document.createElement('div');
  active.id = 'active-shadow-adopted-cache';
  body.appendChild(active);
  globalThis.__childShadowActiveComputed = getComputedStyle(active);

  const frame = document.createElement('iframe');
  frame.id = 'shadow-adopted-child-frame';
  body.appendChild(frame);
  const childWindow = frame.contentWindow;
  const childDocument = childWindow.document;
  childDocument.open();
  childDocument.write('<body><section id="host"></section></body>');
  childDocument.close();
  const shadow = childDocument.getElementById('host').attachShadow({ mode: 'open' });
  shadow.innerHTML = '<span id="target">target</span>';
  globalThis.__childShadowAdoptedSheet = new childWindow.CSSStyleSheet();
  globalThis.__childShadowAdoptedSheet.replaceSync('#target { color: rgb(4, 5, 6); }');
  shadow.adoptedStyleSheets = [globalThis.__childShadowAdoptedSheet];
  globalThis.__childShadowTargetComputed =
    childWindow.getComputedStyle(shadow.getElementById('target'));

  return [
    globalThis.__childShadowActiveComputed.color,
    globalThis.__childShadowTargetComputed.color
  ].join('|');
})()
"#,
        )
        .expect("child shadow adopted stylesheet setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)|rgb(4, 5, 6)");
    let active_document = vm.document_runtime.dom_host().document_handle();
    let child_document = child_document_handle_for_frame_id(&vm, "shadow-adopted-child-frame");
    let active_cache_before = computed_style_cache_entry_count_for_document(&vm, active_document);
    assert!(active_cache_before > 0);
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, child_document),
        1
    );

    let mutated = vm
        .eval(
            r#"
(() => {
  globalThis.__childShadowAdoptedSheet.replaceSync('#target { color: rgb(7, 8, 9); }');
  const result = [
    globalThis.__childShadowActiveComputed.color,
    globalThis.__childShadowTargetComputed.color
  ].join('|');
  delete globalThis.__childShadowActiveComputed;
  delete globalThis.__childShadowTargetComputed;
  delete globalThis.__childShadowAdoptedSheet;
  return result;
})()
"#,
        )
        .expect("child shadow adopted stylesheet mutation should evaluate");

    assert_eq!(mutated, "rgb(1, 2, 3)|rgb(7, 8, 9)");
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, active_document),
        active_cache_before,
        "child shadow adopted stylesheet changes should not clear active document cache"
    );
    assert!(
        computed_style_cache_entry_count_for_document(&vm, child_document) > 0,
        "child shadow adopted stylesheet changes should stay in child document world"
    );
}

#[test]
fn checked_state_change_invalidates_held_computed_style() {
    let mut vm = new_storage_test_vm("https://computed-style-checked-invalidation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = `
    #target { color: rgb(255, 0, 0); }
    #toggle:checked + #target { color: rgb(0, 128, 0); }`;
  (document.head || document.documentElement || document).appendChild(style);
  const toggle = document.createElement('input');
  toggle.id = 'toggle';
  toggle.type = 'checkbox';
  const target = document.createElement('span');
  target.id = 'target';
  (document.body || document.documentElement || document).appendChild(toggle);
  (document.body || document.documentElement || document).appendChild(target);

  const held = getComputedStyle(target);
  const before = held.color;
  toggle.checked = true;
  const after = held.color;
  return `${before}|${after}`;
})()
"#,
        )
        .expect("checked state style invalidation should evaluate");

    assert_eq!(result, "rgb(255, 0, 0)|rgb(0, 128, 0)");
}

#[test]
fn radio_peer_uncheck_invalidates_held_computed_style() {
    let mut vm = new_storage_test_vm("https://computed-style-radio-peer-invalidation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const style = document.createElement('style');
  style.textContent = `
    .target { color: rgb(255, 0, 0); }
    #first:checked + #firstTarget { color: rgb(0, 128, 0); }
    #second:checked + #secondTarget { color: rgb(0, 0, 255); }`;
  (document.head || document.documentElement || document).appendChild(style);

  const first = document.createElement('input');
  first.id = 'first';
  first.type = 'radio';
  first.name = 'group';
  first.checked = true;
  const firstTarget = document.createElement('span');
  firstTarget.id = 'firstTarget';
  firstTarget.className = 'target';
  const second = document.createElement('input');
  second.id = 'second';
  second.type = 'radio';
  second.name = 'group';
  const secondTarget = document.createElement('span');
  secondTarget.id = 'secondTarget';
  secondTarget.className = 'target';
  document.body.append(first, firstTarget, second, secondTarget);

  const firstStyle = getComputedStyle(firstTarget);
  const secondStyle = getComputedStyle(secondTarget);
  const before = [firstStyle.color, secondStyle.color].join(',');
  second.checked = true;
  const after = [firstStyle.color, secondStyle.color].join(',');
  return `${before}|${after}`;
})()
"#,
        )
        .expect("radio peer checked invalidation should evaluate");

    assert_eq!(
        result,
        "rgb(0, 128, 0),rgb(255, 0, 0)|rgb(255, 0, 0),rgb(0, 0, 255)"
    );
}

#[test]
fn form_reset_input_value_invalidates_held_computed_style() {
    let mut vm = new_storage_test_vm("https://form-reset-value-style-invalidation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = `
    #target { color: rgb(255, 0, 0); }
    #input:placeholder-shown + #target { color: rgb(0, 128, 0); }`;
  (document.head || document.documentElement || document).appendChild(style);

  const form = document.createElement('form');
  const input = document.createElement('input');
  input.id = 'input';
  input.placeholder = 'placeholder';
  input.value = 'typed';
  const target = document.createElement('span');
  target.id = 'target';
  form.append(input, target);
  (document.body || document.documentElement || document).appendChild(form);

  const held = getComputedStyle(target);
  const before = held.color;
  form.reset();
  const after = held.color;
  return `${before}|${input.value}|${after}`;
})()
"#,
        )
        .expect("form reset value style invalidation should evaluate");

    assert_eq!(result, "rgb(255, 0, 0)||rgb(0, 128, 0)");
}

#[test]
fn form_reset_checked_state_invalidates_held_computed_style_and_radio_peer() {
    let mut vm = new_storage_test_vm("https://form-reset-checked-style-invalidation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = `
    .target { color: rgb(255, 0, 0); }
    #first:checked + #firstTarget { color: rgb(0, 128, 0); }
    #second:checked + #secondTarget { color: rgb(0, 0, 255); }`;
  (document.head || document.documentElement || document).appendChild(style);

  const form = document.createElement('form');
  const first = document.createElement('input');
  first.id = 'first';
  first.type = 'radio';
  first.name = 'group';
  first.defaultChecked = true;
  const firstTarget = document.createElement('span');
  firstTarget.id = 'firstTarget';
  firstTarget.className = 'target';
  const second = document.createElement('input');
  second.id = 'second';
  second.type = 'radio';
  second.name = 'group';
  const secondTarget = document.createElement('span');
  secondTarget.id = 'secondTarget';
  secondTarget.className = 'target';
  form.append(first, firstTarget, second, secondTarget);
  (document.body || document.documentElement || document).appendChild(form);

  second.checked = true;
  const firstStyle = getComputedStyle(firstTarget);
  const secondStyle = getComputedStyle(secondTarget);
  const before = [first.checked, second.checked, firstStyle.color, secondStyle.color].join(',');
  form.reset();
  const after = [first.checked, second.checked, firstStyle.color, secondStyle.color].join(',');
  return `${before}|${after}`;
})()
"#,
        )
        .expect("form reset checked style invalidation should evaluate");

    assert_eq!(
        result,
        "false,true,rgb(255, 0, 0),rgb(0, 0, 255)|true,false,rgb(0, 128, 0),rgb(255, 0, 0)"
    );
}

#[test]
fn indeterminate_state_change_invalidates_held_computed_style() {
    let mut vm = new_storage_test_vm("https://computed-style-indeterminate-invalidation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const style = document.createElement('style');
  style.textContent = `
    .target { color: rgb(255, 0, 0); }
    #box:indeterminate + #target { color: rgb(0, 128, 0); }`;
  (document.head || document.documentElement || document).appendChild(style);

  const box = document.createElement('input');
  box.id = 'box';
  box.type = 'checkbox';
  box.indeterminate = true;
  const target = document.createElement('span');
  target.id = 'target';
  target.className = 'target';
  document.body.append(box, target);

  const targetStyle = getComputedStyle(target);
  const before = targetStyle.color;
  box.indeterminate = false;
  const after = targetStyle.color;
  return `${before}|${after}`;
})()
"#,
        )
        .expect("indeterminate state invalidation should evaluate");

    assert_eq!(result, "rgb(0, 128, 0)|rgb(255, 0, 0)");
}
