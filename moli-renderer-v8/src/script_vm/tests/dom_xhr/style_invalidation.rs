use super::*;

#[test]
fn dom_content_loaded_page_task_drains_pending_style_invalidations_after_listener() {
    let mut vm = new_storage_test_vm("https://dcl-style-invalidation-drain.test/");
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '.active { color: rgb(30, 40, 50); }';
  head.appendChild(style);
  globalThis.__dclDrainTarget = document.createElement('div');
  body.appendChild(globalThis.__dclDrainTarget);
  document.addEventListener('DOMContentLoaded', () => {
    globalThis.__dclDrainTarget.className = 'active';
  });
  return getComputedStyle(globalThis.__dclDrainTarget).color;
})()
"#,
        )
        .expect("DOMContentLoaded style drain setup should evaluate");

    assert_eq!(initial, "rgb(0, 0, 0)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let owner = vm
        .current_main_document_task_owner()
        .expect("style invalidation test requires a current document owner");
    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser completion should prepare the interactive transition");
    let _ = vm
        .execute_post_parse_lifecycle_work_best_effort(
            PostParseLifecycleWork::ApplyMainDocumentInteractive(interactive),
        )
        .expect("interactive page task should dispatch");
    let _ = vm
        .execute_post_parse_lifecycle_work_best_effort(
            PostParseLifecycleWork::DispatchDomContentLoaded { owner },
        )
        .expect("DOMContentLoaded page task should dispatch");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );

    let resolved = vm
        .eval(
            r#"
(() => {
  const color = getComputedStyle(globalThis.__dclDrainTarget).color;
  delete globalThis.__dclDrainTarget;
  return color;
})()
"#,
        )
        .expect("DOMContentLoaded drained style should resolve");

    assert_eq!(resolved, "rgb(30, 40, 50)");
}

#[test]
fn style_element_text_mutations_cache_owner_stylesheet_text() {
    let mut vm = new_storage_test_vm("https://style-text-cache.test/");

    let setup = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const sheet = document.createElement('style');
  sheet.id = 'sheet';
  sheet.textContent = '#item { color: rgb(1, 2, 3); }';
  head.append(sheet);
  const item = document.createElement('div');
  item.id = 'item';
  body.append(item);
  return getComputedStyle(item).color;
})()
"#,
        )
        .expect("initial style text should evaluate");
    assert_eq!(setup, "rgb(1, 2, 3)");

    let style = vm
        .document_runtime
        .dom_host()
        .element_handle_by_id("sheet")
        .expect("style handle");
    let cached = vm._context_host.borrow().owner_style_sheet_text(style);
    assert_eq!(cached.as_deref(), Some("#item { color: rgb(1, 2, 3); }"));

    let after_append = vm
        .eval(
            r#"
(() => {
  const sheet = document.getElementById('sheet');
  sheet.appendChild(document.createTextNode(' #item { background-color: rgb(4, 5, 6); }'));
  const item = document.getElementById('item');
  return [
    getComputedStyle(item).color,
    getComputedStyle(item).backgroundColor
  ].join('|');
})()
"#,
        )
        .expect("appended style text should evaluate");
    assert_eq!(after_append, "rgb(1, 2, 3)|rgb(4, 5, 6)");

    let cached = vm._context_host.borrow().owner_style_sheet_text(style);
    assert_eq!(
        cached.as_deref(),
        Some("#item { color: rgb(1, 2, 3); } #item { background-color: rgb(4, 5, 6); }")
    );
}

#[test]
fn host_attribute_selector_style_updates_after_shadow_host_reattach() {
    let mut vm = new_storage_test_vm("https://shadow-host-attribute-style.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(':host([red]) { color: red; } :host(.blue) { color: blue; }');

  const host = document.createElement('div');
  host.toggleAttribute('red');
  body.appendChild(host);
  const beforeShadow = getComputedStyle(host).color;

  const shadow = host.attachShadow({ mode: 'open' });
  shadow.adoptedStyleSheets = [sheet];
  const withShadow = getComputedStyle(host).color;

  host.remove();
  body.appendChild(host);
  const reattached = getComputedStyle(host).color;

  host.toggleAttribute('red');
  const afterAttribute = getComputedStyle(host).color;

  host.classList.toggle('blue');
  const afterClass = getComputedStyle(host).color;

  return [beforeShadow, withShadow, reattached, afterAttribute, afterClass].join('|');
})()
"#,
        )
        .expect("shadow host attribute selector style should evaluate");

    assert_eq!(
        result,
        "rgb(0, 0, 0)|rgb(255, 0, 0)|rgb(255, 0, 0)|rgb(0, 0, 0)|rgb(0, 0, 255)"
    );
}
#[test]
fn attribute_selector_style_updates_with_unrelated_mutation_observer() {
    let mut vm = new_storage_test_vm("https://attribute-style-mutation-observer.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '#target[data-state="active"] { color: rgb(0, 0, 255); }';
  head.appendChild(style);

  const target = document.createElement('div');
  target.id = 'target';
  body.appendChild(target);
  const before = getComputedStyle(target).color;

  const observer = new MutationObserver(() => {});
  observer.observe(body, { childList: true });
  target.setAttribute('data-state', 'active');
  const afterSet = getComputedStyle(target).color;

  target.removeAttribute('data-state');
  const afterRemove = getComputedStyle(target).color;
  return [before, afterSet, afterRemove, observer.takeRecords().length].join('|');
})()
"#,
        )
        .expect("attribute selector style should update with unrelated MutationObserver");

    assert_eq!(result, "rgb(0, 0, 0)|rgb(0, 0, 255)|rgb(0, 0, 0)|0");
}

#[test]
fn has_pseudo_class_invalidation_updates_derived_form_states() {
    let mut vm = new_storage_test_vm("https://has-derived-form-state-invalidation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = [
    'main:has(input) div { color: rgb(128, 128, 128); }',
    'main:has(#option:disabled) > #subject2 { color: rgb(0, 128, 0); }',
    'main:not(:has(#option:enabled)) > #subject4 { color: rgb(0, 128, 0); }',
    'main:has(#form:valid) > #subject3 { color: rgb(255, 255, 0); }',
    'main:not(:has(#form:invalid)) > #subject4 { color: rgb(255, 255, 0); }'
  ].join('\n');
  head.appendChild(style);

  const main = document.createElement('main');
  const form = document.createElement('form');
  form.id = 'form';
  const select = document.createElement('select');
  const optgroup = document.createElement('optgroup');
  optgroup.id = 'optgroup';
  const option = document.createElement('option');
  option.id = 'option';
  option.textContent = 'b';
  optgroup.appendChild(option);
  select.appendChild(optgroup);
  const input = document.createElement('input');
  input.id = 'text_input';
  input.required = true;
  form.append(select, input);
  const subject2 = document.createElement('div');
  subject2.id = 'subject2';
  const subject3 = document.createElement('div');
  subject3.id = 'subject3';
  const subject4 = document.createElement('div');
  subject4.id = 'subject4';
  main.append(form, subject2, subject3, subject4);
  body.appendChild(main);

  const before = [subject2, subject3, subject4].map((element) => getComputedStyle(element).color);
  optgroup.disabled = true;
  const afterOptgroup = [subject2, subject4].map((element) => getComputedStyle(element).color);
  optgroup.disabled = false;
  input.value = 'value';
  const afterValue = [subject3, subject4].map((element) => getComputedStyle(element).color);
  return JSON.stringify({ before, afterOptgroup, afterValue });
})()
"#,
        )
        .expect("derived form state invalidation should evaluate");

    assert_eq!(
        result,
        r#"{"before":["rgb(128, 128, 128)","rgb(128, 128, 128)","rgb(128, 128, 128)"],"afterOptgroup":["rgb(0, 128, 0)","rgb(0, 128, 0)"],"afterValue":["rgb(255, 255, 0)","rgb(255, 255, 0)"]}"#
    );
}

