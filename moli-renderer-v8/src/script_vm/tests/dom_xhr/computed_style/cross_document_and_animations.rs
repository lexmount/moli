use super::*;

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
  popup.history.replaceState(null, '', 'about:blank#popup-old-target');
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
  __popupTargetWindow.history.replaceState(null, '', 'about:blank#popup-new-target');
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

    let result = eval_with_layout_publications(
        &mut vm,
        r#"
(function* () {
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

yield; // Publish this scene before reading its geometry.
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
    let mut vm = new_rendered_test_vm(
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

    publish_layout_for_test(&mut vm);
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
    let mut vm = new_rendered_test_vm(
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

    publish_layout_for_test(&mut vm);
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
    let mut vm = new_rendered_test_vm(
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

    publish_layout_for_test(&mut vm);
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

    publish_layout_for_test(&mut vm);
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

#[test]
fn raw_keyframe_scanner_respects_owner_media_changes() {
    let mut vm = new_parsed_test_vm(
        "https://keyframe-owner-media.test/",
        r#"<!doctype html>
        <style>
          #item {
            position: relative;
            animation-name: print-only;
            animation-duration: 1s;
          }
        </style>
        <style media="print">
          @keyframes print-only {
            from { left: 100px; }
            to { left: 400px; }
          }
        </style>
        <div id="item"></div>"#,
    );

    assert_eq!(
        vm.eval(
            r#"(() => {
              const item = document.getElementById('item');
              return [getComputedStyle(item).left, item.getAnimations().length].join('|');
            })()"#,
        )
        .expect("screen animation state should evaluate"),
        "0px|0",
        "a print-only keyframe rule must be invisible to the screen raw-CSS scanner",
    );

    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides {
        media: Some("print".to_owned()),
        ..Default::default()
    });
    assert_eq!(
        vm.eval(
            r#"(() => {
              const item = document.getElementById('item');
              return [getComputedStyle(item).left, item.getAnimations().length].join('|');
            })()"#,
        )
        .expect("print animation state should evaluate"),
        "250px|1",
        "the same retained source must become visible after switching to print",
    );

    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides::default());
    assert_eq!(
        vm.eval(
            r#"(() => {
              const item = document.getElementById('item');
              return [getComputedStyle(item).left, item.getAnimations().length].join('|');
            })()"#,
        )
        .expect("restored screen animation state should evaluate"),
        "0px|0",
        "leaving print must hide the raw keyframe source again",
    );
}

#[test]
fn raw_scanners_follow_live_owner_media_attribute_mutations() {
    let mut vm = new_parsed_test_vm(
        "https://raw-scanner-owner-media-mutation.test/",
        r#"<!doctype html>
        <style>
          #item {
            position: relative;
            animation: owner-gated 1s;
            --actual: --owner-gated();
          }
        </style>
        <style id="gated" media="print">
          @keyframes owner-gated {
            from { left: 20px; }
            to { left: 60px; }
          }
          @function --owner-gated() { result: 17px; }
        </style>
        <div id="item"></div>"#,
    );
    let probe = r#"(() => {
      const item = document.getElementById('item');
      const computed = getComputedStyle(item);
      return [
        computed.left,
        item.getAnimations().length,
        computed.getPropertyValue('--actual')
      ].join('|');
    })()"#;

    assert_eq!(
        vm.eval(probe).expect("initial raw scanner state"),
        "0px|0|--owner-gated()",
    );
    vm.eval("document.getElementById('gated').media = 'screen'")
        .expect("owner media should become effective");
    assert_eq!(
        vm.eval(probe).expect("effective raw scanner state"),
        "40px|1|17px",
        "owner media mutation must update both raw compatibility scanners",
    );
    vm.eval("document.getElementById('gated').media = 'not all'")
        .expect("owner media should become ineffective");
    assert_eq!(
        vm.eval(probe).expect("ineffective raw scanner state"),
        "0px|0|--owner-gated()",
        "the retained source must not leak after a second owner media mutation",
    );
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
fn slotted_nodes_inherit_css_direction_from_slot_without_changing_html_directionality() {
    let mut vm = new_storage_test_vm("https://slotted-direction-inheritance.test/");

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

  const host = document.createElement('div');
  const slotted = document.createElement('span');
  host.appendChild(slotted);
  const shadow = host.attachShadow({ mode: 'open' });
  const style = document.createElement('style');
  style.textContent = 'slot { color: rgb(1, 2, 3); }';
  const slot = document.createElement('slot');
  slot.dir = 'rtl';
  shadow.append(style, slot);
  document.body.appendChild(host);

  const inherited = `${slotted.matches(':dir(ltr)')}:${getComputedStyle(slotted).direction}:${getComputedStyle(slotted).color}`;

  const overriddenHost = document.createElement('div');
  const overriddenSlotted = document.createElement('span');
  overriddenHost.appendChild(overriddenSlotted);
  const overriddenShadow = overriddenHost.attachShadow({ mode: 'open' });
  const overriddenStyle = document.createElement('style');
  overriddenStyle.textContent = 'slot { direction: ltr; }';
  const overriddenSlot = document.createElement('slot');
  overriddenSlot.dir = 'rtl';
  overriddenShadow.append(overriddenStyle, overriddenSlot);
  document.body.appendChild(overriddenHost);

  const authorOverride = `${overriddenSlot.matches(':dir(rtl)')}:${getComputedStyle(overriddenSlot).direction}:${getComputedStyle(overriddenSlotted).direction}`;
  return `${inherited}|${authorOverride}`;
})()
"#,
        )
        .expect("slotted direction inheritance should evaluate");

    assert_eq!(result, "true:rtl:rgb(1, 2, 3)|true:ltr:ltr");
}

#[test]
fn computed_direction_tracks_textarea_auto_value() {
    let mut vm = new_storage_test_vm("https://textarea-auto-direction.test/");

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

  const textarea = document.createElement('textarea');
  textarea.dir = 'auto';
  document.body.appendChild(textarea);
  const empty = `${textarea.matches(':dir(ltr)')}:${getComputedStyle(textarea).direction}`;
  textarea.value = '\u05ea';
  const rtl = `${textarea.matches(':dir(rtl)')}:${getComputedStyle(textarea).direction}`;
  textarea.value = 'A';
  const ltr = `${textarea.matches(':dir(ltr)')}:${getComputedStyle(textarea).direction}`;

  return `${empty}|${rtl}|${ltr}`;
})()
"#,
        )
        .expect("textarea dir=auto value direction should evaluate");

    assert_eq!(result, "true:ltr|true:rtl|true:ltr");
}

#[test]
fn computed_direction_tracks_dir_auto_tree_mutations() {
    let mut vm = new_storage_test_vm("https://dir-auto-tree-mutation.test/");

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
    '#source:dir(rtl) + #target { display: none; }';

  const source = document.createElement('div');
  source.id = 'source';
  source.dir = 'auto';
  const target = document.createElement('div');
  target.id = 'target';
  document.body.append(source, target);

  const before = `${getComputedStyle(source).direction}:${getComputedStyle(target).display}`;
  const text = document.createTextNode('\u0627\u062e\u062a\u0628\u0631');
  source.appendChild(text);
  const afterAppend = `${getComputedStyle(source).direction}:${getComputedStyle(target).display}`;
  text.data = 'A';
  const afterText = `${getComputedStyle(source).direction}:${getComputedStyle(target).display}`;

  return `${before}|${afterAppend}|${afterText}`;
})()
"#,
        )
        .expect("dir=auto tree mutation direction should evaluate");

    assert_eq!(result, "ltr:block|rtl:none|ltr:block");
}
