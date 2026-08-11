use super::*;

fn inspector_active_child_window_scope_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let active = crate::native_bridge::active_child_window_handle(scope).is_some();
    rv.set(v8::Boolean::new(scope, active).into());
}

#[test]
fn menu_uses_block_user_agent_display_default() {
    let mut vm = new_parsed_test_vm(
        "https://menu-user-agent-display.test/",
        r#"<!doctype html>
<html><head><style>#overridden { display: inline; }</style></head><body>
  <menu id="default" type="context"></menu>
  <menu id="overridden" type="context"></menu>
</body></html>"#,
    );

    let result = vm
        .eval(
            r#"
[
  getComputedStyle(document.getElementById('default')).display,
  getComputedStyle(document.getElementById('overridden')).display
].join('|')
"#,
        )
        .expect("menu user-agent display should evaluate");

    assert_eq!(result, "block|inline");
}

#[test]
fn rendered_text_elements_use_chromium_user_agent_defaults() {
    let mut vm = new_parsed_test_vm(
        "https://rendered-text-user-agent-defaults.test/",
        "<!doctype html><body><pre id=pre>text</pre><hr id=hr><optgroup id=optgroup></optgroup>",
    );

    let result = vm
        .eval(
            r#"
['pre', 'hr', 'optgroup']
  .map(id => getComputedStyle(document.getElementById(id)).display)
  .join('|')
"#,
        )
        .expect("rendered-text user-agent defaults should evaluate");

    assert_eq!(result, "block|block|block");
}

#[test]
fn flow_content_uses_chromium_user_agent_typography_defaults() {
    let mut vm = new_parsed_test_vm(
        "https://flow-content-user-agent-defaults.test/",
        r#"<!doctype html>
<html><head><style>
body { font: 16px/1.5 sans-serif; }
.title h1 { font-size: 24px; margin: 19.92px 0; }
</style></head><body>
  <header><a class="title" href="/"><h1 id="title">Title</h1></a></header>
  <main>
    <h1 id="h1">Heading 1</h1>
    <h2 id="h2">Heading 2</h2>
    <p id="paragraph"><strong id="strong">Strong</strong> <a id="link" href="/next">link</a><sup id="sup">1</sup></p>
  </main>
</body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const style = id => getComputedStyle(document.getElementById(id));
  const title = style('title');
  const h1 = style('h1');
  const h2 = style('h2');
  const paragraph = style('paragraph');
  const strong = style('strong');
  const link = style('link');
  const sup = style('sup');
  return [
    title.fontSize, title.fontWeight, title.marginBlockStart, title.marginBlockEnd,
    h1.fontSize, h1.fontWeight, h1.marginBlockStart, h1.marginBlockEnd,
    h2.fontSize, h2.fontWeight, h2.marginBlockStart, h2.marginBlockEnd,
    paragraph.marginBlockStart, paragraph.marginBlockEnd,
    strong.fontWeight,
    link.textDecorationLine, link.cursor,
    sup.fontSize
  ].join('|');
})()
"#,
        )
        .expect("flow-content user-agent typography should evaluate");

    assert_eq!(
        result,
        "24px|700|19.92px|19.92px|32px|700|21.44px|21.44px|24px|700|19.92px|19.92px|16px|16px|700|underline|pointer|13.3281px"
    );
}

#[test]
fn center_uses_legacy_centering_user_agent_default() {
    let mut vm = new_parsed_test_vm(
        "https://center-user-agent-default.test/",
        r#"<!doctype html>
<html><head><style>#overridden { text-align: right; }</style></head><body>
  <center id="default">default</center>
  <center id="overridden">overridden</center>
</body></html>"#,
    );

    let result = vm
        .eval(
            r#"
['default', 'overridden']
  .map(id => getComputedStyle(document.getElementById(id)).textAlign)
  .join('|')
"#,
        )
        .expect("center user-agent alignment should evaluate");

    assert_eq!(result, "-moz-center|right");
}

#[test]
fn list_items_and_first_details_summary_use_user_agent_display_defaults() {
    let mut vm = new_parsed_test_vm(
        "https://list-item-user-agent-display.test/",
        r#"<!doctype html>
<html><head><style>#overridden { display: inline; }</style></head><body>
  <li id="item">item</li>
  <li id="overridden">overridden</li>
  <summary id="outside">outside</summary>
  <details id="details">
    <div>leading content</div>
    <summary id="first">first</summary>
    <summary id="second">second</summary>
  </details>
</body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const display = id => getComputedStyle(document.getElementById(id)).display;
  const values = [
    display('item'),
    display('overridden'),
    display('outside'),
    display('details'),
    display('first'),
    display('second')
  ];
  const details = document.getElementById('details');
  details.insertBefore(document.getElementById('second'), document.getElementById('first'));
  values.push(display('first'), display('second'));
  return values.join('|');
})()
"#,
        )
        .expect("list-item user-agent display should evaluate");

    assert_eq!(
        result,
        "list-item|inline|block|block|list-item|block|block|list-item"
    );
}

#[test]
fn table_elements_use_table_user_agent_display_defaults() {
    let mut vm = new_parsed_test_vm(
        "https://table-user-agent-display.test/",
        r#"<!doctype html>
<table id="table"><caption id="caption">caption</caption><colgroup id="colgroup"><col id="col"></colgroup><thead id="head"><tr id="row"><th id="cell">cell</th></tr></thead><tbody id="body"></tbody><tfoot id="foot"></tfoot></table>"#,
    );

    let result = vm
        .eval(
            r#"
['table', 'caption', 'colgroup', 'col', 'head', 'body', 'foot', 'row', 'cell']
  .map(id => getComputedStyle(document.getElementById(id)).display)
  .join('|')
"#,
        )
        .expect("table user-agent displays should evaluate");

    assert_eq!(
        result,
        "table|table-caption|table-column-group|table-column|table-header-group|table-row-group|table-footer-group|table-row|table-cell"
    );
}

#[test]
fn dialog_user_agent_display_tracks_open_state() {
    let mut vm = new_parsed_test_vm(
        "https://dialog-user-agent-display.test/",
        "<!doctype html><dialog id=target>Dialog</dialog>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const dialog = document.getElementById('target');
  const values = [getComputedStyle(dialog).display];
  dialog.show();
  values.push(getComputedStyle(dialog).display);
  dialog.close();
  values.push(getComputedStyle(dialog).display);
  return values.join('|');
})()
"#,
        )
        .expect("dialog user-agent display should evaluate");

    assert_eq!(result, "none|block|none");
}

#[test]
fn semantic_text_decoration_uses_user_agent_defaults() {
    let mut vm = new_parsed_test_vm(
        "https://semantic-text-decoration.test/",
        r#"<!doctype html>
<style>#overridden { text-decoration: none; }</style>
<u id="u"></u><ins id="ins"></ins>
<s id="s"></s><strike id="strike"></strike><del id="del"></del>
<ins id="overridden"></ins>"#,
    );

    let result = vm
        .eval(
            r#"
['u', 'ins', 's', 'strike', 'del', 'overridden']
  .map(id => getComputedStyle(document.getElementById(id)).textDecorationLine)
  .join('|')
"#,
        )
        .expect("semantic text decoration should evaluate");

    assert_eq!(
        result,
        "underline|underline|line-through|line-through|line-through|none"
    );
}

#[test]
fn computed_style_exposes_root_pointer_events() {
    let mut vm = new_storage_test_vm("https://pointer-events-computed.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const style = document.createElement('style');
  style.textContent = ':root { pointer-events: none; }';
  head.appendChild(style);
  const computed = getComputedStyle(document.documentElement);
  return [
    computed.getPropertyValue('pointer-events'),
    computed.pointerEvents,
    CSS.supports('pointer-events', 'none')
  ].join('|');
})()
"#,
        )
        .expect("root pointer-events computed style should evaluate");

    assert_eq!(result, "none|none|true");
}

fn child_document_handle_for_frame_id(vm: &ScriptVm, frame_id: &str) -> DomHandle {
    let frame = element_handle_by_id(vm, frame_id);
    vm._context_host
        .borrow()
        .child_browsing_context_document_handle(frame)
        .expect("iframe should have a child document handle")
}

fn element_handle_by_id(vm: &ScriptVm, id: &str) -> DomHandle {
    vm.document_runtime
        .dom_host()
        .dom()
        .nodes()
        .iter()
        .enumerate()
        .find_map(|(index, node)| {
            let element = node.as_element()?;
            (element.attribute("id") == Some(id)).then_some(DomHandle::new(index))
        })
        .unwrap_or_else(|| panic!("detached element #{id} should have a native handle"))
}

fn owner_document_handle_for_element_id(vm: &ScriptVm, id: &str) -> DomHandle {
    let element = element_handle_by_id(vm, id);
    vm.document_runtime
        .dom_host()
        .owner_document_handle(element)
        .unwrap_or_else(|| panic!("detached element #{id} should have an owner document"))
}

fn computed_style_cache_entry_count_for_document(vm: &ScriptVm, document: DomHandle) -> usize {
    vm._context_host
        .borrow()
        .computed_style_cache_entry_count_for_document_for_test(document)
}

fn registered_custom_property_for_document(vm: &ScriptVm, document: DomHandle, name: &str) -> bool {
    vm._context_host
        .borrow()
        .registered_css_custom_property_registration(document, name)
        .is_some()
}

#[test]
fn disconnected_element_computed_style_returns_empty_without_caching() {
    let mut vm = new_storage_test_vm("https://disconnected-computed-style.test/page.html");
    let document = vm.document_handle_for_test();
    let initial_cache_count = vm.computed_style_cache_entry_count_for_document_for_test(document);

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const style = document.createElement('style');
  style.textContent = 'div { color: rgb(9, 10, 11); --active-token: active; }';
  head.appendChild(style);

  const detached = document.createElement('div');
  detached.setAttribute('style', 'color: rgb(1, 2, 3); background-image: url(./local.png)');
  const computed = getComputedStyle(detached);
  return JSON.stringify({
    color: computed.getPropertyValue('color'),
    display: computed.getPropertyValue('display'),
    backgroundImage: computed.getPropertyValue('background-image'),
    custom: computed.getPropertyValue('--active-token'),
    length: computed.length,
    cssText: computed.cssText
  });
})()
"#,
        )
        .expect("disconnected computed style should evaluate");

    assert_eq!(
        result,
        r#"{"color":"","display":"","backgroundImage":"","custom":"","length":0,"cssText":""}"#
    );
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        initial_cache_count
    );
}
#[test]
fn detached_document_computed_style_does_not_use_active_document_context() {
    let mut vm = new_storage_test_vm("https://detached-doc-computed-style.test/page.html");
    let document = vm.document_handle_for_test();
    let initial_cache_count = vm.computed_style_cache_entry_count_for_document_for_test(document);

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const base = document.createElement('base');
  base.href = 'https://detached-doc-computed-style.test/active-base/';
  head.appendChild(base);
  const style = document.createElement('style');
  style.textContent = '#probe { color: rgb(7, 8, 9); background-image: url(active.png); --active-token: active; }';
  head.appendChild(style);

  const detachedDoc = document.implementation.createHTMLDocument('');
  const target = detachedDoc.createElement('div');
  target.id = 'probe';
  target.setAttribute('style', 'color: rgb(1, 2, 3); background-image: url(detached.png); --local-token: local;');
  detachedDoc.body.appendChild(target);
  const computed = getComputedStyle(target);
  return JSON.stringify({
    color: computed.getPropertyValue('color'),
    backgroundImage: computed.getPropertyValue('background-image'),
    activeCustom: computed.getPropertyValue('--active-token'),
    localCustom: computed.getPropertyValue('--local-token'),
    length: computed.length
  });
})()
"#,
        )
        .expect("detached document computed style should evaluate");

    assert_eq!(
        result,
        r#"{"color":"","backgroundImage":"","activeCustom":"","localCustom":"","length":0}"#
    );
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        initial_cache_count
    );
}
#[test]
fn adopted_detached_document_node_reenters_and_leaves_active_style_context() {
    let mut vm = new_storage_test_vm("https://adopted-detached-computed-style.test/page.html");
    let document = vm.document_handle_for_test();

    let before = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '#adopted-target { color: rgb(30, 31, 32); --active-token: active; }';
  head.appendChild(style);

  globalThis.__adoptedDetachedDocument = document.implementation.createHTMLDocument('');
  globalThis.__adoptedDetachedTarget = globalThis.__adoptedDetachedDocument.createElement('div');
  globalThis.__adoptedDetachedTarget.id = 'adopted-target';
  globalThis.__adoptedDetachedTarget.setAttribute('style', '--local-token: local;');
  globalThis.__adoptedDetachedDocument.body.appendChild(globalThis.__adoptedDetachedTarget);
  const detached = getComputedStyle(globalThis.__adoptedDetachedTarget);
  const detachedColor = detached.color;
  const detachedLength = detached.length;

  document.adoptNode(globalThis.__adoptedDetachedTarget);
  body.appendChild(globalThis.__adoptedDetachedTarget);
  const active = getComputedStyle(globalThis.__adoptedDetachedTarget);
  globalThis.__adoptedDetachedHeldStyle = active;
  return JSON.stringify({
    detachedColor,
    detachedLength,
    activeColor: active.color,
    activeCustom: active.getPropertyValue('--active-token'),
    ownerIsActive: globalThis.__adoptedDetachedTarget.ownerDocument === document
  });
})()
"#,
        )
        .expect("adopted detached computed style setup should evaluate");

    assert_eq!(
        before,
        r#"{"detachedColor":"","detachedLength":0,"activeColor":"rgb(30, 31, 32)","activeCustom":"active","ownerIsActive":true}"#
    );
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let after = vm
        .eval(
            r#"
(() => {
  globalThis.__adoptedDetachedDocument.adoptNode(globalThis.__adoptedDetachedTarget);
  globalThis.__adoptedDetachedDocument.body.appendChild(globalThis.__adoptedDetachedTarget);
  const detached = getComputedStyle(globalThis.__adoptedDetachedTarget);
  const held = globalThis.__adoptedDetachedHeldStyle;
  const ownerIsDetached =
    globalThis.__adoptedDetachedTarget.ownerDocument === globalThis.__adoptedDetachedDocument;
  delete globalThis.__adoptedDetachedTarget;
  delete globalThis.__adoptedDetachedDocument;
  delete globalThis.__adoptedDetachedHeldStyle;
  return JSON.stringify({
    color: detached.color,
    activeCustom: detached.getPropertyValue('--active-token'),
    localCustom: detached.getPropertyValue('--local-token'),
    length: detached.length,
    heldColor: held.color,
    heldLength: held.length,
    ownerIsDetached
  });
})()
"#,
        )
        .expect("adopted node returning to detached document should evaluate");

    assert_eq!(
        after,
        r#"{"color":"","activeCustom":"","localCustom":"","length":0,"heldColor":"","heldLength":0,"ownerIsDetached":true}"#
    );
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
}
#[test]
fn removed_element_computed_style_is_empty_until_reattached() {
    let mut vm = new_storage_test_vm("https://removed-computed-style-cache.test/page.html");
    let document = vm.document_handle_for_test();

    let connected = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '#target { color: rgb(11, 12, 13); }';
  head.appendChild(style);
  globalThis.__removedStyleTarget = document.createElement('div');
  globalThis.__removedStyleTarget.id = 'target';
  body.appendChild(globalThis.__removedStyleTarget);
  return getComputedStyle(globalThis.__removedStyleTarget).color;
})()
"#,
        )
        .expect("removed computed style setup should evaluate");

    assert_eq!(connected, "rgb(11, 12, 13)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let removed = vm
        .eval(
            r#"
(() => {
  globalThis.__removedStyleTarget.remove();
  const computed = getComputedStyle(globalThis.__removedStyleTarget);
  return `${computed.color}|${computed.length}`;
})()
"#,
        )
        .expect("removed computed style should evaluate");

    assert_eq!(removed, "|0");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );

    let reattached = vm
        .eval(
            r#"
(() => {
  (document.body || document.documentElement || document).appendChild(globalThis.__removedStyleTarget);
  const value = getComputedStyle(globalThis.__removedStyleTarget).color;
  delete globalThis.__removedStyleTarget;
  return value;
})()
"#,
        )
        .expect("reattached computed style should evaluate");

    assert_eq!(reattached, "rgb(11, 12, 13)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
}
#[test]
fn get_computed_style_wrapper_creation_drains_pending_style_invalidations() {
    let mut vm = new_storage_test_vm("https://computed-style-wrapper-drain.test/");
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '.active { color: rgb(10, 20, 30); }';
  head.appendChild(style);
  globalThis.__wrapperDrainTarget = document.createElement('div');
  body.appendChild(globalThis.__wrapperDrainTarget);
  return getComputedStyle(globalThis.__wrapperDrainTarget).color;
})()
"#,
        )
        .expect("computed style wrapper drain setup should evaluate");

    assert_eq!(initial, "rgb(0, 0, 0)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let wrapped = vm
        .eval(
            r#"
(() => {
  globalThis.__wrapperDrainTarget.setAttribute('class', 'active');
  globalThis.__wrapperDrainComputed = getComputedStyle(globalThis.__wrapperDrainTarget);
  return 'wrapped';
})()
"#,
        )
        .expect("computed style wrapper creation should evaluate");

    assert_eq!(wrapped, "wrapped");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );

    let resolved = vm
        .eval(
            r#"
(() => {
  const color = globalThis.__wrapperDrainComputed.color;
  delete globalThis.__wrapperDrainComputed;
  delete globalThis.__wrapperDrainTarget;
  return color;
})()
"#,
        )
        .expect("held computed style should resolve after wrapper drain");

    assert_eq!(resolved, "rgb(10, 20, 30)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
}
#[test]
fn runtime_eval_turn_drains_pending_style_invalidations_without_computed_style_read() {
    let mut vm = new_storage_test_vm("https://runtime-evaluate-style-invalidation-drain.test/");
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '.active { color: rgb(40, 50, 60); }';
  head.appendChild(style);
  globalThis.__runtimeEvalDrainTarget = document.createElement('div');
  body.appendChild(globalThis.__runtimeEvalDrainTarget);
  return getComputedStyle(globalThis.__runtimeEvalDrainTarget).color;
})()
"#,
        )
        .expect("runtime evaluate style drain setup should evaluate");

    assert_eq!(initial, "rgb(0, 0, 0)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let mutated = vm
        .eval(
            r#"
(() => {
  globalThis.__runtimeEvalDrainTarget.className = 'active';
  return 'mutated';
})()
"#,
        )
        .expect("runtime evaluate style mutation should evaluate");

    assert_eq!(mutated, "mutated");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );

    let resolved = vm
        .eval(
            r#"
(() => {
  const color = getComputedStyle(globalThis.__runtimeEvalDrainTarget).color;
  delete globalThis.__runtimeEvalDrainTarget;
  return color;
})()
"#,
        )
        .expect("runtime evaluate drained style should resolve");

    assert_eq!(resolved, "rgb(40, 50, 60)");
}
#[test]
fn isolated_runtime_eval_turn_drains_pending_style_invalidations_without_computed_style_read() {
    let mut vm =
        new_storage_test_vm("https://isolated-runtime-evaluate-style-invalidation-drain.test/");
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '.active { color: rgb(50, 60, 70); }';
  head.appendChild(style);
  const target = document.createElement('div');
  target.id = 'isolated-runtime-eval-drain-target';
  body.appendChild(target);
  return getComputedStyle(target).color;
})()
"#,
        )
        .expect("isolated runtime evaluate style drain setup should evaluate");

    assert_eq!(initial, "rgb(0, 0, 0)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let context_id = vm
        .create_isolated_world("style-drain-test", false)
        .expect("isolated world should be created");
    let mutated = vm
        .eval_in_isolated_context(
            context_id,
            r#"
(() => {
  document.getElementById('isolated-runtime-eval-drain-target').className = 'active';
  return 'mutated';
})()
"#,
        )
        .expect("isolated runtime evaluate style mutation should evaluate");

    assert_eq!(mutated, "mutated");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
}
#[test]
fn isolated_runtime_exec_turn_drains_pending_style_invalidations_without_computed_style_read() {
    let mut vm =
        new_storage_test_vm("https://isolated-runtime-exec-style-invalidation-drain.test/");
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '.active { color: rgb(70, 80, 90); }';
  head.appendChild(style);
  const target = document.createElement('div');
  target.id = 'isolated-runtime-exec-drain-target';
  body.appendChild(target);
  return getComputedStyle(target).color;
})()
"#,
        )
        .expect("isolated runtime exec style drain setup should evaluate");

    assert_eq!(initial, "rgb(0, 0, 0)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let context_id = vm
        .create_isolated_world("style-exec-drain-test", false)
        .expect("isolated world should be created");
    vm.exec_in_execution_context(
        context_id,
        r#"
document.getElementById('isolated-runtime-exec-drain-target').className = 'active';
"#,
    )
    .expect("isolated runtime exec style mutation should execute");

    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
}
#[test]
fn document_start_run_immediately_drains_pending_style_invalidations_without_computed_style_read() {
    let mut vm = new_storage_test_vm("https://document-start-run-immediately-style-drain.test/");
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '.active { color: rgb(80, 90, 100); }';
  head.appendChild(style);
  globalThis.__runImmediatelyDrainTarget = document.createElement('div');
  body.appendChild(globalThis.__runImmediatelyDrainTarget);
  return getComputedStyle(globalThis.__runImmediatelyDrainTarget).color;
})()
"#,
        )
        .expect("run-immediately style drain setup should evaluate");

    assert_eq!(initial, "rgb(0, 0, 0)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let result = vm
        .run_document_start_script_now(&crate::DocumentStartScript {
            registry_key: None,
            source: "globalThis.__runImmediatelyDrainTarget.className = 'active';".to_owned(),
            world_name: None,
            has_bidi_channel_argument: false,
            bidi_channel_handoffs: Vec::new(),
        })
        .expect("run-immediately document-start script should execute");
    assert_eq!(result, None);

    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
}
#[test]
fn child_default_held_computed_style_reflects_class_mutation() {
    let mut vm = new_storage_test_vm("https://child-held-computed-probe.test/");

    let created = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const frame = document.createElement('iframe');
  frame.id = 'child-held-style-frame';
  body.appendChild(frame);
  return 'created';
})()
"#,
        )
        .expect("child frame setup should evaluate");
    assert_eq!(created, "created");

    vm.drain_pending_child_frame_work_for_test();
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child held computed-style setup");
    let child_document = child_document_handle_for_frame_id(&vm, "child-held-style-frame");

    let initial = vm
        .eval_in_child_default_context(
            child_context_id,
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '.active { color: rgb(60, 70, 80); }';
  head.appendChild(style);
  globalThis.__childHeldStyleTarget = document.createElement('div');
  body.appendChild(globalThis.__childHeldStyleTarget);
  globalThis.__childHeldStyle = getComputedStyle(globalThis.__childHeldStyleTarget);
  return globalThis.__childHeldStyle.color;
})()
"#,
        )
        .expect("child held style setup should evaluate");
    assert_eq!(initial, "rgb(0, 0, 0)");
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, child_document),
        1
    );

    let mutated = vm
        .eval_in_child_default_context(
            child_context_id,
            r#"
(() => {
  globalThis.__childHeldStyleTarget.className = 'active';
  return 'mutated';
})()
"#,
        )
        .expect("child held style mutation should evaluate");
    assert_eq!(mutated, "mutated");
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, child_document),
        0
    );

    let after = vm
        .eval_in_child_default_context(
            child_context_id,
            r#"
(() => globalThis.__childHeldStyle.color)()
"#,
        )
        .expect("child held style readback should evaluate");
    assert_eq!(after, "rgb(60, 70, 80)");
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, child_document),
        1
    );
}
#[tokio::test]
async fn host_task_turn_drains_pending_style_invalidations_without_computed_style_read() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader(
        "https://host-task-style-invalidation-drain.test/",
        &loader,
    );
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '.active { color: rgb(20, 30, 40); }';
  head.appendChild(style);
  globalThis.__hostTaskDrainTarget = document.createElement('div');
  body.appendChild(globalThis.__hostTaskDrainTarget);
  return getComputedStyle(globalThis.__hostTaskDrainTarget).color;
})()
"#,
        )
        .expect("host task style drain setup should evaluate");

    assert_eq!(initial, "rgb(0, 0, 0)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    vm.eval(
        r#"
setTimeout(() => {
  globalThis.__hostTaskDrainTarget.className = 'active';
}, 0);
'queued';
"#,
    )
    .expect("host task style mutation should queue");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    assert!(
        vm.apply_next_connected_style_event_body_for_test(),
        "the inline setup stylesheet queues its own event body before the timer"
    );
    assert!(
        vm.run_next_due_timer_callback_for_test(&loader)
            .await
            .expect("exact timer task should run")
    );
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );

    let resolved = vm
        .eval(
            r#"
(() => {
  const color = getComputedStyle(globalThis.__hostTaskDrainTarget).color;
  delete globalThis.__hostTaskDrainTarget;
  return color;
})()
"#,
        )
        .expect("host task drained style should resolve");

    assert_eq!(resolved, "rgb(20, 30, 40)");
}
#[test]
fn computed_color_resolves_simple_custom_property_fallbacks() {
    let mut vm = new_storage_test_vm("https://style-color-custom-property.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const target = document.createElement('span');
  root.append(target);
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('span { color: var(--color, red); }');
  document.adoptedStyleSheets = [sheet];
  const fallback = getComputedStyle(target).color;
  sheet.rules[0].style.setProperty('--color', 'green');
  const resolved = getComputedStyle(target).color;
  return [fallback, resolved].join('|');
})()
"#,
        )
        .expect("computed color custom property fallback should evaluate");

    assert_eq!(result, "rgb(255, 0, 0)|rgb(0, 128, 0)");
}