#[test]
fn focus_selector_invalidation_preserves_unrelated_cache_entries() {
    let mut vm = new_storage_test_vm("https://focus-style-cache-targeted.test/");
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
    '#outside { color: rgb(1, 2, 3); }',
    '#inside { color: rgb(4, 5, 6); }',
    '#target:focus #inside { color: rgb(7, 8, 9); }'
  ].join('\n');
  head.appendChild(style);

  const outside = document.createElement('div');
  outside.id = 'outside';
  const target = document.createElement('div');
  target.id = 'target';
  target.tabIndex = 0;
  const inside = document.createElement('span');
  inside.id = 'inside';
  target.appendChild(inside);
  body.append(outside, target);
  globalThis.__focusTargetedTarget = target;
  globalThis.__focusTargetedInsideStyle = getComputedStyle(inside);
  return [
    getComputedStyle(outside).color,
    globalThis.__focusTargetedInsideStyle.color
  ].join('|');
})()
"#,
        )
        .expect("focus targeted style setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)|rgb(4, 5, 6)");
    let generation_before_focus =
        vm.computed_style_cache_generation_for_document_for_test(document);
    let cache_count_before_focus =
        vm.computed_style_cache_entry_count_for_document_for_test(document);
    assert!(cache_count_before_focus >= 2);

    let focused = vm
        .eval(
            r#"
(() => {
  globalThis.__focusTargetedTarget.focus();
  return String(document.activeElement === globalThis.__focusTargetedTarget);
})()
"#,
        )
        .expect("focus targeted mutation should evaluate");

    assert_eq!(focused, "true");
    assert_eq!(
        vm.computed_style_cache_generation_for_document_for_test(document),
        generation_before_focus,
        "targeted focus invalidation should preserve the retained style generation"
    );
    let cache_count_after_focus =
        vm.computed_style_cache_entry_count_for_document_for_test(document);
    assert!(
        cache_count_after_focus > 0,
        "focus selector without sibling/focus-within should preserve unrelated cache entries"
    );

    let color = vm
        .eval(
            r#"
(() => {
  const color = globalThis.__focusTargetedInsideStyle.color;
  delete globalThis.__focusTargetedTarget;
  delete globalThis.__focusTargetedInsideStyle;
  return color;
})()
"#,
        )
        .expect("focus targeted style read should evaluate");

    assert_eq!(color, "rgb(7, 8, 9)");
}
#[test]
fn has_focus_selector_change_retains_style_system() {
    let mut vm = new_storage_test_vm("https://has-focus-style-cache-targeted.test/");
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
    '#outside { color: rgb(1, 2, 3); }',
    'body:has(#target:focus) #outside { color: rgb(7, 8, 9); }'
  ].join('\n');
  head.appendChild(style);

  const outside = document.createElement('div');
  outside.id = 'outside';
  const target = document.createElement('button');
  target.id = 'target';
  body.append(outside, target);
  globalThis.__hasFocusTarget = target;
  globalThis.__hasFocusOutsideStyle = getComputedStyle(outside);
  return globalThis.__hasFocusOutsideStyle.color;
})()
"#,
        )
        .expect("has focus selector setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)");
    let generation_before_focus =
        vm.computed_style_cache_generation_for_document_for_test(document);

    let focused = vm
        .eval(
            r#"
(() => {
  globalThis.__hasFocusTarget.focus();
  const color = globalThis.__hasFocusOutsideStyle.color;
  delete globalThis.__hasFocusTarget;
  delete globalThis.__hasFocusOutsideStyle;
  return color;
})()
"#,
        )
        .expect("has focus selector mutation should evaluate");

    assert_eq!(focused, "rgb(7, 8, 9)");
    assert_eq!(
        vm.computed_style_cache_generation_for_document_for_test(document),
        generation_before_focus,
        ":has(:focus) invalidation should preserve the retained style generation"
    );
}
#[test]
fn attribute_selector_invalidation_preserves_unaffected_subtree_cache() {
    let mut vm = new_storage_test_vm("https://attribute-style-targeted-cache.test/");
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
    '#outside { color: rgb(1, 2, 3); }',
    '#target[data-state="active"] { color: rgb(4, 5, 6); }',
    '#target[data-state="active"] ~ #later { color: rgb(7, 8, 9); }',
  ].join('\n');
  head.appendChild(style);

  const outsideRoot = document.createElement('section');
  const outside = document.createElement('div');
  outside.id = 'outside';
  outsideRoot.appendChild(outside);

  const mutationRoot = document.createElement('section');
  const target = document.createElement('div');
  target.id = 'target';
  const later = document.createElement('div');
  later.id = 'later';
  mutationRoot.append(target, later);
  body.append(outsideRoot, mutationRoot);

  globalThis.__attributeTarget = target;
  globalThis.__attributeLater = later;
  globalThis.__attributeOutside = outside;
  return [
    getComputedStyle(outside).color,
    getComputedStyle(target).color,
    getComputedStyle(later).color,
  ].join('|');
})()
"#,
        )
        .expect("attribute selector targeted cache setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)|rgb(0, 0, 0)|rgb(0, 0, 0)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        3
    );

    let mutated = vm
        .eval("globalThis.__attributeTarget.setAttribute('data-state', 'active'); 'mutated'")
        .expect("attribute selector targeted cache mutation should evaluate");

    assert_eq!(mutated, "mutated");
    vm._context_host
        .borrow()
        .drain_pending_style_invalidations_for_document_for_test(document);
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let outside_after_mutation = vm
        .eval("getComputedStyle(globalThis.__attributeOutside).color")
        .expect("unaffected attribute selector cache read should evaluate");

    assert_eq!(outside_after_mutation, "rgb(1, 2, 3)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1,
        "unaffected cache entries should remain directly reusable after targeted invalidation"
    );

    let after = vm
        .eval(
            r#"
[
  getComputedStyle(globalThis.__attributeOutside).color,
  getComputedStyle(globalThis.__attributeTarget).color,
  getComputedStyle(globalThis.__attributeLater).color,
].join('|')
"#,
        )
        .expect("attribute selector targeted cache read should evaluate");

    assert_eq!(after, "rgb(1, 2, 3)|rgb(4, 5, 6)|rgb(7, 8, 9)");
}
#[test]
fn non_sibling_attribute_invalidation_preserves_same_parent_sibling_cache() {
    let mut vm = new_storage_test_vm("https://attribute-style-non-sibling-cache.test/");
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
    '#target[data-state="active"] { color: rgb(4, 5, 6); }',
    '#kept { color: rgb(7, 8, 9); }',
    '#target[data-state="active"] .child { color: rgb(10, 11, 12); }',
  ].join('\n');
  head.appendChild(style);

  const mutationRoot = document.createElement('section');
  const target = document.createElement('div');
  target.id = 'target';
  const child = document.createElement('span');
  child.className = 'child';
  target.appendChild(child);
  const kept = document.createElement('div');
  kept.id = 'kept';
  mutationRoot.append(target, kept);
  body.appendChild(mutationRoot);

  globalThis.__nonSiblingAttributeTarget = target;
  globalThis.__nonSiblingAttributeChild = child;
  globalThis.__nonSiblingAttributeKept = kept;
  return [
    getComputedStyle(target).color,
    getComputedStyle(child).color,
    getComputedStyle(kept).color,
  ].join('|');
})()
"#,
        )
        .expect("non-sibling attribute targeted cache setup should evaluate");

    assert_eq!(setup, "rgb(0, 0, 0)|rgb(0, 0, 0)|rgb(7, 8, 9)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        3
    );

    let mutated = vm
        .eval("globalThis.__nonSiblingAttributeTarget.setAttribute('data-state', 'active'); 'mutated'")
        .expect("non-sibling attribute targeted cache mutation should evaluate");

    assert_eq!(mutated, "mutated");
    vm._context_host
        .borrow()
        .drain_pending_style_invalidations_for_document_for_test(document);
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let after = vm
        .eval(
            r#"
[
  getComputedStyle(globalThis.__nonSiblingAttributeTarget).color,
  getComputedStyle(globalThis.__nonSiblingAttributeChild).color,
  getComputedStyle(globalThis.__nonSiblingAttributeKept).color,
].join('|')
"#,
        )
        .expect("non-sibling attribute targeted cache read should evaluate");

    assert_eq!(after, "rgb(4, 5, 6)|rgb(10, 11, 12)|rgb(7, 8, 9)");
}
#[test]
fn sibling_attribute_invalidation_preserves_previous_sibling_cache() {
    let mut vm = new_storage_test_vm("https://attribute-style-sibling-cache.test/");
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
    '#outside { color: rgb(1, 2, 3); }',
    '#previous { color: rgb(4, 5, 6); }',
    '#target { color: rgb(7, 8, 9); }',
    '#later { color: rgb(10, 11, 12); }',
    '#target[data-state="active"] + #later { color: rgb(13, 14, 15); }',
  ].join('\n');
  head.appendChild(style);

  const outside = document.createElement('div');
  outside.id = 'outside';
  const mutationRoot = document.createElement('section');
  const previous = document.createElement('div');
  previous.id = 'previous';
  const target = document.createElement('div');
  target.id = 'target';
  const later = document.createElement('div');
  later.id = 'later';
  mutationRoot.append(previous, target, later);
  body.append(outside, mutationRoot);

  globalThis.__siblingAttributeOutside = outside;
  globalThis.__siblingAttributePrevious = previous;
  globalThis.__siblingAttributeTarget = target;
  globalThis.__siblingAttributeLater = later;
  globalThis.__siblingAttributeLaterStyle = getComputedStyle(later);
  return [
    getComputedStyle(outside).color,
    getComputedStyle(previous).color,
    getComputedStyle(target).color,
    globalThis.__siblingAttributeLaterStyle.color,
  ].join('|');
})()
"#,
        )
        .expect("sibling attribute cache setup should evaluate");

    assert_eq!(
        setup,
        "rgb(1, 2, 3)|rgb(4, 5, 6)|rgb(7, 8, 9)|rgb(10, 11, 12)"
    );
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        4
    );

    let mutated = vm
        .eval("globalThis.__siblingAttributeTarget.setAttribute('data-state', 'active'); 'mutated'")
        .expect("sibling attribute mutation should evaluate");

    assert_eq!(mutated, "mutated");
    vm._context_host
        .borrow()
        .drain_pending_style_invalidations_for_document_for_test(document);
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        3
    );

    let after = vm
        .eval(
            r#"
[
  getComputedStyle(globalThis.__siblingAttributeOutside).color,
  getComputedStyle(globalThis.__siblingAttributePrevious).color,
  getComputedStyle(globalThis.__siblingAttributeTarget).color,
  globalThis.__siblingAttributeLaterStyle.color,
].join('|')
"#,
        )
        .expect("sibling attribute cache read should evaluate");

    assert_eq!(
        after,
        "rgb(1, 2, 3)|rgb(4, 5, 6)|rgb(7, 8, 9)|rgb(13, 14, 15)"
    );
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        4,
        "reading the invalidated sibling style should repopulate its computed cache entry"
    );
}
#[test]
fn class_and_id_attribute_invalidation_preserves_unaffected_subtree_cache() {
    let mut vm = new_storage_test_vm("https://class-id-style-targeted-cache.test/");
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
    '#outside { color: rgb(1, 2, 3); }',
    '.active { color: rgb(4, 5, 6); }',
    '#renamed { color: rgb(7, 8, 9); }',
  ].join('\n');
  head.appendChild(style);

  const outsideRoot = document.createElement('section');
  const outside = document.createElement('div');
  outside.id = 'outside';
  outsideRoot.appendChild(outside);

  const mutationRoot = document.createElement('section');
  const classTarget = document.createElement('div');
  const idTarget = document.createElement('div');
  mutationRoot.append(classTarget, idTarget);
  body.append(outsideRoot, mutationRoot);

  globalThis.__classIdOutside = outside;
  globalThis.__classTarget = classTarget;
  globalThis.__idTarget = idTarget;
  return [
    getComputedStyle(outside).color,
    getComputedStyle(classTarget).color,
    getComputedStyle(idTarget).color,
  ].join('|');
})()
"#,
        )
        .expect("class/id targeted cache setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)|rgb(0, 0, 0)|rgb(0, 0, 0)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        3
    );

    let mutated = vm
        .eval(
            r#"
(() => {
  globalThis.__classTarget.setAttribute('class', 'active');
  globalThis.__idTarget.setAttribute('id', 'renamed');
  return 'mutated';
})()
"#,
        )
        .expect("class/id targeted cache mutation should evaluate");

    assert_eq!(mutated, "mutated");
    vm._context_host
        .borrow()
        .drain_pending_style_invalidations_for_document_for_test(document);
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let after = vm
        .eval(
            r#"
[
  getComputedStyle(globalThis.__classIdOutside).color,
  getComputedStyle(globalThis.__classTarget).color,
  getComputedStyle(globalThis.__idTarget).color,
].join('|')
"#,
        )
        .expect("class/id targeted cache read should evaluate");

    assert_eq!(after, "rgb(1, 2, 3)|rgb(4, 5, 6)|rgb(7, 8, 9)");
}
#[test]
fn has_selector_attribute_mutation_invalidates_ancestor_dependent_styles() {
    let mut vm = new_storage_test_vm("https://has-attribute-style-cache.test/");
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
    '#outside { color: rgb(1, 2, 3); }',
    'body:has(#target[data-state="active"]) #outside { color: rgb(9, 8, 7); }',
  ].join('\n');
  head.appendChild(style);

  const outsideRoot = document.createElement('section');
  const outside = document.createElement('div');
  outside.id = 'outside';
  outsideRoot.appendChild(outside);

  const mutationRoot = document.createElement('section');
  const target = document.createElement('div');
  target.id = 'target';
  mutationRoot.appendChild(target);
  body.append(outsideRoot, mutationRoot);

  globalThis.__hasAttributeTarget = target;
  globalThis.__hasAttributeOutside = outside;
  return getComputedStyle(outside).color;
})()
"#,
        )
        .expect("has selector attribute mutation setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let mutated = vm
        .eval("globalThis.__hasAttributeTarget.setAttribute('data-state', 'active'); 'mutated'")
        .expect("has selector attribute mutation should evaluate");

    assert_eq!(mutated, "mutated");
    vm._context_host
        .borrow()
        .drain_pending_style_invalidations_for_document_for_test(document);
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );

    let after = vm
        .eval("getComputedStyle(globalThis.__hasAttributeOutside).color")
        .expect("has selector attribute mutation read should evaluate");

    assert_eq!(after, "rgb(9, 8, 7)");
}
#[test]
fn detached_style_has_selector_does_not_force_full_attribute_invalidation() {
    let mut vm = new_storage_test_vm("https://detached-style-has-cache.test/");
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
    '#outside { color: rgb(1, 2, 3); }',
    '#target[data-state="active"] { color: rgb(4, 5, 6); }',
  ].join('\n');
  head.appendChild(style);

  const detachedStyle = document.createElement('style');
  detachedStyle.textContent =
    'body:has(#target[data-state="active"]) #outside { color: rgb(9, 8, 7); }';
  globalThis.__detachedHasStyle = detachedStyle;

  const detachedDocument = document.implementation.createHTMLDocument('');
  const detachedDocumentStyle = detachedDocument.createElement('style');
  detachedDocumentStyle.textContent =
    'body:has(#target[data-state="active"]) #outside { color: rgb(6, 6, 6); }';
  detachedDocument.head.appendChild(detachedDocumentStyle);
  globalThis.__detachedHasDocument = detachedDocument;

  const outside = document.createElement('div');
  outside.id = 'outside';
  const target = document.createElement('div');
  target.id = 'target';
  body.append(outside, target);

  globalThis.__detachedHasOutside = outside;
  globalThis.__detachedHasTarget = target;
  return getComputedStyle(outside).color;
})()
"#,
        )
        .expect("detached style has selector setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let mutated = vm
        .eval("globalThis.__detachedHasTarget.setAttribute('data-state', 'active'); 'mutated'")
        .expect("detached style has selector attribute mutation should evaluate");

    assert_eq!(mutated, "mutated");
    vm._context_host
        .borrow()
        .drain_pending_style_invalidations_for_document_for_test(document);
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let after = vm
        .eval("getComputedStyle(globalThis.__detachedHasOutside).color")
        .expect("detached style has selector cache read should evaluate");

    assert_eq!(after, "rgb(1, 2, 3)");
}
#[test]
fn character_data_invalidation_preserves_unaffected_subtree_cache() {
    let mut vm = new_storage_test_vm("https://character-data-style-targeted-cache.test/");
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
    '#outside { color: rgb(1, 2, 3); }',
    '#target { color: rgb(4, 5, 6); }',
    '#later { color: rgb(7, 8, 9); }',
  ].join('\n');
  head.appendChild(style);

  const outsideRoot = document.createElement('section');
  const outside = document.createElement('div');
  outside.id = 'outside';
  outsideRoot.appendChild(outside);

  const mutationRoot = document.createElement('section');
  const target = document.createElement('div');
  target.id = 'target';
  const text = document.createTextNode('before');
  target.appendChild(text);
  const later = document.createElement('div');
  later.id = 'later';
  mutationRoot.append(target, later);
  body.append(outsideRoot, mutationRoot);

  globalThis.__characterDataText = text;
  globalThis.__characterDataOutside = outside;
  globalThis.__characterDataTarget = target;
  globalThis.__characterDataLater = later;
  return [
    getComputedStyle(outside).color,
    getComputedStyle(target).color,
    getComputedStyle(later).color,
  ].join('|');
})()
"#,
        )
        .expect("character data targeted cache setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)|rgb(4, 5, 6)|rgb(7, 8, 9)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        3
    );

    let mutated = vm
        .eval("globalThis.__characterDataText.data = 'after'; 'mutated'")
        .expect("character data targeted cache mutation should evaluate");

    assert_eq!(mutated, "mutated");
    vm._context_host
        .borrow()
        .drain_pending_style_invalidations_for_document_for_test(document);
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        3
    );

    let after = vm
        .eval(
            r#"
[
  getComputedStyle(globalThis.__characterDataOutside).color,
  getComputedStyle(globalThis.__characterDataTarget).color,
  getComputedStyle(globalThis.__characterDataLater).color,
].join('|')
"#,
        )
        .expect("character data targeted cache read should evaluate");

    assert_eq!(after, "rgb(1, 2, 3)|rgb(4, 5, 6)|rgb(7, 8, 9)");
}
#[test]
fn unrelated_sibling_combinator_character_data_invalidation_preserves_later_cache() {
    let mut vm = new_storage_test_vm("https://character-data-sibling-cache.test/");
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
    '#outside { color: rgb(1, 2, 3); }',
    '#previous { color: rgb(2, 3, 4); }',
    '#target { color: rgb(4, 5, 6); }',
    '#target + #later { color: rgb(7, 8, 9); }',
  ].join('\n');
  head.appendChild(style);

  const outsideRoot = document.createElement('section');
  const outside = document.createElement('div');
  outside.id = 'outside';
  outsideRoot.appendChild(outside);

  const mutationRoot = document.createElement('section');
  const previous = document.createElement('div');
  previous.id = 'previous';
  const target = document.createElement('div');
  target.id = 'target';
  const text = document.createTextNode('before');
  target.appendChild(text);
  const later = document.createElement('div');
  later.id = 'later';
  mutationRoot.append(previous, target, later);
  body.append(outsideRoot, mutationRoot);

  globalThis.__characterDataSiblingText = text;
  globalThis.__characterDataSiblingOutside = outside;
  globalThis.__characterDataSiblingPrevious = previous;
  globalThis.__characterDataSiblingTarget = target;
  globalThis.__characterDataSiblingLater = later;
  return [
    getComputedStyle(outside).color,
    getComputedStyle(previous).color,
    getComputedStyle(target).color,
    getComputedStyle(later).color,
  ].join('|');
})()
"#,
        )
        .expect("character data sibling cache setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)|rgb(2, 3, 4)|rgb(4, 5, 6)|rgb(7, 8, 9)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        4
    );

    let mutated = vm
        .eval("globalThis.__characterDataSiblingText.data = 'after'; 'mutated'")
        .expect("character data sibling cache mutation should evaluate");

    assert_eq!(mutated, "mutated");
    vm._context_host
        .borrow()
        .drain_pending_style_invalidations_for_document_for_test(document);
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        4
    );

    let after = vm
        .eval(
            r#"
[
  getComputedStyle(globalThis.__characterDataSiblingOutside).color,
  getComputedStyle(globalThis.__characterDataSiblingPrevious).color,
  getComputedStyle(globalThis.__characterDataSiblingTarget).color,
  getComputedStyle(globalThis.__characterDataSiblingLater).color,
].join('|')
"#,
        )
        .expect("character data sibling cache read should evaluate");

    assert_eq!(after, "rgb(1, 2, 3)|rgb(2, 3, 4)|rgb(4, 5, 6)|rgb(7, 8, 9)");
}
#[test]
fn structural_sibling_character_data_invalidation_refreshes_later_cache() {
    let mut vm = new_storage_test_vm("https://character-data-structural-sibling-cache.test/");
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
    '#outside { color: rgb(1, 2, 3); }',
    '#target { color: rgb(4, 5, 6); }',
    '#later { color: rgb(7, 8, 9); }',
    '#target:empty + #later { color: rgb(9, 8, 7); }',
  ].join('\n');
  head.appendChild(style);

  const outside = document.createElement('div');
  outside.id = 'outside';
  const target = document.createElement('div');
  target.id = 'target';
  const text = document.createTextNode('');
  target.appendChild(text);
  const later = document.createElement('div');
  later.id = 'later';
  body.append(outside, target, later);

  globalThis.__structuralCharacterDataText = text;
  globalThis.__structuralCharacterDataOutside = outside;
  globalThis.__structuralCharacterDataTarget = target;
  globalThis.__structuralCharacterDataLater = later;
  return [
    getComputedStyle(outside).color,
    getComputedStyle(target).color,
    getComputedStyle(later).color,
  ].join('|');
})()
"#,
        )
        .expect("structural character data sibling setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)|rgb(4, 5, 6)|rgb(9, 8, 7)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        3
    );

    let mutated = vm
        .eval("globalThis.__structuralCharacterDataText.data = 'after'; 'mutated'")
        .expect("structural character data sibling mutation should evaluate");

    assert_eq!(mutated, "mutated");
    vm._context_host
        .borrow()
        .drain_pending_style_invalidations_for_document_for_test(document);
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let after = vm
        .eval(
            r#"
[
  getComputedStyle(globalThis.__structuralCharacterDataOutside).color,
  getComputedStyle(globalThis.__structuralCharacterDataTarget).color,
  getComputedStyle(globalThis.__structuralCharacterDataLater).color,
].join('|')
"#,
        )
        .expect("structural character data sibling read should evaluate");

    assert_eq!(after, "rgb(1, 2, 3)|rgb(4, 5, 6)|rgb(7, 8, 9)");
}
#[test]
fn has_selector_character_data_mutation_invalidates_ancestor_dependent_styles() {
    let mut vm = new_storage_test_vm("https://has-character-data-style-cache.test/");
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
    '#outside { color: rgb(1, 2, 3); }',
    'body:has(#target) #outside { background-color: rgb(4, 5, 6); }',
  ].join('\n');
  head.appendChild(style);

  const outside = document.createElement('div');
  outside.id = 'outside';
  const target = document.createElement('div');
  target.id = 'target';
  const text = document.createTextNode('before');
  target.appendChild(text);
  body.append(outside, target);

  globalThis.__hasCharacterDataText = text;
  globalThis.__hasCharacterDataOutside = outside;
  return getComputedStyle(outside).color;
})()
"#,
        )
        .expect("has selector character data setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let mutated = vm
        .eval("globalThis.__hasCharacterDataText.data = 'after'; 'mutated'")
        .expect("has selector character data mutation should evaluate");

    assert_eq!(mutated, "mutated");
    vm._context_host
        .borrow()
        .drain_pending_style_invalidations_for_document_for_test(document);
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
}
#[test]
fn manual_slot_assignment_invalidates_host_subtree_style_cache() {
    let mut vm = new_storage_test_vm("https://manual-slot-style-targeted-cache.test/");
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
    '#outside { color: rgb(1, 2, 3); }',
    '#assigned { color: rgb(4, 5, 6); }',
  ].join('\n');
  head.appendChild(style);

  const outsideRoot = document.createElement('section');
  const outside = document.createElement('div');
  outside.id = 'outside';
  outsideRoot.appendChild(outside);

  const host = document.createElement('div');
  const assigned = document.createElement('span');
  assigned.id = 'assigned';
  host.appendChild(assigned);
  const shadow = host.attachShadow({ mode: 'open', slotAssignment: 'manual' });
  const slotA = document.createElement('slot');
  const slotB = document.createElement('slot');
  shadow.append(slotA, slotB);
  slotA.assign(assigned);
  body.append(outsideRoot, host);

  globalThis.__manualSlotAssigned = assigned;
  globalThis.__manualSlotB = slotB;
  globalThis.__manualSlotOutside = outside;
  return [
    getComputedStyle(outside).color,
    getComputedStyle(assigned).color,
  ].join('|');
})()
"#,
        )
        .expect("manual slot targeted cache setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)|rgb(4, 5, 6)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );

    let reassigned = vm
        .eval("globalThis.__manualSlotB.assign(globalThis.__manualSlotAssigned); 'assigned'")
        .expect("manual slot reassignment should evaluate");

    assert_eq!(reassigned, "assigned");
    vm._context_host
        .borrow()
        .drain_pending_style_invalidations_for_document_for_test(document);
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let after = vm
        .eval(
            r#"
[
  getComputedStyle(globalThis.__manualSlotOutside).color,
  getComputedStyle(globalThis.__manualSlotAssigned).color,
].join('|')
"#,
        )
        .expect("manual slot targeted cache read should evaluate");

    assert_eq!(after, "rgb(1, 2, 3)|rgb(4, 5, 6)");
}
#[test]
fn adopted_stylesheet_internal_slots_are_not_script_writable() {
    let mut vm = new_storage_test_vm("https://adopted-constructed-sheet-private-slots.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const target = body.appendChild(document.createElement('div'));
  target.id = 'target';

  const first = new CSSStyleSheet();
  first.replaceSync('#target { color: rgb(1, 2, 3); }');
  const second = new CSSStyleSheet();
  second.replaceSync('#target { color: rgb(4, 5, 6); }');
  document.adoptedStyleSheets = [first];
  const array = document.adoptedStyleSheets;
  const documentSlotVisible = Object.prototype.hasOwnProperty.call(
    document,
    '__moliAdoptedStyleSheets'
  );
  const ownerKindVisible = Object.prototype.hasOwnProperty.call(
    array,
    '__moliAdoptedStyleSheetsArrayOwnerKind'
  );

  array.__moliAdoptedStyleSheetsArrayOwnerKind = 2;
  array.__moliAdoptedStyleSheetsArrayOwnerHandle = 999999;
  array.__moliAdoptedStyleSheetsArrayStyleDocumentHandle = 999999;
  array.push(second);
  const afterOwnedMutation = getComputedStyle(target).color;

  const borrowed = array.push;
  borrowed.call([first], second);
  const afterBorrowedDocumentMutation = getComputedStyle(target).color;

  const host = body.appendChild(document.createElement('section'));
  const shadow = host.attachShadow({ mode: 'open' });
  const shadowTarget = shadow.appendChild(document.createElement('span'));
  const shadowFirst = new CSSStyleSheet();
  shadowFirst.replaceSync('span { color: rgb(7, 8, 9); }');
  const shadowSecond = new CSSStyleSheet();
  shadowSecond.replaceSync('span { color: rgb(10, 11, 12); }');
  shadow.adoptedStyleSheets = [shadowFirst];
  const shadowArray = shadow.adoptedStyleSheets;
  const shadowSlotVisible = Object.prototype.hasOwnProperty.call(
    shadow,
    '__moliAdoptedStyleSheets'
  );
  shadowArray.__moliAdoptedStyleSheetsArrayOwnerKind = 1;
  shadowArray.__moliAdoptedStyleSheetsArrayOwnerHandle = 999999;
  shadowArray.push(shadowSecond);

  return [
    documentSlotVisible,
    ownerKindVisible,
    afterOwnedMutation,
    afterBorrowedDocumentMutation,
    shadowSlotVisible,
    getComputedStyle(shadowTarget).color
  ].join('|');
})()
"#,
        )
        .expect("adopted stylesheet internal slots should stay private");

    assert_eq!(
        result,
        "false|false|rgb(4, 5, 6)|rgb(4, 5, 6)|false|rgb(10, 11, 12)"
    );
}

#[test]
fn adopted_stylesheets_reject_untracked_non_array_assignments() {
    let mut vm = new_storage_test_vm("https://adopted-non-array-assignment.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      callback();
      return 'ok';
    } catch (error) {
      return error && error.name;
    }
  };
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const target = body.appendChild(document.createElement('div'));
  target.id = 'target';

  const documentSheet = new CSSStyleSheet();
  documentSheet.replaceSync('#target { color: rgb(1, 2, 3); }');
  document.adoptedStyleSheets = [documentSheet];
  const beforeDocument = getComputedStyle(target).color;
  const documentObject = { 0: new CSSStyleSheet(), length: 1 };
  documentObject[0].replaceSync('#target { color: rgb(4, 5, 6); }');
  const documentAssign = probe(() => { document.adoptedStyleSheets = documentObject; });
  const afterDocument = getComputedStyle(target).color;

  const host = body.appendChild(document.createElement('section'));
  const shadow = host.attachShadow({ mode: 'open' });
  const shadowTarget = shadow.appendChild(document.createElement('span'));
  const shadowSheet = new CSSStyleSheet();
  shadowSheet.replaceSync('span { color: rgb(7, 8, 9); }');
  shadow.adoptedStyleSheets = [shadowSheet];
  const beforeShadow = getComputedStyle(shadowTarget).color;
  const shadowObject = { 0: new CSSStyleSheet(), length: 1 };
  shadowObject[0].replaceSync('span { color: rgb(10, 11, 12); }');
  const shadowAssign = probe(() => { shadow.adoptedStyleSheets = shadowObject; });
  const afterShadow = getComputedStyle(shadowTarget).color;

  const detachedDocument = new DOMParser().parseFromString('<section></section>', 'text/html');
  const detachedHost = detachedDocument.querySelector('section');
  const detachedShadow = detachedHost.attachShadow({ mode: 'open' });
  const detachedObject = { 0: new CSSStyleSheet(), length: 1 };
  const detachedAssign = probe(() => { detachedShadow.adoptedStyleSheets = detachedObject; });

  return [
    documentAssign,
    beforeDocument,
    afterDocument,
    Array.isArray(document.adoptedStyleSheets),
    document.adoptedStyleSheets.length,
    shadowAssign,
    beforeShadow,
    afterShadow,
    Array.isArray(shadow.adoptedStyleSheets),
    shadow.adoptedStyleSheets.length,
    detachedAssign,
    Array.isArray(detachedShadow.adoptedStyleSheets),
    detachedShadow.adoptedStyleSheets.length
  ].join('|');
})()
"#,
        )
        .expect("adoptedStyleSheets non-array assignment probe should evaluate");

    assert_eq!(
        result,
        concat!(
            "TypeError|rgb(1, 2, 3)|rgb(1, 2, 3)|true|1|",
            "TypeError|rgb(7, 8, 9)|rgb(7, 8, 9)|true|1|",
            "TypeError|true|0"
        )
    );
}