#[test]
fn computed_style_supports_revert_rule_keyword() {
    let mut vm = new_storage_test_vm("https://revert-rule-computed-style.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = `
    #rule-basic { color: green; }
    #rule-basic { color: red; color: revert-rule; }
    #inline-basic { color: red; }
    #inline-basic { color: green; }
    #z-index-basic { z-index: 1; }
    #z-index-basic { z-index: 2; }
    #z-index-basic { z-index: -1; z-index: revert-rule; }
    #z-index-basic { z-index: -1; z-index: revert-rule; }
    #custom-basic { --a: red; --b: green; }
    #custom-basic { --a: green; --b: revert-rule; }
    #custom-basic { --a: revert-rule; --b: revert-rule; }
  `;
  head.appendChild(style);

  const ruleBasic = document.createElement('div');
  ruleBasic.id = 'rule-basic';
  const inlineBasic = document.createElement('div');
  inlineBasic.id = 'inline-basic';
  inlineBasic.setAttribute('style', 'color:red; color:revert-rule');
  const zIndexBasic = document.createElement('div');
  zIndexBasic.id = 'z-index-basic';
  const customBasic = document.createElement('div');
  customBasic.id = 'custom-basic';
  body.append(ruleBasic, inlineBasic, zIndexBasic, customBasic);

  const customStyle = getComputedStyle(customBasic);
  return [
    CSS.supports('color:revert-rule'),
    CSS.supports('z-index:revert-rule'),
    getComputedStyle(ruleBasic).color,
    getComputedStyle(inlineBasic).color,
    getComputedStyle(zIndexBasic).zIndex,
    customStyle.getPropertyValue('--a'),
    customStyle.getPropertyValue('--b')
  ].join('|');
})()
"#,
        )
        .expect("revert-rule computed style should evaluate");

    assert_eq!(
        result,
        "true|true|rgb(0, 128, 0)|rgb(0, 128, 0)|2|green|green"
    );
}

#[test]
fn focus_without_focus_selectors_preserves_computed_style_cache() {
    let mut vm = new_storage_test_vm("https://focus-style-cache-no-selector.test/");
    let document = vm.document_handle_for_test();

    let setup = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '#outside { color: rgb(1, 2, 3); } #target { color: rgb(4, 5, 6); }';
  head.appendChild(style);

  const outside = document.createElement('div');
  outside.id = 'outside';
  const target = document.createElement('button');
  target.id = 'target';
  body.append(outside, target);
  globalThis.__focusNoSelectorTarget = target;

  return [
    getComputedStyle(outside).color,
    getComputedStyle(target).color
  ].join('|');
})()
"#,
        )
        .expect("focus no-selector style setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)|rgb(4, 5, 6)");
    let generation_before_focus =
        vm.computed_style_cache_generation_for_document_for_test(document);
    let cache_count_before_focus =
        vm.computed_style_cache_entry_count_for_document_for_test(document);

    let focused = vm
        .eval(
            r#"
(() => {
  globalThis.__focusNoSelectorTarget.focus();
  const active = document.activeElement === globalThis.__focusNoSelectorTarget;
  delete globalThis.__focusNoSelectorTarget;
  return String(active);
})()
"#,
        )
        .expect("focus no-selector mutation should evaluate");

    assert_eq!(focused, "true");
    assert_eq!(
        vm.computed_style_cache_generation_for_document_for_test(document),
        generation_before_focus
    );
    let cache_count_after_focus =
        vm.computed_style_cache_entry_count_for_document_for_test(document);
    assert!(
        cache_count_after_focus >= cache_count_before_focus,
        "focus without author focus selectors should not clear existing computed cache entries"
    );
}

#[test]
fn removing_focused_subtree_clears_focus_within_computed_style() {
    let mut vm = new_storage_test_vm("https://focus-within-remove-style.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  root.id = 'html';
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.id = 'body';

  const style = document.createElement('style');
  style.textContent = [
    '* { background-color: white; }',
    ':focus-within { background-color: rgb(1, 2, 3); }'
  ].join('\n');
  head.appendChild(style);

  const test = document.createElement('div');
  test.id = 'test';
  const container = document.createElement('div');
  container.id = 'container1';
  const sibling = document.createElement('div');
  sibling.id = 'sibling2';
  const target = document.createElement('input');
  target.id = 'target1';
  sibling.appendChild(target);
  container.appendChild(sibling);
  test.appendChild(container);
  body.appendChild(test);

  const styled = () => Array.from(document.querySelectorAll('*'))
    .filter((element) => getComputedStyle(element).backgroundColor === 'rgb(1, 2, 3)')
    .map((element) => element.id)
    .join(',');
  const matched = () => Array.from(document.querySelectorAll(':focus-within'))
    .map((element) => element.id)
    .join(',');

  target.focus();
  const before = `${styled()}|${matched()}|${target.matches(':focus')}`;
  container.remove();
  const afterRemove = `${styled()}|${matched()}|${container.querySelectorAll(':focus-within').length}|${target.matches(':focus')}`;
  target.focus();
  const afterDetachedFocus = `${styled()}|${matched()}|${container.querySelectorAll(':focus-within').length}|${target.matches(':focus')}`;
  return `${before}\n${afterRemove}\n${afterDetachedFocus}`;
})()
"#,
        )
        .expect("focus-within subtree removal should evaluate");

    assert_eq!(
        result,
        "html,body,test,container1,sibling2,target1|html,body,test,container1,sibling2,target1|true\n\
||0|false\n\
||0|false"
    );
}

#[test]
fn focus_dependent_selector_invalidates_computed_style_cache() {
    let mut vm = new_storage_test_vm("https://focus-style-cache-selector.test/");
    let document = vm.document_handle_for_test();

    let setup = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = [
    '#sibling { color: rgb(1, 2, 3); }',
    '#target:focus + #sibling { color: rgb(4, 5, 6); }'
  ].join('\n');
  head.appendChild(style);

  const target = document.createElement('button');
  target.id = 'target';
  const sibling = document.createElement('div');
  sibling.id = 'sibling';
  body.append(target, sibling);
  globalThis.__focusSelectorTarget = target;
  globalThis.__focusSelectorSiblingStyle = getComputedStyle(sibling);
  return globalThis.__focusSelectorSiblingStyle.color;
})()
"#,
        )
        .expect("focus selector style setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)");
    let generation_before_focus =
        vm.computed_style_cache_generation_for_document_for_test(document);

    let focused = vm
        .eval(
            r#"
(() => {
  globalThis.__focusSelectorTarget.focus();
  const color = globalThis.__focusSelectorSiblingStyle.color;
  delete globalThis.__focusSelectorTarget;
  delete globalThis.__focusSelectorSiblingStyle;
  return color;
})()
"#,
        )
        .expect("focus selector mutation should evaluate");

    assert_eq!(focused, "rgb(4, 5, 6)");
    assert_eq!(
        vm.computed_style_cache_generation_for_document_for_test(document),
        generation_before_focus,
        "targeted focus invalidation should not bump the retained style generation"
    );
}

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
#[test]
fn text_value_change_invalidates_placeholder_shown_computed_style() {
    let mut vm = new_storage_test_vm("https://computed-style-placeholder-invalidation.test/");

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
    #input:placeholder-shown + #inputTarget { color: rgb(0, 128, 0); }
    #textarea:placeholder-shown + #textareaTarget { color: rgb(0, 0, 255); }`;
  (document.head || document.documentElement || document).appendChild(style);

  const input = document.createElement('input');
  input.id = 'input';
  input.placeholder = 'placeholder';
  const inputTarget = document.createElement('span');
  inputTarget.id = 'inputTarget';
  inputTarget.className = 'target';
  const textarea = document.createElement('textarea');
  textarea.id = 'textarea';
  textarea.placeholder = 'placeholder';
  const textareaTarget = document.createElement('span');
  textareaTarget.id = 'textareaTarget';
  textareaTarget.className = 'target';
  document.body.append(input, inputTarget, textarea, textareaTarget);

  const inputStyle = getComputedStyle(inputTarget);
  const textareaStyle = getComputedStyle(textareaTarget);
  const before = [inputStyle.color, textareaStyle.color].join(',');
  input.value = 'typed';
  textarea.value = 'typed';
  const afterFilled = [inputStyle.color, textareaStyle.color].join(',');
  input.value = '';
  textarea.value = '';
  const afterEmpty = [inputStyle.color, textareaStyle.color].join(',');
  return `${before}|${afterFilled}|${afterEmpty}`;
})()
"#,
        )
        .expect("placeholder-shown value invalidation should evaluate");

    assert_eq!(
        result,
        "rgb(0, 128, 0),rgb(0, 0, 255)|rgb(255, 0, 0),rgb(255, 0, 0)|rgb(0, 128, 0),rgb(0, 0, 255)"
    );
}
#[test]
fn text_value_change_invalidates_validity_computed_style() {
    let mut vm = new_storage_test_vm("https://computed-style-validity-invalidation.test/");

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
    #required:invalid + #invalidTarget { color: rgb(0, 128, 0); }
    #required:valid ~ #validTarget { color: rgb(0, 0, 255); }`;
  (document.head || document.documentElement || document).appendChild(style);

  const required = document.createElement('input');
  required.id = 'required';
  required.required = true;
  const invalidTarget = document.createElement('span');
  invalidTarget.id = 'invalidTarget';
  invalidTarget.className = 'target';
  const validTarget = document.createElement('span');
  validTarget.id = 'validTarget';
  validTarget.className = 'target';

  document.body.append(
    required,
    invalidTarget,
    validTarget
  );

  const invalidStyle = getComputedStyle(invalidTarget);
  const validStyle = getComputedStyle(validTarget);
  const before = [
    invalidStyle.color,
    validStyle.color
  ].join(',');
  required.value = 'filled';
  const after = [
    invalidStyle.color,
    validStyle.color
  ].join(',');
  return `${before}|${after}`;
})()
"#,
        )
        .expect("validity state invalidation should evaluate");

    assert_eq!(
        result,
        "rgb(0, 128, 0),rgb(255, 0, 0)|rgb(255, 0, 0),rgb(0, 0, 255)"
    );
}
#[test]
fn text_value_change_invalidates_range_computed_style() {
    let mut vm = new_storage_test_vm("https://computed-style-range-invalidation.test/");

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
    #range:in-range + #inRangeTarget { color: rgb(0, 128, 0); }
    #range:out-of-range ~ #outRangeTarget { color: rgb(0, 0, 255); }`;
  (document.head || document.documentElement || document).appendChild(style);

  const range = document.createElement('input');
  range.id = 'range';
  range.type = 'number';
  range.min = '0';
  range.max = '10';
  range.value = '5';
  const inRangeTarget = document.createElement('span');
  inRangeTarget.id = 'inRangeTarget';
  inRangeTarget.className = 'target';
  const outRangeTarget = document.createElement('span');
  outRangeTarget.id = 'outRangeTarget';
  outRangeTarget.className = 'target';

  document.body.append(
    range,
    inRangeTarget,
    outRangeTarget
  );

  const inRangeStyle = getComputedStyle(inRangeTarget);
  const outRangeStyle = getComputedStyle(outRangeTarget);
  const before = [
    inRangeStyle.color,
    outRangeStyle.color
  ].join(',');
  range.value = '20';
  const after = [
    inRangeStyle.color,
    outRangeStyle.color
  ].join(',');
  return `${before}|${after}`;
})()
"#,
        )
        .expect("range state invalidation should evaluate");

    assert_eq!(
        result,
        "rgb(0, 128, 0),rgb(255, 0, 0)|rgb(255, 0, 0),rgb(0, 0, 255)"
    );
}
#[test]
fn computed_style_resolves_pseudo_element_argument() {
    let mut vm = new_storage_test_vm("https://computed-pseudo.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = `
    #target { width: 100px; color: rgb(255, 0, 0); }
    #target::before { content: "x"; width: 50%; color: rgb(0, 128, 0); }
    #target::after { content: "y"; width: 25px; color: rgb(0, 0, 255); }
    #target::checkmark { width: 17px; }
    #file::file-selector-button { color: rgb(0, 255, 0); }
    #select::picker(select) { background-color: rgb(0, 128, 0); }
    #select::picker-icon { width: 13px; }
    #picker-div::picker(select) { background-color: rgb(0, 0, 255); }
    #item { display: list-item; }
    #item::marker { color: rgb(10, 20, 30); }
    #input::placeholder { color: rgb(30, 20, 10); }
    #target::highlight(name) { color: rgb(0, 0, 255); }
    #target::highlight(other) { color: rgb(255, 0, 255); }
    #plain { font-style: italic; }`;
  (document.head || document.documentElement || document).appendChild(style);
  const target = document.createElement('div');
  target.id = 'target';
  (document.body || document.documentElement || document).appendChild(target);
  const file = document.createElement('input');
  file.id = 'file';
  file.type = 'file';
  (document.body || document.documentElement || document).appendChild(file);
  const select = document.createElement('select');
  select.id = 'select';
  select.style.width = '321px';
  (document.body || document.documentElement || document).appendChild(select);
  const pickerDiv = document.createElement('div');
  pickerDiv.id = 'picker-div';
  (document.body || document.documentElement || document).appendChild(pickerDiv);
  const pickerDivNoRules = document.createElement('div');
  pickerDivNoRules.id = 'picker-div-no-rules';
  (document.body || document.documentElement || document).appendChild(pickerDivNoRules);
  const item = document.createElement('li');
  item.id = 'item';
  (document.body || document.documentElement || document).appendChild(item);
  const input = document.createElement('input');
  input.id = 'input';
  input.placeholder = 'placeholder';
  (document.body || document.documentElement || document).appendChild(input);
  const plain = document.createElement('div');
  plain.id = 'plain';
  (document.body || document.documentElement || document).appendChild(plain);
  const heldBefore = getComputedStyle(target, '::before');
  const heldAfter = getComputedStyle(target, '::after');
  const heldOrigin = getComputedStyle(target);
  return [
    getComputedStyle(target, 'before').width,
    getComputedStyle(target, ':before').width,
    getComputedStyle(target, '::before').width,
    getComputedStyle(target, '::after').width,
    getComputedStyle(target, '::before(test)').length,
    getComputedStyle(target, ':checkmark').width,
    getComputedStyle(target, '::checkmark').width,
    getComputedStyle(target, 'file-selector-button').color,
    getComputedStyle(file, '::file-selector-button').color,
    getComputedStyle(select, 'picker-icon').width,
    getComputedStyle(select, ':picker-icon').width,
    getComputedStyle(select, '::picker-icon').width,
    getComputedStyle(select, '::picker(select)').backgroundColor,
    getComputedStyle(pickerDiv, '::picker(select)').backgroundColor,
    getComputedStyle(pickerDivNoRules, '::picker(select)').backgroundColor,
    getComputedStyle(select, '::picker(div)').length,
    getComputedStyle(item, '::marker').color,
    getComputedStyle(input, '::placeholder').color,
    getComputedStyle(target, '::highlight(n\\61me)').color,
    getComputedStyle(target, '::highlight(other)').color,
    getComputedStyle(plain).getPropertyValue('font-style'),
    getComputedStyle(plain, '::before').getPropertyValue('font-style'),
    heldBefore.width,
    heldAfter.width,
    heldOrigin.width,
    heldBefore.width
  ].join('|');
})()
"#,
        )
        .expect("computed pseudo element style should evaluate");

    assert_eq!(
        result,
        "100px|50px|50px|25px|0||17px|rgb(255, 0, 0)|rgb(0, 255, 0)|321px||13px|rgb(0, 128, 0)|rgb(0, 0, 255)|rgba(0, 0, 0, 0)|0|rgb(10, 20, 30)|rgb(30, 20, 10)|rgb(0, 0, 255)|rgb(255, 0, 255)|italic|italic|50px|25px|100px|50px"
    );
}

#[test]
fn computed_style_resolves_highlight_font_relative_text_decoration_properties() {
    let mut vm = new_storage_test_vm("https://highlight-font-relative-computed.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  const style = document.createElement('style');
  style.textContent = `
    :root { font-size: 16px; }
    div { font-size: 20px; }
    ::highlight(highlight1) {
      text-underline-offset: 0.5em;
      text-decoration-line: underline;
      text-decoration-color: green;
      text-decoration-thickness: 0.25rem;
    }
    body > div[data-kind="one"]::highlight(highlight1) {
      text-underline-offset: 0.75em;
      text-decoration-thickness: 0.5rem;
    }
    #h2::highlight(highlight1) {
      text-underline-offset: 1.0em;
      text-decoration-line: underline;
      text-decoration-color: blue;
      text-decoration-thickness: 0.125rem;
    }`;
  (document.head || document.documentElement || document).appendChild(style);
  const h1 = document.createElement('div');
  h1.id = 'h1';
  h1.dataset.kind = 'one';
  h1.textContent = 'one';
  const h2 = document.createElement('div');
  h2.id = 'h2';
  h2.dataset.kind = 'two';
  h2.textContent = 'two';
  const body = document.body || document.createElement('body');
  if (!body.parentNode) {
    document.documentElement.appendChild(body);
  }
  body.appendChild(h1);
  body.appendChild(h2);
  const r1 = document.createRange();
  r1.setStart(h1, 0);
  r1.setEnd(h1, 1);
  const r2 = document.createRange();
  r2.setStart(h2, 0);
  r2.setEnd(h2, 1);
  CSS.highlights.set('highlight1', new Highlight(r1, r2));
  const pseudo = '::highlight(highlight1)';
  const rootStyle = getComputedStyle(document.documentElement, pseudo);
  const h1Style = getComputedStyle(h1, pseudo);
  const h2Style = getComputedStyle(h2, pseudo);
  return [
    rootStyle.textUnderlineOffset,
    rootStyle.textDecorationThickness,
    h1Style.textUnderlineOffset,
    h1Style.textDecorationThickness,
    h2Style.textUnderlineOffset,
    h2Style.textDecorationThickness
  ].join('|');
})()
"#,
        )
        .expect("Highlight font-relative computed properties should evaluate");

    assert_eq!(result, "8px|4px|15px|8px|20px|2px");
}

#[test]
fn file_selector_button_pseudo_only_border_style_is_named_property() {
    let mut vm = new_storage_test_vm("https://file-selector-button-pseudo-style.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = `::file-selector-button { border: 3px double black; }`;
  (document.head || document.documentElement || document).appendChild(style);
  const file = document.createElement('input');
  file.type = 'file';
  (document.body || document.documentElement || document).appendChild(file);

  const host = document.createElement('div');
  (document.body || document.documentElement || document).appendChild(host);
  const shadow = host.attachShadow({ mode: 'open' });
  shadow.innerHTML = `
    <style>::file-selector-button { border: 5px dotted black; }</style>
    <input type="file" id="shadow-file">
  `;

  const documentStyle = getComputedStyle(file, '::file-selector-button');
  const shadowStyle = getComputedStyle(
    shadow.getElementById('shadow-file'),
    '::file-selector-button'
  );
  return JSON.stringify({
    documentNamed: documentStyle.borderTopStyle,
    documentMethod: documentStyle.getPropertyValue('border-top-style'),
    shadowNamed: shadowStyle.borderTopStyle,
    shadowMethod: shadowStyle.getPropertyValue('border-top-style')
  });
})()
"#,
        )
        .expect("file selector button pseudo-only border style should evaluate");

    assert_eq!(
        result,
        r#"{"documentNamed":"double","documentMethod":"double","shadowNamed":"dotted","shadowMethod":"dotted"}"#
    );
}
#[test]
fn lazy_pseudo_computed_style_reuses_and_clears_cache() {
    let mut vm = new_storage_test_vm("https://lazy-pseudo-computed-cache.test/");
    let document = vm.document_handle_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '#file::file-selector-button { color: rgb(0, 255, 0); }';
  head.appendChild(style);

  globalThis.__lazyPseudoFile = document.createElement('input');
  globalThis.__lazyPseudoFile.id = 'file';
  globalThis.__lazyPseudoFile.type = 'file';
  body.appendChild(globalThis.__lazyPseudoFile);

  const buttonStyle = getComputedStyle(globalThis.__lazyPseudoFile, '::file-selector-button');
  return [buttonStyle.color, buttonStyle.backgroundColor].join('|');
})()
"#,
        )
        .expect("lazy pseudo cache setup should evaluate");

    assert_eq!(result, "rgb(0, 255, 0)|rgba(0, 0, 0, 0)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );

    let removed = vm
        .eval(
            r#"
(() => {
  globalThis.__lazyPseudoFile.remove();
  delete globalThis.__lazyPseudoFile;
  return 'removed';
})()
"#,
        )
        .expect("lazy pseudo cached node removal should evaluate");

    assert_eq!(removed, "removed");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
}
#[test]
fn computed_style_uses_stylesheet_source_base_urls() {
    let mut vm = new_storage_test_vm("https://stylesheet-base.test/page/index.html");
    let stylesheet_url = url::Url::parse("https://stylesheet-base.test/assets/app.css").unwrap();
    let stylesheet_final_url =
        url::Url::parse("https://stylesheet-base.test/final/app.css").unwrap();
    vm.eval(
        r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || document.documentElement || document;
  const base = document.createElement('base');
  base.href = 'https://stylesheet-base.test/base/';
  head.appendChild(base);

  const link = document.createElement('link');
  link.id = 'source-base-link';
  link.rel = 'stylesheet';
  link.href = '../assets/app.css';
  head.appendChild(link);
  if (!document.body) {
    html.appendChild(document.createElement('body'));
  }

})()
"#,
    )
    .expect("linked stylesheet base URL setup should evaluate");
    let link = element_handle_by_id(&vm, "source-base-link");
    install_linked_stylesheet_for_test(
        &mut vm,
        link,
        stylesheet_url.clone(),
        crate::style_engine::StyloStylesheetSource::new(
            "body { background-image: url(img/linked.png); }".to_owned(),
            stylesheet_final_url,
        )
        .with_sheet_url(stylesheet_url),
    );

    let result = vm
        .eval(
            r#"
(() => {
  const linked = getComputedStyle(document.body).backgroundImage;
  const sheet = new CSSStyleSheet({ baseURL: 'https://constructed-base.test/styles/sheet.css' });
  sheet.replaceSync('body { background-image: url(img/adopted.png); }');
  document.adoptedStyleSheets = [sheet];
  const adopted = getComputedStyle(document.body).backgroundImage;

  return `${linked}|${adopted}`;
})()
"#,
        )
        .expect("computed stylesheet base URL probe should evaluate");

    assert_eq!(
        result,
        r#"url("https://stylesheet-base.test/final/img/linked.png")|url("https://constructed-base.test/styles/img/adopted.png")"#
    );
}

#[test]
fn inline_style_sheet_keeps_processing_base_across_unrelated_source_set_rebuild() {
    let mut vm = new_storage_test_vm("https://inline-sheet-base.test/page/index.html");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const base = document.createElement('base');
  base.href = 'https://inline-sheet-base.test/old/';
  head.appendChild(base);

  const style = document.createElement('style');
  style.textContent = '.subject { background-image: url(img.png); }';
  head.appendChild(style);
  const subject = document.createElement('div');
  subject.className = 'subject';
  body.appendChild(subject);

  const before = getComputedStyle(subject).backgroundImage;
  base.href = 'https://inline-sheet-base.test/new/';

  const unrelated = document.createElement('style');
  unrelated.textContent = '.unrelated { color: red; }';
  head.appendChild(unrelated);
  const after = getComputedStyle(subject).backgroundImage;
  return `${before}|${after}`;
})()
"#,
        )
        .expect("inline stylesheet frozen parser-base probe should evaluate");

    assert_eq!(
        result,
        r#"url("https://inline-sheet-base.test/old/img.png")|url("https://inline-sheet-base.test/old/img.png")"#
    );
}

#[test]
fn inline_style_sheet_content_reprocessing_captures_the_current_base() {
    let mut vm = new_storage_test_vm("https://inline-sheet-reprocess.test/page/index.html");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const base = document.createElement('base');
  base.href = 'https://inline-sheet-reprocess.test/old/';
  head.appendChild(base);

  const style = document.createElement('style');
  style.textContent = '.subject { background-image: url(before.png); }';
  head.appendChild(style);
  const subject = document.createElement('div');
  subject.className = 'subject';
  body.appendChild(subject);

  const before = getComputedStyle(subject).backgroundImage;
  base.href = 'https://inline-sheet-reprocess.test/new/';
  style.textContent = '.subject { background-image: url(after.png); }';
  const after = getComputedStyle(subject).backgroundImage;
  return `${before}|${after}`;
})()
"#,
        )
        .expect("inline stylesheet content reprocessing probe should evaluate");

    assert_eq!(
        result,
        r#"url("https://inline-sheet-reprocess.test/old/before.png")|url("https://inline-sheet-reprocess.test/new/after.png")"#
    );
}

#[test]
fn reconnected_inline_style_sheet_is_processed_with_the_current_base() {
    let mut vm = new_storage_test_vm("https://inline-sheet-reconnect.test/page/index.html");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const base = document.createElement('base');
  base.href = 'https://inline-sheet-reconnect.test/old/';
  head.appendChild(base);

  const style = document.createElement('style');
  style.textContent = '.subject { background-image: url(image.png); }';
  head.appendChild(style);
  const subject = document.createElement('div');
  subject.className = 'subject';
  body.appendChild(subject);

  const before = getComputedStyle(subject).backgroundImage;
  style.remove();
  base.href = 'https://inline-sheet-reconnect.test/new/';
  head.appendChild(style);
  const after = getComputedStyle(subject).backgroundImage;
  return `${before}|${after}`;
})()
"#,
        )
        .expect("inline stylesheet reconnect probe should evaluate");

    assert_eq!(
        result,
        r#"url("https://inline-sheet-reconnect.test/old/image.png")|url("https://inline-sheet-reconnect.test/new/image.png")"#
    );
}

#[test]
fn inline_cssom_rule_edit_keeps_the_sheet_parser_context() {
    let mut vm = new_storage_test_vm("https://inline-sheet-cssom.test/page/index.html");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const base = document.createElement('base');
  base.href = 'https://inline-sheet-cssom.test/old/';
  head.appendChild(base);

  const style = document.createElement('style');
  head.appendChild(style);
  const subject = document.createElement('div');
  subject.className = 'subject';
  body.appendChild(subject);

  base.href = 'https://inline-sheet-cssom.test/new/';
  style.sheet.insertRule('.subject { background-image: url(inserted.png); }', 0);
  return getComputedStyle(subject).backgroundImage;
})()
"#,
        )
        .expect("inline stylesheet CSSOM parser-context probe should evaluate");

    assert_eq!(
        result,
        r#"url("https://inline-sheet-cssom.test/old/inserted.png")"#
    );
}

#[test]
fn inline_cssom_rule_inserted_before_target_connection_applies_later() {
    let mut vm = new_storage_test_vm("https://inline-sheet-before-target.test/");

    vm.eval(
        r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.appendChild(document.createTextNode(''));
  head.appendChild(style);
  style.sheet.insertRule('#target { margin-left: 2px; }');
})()
"#,
    )
    .expect("inline CSSOM rule setup should evaluate");

    vm.sync_live_document_style_sources();

    let result = vm
        .eval(
            r#"
(() => {
  const body = document.body;
  const target = document.createElement('div');
  target.id = 'target';
  body.appendChild(target);
  const beforeDomMutation = getComputedStyle(target).marginLeft;
  document.querySelector('style').textContent = '#target { margin-left: 3px; }';
  const afterDomMutation = getComputedStyle(target).marginLeft;
  return `${beforeDomMutation}|${afterDomMutation}`;
})()
"#,
        )
        .expect("inline CSSOM rule should apply to targets connected later");

    assert_eq!(result, "2px|3px");
}

#[test]
fn constructed_css_stylesheet_insert_rule_uses_constructor_document_base_url() {
    let mut vm = new_storage_test_vm("https://constructable-base.test/css/cssom/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const sheet = new CSSStyleSheet();
  sheet.insertRule(':root { background-image: url("../../images/green.png"); }');
  const base = document.createElement('base');
  base.href = 'https://constructable-base.test/changed/';
  head.appendChild(base);
  document.adoptedStyleSheets = [sheet];
  return getComputedStyle(html).backgroundImage;
})()
"#,
        )
        .expect("constructed insertRule base URL probe should evaluate");

    assert_eq!(
        result,
        r#"url("https://constructable-base.test/images/green.png")"#
    );
}
#[test]
fn computed_style_exposes_reading_flow_and_order() {
    let mut vm = new_storage_test_vm("https://reading-flow-computed.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = `
    #container { reading-flow: grid-order; }
    #child { reading-order: -2; }`;
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  (document.head || html).appendChild(style);
  const container = document.createElement('div');
  container.id = 'container';
  const child = document.createElement('button');
  child.id = 'child';
  container.appendChild(child);
  (document.body || html.appendChild(document.createElement('body'))).appendChild(container);
  const containerStyle = getComputedStyle(container);
  const childStyle = getComputedStyle(child);
  const unset = getComputedStyle(document.body);
  return [
    containerStyle.getPropertyValue('reading-flow'),
    childStyle.getPropertyValue('reading-order'),
    unset.getPropertyValue('reading-flow'),
    unset.getPropertyValue('reading-order')
  ].join('|');
})()
"#,
        )
        .expect("computed reading-flow probe should evaluate");

    assert_eq!(result, "grid-order|-2|normal|0");
}
#[test]
fn computed_width_resolves_percent_against_parent_and_child_frame_viewport() {
    let mut vm = new_storage_test_vm("https://computed-width.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = `
    #outside { width: 200px; }
    #inside { width: 50%; }`;
  (document.documentElement || document.body || document).appendChild(style);
  const outside = document.createElement('div');
  outside.id = 'outside';
  const inside = document.createElement('div');
  inside.id = 'inside';
  outside.appendChild(inside);
  const appendTarget = document.body || document.documentElement || document;
  appendTarget.appendChild(outside);

  const frame = document.createElement('iframe');
  frame.setAttribute('width', '100');
  appendTarget.appendChild(frame);
  const childDocument = frame.contentWindow.document;
  childDocument.open();
  childDocument.write('<body style="margin:0"><div style="width:100%"></div>');
  childDocument.close();

  return [
    getComputedStyle(inside).width,
    frame.contentWindow.getComputedStyle(childDocument.querySelector('div')).width
  ].join('|');
})()
"#,
        )
        .expect("computed width should resolve percent values");

    assert_eq!(result, "100px|100px");
}

#[test]
fn transformed_oversized_inline_iframe_uses_its_containing_block_percentage_basis() {
    let mut vm = new_storage_test_vm("https://inline-iframe-percentage-size.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const container = document.createElement('div');
  container.style.cssText = 'width:auto;max-width:200px;height:100px;overflow:hidden';
  const frame = document.createElement('iframe');
  frame.style.cssText = 'width:calc(100% / 0.8);height:50px;border:0;transform:scale(0.8);transform-origin:0 0';
  container.appendChild(frame);
  body.appendChild(container);
  const rect = frame.getBoundingClientRect();
  return [
    frame.offsetWidth,
    frame.clientWidth,
    rect.width
  ].join('|');
})()
"#,
        )
        .expect("transformed inline iframe geometry should evaluate");

    assert_eq!(result, "250|250|200");
}

#[test]
fn computed_horizontal_margin_reads_preserve_retained_style_viewport_context() {
    let mut vm = new_storage_test_vm("https://computed-margin-retained-context.test/");
    vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
        inner_width: 800,
        inner_height: 600,
        outer_width: 800,
        outer_height: 600,
        device_pixel_ratio: 1.0,
        screen_width: 1920,
        screen_height: 1080,
        screen_avail_width: 1920,
        screen_avail_height: 1040,
    }))
    .expect("viewport surface should update");
    let document = vm.document_handle_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = `
    #margin-parent { width: 400px; }
    #margin-child {
      display: block;
      width: 200px;
      margin-left: auto;
      margin-right: auto;
    }`;
  head.appendChild(style);

  const parent = document.createElement('div');
  parent.id = 'margin-parent';
  const child = document.createElement('div');
  child.id = 'margin-child';
  parent.appendChild(child);
  body.appendChild(parent);

  const computed = getComputedStyle(child);
  let values = '';
  for (let i = 0; i < 16; i += 1) {
    values = [
      computed.marginLeft,
      computed.width,
      computed.container,
      computed.containerType
    ].join('|');
  }
  return values;
})()
"#,
        )
        .expect("horizontal margin reads should preserve the full viewport context");

    assert_eq!(result, "100px|200px|none|normal");
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(document),
        1,
        "nested width resolution must not replace the 800x600 viewport with a width-only key",
    );
}

#[test]
fn child_computed_horizontal_margin_reads_preserve_retained_style_viewport_context() {
    let mut vm = new_storage_test_vm("https://child-computed-margin-retained-context.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const body = document.body || document.documentElement || document;
  const frame = document.createElement('iframe');
  frame.id = 'margin-context-frame';
  frame.style.width = '400px';
  frame.style.height = '250px';
  body.appendChild(frame);

  const childWindow = frame.contentWindow;
  const childDocument = childWindow.document;
  childDocument.open();
  childDocument.write(`
    <style>
      #margin-parent { width: 300px; }
      #margin-child {
        display: block;
        width: 100px;
        margin-left: auto;
        margin-right: auto;
      }
    </style>
    <body>
      <div id="margin-parent">
        <div id="margin-child"></div>
      </div>
    </body>`);
  childDocument.close();

  const computed = childWindow.getComputedStyle(
    childDocument.getElementById('margin-child')
  );
  let values = '';
  for (let i = 0; i < 16; i += 1) {
    values = [
      computed.marginLeft,
      computed.width,
      computed.container,
      computed.containerType
    ].join('|');
  }
  return values;
})()
"#,
        )
        .expect("child horizontal margin reads should preserve the iframe viewport context");

    assert_eq!(result, "100px|100px|none|normal");
    let child_document = child_document_handle_for_frame_id(&vm, "margin-context-frame");
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(child_document),
        1,
        "nested child width resolution must preserve the iframe viewport height and screen",
    );
}

#[test]
fn computed_used_size_clamps_infinite_negative_math_to_zero() {
    let mut vm = new_storage_test_vm("https://computed-used-size-infinity.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const target = document.createElement('div');
  target.style.display = 'block';
  body.appendChild(target);

  const first = 'calc(infinity * 1px - infinity * 1%)';
  const second = 'calc(infinity * 1px - max(infinity * 1%, 0%))';
  const fromAttribute = document.createElement('div');
  fromAttribute.setAttribute('style', `display:block;width:${first};height:${first}`);
  body.appendChild(fromAttribute);

  target.style.setProperty('width', first);
  const firstSpecified = target.style.getPropertyValue('width');
  const firstComputed = getComputedStyle(target).width;

  target.style.setProperty('width', second);
  const secondSpecified = target.style.getPropertyValue('width');
  const secondComputed = getComputedStyle(target).width;

  return [
    CSS.supports('width', first),
    firstSpecified !== '',
    firstComputed,
    CSS.supports('width', second),
    secondSpecified !== '',
    secondComputed,
    getComputedStyle(fromAttribute).width,
    getComputedStyle(fromAttribute).height
  ].join('|');
})()
"#,
        )
        .expect("computed used sizes should clamp negative infinite math");

    assert_eq!(result, "true|true|0px|true|true|0px|0px|0px");
}

#[test]
fn computed_style_child_document_media_queries_use_iframe_viewport() {
    let mut vm = new_storage_test_vm("https://child-document-media-query.test/");

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
  const frame = document.createElement('iframe');
  frame.style.width = '100px';
  frame.style.height = '100px';
  (document.body || document.documentElement || document).appendChild(frame);
  const childDocument = frame.contentWindow.document;
  childDocument.open();
  childDocument.write('<style>body { color: red } @media all and (min-width: 101px) { body { color: green } }</style><body>text</body>');
  childDocument.close();
  const before = getComputedStyle(childDocument.body).color;
  frame.style.width = '200px';
  const after = getComputedStyle(childDocument.body).color;
  return `${before}|${after}`;
})()
"#,
        )
        .expect("child document media queries should use iframe viewport");

    assert_eq!(result, "rgb(255, 0, 0)|rgb(0, 128, 0)");
}

#[test]
fn computed_style_child_document_media_queries_use_iframe_viewport_height_for_calc() {
    let mut vm = new_storage_test_vm("https://child-document-media-query-calc.test/");

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
  const frame = document.createElement('iframe');
  frame.setAttribute('width', '100');
  frame.setAttribute('height', '10');
  (document.body || document.documentElement || document).appendChild(frame);
  const childDocument = frame.contentWindow.document;
  childDocument.open();
  childDocument.write(`
    <style>
      body { background-color: rgb(0, 0, 255); }
      @media (width: calc(200vh + 5em)) {
        body { background-color: rgb(255, 165, 0); }
      }
    </style>
    <body>text</body>`);
  childDocument.close();
  return frame.contentWindow.getComputedStyle(childDocument.body).backgroundColor;
})()
"#,
        )
        .expect("child document calc media query should use iframe viewport height");

    assert_eq!(result, "rgb(255, 165, 0)");
}

#[test]
fn computed_style_child_document_viewport_units_use_css_iframe_size() {
    let mut vm = new_storage_test_vm("https://child-document-viewport-units.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = 'iframe.viewport-probe { width: 200px; height: 100px; }';
  head.appendChild(style);

  const frame = document.createElement('iframe');
  frame.className = 'viewport-probe';
  body.appendChild(frame);

  const childDocument = frame.contentWindow.document;
  childDocument.open();
  childDocument.write(`
    <style>
      html, body { margin: 0; width: 100%; height: 100%; }
      #vh { height: 100vh; }
      #vmin { width: 100vmin; height: 1px; }
      #vmax { width: 100vmax; height: 1px; }
    </style>
    <body>
      <div id="vh"></div>
      <div id="vmin"></div>
      <div id="vmax"></div>
    </body>`);
  childDocument.close();

  const childWindow = frame.contentWindow;
  return [
    childWindow.getComputedStyle(childDocument.getElementById('vh')).height,
    childWindow.getComputedStyle(childDocument.getElementById('vmin')).width,
    childWindow.getComputedStyle(childDocument.getElementById('vmax')).width
  ].join('|');
})()
"#,
        )
        .expect("child document viewport units should use css iframe size");

    assert_eq!(result, "100px|100px|200px");
}

#[test]
fn child_window_and_mock_root_geometry_use_iframe_viewport() {
    let mut vm = new_storage_test_vm("https://child-window-viewport.test/");

    let before = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  frame.id = 'viewport-frame';
  frame.style.width = '300px';
  frame.style.height = '65px';
  (document.body || document.documentElement || document).appendChild(frame);
  const childWindow = frame.contentWindow;
  const childDocument = childWindow.document;
  childDocument.open();
  childDocument.write('<style>html,body{margin:0;width:100%;height:100%}</style><body></body>');
  childDocument.close();

  const snapshot = () => {
    const body = childDocument.body.getBoundingClientRect();
    const root = childDocument.documentElement.getBoundingClientRect();
    return [
      childWindow.innerWidth,
      childWindow.innerHeight,
      childWindow.matchMedia('(width: 300px)').matches,
      body.width,
      body.height,
      root.width,
      root.height
    ].join('|');
  };
  return snapshot();
})()
"#,
        )
        .expect("initial child Window viewport and root geometry should evaluate");
    assert_eq!(before, "300|65|true|300|65|300|65");

    vm.eval(
        r#"
(() => {
  const frame = document.getElementById('viewport-frame');
  frame.style.width = '320px';
  frame.style.height = '80px';
  return 'resized';
})()
"#,
    )
    .expect("child frame resize should evaluate");
    refresh_layout_for_test(&mut vm);
    let after = vm
        .eval(
            r#"
(() => {
  const frame = document.getElementById('viewport-frame');
  const childWindow = frame.contentWindow;
  const childDocument = childWindow.document;
  return [
    childWindow.innerWidth,
    childWindow.innerHeight,
    childWindow.matchMedia('(width: 320px)').matches,
    childDocument.body.getBoundingClientRect().width,
    childDocument.body.getBoundingClientRect().height
  ].join('|');
})()
"#,
        )
        .expect("resized child Window viewport and root geometry should evaluate");

    assert_eq!(after, "320|80|true|320|80");
}

#[test]
fn direct_child_realm_window_surface_uses_iframe_viewport() {
    let mut vm = new_storage_test_vm("https://direct-child-window-viewport.test/");
    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.style.width = '300px';
  frame.style.height = '65px';
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("direct child Window viewport setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "direct child Window viewport realm",
    );

    let result = vm
        .eval_in_child_default_context(
            child_context_id,
            r#"
(() => [
  innerWidth,
  innerHeight,
  matchMedia('(width: 300px)').matches,
  matchMedia('(prefers-reduced-motion: no-preference)').matches
].join('|'))()
"#,
        )
        .expect("direct child Window viewport should evaluate in its own realm");

    assert_eq!(result, "300|65|true|true");

    let top_result = vm
        .eval("[innerWidth, innerHeight].join('|')")
        .expect("top Window viewport should remain top-level after direct child evaluation");
    assert_eq!(top_result, "1920|1080");
}

#[test]
fn child_parser_quirks_mode_does_not_mutate_the_top_document() {
    let mut vm = new_storage_test_vm("https://child-quirks-owner.test/");
    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.srcdoc = '<body>quirks child</body>';
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child quirks-mode owner setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    let result = vm
        .eval(
            "[document.compatMode, document.querySelector('iframe').contentDocument.compatMode].join('|')",
        )
        .expect("top and child document compatMode should evaluate");

    assert_eq!(result, "CSS1Compat|BackCompat");
}

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
        r#"generic(fangsong)|-webkit-generic(fangsong)|Times New Roman|"34J""#
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

    let result = vm
        .eval(
            r#"
(() => {
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
  return [
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
  ].join('|');
})()
"#,
        )
        .expect("box metrics should consume simple computed style facts");

    assert_eq!(
        result,
        "none|0|0|0|80px|80|40px|40|80|40|1|50px|50|30px|30|50"
    );
}

#[test]
fn box_metric_uses_one_computed_style_input_snapshot_for_nested_resolution() {
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
    let builds_before = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();

    let result = vm
        .eval("String(document.getElementById('target').offsetWidth)")
        .expect("box metric should resolve a nested percentage width");

    assert_eq!(result, "100");
    let input_builds = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test()
        .saturating_sub(builds_before);
    assert_eq!(
        input_builds, 1,
        "one synchronous box metric must carry one immutable style input snapshot"
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
    let builds_before = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();

    let result = vm
        .eval("document.getElementById('target').style.fontSize")
        .expect("missing inline font-size should remain readable");

    assert_eq!(result, "");
    let input_builds = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test()
        .saturating_sub(builds_before);
    assert_eq!(
        input_builds, 0,
        "a non-logical inline property must not resolve writing mode or direction"
    );
}

#[test]
fn box_metrics_round_fractional_computed_px_values() {
    let mut vm = new_storage_test_vm("https://box-metrics-fractional-style.test/");

    let result = vm
        .eval(
            r#"
(() => {
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
  return [
    fractional.offsetWidth,
    fractional.clientWidth,
    fractional.scrollWidth,
    fractional.offsetHeight,
    fractional.clientHeight,
    fractional.scrollHeight,
    fractional.getBoundingClientRect().width,
    fractional.getBoundingClientRect().height
  ].join('|');
})()
"#,
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

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  body.innerHTML = '<div id="before" style="display:none">before</div>';
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

    let result = vm
        .eval_in_isolated_context(
            context_id,
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  body.innerHTML = '<div id="before" style="display:none">before</div>';
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

    let result = vm
        .eval(
            r#"
(() => {
  const wrapper = document.createElement('div');
  wrapper.style.cssText = 'height: 20px; width: 40px; padding: 1px 2px';
  const target = document.createElement('div');
  target.style.cssText = 'position: relative; top: 10%; left: calc(25% - 2px)';
  wrapper.appendChild(target);
  (document.body || document.documentElement || document).appendChild(wrapper);
  const computed = getComputedStyle(target);
  return `${computed.top}|${computed.left}`;
})()
"#,
        )
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

    let result = vm
        .eval(
            r#"
(() => {
  const wrapper = document.createElement('div');
  wrapper.style.cssText = 'position: relative; height: 20px; width: 40px';
  const target = document.createElement('div');
  target.style.cssText = 'position: absolute; top: auto; right: 4px; bottom: 3px; left: auto';
  wrapper.appendChild(target);
  (document.body || document.documentElement || document).appendChild(wrapper);
  const computed = getComputedStyle(target);
  return `${computed.top}|${computed.right}|${computed.bottom}|${computed.left}`;
})()
"#,
        )
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
    let mut vm = new_storage_test_vm("https://computed-grid-column-auto.test/");

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
fn computed_style_property_names_are_sorted() {
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
    const segment = name => name.startsWith('--') ? 1 : name.startsWith('-') ? 2 : 0;
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

#[test]
fn computed_style_index_enumeration_does_not_rebuild_inputs_per_standard_property() {
    let mut vm = new_storage_test_vm("https://computed-style-index-cost.test/");
    vm.eval(
        r#"
const target = document.createElement('div');
target.id = 'target';
(document.body || document.documentElement || document).appendChild(target);
globalThis.__computedStyleForIndexCost = getComputedStyle(target);
"#,
    )
    .expect("computed-style fixture should initialize");
    let builds_before = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();

    let length = vm
        .eval("String(__computedStyleForIndexCost.length)")
        .expect("computed style length should be readable");
    let after_length = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();
    vm.eval("String(__computedStyleForIndexCost[0])")
        .expect("first indexed computed property should be readable");
    let after_index = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();
    vm.eval("String(__computedStyleForIndexCost.item(0))")
        .expect("first computed style item should be readable");
    let after_item = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();
    vm.eval("String(0 in __computedStyleForIndexCost)")
        .expect("first computed style index should be queryable");
    let after_query = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();
    let count = vm
        .eval("String(Array.from(__computedStyleForIndexCost).length)")
        .expect("computed properties should be enumerable")
        .parse::<u64>()
        .expect("computed property count");

    let input_builds = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test()
        .saturating_sub(builds_before);
    assert!(count >= 266, "unexpectedly narrow computed style: {count}");
    assert_eq!(after_length.saturating_sub(builds_before), 1);
    assert_eq!(after_index.saturating_sub(after_length), 0);
    assert_eq!(after_item.saturating_sub(after_index), 0);
    assert_eq!(after_query.saturating_sub(after_item), 0);
    assert!(
        input_builds <= 8,
        "indexed enumeration rebuilt style inputs {input_builds} times for {count} properties; \
         length={length}, deltas length/index/item/query={}/{}/{}/{}",
        after_length.saturating_sub(builds_before),
        after_index.saturating_sub(after_length),
        after_item.saturating_sub(after_index),
        after_query.saturating_sub(after_item),
    );
}

#[test]
fn held_computed_style_count_cache_tracks_custom_property_mutations() {
    let mut vm = new_storage_test_vm("https://computed-style-custom-count-cache.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  target.style.setProperty('--before', 'one');
  (document.body || document.documentElement || document).appendChild(target);
  const style = getComputedStyle(target);
  const before = Array.from(style);
  target.style.removeProperty('--before');
  target.style.setProperty('--after', 'two');
  const after = Array.from(style);
  return JSON.stringify({
    beforeLength: before.length,
    afterLength: after.length,
    beforeHasBefore: before.includes('--before'),
    beforeHasAfter: before.includes('--after'),
    afterHasBefore: after.includes('--before'),
    afterHasAfter: after.includes('--after'),
    afterValue: style.getPropertyValue('--after'),
  });
})()
"#,
        )
        .expect("held computed style should track custom-property mutations");

    let result: serde_json::Value = serde_json::from_str(&result).expect("valid JSON summary");
    assert_eq!(result["beforeLength"], result["afterLength"]);
    assert_eq!(result["beforeHasBefore"], serde_json::json!(true));
    assert_eq!(result["beforeHasAfter"], serde_json::json!(false));
    assert_eq!(result["afterHasBefore"], serde_json::json!(false));
    assert_eq!(result["afterHasAfter"], serde_json::json!(true));
    assert_eq!(result["afterValue"], serde_json::json!("two"));
}

#[test]
fn inspector_computed_style_bulk_read_uses_one_input_snapshot() {
    let mut vm = new_storage_test_vm("https://computed-style-inspector-bulk.test/");
    vm.eval(
        r#"
const style = document.createElement('style');
style.textContent = `#target {
  animation-timeline: auto;
  animation-range-start: entry 10%;
  animation-range-end: exit 20%;
  background-position-x: 25%;
  column-span: all;
  column-width: 12px;
  font-variant-alternates: historical-forms;
  font-variant-emoji: emoji;
  font-variant-position: super;
  grid-auto-columns: 17px;
  object-fit: cover;
  overflow-wrap: anywhere;
  pointer-events: none;
  white-space-collapse: preserve;
  zoom: 125%;
}`;
(document.head || document.documentElement || document).appendChild(style);
const target = document.createElement('div');
target.id = 'target';
target.style.setProperty('--inspector-token', 'present');
(document.body || document.documentElement || document).appendChild(target);
const unrelatedHost = document.createElement('div');
(document.body || document.documentElement || document).appendChild(unrelatedHost);
unrelatedHost.attachShadow({ mode: 'open' }).innerHTML =
  '<style>:host { color: rgb(1, 2, 3); }</style><span>unrelated</span>';
"#,
    )
    .expect("inspector computed-style fixture should initialize");
    let target = element_handle_by_id(&vm, "target");
    let builds_before = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();
    let property_reads_before = vm
        ._context_host
        .borrow()
        .stylo_computed_style_property_reads_for_test();

    let properties = vm
        .computed_style_properties_for_inspector_handle(target)
        .expect("live element should resolve");

    let input_builds = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test()
        .saturating_sub(builds_before);
    let property_reads = vm
        ._context_host
        .borrow()
        .stylo_computed_style_property_reads_for_test()
        .saturating_sub(property_reads_before);
    assert_eq!(
        input_builds, 1,
        "one inspector bulk read must carry one immutable style input snapshot"
    );
    assert!(properties.len() >= 267);
    assert!(
        property_reads.saturating_mul(4) < properties.len() as u64,
        "bulk serialization re-entered Stylo {property_reads} times for {} projected properties",
        properties.len(),
    );
    let properties = properties
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>();
    for (name, expected) in [
        ("animation-timeline", "auto"),
        ("animation-range-start", "entry 10%"),
        ("animation-range-end", "exit 20%"),
        ("background-position-x", "25%"),
        ("column-span", "all"),
        ("column-width", "12px"),
        ("font-variant-alternates", "historical-forms"),
        ("font-variant-emoji", "emoji"),
        ("font-variant-position", "super"),
        ("grid-auto-columns", "17px"),
        ("object-fit", "cover"),
        ("overflow-wrap", "anywhere"),
        ("pointer-events", "none"),
        ("white-space-collapse", "preserve"),
        ("zoom", "1.25"),
        ("--inspector-token", "present"),
    ] {
        assert_eq!(properties.get(name).map(String::as_str), Some(expected));
    }
}

#[test]
fn inspector_computed_style_bulk_read_uses_child_document_scope_and_viewport() {
    let mut vm = new_storage_test_vm("https://computed-style-inspector-child.test/");
    vm.eval(
        r#"
const frame = document.createElement('iframe');
frame.id = 'inspector-child-frame';
frame.style.width = '200px';
frame.style.height = '100px';
(document.body || document.documentElement || document).appendChild(frame);
const childDocument = frame.contentWindow.document;
childDocument.open();
childDocument.write(`
  <style>
    #inspector-child-target {
      color: rgb(255, 0, 0);
      pointer-events: auto;
      width: 10px;
    }
    @media (width: 200px) {
      #inspector-child-target {
        color: rgb(0, 128, 0);
        pointer-events: none;
        width: 50vw;
        --child-inspector-token: child;
      }
    }
  </style>
  <body><div id="inspector-child-target"></div></body>`);
childDocument.close();
"#,
    )
    .expect("child inspector computed-style fixture should initialize");
    let target = element_handle_by_id(&vm, "inspector-child-target");

    let properties = vm
        .computed_style_properties_for_inspector_handle(target)
        .expect("child-frame element should resolve")
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>();
    for (name, expected) in [
        ("color", "rgb(0, 128, 0)"),
        ("pointer-events", "none"),
        ("width", "100px"),
        ("--child-inspector-token", "child"),
    ] {
        assert_eq!(properties.get(name).map(String::as_str), Some(expected));
    }

    vm.eval("document.getElementById('inspector-child-frame').style.display = 'none'")
        .expect("child frame should become hidden");
    assert_eq!(
        vm.computed_style_properties_for_inspector_handle(target),
        Some(Vec::new()),
        "an existing element in a hidden child frame has an empty computed declaration",
    );
}

#[test]
fn inspector_computed_style_bulk_read_uses_target_shadow_scope_once() {
    let mut vm = new_storage_test_vm("https://computed-style-inspector-shadow.test/");
    vm.eval(
        r#"
const host = document.createElement('div');
(document.body || document.documentElement || document).appendChild(host);
host.attachShadow({ mode: 'open' }).innerHTML = `
  <style>
    #inspector-shadow-target {
      color: rgb(1, 2, 3);
      pointer-events: none;
      --shadow-inspector-token: shadow;
    }
  </style>
  <div id="inspector-shadow-target"></div>`;
"#,
    )
    .expect("shadow inspector computed-style fixture should initialize");
    let target = element_handle_by_id(&vm, "inspector-shadow-target");
    let builds_before = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test();

    let properties = vm
        .computed_style_properties_for_inspector_handle(target)
        .expect("shadow element should resolve")
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>();

    let input_builds = vm
        ._context_host
        .borrow()
        .stylo_computed_style_input_builds_for_test()
        .saturating_sub(builds_before);
    assert_eq!(
        input_builds, 1,
        "one shadow-target inspector bulk read must use one input snapshot",
    );
    for (name, expected) in [
        ("color", "rgb(1, 2, 3)"),
        ("pointer-events", "none"),
        ("--shadow-inspector-token", "shadow"),
    ] {
        assert_eq!(properties.get(name).map(String::as_str), Some(expected));
    }
}

#[test]
fn computed_style_map_exposes_live_typed_values_and_readonly_map_shape() {
    let mut vm = new_storage_test_vm("https://computed-style-map.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  target.style.cssText = 'font-size: 21px; display: block';
  (document.body || document.documentElement || document).appendChild(target);

  const map = target.computedStyleMap();
  const first = map.get('font-size');
  const display = map.get('display');
  target.style.fontSize = '32px';
  const updated = map.get('FoNt-SiZe');
  let invalid = 'none';
  try {
    map.get('not-a-real-property');
  } catch (error) {
    invalid = error && error.name;
  }
  const constructed = new CSSUnitValue(2.5, 'PX');
  constructed.value = 3;
  const constructedKeyword = new CSSKeywordValue('inline');
  constructedKeyword.value = 'grid';
  let emptyKeyword = 'none';
  try {
    constructedKeyword.value = '';
  } catch (error) {
    emptyKeyword = error && error.name;
  }
  const entries = Array.from(map);
  const fontEntry = entries.find(([name]) => name === 'font-size');

  return JSON.stringify({
    sameMap: map === target.computedStyleMap(),
    mapBrand: map instanceof StylePropertyMapReadOnly,
    firstBrand: first instanceof CSSUnitValue &&
      first instanceof CSSNumericValue &&
      first instanceof CSSStyleValue,
    first: [first.value, first.unit, first.toString()],
    updated: updated.toString(),
    keywordBrand: display instanceof CSSKeywordValue &&
      display instanceof CSSStyleValue,
    keyword: [display.value, display.toString()],
    has: map.has('font-size'),
    missingCustom: map.get('--missing') === undefined,
    getAll: map.getAll('font-size').map(value => value.toString()),
    size: map.size >= 8,
    keys: Array.from(map.keys()).includes('font-size'),
    entry: fontEntry && [
      fontEntry[0],
      Array.isArray(fontEntry[1]),
      fontEntry[1][0].toString()
    ],
    invalid,
    constructed: [
      constructed.value,
      constructed.unit,
      constructed.toString(),
      Object.prototype.toString.call(constructed)
    ],
    constructedKeyword: [
      constructedKeyword.value,
      constructedKeyword.toString(),
      Object.prototype.toString.call(constructedKeyword),
      emptyKeyword
    ]
  });
})()
"#,
        )
        .expect("computed StylePropertyMap should expose typed computed values");

    assert_eq!(
        result,
        r#"{"sameMap":true,"mapBrand":true,"firstBrand":true,"first":[21,"px","21px"],"updated":"32px","keywordBrand":true,"keyword":["block","block"],"has":true,"missingCustom":true,"getAll":["32px"],"size":true,"keys":true,"entry":["font-size",true,"32px"],"invalid":"TypeError","constructed":[3,"px","3px","[object CSSUnitValue]"],"constructedKeyword":["grid","grid","[object CSSKeywordValue]","TypeError"]}"#
    );
}

#[test]
fn computed_style_map_uses_native_reads_and_validates_typed_om_calls() {
    let mut vm = new_storage_test_vm("https://computed-style-map-native.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  target.style.cssText =
    'transition-duration: 1s, 2s; --Token: exact; font-size: 18px';
  (document.body || document.documentElement || document).appendChild(target);
  const map = target.computedStyleMap();

  getComputedStyle = () => ({ getPropertyValue: () => 'wrong' });
  CSSStyleDeclaration.prototype.getPropertyValue = () => 'also-wrong';

  const errorName = callback => {
    try {
      callback();
      return 'none';
    } catch (error) {
      return error && error.name;
    }
  };
  const durations = map.getAll('transition-duration');
  const custom = map.get('--Token');
  target.remove();
  const removed = map.get('font-size');
  (document.body || document.documentElement || document).appendChild(target);

  return JSON.stringify({
    firstDuration: map.get('transition-duration').toString(),
    durations: durations.map(value => [
      value.toString(),
      value instanceof CSSUnitValue,
      value.unit
    ]),
    custom: custom && custom.toString(),
    customCaseSensitive: map.get('--token') === undefined,
    nativeRead: map.get('font-size').toString(),
    removed: removed === undefined,
    reattached: map.get('font-size').toString(),
    errors: [
      errorName(() => map.getAll('not-a-real-property')),
      errorName(() => map.has('not-a-real-property')),
      errorName(() => StylePropertyMapReadOnly.prototype.get.call({}, 'font-size')),
      errorName(() => new CSSUnitValue(0, 'lemon')),
      errorName(() => CSSUnitValue(1, 'px'))
    ]
  });
})()
"#,
        )
        .expect("computed StylePropertyMap should use native computed style reads");

    assert_eq!(
        result,
        r#"{"firstDuration":"1s","durations":[["1s",true,"s"],["2s",true,"s"]],"custom":"exact","customCaseSensitive":true,"nativeRead":"18px","removed":true,"reattached":"18px","errors":["TypeError","TypeError","TypeError","TypeError","TypeError"]}"#
    );
}