#[test]
fn css_stylesheet_rules_slot_is_not_script_writable() {
    let mut vm = new_storage_test_vm("https://css-stylesheet-rules-private-slot.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const target = body.appendChild(document.createElement('div'));
  target.id = 'target';

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('#target { color: rgb(1, 2, 3); }');
  document.adoptedStyleSheets = [sheet];
  const style = getComputedStyle(target);
  const publicSlotBefore = Object.prototype.hasOwnProperty.call(
    sheet,
    '__moliCssStyleSheetRules'
  );
  const before = `${sheet.cssRules.length}:${style.color}`;

  sheet.__moliCssStyleSheetRules = { length: 0 };
  const afterForge = `${sheet.cssRules.length}:${style.color}`;

  sheet.insertRule(
    '#target { color: rgb(4, 5, 6); }',
    sheet.cssRules.length
  );
  const afterInsert = `${sheet.cssRules.length}:${style.color}`;
  sheet.deleteRule(1);
  const afterDelete = `${sheet.cssRules.length}:${style.color}`;

  return [
    publicSlotBefore,
    before,
    afterForge,
    afterInsert,
    afterDelete
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleSheet rules backing store should ignore script-visible slots");

    assert_eq!(
        result,
        "false|1:rgb(1, 2, 3)|1:rgb(1, 2, 3)|2:rgb(4, 5, 6)|1:rgb(1, 2, 3)"
    );
}
#[test]
fn css_stylesheet_owner_node_shadowing_does_not_break_source_sync() {
    let mut vm = new_storage_test_vm("https://css-stylesheet-owner-node-private-slot.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const target = body.appendChild(document.createElement('div'));
  target.id = 'target';

  const style = document.createElement('style');
  style.textContent = '#target { color: rgb(1, 2, 3); }';
  head.appendChild(style);
  const computed = getComputedStyle(target);
  const before = computed.color;

  Object.defineProperty(style.sheet, 'ownerNode', {
    value: null,
    configurable: true
  });
  style.sheet.cssRules[0].style.setProperty('color', 'rgb(4, 5, 6)');

  return [
    before,
    style.sheet.ownerNode === null,
    computed.color
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleSheet ownerNode shadowing should not break source sync");

    assert_eq!(result, "rgb(1, 2, 3)|true|rgb(4, 5, 6)");
}
#[test]
fn adopted_stylesheets_array_mutations_validate_and_resync() {
    let mut vm = new_storage_test_vm("https://css-adopted-array-mutations.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      callback();
      return 'ok';
    } catch (error) {
      return error && error.name;
    }
  };
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const target = document.createElement('span');
  target.id = 'target';
  const style = document.createElement('style');
  style.textContent = '#target { background-color: red; }';
  root.append(style, target);
  const sheet1 = new CSSStyleSheet();
  sheet1.replaceSync('#target { background-color: lime !important; }');
  const sheet2 = new CSSStyleSheet();
  sheet2.replaceSync('#target { background-color: blue !important; }');
  const color = () => getComputedStyle(target).backgroundColor;

  const initial = color();
  document.adoptedStyleSheets = [sheet1];
  const assigned = [document.adoptedStyleSheets.length, color()].join(',');
  const intrinsicArray = Array;
  const intrinsicPush = intrinsicArray.prototype.push;
  intrinsicArray.prototype.push = () => {
    throw new Error('adoptedStyleSheets must use captured Array.prototype.push');
  };
  globalThis.Array = null;
  document.adoptedStyleSheets.push(sheet2);
  const pushed = [document.adoptedStyleSheets.length, color()].join(',');
  document.adoptedStyleSheets.reverse();
  const reversed = [document.adoptedStyleSheets.length, color()].join(',');
  document.adoptedStyleSheets.splice(1, 1);
  const spliced = [document.adoptedStyleSheets.length, color()].join(',');
  const nonConstructed = probe(() => document.adoptedStyleSheets.push(style.sheet));
  const nonSheet = probe(() => document.adoptedStyleSheets.push('foo'));
  globalThis.Array = intrinsicArray;
  intrinsicArray.prototype.push = intrinsicPush;
  return [
    initial,
    assigned,
    pushed,
    reversed,
    spliced,
    nonConstructed,
    nonSheet,
    document.adoptedStyleSheets.length,
    color()
  ].join('|');
})()
"#,
        )
        .expect("adoptedStyleSheets array mutations should evaluate");

    assert_eq!(
        result,
        concat!(
            "rgb(255, 0, 0)|",
            "1,rgb(0, 255, 0)|",
            "2,rgb(0, 0, 255)|",
            "2,rgb(0, 255, 0)|",
            "1,rgb(0, 0, 255)|",
            "NotAllowedError|TypeError|1|rgb(0, 0, 255)"
        )
    );
}