#[test]
fn computed_style_enumerates_registered_custom_properties() {
    let mut vm = new_storage_test_vm("https://computed-style-custom-properties.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.body || document.documentElement || document;
  const outer = document.createElement('div');
  outer.id = 'outer';
  const innerNode = document.createElement('div');
  innerNode.id = 'inner';
  const siblingNode = document.createElement('div');
  siblingNode.id = 'sibling';
  outer.append(innerNode, siblingNode);
  root.appendChild(outer);
  const style = document.createElement('style');
  style.textContent = `
    @property --non-inherited-length {
      syntax: "<length>";
      inherits: false;
      initial-value: 0px;
    }
    @property --inherited-length {
      syntax: "<length>";
      inherits: true;
      initial-value: 0px;
    }
    @property --universal-without-initial {
      syntax: "*";
      inherits: false;
    }
    #outer { --non-registered-outer: 1px; }
    #inner { --non-registered-inner: 2px; }
    #sibling { --universal-without-initial: bar; }
  `;
  (document.head || root).appendChild(style);
  const inner = Array.from(getComputedStyle(document.getElementById('inner')));
  const sibling = Array.from(getComputedStyle(document.getElementById('sibling')));
  return JSON.stringify({
    innerRegistered: inner.includes('--non-inherited-length') && inner.includes('--inherited-length'),
    innerInherited: inner.includes('--non-registered-outer'),
    innerOwn: inner.includes('--non-registered-inner'),
    innerNoInitial: inner.includes('--universal-without-initial'),
    siblingInherited: sibling.includes('--non-registered-outer'),
    siblingOwnNoInitial: sibling.includes('--universal-without-initial'),
    siblingInnerAbsent: sibling.includes('--non-registered-inner')
  });
})()
"#,
        )
        .expect("computed style should enumerate registered custom properties");

    assert_eq!(
        result,
        r#"{"innerRegistered":true,"innerInherited":true,"innerOwn":true,"innerNoInitial":false,"siblingInherited":true,"siblingOwnNoInitial":true,"siblingInnerAbsent":false}"#
    );
}
#[test]
fn css_register_property_validates_and_updates_computed_style() {
    let mut vm = new_storage_test_vm("https://css-register-property.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.body || document.documentElement || document;
  const target = document.createElement('div');
  target.id = 'target';
  target.style.cssText = '--registered-length: 12px; --registered-color: nope;';
  root.appendChild(target);

  const errorName = (fn) => {
    try {
      fn();
      return 'none';
    } catch (error) {
      return `${error && error.name}:${error instanceof DOMException}`;
    }
  };

  const errors = [
    errorName(() => CSS.registerProperty()),
    errorName(() => CSS.registerProperty({ name: '--missing-inherits' })),
    errorName(() => CSS.registerProperty({ name: 'no-leading-dash', inherits: false })),
    errorName(() => CSS.registerProperty({
      name: '--bad-syntax',
      syntax: '<banana>',
      initialValue: 'banana',
      inherits: false
    })),
    errorName(() => CSS.registerProperty({
      name: '--missing-initial',
      syntax: '<length>',
      inherits: false
    })),
    errorName(() => CSS.registerProperty({
      name: '--invalid-universal',
      syntax: '*',
      initialValue: 'semi;colon',
      inherits: false
    })),
    errorName(() => CSS.registerProperty({
      name: '--dependent-length',
      syntax: '<length>',
      initialValue: 'calc(4px + 3em)',
      inherits: false
    })),
    errorName(() => CSS.registerProperty({
      name: '--unitless-angle',
      syntax: '<angle>',
      initialValue: '0',
      inherits: false
    })),
    errorName(() => CSS.registerProperty({
      name: '--negative-resolution',
      syntax: '<resolution>',
      initialValue: '-5.3dpcm',
      inherits: false
    })),
    errorName(() => CSS.registerProperty({
      name: '--empty-transform',
      syntax: '<transform-function>',
      initialValue: 'scale()',
      inherits: false
    })),
    errorName(() => CSS.registerProperty({
      name: '--image-none',
      syntax: '<image>',
      initialValue: 'none',
      inherits: false
    }))
  ];

  CSS.registerProperty({
    name: '--registered-length',
    syntax: '<length>',
    initialValue: '4px',
    inherits: false
  });
  CSS.registerProperty({
    name: '--registered-color',
    syntax: '<color>',
    initialValue: 'red',
    inherits: false
  });
  CSS.registerProperty({
    name: '--registered-image-light-dark',
    syntax: '<image>',
    initialValue: 'light-dark(none, none)',
    inherits: false
  });
  const duplicate = errorName(() => CSS.registerProperty({
    name: '--registered-length',
    syntax: '<percentage>',
    initialValue: '0%',
    inherits: false
  }));
  const computed = getComputedStyle(target);
  return JSON.stringify({
    type: typeof CSS.registerProperty,
    length: CSS.registerProperty.length,
    errors,
    duplicate,
    registeredLength: computed.getPropertyValue('--registered-length'),
    registeredColor: computed.getPropertyValue('--registered-color')
  });
})()
"#,
        )
        .expect("CSS.registerProperty should validate and update computed style");

    assert_eq!(
        result,
        r#"{"type":"function","length":1,"errors":["TypeError:false","TypeError:false","SyntaxError:true","SyntaxError:true","SyntaxError:true","SyntaxError:true","SyntaxError:true","SyntaxError:true","SyntaxError:true","SyntaxError:true","SyntaxError:true"],"duplicate":"InvalidModificationError:true","registeredLength":"12px","registeredColor":"rgb(255, 0, 0)"}"#
    );
}

#[test]
fn child_frame_css_register_property_is_scoped_to_child_document_world() {
    let mut vm = new_storage_test_vm("https://child-register-property-scope.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));

  CSS.registerProperty({
    name: '--cross-doc-token',
    syntax: '<length>',
    initialValue: '9px',
    inherits: false
  });

  const active = document.createElement('div');
  active.id = 'active-register-target';
  active.style.cssText = '--cross-doc-token: rgb(1, 2, 3); width: var(--cross-doc-token);';
  body.appendChild(active);

  const frame = document.createElement('iframe');
  frame.id = 'register-child-frame';
  body.appendChild(frame);
  const childWindow = frame.contentWindow;
  const childDocument = childWindow.document;
  childDocument.open();
  childDocument.write('<body><div id="child-register-target" style="--cross-doc-token: 8px; color: var(--cross-doc-token)"></div></body>');
  childDocument.close();

  childWindow.CSS.registerProperty({
    name: '--cross-doc-token',
    syntax: '<color>',
    initialValue: 'rgb(4, 5, 6)',
    inherits: false
  });

  const childTarget = childDocument.getElementById('child-register-target');
  return [
    getComputedStyle(active).width,
    childWindow.getComputedStyle(childTarget).color,
    childWindow.CSS !== CSS
  ].join('|');
})()
"#,
        )
        .expect("child frame CSS.registerProperty should evaluate");

    assert_eq!(result, "9px|rgb(4, 5, 6)|true");

    let active_document = vm.document_runtime.dom_host().document_handle();
    let child_document = child_document_handle_for_frame_id(&vm, "register-child-frame");
    assert!(computed_style_cache_entry_count_for_document(&vm, active_document) > 0);
    assert!(computed_style_cache_entry_count_for_document(&vm, child_document) > 0);
}

#[test]
fn isolated_world_css_register_property_uses_root_document_world() {
    let mut vm = new_storage_test_vm("https://isolated-register-property-scope.test/");
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const target = document.createElement('div');
  target.id = 'isolated-register-target';
  target.style.cssText = '--isolated-token: 12px; color: var(--isolated-token);';
  body.appendChild(target);
  return getComputedStyle(target).color;
})()
"#,
        )
        .expect("isolated CSS.registerProperty setup should evaluate");

    assert_eq!(initial, "rgb(0, 0, 0)");
    assert!(computed_style_cache_entry_count_for_document(&vm, document) > 0);

    let context_id = vm
        .create_isolated_world("style-register-property-test", false)
        .expect("isolated world should be created");
    let registered = vm
        .eval_in_isolated_context(
            context_id,
            r#"
(() => {
  CSS.registerProperty({
    name: '--isolated-token',
    syntax: '<color>',
    initialValue: 'rgb(10, 20, 30)',
    inherits: false
  });
  return typeof CSS.registerProperty;
})()
"#,
        )
        .expect("isolated CSS.registerProperty should evaluate");

    assert_eq!(registered, "function");
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, document),
        0
    );

    let resolved = vm
        .eval(
            r#"
(() => getComputedStyle(document.getElementById('isolated-register-target')).color)()
"#,
        )
        .expect("default world computed style should use isolated registration");

    assert_eq!(resolved, "rgb(10, 20, 30)");
    assert!(computed_style_cache_entry_count_for_document(&vm, document) > 0);
}

#[test]
fn popup_css_register_property_uses_popup_document_world() {
    let mut vm = new_storage_test_vm("https://popup-register-property-scope.test/");
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const active = document.createElement('div');
  active.id = 'active-popup-register-target';
  active.style.cssText = '--popup-token: 12px; color: var(--popup-token);';
  body.appendChild(active);

  const popup = open('about:blank');
  globalThis.__styleRegisterPopup = popup;
  const popupBody = popup.document.body || popup.document.documentElement || popup.document;
  const target = popup.document.createElement('div');
  target.id = 'popup-register-target';
  target.style.cssText = '--popup-token: 12px; color: var(--popup-token);';
  popupBody.appendChild(target);
  const plain = popup.document.createElement('div');
  plain.id = 'popup-plain-color-target';
  plain.style.cssText = 'color: rgb(7, 8, 9); width: 25%; height: 50%;';
  popupBody.appendChild(plain);

  return [
    getComputedStyle(active).color,
    popup.getComputedStyle(target).color,
    popup.getComputedStyle(plain).color,
    popup.getComputedStyle(plain).width,
    popup.getComputedStyle(plain).height,
    plain.getAttribute('style'),
    popup.CSS !== CSS
  ].join('|');
})()
"#,
        )
        .expect("popup CSS.registerProperty setup should evaluate");

    assert_eq!(
        initial,
        "rgb(0, 0, 0)|rgb(0, 0, 0)|rgb(7, 8, 9)|480px|540px|color: rgb(7, 8, 9); width: 25%; height: 50%;|true"
    );
    let popup_document = owner_document_handle_for_element_id(&vm, "popup-register-target");
    assert_ne!(popup_document, document);
    assert!(computed_style_cache_entry_count_for_document(&vm, document) > 0);
    assert!(computed_style_cache_entry_count_for_document(&vm, popup_document) > 0);
    assert!(!registered_custom_property_for_document(
        &vm,
        document,
        "--popup-token"
    ));
    assert!(!registered_custom_property_for_document(
        &vm,
        popup_document,
        "--popup-token"
    ));

    let registered = vm
        .eval(
            r#"
(() => {
  __styleRegisterPopup.CSS.registerProperty({
    name: '--popup-token',
    syntax: '<color>',
    initialValue: 'rgb(40, 50, 60)',
    inherits: false
  });
  return typeof __styleRegisterPopup.CSS.registerProperty;
})()
"#,
        )
        .expect("popup CSS.registerProperty should evaluate");

    assert_eq!(registered, "function");
    assert!(computed_style_cache_entry_count_for_document(&vm, document) > 0);
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, popup_document),
        0
    );
    assert!(!registered_custom_property_for_document(
        &vm,
        document,
        "--popup-token"
    ));
    assert!(registered_custom_property_for_document(
        &vm,
        popup_document,
        "--popup-token"
    ));

    let resolved = vm
        .eval(
            r#"
(() => {
  const active = document.getElementById('active-popup-register-target');
  const popupTarget = __styleRegisterPopup.document.getElementById('popup-register-target');
  return [
    getComputedStyle(active).color,
    __styleRegisterPopup.getComputedStyle(popupTarget).color,
    __styleRegisterPopup.getComputedStyle(popupTarget).getPropertyValue('color'),
    __styleRegisterPopup.getComputedStyle(popupTarget).getPropertyValue('--popup-token')
  ].join('|');
})()
"#,
        )
        .expect("computed styles should use each document registration");

    assert_eq!(
        resolved,
        "rgb(0, 0, 0)|rgb(40, 50, 60)|rgb(40, 50, 60)|rgb(40, 50, 60)"
    );
    assert!(computed_style_cache_entry_count_for_document(&vm, document) > 0);
    assert!(computed_style_cache_entry_count_for_document(&vm, popup_document) > 0);
}

#[test]
fn popup_held_computed_style_wrapper_is_empty_after_about_blank_navigation() {
    let mut vm = new_storage_test_vm("https://popup-held-computed-navigation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const popup = open('about:blank');
  const target = popup.document.createElement('div');
  target.id = 'popup-held-computed-target';
  target.style.cssText = 'color: rgb(11, 22, 33);';
  popup.document.body.appendChild(target);
  const held = popup.getComputedStyle(target);
  const oldDocument = popup.document;
  const before = [held.color, held.length > 200].join(':');

  popup.location.href = 'about:blank?next';
  const replacement = popup.document.createElement('div');
  replacement.style.cssText = 'color: rgb(44, 55, 66);';
  popup.document.body.appendChild(replacement);

  return [
    before,
    popup.location.href,
    popup.document !== oldDocument,
    held.color,
    held.length,
    popup.getComputedStyle(replacement).color
  ].join('|');
})()
"#,
        )
        .expect("popup held computed style navigation should evaluate");

    assert_eq!(
        result,
        "rgb(11, 22, 33):true|about:blank?next|true||0|rgb(44, 55, 66)"
    );
}

#[tokio::test]
async fn popup_held_computed_style_wrapper_is_empty_after_loaded_navigation() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://popup-held-loaded-computed-navigation.test/",
        &loader,
    );

    let setup = vm
        .eval(
            r#"
(() => {
  const popup = open('about:blank');
  globalThis.__popupLoadedComputedReady = false;
  globalThis.__popupLoadedComputedPopup = popup;
  const target = popup.document.createElement('div');
  target.style.cssText = 'color: rgb(11, 22, 33);';
  popup.document.body.appendChild(target);
  globalThis.__popupLoadedComputedHeld = popup.getComputedStyle(target);
  globalThis.__popupLoadedComputedOldDocument = popup.document;
  const html = `<!doctype html><body>
    <div id="replacement" style="color: rgb(44, 55, 66)">replacement</div>
    <script>opener.__popupLoadedComputedReady = true;<\/script>
  </body>`;
  globalThis.__popupLoadedComputedUrl =
    URL.createObjectURL(new Blob([html], { type: 'text/html' }));
  popup.location.href = __popupLoadedComputedUrl;
  return [
    __popupLoadedComputedHeld.color,
    __popupLoadedComputedHeld.length > 200,
    String(__popupLoadedComputedReady),
    popup.location.href === __popupLoadedComputedUrl
  ].join('|');
})()
"#,
        )
        .expect("popup loaded held computed style setup should evaluate");

    assert_eq!(setup, "rgb(11, 22, 33)|true|false|true");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__popupLoadedComputedReady)",
        "true",
        "loaded popup document should run",
    )
    .await;

    let result = vm
        .eval(
            r#"
(() => {
  const popup = __popupLoadedComputedPopup;
  const replacement = popup.document.getElementById('replacement');
  return [
    popup.document !== __popupLoadedComputedOldDocument,
    __popupLoadedComputedHeld.color,
    __popupLoadedComputedHeld.length,
    popup.getComputedStyle(replacement).color,
    popup.location.href === __popupLoadedComputedUrl
  ].join('|');
})()
"#,
        )
        .expect("popup loaded held computed style result should evaluate");

    assert_eq!(result, "true||0|rgb(44, 55, 66)|true");
}

#[test]
fn css_register_property_updates_var_substitution_computed_value() {
    let mut vm = new_storage_test_vm("https://css-register-property-var-cascade.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.body || document.documentElement || document;
  const outer = document.createElement('div');
  outer.style.color = 'rgb(1, 1, 1)';
  const inner = document.createElement('div');
  inner.style.cssText = `
    --my-color: rgb(2, 2, 2);
    --my-color: url(not-a-color);
    color: var(--my-color);
  `;
  outer.appendChild(inner);
  root.appendChild(outer);

  const before = getComputedStyle(inner).color;
  CSS.registerProperty({
    name: '--my-color',
    syntax: '<color>',
    initialValue: 'rgb(3, 3, 3)',
    inherits: false
  });
  const after = getComputedStyle(inner).color;
  return `${before}|${after}`;
})()
"#,
        )
        .expect("registered custom property should update var substitution computed value");

    assert_eq!(result, "rgb(1, 1, 1)|rgb(3, 3, 3)");
}

#[test]
fn css_register_property_inline_var_mutation_preserves_existing_font_size_unset() {
    let mut vm =
        new_storage_test_vm("https://css-register-property-inline-var-preserve-unset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  CSS.registerProperty({
    name: '--x',
    syntax: '*',
    initialValue: '0px',
    inherits: false
  });
  const style = document.createElement('style');
  style.textContent = `
    :root, #target { --x: 2em; }
    #target { font-size: 11px; line-height: 13px; }
  `;
  head.appendChild(style);
  const target = document.createElement('div');
  target.id = 'target';
  body.appendChild(target);

  target.style.fontSize = 'unset';
  const before = getComputedStyle(target).fontSize;
  target.style.marginBottom = 'var(--x)';
  const after = getComputedStyle(target).fontSize;
  const margin = getComputedStyle(target).marginBottom;
  const cssText = target.style.cssText;
  target.remove();
  style.remove();
  return [cssText, before, after, margin].join('|');
})()
"#,
        )
        .expect("registered custom property inline var mutation should evaluate");

    assert_eq!(
        result,
        "font-size: unset; margin-bottom: var(--x);|16px|16px|32px"
    );
}

#[test]
fn registered_custom_ident_accepts_ident_function() {
    let mut vm = new_storage_test_vm("https://registered-custom-ident-function.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.body || document.documentElement || document;
  CSS.registerProperty({
    name: '--ident',
    syntax: '<custom-ident>',
    inherits: true,
    initialValue: 'none'
  });
  const target = document.createElement('div');
  target.style.setProperty('--ident', 'ident("--myident" calc(42 * sign(1em - 1px)))');
  root.appendChild(target);
  const value = getComputedStyle(target).getPropertyValue('--ident');
  target.remove();
  return value;
})()
"#,
        )
        .expect("registered custom-ident ident function should evaluate");

    assert_eq!(result, "--myident42");
}