#[test]
fn duplicate_constructed_adopted_stylesheets_reconcile_document_and_shadow_cascade_order() {
    let mut vm = new_storage_test_vm("https://duplicate-adopted-cascade.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const target = body.appendChild(document.createElement('div'));
  const host = body.appendChild(document.createElement('section'));
  const shadow = host.attachShadow({ mode: 'open' });
  const shadowTarget = shadow.appendChild(document.createElement('div'));

  const makeSheets = () => {
    const zero = new CSSStyleSheet();
    zero.replaceSync('div { z-index: 0; color: blue; }');
    const one = new CSSStyleSheet();
    one.replaceSync('div { z-index: 1; color: blue; }');
    const red = new CSSStyleSheet();
    red.replaceSync('div { color: red; }');
    return { zero, one, red };
  };
  const read = element => {
    const style = getComputedStyle(element);
    return `${style.zIndex}:${style.color}`;
  };

  const doc = makeSheets();
  document.adoptedStyleSheets = [doc.one, doc.zero, doc.one];
  const documentDuplicate = read(target);
  document.adoptedStyleSheets = [];
  const documentEmpty = read(target);
  document.adoptedStyleSheets = [doc.one, doc.zero];
  const documentUnique = read(target);
  document.adoptedStyleSheets = [doc.one, doc.zero, doc.one, doc.red];
  const documentExpanded = read(target);
  document.adoptedStyleSheets = [];
  const documentRemoved = read(target);

  const scoped = makeSheets();
  shadow.adoptedStyleSheets = [scoped.one, scoped.zero, scoped.one];
  const shadowDuplicate = read(shadowTarget);
  shadow.adoptedStyleSheets = [];
  const shadowEmpty = read(shadowTarget);
  shadow.adoptedStyleSheets = [scoped.one, scoped.zero];
  const shadowUnique = read(shadowTarget);
  shadow.adoptedStyleSheets = [scoped.one, scoped.zero, scoped.one, scoped.red];
  const shadowExpanded = read(shadowTarget);
  shadow.adoptedStyleSheets = [];
  const shadowRemoved = read(shadowTarget);

  return [
    documentDuplicate,
    documentEmpty,
    documentUnique,
    documentExpanded,
    documentRemoved,
    shadowDuplicate,
    shadowEmpty,
    shadowUnique,
    shadowExpanded,
    shadowRemoved,
  ].join('|');
})()
"#,
        )
        .expect("duplicate adopted stylesheet cascades should reconcile without panicking");

    assert_eq!(
        result,
        concat!(
            "1:rgb(0, 0, 255)|auto:rgb(0, 0, 0)|",
            "0:rgb(0, 0, 255)|1:rgb(255, 0, 0)|auto:rgb(0, 0, 0)|",
            "1:rgb(0, 0, 255)|auto:rgb(0, 0, 0)|",
            "0:rgb(0, 0, 255)|1:rgb(255, 0, 0)|auto:rgb(0, 0, 0)"
        )
    );
}