#[test]
fn registered_color_resolves_tree_counting_math() {
    let mut vm = new_storage_test_vm("https://registered-color-tree-counting.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = `
    @property --color {
      inherits: false;
      initial-value: black;
      syntax: "<color>";
    }
    #target {
      --color: color(srgb 0 sibling-index() 0);
    }
  `;
  head.appendChild(style);
  const parent = document.createElement('div');
  const target = document.createElement('div');
  target.id = 'target';
  parent.appendChild(target);
  body.appendChild(parent);
  return getComputedStyle(target).getPropertyValue('--color');
})()
"#,
        )
        .expect("registered color tree-counting math should evaluate");

    assert_eq!(result, "color(srgb 0 1 0)");
}

#[test]
fn computed_style_resolves_border_width_from_border_shorthand() {
    let mut vm = new_storage_test_vm("https://computed-border-width.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('input');
  target.type = 'checkbox';
  target.style = 'border: 5px solid red';
  (document.body || document.documentElement || document).appendChild(target);
  const style = getComputedStyle(target);
  return [
    style.getPropertyValue('border-width'),
    style.getPropertyValue('border-top-width'),
    style.getPropertyValue('border-right-width'),
    style.getPropertyValue('border-bottom-width'),
    style.getPropertyValue('border-left-width')
  ].join('|');
})()
"#,
        )
        .expect("computed style should resolve border widths from border shorthand");

    assert_eq!(result, "5px|5px|5px|5px|5px");
}
#[test]
fn keyframe_rule_style_rejects_animation_properties() {
    let mut vm = new_storage_test_vm("https://keyframe-style-restrictions.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = '@keyframes foo { from { margin-top: 10px; animation-name: none; } }';
  (document.body || document.documentElement || document).appendChild(style);
  const declaration = document.styleSheets[0].cssRules[0].cssRules[0].style;
  const initial = `${declaration.length}|${declaration.marginTop}|${declaration.getPropertyValue('animation-name')}`;
  declaration.setProperty('animation-name', 'none');
  const afterSet = `${declaration.length}|${declaration.getPropertyValue('animation-name')}`;
  declaration.cssText = 'margin-bottom: 10px; animation-name: none;';
  const afterText = `${declaration.length}|${declaration.marginBottom}|${declaration.getPropertyValue('animation-name')}`;
  return `${initial}|${afterSet}|${afterText}`;
})()
"#,
        )
        .expect("keyframe style should reject animation properties");

    assert_eq!(result, "1|10px||1||1|10px|");
}
#[test]
fn computed_style_resolves_system_color_properties() {
    let mut vm = new_storage_test_vm("https://computed-system-colors.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const target = document.createElement('div');
  target.id = 'target';
  const style = document.createElement('style');
  style.textContent = `
    #target {
      background-color: Menu;
      border: 1px solid Menu;
      box-shadow: 1px 1px MenuText, 2px 2px LinkText;
      caret-color: Menu;
      color: Menu;
      outline-color: Menu;
    }
  `;
  body.append(style, target);
  const computed = getComputedStyle(target);
  const properties = [
    'background-color',
    'border-top-color',
    'border-right-color',
    'border-bottom-color',
    'border-left-color',
    'box-shadow',
    'caret-color',
    'color',
    'outline-color'
  ];
  return JSON.stringify({
    allRgb: properties.every((property) => /^rgb/.test(computed.getPropertyValue(property))),
    boxShadow: computed.getPropertyValue('box-shadow')
  });
})()
"#,
        )
        .expect("computed style should resolve system colors");

    assert_eq!(
        result,
        r#"{"allRgb":true,"boxShadow":"rgb(0, 0, 0) 1px 1px 0px 0px, rgb(0, 0, 238) 2px 2px 0px 0px"}"#
    );
}

#[test]
fn computed_style_resolves_env_color_fallbacks() {
    let mut vm = new_storage_test_vm("https://computed-env-color.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = 'div { background-color: rgb(0, 128, 0); }';
  body.appendChild(style);

  const values = [
    'env(test)',
    'ENV(test)',
    'env(test, blue)',
    'env(test, env(another, blue))',
    'env(test, {})',
    'env(env(test))'
  ];
  return values.map((value) => {
    const element = document.createElement('div');
    body.appendChild(element);
    element.style.backgroundColor = value;
    return getComputedStyle(element).getPropertyValue('background-color');
  }).join('|');
})()
"#,
        )
        .expect("computed env() color fallbacks should evaluate");

    assert_eq!(
        result,
        "rgba(0, 0, 0, 0)|rgba(0, 0, 0, 0)|rgb(0, 0, 255)|rgb(0, 0, 255)|rgba(0, 0, 0, 0)|rgb(0, 128, 0)"
    );
}
#[test]
fn computed_style_resolves_auto_min_size() {
    let mut vm = new_storage_test_vm("https://computed-auto-min-size.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  body.innerHTML = `
    <div id="plain"></div>
    <div id="ratio" style="aspect-ratio: 1/1"></div>
    <div style="display:flex"><div id="flexItem"></div></div>
    <div style="display:none"><div id="hiddenRatio" style="aspect-ratio: 1/1"></div></div>
  `;
  const ids = ['plain', 'ratio', 'flexItem', 'hiddenRatio'];
  return ids.map((id) => {
    const style = getComputedStyle(document.getElementById(id));
    return `${style.minWidth}/${style.minHeight}`;
  }).join('|');
})()
"#,
        )
        .expect("computed style should resolve auto min sizes");

    assert_eq!(result, "0px/0px|auto/auto|auto/auto|0px/0px");
}
#[test]
fn computed_style_enumerates_logical_longhands_only() {
    let mut vm = new_storage_test_vm("https://computed-style-logical.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  (document.body || document.documentElement || document).appendChild(target);
  const properties = Array.from(getComputedStyle(target));
  return JSON.stringify({
    blockSize: properties.includes('block-size'),
    paddingBlock: properties.includes('padding-block'),
    safeAreaInsetTop: properties.includes('safe-area-inset-top'),
  });
})()
"#,
        )
        .expect("computed style logical property enumeration should evaluate");

    assert_eq!(
        result,
        r#"{"blockSize":true,"paddingBlock":false,"safeAreaInsetTop":false}"#
    );
}
#[test]
fn detached_child_document_style_sheet_survives_iframe_removal() {
    let mut vm = new_storage_test_vm("https://stylesheet-removed-frame.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  frame.contentDocument.body.innerHTML = '<style>div { color: red; }</style>';
  const sheet = frame.contentDocument.querySelector('style').sheet;
  const before = sheet && sheet.cssRules.length;
  frame.remove();
  sheet.insertRule('span { color: green; }', 0);
  return [before, sheet.cssRules.length].join(',');
})()
"#,
        )
        .expect("removed child document stylesheet should remain mutable");

    assert_eq!(result, "1,2");
}
#[test]
fn live_inline_font_shorthand_serializes_line_height_slash_spacing() {
    let mut vm = new_storage_test_vm("https://font-shorthand-css-text.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  target.setAttribute('style', 'font: 10px/1 Ahem;');
  const attribute = [target.style.cssText, target.style.font].join('|');
  target.style.font = 'italic 16px/2 "A B", serif';
  const setter = [target.style.cssText, target.style.font].join('|');
  return `${attribute}||${setter}`;
})()
"#,
        )
        .expect("font shorthand serialization should evaluate");

    assert_eq!(
        result,
        r#"font: 10px / 1 Ahem;|10px / 1 Ahem||font: italic 16px / 2 "A B", serif;|italic 16px / 2 "A B", serif"#
    );
}
#[test]
fn computed_color_normalizes_named_and_hex_colors() {
    let mut vm = new_storage_test_vm("https://computed-color-normalization.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const values = ['white', 'orange', 'rebeccapurple', 'transparent', '#0f8', '#112233'];
  return values.map((value) => {
    const element = document.createElement('div');
    element.style.color = value;
    document.appendChild(element);
    return getComputedStyle(element).color;
  }).join('|');
})()
"##,
        )
        .expect("computed color normalization should evaluate");

    assert_eq!(
        result,
        "rgb(255, 255, 255)|rgb(255, 165, 0)|rgb(102, 51, 153)|rgba(0, 0, 0, 0)|rgb(0, 255, 136)|rgb(17, 34, 51)"
    );
}

#[test]
fn computed_currentcolor_resolves_for_text_decoration_independent_of_webkit_text_fill() {
    let mut vm = new_storage_test_vm("https://computed-text-fill-currentcolor.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('p');
  target.style.cssText = 'text-decoration-color: currentColor; color: blue; -webkit-text-fill-color: red;';
  (document.body || document.documentElement || document).appendChild(target);
  const computed = getComputedStyle(target);
  return [
    computed.getPropertyValue('text-decoration-color'),
    computed.getPropertyValue('color'),
    computed.getPropertyValue('-webkit-text-fill-color')
  ].join('|');
})()
"#,
        )
        .expect("computed text decoration color should resolve currentColor");

    assert_eq!(result, "rgb(0, 0, 255)|rgb(0, 0, 255)|rgb(255, 0, 0)");
}
#[test]
fn computed_color_inherits_through_shadow_host_and_hidden_subtree() {
    let mut vm = new_storage_test_vm("https://computed-color-inheritance.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = '#container { color: red }';
  (document.head || document.documentElement || document).appendChild(style);
  const container = document.createElement('div');
  container.id = 'container';
  const host = document.createElement('div');
  const hidden = document.createElement('div');
  hidden.style.display = 'none';
  const hiddenChild = document.createElement('span');
  hidden.appendChild(hiddenChild);
  container.append(host, hidden);
  (document.body || document.documentElement || document).appendChild(container);
  const root = host.attachShadow({ mode: 'open' });
  root.innerHTML = '<div id="target"></div>';
  return [
    getComputedStyle(root.getElementById('target')).color,
    getComputedStyle(hiddenChild).color
  ].join('|');
})()
"#,
        )
        .expect("computed color should inherit through shadow and hidden subtrees");

    assert_eq!(result, "rgb(255, 0, 0)|rgb(255, 0, 0)");
}
#[test]
fn target_pseudo_updates_computed_style_after_fragment_changes() {
    let mut vm = new_parsed_test_vm(
        "https://target-style.test/#old",
        r#"
        <html>
          <head>
            <style>
              .probe { color: rgb(0, 0, 0); }
              #old:target { color: rgb(255, 0, 0); }
              #new:target { color: rgb(0, 128, 0); }
            </style>
          </head>
          <body>
            <div id="old" class="probe"></div>
            <div id="new" class="probe"></div>
          </body>
        </html>
        "#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const oldTarget = document.getElementById('old');
  const newTarget = document.getElementById('new');
  const read = () => [
    oldTarget.matches(':target'),
    newTarget.matches(':target'),
    getComputedStyle(oldTarget).color,
    getComputedStyle(newTarget).color
  ].join('/');

  const initial = read();
  history.replaceState(null, '', '#new');
  const replaced = read();
  history.replaceState(null, '', '#missing');
  const missing = read();

  return [initial, replaced, missing].join('|');
})()
"#,
        )
        .expect("target pseudo computed style should update after fragment changes");

    assert_eq!(
        result,
        "true/false/rgb(255, 0, 0)/rgb(0, 0, 0)|false/true/rgb(0, 0, 0)/rgb(0, 128, 0)|false/false/rgb(0, 0, 0)/rgb(0, 0, 0)"
    );
}
#[test]
fn target_selector_fragment_change_invalidates_held_computed_style() {
    let mut vm = new_parsed_test_vm(
        "https://target-held-style.test/#old",
        r#"
        <html>
          <head>
            <style>
              .probe { color: rgb(0, 0, 0); }
              #old:target { color: rgb(255, 0, 0); }
              #new:target { color: rgb(0, 128, 0); }
            </style>
          </head>
          <body>
            <div id="old" class="probe"></div>
            <div id="new" class="probe"></div>
          </body>
        </html>
        "#,
    );
    let document = vm.document_handle_for_test();

    let setup = vm
        .eval(
            r#"
(() => {
  const oldTarget = document.getElementById('old');
  const newTarget = document.getElementById('new');
  globalThis.__targetOldStyle = getComputedStyle(oldTarget);
  globalThis.__targetNewStyle = getComputedStyle(newTarget);
  return [globalThis.__targetOldStyle.color, globalThis.__targetNewStyle.color].join('|');
})()
"#,
        )
        .expect("target selector held style setup should evaluate");

    assert_eq!(setup, "rgb(255, 0, 0)|rgb(0, 0, 0)");
    let generation_before_fragment =
        vm.computed_style_cache_generation_for_document_for_test(document);

    let replaced = vm
        .eval(
            r#"
(() => {
  history.replaceState(null, '', '#new');
  const result = [globalThis.__targetOldStyle.color, globalThis.__targetNewStyle.color].join('|');
  delete globalThis.__targetOldStyle;
  delete globalThis.__targetNewStyle;
  return result;
})()
"#,
        )
        .expect("target selector held style mutation should evaluate");

    assert_eq!(replaced, "rgb(0, 0, 0)|rgb(0, 128, 0)");
    assert_eq!(
        vm.computed_style_cache_generation_for_document_for_test(document),
        generation_before_fragment,
        "targeted :target invalidation should not bump the retained style generation"
    );
}

#[test]
fn child_frame_target_selector_invalidation_uses_child_document_world() {
    let mut vm = new_storage_test_vm("https://child-target-style-cache.test/");

    let setup = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const activeStyle = document.createElement('style');
  activeStyle.textContent = '#active-target-cache { color: rgb(1, 2, 3); }';
  head.appendChild(activeStyle);
  const active = document.createElement('div');
  active.id = 'active-target-cache';
  body.appendChild(active);
  globalThis.__childTargetActiveStyle = getComputedStyle(active);

  const frame = document.createElement('iframe');
  frame.id = 'target-child-frame';
  body.appendChild(frame);
  const childWindow = frame.contentWindow;
  const childDocument = childWindow.document;
  childDocument.open();
  childDocument.write(`
    <style>
      .probe { color: rgb(4, 5, 6); }
      #old:target { color: rgb(7, 8, 9); }
      #new:target { color: rgb(10, 11, 12); }
    </style>
    <body>
      <div id="old" class="probe"></div>
      <div id="new" class="probe"></div>
    </body>
  `);
  childDocument.close();
  childWindow.history.replaceState(null, '', '#old');
  globalThis.__childTargetFrame = frame;
  globalThis.__childTargetOldStyle =
    childWindow.getComputedStyle(childDocument.getElementById('old'));
  globalThis.__childTargetNewStyle =
    childWindow.getComputedStyle(childDocument.getElementById('new'));

  return [
    globalThis.__childTargetActiveStyle.color,
    globalThis.__childTargetOldStyle.color,
    globalThis.__childTargetNewStyle.color
  ].join('|');
})()
"#,
        )
        .expect("child frame target style setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)|rgb(7, 8, 9)|rgb(4, 5, 6)");
    let active_document = vm.document_runtime.dom_host().document_handle();
    let child_document = child_document_handle_for_frame_id(&vm, "target-child-frame");
    let active_cache_before = computed_style_cache_entry_count_for_document(&vm, active_document);
    assert!(active_cache_before > 0);
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, child_document),
        2
    );

    let replaced = vm
        .eval(
            r#"
(() => {
  const childWindow = globalThis.__childTargetFrame.contentWindow;
  childWindow.history.replaceState(null, '', '#new');
  const result = [
    globalThis.__childTargetOldStyle.color,
    globalThis.__childTargetNewStyle.color
  ].join('|');
  delete globalThis.__childTargetFrame;
  delete globalThis.__childTargetOldStyle;
  delete globalThis.__childTargetNewStyle;
  delete globalThis.__childTargetActiveStyle;
  return result;
})()
"#,
        )
        .expect("child frame target style mutation should evaluate");

    assert_eq!(replaced, "rgb(4, 5, 6)|rgb(10, 11, 12)");
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, active_document),
        active_cache_before,
        "child target invalidation should not clear active document cache"
    );
    assert!(
        computed_style_cache_entry_count_for_document(&vm, child_document) > 0,
        "child target invalidation should keep style work in the child document world"
    );
}

#[test]
fn popup_target_selector_invalidation_uses_popup_document_world() {
    let mut vm = new_storage_test_vm("https://popup-target-style-cache.test/");

    let setup = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const activeStyle = document.createElement('style');
  activeStyle.textContent = '#active-popup-target-cache { color: rgb(1, 2, 3); }';
  head.appendChild(activeStyle);
  const active = document.createElement('div');
  active.id = 'active-popup-target-cache';
  body.appendChild(active);
  globalThis.__popupTargetActiveStyle = getComputedStyle(active);

  const popup = open('about:blank');
  globalThis.__popupTargetWindow = popup;
  const popupRoot = popup.document.documentElement ||
    popup.document.appendChild(popup.document.createElement('html'));
  const popupHead = popup.document.head ||
    popupRoot.appendChild(popup.document.createElement('head'));
  const popupBody = popup.document.body ||
    popupRoot.appendChild(popup.document.createElement('body'));
  const style = popup.document.createElement('style');
  style.textContent = [
    '.probe { color: rgb(4, 5, 6); }',
    '#popup-old-target:target { color: rgb(7, 8, 9); }',
    '#popup-new-target:target { color: rgb(10, 11, 12); }'
  ].join('\n');
  popupHead.appendChild(style);
  const oldTarget = popup.document.createElement('div');
  oldTarget.id = 'popup-old-target';
  oldTarget.className = 'probe';
  const newTarget = popup.document.createElement('div');
  newTarget.id = 'popup-new-target';
  newTarget.className = 'probe';
  popupBody.append(oldTarget, newTarget);
  popup.history.replaceState(null, '', '#popup-old-target');
  globalThis.__popupTargetOldStyle = popup.getComputedStyle(oldTarget);
  globalThis.__popupTargetNewStyle = popup.getComputedStyle(newTarget);

  return [
    globalThis.__popupTargetActiveStyle.color,
    globalThis.__popupTargetOldStyle.color,
    globalThis.__popupTargetNewStyle.color
  ].join('|');
})()
"#,
        )
        .expect("popup target style setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)|rgb(7, 8, 9)|rgb(4, 5, 6)");
    let active_document = vm.document_handle_for_test();
    let popup_document = owner_document_handle_for_element_id(&vm, "popup-old-target");
    assert_ne!(popup_document, active_document);
    let active_cache_before = computed_style_cache_entry_count_for_document(&vm, active_document);
    assert!(active_cache_before > 0);
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, popup_document),
        2
    );

    let replaced = vm
        .eval(
            r#"
(() => {
  __popupTargetWindow.history.replaceState(null, '', '#popup-new-target');
  const result = [
    globalThis.__popupTargetOldStyle.color,
    globalThis.__popupTargetNewStyle.color
  ].join('|');
  delete globalThis.__popupTargetWindow;
  delete globalThis.__popupTargetOldStyle;
  delete globalThis.__popupTargetNewStyle;
  delete globalThis.__popupTargetActiveStyle;
  return result;
})()
"#,
        )
        .expect("popup target style mutation should evaluate");

    assert_eq!(replaced, "rgb(4, 5, 6)|rgb(10, 11, 12)");
    assert_eq!(
        computed_style_cache_entry_count_for_document(&vm, active_document),
        active_cache_before,
        "popup target invalidation should not clear active document cache"
    );
    assert!(
        computed_style_cache_entry_count_for_document(&vm, popup_document) > 0,
        "popup target invalidation should keep style work in the popup document world"
    );
}

#[test]
fn isolated_world_target_selector_invalidation_uses_root_document_world() {
    let mut vm = new_storage_test_vm("https://isolated-target-style-cache.test/");

    let setup = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = [
    '.probe { color: rgb(4, 5, 6); }',
    '#isolated-old-target:target { color: rgb(7, 8, 9); }',
    '#isolated-new-target:target { color: rgb(10, 11, 12); }'
  ].join('\n');
  head.appendChild(style);
  const oldTarget = document.createElement('div');
  oldTarget.id = 'isolated-old-target';
  oldTarget.className = 'probe';
  const newTarget = document.createElement('div');
  newTarget.id = 'isolated-new-target';
  newTarget.className = 'probe';
  body.append(oldTarget, newTarget);
  history.replaceState(null, '', '#isolated-old-target');
  globalThis.__isolatedTargetOldStyle = getComputedStyle(oldTarget);
  globalThis.__isolatedTargetNewStyle = getComputedStyle(newTarget);
  return [
    globalThis.__isolatedTargetOldStyle.color,
    globalThis.__isolatedTargetNewStyle.color
  ].join('|');
})()
"#,
        )
        .expect("isolated target style setup should evaluate");

    assert_eq!(setup, "rgb(7, 8, 9)|rgb(4, 5, 6)");
    let document = vm.document_handle_for_test();
    let generation_before_target =
        vm.computed_style_cache_generation_for_document_for_test(document);
    let context_id = vm
        .create_isolated_world("style-target-test", false)
        .expect("isolated world should be created");
    let changed = vm
        .eval_in_isolated_context(
            context_id,
            r#"
(() => {
  history.replaceState(null, '', '#isolated-new-target');
  return [
    location.hash,
    document.getElementById('isolated-new-target').matches(':target')
  ].join('|');
})()
"#,
        )
        .expect("isolated target mutation should evaluate");

    assert_eq!(changed, "#isolated-new-target|true");
    let resolved = vm
        .eval(
            r#"
(() => {
  const result = [
    globalThis.__isolatedTargetOldStyle.color,
    globalThis.__isolatedTargetNewStyle.color
  ].join('|');
  delete globalThis.__isolatedTargetOldStyle;
  delete globalThis.__isolatedTargetNewStyle;
  return result;
})()
"#,
        )
        .expect("default world held style should see isolated target invalidation");

    assert_eq!(resolved, "rgb(4, 5, 6)|rgb(10, 11, 12)");
    assert_eq!(
        vm.computed_style_cache_generation_for_document_for_test(document),
        generation_before_target,
        "isolated target invalidation should not bump the retained style generation"
    );
    assert!(computed_style_cache_entry_count_for_document(&vm, document) > 0);
}

#[test]
fn moving_stylesheet_link_into_shadow_root_removes_document_stylesheet_entry() {
    let mut vm = new_storage_test_vm("https://shadow-stylesheets-boundary.test/");
    let request_url =
        url::Url::parse("https://shadow-stylesheets-boundary.test/sheet.css").unwrap();
    let initial = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const initial = document.styleSheets.length;
  const link = document.createElement('link');
  link.id = 'shadow-boundary-link';
  link.rel = 'stylesheet';
  link.href = '/sheet.css';
  document.body.appendChild(link);
  const host = document.createElement('div');
  host.id = 'shadow-boundary-host';
  document.body.appendChild(host);
  host.attachShadow({ mode: 'open' });
  return initial;
})()
"#,
        )
        .expect("document stylesheet shadow-boundary setup should evaluate");
    let link = element_handle_by_id(&vm, "shadow-boundary-link");
    install_linked_stylesheet_for_test(
        &mut vm,
        link,
        request_url.clone(),
        crate::style_engine::StyloStylesheetSource::new(String::new(), request_url.clone())
            .with_sheet_url(request_url),
    );
    let result = vm
        .eval(
            r#"
(() => {
  const link = document.getElementById('shadow-boundary-link');
  const afterBody = document.styleSheets.length;
  document.getElementById('shadow-boundary-host').shadowRoot.appendChild(link);
  return [afterBody, document.styleSheets.length].join('|');
})()
"#,
        )
        .expect("document styleSheets should update when stylesheet moves into shadow root");

    assert_eq!(format!("{initial}|{result}"), "0|1|0");
}
#[test]
fn child_document_stylesheet_link_moved_into_shadow_root_is_hidden_from_document_stylesheets() {
    let mut vm = new_storage_test_vm("https://child-shadow-stylesheets-boundary.test/");
    let request_url =
        url::Url::parse("https://child-shadow-stylesheets-boundary.test/sheet.css").unwrap();
    let initial = vm
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
  document.body.appendChild(frame);
  const doc = frame.contentWindow.document;
  if (!doc.documentElement) {
    doc.appendChild(doc.createElement('html'));
  }
  if (!doc.body) {
    doc.documentElement.appendChild(doc.createElement('body'));
  }
  const initial = doc.styleSheets.length;
  const link = doc.createElement('link');
  link.id = 'child-shadow-boundary-link';
  link.setAttribute('rel', 'stylesheet');
  link.href = 'https://child-shadow-stylesheets-boundary.test/sheet.css';
  doc.body.appendChild(link);
  const host = doc.createElement('div');
  host.id = 'child-shadow-boundary-host';
  doc.body.appendChild(host);
  host.attachShadow({ mode: 'open' });
  return initial;
})()
"#,
        )
        .expect("child document stylesheet shadow-boundary setup should evaluate");
    let link = element_handle_by_id(&vm, "child-shadow-boundary-link");
    install_linked_stylesheet_for_test(
        &mut vm,
        link,
        request_url.clone(),
        crate::style_engine::StyloStylesheetSource::new(String::new(), request_url.clone())
            .with_sheet_url(request_url),
    );
    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.querySelector('iframe').contentDocument;
  const link = doc.getElementById('child-shadow-boundary-link');
  const afterBody = doc.styleSheets.length;
  doc.getElementById('child-shadow-boundary-host').shadowRoot.appendChild(link);
  return [afterBody, doc.styleSheets.length].join('|');
})()
"#,
        )
        .expect("child document styleSheets should update across shadow root moves");

    assert_eq!(format!("{initial}|{result}"), "0|1|0");
}
#[test]
fn shadow_styles_scope_and_inherit_host_font_size() {
    let mut vm = new_storage_test_vm("https://shadow-style-scope-geometry.test/");

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
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const d = frame.contentWindow.document;

  d.head.innerHTML = '<style>.document-only-hidden { display: none; }</style>';
  const documentStyleHost = d.createElement('div');
  d.body.appendChild(documentStyleHost);
  const documentStyleRoot = documentStyleHost.attachShadow({ mode: 'open' });
  const shadowVisibleWrapper = d.createElement('div');
  shadowVisibleWrapper.innerHTML = '<span id="shadowVisible" class="document-only-hidden">shadow</span>';
  documentStyleRoot.appendChild(shadowVisibleWrapper);
  const shadowVisible = documentStyleRoot.querySelector('#shadowVisible');

  const shadowStyleHost = d.createElement('div');
  d.body.appendChild(shadowStyleHost);
  const documentSpan = d.createElement('span');
  documentSpan.id = 'documentSpan';
  documentSpan.className = 'shadow-only-hidden';
  d.body.appendChild(documentSpan);
  const shadowStyleRoot = shadowStyleHost.attachShadow({ mode: 'open' });
  const style = d.createElement('style');
  style.textContent = '.shadow-only-hidden { display: none; }';
  shadowStyleRoot.appendChild(style);
  const shadowHidden = d.createElement('span');
  shadowHidden.id = 'shadowHidden';
  shadowHidden.className = 'shadow-only-hidden';
  shadowStyleRoot.appendChild(shadowHidden);
  const classPrefixVisible = d.createElement('span');
  classPrefixVisible.id = 'classPrefixVisible';
  classPrefixVisible.className = 'shadow-only';
  shadowStyleRoot.appendChild(classPrefixVisible);

  const inheritHost = d.createElement('div');
  inheritHost.setAttribute('style', 'font-size:10px');
  inheritHost.innerHTML = '<span id="lightChild">light</span>';
  d.body.appendChild(inheritHost);
  const inheritRoot = inheritHost.attachShadow({ mode: 'open' });
  const inherited = d.createElement('span');
  inherited.id = 'inherited';
  inherited.textContent = 'shadow';
  inheritRoot.appendChild(inherited);
  const lightChild = d.querySelector('#lightChild');
  const lightChildRect = lightChild.getBoundingClientRect();
  const initialFontSize = getComputedStyle(inherited).fontSize;
  inheritHost.setAttribute('style', 'font-size:20px');

	  return JSON.stringify({
	    documentStyleDoesNotEnterShadow: getComputedStyle(shadowVisible).display !== 'none',
	    shadowStyleDoesNotLeaveShadow: getComputedStyle(documentSpan).display !== 'none',
	    shadowStyleAppliesInsideShadow: getComputedStyle(shadowHidden).display === 'none',
	    classSelectorDoesNotMatchPrefix: getComputedStyle(classPrefixVisible).display !== 'none',
	    lightChildWithoutSlotHasNoBox:
        lightChild.offsetTop === 0 && lightChildRect.width === 0 && lightChildRect.height === 0,
	    inheritedFontSizeUpdates: initialFontSize === '10px' && getComputedStyle(inherited).fontSize === '20px'
	  });
})()
"#,
        )
        .expect("shadow scoped style should evaluate");

    assert_eq!(
        result,
        r#"{"documentStyleDoesNotEnterShadow":true,"shadowStyleDoesNotLeaveShadow":true,"shadowStyleAppliesInsideShadow":true,"classSelectorDoesNotMatchPrefix":true,"lightChildWithoutSlotHasNoBox":true,"inheritedFontSizeUpdates":true}"#
    );
}

#[test]
fn child_document_shadow_visibility_offsets_match_rendered_wpt_probe() {
    let mut vm = new_storage_test_vm("https://shadow-style-offset-wpt-probe.test/");

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
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const d = frame.contentWindow.document;

  d.head.innerHTML = '<style>.document-only-hidden { display: none; }</style>';
  const documentStyleHost = d.createElement('div');
  d.body.appendChild(documentStyleHost);
  const documentStyleRoot = documentStyleHost.attachShadow({ mode: 'open' });
  const shadowVisibleWrapper = d.createElement('div');
  shadowVisibleWrapper.innerHTML =
    '<span id="shadowVisible" class="document-only-hidden">shadow</span>';
  documentStyleRoot.appendChild(shadowVisibleWrapper);
  const shadowVisible = documentStyleRoot.querySelector('#shadowVisible');

  const shadowStyleHost = d.createElement('div');
  d.body.appendChild(shadowStyleHost);
  const documentSpan = d.createElement('span');
  documentSpan.id = 'documentSpan';
  documentSpan.className = 'shadow-only-hidden';
  d.body.appendChild(documentSpan);
  const shadowStyleRoot = shadowStyleHost.attachShadow({ mode: 'open' });
  const style = d.createElement('style');
  style.textContent = '.shadow-only-hidden { display: none; }';
  shadowStyleRoot.appendChild(style);
  const shadowHidden = d.createElement('span');
  shadowHidden.id = 'shadowHidden';
  shadowHidden.className = 'shadow-only-hidden';
  shadowStyleRoot.appendChild(shadowHidden);

  return JSON.stringify({
    documentStyleDoesNotEnterShadow:
      frame.contentWindow.getComputedStyle(shadowVisible).display !== 'none',
    documentStyleShadowOffsetIsPositive: shadowVisible.offsetTop > 0,
    shadowStyleDoesNotLeaveDocument:
      frame.contentWindow.getComputedStyle(documentSpan).display !== 'none',
    documentTreeOffsetIsPositive: documentSpan.offsetTop > 0,
    shadowStyleAppliesInsideShadow:
      frame.contentWindow.getComputedStyle(shadowHidden).display === 'none',
    shadowHiddenClientRectIsEmpty:
      shadowHidden.getBoundingClientRect().width === 0 &&
      shadowHidden.getBoundingClientRect().height === 0
  });
})()
"#,
        )
        .expect("child document shadow style visibility offsets should evaluate");

    assert_eq!(
        result,
        r#"{"documentStyleDoesNotEnterShadow":true,"documentStyleShadowOffsetIsPositive":true,"shadowStyleDoesNotLeaveDocument":true,"documentTreeOffsetIsPositive":true,"shadowStyleAppliesInsideShadow":true,"shadowHiddenClientRectIsEmpty":true}"#
    );
}

#[test]
fn shadow_dir_pseudo_styles_slotted_nodes_from_document_direction() {
    let mut vm = new_storage_test_vm("https://shadow-dir-pseudo-style.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.head) {
    document.documentElement.appendChild(document.createElement('head'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  document.head.appendChild(document.createElement('style')).textContent =
    '.slotted { color: red; } .slotted:dir(rtl) { color: green; }';
  document.body.setAttribute('dir', 'rtl');

  const host = document.createElement('div');
  const slotted = document.createElement('div');
  slotted.className = 'slotted';
  host.appendChild(slotted);
  host.attachShadow({ mode: 'open' }).appendChild(document.createElement('slot'));
  document.body.appendChild(host);

  return getComputedStyle(slotted).color;
})()
"#,
        )
        .expect(":dir() pseudo-class should style slotted nodes");

    assert_eq!(result, "rgb(0, 128, 0)");
}

#[test]
fn computed_direction_tracks_input_html_directionality() {
    let mut vm = new_storage_test_vm("https://input-direction-computed-style.test/");

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

  const container = document.createElement('div');
  container.dir = 'rtl';
  const input = document.createElement('input');
  input.type = 'tel';
  container.appendChild(input);
  document.body.appendChild(container);

  const tel = `${input.matches(':dir(ltr)')}:${getComputedStyle(input).direction}`;
  input.type = 'text';
  const text = `${input.matches(':dir(rtl)')}:${getComputedStyle(input).direction}`;
  input.type = 'tel';
  const restoredTel = `${input.matches(':dir(ltr)')}:${getComputedStyle(input).direction}`;
  input.dir = 'auto';
  input.value = '\u05ea';
  const auto = `${input.matches(':dir(rtl)')}:${getComputedStyle(input).direction}`;
  input.style.direction = 'ltr';
  const inlineOverride = `${input.matches(':dir(rtl)')}:${getComputedStyle(input).direction}`;

  return `${tel}|${text}|${restoredTel}|${auto}|${inlineOverride}`;
})()
"#,
        )
        .expect("input direction computed style should evaluate");

    assert_eq!(result, "true:ltr|true:rtl|true:ltr|true:rtl|true:ltr");
}

#[test]
fn has_pseudo_class_computed_style_updates_after_dom_mutation() {
    let mut vm = new_storage_test_vm("https://has-pseudo-computed-style.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.head) {
    document.documentElement.appendChild(document.createElement('head'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  document.head.appendChild(document.createElement('style')).textContent =
    '.target { color: red; } .parent:has(.marker) .target { color: green; }';

  const parent = document.createElement('div');
  parent.className = 'parent';
  const target = document.createElement('span');
  target.className = 'target';
  parent.appendChild(target);
  document.body.appendChild(parent);

  const before = getComputedStyle(target).color;
  const marker = document.createElement('span');
  marker.className = 'marker';
  parent.appendChild(marker);
  const after = getComputedStyle(target).color;
  return `${before}|${after}`;
})()
"#,
        )
        .expect(":has() computed style mutation test should evaluate");

    assert_eq!(result, "rgb(255, 0, 0)|rgb(0, 128, 0)");
}
#[test]
fn empty_pseudo_class_computed_style_updates_after_child_list_mutation() {
    let mut vm = new_storage_test_vm("https://empty-pseudo-computed-style.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.head) {
    document.documentElement.appendChild(document.createElement('head'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  document.head.appendChild(document.createElement('style')).textContent =
    '.box { color: red; } .box:empty { color: green; }';

  const box = document.createElement('div');
  box.className = 'box';
  document.body.appendChild(box);
  const before = getComputedStyle(box).color;
  const child = document.createElement('span');
  box.appendChild(child);
  const afterInsert = getComputedStyle(box).color;
  child.remove();
  const afterRemove = getComputedStyle(box).color;
  return `${before}|${afterInsert}|${afterRemove}`;
})()
"#,
        )
        .expect(":empty computed style mutation test should evaluate");

    assert_eq!(result, "rgb(0, 128, 0)|rgb(255, 0, 0)|rgb(0, 128, 0)");
}
#[test]
fn computed_style_create_read_remove_loop_does_not_grow_caches() {
    let mut vm = new_storage_test_vm("https://computed-style-cache-loop.test/");
    let document = vm.document_handle_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '.target { color: rgb(20, 21, 22); }';
  head.appendChild(style);

  let last = '';
  for (let i = 0; i < 40; i += 1) {
    const target = document.createElement('div');
    target.className = 'target';
    body.appendChild(target);
    last = getComputedStyle(target).color;
    target.remove();
  }
  return last;
})()
"#,
        )
        .expect("computed style create/read/remove loop should evaluate");

    assert_eq!(result, "rgb(20, 21, 22)");
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
}
#[test]
fn tab_key_respects_reading_flow_item_order() {
    let mut vm = new_streamed_parser_test_vm(
        "https://shadow-reading-flow.test/",
        r#"<!doctype html>
<style>
.source {
  display: block;
  reading-flow: source-order;
}
.grid {
  display: grid;
  reading-flow: grid-order;
}
</style>
<div class="source">
  <button id="a" style="reading-order: 1">A</button>
  <button id="b" style="reading-order: -1">B</button>
  <button id="c">C</button>
</div>
<div class="grid">
  <button id="gA" style="order: -1">Grid A</button>
  <button id="gB">Grid B</button>
  <button id="gC" tabindex="1" style="order: -1">Grid C</button>
</div>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  function pressTab() {
    __moliDispatchTrustedKey('keydown', 'Tab', 'Tab', false, false, false, false);
  }
  const seen = [];
  for (let i = 0; i < 6; i++) {
    pressTab();
    seen.push(document.activeElement && document.activeElement.id);
  }
  return seen.join(',');
})()
"#,
        )
        .expect("Tab default action should respect reading-flow item order");

    assert_eq!(result, "b,c,a,gA,gC,gB");
}
#[test]
fn tab_key_respects_reading_flow_display_contents_items() {
    let mut vm = new_streamed_parser_test_vm(
        "https://shadow-reading-flow-display-contents.test/",
        r#"<!doctype html>
<style>
.wrapper {
  display: grid;
  reading-flow: grid-order;
}
</style>
<div class="wrapper">
  <div style="display: contents">
    <button id="order3" style="order: 3">Order 3</button>
    <button id="order1" style="order: 1">Order 1</button>
    <div style="display: contents">
      <button id="order4" style="order: 4">Order 4</button>
      <button id="order2" style="order: 2">Order 2</button>
    </div>
  </div>
</div>
<div class="wrapper">
  <div id="div1B" style="display: contents" tabindex="0">
    <button id="order3B" style="order: 3">Order 3</button>
    <button id="order1B" style="order: 1">Order 1</button>
    <div id="div2B" style="display: contents" tabindex="0">
      <button id="order4B" style="order: 4">Order 4</button>
      <button id="order2B" style="order: 2">Order 2</button>
    </div>
  </div>
</div>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  function pressTab() {
    __moliDispatchTrustedKey('keydown', 'Tab', 'Tab', false, false, false, false);
  }
  const seen = [];
  for (let i = 0; i < 10; i++) {
    pressTab();
    seen.push(document.activeElement && document.activeElement.id);
  }
  return seen.join(',');
})()
"#,
        )
        .expect(
            "Tab default action should respect independent display:contents reading-flow items",
        );

    assert_eq!(
        result,
        "order1,order2,order4,order3,div1B,order1B,div2B,order2B,order4B,order3B"
    );
}
#[tokio::test]
async fn css_transition_state_uses_final_values_without_runtime_events() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_test_vm(
        "https://move-before-transition-state.test/",
        r#"<html><head></head><body></body></html>"#,
    );

    vm.eval(
        r#"
(() => {
  document.head.appendChild(document.createElement('style')).textContent = `
    body { margin-left: 0; }
    section { position: relative; }
    #item, #trigger, #pseudo {
      width: 100px;
      height: 100px;
      position: absolute;
      left: 0;
      transition: left 10s steps(1, jump-both);
    }
    #new-parent #trigger { left: 400px; }
    #pseudo::before {
      content: "x";
      position: absolute;
      left: 0;
      transition: left 10s steps(1, jump-both);
    }
    #pseudo.big::before { left: 400px; }
  `;
  document.body.innerHTML = `
    <section id="old-parent">
      <div id="item"></div>
      <div id="trigger"></div>
      <div id="pseudo"></div>
    </section>
    <section id="new-parent"></section>`;
  globalThis.__lmTransitionEvents = 0;
  for (const node of document.querySelectorAll('div')) {
    node.addEventListener('transitionstart', () => { globalThis.__lmTransitionEvents += 1; });
  }
  document.getElementById('item').style.left = '400px';
  document.getElementById('pseudo').classList.add('big');
  document.getElementById('new-parent').moveBefore(document.getElementById('trigger'), null);
})()
"#,
    )
    .expect("transition setup should evaluate");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("transition timers should drain");

    let result = vm
        .eval(
            r#"
(() => {
  const item = document.getElementById('item');
  const trigger = document.getElementById('trigger');
  const pseudo = document.getElementById('pseudo');
  const itemStyle = getComputedStyle(item);
  return [
    globalThis.__lmTransitionEvents,
    item.getBoundingClientRect().x,
    item.getAnimations().length,
    trigger.getBoundingClientRect().x,
    itemStyle.left,
    itemStyle.transitionProperty,
    itemStyle.transitionDuration,
    itemStyle.transitionDelay,
    itemStyle.transitionTimingFunction,
    itemStyle.transitionBehavior,
    getComputedStyle(pseudo, '::before').left
  ].join('|');
})()
"#,
        )
        .expect("transition final-state should be readable");

    assert_eq!(
        result,
        "0|400|0|400|400px|left|10s|0s|steps(1, jump-both)|normal|400px"
    );
}
#[tokio::test]
async fn invalid_move_before_does_not_run_plain_transition_runtime() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_test_vm(
        "https://move-before-invalid-transition.test/",
        r#"<html><head></head><body></body></html>"#,
    );

    vm.eval(
        r#"
(() => {
  document.head.appendChild(document.createElement('style')).textContent = `
    body { margin-left: 0; }
    #item {
      width: 100px;
      height: 100px;
      position: absolute;
      left: 0;
      transition: left 10s;
    }
  `;
  document.body.innerHTML = `<div id="item"></div>`;
  const item = document.getElementById('item');
  globalThis.__lmPlainTransitionEvents = 0;
  item.addEventListener('transitionstart', () => { globalThis.__lmPlainTransitionEvents += 1; });
  item.style.left = '400px';
})()
"#,
    )
    .expect("plain transition setup should evaluate");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("plain transition timers should drain");

    let result = vm
        .eval(
            r#"
(() => {
  const item = document.getElementById('item');
  const doc = document.implementation.createHTMLDocument();
  let error = '';
  try {
    doc.body.moveBefore(item, null);
  } catch (err) {
    error = err.name;
  }
  return [
    globalThis.__lmPlainTransitionEvents,
    error,
    item.getBoundingClientRect().x
  ].join('|');
})()
"#,
        )
        .expect("invalid moveBefore transition state should evaluate");

    assert_eq!(result, "0|HierarchyRequestError|400");
}
#[tokio::test]
async fn zero_duration_transform_transition_applies_final_layout_geometry() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_test_vm(
        "https://zero-duration-transform-transition.test/",
        r#"<html><head></head><body></body></html>"#,
    );

    vm.eval(
        r#"
(() => {
  document.head.appendChild(document.createElement('style')).textContent = `
    body { margin-left: 0; }
    #item {
      width: 100px;
      height: 100px;
      position: absolute;
      left: 0;
      transition: transform 0s linear 1s;
    }
  `;
  document.body.innerHTML = `<div id="item"></div>`;
  const item = document.getElementById('item');
  globalThis.__lmZeroDurationTransformEvents = 0;
  item.addEventListener('transitionstart', () => {
    globalThis.__lmZeroDurationTransformEvents += 1;
  });
  item.style.transform = 'translateX(400px)';
})()
"#,
    )
    .expect("zero-duration transform transition setup should evaluate");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("zero-duration transform transition timers should drain");

    let result = vm
        .eval(
            r#"
(() => {
  const item = document.getElementById('item');
  return [
    globalThis.__lmZeroDurationTransformEvents,
    item.getBoundingClientRect().x
  ].join('|');
})()
"#,
        )
        .expect("zero-duration transform geometry should evaluate");

    assert_eq!(result, "0|400");
}
#[tokio::test]
async fn child_content_document_created_elements_use_transition_final_state() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm("https://child-created-transition.test/");

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child document frame setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    vm.eval(
        r#"
(() => {
  const frame = document.querySelector('iframe');
  const doc = frame.contentDocument;
  doc.head.appendChild(doc.createElement('style')).textContent = `
    body { margin-left: 0; }
    #item {
      position: absolute;
      left: 0;
      transition: left 10s;
    }
  `;
  const item = doc.createElement('div');
  item.id = 'item';
  doc.body.append(item);
  globalThis.__lmChildTransitionEvents = 0;
  item.addEventListener('transitionstart', () => { globalThis.__lmChildTransitionEvents += 1; });
  item.style.left = '400px';
})()
"#,
    )
    .expect("child document transition setup should evaluate");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("child document transition timers should drain");

    let result = vm
        .eval(
            r#"
(() => {
  const item = document.querySelector('iframe').contentDocument.getElementById('item');
  return [
    globalThis.__lmChildTransitionEvents,
    item.getBoundingClientRect().x
  ].join('|');
})()
"#,
        )
        .expect("child document transition state should evaluate");

    assert_eq!(result, "0|400");
}

#[tokio::test]
async fn child_content_document_animation_start_scans_child_stylesheets() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://child-animation-source-scope.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child document frame setup should evaluate");
    while vm
        .run_one_oldest_ready_page_task_executor_turn(&loader)
        .await
        .expect("child frame setup task should run")
    {}

    vm.eval(
        r#"
(() => {
  const frame = document.querySelector('iframe');
  const doc = frame.contentDocument;
  doc.head.appendChild(doc.createElement('style')).textContent = `
    @keyframes childAnim {
      from { left: 100px; }
      to { left: 400px; }
    }
    #item {
      position: relative;
      animation: 1s linear infinite alternate childAnim;
      animation-delay: 100ms;
    }
  `;
  const item = doc.createElement('div');
  item.id = 'item';
  doc.body.append(item);
  globalThis.__lmChildAnimationEvents = 0;
  item.addEventListener('animationstart', () => { globalThis.__lmChildAnimationEvents += 1; });
})()
"#,
    )
    .expect("child document animation setup should evaluate");

    assert!(
        !vm.has_ready_timeout(),
        "child animationstart must not manufacture a PageTimer"
    );
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .expect("child animation rendering update should run")
    );

    let result = vm
        .eval(
            r#"
(() => {
  const item = document.querySelector('iframe').contentDocument.getElementById('item');
  return [
    globalThis.__lmChildAnimationEvents,
    getComputedStyle(item).left,
    item.getAnimations().length
  ].join('|');
})()
"#,
        )
        .expect("child document animation state should evaluate");

    assert_eq!(result, "1|250px|1");
}

#[tokio::test]
async fn css_animation_start_and_midpoint_style_are_observable() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-animation-midpoint.test/",
        r#"<html><head></head><body></body></html>"#,
        &loader,
    );

    vm.eval(
        r#"
(() => {
  document.head.appendChild(document.createElement('style')).textContent = `
    @keyframes anim {
      from { left: 100px; }
      to { left: 400px; }
    }
    #item {
      position: relative;
      animation: 1s linear infinite alternate anim;
      animation-delay: 100ms;
    }
  `;
  document.body.innerHTML = `<div id="item"></div>`;
  globalThis.__lmAnimationEvents = 0;
  addEventListener('animationstart', () => { globalThis.__lmAnimationEvents += 1; });
})()
"#,
    )
    .expect("animation setup should evaluate");

    assert!(
        !vm.has_ready_timeout(),
        "animationstart must not manufacture a PageTimer"
    );
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .expect("animation rendering update should run")
    );

    let result = vm
        .eval(
            r#"
(() => {
  const item = document.getElementById('item');
  return [
    globalThis.__lmAnimationEvents,
    getComputedStyle(item).left,
    item.getAnimations().length
  ].join('|');
})()
"#,
        )
        .expect("animation midpoint state should be readable");

    assert_eq!(result, "1|250px|1");
}

#[tokio::test]
async fn registered_length_custom_property_animation_revert_uses_underlying_value() {
    let mut vm = new_parsed_test_vm(
        "https://registered-custom-animation-revert.test/",
        r#"<html><head></head><body></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  CSS.registerProperty({
    name: "--animated-non-inherited",
    syntax: "<length>",
    initialValue: "0px",
    inherits: false
  });
  CSS.registerProperty({
    name: "--animated-inherited",
    syntax: "<length>",
    initialValue: "10000px",
    inherits: true
  });
  const style = document.createElement("style");
  style.textContent = `
    @keyframes revert_animation {
      from {
        --animated-inherited: revert;
        --animated-non-inherited: revert;
      }
      to {
        --animated-inherited: 100px;
        --animated-non-inherited: 100px;
      }
    }
    #parent {
      --animated-inherited: 0px;
    }
    #child {
      animation: revert_animation 10s -5s linear paused;
    }
  `;
  document.head.append(style);
  document.body.innerHTML = `<div id="parent"><div id="child"></div></div>`;
  const computed = getComputedStyle(document.getElementById("child"));
  return [
    computed.getPropertyValue("--animated-non-inherited"),
    computed.getPropertyValue("--animated-inherited")
  ].join("|");
})()
"#,
        )
        .expect("registered custom property animation revert should evaluate");

    assert_eq!(result, "50px|50px");
}

#[tokio::test]
async fn css_animation_start_and_zero_timeout_use_distinct_task_sources() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-animation-window-order.test/",
        r#"<html><head></head><body></body></html>"#,
        &loader,
    );

    vm.eval(
        r#"
(() => {
  document.head.appendChild(document.createElement('style')).textContent = `
    @keyframes anim {
      from { left: 100px; }
      to { left: 400px; }
    }
    #item {
      position: relative;
      animation: 1s linear infinite alternate anim;
    }
  `;
  document.body.innerHTML = `<div id="item"></div>`;
  globalThis.__lmAnimationOrder = [];
  addEventListener('animationstart', () => { globalThis.__lmAnimationOrder.push('animation'); });
  setTimeout(() => { globalThis.__lmAnimationOrder.push('timeout'); }, 0);
})()
"#,
    )
    .expect("animation ordering setup should evaluate");

    assert!(
        vm.has_ready_timeout(),
        "the explicit setTimeout must remain a real PageTimer"
    );
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .expect("animation rendering update should run")
    );
    assert_eq!(
        vm.eval("globalThis.__lmAnimationOrder.join(',')")
            .expect("pre-timer animation order should evaluate"),
        "animation"
    );

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("the genuine zero timeout should drain");

    let result = vm
        .eval("globalThis.__lmAnimationOrder.join(',')")
        .expect("animation order should evaluate");

    assert_eq!(result, "animation,timeout");
}
#[tokio::test]
async fn css_animation_commit_styles_commits_midpoint_transform() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-animation-commit.test/",
        r#"<html><head></head><body></body></html>"#,
        &loader,
    );

    vm.eval(
        r#"
(() => {
  document.head.appendChild(document.createElement('style')).textContent = `
    @keyframes anim {
      from { transform: translateX(100px); }
      to { transform: translateX(400px); }
    }
    #item {
      animation: 1s linear infinite alternate anim;
      animation-delay: 100ms;
    }
  `;
  document.body.innerHTML = `<div id="item"></div>`;
  globalThis.__lmAnimationEvents = 0;
  document.getElementById('item').addEventListener('animationstart', () => {
    globalThis.__lmAnimationEvents += 1;
  });
})()
"#,
    )
    .expect("animation commit setup should evaluate");

    assert!(!vm.has_ready_timeout());
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .expect("animation rendering update should run")
    );

    let result = vm
        .eval(
            r#"
(() => {
  const item = document.getElementById('item');
  const animations = item.getAnimations();
  animations[0].commitStyles();
  return [
    globalThis.__lmAnimationEvents,
    animations.length,
    'transform' in item.style,
    item.style.transform
  ].join('|');
})()
"#,
        )
        .expect("animation committed style should be readable");

    assert_eq!(result, "1|1|true|translateX(250px)");
}
#[tokio::test]
async fn css_animation_start_capture_and_bubble_listeners_share_queued_event() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-animation-listeners.test/",
        r#"<html><head></head><body></body></html>"#,
        &loader,
    );

    vm.eval(
        r#"
(() => {
  document.head.appendChild(document.createElement('style')).textContent = `
    @keyframes anim {
      from { left: 100px; }
      to { left: 400px; }
    }
    #item {
      position: relative;
      animation: 1s linear infinite alternate anim;
    }
  `;
  document.body.innerHTML = `<div id="item"></div>`;
  const item = document.getElementById('item');
  globalThis.__lmAnimationEvents = 0;
  item.addEventListener('animationstart', () => { globalThis.__lmAnimationEvents += 1; });
  item.addEventListener('animationstart', () => { globalThis.__lmAnimationEvents += 1; }, true);
})()
"#,
    )
    .expect("animation listener setup should evaluate");

    assert!(!vm.has_ready_timeout());
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .expect("animation rendering update should run")
    );

    let result = vm
        .eval("globalThis.__lmAnimationEvents")
        .expect("animation event count should evaluate");

    assert_eq!(result, "2");
}
#[tokio::test]
async fn css_animation_start_listener_removed_before_queued_event_still_deduplicates() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-animation-listener-remove.test/",
        r#"<html><head></head><body></body></html>"#,
        &loader,
    );

    vm.eval(
        r#"
(() => {
  document.head.appendChild(document.createElement('style')).textContent = `
    @keyframes anim {
      from { left: 100px; }
      to { left: 400px; }
    }
    #item {
      position: relative;
      animation: 1s linear infinite alternate anim;
    }
  `;
  document.body.innerHTML = `<div id="item"></div>`;
  const item = document.getElementById('item');
  globalThis.__lmAnimationEvents = 0;
  function first() { globalThis.__lmAnimationEvents += 100; }
  function second() { globalThis.__lmAnimationEvents += 1; }
  item.addEventListener('animationstart', first);
  item.addEventListener('animationstart', second, true);
  item.removeEventListener('animationstart', first);
})()
"#,
    )
    .expect("animation listener setup should evaluate");

    assert!(!vm.has_ready_timeout());
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .expect("animation rendering update should run")
    );

    let result = vm
        .eval("globalThis.__lmAnimationEvents")
        .expect("animation event count should evaluate");

    assert_eq!(result, "1");
}
#[tokio::test]
async fn css_animation_start_later_listener_gets_own_retroactive_scan() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-animation-later-listener.test/",
        r#"<html><head></head><body></body></html>"#,
        &loader,
    );

    vm.eval(
        r#"
(() => {
  document.head.appendChild(document.createElement('style')).textContent = `
    @keyframes anim {
      from { left: 100px; }
      to { left: 400px; }
    }
    #item {
      position: relative;
      animation: 1s linear infinite alternate anim;
    }
  `;
  document.body.innerHTML = `<div id="item"></div>`;
  const item = document.getElementById('item');
  globalThis.__lmAnimationFirstEvents = 0;
  globalThis.__lmAnimationSecondEvents = 0;
  item.addEventListener('animationstart', () => { globalThis.__lmAnimationFirstEvents += 1; });
})()
"#,
    )
    .expect("animation listener setup should evaluate");

    assert!(!vm.has_ready_timeout());
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .expect("initial animation rendering update should run")
    );
    assert_eq!(
        vm.eval(
            "[globalThis.__lmAnimationFirstEvents, globalThis.__lmAnimationSecondEvents].join('|')"
        )
        .expect("initial animation event count should evaluate"),
        "1|0"
    );

    vm.eval(
        r#"
document.getElementById('item').addEventListener('animationstart', () => {
  globalThis.__lmAnimationSecondEvents += 1;
});
"#,
    )
    .expect("later animation listener should evaluate");

    assert!(!vm.has_ready_timeout());
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .expect("later animation rendering update should run")
    );

    let result = vm
        .eval("globalThis.__lmAnimationSecondEvents")
        .expect("later animation event count should evaluate");

    assert_eq!(result, "1");
}
#[test]
fn computed_style_uses_static_color_animation_value_for_inserted_node() {
    let mut vm = new_parsed_test_vm(
        "https://css-animation-static-color.test/",
        r#"<html><head></head><body></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  document.head.appendChild(document.createElement('style')).textContent = `
    @keyframes my-animation {
      from { color: green; }
      to { color: green; }
    }
    div {
      color: red;
      animation: my-animation 1s infinite linear paused;
    }
  `;
  const span = document.body.appendChild(document.createElement('span'));
  const oldDiv = span.appendChild(document.createElement('div'));
  const before = getComputedStyle(oldDiv).color;
  const newDiv = document.createElement('div');
  oldDiv.replaceWith(newDiv);
  return [before, getComputedStyle(newDiv).color].join('|');
})()
"#,
        )
        .expect("static color animation computed style should evaluate");

    assert_eq!(result, "rgb(0, 128, 0)|rgb(0, 128, 0)");
}

#[test]
fn computed_style_serializes_animation_shorthand_from_longhands() {
    let mut vm = new_parsed_test_vm(
        "https://css-animation-computed-shorthand.test/",
        r#"<html><head></head><body><div id="target"></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.getElementById('target');
  const values = [];

  values.push(getComputedStyle(target).animation);

  target.style.animation = 'anim paused both reverse 4 1s -3s cubic-bezier(0, -2, 1, 3)';
  values.push(getComputedStyle(target).animation);

  target.style.animation = 'anim paused both reverse, 4 1s -3s cubic-bezier(0, -2, 1, 3)';
  values.push(getComputedStyle(target).animation);

  target.style.animation = 'initial';
  target.style.animationDelay = '1s';
  values.push(getComputedStyle(target).animation);

  return values.join('|');
})()
"#,
        )
        .expect("computed animation shorthand should evaluate");

    assert_eq!(
        result,
        "none|1s cubic-bezier(0, -2, 1, 3) -3s 4 reverse both paused anim|reverse both paused anim, 1s cubic-bezier(0, -2, 1, 3) -3s 4|0s 1s"
    );
}

#[test]
fn computed_style_serializes_compositing_longhands() {
    let mut vm = new_parsed_test_vm(
        "https://css-compositing-computed-longhands.test/",
        r#"<html><head><style>#styled { background-blend-mode: screen, overlay; mix-blend-mode: multiply; isolation: isolate; }</style></head><body><div id="initial"></div><div id="inline" style="background-blend-mode: normal, luminosity; mix-blend-mode: color; isolation: auto"></div><div id="styled"></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const initial = getComputedStyle(document.getElementById('initial'));
  const inline = getComputedStyle(document.getElementById('inline'));
  const styled = getComputedStyle(document.getElementById('styled'));
  return [
    initial.backgroundBlendMode,
    initial.mixBlendMode,
    initial.isolation,
    inline.backgroundBlendMode,
    inline.mixBlendMode,
    inline.isolation,
    styled.backgroundBlendMode,
    styled.mixBlendMode,
    styled.isolation
  ].join('|');
})()
"#,
        )
        .expect("compositing computed longhands should evaluate");

    assert_eq!(
        result,
        "normal|normal|auto|normal, luminosity|color|auto|screen, overlay|multiply|isolate"
    );
}

#[test]
fn computed_style_serializes_color_adjust_longhands() {
    let mut vm = new_parsed_test_vm(
        "https://css-color-adjust-computed-longhands.test/",
        r#"<html><head></head><body><div id="container"><div id="target"></div></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const container = document.getElementById('container');
  const target = document.getElementById('target');
  const computed = getComputedStyle(target);
  const values = [];

  values.push('color-scheme' in computed);
  values.push('color-adjust' in computed);
  values.push('forced-color-adjust' in computed);
  values.push(computed.getPropertyValue('color-scheme'));
  values.push(computed.getPropertyValue('color-adjust'));
  values.push(computed.getPropertyValue('forced-color-adjust'));

  target.style.colorScheme = 'only light';
  target.style.colorAdjust = 'exact';
  target.style.forcedColorAdjust = 'preserve-parent-color';
  values.push(computed.getPropertyValue('color-scheme'));
  values.push(computed.getPropertyValue('print-color-adjust'));
  values.push(computed.getPropertyValue('color-adjust'));
  values.push(computed.getPropertyValue('forced-color-adjust'));

  container.style.colorScheme = 'light dark';
  container.style.colorAdjust = 'economy';
  container.style.forcedColorAdjust = 'none';
  target.style.colorScheme = 'unset';
  target.style.colorAdjust = 'unset';
  target.style.forcedColorAdjust = 'unset';
  values.push(computed.getPropertyValue('color-scheme'));
  values.push(computed.getPropertyValue('color-adjust'));
  values.push(computed.getPropertyValue('forced-color-adjust'));

  target.style.colorScheme = 'initial';
  target.style.colorAdjust = 'initial';
  target.style.forcedColorAdjust = 'initial';
  values.push(computed.getPropertyValue('color-scheme'));
  values.push(computed.getPropertyValue('color-adjust'));
  values.push(computed.getPropertyValue('forced-color-adjust'));

  target.style.colorScheme = 'inherit';
  target.style.colorAdjust = 'inherit';
  target.style.forcedColorAdjust = 'inherit';
  values.push(computed.getPropertyValue('color-scheme'));
  values.push(computed.getPropertyValue('color-adjust'));
  values.push(computed.getPropertyValue('forced-color-adjust'));

  return values.join('|');
})()
"#,
        )
        .expect("color-adjust computed longhands should evaluate");

    assert_eq!(
        result,
        "true|true|true|normal|economy|auto|light only|exact|exact|preserve-parent-color|light dark|economy|none|normal|economy|auto|light dark|economy|none"
    );
}

#[test]
fn computed_style_serializes_scrollbar_longhands() {
    let mut vm = new_parsed_test_vm(
        "https://css-scrollbar-computed-longhands.test/",
        r#"<html><head></head><body><div id="container"><div id="target"></div></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const container = document.getElementById('container');
  const target = document.getElementById('target');
  const computed = getComputedStyle(target);
  const values = [];

  values.push('scrollbar-color' in computed);
  values.push('scrollbar-width' in computed);
  values.push(computed.getPropertyValue('scrollbar-color'));
  values.push(computed.getPropertyValue('scrollbar-width'));

  target.style.scrollbarColor = 'red green';
  target.style.scrollbarWidth = 'thin';
  values.push(computed.getPropertyValue('scrollbar-color'));
  values.push(computed.getPropertyValue('scrollbar-width'));

  container.style.scrollbarColor = 'rgb(1, 2, 3) rgb(4, 5, 6)';
  container.style.scrollbarWidth = 'none';
  target.style.scrollbarColor = 'unset';
  target.style.scrollbarWidth = 'unset';
  values.push(computed.getPropertyValue('scrollbar-color'));
  values.push(computed.getPropertyValue('scrollbar-width'));

  target.style.scrollbarColor = 'initial';
  target.style.scrollbarWidth = 'initial';
  values.push(computed.getPropertyValue('scrollbar-color'));
  values.push(computed.getPropertyValue('scrollbar-width'));

  target.style.scrollbarColor = 'inherit';
  target.style.scrollbarWidth = 'inherit';
  values.push(computed.getPropertyValue('scrollbar-color'));
  values.push(computed.getPropertyValue('scrollbar-width'));

  return values.join('|');
})()
"#,
        )
        .expect("scrollbar computed longhands should evaluate");

    assert_eq!(
        result,
        "true|true|auto|auto|rgb(255, 0, 0) rgb(0, 128, 0)|thin|rgb(1, 2, 3) rgb(4, 5, 6)|auto|auto|auto|rgb(1, 2, 3) rgb(4, 5, 6)|none"
    );
}

#[test]
fn computed_style_serializes_text_size_adjust() {
    let mut vm = new_parsed_test_vm(
        "https://css-text-size-adjust-computed.test/",
        r#"<html><head></head><body><div id="container"><div id="target"></div></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const container = document.getElementById('container');
  const target = document.getElementById('target');
  const computed = getComputedStyle(target);
  const values = [];

  values.push('text-size-adjust' in computed);
  values.push(computed.getPropertyValue('text-size-adjust'));

  target.style.textSizeAdjust = 'none';
  values.push(computed.getPropertyValue('text-size-adjust'));

  target.style.textSizeAdjust = '200%';
  values.push(computed.getPropertyValue('text-size-adjust'));

  target.style.textSizeAdjust = 'calc(10% * sibling-index())';
  values.push(computed.getPropertyValue('text-size-adjust'));

  container.style.textSizeAdjust = '10%';
  target.style.textSizeAdjust = 'unset';
  values.push(computed.getPropertyValue('text-size-adjust'));

  target.style.textSizeAdjust = 'initial';
  values.push(computed.getPropertyValue('text-size-adjust'));

  target.style.textSizeAdjust = 'inherit';
  values.push(computed.getPropertyValue('text-size-adjust'));

  return values.join('|');
})()
"#,
        )
        .expect("text-size-adjust computed longhand should evaluate");

    // Tree-counting belongs to Stylo's lazy cascade context. The renderer's
    // post-cascade numeric adapter deliberately does not synthesize sibling
    // state, because doing so would scan siblings for every numeric computed
    // style read. Keep this unresolved fallback as the compatibility baseline.
    assert_eq!(
        result,
        "true|auto|100%|200%|calc(10% * sibling-index())|10%|auto|10%"
    );
}

#[test]
fn computed_style_serializes_link_parameters() {
    let mut vm = new_parsed_test_vm(
        "https://css-link-parameters-computed.test/",
        r#"<html><head></head><body><div id="container"><div id="target"></div></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const container = document.getElementById('container');
  const target = document.getElementById('target');
  const computed = getComputedStyle(target);
  const values = [];

  values.push('link-parameters' in computed);
  values.push(computed.getPropertyValue('link-parameters'));

  target.style.linkParameters = 'param(--a, orange)';
  values.push(computed.getPropertyValue('link-parameters'));

  target.style.linkParameters = 'param(--a, ), param(--b)';
  values.push(computed.getPropertyValue('link-parameters'));

  container.style.linkParameters = 'param(--parent)';
  target.style.linkParameters = 'unset';
  values.push(computed.getPropertyValue('link-parameters'));

  target.style.linkParameters = 'inherit';
  values.push(computed.getPropertyValue('link-parameters'));

  target.style.linkParameters = 'initial';
  values.push(computed.getPropertyValue('link-parameters'));

  return values.join('|');
})()
"#,
        )
        .expect("link-parameters computed longhand should evaluate");

    assert_eq!(
        result,
        "true|none|param(--a, orange)|param(--a, ), param(--b)|none|param(--parent)|none"
    );
}

#[test]
fn computed_style_serializes_content_quotes_and_bookmarks() {
    let mut vm = new_parsed_test_vm(
        "https://css-content-computed.test/",
        r#"<html><head></head><body><div id="container"><div id="target"></div></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const container = document.getElementById('container');
  const target = document.getElementById('target');
  const computed = getComputedStyle(target);
  const before = getComputedStyle(target, '::before');
  const values = [];

  values.push('content' in computed);
  values.push('quotes' in computed);
  values.push('bookmark-level' in computed);
  values.push('bookmark-state' in computed);
  values.push(computed.getPropertyValue('content'));
  values.push(before.getPropertyValue('content'));

  target.style.content = 'counter(counter-name, DECIMAL) / "alt text"';
  values.push(computed.getPropertyValue('content'));

  container.style.quotes = 'none';
  target.style.quotes = 'unset';
  values.push(computed.getPropertyValue('quotes'));

  target.style.quotes = 'initial';
  values.push(computed.getPropertyValue('quotes'));

  container.style.bookmarkLevel = '1';
  target.style.bookmarkLevel = 'unset';
  values.push(computed.getPropertyValue('bookmark-level'));

  target.style.bookmarkLevel = 'inherit';
  values.push(computed.getPropertyValue('bookmark-level'));

  target.style.bookmarkState = 'closed';
  values.push(computed.getPropertyValue('bookmark-state'));

  target.style.bookmarkState = 'initial';
  values.push(computed.getPropertyValue('bookmark-state'));

  return values.join('|');
})()
"#,
        )
        .expect("content computed longhands should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|normal|none|counter(counter-name) / \"alt text\"|none|auto|none|1|closed|open"
    );
}

#[test]
fn computed_style_serializes_will_change() {
    let mut vm = new_parsed_test_vm(
        "https://css-will-change-computed.test/",
        r#"<html><head></head><body><div id="container"><div id="target" style="will-change: inherit"></div></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const container = document.getElementById('container');
  const target = document.getElementById('target');
  const computed = getComputedStyle(target);
  const values = [];

  values.push('will-change' in computed);
  values.push('willChange' in computed);
  values.push(computed.getPropertyValue('will-change'));

  container.style.willChange = 'color';
  values.push(getComputedStyle(container).willChange);
  values.push(computed.getPropertyValue('will-change'));

  target.style.willChange = 'initial';
  values.push(computed.getPropertyValue('will-change'));

  target.style.willChange = 'scroll-position, TRANSFORM';
  values.push(computed.getPropertyValue('will-change'));

  target.style.willChange = 'auto, transform';
  values.push(computed.getPropertyValue('will-change'));

  return values.join('|');
})()
"#,
        )
        .expect("will-change computed longhand should evaluate");

    assert_eq!(
        result,
        "true|true|auto|color|color|auto|scroll-position, TRANSFORM|scroll-position, TRANSFORM"
    );
}

#[test]
fn computed_style_serializes_text_decoration_longhands() {
    let mut vm = new_parsed_test_vm(
        "https://css-text-decoration-computed.test/",
        r#"<html><body><div id="target"></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.getElementById('target');
  const computed = getComputedStyle(target);
  const values = [];

  values.push('text-decoration-line' in computed);
  values.push('textDecorationLine' in computed);
  values.push('text-decoration-inset' in computed);
  values.push('textDecorationInset' in computed);
  values.push('text-decoration-skip-ink' in computed);
  values.push('textDecorationSkipInk' in computed);
  values.push('text-decoration-skip-spaces' in computed);
  values.push('textDecorationSkipSpaces' in computed);
  values.push('text-decoration-style' in computed);
  values.push('textDecorationStyle' in computed);
  values.push('text-decoration-thickness' in computed);
  values.push('textDecorationThickness' in computed);
  values.push('text-underline-offset' in computed);
  values.push('textUnderlineOffset' in computed);
  values.push('text-underline-position' in computed);
  values.push('textUnderlinePosition' in computed);

  target.style.color = 'blue';
  target.style.fontSize = '20px';

  target.style.textDecoration = 'underline red from-font';
  values.push(computed.getPropertyValue('text-decoration'));

  target.style.textDecoration = 'rgba(10, 20, 30, 0.4) dotted';
  values.push(computed.getPropertyValue('text-decoration'));

  target.style.textDecoration = 'currentcolor';
  values.push(computed.getPropertyValue('text-decoration'));

  target.style.textDecoration = 'from-font';
  values.push(computed.getPropertyValue('text-decoration'));

  const sheetTarget = document.createElement('div');
  sheetTarget.id = 'sheet-target';
  document.body.appendChild(sheetTarget);
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('#sheet-target { color: blue; text-decoration: underline red from-font; }');
  document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
  const sheetComputed = getComputedStyle(sheetTarget);
  values.push(sheetComputed.getPropertyValue('text-decoration'));
  sheet.replaceSync('#sheet-target { color: blue; text-decoration: underline currentcolor; }');
  values.push(sheetComputed.getPropertyValue('text-decoration'));

  target.style.textDecorationLine = 'overline underline';
  values.push(computed.getPropertyValue('text-decoration-line'));
  values.push(computed.textDecorationLine);

  target.style.textDecorationInset = '0.5em';
  values.push(computed.getPropertyValue('text-decoration-inset'));

  target.style.textDecorationSkipInk = 'ALL';
  values.push(computed.getPropertyValue('text-decoration-skip-ink'));
  values.push(computed.textDecorationSkipInk);

  target.style.textDecorationSkipSpaces = 'end start';
  values.push(computed.getPropertyValue('text-decoration-skip-spaces'));

  target.style.textDecorationStyle = 'WAVY';
  values.push(computed.getPropertyValue('text-decoration-style'));
  values.push(computed.textDecorationStyle);

  target.style.textDecorationThickness = '2em';
  values.push(computed.getPropertyValue('text-decoration-thickness'));

  target.style.textUnderlineOffset = '2em';
  values.push(computed.getPropertyValue('text-underline-offset'));

  target.style.textUnderlinePosition = 'right under';
  values.push(computed.getPropertyValue('text-underline-position'));
  values.push(computed.textUnderlinePosition);

  target.style.textDecorationLine = 'Spelling-Error';
  values.push(computed.getPropertyValue('text-decoration-line'));

  target.style.textDecorationLine = 'underline underline';
  target.style.textDecorationSkipInk = 'auto none';
  target.style.textDecorationStyle = 'solid wavy';
  target.style.textUnderlinePosition = 'left right';
  values.push(computed.getPropertyValue('text-decoration-line'));
  values.push(computed.getPropertyValue('text-decoration-skip-ink'));
  values.push(computed.getPropertyValue('text-decoration-style'));
  values.push(computed.getPropertyValue('text-underline-position'));

  return values.join('|');
})()
"#,
        )
        .expect("text-decoration computed longhands should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|underline from-font rgb(255, 0, 0)|dotted rgba(10, 20, 30, 0.4)|none|from-font|underline from-font rgb(255, 0, 0)|underline|underline overline|underline overline|10px|all|all|start end|wavy|wavy|40px|40px|under right|under right|spelling-error|spelling-error|auto|wavy|under right"
    );
}

#[test]
fn computed_style_serializes_text_decoration_paint_and_webkit_text_stroke() {
    let mut vm = new_parsed_test_vm(
        "https://css-fill-stroke-computed.test/",
        r#"<html><body><div id="target"></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.getElementById('target');
  const computed = getComputedStyle(target);
  const values = [];

  values.push('text-decoration-fill' in computed);
  values.push('textDecorationFill' in computed);
  values.push('text-decoration-stroke' in computed);
  values.push('textDecorationStroke' in computed);
  values.push('-webkit-text-stroke' in computed);
  values.push('webkitTextStroke' in computed);

  values.push(computed.getPropertyValue('text-decoration-fill'));
  values.push(computed.getPropertyValue('text-decoration-stroke'));

  target.style.textDecorationFill = 'red';
  values.push(computed.textDecorationFill);

  target.style.textDecorationFill = 'rgb(12, 34, 56)';
  target.style.textDecorationStroke = 'context-stroke';
  values.push(computed.textDecorationFill);
  values.push(computed.textDecorationStroke);

  target.style.color = 'lime';
  target.style.webkitTextStroke = 'green';
  values.push(computed.getPropertyValue('-webkit-text-stroke'));

  target.style.webkitTextStroke = '3px';
  values.push(computed.getPropertyValue('-webkit-text-stroke'));

  target.style.webkitTextStroke = '1px red';
  values.push(computed.webkitTextStroke);
  values.push(computed.getPropertyValue('-webkit-text-stroke-width'));
  values.push(computed.getPropertyValue('-webkit-text-stroke-color'));

  return values.join('|');
})()
"#,
        )
        .expect("fill/stroke computed style should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|true|true|match-text|match-text|red|rgb(12, 34, 56)|context-stroke|0px rgb(0, 128, 0)|3px rgb(0, 255, 0)|1px rgb(255, 0, 0)|1px|rgb(255, 0, 0)"
    );
}

#[test]
fn computed_style_serializes_and_inherits_text_shadow() {
    let mut vm = new_parsed_test_vm(
        "https://css-text-shadow-computed.test/",
        r#"<html><body><div id="container"><div id="target"></div></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const container = document.getElementById('container');
  const target = document.getElementById('target');
  const computed = getComputedStyle(target);
  const values = [];

  values.push('text-shadow' in computed);
  values.push('textShadow' in computed);
  values.push(computed.getPropertyValue('text-shadow'));

  target.style.color = 'blue';
  target.style.fontSize = '40px';
  target.style.textShadow = '10px 20px';
  values.push(computed.getPropertyValue('text-shadow'));

  target.style.textShadow = 'red 10px 20px 30px';
  values.push(computed.textShadow);

  target.style.textShadow = 'calc(0.5em + 10px) calc(0.5em + 10px) calc(0.5em + 10px)';
  values.push(computed.getPropertyValue('text-shadow'));

  target.style.textShadow = 'calc(-0.5em + 10px) calc(-0.5em + 10px) calc(-0.5em + 10px)';
  values.push(computed.getPropertyValue('text-shadow'));

  target.style.textShadow = 'lime 10px 20px 30px, red 40px 50px';
  values.push(computed.getPropertyValue('text-shadow'));

  target.style.textShadow = '';
  container.style.color = 'rgba(2, 3, 4, 0.5)';
  container.style.textShadow = 'rgba(42, 53, 64, 0.75) 10px 20px';
  values.push(computed.getPropertyValue('text-shadow'));

  return values.join('|');
})()
"#,
        )
        .expect("text-shadow computed style should evaluate");

    assert_eq!(
        result,
        "true|true|none|rgb(0, 0, 255) 10px 20px 0px|rgb(255, 0, 0) 10px 20px 30px|rgb(0, 0, 255) 30px 30px 30px|rgb(0, 0, 255) -10px -10px 0px|rgb(0, 255, 0) 10px 20px 30px, rgb(255, 0, 0) 40px 50px 0px|rgba(42, 53, 64, 0.75) 10px 20px 0px"
    );
}

#[test]
fn computed_style_serializes_text_emphasis_properties() {
    let mut vm = new_parsed_test_vm(
        "https://css-text-emphasis-computed.test/",
        r#"<html><body><div id="target" style="color: blue"></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.getElementById('target');
  const computed = getComputedStyle(target);
  const values = [];

  values.push('text-emphasis' in computed);
  values.push('textEmphasis' in computed);
  values.push('text-emphasis-color' in computed);
  values.push('textEmphasisColor' in computed);
  values.push('text-emphasis-position' in computed);
  values.push('textEmphasisPosition' in computed);
  values.push('text-emphasis-style' in computed);
  values.push('textEmphasisStyle' in computed);

  target.style.textEmphasis = 'dot';
  values.push(computed.getPropertyValue('text-emphasis'));
  values.push(computed.textEmphasis);

  target.style.textEmphasis = 'currentColor';
  values.push(computed.getPropertyValue('text-emphasis'));

  target.style.textEmphasis = 'black';
  values.push(computed.getPropertyValue('text-emphasis'));

  target.style.textEmphasis = 'dot red';
  values.push(computed.getPropertyValue('text-emphasis'));
  values.push(computed.getPropertyValue('text-emphasis-style'));
  values.push(computed.getPropertyValue('text-emphasis-color'));

  target.style.textEmphasisPosition = 'right under';
  values.push(computed.getPropertyValue('text-emphasis-position'));

  target.style.textEmphasisStyle = 'filled';
  values.push(computed.getPropertyValue('text-emphasis-style'));

  target.style.writingMode = 'vertical-lr';
  target.style.textEmphasisStyle = 'filled';
  values.push(computed.getPropertyValue('text-emphasis-style'));

  return values.join('|');
})()
"#,
        )
        .expect("text-emphasis computed properties should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|true|true|true|true|dot rgb(0, 0, 255)|dot rgb(0, 0, 255)|none rgb(0, 0, 255)|none rgb(0, 0, 0)|dot rgb(255, 0, 0)|dot|rgb(255, 0, 0)|under|circle|sesame"
    );
}

#[test]
fn computed_style_uses_stylo_owned_extended_longhands() {
    let mut vm = new_parsed_test_vm(
        "https://css-stylo-owned-extended-longhands.test/",
        r#"<html><head><style>
          #parent {
            font-variant-alternates: historical-forms;
            font-variant-emoji: emoji;
            font-variant-position: super;
          }
          #target {
            animation-timeline: auto;
            animation-range-start: entry 10%;
            animation-range-end: exit 20%;
            column-span: all;
            column-width: 12px;
            font-variant-alternates: inherit;
            font-variant-emoji: inherit;
            font-variant-position: inherit;
            zoom: 125%;
          }
        </style></head><body><div id="parent"><div id="target"></div></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const style = getComputedStyle(document.getElementById('target'));
  const names = Array.from(style);
  return JSON.stringify({
    values: Object.fromEntries([
      'animation-timeline',
      'animation-range-start',
      'animation-range-end',
      'column-span',
      'column-width',
      'font-variant-alternates',
      'font-variant-emoji',
      'font-variant-position',
      'zoom',
    ].map(name => [name, style.getPropertyValue(name)])),
    enumerated: [
      'animation-timeline',
      'animation-range-start',
      'animation-range-end',
      'column-span',
      'column-width',
      'font-variant-alternates',
      'font-variant-emoji',
      'font-variant-position',
      'zoom',
    ].every(name => names.includes(name)),
  });
})()
"#,
        )
        .expect("Stylo-owned extended longhands should compute");

    let result: serde_json::Value = serde_json::from_str(&result).expect("valid JSON summary");
    assert_eq!(result["enumerated"], serde_json::json!(true));
    for (name, expected) in [
        ("animation-timeline", "auto"),
        ("animation-range-start", "entry 10%"),
        ("animation-range-end", "exit 20%"),
        ("column-span", "all"),
        ("column-width", "12px"),
        ("font-variant-alternates", "historical-forms"),
        ("font-variant-emoji", "emoji"),
        ("font-variant-position", "super"),
        ("zoom", "1.25"),
    ] {
        assert_eq!(result["values"][name], expected, "computed {name}");
    }
}

#[test]
fn computed_style_resolves_zoom() {
    let mut vm = new_parsed_test_vm(
        "https://css-zoom-computed.test/",
        r#"<html><head><style>#container { container-type: inline-size; width: 100px; }</style></head><body><div id="container"><div id="target" style="zoom: inherit"></div></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const container = document.getElementById('container');
  const target = document.getElementById('target');
  const computed = getComputedStyle(target);
  const values = [];

  values.push('zoom' in computed);
  values.push(computed.getPropertyValue('zoom'));

  container.style.zoom = '150%';
  values.push(getComputedStyle(container).zoom);
  values.push(computed.zoom);

  target.style.zoom = 'normal';
  values.push(computed.zoom);

  target.style.zoom = '100%';
  values.push(computed.zoom);

  target.style.zoom = '0';
  values.push(computed.zoom);

  target.style.zoom = 'calc(1 - 0.5)';
  values.push(computed.zoom);

  target.style.zoom = 'calc(1 + (sign(30deg - 40deg) * 0.5))';
  values.push(computed.zoom);

  target.style.zoom = 'calc(100% + (sign(2cqw - 10px) * 50%))';
  values.push(computed.zoom);

  return values.join('|');
})()
"#,
        )
        .expect("zoom computed longhand should evaluate");

    assert_eq!(result, "true|1|1.5|1.5|1|1|1|0.5|0.5|0.5");
}

#[test]
fn computed_style_resolves_animation_duration_auto_against_timeline() {
    let mut vm = new_parsed_test_vm(
        "https://css-animation-duration-auto.test/",
        r#"<html><head></head><body><div id="target"></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.getElementById('target');
  const values = [];

  target.style.animationDuration = 'auto';
  values.push(getComputedStyle(target).animationDuration);

  target.style.animationDuration = 'auto, auto';
  values.push(getComputedStyle(target).animationDuration);

  target.style.animationTimeline = 'auto, auto';
  values.push(getComputedStyle(target).animationDuration);

  target.style.animationTimeline = '--t';
  values.push(getComputedStyle(target).animationDuration);

  target.style.animationDuration = '0s';
  target.style.animationTimeline = 'auto, auto';
  values.push(getComputedStyle(target).animationDuration);

  return values.join('|');
})()
"#,
        )
        .expect("animation-duration auto computed style should evaluate");

    assert_eq!(result, "0s|0s, 0s|auto, auto|auto, auto|0s");
}

#[test]
fn computed_style_resolves_animation_math_against_query_container_width() {
    let mut vm = new_parsed_test_vm(
        "https://css-animation-container-math.test/",
        r#"<html><head></head><body><div id="container" style="container-type:inline-size; width:100px"><div id="target"></div></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.getElementById('target');
  target.style.animationDelay = 'calc(10s + (sign(2cqw - 10px) * 5s))';
  target.style.animationDuration = 'calc(10s + (sign(2cqw - 10px) * 5s))';
  target.style.animationIterationCount = 'calc(10 + (sign(2cqw - 10px) * 5))';
  const computed = getComputedStyle(target);
  return [
    CSS.supports('animation-delay', 'calc(10s + (sign(2cqw - 10px) * 5s))'),
    CSS.supports('animation-duration', 'calc(10s + (sign(2cqw - 10px) * 5s))'),
    CSS.supports('animation-iteration-count', 'calc(10 + (sign(2cqw - 10px) * 5))'),
    target.style.animationDelay,
    target.style.animationDuration,
    target.style.animationIterationCount,
    computed.animationDelay,
    computed.animationDuration,
    computed.animationIterationCount
  ].join('|');
})()
"#,
        )
        .expect("animation math computed style should evaluate");

    assert_eq!(
        result,
        "true|true|true|calc(10s + (5s * sign(2cqw - 10px)))|calc(10s + (5s * sign(2cqw - 10px)))|calc(10 + (5 * sign(2cqw - 10px)))|5s|5s|5"
    );
}

#[test]
fn computed_style_serializes_animation_timing_function_css_easing_math() {
    let mut vm = new_parsed_test_vm(
        "https://css-easing-computed.test/",
        r#"<html><body><div id="target"></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.getElementById('target');
  const computed = getComputedStyle(target);
  const values = [];

  target.style.animationTimingFunction = 'cubic-bezier(calc(-2), calc(0.7 / 2), calc(1.5), calc(0))';
  values.push(computed.animationTimingFunction);

  target.style.animationTimingFunction = 'steps(calc(-10), start)';
  values.push(computed.animationTimingFunction);

  target.style.animationTimingFunction = 'steps(calc(1), jump-none)';
  values.push(computed.animationTimingFunction);

  target.style.animationTimingFunction = 'linear(0, 1.3, 1, 0.92, 1, 0.99, 1, 1.004, 0.998, 1 100% 100%)';
  const linearComputed = computed.animationTimingFunction;
  values.push(linearComputed);

  target.style['animation-timing-function'] = linearComputed;
  values.push(computed.animationTimingFunction);

  return values.join('|');
})()
"#,
        )
        .expect("animation timing computed CSS easing math should evaluate");

    assert_eq!(
        result,
        "cubic-bezier(0, 0.35, 1, 0)|steps(1, start)|steps(2, jump-none)|linear(0 0%, 1.3 11.111111%, 1 22.222222%, 0.92 33.333333%, 1 44.444444%, 0.99 55.555556%, 1 66.666667%, 1.004 77.777778%, 0.998 88.888889%, 1 100%, 1 100%)|linear(0 0%, 1.3 11.111111%, 1 22.222222%, 0.92 33.333333%, 1 44.444444%, 0.99 55.555556%, 1 66.666667%, 1.004 77.777778%, 0.998 88.888889%, 1 100%, 1 100%)"
    );
}

#[test]
fn computed_style_resolves_shared_css_numeric_math_for_style_properties() {
    let mut vm = new_parsed_test_vm(
        "https://css-shared-numeric-math.test/",
        r#"<html style="font-size:30px"><head></head><body><div id="container" style="container-type:inline-size; width:100px"><div id="target" style="font-size:10px"></div></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.getElementById('target');
  target.style.transitionDelay = '-500ms, calc(2 * 3s)';
  target.style.transitionDuration = 'calc(10s + (sign(2cqw - 10px) * 5s))';
  target.style.animationRangeStart = 'exit calc(1em + 10px), cover calc(41% + 1%)';
  target.style.animationRangeEnd = 'normal, contain 100%';
  target.style.transitionTimingFunction = 'steps(calc(2 * sibling-index()), jump-none)';
  const computed = getComputedStyle(target);
  return [
    CSS.supports('transition-delay', '-500ms, calc(2 * 3s)'),
    CSS.supports('transition-duration', 'calc(10s + (sign(2cqw - 10px) * 5s))'),
    CSS.supports('transition', 'allow-discrete display 3s ease-in-out 1s'),
    target.style.transitionDelay,
    target.style.transitionDuration,
    target.style.transitionTimingFunction,
    computed.transitionDelay,
    computed.transitionDuration,
    computed.transitionTimingFunction,
    computed.animationRangeStart,
    computed.animationRangeEnd,
    computed.animationRange
  ].join('|');
})()
"#,
        )
        .expect("shared CSS numeric math computed style should evaluate");

    assert_eq!(
        result,
        "true|true|true|-500ms, calc(6s)|calc(10s + (5s * sign(2cqw - 10px)))|steps(calc(2 * sibling-index()), jump-none)|-0.5s, 6s|5s|steps(2, jump-none)|exit 20px, cover 42%|normal, contain|exit 20px normal, cover 42% contain"
    );
}

#[test]
fn computed_style_resolves_valid_typed_css_math_and_rejects_invalid_unit_algebra() {
    let mut vm = new_parsed_test_vm(
        "https://css-typed-product-math.test/",
        r#"<html style="font-size:30px"><head></head><body style="font-size:16px; line-height:1.25; width:520px; margin:20px"><div id="target"></div><div id="letter" style="font-size:20px"></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.getElementById('target');
  const letter = document.getElementById('letter');
  const validWidthValues = [
    'calc(5px * 10)',
    'calc(20% * 0.5)',
    'calc(4px * 4)',
    'calc(400px / 4)',
    'calc((20% + 1em) * 0.5)',
    'calc(100px / 1 / 1)'
  ];
  const values = validWidthValues.map((value) => {
    target.style.width = 'initial';
    target.style.width = value;
    return getComputedStyle(target).width;
  });

  letter.style.letterSpacing = 'calc(1em / 4)';
  values.push(getComputedStyle(letter).letterSpacing);
  letter.style.letterSpacing = 'calc(2 * 1em)';
  values.push(getComputedStyle(letter).letterSpacing);
  letter.style.letterSpacing = '7px';
  letter.style.letterSpacing = 'calc(1em / 1rem * 1px)';
  values.push(letter.style.letterSpacing);

  const invalidWidthValues = [
    'calc(5px * 10lh / 1px)',
    'calc(20% * 0.5em / 1px)',
    'calc(400px / 4lh * 1px)',
    'calc(20% / 0.5em * 1px)',
    'calc(52px * 1px / 10%)',
    'calc(100px * 1px / 1px / 1)'
  ];
  const invalidValues = invalidWidthValues.map((value) => {
    target.style.width = '13px';
    target.style.width = value;
    return target.style.width;
  });

  return values.join('|') + '::' + invalidValues.join('|');
})()
"#,
        )
        .expect("typed CSS math products should compute");

    assert_eq!(
        result,
        "50px|52px|16px|100px|60px|100px|5px|40px|7px::13px|13px|13px|13px|13px|13px"
    );
}

#[test]
fn computed_transform_scale_serializes_as_matrix() {
    let mut vm = new_parsed_test_vm(
        "https://computed-transform-scale.test/",
        r#"<html><body></body></html>"#,
    );
    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  document.body.appendChild(target);
  const values = [];

  target.style.transform = 'scale(min(.1))';
  values.push(target.style.transform);
  values.push(getComputedStyle(target).transform);

  target.style.transform = 'scale(calc(max(.1) + .1))';
  values.push(getComputedStyle(target).transform);

  target.style.transform = 'scale(.25, .5)';
  values.push(getComputedStyle(target).transform);

  return values.join('|');
})()
"#,
        )
        .expect("computed transform scale matrix serialization should evaluate");

    assert_eq!(
        result,
        "scale(calc(0.1))|matrix(0.1, 0, 0, 0.1, 0, 0)|matrix(0.2, 0, 0, 0.2, 0, 0)|matrix(0.25, 0, 0, 0.5, 0, 0)"
    );
}

#[test]
fn computed_transform_rotate_calc_serializes_as_matrix() {
    let mut vm = new_parsed_test_vm(
        "https://computed-transform-rotate-calc.test/",
        r#"<html><body><div id="target"></div></body></html>"#,
    );
    let result = vm
        .eval(
            r#"
(() => {
  const target = document.getElementById('target');

  function matrixValues(text) {
    if (!text.startsWith('matrix(')) {
      return null;
    }
    return text.slice('matrix('.length, -1).split(',').map((part) => Number(part.trim()));
  }

  function closeToSpecified(specified, expected) {
    target.style.transform = 'initial';
    target.style.transform = specified;
    const actualText = getComputedStyle(target).transform;
    const actual = matrixValues(actualText);

    target.style.transform = 'initial';
    target.style.transform = expected;
    const referenceText = getComputedStyle(target).transform;
    const reference = matrixValues(referenceText);

    const ok = actual !== null &&
      reference !== null &&
      actual.length === reference.length &&
      actual.every((value, index) => Math.abs(value - reference[index]) < 0.0001);
    return ok ? 'ok' : `${specified}=>${actualText} / ${expected}=>${referenceText}`;
  }

  return [
    closeToSpecified('rotate(calc(45deg + 45deg))', 'rotate(90deg)'),
    closeToSpecified('rotate(calc(90deg - 1rad))', 'rotate(32.70422deg)'),
    closeToSpecified('rotate(calc(45rad + 45rad))', 'rotate(90rad)'),
    closeToSpecified('rotate(calc(30rad - 10grad))', 'rotate(1709.87339deg)'),
    closeToSpecified('rotate(calc(2 * 45rad))', 'rotate(90rad)'),
    closeToSpecified('rotate(calc(45grad * 2))', 'rotate(90grad)'),
    closeToSpecified('rotate(calc(90turn / 2))', 'rotate(45turn)')
  ].join('|');
})()
"#,
        )
        .expect("computed transform rotate calc matrix serialization should evaluate");

    assert_eq!(result, "ok|ok|ok|ok|ok|ok|ok");
}

#[test]
fn computed_style_serializes_animation_range_shorthand() {
    let mut vm = new_parsed_test_vm(
        "https://css-animation-range-computed-shorthand.test/",
        r#"<html><head></head><body><div id="target" style="font-size:10px"></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.getElementById('target');
  const values = [];

  values.push(getComputedStyle(target).animationRange);

  target.style.animationRange = 'entry 10% exit 20%';
  values.push(getComputedStyle(target).animationRange);

  target.style.animationRange = 'normal normal';
  values.push(getComputedStyle(target).animationRange);

  target.style.animationRange = '100px normal';
  values.push(getComputedStyle(target).animationRange);

  target.style.animationRangeStart = 'cover 100%';
  target.style.animationRangeEnd = 'cover 0%';
  values.push(`${getComputedStyle(target).animationRangeStart}/${getComputedStyle(target).animationRangeEnd}`);

  target.style.animationRange = 'entry calc(10% - 10%) entry calc(50% + 50%)';
  values.push(getComputedStyle(target).animationRange);

  target.style.animationRange = '10% calc(70% + 10% * sign(100em - 1px))';
  values.push(getComputedStyle(target).animationRange);

  target.style.animationRangeStart = 'cover 120%';
  target.style.animationRangeEnd = '120%';
  values.push(`${getComputedStyle(target).animationRangeStart}/${getComputedStyle(target).animationRangeEnd}`);

  return values.join('|');
})()
"#,
        )
        .expect("computed animation-range shorthand should evaluate");

    assert_eq!(
        result,
        "normal|entry 10% exit 20%|normal|100px|cover 100%/cover 0%|entry|10% 80%|cover 120%/120%"
    );
}
#[test]
fn mouse_event_offsets_follow_retargeted_shadow_targets() {
    let mut vm = new_parsed_test_vm(
        "https://shadow-mouse-offset.test/",
        r#"<html><head></head><body></body></html>"#,
    );
    vm.force_fresh_layout_reads_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const pageStyle = document.createElement('style');
  pageStyle.textContent =
    'html, body { padding: 0; margin: 0; } ' +
    'my-host { display: block; width: 180px; height: 80px; margin: 10px 20px; padding: 10px; }';
  document.documentElement.firstChild.appendChild(pageStyle);
  // Keep the span non-atomic while fixing the shared line-box top used by
  // MouseEvent offsets. The span's own fragment top still depends on font
  // metrics, so compare it only across equivalent light/shadow trees below.
  const shadowStyle =
    '#container { width: 160px; height: 60px; padding: 10px; } ' +
    '#target { line-height: 20px; vertical-align: top; margin-left: 5px; }';

  function makeHost(id) {
    for (const host of document.querySelectorAll('my-host')) {
      host.remove();
    }
    const host = document.createElement('my-host');
    host.id = id;
    document.body.appendChild(host);
    return host;
  }

  function attachLoggers(targets, eventType) {
    const logs = [];
    for (const target of targets) {
      target.addEventListener(eventType, function (event) {
        logs.push([
          this.localName || '#shadow-root',
          event.target.localName,
          event.offsetX,
          event.offsetY
        ].join(':'));
      });
    }
    return logs;
  }

  const light = makeHost('light');
  light.innerHTML =
    '<style>' + shadowStyle + '</style><div id="container"><span id="target">Click</span></div>';
  const lightTarget = light.querySelector('#target');
  const lightTargetOffsetTop = lightTarget.offsetTop;
  const lightContainer = light.querySelector('#container');
  const lightLogs = attachLoggers([lightTarget, lightContainer, light, document.body], 'light-down');
  const lightEvent = new MouseEvent('light-down', {
    clientX: 51.4,
    clientY: 37.4,
    composed: true,
    bubbles: true
  });
  lightTarget.dispatchEvent(lightEvent);
  const offsetDescriptor =
    Object.getOwnPropertyDescriptor(MouseEvent.prototype, 'offsetX');
  let incompatibleReceiver;
  try {
    offsetDescriptor.get.call({});
    incompatibleReceiver = 'accepted';
  } catch (error) {
    incompatibleReceiver = error.name;
  }
  const lightResult = [
    light.offsetLeft,
    light.offsetTop,
    lightTarget.offsetLeft,
    Object.hasOwn(lightEvent, 'offsetX'),
    Object.hasOwn(lightEvent, 'offsetY'),
    Object.hasOwn(MouseEvent.prototype, 'offsetX'),
    Object.hasOwn(MouseEvent.prototype, 'offsetY'),
    offsetDescriptor.get.name,
    offsetDescriptor.get.length,
    offsetDescriptor.enumerable,
    offsetDescriptor.configurable,
    offsetDescriptor.get.call(lightEvent) === lightEvent.offsetX,
    incompatibleReceiver,
    lightLogs.join(',')
  ].join('|');

  const closed = makeHost('closed');
  const root = closed.attachShadow({ mode: 'closed' });
  root.innerHTML =
    '<style>' + shadowStyle + '</style><div id="container"><span id="target">Click</span></div>';
  const closedTarget = root.querySelector('#target');
  const closedContainer = root.querySelector('#container');
  const closedLogs = attachLoggers([closedTarget, closedContainer, root, closed, document.body], 'closed-down');
  closedTarget.dispatchEvent(new MouseEvent('closed-down', {
    clientX: 51.4,
    clientY: 37.4,
    composed: true,
    bubbles: true
  }));
  const closedResult = [
    closedTarget.offsetLeft,
    closedTarget.offsetTop === lightTargetOffsetTop,
    closedLogs.join(',')
  ].join('|');

  const slotted = makeHost('slotted');
  const slottedRoot = slotted.attachShadow({ mode: 'open' });
  slottedRoot.innerHTML =
    '<style>' + shadowStyle + '</style><div id="container"><slot></slot></div>';
  slotted.innerHTML =
    '<style>' + shadowStyle + '</style><div id="target">Click</div>';
  const slottedTarget = slotted.querySelector('#target');
  const slottedContainer = slottedRoot.querySelector('#container');
  const slottedLogs = attachLoggers([
    slottedTarget,
    slottedContainer,
    slottedRoot,
    slotted,
    document.body
  ], 'slotted-down');
  slottedTarget.dispatchEvent(new MouseEvent('slotted-down', {
    clientX: 51.4,
    clientY: 37.4,
    composed: true,
    bubbles: true
  }));
  const slottedResult = [
    slottedTarget.offsetLeft,
    slottedTarget.offsetTop,
    slottedLogs.join(',')
  ].join('|');

  return [lightResult, closedResult, slottedResult].join('|');
})()
"#,
        )
        .expect("MouseEvent offsetX/Y should evaluate across shadow boundaries");

    assert_eq!(
        result,
        concat!(
            "20|10|45|false|false|true|true|get offsetX|0|true|true|true|TypeError|",
            "span:span:21:17,div:span:21:17,my-host:span:21:17,body:span:21:17|",
            "45|true|",
            "span:span:21:17,div:span:21:17,#shadow-root:span:21:17,",
            "my-host:my-host:31:27,body:my-host:31:27|",
            "45|30|",
            "div:div:6:7,div:div:6:7,#shadow-root:div:6:7,",
            "my-host:div:6:7,body:div:6:7"
        )
    );
}

#[test]
fn computed_custom_functions_resolve_shadow_scoped_container_queries() {
    let mut vm = new_parsed_test_vm(
        "https://shadow-custom-function-container.test/",
        r#"<html><head></head><body></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const partHost = document.createElement('x-part-host');
  document.body.appendChild(partHost);
  const partOuter = partHost.attachShadow({ mode: 'open' });
  partOuter.innerHTML = `
    <style>
      @function --b() {
        @container --cont (width = 5px) { result: 5px; }
        @container --cont (width = 10px) { result: 10px; }
      }
      ::part(target) {
        --actual: --b();
        --expected: 5px;
      }
      section > .container[data-kind="outer"] {
        container: --cont / size;
        width: 10px;
        height: 10px;
      }
    </style>
    <section><div class="container" data-kind="outer"></div></section>`;
  const partInnerHost = partOuter.querySelector('.container');
  const partInner = partInnerHost.attachShadow({ mode: 'open' });
  partInner.innerHTML = `
    <style>
      @function --b() { result: FAIL; }
      section > .container[data-kind="inner"] {
        container: --cont / size;
        width: 5px;
        height: 5px;
      }
    </style>
    <section><div class="container" data-kind="inner"><div id="target" part="target"></div></div></section>`;
  const partStyle = getComputedStyle(partInner.querySelector('#target'));
  const partContainerStyle = getComputedStyle(partInner.querySelector('.container'));

  const slotHost = document.createElement('x-slot-host');
  document.body.appendChild(slotHost);
  const slotOuter = slotHost.attachShadow({ mode: 'open' });
  slotOuter.innerHTML = `
    <style>
      @function --b() {
        @container --cont (width = 5px) { result: 5px; }
        @container --cont (width = 10px) { result: 10px; }
      }
      section > .container[data-kind="outer"] {
        container: --cont / size;
        width: 10px;
        height: 10px;
      }
    </style>
    <section><div class="container" data-kind="outer"><div id="target"></div></div></section>`;
  const slotInnerHost = slotOuter.querySelector('.container');
  const slotInner = slotInnerHost.attachShadow({ mode: 'open' });
  slotInner.innerHTML = `
    <style>
      @function --c() {
        @container --cont (width = 5px) { result: 5px; }
        @container --cont (width = 10px) { result: 10px; }
      }
      section > .container[data-kind="inner"] {
        container: --cont / size;
        width: 5px;
        height: 5px;
      }
      ::slotted(#target) {
        --actual: --b() --c();
        --expected: 5px 5px;
      }
    </style>
    <section><div class="container" data-kind="inner"><slot></slot></div></section>`;
  const slotStyle = getComputedStyle(slotOuter.querySelector('#target'));
  const slotContainerStyle = getComputedStyle(slotInner.querySelector('.container'));

  return [
    partStyle.getPropertyValue('--actual'),
    partStyle.getPropertyValue('--expected'),
    slotStyle.getPropertyValue('--actual'),
    slotStyle.getPropertyValue('--expected'),
    partContainerStyle.containerName,
    partContainerStyle.containerType,
    slotContainerStyle.containerName,
    slotContainerStyle.containerType
  ].join('|');
})()
"#,
        )
        .expect("shadow-scoped custom functions should evaluate");

    assert_eq!(result, "5px|5px|5px 5px|5px 5px|--cont|size|--cont|size");
}