#[test]
fn fragment_change_without_target_selectors_preserves_style_generation() {
    let mut vm = new_parsed_test_vm(
        "https://fragment-style-cache-no-target-selector.test/#old",
        r#"
        <html>
          <head>
            <style>
              .probe { color: rgb(1, 2, 3); }
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
  globalThis.__fragmentNoTargetOldStyle = getComputedStyle(oldTarget);
  globalThis.__fragmentNoTargetNewStyle = getComputedStyle(newTarget);
  return [
    oldTarget.matches(':target'),
    newTarget.matches(':target'),
    globalThis.__fragmentNoTargetOldStyle.color,
    globalThis.__fragmentNoTargetNewStyle.color
  ].join('|');
})()
"#,
        )
        .expect("fragment no-target selector setup should evaluate");

    assert_eq!(setup, "true|false|rgb(1, 2, 3)|rgb(1, 2, 3)");
    let generation_before_fragment =
        vm.computed_style_cache_generation_for_document_for_test(document);
    let rebuilds_before_fragment =
        vm.retained_style_system_rebuild_count_for_document_for_test(document);

    let replaced = vm
        .eval(
            r#"
(() => {
  const oldTarget = document.getElementById('old');
  const newTarget = document.getElementById('new');
  history.replaceState(null, '', '#new');
  const result = [
    oldTarget.matches(':target'),
    newTarget.matches(':target'),
    globalThis.__fragmentNoTargetOldStyle.color,
    globalThis.__fragmentNoTargetNewStyle.color
  ].join('|');
  delete globalThis.__fragmentNoTargetOldStyle;
  delete globalThis.__fragmentNoTargetNewStyle;
  return result;
})()
"#,
        )
        .expect("fragment no-target selector mutation should evaluate");

    assert_eq!(replaced, "false|true|rgb(1, 2, 3)|rgb(1, 2, 3)");
    assert_eq!(
        vm.computed_style_cache_generation_for_document_for_test(document),
        generation_before_fragment,
        "fragment-only changes without author :target selectors should not clear style cache"
    );
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(document),
        rebuilds_before_fragment,
        "fragment-only changes should not rebuild Stylo when the style key is otherwise stable"
    );
}
#[test]
fn target_selector_invalidation_preserves_unrelated_cache_entries() {
    let mut vm = new_parsed_test_vm(
        "https://target-style-cache-targeted.test/#old",
        r#"
        <html>
          <head>
            <style>
              #outside { color: rgb(1, 2, 3); }
              #inside { color: rgb(4, 5, 6); }
              #old:target #inside { color: rgb(7, 8, 9); }
            </style>
          </head>
          <body>
            <div id="outside"></div>
            <div id="old"><span id="inside"></span></div>
            <div id="new"></div>
          </body>
        </html>
        "#,
    );
    let document = vm.document_handle_for_test();

    let setup = vm
        .eval(
            r#"
(() => {
  const outside = document.getElementById('outside');
  const inside = document.getElementById('inside');
  globalThis.__targetTargetedInsideStyle = getComputedStyle(inside);
  return [
    getComputedStyle(outside).color,
    globalThis.__targetTargetedInsideStyle.color
  ].join('|');
})()
"#,
        )
        .expect("target targeted style setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)|rgb(7, 8, 9)");
    let generation_before_fragment =
        vm.computed_style_cache_generation_for_document_for_test(document);
    let cache_count_before_fragment =
        vm.computed_style_cache_entry_count_for_document_for_test(document);
    assert!(cache_count_before_fragment >= 2);

    let replaced = vm
        .eval(
            r#"
(() => {
  history.replaceState(null, '', '#new');
  return globalThis.__targetTargetedInsideStyle.color;
})()
"#,
        )
        .expect("target targeted fragment mutation should evaluate");

    assert_eq!(replaced, "rgb(4, 5, 6)");
    assert_eq!(
        vm.computed_style_cache_generation_for_document_for_test(document),
        generation_before_fragment,
        "targeted :target invalidation should preserve the retained style generation"
    );
    let cache_count_after_fragment =
        vm.computed_style_cache_entry_count_for_document_for_test(document);
    assert!(
        cache_count_after_fragment > 0,
        ":target selector without sibling/:has should preserve unrelated cache entries"
    );

    let cleanup = vm
        .eval(
            r#"
(() => {
  delete globalThis.__targetTargetedInsideStyle;
  return 'ok';
})()
"#,
        )
        .expect("target targeted cleanup should evaluate");

    assert_eq!(cleanup, "ok");
}
#[test]
fn has_target_selector_fragment_change_retains_style_system() {
    let mut vm = new_parsed_test_vm(
        "https://has-target-style-cache-targeted.test/#old",
        r#"
        <html>
          <head>
            <style>
              #outside { color: rgb(1, 2, 3); }
              body:has(#old:target) #outside { color: rgb(7, 8, 9); }
            </style>
          </head>
          <body>
            <div id="outside"></div>
            <div id="old"></div>
            <div id="new"></div>
          </body>
        </html>
        "#,
    );
    let document = vm.document_handle_for_test();

    let setup = vm
        .eval(
            r#"
(() => {
  const outside = document.getElementById('outside');
  globalThis.__hasTargetOutsideStyle = getComputedStyle(outside);
  return globalThis.__hasTargetOutsideStyle.color;
})()
"#,
        )
        .expect("has target selector setup should evaluate");

    assert_eq!(setup, "rgb(7, 8, 9)");
    let generation_before_fragment =
        vm.computed_style_cache_generation_for_document_for_test(document);

    let replaced = vm
        .eval(
            r#"
(() => {
  history.replaceState(null, '', '#new');
  const result = globalThis.__hasTargetOutsideStyle.color;
  delete globalThis.__hasTargetOutsideStyle;
  return result;
})()
"#,
        )
        .expect("has target selector mutation should evaluate");

    assert_eq!(replaced, "rgb(1, 2, 3)");
    assert_eq!(
        vm.computed_style_cache_generation_for_document_for_test(document),
        generation_before_fragment,
        ":has(:target) invalidation should preserve the retained style generation"
    );
}
#[test]
fn child_list_style_invalidation_preserves_unaffected_computed_cache_entries() {
    let mut vm = new_storage_test_vm("https://targeted-child-list-style-cache.test/");
    let document = vm.document_handle_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '#outside { color: rgb(1, 2, 3); } #removed { color: rgb(4, 5, 6); }';
  head.appendChild(style);

  const outside = document.createElement('div');
  outside.id = 'outside';
  const container = document.createElement('section');
  const removed = document.createElement('span');
  removed.id = 'removed';
  container.appendChild(removed);
  body.append(outside, container);
  globalThis.__targetedStyleRemoved = removed;
  return [getComputedStyle(outside).color, getComputedStyle(removed).color].join('|');
})()
"#,
        )
        .expect("targeted child-list style cache setup should evaluate");

    assert_eq!(result, "rgb(1, 2, 3)|rgb(4, 5, 6)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );

    let remove_result = vm
        .eval(
            r#"
(() => {
  globalThis.__targetedStyleRemoved.remove();
  delete globalThis.__targetedStyleRemoved;
  return 'removed';
})()
"#,
        )
        .expect("targeted child-list style cache removal should evaluate");

    assert_eq!(remove_result, "removed");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
}
#[test]
fn child_list_style_invalidation_ignores_unrelated_mutation_observer_state() {
    let mut vm = new_storage_test_vm("https://targeted-child-list-observer-cache.test/");
    let document = vm.document_handle_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '#outside { color: rgb(1, 2, 3); } #removed { color: rgb(4, 5, 6); }';
  head.appendChild(style);

  const outsideRoot = document.createElement('section');
  const outside = document.createElement('div');
  outside.id = 'outside';
  outsideRoot.appendChild(outside);

  const container = document.createElement('section');
  const removed = document.createElement('span');
  removed.id = 'removed';
  container.appendChild(removed);
  body.append(outsideRoot, container);

  const observer = new MutationObserver(() => {});
  observer.observe(body, { childList: true, subtree: true });

  globalThis.__targetedObserver = observer;
  globalThis.__targetedObserverRemoved = removed;
  return [getComputedStyle(outside).color, getComputedStyle(removed).color].join('|');
})()
"#,
        )
        .expect("targeted child-list observer cache setup should evaluate");

    assert_eq!(result, "rgb(1, 2, 3)|rgb(4, 5, 6)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );

    let remove_result = vm
        .eval(
            r#"
(() => {
  globalThis.__targetedObserverRemoved.remove();
  delete globalThis.__targetedObserverRemoved;
  return 'removed';
})()
"#,
        )
        .expect("targeted child-list observer removal should evaluate");

    assert_eq!(remove_result, "removed");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
}
#[test]
fn non_structural_child_list_invalidation_preserves_parent_descendant_cache() {
    let mut vm = new_storage_test_vm("https://non-structural-child-list-cache.test/");
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
    '#outside { color: rgb(1, 2, 3); }',
    '#container { color: rgb(4, 5, 6); }',
    '#kept { color: rgb(7, 8, 9); }',
  ].join('\n');
  head.appendChild(style);

  const outside = document.createElement('div');
  outside.id = 'outside';
  const container = document.createElement('section');
  container.id = 'container';
  const kept = document.createElement('span');
  kept.id = 'kept';
  container.appendChild(kept);
  body.append(outside, container);

  globalThis.__nonStructuralContainer = container;
  globalThis.__nonStructuralOutside = outside;
  globalThis.__nonStructuralKept = kept;
  return [
    getComputedStyle(outside).color,
    getComputedStyle(container).color,
    getComputedStyle(kept).color,
  ].join('|');
})()
"#,
        )
        .expect("non-structural child-list cache setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)|rgb(4, 5, 6)|rgb(7, 8, 9)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        3
    );

    let appended = vm
        .eval(
            r#"
(() => {
  globalThis.__nonStructuralContainer.appendChild(document.createElement('em'));
  return 'appended';
})()
"#,
        )
        .expect("non-structural child-list append should evaluate");

    assert_eq!(appended, "appended");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        3
    );

    let after = vm
        .eval(
            r#"
[
  getComputedStyle(globalThis.__nonStructuralOutside).color,
  getComputedStyle(globalThis.__nonStructuralContainer).color,
  getComputedStyle(globalThis.__nonStructuralKept).color,
].join('|')
"#,
        )
        .expect("non-structural child-list cached values should evaluate");

    assert_eq!(after, "rgb(1, 2, 3)|rgb(4, 5, 6)|rgb(7, 8, 9)");
}
#[test]
fn sibling_combinator_child_list_invalidation_refreshes_existing_sibling() {
    let mut vm = new_storage_test_vm("https://sibling-child-list-cache.test/");
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
    '#outside { color: rgb(1, 2, 3); }',
    '#previous { color: rgb(2, 3, 4); }',
    '#second { color: rgb(255, 0, 0); }',
    '#inserted + #second { color: rgb(0, 128, 0); }',
  ].join('\n');
  head.appendChild(style);

  const outside = document.createElement('div');
  outside.id = 'outside';
  const container = document.createElement('section');
  const previous = document.createElement('span');
  previous.id = 'previous';
  const second = document.createElement('span');
  second.id = 'second';
  container.append(previous, second);
  body.append(outside, container);

  globalThis.__siblingContainer = container;
  globalThis.__siblingOutside = outside;
  globalThis.__siblingPrevious = previous;
  globalThis.__siblingSecond = second;
  globalThis.__siblingHeldStyle = getComputedStyle(second);
  return [
    getComputedStyle(outside).color,
    getComputedStyle(previous).color,
    globalThis.__siblingHeldStyle.color,
  ].join('|');
})()
"#,
        )
        .expect("sibling combinator child-list setup should evaluate");

    assert_eq!(setup, "rgb(1, 2, 3)|rgb(2, 3, 4)|rgb(255, 0, 0)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        3
    );

    let inserted = vm
        .eval(
            r#"
(() => {
  const inserted = document.createElement('span');
  inserted.id = 'inserted';
  globalThis.__siblingContainer.insertBefore(inserted, globalThis.__siblingSecond);
  return 'inserted';
})()
"#,
        )
        .expect("sibling combinator child-list mutation should evaluate");

    assert_eq!(inserted, "inserted");
    vm._context_host
        .borrow()
        .drain_pending_style_invalidations_for_document_for_test(document);
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );

    let after = vm
        .eval(
            r#"
[
  getComputedStyle(globalThis.__siblingOutside).color,
  getComputedStyle(globalThis.__siblingPrevious).color,
  globalThis.__siblingHeldStyle.color,
].join('|')
"#,
        )
        .expect("sibling combinator child-list cached read should evaluate");

    assert_eq!(after, "rgb(1, 2, 3)|rgb(2, 3, 4)|rgb(0, 128, 0)");
}

#[test]
fn has_side_effect_child_list_move_refreshes_previous_sibling_subjects() {
    let mut vm = new_storage_test_vm("https://has-side-effect-child-list-cache.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = [
    'div, main { color: grey }',
    '#subject:has(+ #next_sibling) { color: red; }',
    '#prev_sibling:has(+ #subject + #next_sibling) { color: green; }',
  ].join('\n');
  head.appendChild(style);

  const main = document.createElement('main');
  const prev = document.createElement('div');
  prev.id = 'prev_sibling';
  const subject = document.createElement('div');
  subject.id = 'subject';
  const blocker = document.createElement('div');
  blocker.id = 'blocks_match';
  const next = document.createElement('div');
  next.id = 'next_sibling';
  main.append(prev, subject, blocker, next);
  body.append(main);

  const before = [
    getComputedStyle(subject).color,
    getComputedStyle(prev).color,
  ].join('|');
  blocker.remove();
  const afterRemove = [
    getComputedStyle(subject).color,
    getComputedStyle(prev).color,
  ].join('|');
  subject.after(blocker);
  const afterAdd = [
    getComputedStyle(subject).color,
    getComputedStyle(prev).color,
  ].join('|');
  return `${before};${afterRemove};${afterAdd}`;
})()
"#,
        )
        .expect(":has side-effect child-list move probe should evaluate");

    assert_eq!(
        result,
        "rgb(128, 128, 128)|rgb(128, 128, 128);rgb(255, 0, 0)|rgb(0, 128, 0);rgb(128, 128, 128)|rgb(128, 128, 128)"
    );
}

#[test]
fn parsed_has_side_effect_child_list_move_refreshes_styles() {
    let mut vm = new_parsed_test_vm(
        "https://has-side-effect-parsed.test/",
        r#"
<!DOCTYPE html>
<style>
div, main { color: grey }
#subject:has(+ #next_sibling) { color: red; }
#prev_sibling:has(+ #subject + #next_sibling) { color: green; }
</style>
<main id=main>
  <div id=prev_sibling></div>
  <div id=subject></div>
  <div id=blocks_match></div>
  <div id=next_sibling></div>
</main>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const before = [
    getComputedStyle(subject).color,
    getComputedStyle(prev_sibling).color,
  ].join('|');
  const d = blocks_match;
  d.remove();
  const afterRemove = [
    getComputedStyle(subject).color,
    getComputedStyle(prev_sibling).color,
  ].join('|');
  subject.after(d);
  const afterAdd = [
    getComputedStyle(subject).color,
    getComputedStyle(prev_sibling).color,
  ].join('|');
  return `${before};${afterRemove};${afterAdd}`;
})()
"#,
        )
        .expect("parsed :has side-effect child-list move probe should evaluate");

    assert_eq!(
        result,
        "rgb(128, 128, 128)|rgb(128, 128, 128);rgb(255, 0, 0)|rgb(0, 128, 0);rgb(128, 128, 128)|rgb(128, 128, 128)"
    );
}

#[test]
fn targeted_child_list_style_invalidation_retains_stylist() {
    let mut vm = new_storage_test_vm("https://targeted-child-list-retained-stylist.test/");
    let document = vm.document_handle_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '#outside { color: rgb(1, 2, 3); } #removed { color: rgb(4, 5, 6); }';
  head.appendChild(style);

  const outside = document.createElement('div');
  outside.id = 'outside';
  const container = document.createElement('section');
  const removed = document.createElement('span');
  removed.id = 'removed';
  container.appendChild(removed);
  body.append(outside, container);
  globalThis.__retainedStylistOutside = outside;
  globalThis.__retainedStylistRemoved = removed;
  return [getComputedStyle(outside).color, getComputedStyle(removed).color].join('|');
})()
"#,
        )
        .expect("retained Stylist setup should evaluate");

    assert_eq!(result, "rgb(1, 2, 3)|rgb(4, 5, 6)");
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(document),
        1
    );

    let remove_result = vm
        .eval(
            r#"
(() => {
  globalThis.__retainedStylistRemoved.remove();
  delete globalThis.__retainedStylistRemoved;
  return getComputedStyle(globalThis.__retainedStylistOutside).color;
})()
"#,
        )
        .expect("retained Stylist removal should evaluate");

    assert_eq!(remove_result, "rgb(1, 2, 3)");
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(document),
        1
    );
}
#[test]
fn has_selector_child_list_mutation_invalidates_ancestor_dependent_styles() {
    let mut vm = new_storage_test_vm("https://has-selector-child-list-style.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '#outside { color: rgb(255, 0, 0); } body:has(#marker) #outside { color: rgb(0, 128, 0); }';
  head.appendChild(style);

  const outside = document.createElement('div');
  outside.id = 'outside';
  const container = document.createElement('section');
  body.append(outside, container);
  const before = getComputedStyle(outside).color;

  const marker = document.createElement('span');
  marker.id = 'marker';
  container.appendChild(marker);
  const after = getComputedStyle(outside).color;
  return `${before}|${after}`;
})()
"#,
        )
        .expect(":has child-list style invalidation test should evaluate");

    assert_eq!(result, "rgb(255, 0, 0)|rgb(0, 128, 0)");
}
#[test]
fn uppercase_has_selector_child_list_mutation_invalidates_ancestor_dependent_styles() {
    let mut vm = new_storage_test_vm("https://uppercase-has-selector-child-list-style.test/");
    let document = vm.document_handle_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '#outside { color: rgb(255, 0, 0); } body:HAS(#marker) #outside { color: rgb(0, 128, 0); }';
  head.appendChild(style);

  const outside = document.createElement('div');
  outside.id = 'outside';
  const container = document.createElement('section');
  body.append(outside, container);
  globalThis.__uppercaseHasOutside = outside;
  globalThis.__uppercaseHasContainer = container;
  return getComputedStyle(outside).color;
})()
"#,
        )
        .expect("uppercase :HAS child-list style setup should evaluate");

    assert_eq!(result, "rgb(255, 0, 0)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let added = vm
        .eval(
            r#"
(() => {
  const marker = document.createElement('span');
  marker.id = 'marker';
  globalThis.__uppercaseHasContainer.appendChild(marker);
  return 'added';
})()
"#,
        )
        .expect("uppercase :HAS marker append should evaluate");

    assert_eq!(added, "added");
    vm._context_host
        .borrow()
        .drain_pending_style_invalidations_for_document_for_test(document);
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );

    let updated = vm
        .eval("getComputedStyle(globalThis.__uppercaseHasOutside).color")
        .expect("uppercase :HAS updated color should evaluate");

    assert_eq!(updated, "rgb(0, 128, 0)");
}
#[test]
fn escaped_has_selector_child_list_mutation_invalidates_ancestor_dependent_styles() {
    let mut vm = new_storage_test_vm("https://escaped-has-selector-child-list-style.test/");
    let document = vm.document_handle_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = String.raw`#outside { color: rgb(255, 0, 0); } body:h\61s(#marker) #outside { color: rgb(0, 128, 0); }`;
  head.appendChild(style);

  const outside = document.createElement('div');
  outside.id = 'outside';
  const container = document.createElement('section');
  body.append(outside, container);
  globalThis.__escapedHasOutside = outside;
  globalThis.__escapedHasContainer = container;
  return getComputedStyle(outside).color;
})()
"#,
        )
        .expect("escaped :has child-list style setup should evaluate");

    assert_eq!(result, "rgb(255, 0, 0)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let added = vm
        .eval(
            r#"
(() => {
  const marker = document.createElement('span');
  marker.id = 'marker';
  globalThis.__escapedHasContainer.appendChild(marker);
  return 'added';
})()
"#,
        )
        .expect("escaped :has marker append should evaluate");

    assert_eq!(added, "added");
    vm._context_host
        .borrow()
        .drain_pending_style_invalidations_for_document_for_test(document);
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );

    let updated = vm
        .eval("getComputedStyle(globalThis.__escapedHasOutside).color")
        .expect("escaped :has updated color should evaluate");

    assert_eq!(updated, "rgb(0, 128, 0)");
}
#[test]
fn quoted_has_text_does_not_force_full_child_list_style_invalidation() {
    let mut vm = new_storage_test_vm("https://quoted-has-style-cache.test/");
    let document = vm.document_handle_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = String.raw`
    /* :has(#marker) should not force full invalidation. */
    #outside { color: rgb(1, 2, 3); }
    #unused::before { content: ":has(#marker)"; }
  `;
  head.appendChild(style);

  const outsideRoot = document.createElement('section');
  const outside = document.createElement('div');
  outside.id = 'outside';
  outsideRoot.appendChild(outside);

  const container = document.createElement('section');
  const marker = document.createElement('span');
  marker.id = 'marker';
  body.append(outsideRoot, container);
  globalThis.__quotedHasContainer = container;
  globalThis.__quotedHasMarker = marker;
  return getComputedStyle(outside).color;
})()
"#,
        )
        .expect("quoted :has style cache setup should evaluate");

    assert_eq!(result, "rgb(1, 2, 3)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let added = vm
        .eval(
            r#"
(() => {
  globalThis.__quotedHasContainer.appendChild(globalThis.__quotedHasMarker);
  return 'added';
})()
"#,
        )
        .expect("quoted :has marker append should evaluate");

    assert_eq!(added, "added");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
}
#[test]
fn manual_slot_assignment_suppresses_fallback_layout() {
    let mut vm = new_storage_test_vm("https://html-slot-manual-layout.test/");

    vm.eval(
        r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const textFallbackHost = document.createElement('div');
  textFallbackHost.id = 'text-fallback-host';
  const elementFallbackHost = document.createElement('div');
  elementFallbackHost.id = 'element-fallback-host';
  textFallbackHost.appendChild(document.createElement('span'));
  elementFallbackHost.appendChild(document.createElement('span'));
  document.body.append(textFallbackHost, elementFallbackHost);

  const textFallbackShadow = textFallbackHost.attachShadow({
    mode: 'open',
    slotAssignment: 'manual'
  });
  textFallbackShadow.innerHTML = '<slot>fallback</slot>';

  const elementFallbackShadow = elementFallbackHost.attachShadow({
    mode: 'open',
    slotAssignment: 'manual'
  });
  elementFallbackShadow.innerHTML = '<slot><span>fallback</span></slot>';
  return 'installed';
})()
"#,
    )
    .expect("manual slot fallback fixture should initialize");
    refresh_layout_for_test(&mut vm);

    let assigned = vm
        .eval(
            r#"
(() => {
  const textFallbackHost = document.getElementById('text-fallback-host');
  const elementFallbackHost = document.getElementById('element-fallback-host');
  const textFallbackSlot = textFallbackHost.shadowRoot.firstChild;
  const elementFallbackSlot = elementFallbackHost.shadowRoot.firstChild;
  const textFallbackBefore = textFallbackHost.offsetHeight;
  const elementFallbackBefore = elementFallbackHost.offsetHeight;
  textFallbackSlot.assign(textFallbackHost.firstChild);
  elementFallbackSlot.assign(elementFallbackHost.firstChild);

  return [
    textFallbackBefore > 0,
    textFallbackSlot.assignedNodes()[0] === textFallbackHost.firstChild,
    elementFallbackBefore > 0,
    elementFallbackSlot.assignedNodes()[0] === elementFallbackHost.firstChild
  ].join('|');
})()
"#,
        )
        .expect("manual slot assignment should suppress fallback layout");
    assert_eq!(assigned, "true|true|true|true");
    refresh_layout_for_test(&mut vm);
    let heights = vm
        .eval(
            "[document.getElementById('text-fallback-host').offsetHeight, document.getElementById('element-fallback-host').offsetHeight].join('|')",
        )
        .expect("assigned manual slot geometry should evaluate after refresh");

    assert_eq!(heights, "0|0");
}
#[test]
fn input_type_change_to_radio_unchecks_checked_group_peer() {
    let mut vm = new_storage_test_vm("https://radio-type-morph.test/");

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

  document.body.innerHTML = '<input id="radio" type="radio" name="name_7" checked><input id="text" name="name_7" checked>';
  const radio = document.getElementById('radio');
  const text = document.getElementById('text');
  text.type = 'radio';
  return `${radio.checked}:${text.checked}`;
})()
"#,
        )
        .expect("radio type morph should update checked group peer");

    assert_eq!(result, "false:true");
}
#[test]
fn live_input_validity_state_preserves_expected_shape() {
    let mut vm = new_storage_test_vm("https://live-validity-state-shape.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const input = document.createElement('input');
  input.required = true;
  const validity = input.validity;
  return [
    Object.keys(validity).join(','),
    validity.valueMissing,
    validity.typeMismatch,
    validity.patternMismatch,
    validity.tooLong,
    validity.tooShort,
    validity.rangeUnderflow,
    validity.rangeOverflow,
    validity.stepMismatch,
    validity.badInput,
    validity.customError,
    validity.valid
  ].join('|');
})()
"#,
        )
        .expect("live input validity state should expose stable shape");

    assert_eq!(
        result,
        "valueMissing,typeMismatch,patternMismatch,tooLong,tooShort,rangeUnderflow,rangeOverflow,stepMismatch,badInput,customError,valid|true|false|false|false|false|false|false|false|false|false|false"
    );
}

#[test]
fn cloned_text_controls_preserve_user_edited_length_validity() {
    let mut vm = new_parsed_test_vm(
        "https://clone-user-edited-validity.test/",
        "<!doctype html><body></body>",
    );

    let result = vm
        .eval(
            r#"
(() => ['input', 'textarea'].map(tag => {
  const control = document.createElement(tag);
  document.body.append(control);
  control.focus();
  document.execCommand('insertText', false, 'something');
  control.maxLength = 0;
  const clone = control.cloneNode(true);
  return [
    control.value,
    control.validity.tooLong,
    control.matches(':invalid'),
    clone.value,
    clone.validity.tooLong,
    clone.matches(':invalid')
  ].join(':');
}).join('|'))()
"#,
        )
        .expect("cloned text-control validity should evaluate");

    assert_eq!(
        result,
        "something:true:true:something:true:true|something:true:true:something:true:true"
    );
}

#[test]
fn detached_input_and_textarea_track_dirty_state_for_form_reset() {
    let mut vm = new_storage_test_vm("https://detached-input-dirty-state.test/path/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body><form id="form">' +
      '<input id="text" name="q" value="seed">' +
      '<input id="check" type="checkbox" checked>' +
      '<textarea id="bio" name="bio" value="initial"></textarea>' +
    '</form></body></html>',
    'text/html'
  );
  const form = doc.getElementById('form');
  const text = doc.getElementById('text');
  const check = doc.getElementById('check');
  const bio = doc.getElementById('bio');
  const out = [];

  out.push([
    text.value,
    text.defaultValue,
    text.getAttribute('value'),
    check.checked,
    check.defaultChecked,
    check.getAttribute('checked') !== null,
    bio.value,
    bio.defaultValue,
    bio.getAttribute('value')
  ].join(':'));

  text.value = 'typed';
  text.defaultValue = 'new-default';
  check.checked = false;
  check.defaultChecked = true;
  bio.value = 'dirty-bio';
  bio.defaultValue = 'default-bio';
  out.push([
    text.value,
    text.defaultValue,
    text.getAttribute('value'),
    check.checked,
    check.defaultChecked,
    check.getAttribute('checked') !== null,
    bio.value,
    bio.defaultValue,
    bio.getAttribute('value')
  ].join(':'));

  form.reset();
  out.push([
    text.value,
    text.defaultValue,
    check.checked,
    check.defaultChecked,
    bio.value,
    bio.defaultValue
  ].join(':'));

  check.checked = true;
  check.defaultChecked = false;
  form.reset();
  out.push([
    check.checked,
    check.defaultChecked,
    check.getAttribute('checked') === null
  ].join(':'));

  return out.join('|');
})()
"#,
        )
        .expect("detached input and textarea dirty state should reset to defaults");

    assert_eq!(
        result,
        "seed:seed:seed:true:true:true:initial:initial:initial|typed:new-default:new-default:false:true:true:dirty-bio:default-bio:default-bio|new-default:new-default:true:true:default-bio:default-bio|false:false:true"
    );
}
