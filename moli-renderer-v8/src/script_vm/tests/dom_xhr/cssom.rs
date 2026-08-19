use super::*;

fn cssom_element_handle_by_id(vm: &ScriptVm, id: &str) -> DomHandle {
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
        .unwrap_or_else(|| panic!("element #{id} should have a native handle"))
}

fn cssom_owner_document_handle_for_element_id(vm: &ScriptVm, id: &str) -> DomHandle {
    let element = cssom_element_handle_by_id(vm, id);
    vm.document_runtime
        .dom_host()
        .owner_document_handle(element)
        .unwrap_or_else(|| panic!("element #{id} should have an owner document"))
}

#[test]
fn linked_stylesheet_sheet_getter_uses_captured_request_url() {
    let mut vm = new_storage_test_vm("https://cssom-linked-base.test/page/index.html");
    let stylesheet_url = url::Url::parse("https://cssom-linked-base.test/base/app.css").unwrap();
    vm.eval(
        r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const base = document.createElement('base');
  base.href = 'https://cssom-linked-base.test/base/';
  head.appendChild(base);

  const link = document.createElement('link');
  link.id = 'captured-request-link';
  link.rel = 'stylesheet';
  link.href = 'app.css';
  head.appendChild(link);
  return link.href;
})()
"#,
    )
    .expect("linked stylesheet setup should evaluate");
    let link = cssom_element_handle_by_id(&vm, "captured-request-link");
    install_linked_stylesheet_for_test(
        &mut vm,
        link,
        stylesheet_url.clone(),
        crate::style_engine::StyloStylesheetSource::new(
            "body { color: rgb(1, 2, 3); }".to_owned(),
            stylesheet_url.clone(),
        )
        .with_sheet_url(stylesheet_url),
    );

    let result = vm
        .eval(
            r#"
(() => {
  const link = document.getElementById('captured-request-link');
  document.querySelector('base').href = 'https://cssom-linked-base.test/theme/';
  return [
    link.href,
    link.sheet.href,
    link.sheet.cssRules.length,
    link.sheet.cssRules[0].cssText
  ].join('|');
})()
"#,
        )
        .expect("captured linked stylesheet source should evaluate");

    assert_eq!(
        result,
        "https://cssom-linked-base.test/theme/app.css|https://cssom-linked-base.test/base/app.css|1|body { color: rgb(1, 2, 3); }"
    );
}

#[test]
fn linked_stylesheet_resource_parse_is_shared_but_owner_mutation_is_copy_on_write() {
    let mut vm = new_storage_test_vm("https://linked-sheet-copy-on-write.test/page.html");
    let stylesheet_url =
        url::Url::parse("https://linked-sheet-copy-on-write.test/shared.css").unwrap();
    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  for (const id of ['first-link', 'second-link']) {
    const link = document.createElement('link');
    link.id = id;
    link.rel = 'stylesheet';
    link.href = '/shared.css';
    head.appendChild(link);
  }
  const target = document.createElement('div');
  target.className = 'shared-target';
  body.appendChild(target);
})()
"#,
    )
    .expect("linked stylesheet COW setup should evaluate");
    let first = cssom_element_handle_by_id(&vm, "first-link");
    let second = cssom_element_handle_by_id(&vm, "second-link");

    crate::live_stylesheet::reset_live_stylesheet_parse_count_for_test();
    crate::style_engine::reset_author_source_text_parse_count_for_test();
    let prepared = vm
        ._context_host
        .borrow()
        .prepare_linked_stylesheet_resource(
            first,
            ".shared-target { color: rgb(1, 2, 3); margin-left: 2px; }",
            stylesheet_url.clone(),
            stylesheet_url.clone(),
            true,
        )
        .expect("connected link should prepare a resource source");
    {
        let mut host = vm._context_host.borrow_mut();
        host.install_linked_stylesheet(
            crate::document_runtime::InstallLinkedStylesheet::from_prepared(
                first,
                stylesheet_url.clone(),
                prepared.clone(),
            ),
        );
        host.install_linked_stylesheet(
            crate::document_runtime::InstallLinkedStylesheet::from_prepared(
                second,
                stylesheet_url.clone(),
                prepared,
            ),
        );
    }
    vm.apply_pending_stylesheet_source_css_projections();

    let result = vm
        .eval(
            r#"
(() => {
  const firstLink = document.getElementById('first-link');
  const secondLink = document.getElementById('second-link');
  const target = document.querySelector('.shared-target');
  const firstSheet = firstLink.sheet;
  const secondSheet = secondLink.sheet;
  const firstRule = firstSheet.cssRules[0];
  const secondRule = secondSheet.cssRules[0];
  firstRule.marker = 'first';
  secondRule.marker = 'second';

  firstRule.style.color = 'rgb(4, 5, 6)';
  firstRule.style.marginLeft = '7px';
  const isolated = [
    firstRule.cssText,
    secondRule.cssText,
    firstRule.marker,
    secondRule.marker,
  ];

  secondLink.disabled = true;
  const firstComputed = getComputedStyle(target);
  const firstResult = [firstComputed.color, firstComputed.marginLeft].join('|');
  const detachedSecond = secondSheet.ownerNode === null;
  secondLink.disabled = false;
  const reboundSheet = secondLink.sheet;
  const reboundComputed = getComputedStyle(target);

  return JSON.stringify({
    distinctSheets: firstSheet !== secondSheet,
    distinctRules: firstRule !== secondRule,
    isolated,
    firstResult,
    detachedSecond,
    reboundIsNew: reboundSheet !== secondSheet && reboundSheet.ownerNode === secondLink,
    reboundRule: reboundSheet.cssRules[0].cssText,
    reboundResult: [reboundComputed.color, reboundComputed.marginLeft].join('|'),
  });
})()
"#,
        )
        .expect("linked stylesheet owners should remain isolated after mutation");

    assert_eq!(
        result,
        r#"{"distinctSheets":true,"distinctRules":true,"isolated":[".shared-target { color: rgb(4, 5, 6); margin-left: 7px; }",".shared-target { color: rgb(1, 2, 3); margin-left: 2px; }","first","second"],"firstResult":"rgb(4, 5, 6)|7px","detachedSecond":true,"reboundIsNew":true,"reboundRule":".shared-target { color: rgb(1, 2, 3); margin-left: 2px; }","reboundResult":"rgb(1, 2, 3)|2px"}"#
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_parse_count_for_test(),
        1,
        "one linked response must create one parsed template across owner clients and cache rebinds"
    );
    assert_eq!(
        crate::style_engine::author_source_text_parse_count_for_test(),
        0,
        "linked owner cascade must consume the live parsed stylesheet"
    );
}

#[test]
fn linked_stylesheet_response_replacement_detaches_the_previous_live_wrapper() {
    let mut vm = new_storage_test_vm("https://linked-sheet-replacement.test/page.html");
    let stylesheet_url = url::Url::parse("https://linked-sheet-replacement.test/app.css").unwrap();
    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const link = document.createElement('link');
  link.id = 'replacement-link';
  link.rel = 'stylesheet';
  link.href = '/app.css';
  head.appendChild(link);
  const target = document.createElement('div');
  target.className = 'replacement-target';
  body.appendChild(target);
})()
"#,
    )
    .expect("linked replacement setup should evaluate");
    let link = cssom_element_handle_by_id(&vm, "replacement-link");

    for css_text in [
        ".replacement-target { color: rgb(1, 2, 3); }",
        ".replacement-target { color: rgb(4, 5, 6); }",
    ] {
        let prepared = vm
            ._context_host
            .borrow()
            .prepare_linked_stylesheet_resource(
                link,
                css_text,
                stylesheet_url.clone(),
                stylesheet_url.clone(),
                true,
            )
            .expect("connected link should prepare replacement resource");
        vm._context_host.borrow_mut().install_linked_stylesheet(
            crate::document_runtime::InstallLinkedStylesheet::from_prepared(
                link,
                stylesheet_url.clone(),
                prepared,
            ),
        );
        vm.apply_pending_stylesheet_source_css_projections();
        if css_text.contains("1, 2, 3") {
            vm.eval(
                r#"
globalThis.__oldLinkedSheet = document.getElementById('replacement-link').sheet;
globalThis.__oldLinkedRule = globalThis.__oldLinkedSheet.cssRules[0];
globalThis.__oldLinkedRule.marker = 'retained';
"#,
            )
            .expect("old linked wrapper should materialize");
        }
    }

    let result = vm
        .eval(
            r#"
(() => {
  const link = document.getElementById('replacement-link');
  const target = document.querySelector('.replacement-target');
  const currentSheet = link.sheet;
  const beforeDetachedMutation = getComputedStyle(target).color;
  globalThis.__oldLinkedRule.style.color = 'rgb(7, 8, 9)';
  const afterDetachedMutation = getComputedStyle(target).color;
  return [
    currentSheet !== globalThis.__oldLinkedSheet,
    globalThis.__oldLinkedSheet.ownerNode === null,
    currentSheet.ownerNode === link,
    globalThis.__oldLinkedRule.marker,
    currentSheet.cssRules[0].cssText,
    beforeDetachedMutation,
    afterDetachedMutation,
  ].join('|');
})()
"#,
        )
        .expect("replacement linked wrapper should own current cascade state");

    assert_eq!(
        result,
        "true|true|true|retained|.replacement-target { color: rgb(4, 5, 6); }|rgb(4, 5, 6)|rgb(4, 5, 6)"
    );
}

#[test]
fn alternate_stylesheet_requires_non_empty_title_for_sheet_and_activation() {
    let mut vm = new_storage_test_vm("https://alternate-stylesheet-title.test/page.html");
    let invalid_url =
        url::Url::parse("https://alternate-stylesheet-title.test/invalid.css").unwrap();
    let valid_url = url::Url::parse("https://alternate-stylesheet-title.test/valid.css").unwrap();

    vm.eval(
        r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));

  const invalid = document.createElement('link');
  invalid.id = 'invalid-alternate';
  invalid.rel = 'alternate stylesheet';
  invalid.title = '';
  invalid.href = '/invalid.css';
  head.appendChild(invalid);
  invalid.disabled = true;
  invalid.disabled = false;

  const valid = document.createElement('link');
  valid.id = 'valid-alternate';
  valid.rel = 'alternate stylesheet';
  valid.title = 'contrast';
  valid.href = '/valid.css';
  head.appendChild(valid);

  const target = document.createElement('div');
  target.className = 'alternate-target';
  body.appendChild(target);
})()
"#,
    )
    .expect("alternate stylesheet title setup should evaluate");

    let invalid = cssom_element_handle_by_id(&vm, "invalid-alternate");
    install_linked_stylesheet_for_test(
        &mut vm,
        invalid,
        invalid_url.clone(),
        crate::style_engine::StyloStylesheetSource::new(
            ".alternate-target { color: rgb(1, 2, 3); }".to_owned(),
            invalid_url.clone(),
        )
        .with_sheet_url(invalid_url),
    );
    let valid = cssom_element_handle_by_id(&vm, "valid-alternate");
    install_linked_stylesheet_for_test(
        &mut vm,
        valid,
        valid_url.clone(),
        crate::style_engine::StyloStylesheetSource::new(
            ".alternate-target { background: rgb(4, 5, 6); }".to_owned(),
            valid_url.clone(),
        )
        .with_sheet_url(valid_url),
    );

    let result = vm
        .eval(
            r#"
(() => {
  const invalid = document.getElementById('invalid-alternate');
  const valid = document.getElementById('valid-alternate');
  const target = document.querySelector('.alternate-target');
  const initialColor = getComputedStyle(target).color;
  const validSheet = valid.sheet;
  const invalidInitialNull = invalid.sheet === null;
  valid.title = 'contrast-updated';
  const validSheetPreserved = valid.sheet === validSheet && validSheet.title === 'contrast-updated';

  invalid.title = 'enabled';
  const enabledSheet = invalid.sheet;
  const enabledSheetBound = enabledSheet instanceof CSSStyleSheet && enabledSheet.ownerNode === invalid;
  const enabledColor = getComputedStyle(target).color;

  invalid.title = '';
  return JSON.stringify({
    invalidInitialNull,
    validSheet: validSheet instanceof CSSStyleSheet && validSheet.ownerNode === valid,
    validSheetPreserved,
    initialColor,
    enabledSheet: enabledSheetBound,
    enabledColor,
    clearedSheet: invalid.sheet === null && enabledSheet.ownerNode === null,
    clearedColor: getComputedStyle(target).color
  });
})()
"#,
        )
        .expect("alternate stylesheet title transitions should evaluate");

    assert_eq!(
        result,
        r#"{"invalidInitialNull":true,"validSheet":true,"validSheetPreserved":true,"initialColor":"rgb(0, 0, 0)","enabledSheet":true,"enabledColor":"rgb(1, 2, 3)","clearedSheet":true,"clearedColor":"rgb(0, 0, 0)"}"#
    );
}

#[test]
fn linked_stylesheet_origin_clean_controls_cssom_rule_access() {
    let mut vm = new_storage_test_vm("https://cssom-origin-clean.test/page.html");
    let stylesheet_url = url::Url::parse("https://cdn.cssom-origin-clean.test/app.css").unwrap();
    vm.eval(
        r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const link = document.createElement('link');
  link.id = 'origin-clean-link';
  link.rel = 'stylesheet';
  link.href = 'https://cdn.cssom-origin-clean.test/app.css';
  head.appendChild(link);
  const target = document.createElement('div');
  target.className = 'target';
  body.appendChild(target);
})()
"#,
    )
    .expect("origin-clean linked stylesheet setup should evaluate");
    let link = cssom_element_handle_by_id(&vm, "origin-clean-link");
    install_linked_stylesheet_for_test(
        &mut vm,
        link,
        stylesheet_url.clone(),
        crate::style_engine::StyloStylesheetSource::new(
            ".target { color: rgb(4, 5, 6); }".to_owned(),
            stylesheet_url.clone(),
        )
        .with_sheet_url(stylesheet_url)
        .with_origin_clean(false),
    );

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      callback();
      return 'ok';
    } catch (error) {
      return `${error.name}:${error instanceof DOMException}`;
    }
  };
  const link = document.getElementById('origin-clean-link');
  const target = document.querySelector('.target');
  const sheet = link.sheet;
  return [
    sheet !== null,
    getComputedStyle(target).color,
    probe(() => sheet.cssRules.length),
    probe(() => sheet.insertRule('.x { color: red; }', 0)),
    probe(() => sheet.deleteRule(0))
  ].join('|');
})()
"#,
        )
        .expect("linked stylesheet origin-clean CSSOM probe should evaluate");

    assert_eq!(
        result,
        "true|rgb(4, 5, 6)|SecurityError:true|SecurityError:true|SecurityError:true"
    );
}

#[test]
fn element_stylesheet_cache_ignores_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://stylesheet-cache-private-slot.test/");
    let linked_url =
        url::Url::parse("data:text/css,.linkreal%20%7B%20color%3A%20rgb(4%2C%205%2C%206)%3B%20%7D")
            .unwrap();

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const internal = '__moliStyleSheet';
  const internalNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name.startsWith(internal))
    .sort()
    .join(',');

  const style = document.createElement('style');
  style.textContent = '.real { color: rgb(1, 2, 3); }';
  head.appendChild(style);
  const styleInitialNames = internalNames(style);
  const stylePrototypeSheet = new CSSStyleSheet();
  stylePrototypeSheet.replaceSync('.proto { color: red; }');
  const styleOwnSheet = new CSSStyleSheet();
  styleOwnSheet.replaceSync('.own { color: blue; }');
  Object.getPrototypeOf(style)[internal] = stylePrototypeSheet;
  style[internal] = styleOwnSheet;
  const styleSheet = style.sheet;
  const styleSheetAfter = style.sheet;

  const link = document.createElement('link');
  link.id = 'private-slot-link';
  link.rel = 'stylesheet';
  link.href = 'data:text/css,.linkreal%20%7B%20color%3A%20rgb(4%2C%205%2C%206)%3B%20%7D';
  head.appendChild(link);
  const linkInitialNames = internalNames(link);
  const linkPrototypeSheet = new CSSStyleSheet();
  linkPrototypeSheet.replaceSync('.linkproto { color: red; }');
  const linkOwnSheet = new CSSStyleSheet();
  linkOwnSheet.replaceSync('.linkown { color: blue; }');
  Object.getPrototypeOf(link)[internal] = linkPrototypeSheet;
  link[internal] = linkOwnSheet;

  globalThis.__styleSheetPrivateSlotProbe = {
    internal,
    internalNames,
    style,
    styleInitialNames,
    stylePrototypeSheet,
    styleOwnSheet,
    styleSheet,
    styleSheetAfter,
    link,
    linkInitialNames,
    linkPrototypeSheet,
    linkOwnSheet
  };
})()
"#,
    )
    .expect("element stylesheet cache setup should evaluate");
    let link = cssom_element_handle_by_id(&vm, "private-slot-link");
    install_linked_stylesheet_for_test(
        &mut vm,
        link,
        linked_url.clone(),
        crate::style_engine::StyloStylesheetSource::new(
            ".linkreal { color: rgb(4, 5, 6); }".to_owned(),
            linked_url.clone(),
        )
        .with_sheet_url(linked_url),
    );

    let result = vm
        .eval(
            r#"
(() => {
  const {
    internal,
    internalNames,
    style,
    styleInitialNames,
    stylePrototypeSheet,
    styleOwnSheet,
    styleSheet,
    styleSheetAfter,
    link,
    linkInitialNames,
    linkPrototypeSheet,
    linkOwnSheet
  } = globalThis.__styleSheetPrivateSlotProbe;
  delete globalThis.__styleSheetPrivateSlotProbe;
  const linkSheet = link.sheet;
  link.disabled = true;
  const ownerCleared = linkSheet.ownerNode === null;
  link.disabled = false;
  const replacementLinkSheet = link.sheet;
  const oldOwnerStayedCleared = linkSheet.ownerNode === null;

  return JSON.stringify({
    styleInitialNames,
    styleSpoofedNames: internalNames(style),
    stylePublicSpoof: style[internal] === styleOwnSheet,
    styleReturnedReal: styleSheet !== styleOwnSheet && styleSheet !== stylePrototypeSheet,
    styleStable: styleSheetAfter === styleSheet,
    styleRule: styleSheet.cssRules[0].cssText,
    linkInitialNames,
    linkSpoofedNames: internalNames(link),
    linkPublicSpoof: link[internal] === linkOwnSheet,
    linkReturnedReal: linkSheet !== linkOwnSheet && linkSheet !== linkPrototypeSheet,
    linkUnavailableWhileReloading: link.sheet === null,
    linkReplacementCreated: replacementLinkSheet !== linkSheet && replacementLinkSheet.ownerNode === link,
    linkRule: linkSheet.cssRules[0].cssText,
    ownerCleared,
    oldOwnerStayedCleared
  });
})()
"#,
        )
        .expect("element stylesheet cache should ignore public spoofing");

    assert_eq!(
        result,
        r#"{"styleInitialNames":"","styleSpoofedNames":"__moliStyleSheet","stylePublicSpoof":true,"styleReturnedReal":true,"styleStable":true,"styleRule":".real { color: rgb(1, 2, 3); }","linkInitialNames":"","linkSpoofedNames":"__moliStyleSheet","linkPublicSpoof":true,"linkReturnedReal":true,"linkUnavailableWhileReloading":false,"linkReplacementCreated":true,"linkRule":".linkreal { color: rgb(4, 5, 6); }","ownerCleared":true,"oldOwnerStayedCleared":true}"#
    );
}

#[test]
fn child_document_create_element_exposes_css_style_declaration() {
    let mut vm = new_storage_test_vm("https://child-window-detached-style.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  frame.srcdoc = '<body></body>';
  (document.body || document.documentElement || document).appendChild(frame);
  const style = frame.contentDocument.createElement('div').style;
    style.display = 'none';
    style.setProperty('background-color', 'red');
    const transitionProbe = 'WebkitTransition' in style;
    return [
    typeof style,
    Object.prototype.toString.call(style),
    transitionProbe,
    style.display,
    style.getPropertyValue('display'),
    style.item(0),
    style.getPropertyPriority('background-color'),
    style.getPropertyValue('background-color'),
    style.removeProperty('background-color'),
    style.getPropertyValue('background-color')
  ].join('|');
})()
"#,
        )
        .expect("detached child document element style should be readable");

    assert_eq!(
        result,
        "object|[object CSSStyleProperties]|true|none|none|display||red|red|"
    );
}
#[test]
fn detached_css_style_css_text_parses_serializes_and_tracks_order() {
    let mut vm = new_storage_test_vm("https://detached-style-css-text.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const style = doc.createElement('div').style;
  style.cssText = 'color: red; background-image: url("a;b"); margin: 0 !important; broken';
  const afterCssText = [
    style.length,
    style.item(0),
    style.item(1),
    style.item(2),
    style.item(3),
    style.getPropertyValue('color'),
    style.getPropertyValue('background-image'),
    style.getPropertyValue('margin'),
    style.getPropertyPriority('margin'),
    style.cssText
  ].join(',');
  style.setProperty('display', 'none');
  style.setProperty('color', 'blue');
  style.setProperty('opacity', '0.5', 'invalid');
  const removed = style.removeProperty('background-image');
  const afterMutation = [
    style.length,
    style.item(0),
    style.item(1),
    style.item(2),
    style.getPropertyValue('color'),
    style.getPropertyValue('display'),
    style.getPropertyValue('opacity'),
    removed,
    style.cssText
  ].join(',');
  const ownInternalSlotCount = Object.getOwnPropertyNames(style)
    .filter((key) => key.startsWith('__moli'))
    .length;
  const sheet = new CSSStyleSheet();
  sheet.insertRule('.x { color: red; }', 0);
  const ruleStyle = sheet.cssRules[0].style;
  ruleStyle.setProperty('margin-left', '4px', 'important');
  const ruleOwnInternalSlotCount = Object.getOwnPropertyNames(ruleStyle)
    .filter((key) => key.startsWith('__moli'))
    .length;
  const ruleStyleSync = [
    ruleStyle.getPropertyPriority('margin-left'),
    sheet.cssRules[0].cssText.includes('margin-left: 4px !important')
  ].join(',');
  style.cssText = null;
  return [
    afterCssText,
    afterMutation,
    ownInternalSlotCount,
    ruleOwnInternalSlotCount,
    ruleStyleSync,
    style.length,
    style.cssText
  ].join('|');
})()
"#,
        )
        .expect("detached CSSStyleDeclaration cssText should parse and serialize");

    assert_eq!(
        result,
        "6,color,background-image,margin-top,margin-right,red,url(\"a;b\"),0px,important,color: red; background-image: url(\"a;b\"); margin: 0px !important;|6,margin-top,margin-right,margin-bottom,blue,none,,url(\"a;b\"),margin: 0px !important; display: none; color: blue;|0|0|important,true|0|"
    );
}

#[test]
fn css_math_style_api_uses_stylo_parser_and_serialization() {
    let mut vm = new_storage_test_vm("https://css-math-style-api.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  (document.body || document.documentElement || document).appendChild(target);
  const style = target.style;

  const supports = [
    CSS.supports('width', 'calc(10px + 1vmin + 10%)'),
    CSS.supports('width', 'calc(7px * up)'),
    CSS.supports('margin-top', 'clamp(1px,2px,3px)'),
    CSS.supports('transform', 'rotate(calc((0.25turn error)))'),
    CSS.supports('tab-size', 'calc(2 * 3)')
  ].join(',');

  style.width = 'calc(7px * up)';
  const invalidWidth = style.width;
  style.width = 'calc(10px + 1vmin + 10%)';
  const validWidth = style.width;
  style.marginTop = 'clamp(1px,2px,3px)';
  const marginTop = style.marginTop;
  style.border = 'calc(calc(10px)) solid pink';
  const border = style.border;
  const borderColor = style.borderColor;
  const borderStyle = style.borderStyle;
  const borderWidthFromBorder = style.borderWidth;
  style.borderTop = 'calc(calc(11px)) solid pink';
  const borderTop = style.borderTop;
  style.borderWidth = 'calc(calc(12px))';
  const borderWidth = style.borderWidth;

  target.setAttribute('style', 'width: calc(7px * up); margin-top: clamp(1px,2px,3px); tab-size: calc(2 * 3);');
  const inlineValues = [
    target.style.width,
    target.style.marginTop,
    target.style.tabSize,
    getComputedStyle(target).tabSize
  ].join(',');

  return [supports, invalidWidth, validWidth, marginTop, border, borderColor, borderStyle, borderWidthFromBorder, borderTop, borderWidth, inlineValues].join('|');
})()
"#,
        )
        .expect("CSS math style API probe should evaluate");

    assert_eq!(
        result,
        "true,false,true,false,true||calc(10% + 10px + 1vmin)|calc(2px)|calc(10px) solid pink|pink|solid|calc(10px)|calc(11px) solid pink|calc(12px)|,calc(2px),calc(6),6"
    );
}

#[test]
fn css_style_border_side_idl_setter_preserves_css_math_shorthand() {
    let mut vm = new_storage_test_vm("https://css-border-side-idl-math.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('div').style;
  style.borderTop = 'calc(calc(11px)) solid pink';
  const direct = [
    style.length,
    style.item(0),
    style.item(1),
    style.item(2),
    style.borderTop,
    style.getPropertyValue('border-top'),
    style.cssText
  ].join('|');
  style.cssText = '';
  style.border = 'calc(calc(10px)) solid pink';
  void style.borderColor;
  void style.borderStyle;
  void style.borderWidth;
  style.borderTop = 'calc(calc(11px)) solid pink';
  const afterBorder = [
    style.length,
    style.item(0),
    style.item(1),
    style.borderTop,
    style.getPropertyValue('border-top'),
    style.cssText
  ].join('|');
  return [direct, afterBorder].join('/');
})()
"#,
        )
        .expect("border side CSS math IDL setter should evaluate");

    assert_eq!(
        result,
        "3|border-top-width|border-top-style|border-top-color|calc(11px) solid pink|calc(11px) solid pink|border-top: calc(11px) solid pink;/17|border-right-width|border-right-style|calc(11px) solid pink|calc(11px) solid pink|border-width: calc(11px) calc(10px) calc(10px); border-style: solid; border-color: pink; border-image: none;"
    );
}

#[test]
fn css_math_computed_values_resolve_known_percentage_basis() {
    let mut vm = new_storage_test_vm("https://css-math-computed-basis.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const parent = document.createElement('div');
  parent.style.cssText = 'width: 100px; height: 570px;';
  const target = document.createElement('div');
  target.style.fontSize = '16px';
  parent.appendChild(target);
  body.appendChild(parent);

  target.style.backgroundPosition = 'calc(100% - 100% + 20em)';
  const backgroundPosition = getComputedStyle(target).backgroundPosition;
  target.style.height = 'calc(60% - 50% + 3em)';
  const height = getComputedStyle(target).height;
  target.style.marginLeft = 'min(20px, 10%)';
  const marginLeft = getComputedStyle(target).marginLeft;
  target.style.width = 'max((min(10%, 30px) + 10px) * 2 + 10px, 5em + 5%)';
  const width = getComputedStyle(target).width;
  target.style.marginLeft = 'min(1cm)';
  const minCm = getComputedStyle(target).marginLeft;
  target.style.marginLeft = '1cm';
  const cm = getComputedStyle(target).marginLeft;
  const absoluteLengthEquivalent = String(minCm === cm);

  root.style.fontSize = '30px';
  const remParent = document.createElement('div');
  remParent.style.width = '520px';
  const remTarget = document.createElement('div');
  remParent.appendChild(remTarget);
  body.appendChild(remParent);
  remTarget.style.width = 'calc(5% + 4rem)';
  const remWidth = getComputedStyle(remTarget).width;

  return [backgroundPosition, height, marginLeft, width, absoluteLengthEquivalent, remWidth].join('|');
})()
"#,
        )
        .expect("CSS math computed basis probe should evaluate");

    assert_eq!(result, "calc(0% + 320px) 50%|105px|10px|85px|true|146px");
}

#[test]
fn css_math_length_surface_and_steps_integer_syntax_match_css_values() {
    let mut vm = new_storage_test_vm("https://css-math-length-steps.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  (document.body || document.documentElement || document).appendChild(target);
  target.style.letterSpacing = 'clamp(10px, 20px, 30px)';
  const computed = getComputedStyle(target);

  const timing = document.createElement('div').style;
  timing.animationTimingFunction = 'steps(10)';
  const initial = timing.animationTimingFunction;
  timing.animationTimingFunction = 'steps(1e1)';
  const bareExponent = timing.animationTimingFunction;
  timing.animationTimingFunction = 'steps(calc(1e1))';
  const calcExponent = timing.animationTimingFunction;

  return [
    'letter-spacing' in computed,
    CSS.supports('letter-spacing', 'clamp(10px, 20px, 30px)'),
    computed.letterSpacing,
    CSS.supports('animation-timing-function', 'steps(1e1)'),
    CSS.supports('animation-timing-function', 'steps(calc(1e1))'),
    initial,
    bareExponent,
    calcExponent
  ].join('|');
})()
"#,
        )
        .expect("CSS math letter-spacing and steps integer syntax probe should evaluate");

    assert_eq!(
        result,
        "true|true|20px|false|true|steps(10)|steps(10)|steps(calc(10))"
    );
}

#[test]
fn css_math_individual_transform_properties_use_stylo() {
    let mut vm = new_storage_test_vm("https://css-math-individual-transform.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  (document.body || document.documentElement || document).appendChild(target);
  target.style.scale = 'min(0.2, max(0.1, 0.15))';
  target.style.rotate = 'min(1deg, 2deg)';
  const computed = getComputedStyle(target);
  return [
    'scale' in computed,
    'rotate' in computed,
    CSS.supports('scale', 'min(0.2, max(0.1, 0.15))'),
    CSS.supports('rotate', 'min(1deg, 2deg)'),
    computed.scale,
    computed.rotate
  ].join('|');
})()
"#,
        )
        .expect("CSS math individual transform probe should evaluate");

    assert_eq!(result, "true|true|true|true|0.15|1deg");
}

#[test]
fn css_variable_specified_values_preserve_cssom_shorthand_boundaries() {
    let mut vm = new_storage_test_vm("https://css-var-cssom-specified.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  (document.body || document.documentElement || document).appendChild(target);
  const style = target.style;

  style.cssText = 'margin: var(--prop);';
  const simple = [
    style.cssText,
    style.getPropertyValue('margin'),
    style.getPropertyValue('margin-top')
  ].join(',');

  style.cssText = 'margin: var(--prop); margin-top: 10px';
  const overridden = [
    style.cssText,
    style.getPropertyValue('margin'),
    style.getPropertyValue('margin-left'),
    style.getPropertyValue('margin-top')
  ].join(',');

  style.cssText = 'margin: var(--prop) !important; margin-top: 10px';
  const important = [
    style.getPropertyValue('margin'),
    style.getPropertyValue('margin-top')
  ].join(',');

  style.cssText = 'width: var(--x ()); expando: var(--prop); color: /* drop */ var(--prop)  /* keep */ var(--prop);';
  const validation = [
    style.getPropertyValue('width'),
    style.getPropertyValue('expando'),
    style.getPropertyValue('color'),
    style.cssText
  ].join(',');

  const sheetStyle = document.createElement('style');
  sheetStyle.textContent = 'div { width: var(--open';
  (document.head || document.documentElement || document).appendChild(sheetStyle);
  const leftOpen = sheetStyle.sheet.cssRules[0].style.getPropertyValue('width');

  const borderTarget = document.createElement('div');
  borderTarget.style.cssText = 'border-style: dashed; --border1: 5px solid rgb(0, 0, 0); --border2: 3px dotted red; --width: 1px; border-left: var(--border1); border-width: var(--width);';
  borderTarget.style.borderLeft = 'var(--border2)';
  (document.body || document.documentElement || document).appendChild(borderTarget);
  const computed = getComputedStyle(borderTarget);
  const borderProjection = [
    computed.getPropertyValue('border-left-width'),
    computed.getPropertyValue('border-top-width'),
    computed.getPropertyValue('border-right-width'),
    computed.getPropertyValue('border-bottom-width')
  ].join(',');

  return [simple, overridden, important, validation, leftOpen, borderProjection].join('|');
})()
"#,
        )
        .expect("CSS variable specified CSSOM probe should evaluate");

    assert_eq!(
        result,
        "margin: var(--prop);,var(--prop),|margin-right: ; margin-bottom: ; margin-left: ; margin-top: 10px;,,,10px|var(--prop),|,,var(--prop)  /* keep */ var(--prop),color: var(--prop)  /* keep */ var(--prop);|var(--open)|3px,1px,1px,1px"
    );
}

#[test]
fn cssom_text_decoration_longhands_reject_invalid_values_and_normalize_valid_ones() {
    let mut vm = new_storage_test_vm("https://cssom-text-decoration.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('div').style;
  const lineExposed = 'text-decoration-line' in style && 'textDecorationLine' in style;
  const insetExposed = 'text-decoration-inset' in style && 'textDecorationInset' in style;
  const skipInkExposed = 'text-decoration-skip-ink' in style && 'textDecorationSkipInk' in style;
  const skipSpacesExposed = 'text-decoration-skip-spaces' in style && 'textDecorationSkipSpaces' in style;
  const styleExposed = 'text-decoration-style' in style && 'textDecorationStyle' in style;
  const thicknessExposed = 'text-decoration-thickness' in style && 'textDecorationThickness' in style;
  const underlineOffsetExposed = 'text-underline-offset' in style && 'textUnderlineOffset' in style;
  const underlinePositionExposed = 'text-underline-position' in style && 'textUnderlinePosition' in style;

  style.setProperty('text-decoration-line', 'overline underline');
  const lineCanonical = style.getPropertyValue('text-decoration-line');
  style.setProperty('text-decoration-line', 'underline underline');
  const lineAfterInvalid = style.getPropertyValue('text-decoration-line');
  style.setProperty('text-decoration-line', 'Spelling-Error');
  const spelling = style.getPropertyValue('text-decoration-line');
  style.setProperty('text-decoration-line', 'underline/**/overline');
  const commentWhitespace = style.getPropertyValue('text-decoration-line');

  style.setProperty('text-decoration-style', 'WAVY');
  const styleCanonical = style.getPropertyValue('text-decoration-style');
  style.setProperty('text-decoration-style', 'solid wavy');
  const styleAfterInvalid = style.getPropertyValue('text-decoration-style');

  style.setProperty('text-decoration-skip-ink', 'ALL');
  const skipInkCanonical = style.getPropertyValue('text-decoration-skip-ink');
  style.setProperty('text-decoration-skip-ink', 'auto none');
  const skipInkAfterInvalid = style.getPropertyValue('text-decoration-skip-ink');

  style.setProperty('text-decoration-skip-spaces', 'end start');
  const skipSpacesCanonical = style.getPropertyValue('text-decoration-skip-spaces');
  style.setProperty('text-decoration-skip-spaces', 'all start');
  const skipSpacesAfterInvalid = style.getPropertyValue('text-decoration-skip-spaces');

  style.setProperty('text-decoration-inset', '0px 0px');
  const insetCollapsed = style.getPropertyValue('text-decoration-inset');
  style.setProperty('text-decoration-inset', 'calc(1em / 4) calc(-1ch)');
  const insetMath = style.getPropertyValue('text-decoration-inset');

  style.setProperty('text-decoration-thickness', '10e2');
  const invalidThickness = style.getPropertyValue('text-decoration-thickness');
  style.setProperty('text-decoration-thickness', 'calc(40% - 20px)');
  const mathThickness = style.getPropertyValue('text-decoration-thickness');
  style.setProperty('text-decoration-thickness', 'from-font');
  const fromFont = style.getPropertyValue('text-decoration-thickness');

  style.setProperty('text-underline-offset', 'from-font');
  const invalidOffset = style.getPropertyValue('text-underline-offset');
  style.setProperty('text-underline-offset', 'calc(45% - 0.3em)');
  const mathOffset = style.getPropertyValue('text-underline-offset');
  style.setProperty('text-underline-position', 'right under');
  const underlinePositionCanonical = style.getPropertyValue('text-underline-position');
  style.setProperty('text-underline-position', 'left right');
  const underlinePositionAfterInvalid = style.getPropertyValue('text-underline-position');

  style.setProperty('text-decoration', 'overline from-font dotted green');
  const shorthand = style.getPropertyValue('text-decoration');
  const shorthandLine = style.getPropertyValue('text-decoration-line');
  const shorthandThickness = style.getPropertyValue('text-decoration-thickness');
  const shorthandStyle = style.getPropertyValue('text-decoration-style');
  const shorthandColor = style.getPropertyValue('text-decoration-color');
  style.setProperty('text-decoration', 'double overline underline dotted');
  const shorthandAfterInvalid = style.getPropertyValue('text-decoration');

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0].style;
  rule.setProperty('text-decoration-line', 'Grammar-Error');
  const ruleLine = rule.getPropertyValue('text-decoration-line');
  rule.setProperty('text-decoration-style', 'dashed');
  const ruleStyle = rule.getPropertyValue('text-decoration-style');
  rule.setProperty('text-decoration-style', 'solid wavy');
  const ruleStyleAfterInvalid = rule.getPropertyValue('text-decoration-style');
  rule.setProperty('text-decoration-skip-ink', 'none');
  const ruleSkipInk = rule.getPropertyValue('text-decoration-skip-ink');
  rule.setProperty('text-decoration-skip-spaces', 'all');
  const ruleSkipSpaces = rule.getPropertyValue('text-decoration-skip-spaces');
  rule.setProperty('text-underline-position', 'right from-font');
  const ruleUnderlinePosition = rule.getPropertyValue('text-underline-position');

  return [
    lineExposed,
    insetExposed,
    skipInkExposed,
    skipSpacesExposed,
    styleExposed,
    thicknessExposed,
    underlineOffsetExposed,
    underlinePositionExposed,
    CSS.supports('text-decoration', 'overline from-font dotted green'),
    CSS.supports('text-decoration', 'double overline underline dotted'),
    CSS.supports('text-decoration-inset', 'calc(1em / 4) calc(-1ch)'),
    CSS.supports('text-decoration-skip-ink', 'all'),
    CSS.supports('text-decoration-skip-ink', 'auto none'),
    CSS.supports('text-decoration-skip-spaces', 'end start'),
    CSS.supports('text-decoration-skip-spaces', 'all start'),
    CSS.supports('text-decoration-style', 'wavy'),
    CSS.supports('text-decoration-style', 'solid wavy'),
    CSS.supports('text-decoration-line', 'spelling-error'),
    CSS.supports('text-decoration-line', 'Grammar-Error'),
    CSS.supports('text-decoration-style', 'blink'),
    CSS.supports('text-underline-position', 'right under'),
    CSS.supports('text-underline-position', 'left right'),
    lineCanonical,
    lineAfterInvalid,
    spelling,
    commentWhitespace,
    skipInkCanonical,
    skipInkAfterInvalid,
    skipSpacesCanonical,
    skipSpacesAfterInvalid,
    insetCollapsed,
    insetMath,
    styleCanonical,
    styleAfterInvalid,
    invalidThickness,
    mathThickness,
    fromFont,
    invalidOffset,
    mathOffset,
    underlinePositionCanonical,
    underlinePositionAfterInvalid,
    shorthand,
    shorthandLine,
    shorthandThickness,
    shorthandStyle,
    shorthandColor,
    shorthandAfterInvalid,
    ruleLine,
    ruleStyle,
    ruleStyleAfterInvalid,
    ruleSkipInk,
    ruleSkipSpaces,
    ruleUnderlinePosition
  ].join('|');
})()
"#,
        )
        .expect("text decoration CSSOM parser probe should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|true|true|true|true|true|false|true|true|false|true|false|true|false|true|true|false|true|false|underline overline|underline overline|spelling-error|underline overline|all|all|start end|start end|0px|calc(0.25em) calc(-1ch)|wavy|wavy||calc(40% - 20px)|from-font||calc(45% - 0.3em)|under right|under right|overline from-font dotted green|overline|from-font|dotted|green|overline from-font dotted green|grammar-error|dashed|dashed|none|all|from-font right"
    );
}

#[test]
fn cssom_text_decoration_paint_and_webkit_text_stroke_parse() {
    let mut vm = new_storage_test_vm("https://cssom-fill-stroke.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('div').style;
  const exposed = [
    'text-decoration-fill' in style,
    'textDecorationFill' in style,
    'text-decoration-stroke' in style,
    'textDecorationStroke' in style,
    '-webkit-text-stroke' in style,
    'webkitTextStroke' in style,
    '-webkit-text-stroke-color' in style,
    'webkitTextStrokeColor' in style,
    '-webkit-text-stroke-width' in style,
    'webkitTextStrokeWidth' in style
  ].join(',');

  style.setProperty('text-decoration-fill', 'match-text');
  const fillMatchText = style.getPropertyValue('text-decoration-fill');
  style.setProperty('text-decoration-fill', 'rgb(12, 34, 56)');
  const fillColor = style.textDecorationFill;
  style.setProperty('text-decoration-fill', 'none red');
  const fillAfterInvalid = style.getPropertyValue('text-decoration-fill');

  style.setProperty('text-decoration-stroke', 'context-fill');
  const strokeContext = style.getPropertyValue('text-decoration-stroke');
  style.setProperty('text-decoration-stroke', 'auto');
  const strokeAfterInvalid = style.getPropertyValue('text-decoration-stroke');

  style.setProperty('-webkit-text-stroke', 'green');
  const webkitColorOnly = [
    style.getPropertyValue('-webkit-text-stroke'),
    style.getPropertyValue('-webkit-text-stroke-width'),
    style.getPropertyValue('-webkit-text-stroke-color')
  ].join(',');

  style.setProperty('-webkit-text-stroke', '3px');
  const webkitWidthOnly = [
    style.webkitTextStroke,
    style.webkitTextStrokeWidth,
    style.webkitTextStrokeColor
  ].join(',');

  style.setProperty('-webkit-text-stroke', '1px red');
  const webkitBoth = style.getPropertyValue('-webkit-text-stroke');

  return [
    exposed,
    CSS.supports('text-decoration-fill', 'match-text'),
    CSS.supports('text-decoration-fill', 'none red'),
    CSS.supports('text-decoration-stroke', 'url("https://example.com/") rgb(12, 34, 56)'),
    CSS.supports('-webkit-text-stroke', '1px red'),
    CSS.supports('-webkit-text-stroke', '1px 2px red'),
    fillMatchText,
    fillColor,
    fillAfterInvalid,
    strokeContext,
    strokeAfterInvalid,
    webkitColorOnly,
    webkitWidthOnly,
    webkitBoth
  ].join('|');
})()
"#,
        )
        .expect("fill/stroke CSSOM parser probe should evaluate");

    assert_eq!(
        result,
        "true,true,true,true,true,true,true,true,true,true|true|false|true|true|false|match-text|rgb(12, 34, 56)|rgb(12, 34, 56)|context-fill|context-fill|0px green,0px,green|3px currentcolor,3px,currentcolor|1px red"
    );
}

#[test]
fn cssom_text_shadow_accepts_valid_values_and_rejects_invalid_ones() {
    let mut vm = new_storage_test_vm("https://cssom-text-shadow.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('div').style;
  const exposed = 'text-shadow' in style && 'textShadow' in style;

  style.setProperty('text-shadow', 'none');
  const none = style.getPropertyValue('text-shadow');

  style.textShadow = '10px 20px 30px lime';
  const colorLast = style.getPropertyValue('text-shadow');

  style.setProperty('text-shadow', 'calc(1em + 2px) calc(3em + 4px) calc(5em + 6px)');
  const math = style.textShadow;

  style.setProperty('text-shadow', '10px 20px, 30px 40px');
  const list = style.getPropertyValue('text-shadow');

  style.setProperty('text-shadow', '10px 20px -1px');
  const afterInvalid = style.getPropertyValue('text-shadow');

  return [
    exposed,
    CSS.supports('text-shadow', '10px 20px 30px lime'),
    CSS.supports('text-shadow', '10px 20px -1px'),
    none,
    colorLast,
    math,
    list,
    afterInvalid
  ].join('|');
})()
"#,
        )
        .expect("text-shadow CSSOM parser probe should evaluate");

    assert_eq!(
        result,
        "true|true|false|none|lime 10px 20px 30px|calc(1em + 2px) calc(3em + 4px) calc(5em + 6px)|10px 20px, 30px 40px|10px 20px, 30px 40px"
    );
}

#[test]
fn cssom_text_emphasis_properties_parse_and_normalize() {
    let mut vm = new_storage_test_vm("https://cssom-text-emphasis.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('div').style;
  const shorthandExposed = 'text-emphasis' in style && 'textEmphasis' in style;
  const colorExposed = 'text-emphasis-color' in style && 'textEmphasisColor' in style;
  const positionExposed = 'text-emphasis-position' in style && 'textEmphasisPosition' in style;
  const styleExposed = 'text-emphasis-style' in style && 'textEmphasisStyle' in style;

  style.setProperty('text-emphasis-style', 'open sesame');
  const emphasisStyle = style.getPropertyValue('text-emphasis-style');
  style.setProperty('text-emphasis-style', 'filled open');
  const emphasisStyleAfterInvalid = style.getPropertyValue('text-emphasis-style');

  style.setProperty('text-emphasis-position', 'right under');
  const emphasisPosition = style.getPropertyValue('text-emphasis-position');
  style.setProperty('text-emphasis-position', 'left right');
  const emphasisPositionAfterInvalid = style.getPropertyValue('text-emphasis-position');

  style.setProperty('text-emphasis', 'dot red');
  const shorthand = style.getPropertyValue('text-emphasis');
  const shorthandStyle = style.getPropertyValue('text-emphasis-style');
  const shorthandColor = style.getPropertyValue('text-emphasis-color');
  style.setProperty('text-emphasis', 'filled open');
  const shorthandAfterInvalid = style.getPropertyValue('text-emphasis');

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0].style;
  rule.setProperty('text-emphasis-position', 'over left');
  const rulePosition = rule.getPropertyValue('text-emphasis-position');
  rule.setProperty('text-emphasis', 'dot red');
  const ruleStyle = rule.getPropertyValue('text-emphasis-style');
  const ruleColor = rule.getPropertyValue('text-emphasis-color');

  return [
    shorthandExposed,
    colorExposed,
    positionExposed,
    styleExposed,
    CSS.supports('text-emphasis-style', 'open sesame'),
    CSS.supports('text-emphasis-style', 'filled open'),
    CSS.supports('text-emphasis-position', 'right under'),
    CSS.supports('text-emphasis-position', 'left right'),
    CSS.supports('text-emphasis', 'dot red'),
    CSS.supports('text-emphasis', 'filled open'),
    emphasisStyle,
    emphasisStyleAfterInvalid,
    emphasisPosition,
    emphasisPositionAfterInvalid,
    shorthand,
    shorthandStyle,
    shorthandColor,
    shorthandAfterInvalid,
    rulePosition,
    ruleStyle,
    ruleColor
  ].join('|');
})()
"#,
        )
        .expect("text-emphasis CSSOM parser probe should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|true|false|true|false|true|false|open sesame|open sesame|under|under|dot red|dot|red|dot red|over left|dot|red"
    );
}

#[test]
fn inline_style_declaration_preserves_url_base_across_unrelated_mutation() {
    let mut vm = new_storage_test_vm("https://inline-style-base.test/cssom/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const elem = document.createElement('div');
  elem.setAttribute('style', 'background-image: url(./);');
  const style = elem.style;
  const attrOnly = document.createElement('div');
  attrOnly.setAttribute('style', 'background-image: url(./);');
  const removedAttr = document.createElement('div');
  removedAttr.setAttribute('style', 'background-image: url(./);');
  const base = document.createElement('base');
  base.href = '/';
  body.appendChild(elem);
  body.appendChild(attrOnly);
  body.appendChild(removedAttr);
  const original = getComputedStyle(elem).backgroundImage;
  const removedAttrOriginal = getComputedStyle(removedAttr).backgroundImage;

  head.appendChild(base);
  style.setProperty('background-color', 'green');
  const unrelatedMutation = getComputedStyle(elem).backgroundImage;
  attrOnly.setAttribute('style', 'background-image: url(./);');
  const attrOnlyNoOpImage = getComputedStyle(attrOnly).backgroundImage;
  removedAttr.removeAttribute('style');
  removedAttr.setAttribute('style', 'background-image: url(./);');
  const removedAttrReaddedImage = getComputedStyle(removedAttr).backgroundImage;
  style.setProperty('background-image', 'url(./)');
  const replacedImage = getComputedStyle(elem).backgroundImage;
  const attrOnlyImage = getComputedStyle(attrOnly).backgroundImage;
  base.remove();
  elem.remove();
  attrOnly.remove();
  removedAttr.remove();

  return [
    original,
    unrelatedMutation,
    attrOnlyNoOpImage,
    removedAttrOriginal,
    removedAttrReaddedImage,
    replacedImage,
    attrOnlyImage,
    original === unrelatedMutation,
    original === replacedImage,
    removedAttrOriginal === removedAttrReaddedImage
  ].join('|');
})()
"#,
        )
        .expect("inline style URL base mutation probe should evaluate");

    assert_eq!(
        result,
        "url(\"https://inline-style-base.test/cssom/\")|url(\"https://inline-style-base.test/cssom/\")|url(\"https://inline-style-base.test/cssom/\")|url(\"https://inline-style-base.test/cssom/\")|url(\"https://inline-style-base.test/\")|url(\"https://inline-style-base.test/\")|url(\"https://inline-style-base.test/cssom/\")|true|false|false"
    );
}
#[test]
fn inline_style_base_side_table_tracks_style_attribute_lifecycle() {
    let mut vm = new_storage_test_vm("https://inline-style-base-lifecycle.test/cssom/page.html");
    let document = vm.document_handle_for_test();
    let initial_base_count = vm.inline_style_base_url_count_for_document_for_test(document);

    let loop_result = vm
        .eval(
            r#"
(() => {
  const body = document.body || document.documentElement || document.appendChild(document.createElement('body'));
  for (let i = 0; i < 25; i++) {
    const elem = document.createElement('div');
    void elem.style;
    body.appendChild(elem);
    getComputedStyle(elem).color;
    elem.remove();
  }
  return 'done';
})()
"#,
        )
        .expect("empty inline style lifecycle probe should evaluate");

    assert_eq!(loop_result, "done");
    assert_eq!(
        vm.inline_style_base_url_count_for_document_for_test(document),
        initial_base_count
    );
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );

    let set_result = vm
        .eval(
            r#"
(() => {
  const body = document.body || document.documentElement || document.appendChild(document.createElement('body'));
  globalThis.__inlineStyleLifecycleTarget = document.createElement('div');
  body.appendChild(globalThis.__inlineStyleLifecycleTarget);
  globalThis.__inlineStyleLifecycleTarget.setAttribute('style', 'background-image: url(./);');
  void globalThis.__inlineStyleLifecycleTarget.style;
  return getComputedStyle(globalThis.__inlineStyleLifecycleTarget).backgroundImage;
})()
"#,
        )
        .expect("inline style base side table setup should evaluate");

    assert_eq!(
        set_result,
        "url(\"https://inline-style-base-lifecycle.test/cssom/\")"
    );
    assert_eq!(
        vm.inline_style_base_url_count_for_document_for_test(document),
        initial_base_count + 1
    );

    let remove_result = vm
        .eval(
            r#"
(() => {
  globalThis.__inlineStyleLifecycleTarget.removeAttribute('style');
  const value = getComputedStyle(globalThis.__inlineStyleLifecycleTarget).backgroundImage;
  globalThis.__inlineStyleLifecycleTarget.remove();
  delete globalThis.__inlineStyleLifecycleTarget;
  return value;
})()
"#,
        )
        .expect("inline style base side table cleanup should evaluate");

    assert_eq!(remove_result, "none");
    assert_eq!(
        vm.inline_style_base_url_count_for_document_for_test(document),
        initial_base_count
    );
}

#[test]
fn inline_style_base_side_table_moves_with_adopted_element_owner_document() {
    let mut vm = new_storage_test_vm("https://inline-style-adopt.test/cssom/page.html");
    let document = vm.document_handle_for_test();
    let initial_base_count = vm.inline_style_base_url_count_for_document_for_test(document);

    let setup_result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const target = document.createElement('div');
  target.id = 'inline-style-adopt-target';
  body.appendChild(target);
  target.style.backgroundImage = 'url(./asset.png)';
  const before = getComputedStyle(target).backgroundImage;
  const base = document.createElement('base');
  base.href = '/';
  head.appendChild(base);

  globalThis.__inlineStyleAdoptDocument = document.implementation.createHTMLDocument('');
  globalThis.__inlineStyleAdoptTarget = target;
  globalThis.__inlineStyleAdoptDocument.adoptNode(target);
  globalThis.__inlineStyleAdoptDocument.body.appendChild(target);
  return JSON.stringify({
    before,
    ownerIsDetached: target.ownerDocument === globalThis.__inlineStyleAdoptDocument
  });
})()
"#,
        )
        .expect("inline style metadata adoption setup should evaluate");

    assert_eq!(
        setup_result,
        r#"{"before":"url(\"https://inline-style-adopt.test/cssom/asset.png\")","ownerIsDetached":true}"#
    );
    let detached_document =
        cssom_owner_document_handle_for_element_id(&vm, "inline-style-adopt-target");
    assert_eq!(
        vm.inline_style_base_url_count_for_document_for_test(document),
        initial_base_count,
        "active document world must release adopted inline metadata"
    );
    assert_eq!(
        vm.inline_style_base_url_count_for_document_for_test(detached_document),
        1
    );

    let return_result = vm
        .eval(
            r#"
(() => {
  document.body.appendChild(globalThis.__inlineStyleAdoptTarget);
  const after = getComputedStyle(globalThis.__inlineStyleAdoptTarget).backgroundImage;
  const ownerIsActive = globalThis.__inlineStyleAdoptTarget.ownerDocument === document;
  delete globalThis.__inlineStyleAdoptTarget;
  delete globalThis.__inlineStyleAdoptDocument;
  return JSON.stringify({ after, ownerIsActive });
})()
"#,
        )
        .expect("inline style metadata adoption return should evaluate");

    assert_eq!(
        return_result,
        r#"{"after":"url(\"https://inline-style-adopt.test/cssom/asset.png\")","ownerIsActive":true}"#
    );
    assert_eq!(
        vm.inline_style_base_url_count_for_document_for_test(detached_document),
        0,
        "detached document world must release returned inline metadata"
    );
    assert_eq!(
        vm.inline_style_base_url_count_for_document_for_test(document),
        initial_base_count + 1
    );
}

#[test]
fn live_inline_style_round_trips_escaped_custom_property_names() {
    let mut vm = new_storage_test_vm("https://css-custom-property-names.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('span');
  target.style = '--a\\;b:value; --\\\\: other; --value: a\\;b;';
  const before = [
    target.style.length,
    target.style[0],
    target.style.getPropertyValue('--a;b'),
    target.style[1],
    target.style.getPropertyValue('--\\'),
    target.style[2],
    target.style.getPropertyValue('--value')
  ].join(',');
  target.style = target.style.cssText;
  const after = [
    target.style.cssText,
    target.style.length,
    target.style[0],
    target.style.getPropertyValue('--a;b'),
    target.style[1],
    target.style.getPropertyValue('--\\'),
    target.style[2],
    target.style.getPropertyValue('--value')
  ].join(',');
  const cssom = document.createElement('span');
  cssom.style.setProperty('--value', 'a;b');
  const cssomSemicolon = [
    cssom.style.length,
    cssom.style.cssText,
    cssom.style.getPropertyValue('--value')
  ].join(',');
  const cssomEscaped = document.createElement('span');
  cssomEscaped.style.setProperty('--value', 'a\\;b');
  const cssomEscapedSemicolon = [
    cssomEscaped.style.length,
    cssomEscaped.style.cssText,
    cssomEscaped.style.getPropertyValue('--value')
  ].join(',');
  const cssomBareBang = document.createElement('span');
  cssomBareBang.style.setProperty('--value', 'Hello\\; world!');
  const cssomBareBangValue = [
    cssomBareBang.style.length,
    cssomBareBang.style.cssText,
    cssomBareBang.style.getPropertyValue('--value')
  ].join(',');
  const cssomEscapedBang = document.createElement('span');
  cssomEscapedBang.style.setProperty('--value', 'Hello\\; world\\!');
  const cssomEscapedBangValue = [
    cssomEscapedBang.style.length,
    cssomEscapedBang.style.cssText,
    cssomEscapedBang.style.getPropertyValue('--value')
  ].join(',');
  return [before, after, cssomSemicolon, cssomEscapedSemicolon, cssomBareBangValue, cssomEscapedBangValue].join('|');
})()
"#,
        )
        .expect("escaped custom property names should evaluate");

    assert_eq!(
        result,
        r#"3,--a;b,value,--\,other,--value,a\;b|--a\;b: value; --\\: other; --value: a\;b;,3,--a;b,value,--\,other,--value,a\;b|0,,|1,--value: a\;b;,a\;b|0,,|1,--value: Hello\; world\!;,Hello\; world\!"#
    );
}

#[test]
fn custom_property_empty_values_and_invalid_cssom_names_match_cssom() {
    let mut vm = new_storage_test_vm("https://css-custom-property-empty-values.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const parent = document.createElement('div');
  parent.style.cssText = '--empty:; --space:  ; --overwrite:value;--overwrite:;';
  const child = document.createElement('div');
  parent.append(child);
  root.append(parent);

  const invalid = document.createElement('div');
  invalid.style.setProperty('--bad ', 'green');
  invalid.style.setProperty('--bad name', 'green');
  invalid.style.setProperty('--ok', 'green');
  root.append(invalid);

  const cssom = document.createElement('div');
  cssom.style.setProperty('--blank', '  ', 'important');
  const blankBeforeRemove = [
    cssom.style.length,
    cssom.style[0],
    cssom.style.getPropertyValue('--blank'),
    cssom.style.getPropertyPriority('--blank'),
    cssom.style.cssText
  ].join(',');
  cssom.style.setProperty('--blank', '');
  const blankAfterRemove = [
    cssom.style.length,
    cssom.style.getPropertyValue('--blank'),
    cssom.style.cssText
  ].join(',');

  const parentComputed = getComputedStyle(parent);
  const childComputed = getComputedStyle(child);
  const invalidComputed = getComputedStyle(invalid);
  const values = [
    parent.style.getPropertyValue('--empty'),
    parent.style.getPropertyValue('--space'),
    parent.style.getPropertyValue('--overwrite'),
    parentComputed.getPropertyValue('--empty'),
    parentComputed.getPropertyValue('--space'),
    parentComputed.getPropertyValue('--overwrite'),
    childComputed.getPropertyValue('--empty'),
    childComputed.getPropertyValue('--space'),
    childComputed.getPropertyValue('--overwrite'),
    invalid.style.getPropertyValue('--bad '),
    invalidComputed.getPropertyValue('--bad '),
    invalidComputed.getPropertyValue('--bad'),
    invalidComputed.getPropertyValue('--bad name'),
    invalidComputed.getPropertyValue('--ok'),
    invalidComputed.getPropertyValue('--ok '),
    blankBeforeRemove,
    blankAfterRemove
  ];
  parent.remove();
  invalid.remove();
  return values.map(value => JSON.stringify(value)).join('|');
})()
"#,
        )
        .expect("custom property empty values should evaluate");

    assert_eq!(
        result,
        r#"" "|" "|" "|" "|" "|" "|" "|" "|" "|""|""|""|""|"green"|""|"1,--blank, ,important,--blank:  !important;"|"0,,""#
    );
}

#[test]
fn custom_property_cycles_ignore_unused_fallback_references() {
    let mut vm = new_storage_test_vm("https://css-custom-property-unused-fallback-cycle.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const element = document.createElement('div');
  element.style.cssText = [
    '--x:var(--a, valid)',
    '--a:var(--y, var(--b, cycle))',
    '--b:var(--y, var(--c, cycle))',
    '--c:var(--y, var(--a, cycle))',
    '--y:valid'
  ].join(';');
  root.append(element);
  const style = getComputedStyle(element);
  const values = ['--a', '--b', '--c', '--x', '--y'].map(name => style.getPropertyValue(name));
  element.remove();
  return values.join('|');
})()
"#,
        )
        .expect("unused fallback cycle should evaluate");

    assert_eq!(result, "valid|valid|valid|valid|valid");
}

#[test]
fn custom_property_cycle_uses_nested_fallback_default() {
    let mut vm = new_storage_test_vm("https://css-custom-property-nested-fallback-cycle.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const element = document.createElement('div');
  element.style.cssText = [
    '--varA: var(--varB)',
    '--varB: var(--varA) var(--varDoesNotExist, var(--varC))',
    '--varC: var(--varB, 13px)'
  ].join(';');
  root.append(element);
  const style = getComputedStyle(element);
  const values = ['--varA', '--varB', '--varC'].map(name => style.getPropertyValue(name));
  element.remove();
  return values.join('|');
})()
"#,
        )
        .expect("nested fallback cycle should evaluate");

    assert_eq!(result, "||13px");
}

#[test]
fn custom_property_var_ident_function_resolves_and_falls_back() {
    let mut vm = new_storage_test_vm("https://css-custom-property-var-ident.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const element = document.createElement('div');
  element.style.cssText = [
    '--myprop3: PASS',
    '--var-with-ident-fn: FAIL1',
    '--var-with-ident-fn: var(ident("--myprop" calc(3 * sign(1em - 1px))), FAIL2)',
    '--nodash: var(ident("nodash"))',
    '--nodash-fallback: var(ident("nodash"), PASS)',
    '--nodash-fallback-inherit: var(ident("nodash"), inherit)'
  ].join(';');
  root.append(element);
  const computed = getComputedStyle(element);
  const values = [
    computed.getPropertyValue('--var-with-ident-fn'),
    computed.getPropertyValue('--nodash'),
    computed.getPropertyValue('--nodash-fallback'),
    computed.getPropertyValue('--nodash-fallback-inherit')
  ];
  element.remove();
  return values.join('|');
})()
"#,
        )
        .expect("custom property var ident function should evaluate");

    assert_eq!(result, "PASS||PASS|");
}

#[test]
fn computed_z_index_resolves_simple_custom_property_reference() {
    let mut vm = new_storage_test_vm("https://style-z-index-custom-property.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const target = document.createElement('div');
  root.append(target);
  target.style.zIndex = '1111111111111111111111111';
  const direct = getComputedStyle(target).zIndex;
  target.style.setProperty('--depth', '1111111111111111111111111');
  target.style.zIndex = 'var(--depth)';
  const large = getComputedStyle(target).zIndex;
  target.style.setProperty('--Depth', '42');
  target.style.zIndex = 'var(--Depth)';
  const caseSensitive = getComputedStyle(target).zIndex;
  target.style.zIndex = 'var(--missing)';
  const missing = getComputedStyle(target).zIndex;
  return [direct, large, caseSensitive, missing].join('|');
})()
"#,
        )
        .expect("computed z-index custom property should evaluate");

    assert_eq!(result, "2147483647|2147483647|42|auto");
}
#[test]
fn css_style_declaration_set_property_accepts_common_shorthands() {
    let mut vm = new_storage_test_vm("https://style-shorthand-set-property.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('span').style;
  const properties = [
    'font',
    'border-top',
    'border-right',
    'border-bottom',
    'border-left',
    'border-color',
    'border-style',
    'border-width',
    'background-repeat',
    'border-spacing',
    'list-style',
    'outline',
    'border-radius',
  ];
  for (const property of properties) {
    style.setProperty(property, 'initial');
    if (style.getPropertyValue(property) !== 'initial') {
      return property + ':set';
    }
    style.removeProperty(property);
    if (style.getPropertyValue(property) !== '') {
      return property + ':remove';
    }
    style.setProperty(property, 'initial', 'important');
    if (style.getPropertyValue(property) !== 'initial') {
      return property + ':important';
    }
    style.removeProperty(property);
  }
  return 'ok';
})()
"#,
        )
        .expect("CSSStyleDeclaration shorthand setProperty should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn css_style_inset_accessor_rejects_quirky_unitless_lengths() {
    let mut vm = new_parsed_test_vm(
        "https://cssom-inset-quirky-length.test/",
        "<html><body><div id=target></div></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.getElementById('target').style;
  const quirkyValues = [
    '1',
    '1 2px',
    '1px 2',
    '1 2',
    '1 2px 3px',
    '1px 2 3px',
    '1px 2px 3',
    '1 2 3',
    '1 2px 3px 4px',
    '1px 2 3px 4px',
    '1px 2px 3 4px',
    '1px 2px 3px 4',
    '1 2 3 4'
  ];

  style.inset = '5px 6px 7px 8px';
  for (const value of quirkyValues) {
    style.inset = value;
    if (style.inset !== '5px 6px 7px 8px') {
      return `accepted:${value}:${style.inset}`;
    }
  }

  return [
    document.compatMode,
    Object.prototype.hasOwnProperty.call(style, 'inset'),
    style.getPropertyValue('inset'),
    style.cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleDeclaration inset accessor should reject quirky lengths");

    assert_eq!(
        result,
        "BackCompat|false|5px 6px 7px 8px|inset: 5px 6px 7px 8px;"
    );
}

#[test]
fn css_style_excluded_properties_reject_quirky_unitless_lengths() {
    let mut vm = new_parsed_test_vm(
        "https://cssom-excluded-quirky-length.test/",
        "<html><body><div id=target></div></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.getElementById('target');
  const properties = [
    'background-blend-mode',
    'background-size',
    'box-shadow',
    'clip-path',
    'column-span',
    'filter',
    'mask',
    'object-position',
    'perspective-origin',
    'text-shadow',
    'transform-origin'
  ];

  if (!CSS.supports('mask', 'none') || CSS.supports('mask', 'banana')) {
    return 'mask-supports';
  }
  target.style.mask = 'none';
  if (target.style.mask !== 'none') {
    return `mask-valid:${target.style.mask}`;
  }
  target.style.mask = '1234';
  if (target.style.mask !== 'none') {
    return `mask-invalid:${target.style.mask}`;
  }

  for (const property of properties) {
    if (!getComputedStyle(target)[property]) {
      return `unsupported:${property}`;
    }
    target.style[property] = '1234';
    const value = target.style[property];
    if (value === '1234' || value === '1234px' || value === '1234px auto') {
      return `accepted:${property}:${value}`;
    }
    if (Object.prototype.hasOwnProperty.call(target.style, property)) {
      return `expando:${property}`;
    }
  }

  return `${document.compatMode}|ok`;
})()
"#,
        )
        .expect("excluded CSS properties should reject quirky unitless lengths");

    assert_eq!(result, "BackCompat|ok");
}

#[test]
fn css_style_declaration_serializes_font_variant_longhands() {
    let mut vm = new_storage_test_vm("https://style-font-variant-cssom.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const longhands = [
    'fontVariantLigatures',
    'fontVariantCaps',
    'fontVariantAlternates',
    'fontVariantNumeric',
    'fontVariantEastAsian',
    'fontVariantPosition',
    'fontVariantEmoji',
  ];
  const longhandProperties = [
    'font-variant-ligatures',
    'font-variant-caps',
    'font-variant-alternates',
    'font-variant-numeric',
    'font-variant-east-asian',
    'font-variant-position',
    'font-variant-emoji',
  ];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };

  const target = document.createElement('div');
  const read = () => [target.style.fontVariant, ...longhands.map((name) => target.style[name])].join(',');

  target.style.fontVariant = 'normal';
  const normal = read();

  target.removeAttribute('style');
  target.style.fontVariant = 'normal';
  target.style.fontVariantLigatures = 'none';
  const none = read();

  target.removeAttribute('style');
  target.style.fontVariant = 'normal';
  target.style.fontVariantCaps = 'small-caps';
  const caps = read();

  target.removeAttribute('style');
  target.style.fontVariant = 'normal';
  target.style.fontVariantLigatures = 'initial';
  const mixedCssWide = read();

  target.removeAttribute('style');
  target.style.fontVariant = 'normal';
  target.style.font = 'menu';
  const fontReset = read();

  eq('inline-basic', [normal, none, caps, mixedCssWide, fontReset].join('|'), 'normal,normal,normal,normal,normal,normal,normal,normal|none,none,normal,normal,normal,normal,normal,normal|small-caps,normal,small-caps,normal,normal,normal,normal,normal|,initial,normal,normal,normal,normal,normal,normal|,,,,,,,');

  function exercisePdbStyle(style, label, textOwner) {
    style.setProperty('font-variant', 'normal', 'important');
    style.setProperty('font-variant-caps', 'small-caps', 'important');
    style.setProperty('font-variant-alternates', 'historical-forms', 'important');
    eq(`${label}-variant`, style.getPropertyValue('font-variant'), 'small-caps historical-forms');
    eq(`${label}-priority`, style.getPropertyPriority('font-variant'), 'important');
    eq(`${label}-ligatures`, style.getPropertyValue('font-variant-ligatures'), 'normal');
    eq(`${label}-caps`, style.getPropertyValue('font-variant-caps'), 'small-caps');
    eq(`${label}-alternates`, style.getPropertyValue('font-variant-alternates'), 'historical-forms');
    hasAll(`${label}-names`, style, longhandProperties);
    if (textOwner && textOwner.cssText.includes('font-variant: small-caps historical-forms !important;') === false) {
      failures.push(`${label}-cssText:${textOwner.cssText}`);
    }
    const removed = style.removeProperty('font-variant');
    eq(`${label}-removed`, removed, 'small-caps historical-forms');
    eq(`${label}-after-remove`, style.getPropertyValue('font-variant'), '');
    eq(`${label}-caps-after-remove`, style.getPropertyValue('font-variant-caps'), '');

    style.setProperty('font-variant', 'normal', 'important');
    style.setProperty('font-variant-ligatures', 'none', 'important');
    eq(`${label}-none-variant`, style.getPropertyValue('font-variant'), 'none');
    eq(`${label}-none-priority`, style.getPropertyPriority('font-variant'), 'important');
    eq(`${label}-none-ligatures`, style.getPropertyValue('font-variant-ligatures'), 'none');
  }

  exercisePdbStyle(document.createElement('div').style, 'inline-pdb');

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  exercisePdbStyle(detachedDoc.createElement('div').style, 'detached-pdb');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  exercisePdbStyle(rule.style, 'rule-pdb', rule);

  const keyframe = sheet.cssRules[1].cssRules[0];
  exercisePdbStyle(keyframe.style, 'keyframe-pdb', keyframe);

  return failures.length ? failures.slice(0, 12).join('|') : 'PASS';
})()
"#,
        )
        .expect("font-variant CSSOM serialization should evaluate");

    assert_eq!(result, "PASS");
}

#[test]
fn font_shorthand_uses_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://font-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const fontLonghands = [
    'font-style',
    'font-variant-ligatures',
    'font-variant-caps',
    'font-variant-numeric',
    'font-variant-east-asian',
    'font-weight',
    'font-stretch',
    'font-size',
    'line-height',
    'font-family',
    'font-kerning'
  ];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };
  const lacks = (label, style, name) => {
    const actual = names(style);
    if (actual.includes(name)) failures.push(`${label}:unexpected:${name}:${actual.join(',')}`);
  };

  function exerciseFontStyle(style, label, textOwner) {
    style.setProperty('font-variant-caps', 'small-caps');
    style.setProperty('font', 'italic small-caps 700 16px / 2 Ahem', 'important');
    eq(`${label}-font`, style.getPropertyValue('font'), 'italic small-caps 700 16px / 2 Ahem');
    eq(`${label}-priority`, style.getPropertyPriority('font'), 'important');
    eq(`${label}-style`, style.getPropertyValue('font-style'), 'italic');
    eq(`${label}-caps`, style.getPropertyValue('font-variant-caps'), 'small-caps');
    eq(`${label}-weight`, style.getPropertyValue('font-weight'), '700');
    eq(`${label}-size`, style.getPropertyValue('font-size'), '16px');
    eq(`${label}-line-height`, style.getPropertyValue('line-height'), '2');
    eq(`${label}-family`, style.getPropertyValue('font-family'), 'Ahem');
    hasAll(`${label}-names`, style, fontLonghands);
    ok(`${label}-cssText`, style.cssText.includes('font: italic small-caps 700 16px / 2 Ahem !important;'));
    if (textOwner) {
      ok(`${label}-owner-cssText`, textOwner.cssText.includes('font: italic small-caps 700 16px / 2 Ahem !important;'));
    }

    style.setProperty('--token', 'value');
    style.setProperty('-webkit-text-fill-color', 'red');
    const removedFont = style.removeProperty('font');
    eq(`${label}-removed`, removedFont, 'italic small-caps 700 16px / 2 Ahem');
    eq(`${label}-font-after-remove`, style.getPropertyValue('font'), '');
    eq(`${label}-size-after-remove`, style.getPropertyValue('font-size'), '');
    lacks(`${label}-name-after-remove`, style, 'font-size');
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    style.setProperty('font', 'italic 12px / 1.5 "A B", serif');
    style.setProperty('font-size', '20px', 'important');
    eq(`${label}-font-after-longhand`, style.getPropertyValue('font'), '');
    eq(`${label}-size-after-longhand`, style.getPropertyValue('font-size'), '20px');
    eq(`${label}-size-priority-after-longhand`, style.getPropertyPriority('font-size'), 'important');
  }

  const inline = document.createElement('div').style;
  exerciseFontStyle(inline, 'inline');

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = detachedDoc.createElement('div').style;
  detached.cssText = 'font: italic small-caps 700 16px/2 Ahem !important; --token: value; -webkit-text-fill-color: red;';
  eq('detached-cssText-font', detached.getPropertyValue('font'), 'italic small-caps 700 16px / 2 Ahem');
  eq('detached-cssText-priority', detached.getPropertyPriority('font'), 'important');
  eq('detached-cssText-token', detached.getPropertyValue('--token'), 'value');
  eq('detached-cssText-webkit', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  exerciseFontStyle(detached, 'detached');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  exerciseFontStyle(rule.style, 'rule', rule);

  const keyframe = sheet.cssRules[1].cssRules[0];
  exerciseFontStyle(keyframe.style, 'keyframe', keyframe);

  return failures.length ? failures.slice(0, 16).join('|') : 'PASS';
})()
"#,
        )
        .expect("font shorthand PDB backing should evaluate");

    assert_eq!(result, "PASS");
}

#[test]
fn css_rule_family_exposes_cssom_rule_branding_and_style_rule_surface() {
    let mut vm = new_storage_test_vm("https://css-rule-family.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value === undefined ? 'undefined' : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const sheet = new CSSStyleSheet();
  const index = sheet.insertRule('.a, main > .b { color: red; margin: 0 !important; }');
  const rules = sheet.cssRules;
  const rule = rules.item(0);
  const before = [
    index,
    typeof CSSRule,
    typeof CSSRuleList,
    typeof CSSStyleRule,
    Object.prototype.toString.call(rules),
    rules instanceof CSSRuleList,
    Object.hasOwn(rules, 'item'),
    Object.prototype.toString.call(rule),
    rule instanceof CSSStyleRule,
    rule instanceof CSSRule,
    Object.getPrototypeOf(CSSStyleRule.prototype) === CSSGroupingRule.prototype
      && Object.getPrototypeOf(CSSGroupingRule.prototype) === CSSRule.prototype,
    CSSRule.STYLE_RULE,
    CSSRule.prototype.STYLE_RULE,
    rule.type,
    rule.cssText,
    rule.selectorText,
    Object.prototype.toString.call(rule.style),
    rule.style.getPropertyValue('color'),
    rule.style.getPropertyPriority('margin'),
    rule.parentStyleSheet === sheet,
    rule.parentRule === null
  ].join(',');
  rule.selectorText = '.c';
  const afterSelector = [rule.selectorText, rule.cssText].join(',');
  rule.cssText = '.d { display: block; }';
  const afterCssText = [
    rule.selectorText,
    rule.cssText,
    rule.style.getPropertyValue('display'),
    rule.type
  ].join(',');
  rule.style.setProperty('opacity', '0.5', 'important');
  const afterSetProperty = [
    rule.style.cssText,
    rule.cssText
  ].join(',');
  rule.style.display = 'inline';
  const afterNamedProperty = rule.cssText;
  rule.style.removeProperty('opacity');
  const afterRemoveProperty = rule.cssText;
  const selectorSymbol = probe(() => { rule.selectorText = Symbol(); });
  const afterSelectorSymbol = rule.selectorText;
  const selectorThrow = probe(() => { rule.selectorText = { toString() { throw new RangeError('selector'); } }; });
  rule.selectorText = null;
  const afterSelectorNull = [rule.selectorText, rule.cssText].join(',');
  const cssTextSymbol = probe(() => { rule.cssText = Symbol(); });
  const afterCssTextSymbol = rule.cssText;
  const cssTextThrow = probe(() => { rule.cssText = { toString() { throw new RangeError('rule'); } }; });
  rule.cssText = null;
  const afterCssTextNull = rule.cssText;
  const replaceSyncSymbol = probe(() => sheet.replaceSync(Symbol()));
  const replaceSyncThrow = probe(() => sheet.replaceSync({ toString() { throw new RangeError('replace'); } }));
  sheet.replaceSync('main { color: blue; } aside { display: none; }');
  const afterReplaceSync = [
    sheet.cssRules === rules,
    rules.length,
    rules[0].cssText,
    rules[0].parentStyleSheet === sheet,
    rules[1].cssText
  ].join(',');
  return [
    before,
    afterSelector,
    afterCssText,
    afterSetProperty,
    afterNamedProperty,
    afterRemoveProperty,
    [
      selectorSymbol,
      afterSelectorSymbol,
      selectorThrow,
      afterSelectorNull,
      cssTextSymbol,
      afterCssTextSymbol,
      cssTextThrow,
      afterCssTextNull,
      replaceSyncSymbol,
      replaceSyncThrow
    ].join(','),
    afterReplaceSync
  ].join('|');
})()
"#,
        )
        .expect("CSSRule family should expose CSSOM rule surface");

    assert_eq!(
        result,
        "0,function,function,function,[object CSSRuleList],true,false,[object CSSStyleRule],true,true,true,1,1,1,.a, main > .b { color: red; margin: 0px !important; },.a, main > .b,[object CSSStyleProperties],red,important,true,true|.c,.c { color: red; margin: 0px !important; }|.d,.d { display: block; },block,1|display: block; opacity: 0.5 !important;,.d { display: block; opacity: 0.5 !important; }|.d { opacity: 0.5 !important; display: inline; }|.d { display: inline; }|throw:TypeError,.d,throw:RangeError,null,null { display: inline; },throw:TypeError,null { display: inline; },throw:RangeError,null { display: inline; },throw:TypeError,throw:RangeError|true,2,main { color: blue; },true,aside { display: none; }"
    );
}

#[test]
fn css_style_rule_style_stores_border_css_wide_keyword() {
    let mut vm = new_storage_test_vm("https://css-rule-border-wide-keyword.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: unset; border: unset; }');
  const style = sheet.cssRules[0].style;
  return [
    style.getPropertyValue('color'),
    style.getPropertyValue('border'),
    style.getPropertyValue('border-left'),
    style.getPropertyValue('border-color'),
    style.getPropertyValue('border-right-style')
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleRule style should preserve border CSS-wide keyword");

    assert_eq!(result, "unset|unset|unset|unset|unset");
}

#[test]
fn css_stylesheet_constructor_initializes_media_list_option() {
    let mut vm = new_storage_test_vm("https://css-sheet-init-media.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet({ disabled: true, media: 'screen, print' });
  const before = [
    sheet.disabled,
    sheet.media.length,
    sheet.media.item(0),
    sheet.media.item(1),
    sheet.media.mediaText
  ].join(',');
  sheet.media.appendMedium('speech');
  return [before, sheet.media.length, sheet.media.mediaText].join('|');
})()
"#,
        )
        .expect("CSSStyleSheet constructor media option should evaluate");

    assert_eq!(
        result,
        "true,2,screen,print,screen, print|3|screen, print, speech"
    );
}

#[test]
fn stylesheet_runtime_state_does_not_reflect_back_to_owner_attributes() {
    let mut vm = new_storage_test_vm("https://stylesheet-runtime-state-authority.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const style = document.createElement('style');
  style.media = 'screen';
  style.textContent = 'body { color: red; }';
  head.appendChild(style);

  const sheet = style.sheet;
  style.disabled = true;
  const disabledFromOwner = [
    style.disabled,
    sheet.disabled,
    style.hasAttribute('disabled'),
    style.sheet === sheet
  ];
  sheet.disabled = false;
  const enabledFromSheet = [
    style.disabled,
    sheet.disabled,
    style.hasAttribute('disabled'),
    style.sheet === sheet
  ];

  const media = sheet.media;
  media.mediaText = 'print';
  const mediaFromSheet = [style.media, media.mediaText, sheet.media === media];
  style.media = 'speech';
  const mediaFromOwner = [style.media, media.mediaText, sheet.media === media];
  sheet.media = 'screen and (min-width: 1px)';
  const assignedMedia = [style.media, media.mediaText, sheet.media === media];

  return JSON.stringify({
    disabledFromOwner,
    enabledFromSheet,
    mediaFromSheet,
    mediaFromOwner,
    assignedMedia
  });
})()
"#,
        )
        .expect("stylesheet runtime-state authority probe should evaluate");

    assert_eq!(
        result,
        r#"{"disabledFromOwner":[true,true,false,true],"enabledFromSheet":[false,false,false,true],"mediaFromSheet":["screen","print",true],"mediaFromOwner":["speech","speech",true],"assignedMedia":["speech","screen and (min-width: 1px)",true]}"#
    );
}

#[test]
fn cssom_stylesheet_interfaces_expose_webidl_prototype_shape() {
    let mut vm = new_storage_test_vm("https://cssom-interface-shape.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.media = 'screen, print';
  (document.head || document.documentElement || document).appendChild(style);
  const sheet = style.sheet;
  const media = sheet.media;
  const styleSheets = document.styleSheets;
  const rules = sheet.cssRules;
  const descriptor = name => Object.getOwnPropertyDescriptor(globalThis[name], 'prototype');
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };
  return [
    typeof StyleSheet,
    sheet instanceof StyleSheet,
    sheet instanceof CSSStyleSheet,
    Object.getPrototypeOf(CSSStyleSheet) === StyleSheet,
    Object.getPrototypeOf(CSSStyleSheet.prototype) === StyleSheet.prototype,
    descriptor('StyleSheet').writable,
    descriptor('StyleSheetList').writable,
    descriptor('MediaList').writable,
    descriptor('CSSStyleSheet').writable,
    styleSheets instanceof StyleSheetList,
    Object.prototype.hasOwnProperty.call(styleSheets, 'length'),
    'length' in styleSheets,
    Object.prototype.hasOwnProperty.call(StyleSheetList.prototype, 'length'),
    Array.isArray(styleSheets),
    rules instanceof CSSRuleList,
    Object.prototype.hasOwnProperty.call(rules, 'length'),
    'length' in rules,
    Object.prototype.hasOwnProperty.call(CSSRuleList.prototype, 'length'),
    Array.isArray(rules),
    rules.length,
    'mediaText' in media,
    Object.prototype.hasOwnProperty.call(media, 'mediaText'),
    'length' in media,
    Object.prototype.hasOwnProperty.call(media, 'length'),
    'item' in media,
    Object.prototype.hasOwnProperty.call(media, 'item'),
    media.length,
    media.item(0),
    media.item(2) === null,
    String(media),
    throwsTypeError(() => CSSStyleSheet.prototype.cssRules),
    throwsTypeError(() => CSSRule.prototype.cssText),
    throwsTypeError(() => MediaList.prototype.item.call(null, 0))
  ].join('|');
})()
"#,
        )
        .expect("CSSOM stylesheet interface shape probe should evaluate");

    assert_eq!(
        result,
        "function|true|true|true|true|false|false|false|false|true|false|true|true|false|true|false|true|true|false|0|true|false|true|false|true|false|2|screen|true|screen, print|true|true|true"
    );
}
#[test]
fn lightweight_css_declarations_inherit_cssom_prototype_surface() {
    let mut vm = new_storage_test_vm("https://cssom-lightweight-style-shape.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = '@page { margin-top: 1px; }';
  (document.head || document.documentElement || document).appendChild(style);
  const pageStyle = style.sheet.cssRules[0].style;
  const descriptor = name => Object.getOwnPropertyDescriptor(globalThis[name], 'prototype');
  return [
    Object.getPrototypeOf(CSSStyleProperties) === CSSStyleDeclaration,
    Object.getPrototypeOf(CSSFontFaceDescriptors) === CSSStyleDeclaration,
    Object.getPrototypeOf(CSSPageDescriptors) === CSSStyleDeclaration,
    descriptor('CSSStyleDeclaration').writable,
    descriptor('CSSStyleProperties').writable,
    descriptor('CSSFontFaceDescriptors').writable,
    descriptor('CSSPageDescriptors').writable,
    pageStyle instanceof CSSPageDescriptors,
    pageStyle instanceof CSSStyleDeclaration,
    Object.getPrototypeOf(pageStyle) === CSSPageDescriptors.prototype,
    Object.prototype.hasOwnProperty.call(pageStyle, 'cssText'),
    'cssText' in pageStyle,
    Object.prototype.hasOwnProperty.call(CSSPageDescriptors.prototype, 'cssText'),
    Object.prototype.hasOwnProperty.call(pageStyle, 'marginTop'),
    'marginTop' in pageStyle,
    Object.prototype.hasOwnProperty.call(CSSPageDescriptors.prototype, 'marginTop'),
    Object.prototype.hasOwnProperty.call(CSSPageDescriptors.prototype, 'marks'),
    Object.prototype.hasOwnProperty.call(CSSPageDescriptors.prototype, 'bleed'),
    Object.prototype.hasOwnProperty.call(pageStyle, 'getPropertyValue'),
    'getPropertyValue' in pageStyle,
    Object.prototype.hasOwnProperty.call(CSSPageDescriptors.prototype, 'getPropertyValue'),
    pageStyle.getPropertyValue('margin-top'),
    pageStyle.marginTop,
    pageStyle.cssText
  ].join('|');
})()
"#,
        )
        .expect("lightweight CSS declaration prototype shape should evaluate");

    assert_eq!(
        result,
        "true|true|true|false|false|false|false|true|true|true|false|true|true|false|true|true|true|true|false|true|true|1px|1px|margin-top: 1px;"
    );
}

#[test]
fn css_style_property_accessors_reject_prototype_receivers() {
    let mut vm = new_storage_test_vm("https://cssom-style-accessor-receiver.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const style = document.createElement('style');
  style.textContent = '@page { margin-top: 1px; }';
  (document.head || document.documentElement || document).appendChild(style);
  const pageStyle = style.sheet.cssRules[0].style;
  const inlineStyle = document.createElement('div').style;
  inlineStyle.cssFloat = 'left';
  return [
    pageStyle.marginTop,
    probe(() => CSSPageDescriptors.prototype.marginTop),
    probe(() => { CSSPageDescriptors.prototype.marginTop = '2px'; return 'set'; }),
    inlineStyle.cssFloat,
    probe(() => CSSStyleProperties.prototype.cssFloat),
    probe(() => { CSSStyleProperties.prototype.cssFloat = 'right'; return 'set'; })
  ].join('|');
})()
"#,
        )
        .expect("CSS style property receiver checks should evaluate");

    assert_eq!(
        result,
        "1px|throw:TypeError|throw:TypeError|left|throw:TypeError|throw:TypeError"
    );
}

#[test]
fn css_style_property_accessors_keep_webidl_descriptors_when_template_installed() {
    let mut vm = new_storage_test_vm("https://cssom-style-accessor-template.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const describe = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      descriptor.get.name,
      descriptor.get.length,
      descriptor.set.name,
      descriptor.set.length,
      descriptor.enumerable,
      descriptor.configurable
    ].join(',');
  };
  return [
    describe(CSSStyleDeclaration.prototype, 'colorAdjust'),
    describe(CSSStyleProperties.prototype, 'color'),
    describe(CSSFontFaceDescriptors.prototype, 'fontFamily'),
    describe(CSSPageDescriptors.prototype, 'marginTop')
  ].join('|');
})()
"#,
        )
        .expect("template-installed CSS accessors should preserve WebIDL descriptors");

    assert_eq!(
        result,
        concat!(
            "get colorAdjust,0,set colorAdjust,1,true,true|",
            "get color,0,set color,1,true,true|",
            "get fontFamily,0,set fontFamily,1,true,true|",
            "get marginTop,0,set marginTop,1,true,true",
        )
    );
}

#[test]
fn constructed_css_stylesheets_reject_or_ignore_import_rules() {
    let mut vm = new_storage_test_vm("https://css-constructed-import.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value === undefined ? 'undefined' : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const sheet = new CSSStyleSheet();
  const insert = probe(() => sheet.insertRule('@import url("ignored.css");'));
  sheet.replaceSync('@import url("ignored.css"); .target { color: blue; }');
  const syncRules = [sheet.cssRules.length, sheet.cssRules[0].cssText].join(',');
  globalThis.__constructedImportProbe = [];
  sheet.replace('@import url("ignored.css"); .next { color: green; }').then(
    value => globalThis.__constructedImportProbe.push([
      value === sheet,
      sheet.cssRules.length,
      sheet.cssRules[0].cssText
    ].join(',')),
    error => globalThis.__constructedImportProbe.push(`reject:${error && error.name}`)
  );
  return [insert, syncRules].join('|');
})()
"#,
        )
        .expect("constructed CSSStyleSheet import handling should evaluate");

    let async_result = vm
        .eval("globalThis.__constructedImportProbe.join('|')")
        .expect("constructed CSSStyleSheet replace promise should settle");

    assert_eq!(result, "throw:SyntaxError|1,.target { color: blue; }");
    assert_eq!(async_result, "true,1,.next { color: green; }");
}
#[test]
fn constructed_css_stylesheet_insert_and_delete_rules_match_cssom_defaults() {
    let mut vm = new_storage_test_vm("https://css-constructed-rule-mutation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const red = '.red { color: red; }';
  const adjacent = '.red + span + span { color: red; }';
  const sheet = new CSSStyleSheet({ disabled: true, media: 'screen, print' });
  const firstInsert = sheet.insertRule(red);
  const secondInsert = sheet.insertRule(adjacent);
  const sheet2 = new CSSStyleSheet({});
  sheet2.insertRule(adjacent);
  sheet2.deleteRule(0);
  const sheet3 = new CSSStyleSheet();
  sheet3.insertRule(adjacent);
  sheet3.deleteRule(0);
  return [
    document.adoptedStyleSheets.length,
    sheet.ownerNode === null,
    sheet.ownerRule === null,
    sheet2.media.length,
    sheet3.media.length,
    firstInsert,
    secondInsert,
    sheet.cssRules.length,
    sheet.cssRules[0].cssText,
    sheet2.cssRules.length,
    sheet3.cssRules.length
  ].join('|');
})()
"#,
        )
        .expect("constructed CSSStyleSheet rule mutation should evaluate");

    assert_eq!(
        result,
        "0|true|true|0|0|0|0|2|.red + span + span { color: red; }|0|0"
    );
}

#[test]
fn constructed_css_stylesheet_insert_rule_materializes_stylo_mutation_children() {
    let mut vm = new_storage_test_vm("https://css-constructed-insert-rule-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.one { color: red; }');
  const existing = sheet.cssRules[0];
  const index = sheet.insertRule('@media screen { .two { padding: 0 1px; } }', 1);
  const media = sheet.cssRules[1];
  return [
    index,
    sheet.cssRules.length,
    sheet.cssRules[0] === existing,
    media instanceof CSSMediaRule,
    media.cssRules.length,
    media.cssRules[0].cssText,
    media.cssText,
  ].join('|');
})()
"#,
        )
        .expect("constructed insertRule Stylo mutation view should evaluate");

    assert_eq!(
        result,
        "1|2|true|true|1|.two { padding: 0px 1px; }|@media screen {\n  .two { padding: 0px 1px; }\n}"
    );
}

#[test]
fn constructed_css_stylesheet_live_rules_refresh_after_rule_style_mutation() {
    let mut vm = new_storage_test_vm("https://css-constructed-live-rules-refresh.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.one { color: red; }');
  const first = sheet.cssRules[0];
  first.style.setProperty('color', 'blue');
  const index = sheet.insertRule('.two { margin: 0; }', 1);
  return [
    index,
    sheet.cssRules.length,
    sheet.cssRules[0] === first,
    sheet.cssRules[0].cssText,
    sheet.cssRules[1].cssText,
  ].join('|');
})()
"#,
        )
        .expect("constructed live rules should refresh after rule style mutation");

    assert_eq!(
        result,
        "1|2|true|.one { color: blue; }|.two { margin: 0px; }"
    );
}

#[test]
fn constructed_css_stylesheet_delete_rule_uses_stylo_remove_semantics() {
    let mut vm = new_storage_test_vm("https://css-constructed-delete-rule-tree.test/");

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
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@namespace svg url("http://www.w3.org/2000/svg"); .one { color: red; } @media screen { .two { margin: 0; } }');
  const namespaceDelete = probe(() => sheet.deleteRule(0));
  const namespaceRemove = probe(() => sheet.removeRule(0));
  const media = sheet.cssRules[2];
  sheet.deleteRule(1);
  return [
    namespaceDelete,
    namespaceRemove,
    sheet.cssRules.length,
    sheet.cssRules[1] === media,
    sheet.cssRules[1].cssRules[0].cssText,
    Array.from(sheet.cssRules).map(rule => rule.cssText).join(' / '),
  ].join('|');
})()
"#,
        )
        .expect("constructed deleteRule Stylo mutation view should evaluate");

    assert_eq!(
        result,
        "InvalidStateError|InvalidStateError|2|true|.two { margin: 0px; }|@namespace svg url(\"http://www.w3.org/2000/svg\"); / @media screen {\n  .two { margin: 0px; }\n}"
    );
}

#[test]
fn constructed_stylesheet_replacement_retires_rule_wrappers_without_rebinding() {
    let mut vm = new_storage_test_vm("https://css-constructed-replace-rule-identity.test/");

    crate::context_bootstrap::css_stylesheet_runtime::reset_detached_rule_mutation_count_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  const source = '@media screen { .same { color: red; } }';
  sheet.replaceSync(source);
  const oldMedia = sheet.cssRules[0];
  const oldRule = oldMedia.cssRules[0];
  oldMedia.expando = 'media';
  oldRule.expando = 'rule';

  sheet.replaceSync(source);
  oldRule.style.color = 'blue';
  oldMedia.insertRule('.old-only { color: green; }');

  return JSON.stringify({
    sameMedia: oldMedia === sheet.cssRules[0],
    sameRule: oldRule === sheet.cssRules[0].cssRules[0],
    mediaParent: oldMedia.parentStyleSheet === sheet,
    ruleParent: oldRule.parentStyleSheet === sheet,
    parentRule: oldRule.parentRule === oldMedia,
    oldLength: oldMedia.cssRules.length,
    oldText: oldMedia.cssText,
    currentText: sheet.cssRules[0].cssText,
    mediaExpando: oldMedia.expando,
    ruleExpando: oldRule.expando,
  });
})()
"#,
        )
        .expect("replaceSync should retire the previous native rule tree");

    assert_eq!(
        result,
        r#"{"sameMedia":false,"sameRule":false,"mediaParent":true,"ruleParent":true,"parentRule":true,"oldLength":2,"oldText":"@media screen {\n  .old-only { color: green; }\n  .same { color: blue; }\n}","currentText":"@media screen {\n  .same { color: red; }\n}","mediaExpando":"media","ruleExpando":"rule"}"#
    );
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::detached_rule_mutation_count_for_test(),
        1
    );
}

#[test]
fn attached_rule_sync_writes_snapshot_only_when_the_rule_detaches() {
    crate::context_bootstrap::css_stylesheet_runtime::reset_detached_css_rule_snapshot_write_count_for_test();
    let mut vm = new_storage_test_vm("https://css-attached-rule-snapshot.test/");

    let attached_text = vm
        .eval(
            r#"
globalThis.__snapshotSheet = new CSSStyleSheet();
__snapshotSheet.replaceSync('.before { color: red; }');
globalThis.__snapshotRule = __snapshotSheet.cssRules[0];
__snapshotRule.selectorText = '.after';
__snapshotRule.style.color = 'blue';
__snapshotRule.cssText = 'not a valid style rule';
__snapshotRule.cssText;
"#,
        )
        .expect("attached CSS rule mutation should evaluate");
    assert_eq!(attached_text, ".after { color: blue; }");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::detached_css_rule_snapshot_write_count_for_test(),
        0,
        "attached synchronization must not retain a full cssText snapshot"
    );

    let detached_text = vm
        .eval("__snapshotSheet.deleteRule(0); __snapshotRule.cssText;")
        .expect("detaching the CSS rule should freeze its current native state");
    assert_eq!(detached_text, ".after { color: blue; }");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::detached_css_rule_snapshot_write_count_for_test(),
        1,
        "detach should freeze exactly one snapshot for the retained wrapper"
    );

    let writes_before_detached_mutation =
        crate::context_bootstrap::css_stylesheet_runtime::detached_css_rule_snapshot_write_count_for_test();
    let mutated_text = vm
        .eval("__snapshotRule.style.color = 'green'; __snapshotRule.cssText;")
        .expect("detached CSS rule should remain independently mutable");
    assert_eq!(mutated_text, ".after { color: green; }");
    assert!(
        crate::context_bootstrap::css_stylesheet_runtime::detached_css_rule_snapshot_write_count_for_test()
            > writes_before_detached_mutation
    );
}

#[test]
fn deleted_rule_subtree_becomes_independent_and_clears_stylesheet_parent() {
    let mut vm = new_storage_test_vm("https://css-deleted-rule-subtree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@media screen { .old { color: red; } }');
  const media = sheet.cssRules[0];
  const child = media.cssRules[0];

  sheet.deleteRule(0);
  child.style.color = 'blue';
  media.insertRule('.detached { color: green; }');

  return JSON.stringify({
    sheetLength: sheet.cssRules.length,
    mediaParent: media.parentStyleSheet === null,
    childParent: child.parentStyleSheet === null,
    childParentRule: child.parentRule === media,
    oldLength: media.cssRules.length,
    oldText: media.cssText,
  });
})()
"#,
        )
        .expect("deleted CSS rule subtree should retain independent CSSOM state");

    assert_eq!(
        result,
        r#"{"sheetLength":0,"mediaParent":true,"childParent":true,"childParentRule":true,"oldLength":2,"oldText":"@media screen {\n  .detached { color: green; }\n  .old { color: blue; }\n}"}"#
    );
}

#[test]
fn deleted_large_grouping_rule_keeps_detached_children_sparse() {
    let mut vm = new_storage_test_vm("https://css-deleted-large-rule-subtree.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  const children = Array.from(
    { length: 1000 },
    (_, index) => `.rule-${index} { --index: ${index}; }`
  ).join('\n');
  sheet.replaceSync(`@media screen { ${children} }`);
  const media = sheet.cssRules[0];
  const rules = media.cssRules;
  globalThis.__detachedLargeSheet = sheet;
  globalThis.__detachedLargeMedia = media;
  globalThis.__detachedLargeRules = rules;
  globalThis.__detachedLargeFirst = rules[0];
  globalThis.__detachedLargeMiddle = rules[500];
})()
"#,
    )
    .expect("large detached grouping fixture should initialize");

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_wrapper_construction_count_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = globalThis.__detachedLargeSheet;
  const media = globalThis.__detachedLargeMedia;
  const rules = globalThis.__detachedLargeRules;
  const first = globalThis.__detachedLargeFirst;
  const middle = globalThis.__detachedLargeMiddle;
  sheet.deleteRule(0);
  const inserted = media.insertRule('.inserted { color: green; }', 1);
  return [
    sheet.cssRules.length,
    rules.length,
    inserted,
    first.parentStyleSheet === null,
    first.parentRule === media,
    middle.parentStyleSheet === null,
    rules[501] === middle,
  ].join('|');
})()
"#,
        )
        .expect("large detached grouping mutation should remain sparse");

    assert_eq!(result, "0|1001|1|true|true|true|true");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        0,
        "detach and detached insertion must not materialize untouched child wrappers"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const media = globalThis.__detachedLargeMedia;
  const rules = globalThis.__detachedLargeRules;
  const inserted = rules[1];
  const tail = rules[1000];
  return [
    inserted.cssText,
    inserted.parentStyleSheet === null,
    inserted.parentRule === media,
    tail.cssText,
    rules[1] === inserted,
    rules[1000] === tail,
  ].join('|');
})()
"#,
        )
        .expect("detached children should materialize on indexed access");

    assert_eq!(
        result,
        ".inserted { color: green; }|true|true|.rule-999 { --index: 999; }|true|true"
    );
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        2,
        "only explicitly accessed detached child wrappers should materialize"
    );
}

#[test]
fn deleted_large_keyframes_rule_keeps_detached_children_sparse() {
    let mut vm = new_storage_test_vm("https://css-deleted-large-keyframes.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  const frames = Array.from(
    { length: 1000 },
    (_, index) => `${(index / 10).toFixed(1)}% { --index: ${index}; }`
  ).join('\n');
  sheet.replaceSync(`@keyframes pulse { ${frames} }`);
  const keyframes = sheet.cssRules[0];
  const rules = keyframes.cssRules;
  globalThis.__detachedLargeKeyframesSheet = sheet;
  globalThis.__detachedLargeKeyframes = keyframes;
  globalThis.__detachedLargeKeyframeRules = rules;
  globalThis.__detachedLargeFirstFrame = rules[0];
  globalThis.__detachedLargeMiddleFrame = rules[500];
})()
"#,
    )
    .expect("large detached keyframes fixture should initialize");

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_wrapper_construction_count_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = globalThis.__detachedLargeKeyframesSheet;
  const keyframes = globalThis.__detachedLargeKeyframes;
  const rules = globalThis.__detachedLargeKeyframeRules;
  const first = globalThis.__detachedLargeFirstFrame;
  const middle = globalThis.__detachedLargeMiddleFrame;
  sheet.deleteRule(0);
  keyframes.appendRule('100% { opacity: 1; }');
  return [
    sheet.cssRules.length,
    rules.length,
    first.parentStyleSheet === null,
    first.parentRule === keyframes,
    middle.parentStyleSheet === null,
    rules[500] === middle,
  ].join('|');
})()
"#,
        )
        .expect("large detached keyframes mutation should remain sparse");

    assert_eq!(result, "0|1001|true|true|true|true");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        0,
        "detach and detached appendRule must not materialize untouched keyframes"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const keyframes = globalThis.__detachedLargeKeyframes;
  const rules = globalThis.__detachedLargeKeyframeRules;
  const found = keyframes.findRule('99.9%');
  return [found === rules[999], found.cssText].join('|');
})()
"#,
        )
        .expect("detached keyframe lookup should materialize only the match");

    assert_eq!(result, "true|99.9% { --index: 999; }");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        1,
        "detached findRule must search snapshots without materializing preceding keyframes"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const keyframes = globalThis.__detachedLargeKeyframes;
  const rules = globalThis.__detachedLargeKeyframeRules;
  const tail = rules[999];
  const appended = rules[1000];
  return [
    tail.cssText,
    tail.parentStyleSheet === null,
    tail.parentRule === keyframes,
    appended.cssText,
    appended.parentStyleSheet === null,
    appended.parentRule === keyframes,
  ].join('|');
})()
"#,
        )
        .expect("detached keyframes should materialize on indexed access");

    assert_eq!(
        result,
        "99.9% { --index: 999; }|true|true|100% { opacity: 1; }|true|true"
    );
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        2,
        "only explicitly accessed detached keyframes should materialize"
    );
}

#[test]
fn css_grouping_rule_insert_delete_materializes_stylo_mutation_children() {
    let mut vm = new_storage_test_vm("https://css-grouping-rule-mutation-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@media screen { .one { color: red; } }');
  const media = sheet.cssRules[0];
  const existing = media.cssRules[0];
  const index = media.insertRule('@supports (display: grid) { .two { display: grid; } }', 1);
  const supports = media.cssRules[1];
  media.deleteRule(0);
  return [
    index,
    media.cssRules.length,
    media.cssRules[0] === supports,
    existing.parentRule === null,
    supports instanceof CSSSupportsRule,
    supports.cssRules[0].cssText,
    media.cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSGroupingRule Stylo mutation view should evaluate");

    assert_eq!(
        result,
        "1|1|true|true|true|.two { display: grid; }|@media screen {\n  @supports (display: grid) {\n  .two { display: grid; }\n}\n}"
    );
}

#[test]
fn css_deep_grouping_rule_live_stylesheet_mutation_preserves_stylesheet_path() {
    let mut vm = new_storage_test_vm("https://css-deep-grouping-live-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@media screen { @supports (display: grid) { .one { color: red; } } }');
  const media = sheet.cssRules[0];
  const mediaRules = media.cssRules;
  const supports = mediaRules[0];
  const supportsRules = supports.cssRules;
  const old = supportsRules[0];

  const nestedIndex = supports.insertRule('.two { color: blue; }', 1);
  const topLevelIndex = sheet.insertRule('.after { margin: 0; }', 1);
  supports.deleteRule(0);
  const deleted = sheet.deleteRule(1);

  return [
    nestedIndex,
    topLevelIndex,
    deleted === undefined,
    sheet.cssRules.length,
    sheet.cssRules[0] === media,
    media.cssRules === mediaRules,
    media.cssRules[0] === supports,
    supports.cssRules === supportsRules,
    old.parentRule === null,
    supports.cssRules.length,
    supports.cssRules[0].cssText,
    supports.cssText,
    media.cssText,
    sheet.cssRules[0].cssText,
  ].join('|');
})()
"#,
        )
        .expect("deep CSSGroupingRule live mutation path should evaluate");

    assert_eq!(
        result,
        "1|1|true|1|true|true|true|true|true|1|.two { color: blue; }|@supports (display: grid) {\n  .two { color: blue; }\n}|@media screen {\n  @supports (display: grid) {\n  .two { color: blue; }\n}\n}|@media screen {\n  @supports (display: grid) {\n  .two { color: blue; }\n}\n}"
    );
}

#[test]
fn css_grouping_rule_css_text_reset_refreshes_existing_child_rule_list_from_stylo() {
    let mut vm = new_storage_test_vm("https://css-grouping-rule-css-text-reset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@media screen { .old { color: red; } }');
  const media = sheet.cssRules[0];
  const rules = media.cssRules;
  const old = rules[0];
  media.cssText = '@media screen { .new { color: blue; } @supports (display: grid) { .grid { display: grid; } } }';
  return [
    media.cssRules === rules,
    rules.length,
    old.parentRule === null,
    rules[0].selectorText,
    rules[0].cssText,
    rules[1] instanceof CSSSupportsRule,
    rules[1].cssRules[0].cssText,
    Array.from(rules).map(rule => rule.parentRule === media).join(','),
  ].join('|');
})()
"#,
        )
        .expect("CSSGroupingRule cssText reset should refresh existing child cssRules");

    assert_eq!(
        result,
        "true|2|true|.new|.new { color: blue; }|true|.grid { display: grid; }|true,true"
    );
}

#[test]
fn css_grouping_rule_css_text_reset_uses_native_rule_serialization() {
    let mut vm = new_storage_test_vm("https://css-grouping-rule-css-text-serialization.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@media screen { .old { color: red; } }');
  const media = sheet.cssRules[0];
  media.cssText = '@media screen { .new { margin: 0; } }';
  return [
    media.cssText,
    media.cssRules[0].cssText,
    sheet.cssRules[0].cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSGroupingRule cssText reset should use Stylo serialization");

    assert_eq!(
        result,
        "@media screen {\n  .new { margin: 0px; }\n}|.new { margin: 0px; }|@media screen {\n  .new { margin: 0px; }\n}"
    );
}

#[test]
fn css_grouping_rule_css_text_reset_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-grouping-css-text-live-reset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@supports (display: grid) { @media screen { .old { color: red; } } } .after { color: black; }');
  const supports = sheet.cssRules[0];
  const supportsRules = supports.cssRules;
  const oldMedia = supportsRules[0];

  supports.cssText = '@supports (display: flex) { @container card (min-width: 10px) { .new { margin: 0; } } }';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  const container = supports.cssRules[0];
  const child = container.cssRules[0];
  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === supports,
    supports.cssRules === supportsRules,
    oldMedia.parentRule === null,
    container instanceof CSSContainerRule,
    container.conditionText,
    child.cssText,
    supports.cssText,
    sheet.cssRules[0].cssText === supports.cssText,
    sheet.cssRules[1].cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSGroupingRule cssText reset should preserve live Stylo rule tree");

    assert_eq!(
        result,
        "2|true|true|true|true|card (min-width: 10px)|.new { margin: 0px; }|@supports (display: flex) {\n  @container card (min-width: 10px) {\n  .new { margin: 0px; }\n}\n}|true|.after { color: black; }"
    );
}

#[test]
fn css_rule_type_getter_uses_attached_native_rule_after_reset() {
    let mut vm = new_storage_test_vm("https://css-rule-type-live-view.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@supports (display: grid) { @media screen { .old { color: red; } } } .after { color: black; }');
  const supports = sheet.cssRules[0];
  const supportsRules = supports.cssRules;

  supports.cssText = '@supports (display: flex) { @container card (min-width: 10px) { .new { margin: 0; } } }';
  const container = supports.cssRules[0];
  const child = container.cssRules[0];
  return [
    supports.type,
    CSSRule.SUPPORTS_RULE,
    container.type,
    CSSRule.CONTAINER_RULE,
    child.type,
    CSSRule.STYLE_RULE,
    supports.cssRules === supportsRules,
    sheet.cssRules[0].type,
    sheet.cssRules[0].cssText === supports.cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSRule.type should use the attached native rule after cssText reset");

    assert_eq!(result, "12|12|17|17|1|1|true|12|true");
}

#[test]
fn css_supported_at_rule_public_getters_use_attached_native_rules() {
    let mut vm = new_storage_test_vm("https://css-at-rule-public-getters-live-view.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const style = document.createElement('style');
  style.textContent = `
    @import url("support/a.css") layer(old) supports((display: flex)) screen;
    @namespace svg url("http://www.w3.org/2000/svg");
    @layer alpha { .layered { color: red; } }
    @media screen { .media { color: red; } }
    @supports (display: grid) { .supports { display: grid; } }
    @page :first { margin-top: 1px; @top-left { content: "x"; } }
    @property --accent { syntax: "<color>"; inherits: false; initial-value: red; }
    @counter-style thumbs { system: cyclic; symbols: "*"; suffix: " "; }
    @font-feature-values old_family { @annotation { mark: 1; } }
    @keyframes fade { from { opacity: 0; } to { opacity: 1; } }
  `;
  head.appendChild(style);
  const sheet = style.sheet;
  const [
    importRule,
    namespaceRule,
    layerRule,
    mediaRule,
    supportsRule,
    pageRule,
    propertyRule,
    counterRule,
    fontFeatureRule,
    keyframesRule
  ] = Array.from(sheet.cssRules);

  const importMedia = importRule.media;
  const mediaList = mediaRule.media;
  const pageRules = pageRule.cssRules;
  const fontAnnotation = fontFeatureRule.annotation;
  const keyframeRules = keyframesRule.cssRules;

  importRule.media.mediaText = 'print and (WiDtH)';
  mediaRule.media.mediaText = 'speech';
  supportsRule.cssText = '@supports (display: flex) { .supports { display: flex; } }';
  pageRule.selectorText = ':left';
  propertyRule.cssText = '@property --tone { syntax: "*"; inherits: true; }';
  counterRule.cssText = '@counter-style dots { system: cyclic; symbols: "."; suffix: " "; }';
  fontFeatureRule.fontFamily = 'new_family';
  keyframesRule.name = 'slide';
  const index = sheet.insertRule('.temp { color: green; }', sheet.cssRules.length);
  sheet.deleteRule(index);

  return [
    sheet.cssRules.length === 10,
    sheet.cssRules[0] === importRule,
    importRule.media === importMedia,
    importRule.href.includes('support/a.css'),
    importRule.media.mediaText === 'print and (width)',
    importRule.layerName === 'old',
    importRule.supportsText === '(display: flex)',
    namespaceRule.prefix === 'svg',
    namespaceRule.namespaceURI === 'http://www.w3.org/2000/svg',
    layerRule.name === 'alpha',
    layerRule.cssRules[0].selectorText === '.layered',
    mediaRule.media === mediaList,
    mediaRule.media.mediaText === 'speech',
    supportsRule.conditionText === '(display: flex)',
    pageRule.selectorText === ':left',
    pageRule.cssRules === pageRules,
    pageRule.cssRules[0].name === 'top-left',
    pageRule.style.marginTop === '1px',
    propertyRule.name === '--tone',
    propertyRule.syntax === '*',
    propertyRule.inherits === true,
    propertyRule.initialValue === null,
    counterRule.name === 'dots',
    fontFeatureRule.fontFamily === 'new_family',
    fontFeatureRule.annotation === fontAnnotation,
    fontFeatureRule.annotation.get('mark').join(',') === '1',
    keyframesRule.name === 'slide',
    keyframesRule.cssRules === keyframeRules,
    keyframesRule.findRule('from').style.opacity === '0',
    sheet.cssRules[9].cssText === keyframesRule.cssText
  ].join('|');
})()
"#,
        )
        .expect("supported at-rule public getters should use attached native rules");

    assert_eq!(
        result,
        "true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true"
    );
}

#[test]
fn css_supported_at_rule_materialization_uses_shallow_native_wrappers() {
    let mut vm = new_storage_test_vm("https://css-at-rule-materialization-stylo-view.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    @layer alpha, beta;
    @layer gamma { .x { margin: 0; } }
    @container card (min-width: 10px) { .y { color: red; } }
    @scope (.host) to (.end) { .z { display: block; } }
  `);
  const [statement, layer, container, scope] = Array.from(sheet.cssRules);
  const before = [
    sheet.cssRules.length,
    statement.constructor === CSSLayerStatementRule,
    statement instanceof CSSLayerStatementRule,
    statement.type,
    statement.nameList.join(','),
    Object.isFrozen(statement.nameList),
    layer.constructor === CSSLayerBlockRule,
    layer instanceof CSSLayerBlockRule,
    layer instanceof CSSGroupingRule,
    layer.type,
    layer.name,
    layer.cssRules[0].cssText,
    container.constructor === CSSContainerRule,
    container.type,
    container.containerName,
    container.conditionText,
    scope.constructor === CSSScopeRule,
    scope.start,
    scope.end
  ].join('|');

  sheet.insertRule('@layer delta { .d { padding: 1px; } }', sheet.cssRules.length);
  const inserted = sheet.cssRules[sheet.cssRules.length - 1];
  scope.cssText = '@scope (.fresh) { .fresh-rule { opacity: 1; } }';
  const after = [
    inserted.constructor === CSSLayerBlockRule,
    inserted.name,
    inserted.cssRules[0].cssText,
    scope.constructor === CSSScopeRule,
    scope.cssRules[0].cssText,
    sheet.cssRules[3].cssText === scope.cssText
  ].join('|');
  return `${before}||${after}`;
})()
"#,
        )
        .expect("supported at-rule materialization should use shallow native wrappers");

    assert_eq!(
        result,
        "4|true|true|0|alpha,beta|true|true|true|true|0|gamma|.x { margin: 0px; }|true|17|card|card (min-width: 10px)|true|.host|.end||true|delta|.d { padding: 1px; }|true|.fresh-rule { opacity: 1; }|true"
    );
}

#[test]
fn css_grouping_insert_rule_materializes_supported_at_rules_from_native_seeds() {
    let mut vm = new_storage_test_vm("https://css-grouping-insert-supported-at-rules.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@media screen {}');
  const media = sheet.cssRules[0];
  const supportsIndex = media.insertRule('@supports (display: grid) { .s { display: grid; } }', 0);
  const containerIndex = media.insertRule('@container card (min-width: 10px) { .c { margin: 0; } }', 1);
  const layerIndex = media.insertRule('@layer inner { .l { padding: 1px; } }', 2);
  const scopeIndex = media.insertRule('@scope (.host) { .sc { opacity: 1; } }', 3);
  const [supports, container, layer, scope] = Array.from(media.cssRules);

  return [
    supportsIndex,
    containerIndex,
    layerIndex,
    scopeIndex,
    supports.constructor === CSSSupportsRule,
    supports.cssRules[0].cssText,
    container.constructor === CSSContainerRule,
    container.containerName,
    container.cssRules[0].cssText,
    layer.constructor === CSSLayerBlockRule,
    layer.name,
    layer.cssRules[0].cssText,
    scope.constructor === CSSScopeRule,
    scope.start,
    scope.cssRules[0].cssText,
    sheet.cssRules[0].cssText === media.cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSGroupingRule.insertRule should materialize supported at-rules from native seeds");

    assert_eq!(
        result,
        "0|1|2|3|true|.s { display: grid; }|true|card|.c { margin: 0px; }|true|inner|.l { padding: 1px; }|true|.host|.sc { opacity: 1; }|true"
    );
}

#[test]
fn css_grouping_child_css_text_reset_syncs_parent_rule_from_stylo() {
    let mut vm = new_storage_test_vm("https://css-grouping-child-css-text-parent-sync.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@media screen { @supports (display: grid) { .old { color: red; } } }');
  const media = sheet.cssRules[0];
  const supports = media.cssRules[0];
  supports.cssText = '@supports (display: flex) { .new { margin: 0; } }';
  return [
    supports.cssText,
    supports.cssRules[0].cssText,
    media.cssText,
    sheet.cssRules[0].cssText,
  ].join('|');
})()
"#,
        )
        .expect("nested CSSGroupingRule cssText reset should sync parent rule");

    assert_eq!(
        result,
        "@supports (display: flex) {\n  .new { margin: 0px; }\n}|.new { margin: 0px; }|@media screen {\n  @supports (display: flex) {\n  .new { margin: 0px; }\n}\n}|@media screen {\n  @supports (display: flex) {\n  .new { margin: 0px; }\n}\n}"
    );
}

#[test]
fn css_style_rule_insert_rule_uses_stylo_nested_mutation_context() {
    let mut vm = new_storage_test_vm("https://css-style-rule-nested-mutation-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@namespace svg url("http://www.w3.org/2000/svg"); .host { & .one { color: red; } }');
  const rule = sheet.cssRules[1];
  const index = rule.insertRule('> svg|path { color: blue; }', 1);
  rule.insertRule('margin: 0; padding: 1px;', 0);
  const declaration = rule.cssRules[0];
  const relative = rule.cssRules[2];
  rule.deleteRule(1);
  return [
    index,
    rule.cssRules.length,
    declaration instanceof CSSNestedDeclarations,
    declaration.cssText,
    relative.selectorText,
    relative.cssText,
    rule.cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleRule nested Stylo mutation context should evaluate");

    assert_eq!(
        result,
        "1|2|true|margin: 0px; padding: 1px;|& > svg|path|& > svg|path { color: blue; }|.host {\n  margin: 0px; padding: 1px;\n  & > svg|path { color: blue; }\n}"
    );
}

#[test]
fn css_style_rule_css_text_reset_refreshes_existing_nested_rule_list_from_stylo() {
    let mut vm = new_storage_test_vm("https://css-style-rule-nested-css-text-reset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@namespace svg url("http://www.w3.org/2000/svg"); .host { & .old { color: red; } }');
  const rule = sheet.cssRules[1];
  const rules = rule.cssRules;
  const old = rules[0];
  rule.cssText = '.host { color: red; & > svg|path { color: blue; } width: 1px; }';
  return [
    rule.cssRules === rules,
    rules.length,
    old.parentRule === null,
    rules[0].selectorText,
    rules[0].cssText,
    rules[1] instanceof CSSNestedDeclarations,
    rules[1].cssText,
    rule.style.getPropertyValue('color'),
    rule.cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleRule cssText reset should refresh existing nested cssRules");

    assert_eq!(
        result,
        "true|2|true|& > svg|path|& > svg|path { color: blue; }|true|width: 1px;|red|.host {\n  color: red;\n  & > svg|path { color: blue; }\n  width: 1px;\n}"
    );
}

#[test]
fn css_keyframes_rule_append_delete_materializes_stylo_mutation_children() {
    let mut vm = new_storage_test_vm("https://css-keyframes-rule-mutation-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes slide { from { opacity: 0; } }');
  const keyframes = sheet.cssRules[0];
  const first = keyframes.cssRules[0];
  keyframes.appendRule('to { opacity: 1; transform: translateX(10px); }');
  const deleted = keyframes.cssRules[1];
  keyframes.appendRule('50% { opacity: 0.5; }');
  const middle = keyframes.cssRules[2];
  keyframes.deleteRule('to');
  return [
    keyframes.cssRules.length,
    keyframes.length,
    keyframes[0] === first,
    keyframes[1] === middle,
    deleted.parentRule === null,
    keyframes.findRule('from') === first,
    keyframes.findRule('to') === null,
    middle.keyText,
    middle.cssText,
    keyframes.cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSKeyframesRule Stylo mutation view should evaluate");

    assert_eq!(
        result,
        "2|2|true|true|true|true|true|50%|50% { opacity: 0.5; }|@keyframes slide {\n0% { opacity: 0; }\n50% { opacity: 0.5; }\n}"
    );
}

#[test]
fn css_keyframes_rule_live_stylesheet_mutation_preserves_stylesheet_mutation_path() {
    let mut vm = new_storage_test_vm("https://css-keyframes-rule-live-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes slide { from { opacity: 0; } }');
  const keyframes = sheet.cssRules[0];
  const rules = keyframes.cssRules;

  keyframes.appendRule('to { opacity: 1; transform: translateX(10px); }');
  const inserted = sheet.insertRule('.after { margin: 0; }', 1);
  keyframes.deleteRule('to');
  const deleted = sheet.deleteRule(1);

  return [
    inserted,
    deleted === undefined,
    sheet.cssRules.length,
    sheet.cssRules[0] === keyframes,
    keyframes.cssRules === rules,
    keyframes.length,
    keyframes.cssRules[0].cssText,
    keyframes.cssText,
  ].join('|');
})()
"#,
        )
        .expect("live keyframes mutation should preserve stylesheet mutation path");

    assert_eq!(
        result,
        "1|true|1|true|true|1|0% { opacity: 0; }|@keyframes slide {\n0% { opacity: 0; }\n}"
    );
}

#[test]
fn css_keyframes_rule_name_mutation_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-keyframes-name-live-mutation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes fade { from { opacity: 0; } to { opacity: 1; } } .after { color: black; }');
  const keyframes = sheet.cssRules[0];
  const keyframeRules = keyframes.cssRules;
  const first = keyframeRules[0];
  const firstStyle = first.style;

  keyframes.name = 'slide';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);
  keyframes.appendRule('50% { opacity: .5; }');
  keyframes.deleteRule('50%');

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === keyframes,
    keyframes.cssRules === keyframeRules,
    keyframeRules[0] === first,
    first.style === firstStyle,
    keyframes.name,
    first.keyText,
    first.style.opacity,
    keyframes.findRule('from') === first,
    keyframes.cssText.includes('@keyframes slide'),
    sheet.cssRules[0].cssText === keyframes.cssText,
    sheet.cssRules[1].cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSKeyframesRule name mutation should preserve Stylo rule tree");

    assert_eq!(
        result,
        "2|true|true|true|true|slide|0%|0|true|true|true|.after { color: black; }"
    );
}

#[test]
fn css_keyframes_rule_css_text_reset_uses_attached_native_rule() {
    let mut vm = new_storage_test_vm("https://css-keyframes-css-text-stylo-view.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes fade { from { opacity: 0; } to { opacity: 1; } } .after { color: black; }');
  const keyframes = sheet.cssRules[0];
  const keyframeRules = keyframes.cssRules;

  keyframes.cssText = '@keyframes "slide show" { from { opacity: .25; } to { transform: translateX(1px); } }';
  const beforeInvalidReset = keyframes.cssText;
  keyframes.cssText = '@keyframes none { from { opacity: 0; } }';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === keyframes,
    keyframes.cssRules === keyframeRules,
    keyframes.length,
    keyframes.name,
    keyframes.cssText === beforeInvalidReset,
    keyframes.cssText.includes('@keyframes slide\\ show'),
    keyframes.findRule('from').style.opacity,
    keyframes.findRule('to').style.transform,
    sheet.cssRules[0].cssText === keyframes.cssText,
    sheet.cssRules[1].cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSKeyframesRule cssText reset should use the attached native rule");

    assert_eq!(
        result,
        "2|true|true|2|slide show|true|true|0.25|translateX(1px)|true|.after { color: black; }"
    );
}

#[test]
fn css_keyframes_rule_lazy_css_rules_use_attached_native_rules() {
    let mut vm = new_storage_test_vm("https://css-keyframes-lazy-css-rules-stylo-view.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes fade { from { opacity: 0; } } .after { color: black; }');
  const keyframes = sheet.cssRules[0];

  keyframes.cssText = '@keyframes slide { from { opacity: .25; } to { transform: translateX(1px); } }';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  const rules = keyframes.cssRules;
  return [
    keyframes.name,
    keyframes.length,
    rules.length,
    rules[0].cssText,
    rules[1].style.transform,
    sheet.cssRules[0].cssText === keyframes.cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSKeyframesRule lazy cssRules should use attached native rules");

    assert_eq!(
        result,
        "slide|2|2|0% { opacity: 0.25; }|translateX(1px)|true"
    );
}

#[test]
fn css_keyframe_key_text_uses_stylo_selector_helpers() {
    let mut vm = new_storage_test_vm("https://css-keyframe-key-text-stylo.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes slide { from { opacity: 0; } }');
  const keyframes = sheet.cssRules[0];
  const frame = keyframes.cssRules[0];

  frame.keyText = '50%, to';
  const afterValid = [
    frame.keyText,
    frame.cssText,
    keyframes.findRule('50%, 100%') === frame,
    keyframes.findRule('50%, to') === frame,
    keyframes.findRule('100%') === null,
    keyframes.cssText,
  ].join('/');

  frame.keyText = 'body';
  const afterInvalid = [
    frame.keyText,
    keyframes.findRule('body') === null,
    keyframes.findRule('50%, 100%') === frame,
  ].join('/');

  keyframes.deleteRule('50%, to');
  return [afterValid, afterInvalid, keyframes.cssRules.length].join('|');
})()
"#,
        )
        .expect("CSSKeyframeRule keyText Stylo selector helpers should evaluate");

    assert_eq!(
        result,
        "50%, 100%/50%, 100% { opacity: 0; }/true/true/true/@keyframes slide {\n50%, 100% { opacity: 0; }\n}|50%, 100%/true/true|0"
    );
}

#[test]
fn css_keyframe_rule_key_text_live_stylesheet_mutation_preserves_stylesheet_path() {
    let mut vm = new_storage_test_vm("https://css-keyframe-key-text-live-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes fade { from { opacity: 0; } to { opacity: 1; } }');
  const keyframes = sheet.cssRules[0];
  const keyframeRules = keyframes.cssRules;
  const keyframe = keyframeRules[1];

  keyframe.keyText = '75%, to';
  keyframes.appendRule('50% { opacity: .5; }');
  keyframes.deleteRule('50%');

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === keyframes,
    keyframes.cssRules === keyframeRules,
    keyframes.cssRules[1] === keyframe,
    keyframe.keyText,
    keyframe.cssText,
    keyframes.findRule('75%, 100%') === keyframe,
    keyframes.findRule('100%') === null,
    keyframes.cssRules.length,
    keyframes.cssText.includes('75%, 100%'),
    sheet.cssRules[0].cssText === keyframes.cssText,
  ].join('|');
})()
"#,
        )
        .expect("live CSSKeyframeRule keyText mutation path should evaluate");

    assert_eq!(
        result,
        "1|true|true|true|75%, 100%|75%, 100% { opacity: 1; }|true|true|2|true|true"
    );
}

#[test]
fn regular_css_style_sheet_replace_is_not_allowed() {
    let mut vm = new_storage_test_vm("https://css-regular-sheet-replace.test/");

    let sync_result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value === undefined ? 'undefined' : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const style = document.createElement('style');
  style.textContent = 'html { background-color: green; }';
  root.append(style);
  const regular = style.sheet;
  const imported = document.createElement('style');
  imported.textContent = '@import url("data:text/css,span%7Bcolor%3Ablue%7D");';
  root.append(imported);
  const childRule = imported.sheet.cssRules[0];
  const child = childRule.styleSheet;
  const before = regular.cssRules[0].cssText;
  const regularSync = probe(() => regular.replaceSync('main { color: red; }'));
  const childSync = probe(() => child.replaceSync('span { color: red; }'));
  globalThis.__regularSheetReplaceProbe = [];
  regular.replace('main { color: red; }').then(
    () => globalThis.__regularSheetReplaceProbe.push('regular:resolved'),
    error => globalThis.__regularSheetReplaceProbe.push(`regular:${error && error.name}:${regular.cssRules[0].cssText}`)
  );
  child.replace('span { color: red; }').then(
    () => globalThis.__regularSheetReplaceProbe.push('child:resolved'),
    error => globalThis.__regularSheetReplaceProbe.push(`child:${error && error.name}:${child.cssRules.length}`)
  );
  style.remove();
  const removedBackground = getComputedStyle(root).backgroundColor;
  regular.replace('html { background-color: red; }').then(
    () => globalThis.__regularSheetReplaceProbe.push('removed:resolved'),
    error => globalThis.__regularSheetReplaceProbe.push(`removed:${error && error.name}:${getComputedStyle(root).backgroundColor}`)
  );
  return [
    regular instanceof CSSStyleSheet,
    before,
    regularSync,
    regular.cssRules[0].cssText,
    child instanceof CSSStyleSheet,
    child.ownerRule === childRule,
    childSync,
    removedBackground
  ].join('|');
})()
"#,
        )
        .expect("regular CSSStyleSheet replaceSync should evaluate");

    let async_result = vm
        .eval("globalThis.__regularSheetReplaceProbe.join('|')")
        .expect("regular CSSStyleSheet replace promises should settle");

    assert_eq!(
        sync_result,
        "true|html { background-color: green; }|throw:NotAllowedError|html { background-color: green; }|true|true|throw:NotAllowedError|rgba(0, 0, 0, 0)"
    );
    assert_eq!(
        async_result,
        "regular:NotAllowedError:html { background-color: green; }|child:NotAllowedError:1|removed:NotAllowedError:rgba(0, 0, 0, 0)"
    );
}
#[test]
fn disabled_constructed_stylesheet_is_ignored_by_adopted_stylesheets() {
    let mut vm = new_storage_test_vm("https://css-disabled-constructed-sheet.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const host = document.createElement('div');
  root.append(host);
  const shadow = host.attachShadow({ mode: 'open' });
  const sheet = new CSSStyleSheet({ disabled: true });
  sheet.replaceSync('div { color: red; }');
  shadow.adoptedStyleSheets = [sheet];
  shadow.innerHTML = '<style>div { color: green; }</style><div>target</div>';
  const target = shadow.querySelector('div');
  const disabled = [
    sheet.disabled,
    getComputedStyle(target).color
  ].join(',');
  sheet.disabled = false;
  const enabled = [
    sheet.disabled,
    getComputedStyle(target).color
  ].join(',');
  sheet.disabled = true;
  const disabledAgain = [
    sheet.disabled,
    getComputedStyle(target).color
  ].join(',');
  return [disabled, enabled, disabledAgain].join('|');
})()
"#,
        )
        .expect("disabled constructed adopted stylesheet should evaluate");

    assert_eq!(
        result,
        "true,rgb(0, 128, 0)|false,rgb(255, 0, 0)|true,rgb(0, 128, 0)"
    );
}

#[test]
fn owner_style_data_import_feeds_computed_style_without_hiding_cssom_import_rule() {
    let mut vm = new_storage_test_vm("https://owner-style-data-import.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const fallback = document.createElement('style');
  fallback.textContent = '@layer { .target { color: red; } }';
  head.appendChild(fallback);

  const target = document.createElement('div');
  target.className = 'target';
  body.appendChild(target);

  const style = document.createElement('style');
  style.textContent = '@import url("data:text/css,.target%7Bcolor:green%7D") supports(display:block);';
  head.appendChild(style);

  const rule = style.sheet.cssRules[0];
  const child = rule.styleSheet;
  return [
    getComputedStyle(target).color,
    style.sheet.cssRules.length,
    rule.constructor.name,
    rule.href,
    rule.supportsText,
    rule.cssText,
    child instanceof CSSStyleSheet,
    child.cssRules.length,
    child.cssRules[0].cssText,
    child.ownerRule === rule,
    child.parentStyleSheet === style.sheet,
    rule.styleSheet === child
  ].join('|');
})()
"#,
        )
        .expect("owner style data import computed-style probe should evaluate");

    assert_eq!(
        result,
        "rgb(0, 128, 0)|1|CSSImportRule|data:text/css,.target%7Bcolor:green%7D|display:block|@import url(\"data:text/css,.target%7Bcolor:green%7D\") supports(display:block);|true|1|.target { color: green; }|true|true|true"
    );
}

#[test]
fn pending_css_import_rule_has_no_child_stylesheet() {
    let mut vm = new_storage_test_vm("https://pending-import-sheet.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const style = document.createElement('style');
  style.textContent = '@import url("https://pending-import-sheet.test/slow.css");';
  root.append(style);
  const rule = style.sheet.cssRules[0];
  return [rule.styleSheet === null, rule.styleSheet === null].join('|');
})()
"#,
        )
        .expect("pending CSSImportRule styleSheet probe should evaluate");

    assert_eq!(result, "true|true");
}

#[test]
fn unadopted_constructed_stylesheet_mutations_do_not_resync_document_stylesheets() {
    let mut vm = new_storage_test_vm("https://unadopted-constructed-sheet-sync.test/");
    let document = vm.document_handle_for_test();

    let generation_before = vm.computed_style_cache_generation_for_document_for_test(document);
    let result = vm
        .eval(
            r#"
(() => {
  const untouched = document.adoptedStyleSheets.length;
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('body { color: rgb(1, 2, 3); }');
  sheet.insertRule('main { color: rgb(4, 5, 6); }', sheet.cssRules.length);
  sheet.disabled = true;
  sheet.disabled = false;
  return `${untouched}|${document.adoptedStyleSheets.length}|${sheet.cssRules.length}`;
})()
"#,
        )
        .expect("unadopted constructed stylesheet mutations should evaluate");

    assert_eq!(result, "0|0|2");
    assert_eq!(
        vm.computed_style_cache_generation_for_document_for_test(document),
        generation_before
    );
}
#[test]
fn adopted_constructed_stylesheet_noop_syncs_preserve_style_generation() {
    let mut vm = new_storage_test_vm("https://adopted-constructed-sheet-noop-sync.test/");
    let document = vm.document_handle_for_test();

    let initial = vm
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
  globalThis.__noopSyncSheet = sheet;
  globalThis.__noopSyncStyle = getComputedStyle(target);
  return globalThis.__noopSyncStyle.color;
})()
"#,
        )
        .expect("adopted constructed stylesheet setup should evaluate");
    assert_eq!(initial, "rgb(1, 2, 3)");
    let generation_after_setup = vm.computed_style_cache_generation_for_document_for_test(document);

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__noopSyncSheet.disabled = false;
  const first = globalThis.__noopSyncStyle.color;
  globalThis.__noopSyncSheet.disabled = false;
  const second = globalThis.__noopSyncStyle.color;
  return `${first}|${second}|${globalThis.__noopSyncSheet.disabled}`;
})()
"#,
        )
        .expect("adopted constructed stylesheet no-op sync should evaluate");

    assert_eq!(result, "rgb(1, 2, 3)|rgb(1, 2, 3)|false");
    assert_eq!(
        vm.computed_style_cache_generation_for_document_for_test(document),
        generation_after_setup
    );
}
#[test]
fn cssom_rule_mutation_refreshes_computed_style_without_generation_bump() {
    let mut vm = new_storage_test_vm("https://cssom-rule-source-revision.test/");
    let document = vm.document_handle_for_test();

    let initial = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const target = body.appendChild(document.createElement('div'));
  target.id = 'target';
  globalThis.__sourceRevisionSheet = new CSSStyleSheet();
  globalThis.__sourceRevisionSheet.replaceSync('#target { color: rgb(1, 2, 3); }');
  document.adoptedStyleSheets = [globalThis.__sourceRevisionSheet];
  globalThis.__sourceRevisionStyle = getComputedStyle(target);
  return globalThis.__sourceRevisionStyle.color;
})()
"#,
        )
        .expect("CSSOM source revision setup should evaluate");
    assert_eq!(initial, "rgb(1, 2, 3)");
    let generation_after_setup = vm.computed_style_cache_generation_for_document_for_test(document);

    let same = vm
        .eval(
            r#"
(() => {
  const rule = globalThis.__sourceRevisionSheet.cssRules[0];
  rule.style.setProperty('color', 'rgb(1, 2, 3)');
  return globalThis.__sourceRevisionStyle.color;
})()
"#,
        )
        .expect("CSSOM no-op rule mutation should evaluate");
    assert_eq!(same, "rgb(1, 2, 3)");
    assert_eq!(
        vm.computed_style_cache_generation_for_document_for_test(document),
        generation_after_setup
    );

    let changed = vm
        .eval(
            r#"
(() => {
  const rule = globalThis.__sourceRevisionSheet.cssRules[0];
  rule.style.setProperty('color', 'rgb(4, 5, 6)');
  return globalThis.__sourceRevisionStyle.color;
})()
"#,
        )
        .expect("CSSOM changed rule mutation should evaluate");
    assert_eq!(changed, "rgb(4, 5, 6)");
    assert_eq!(
        vm.computed_style_cache_generation_for_document_for_test(document),
        generation_after_setup
    );
}
#[test]
fn constructed_stylesheet_mutation_resyncs_only_its_adopted_owners() {
    let mut vm = new_storage_test_vm("https://adopted-constructed-sheet-owner-sync.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const documentTarget = body.appendChild(document.createElement('div'));
  documentTarget.id = 'document-target';
  const host = body.appendChild(document.createElement('section'));
  const shadow = host.attachShadow({ mode: 'open' });
  const shadowTarget = shadow.appendChild(document.createElement('span'));

  const documentSheet = new CSSStyleSheet();
  documentSheet.replaceSync('#document-target { color: rgb(1, 2, 3); }');
  document.adoptedStyleSheets = [documentSheet];

  const shadowSheet = new CSSStyleSheet();
  shadowSheet.replaceSync('span { color: rgb(4, 5, 6); }');
  shadow.adoptedStyleSheets = [shadowSheet];

  const shadowStyle = getComputedStyle(shadowTarget);
  const beforeShadow = shadowStyle.color;

  // This is Moli's wrapper cache slot. The stylesheet change below is
  // unrelated to the shadow owner, so it must not resync the shadow adopted
  // sources through this stale wrapper value.
  shadow.__moliAdoptedStyleSheets = [];
  documentSheet.replaceSync('#document-target { color: rgb(7, 8, 9); }');

  return [
    beforeShadow,
    shadowStyle.color,
    getComputedStyle(documentTarget).color
  ].join('|');
})()
"#,
        )
        .expect("owner-scoped constructed stylesheet sync should evaluate");

    assert_eq!(result, "rgb(4, 5, 6)|rgb(4, 5, 6)|rgb(7, 8, 9)");
}
#[test]
fn replaced_constructed_stylesheet_loses_adopted_owner_tracking() {
    let mut vm = new_storage_test_vm("https://adopted-constructed-sheet-owner-detach.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const target = body.appendChild(document.createElement('div'));
  target.id = 'target';

  const oldSheet = new CSSStyleSheet();
  oldSheet.replaceSync('#target { color: rgb(1, 2, 3); }');
  document.adoptedStyleSheets = [oldSheet];
  const oldArray = document.adoptedStyleSheets;
  const before = getComputedStyle(target).color;

  document.adoptedStyleSheets = [];
  const afterReplace = getComputedStyle(target).color;
  oldArray.push(oldSheet);
  const afterOldArrayMutation = getComputedStyle(target).color;

  document.__moliAdoptedStyleSheets = [oldSheet];
  oldSheet.replaceSync('#target { color: rgb(7, 8, 9); }');

  return [
    before,
    afterReplace,
    afterOldArrayMutation,
    getComputedStyle(target).color
  ].join('|');
})()
"#,
        )
        .expect("replaced constructed stylesheet owner tracking should evaluate");

    assert_eq!(
        result,
        "rgb(1, 2, 3)|rgb(0, 0, 0)|rgb(0, 0, 0)|rgb(0, 0, 0)"
    );
}
#[test]
fn constructed_stylesheet_mutation_uses_tracked_adopted_array_not_public_wrapper_slot() {
    let mut vm = new_storage_test_vm("https://adopted-constructed-sheet-private-array.test/");

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
  const before = style.color;

  document.__moliAdoptedStyleSheets = [];
  sheet.replaceSync('#target { color: rgb(7, 8, 9); }');

  return [before, style.color].join('|');
})()
"#,
        )
        .expect("constructed stylesheet sync should use tracked adopted array");

    assert_eq!(result, "rgb(1, 2, 3)|rgb(7, 8, 9)");
}
#[test]
fn adopted_stylesheets_direct_assignment_rejects_non_constructed_sheets() {
    let mut vm = new_storage_test_vm("https://css-adopted-assignment-validation.test/");

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

  const constructed = new CSSStyleSheet();
  constructed.replaceSync('#target { background-color: lime !important; }');
  document.adoptedStyleSheets = [constructed];
  const documentResult = probe(() => {
    document.adoptedStyleSheets = [style.sheet];
  });
  const frame = document.createElement('iframe');
  root.append(frame);
  const frameResult = probe(() => {
    frame.contentDocument.adoptedStyleSheets = [style.sheet];
  });

  const host = document.createElement('section');
  root.append(host);
  const shadow = host.attachShadow({ mode: 'open' });
  shadow.innerHTML = '<style>span { color: red; }</style><span>target</span>';
  const shadowConstructed = new CSSStyleSheet();
  shadowConstructed.replaceSync('span { color: green !important; }');
  shadow.adoptedStyleSheets = [shadowConstructed];
  const shadowResult = probe(() => {
    shadow.adoptedStyleSheets = [shadow.querySelector('style').sheet];
  });

  return [
    documentResult,
    document.adoptedStyleSheets.length,
    getComputedStyle(target).backgroundColor,
    frameResult,
    frame.contentDocument.adoptedStyleSheets.length,
    shadowResult,
    shadow.adoptedStyleSheets.length,
    getComputedStyle(shadow.querySelector('span')).color
  ].join('|');
})()
"#,
        )
        .expect("adoptedStyleSheets assignment validation should evaluate");

    assert_eq!(
        result,
        "NotAllowedError|1|rgb(0, 255, 0)|NotAllowedError|0|NotAllowedError|1|rgb(0, 128, 0)"
    );
}
#[test]
fn css_rule_style_properties_forward_assignment_to_css_text() {
    let mut vm = new_storage_test_vm("https://css-rule-style-put-forwards.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    #target { color: red; margin: 1px; }
    @keyframes slide {
      from { margin-left: 100%; width: 300%; }
      to { margin-left: 0%; width: 100%; }
    }
  `);
  const styleRule = sheet.cssRules[0];
  const keyframeRule = sheet.cssRules[1].cssRules[0];
  styleRule.style = 'color: blue; padding: 2px;';
  keyframeRule.style = 'margin-left: 50%; width: 100%;';
  return [
    styleRule.style.cssText,
    styleRule.cssText,
    keyframeRule.style.marginLeft,
    keyframeRule.style.width,
    keyframeRule.cssText
  ].join('|');
})()
"#,
        )
        .expect("CSS rule style [PutForwards] assignment should evaluate");

    assert_eq!(
        result,
        "color: blue; padding: 2px;|#target { color: blue; padding: 2px; }|50%|100%|0% { margin-left: 50%; width: 100%; }"
    );
}

#[test]
fn css_keyframe_rule_style_uses_stylo_declaration_block_for_plain_properties() {
    let mut vm = new_storage_test_vm("https://keyframe-style-pdb.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes slide { from { } }');
  const rule = sheet.cssRules[0].cssRules[0];
  const style = rule.style;

  style.setProperty('place-content', 'center start', 'important');
  const pdbWrite = [
    style.length,
    Array.from({ length: style.length }, (_, index) => style.item(index)).join(','),
    style.getPropertyValue('place-content'),
    style.getPropertyPriority('place-content'),
    style.cssText,
    rule.cssText
  ].join(',');

  style.cssText = [
    'display: invalid;',
    'display: block;',
    'place-content: center start;',
    'animation-name: spin;'
  ].join(' ');
  const cssTextWrite = [
    style.getPropertyValue('display'),
    style.getPropertyValue('place-content'),
    style.getPropertyValue('animation-name'),
    style.cssText,
    rule.cssText.includes('animation-name')
  ].join(',');

  style.setProperty('animation-timing-function', 'steps(2)');
  const timing = style.getPropertyValue('animation-timing-function');

  style.removeProperty('place-content');
  const removedPlaceContent = [
    timing,
    style.getPropertyValue('place-content'),
    style.getPropertyValue('align-content'),
    style.cssText
  ].join(',');

  return [pdbWrite, cssTextWrite, removedPlaceContent].join('|');
})()
"#,
        )
        .expect("CSSKeyframeRule style should use Stylo declarations for plain properties");

    assert_eq!(
        result,
        "2,align-content,justify-content,center start,important,place-content: center start !important;,0% { place-content: center start !important; }|block,center start,,display: block; place-content: center start;,false|steps(2),,,display: block; animation-timing-function: steps(2);"
    );
}

#[test]
fn css_stylesheet_exposes_escaped_namespace_attribute_selector_rules() {
    let mut vm = new_storage_test_vm("https://css-escaped-namespace-selector.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = '@namespace ns\\:odd url(ns);[ns\\:odd|odd\\:name] { color: red; }';
  (document.head || document.documentElement || document).appendChild(style);
  const rules = style.sheet.cssRules;
  return [
    rules.length,
    rules[0]?.cssText,
    rules[1]?.selectorText,
    rules[1]?.cssText
  ].join('|');
})()
"#,
        )
        .expect("escaped namespace selector rule should evaluate");

    assert_eq!(
        result,
        r#"2|@namespace ns\:odd url("ns");|[ns\:odd|odd\:name]|[ns\:odd|odd\:name] { color: red; }"#
    );
}
#[test]
fn css_nested_declarations_update_parent_rule_style() {
    let mut vm = new_storage_test_vm("https://css-nested-declarations.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = '.parent { color: red; .child { background: blue; } font-size: 20px; }';
  (document.head || document.documentElement || document).appendChild(style);
  const parent = document.createElement('div');
  parent.className = 'parent';
  const child = document.createElement('div');
  child.className = 'child';
  parent.appendChild(child);
  (document.body || document.documentElement || document).appendChild(parent);

  const rule = style.sheet.cssRules[0];
  const nestedRule = rule.cssRules[0];
  const nestedDeclarations = rule.cssRules[1];
  const before = [
    typeof CSSNestedDeclarations,
    nestedDeclarations instanceof CSSNestedDeclarations,
    nestedDeclarations instanceof CSSRule,
    nestedRule.cssText,
    nestedDeclarations.style.getPropertyValue('color'),
    nestedDeclarations.style.getPropertyValue('font-size'),
    rule.style.getPropertyValue('color')
  ].join(',');
  nestedDeclarations.style.color = 'green';
  const after = [
    nestedDeclarations.style.getPropertyValue('color'),
    rule.cssText,
    getComputedStyle(parent).color
  ].join(',');
  return `${before}|${after}`;
})()
"#,
        )
        .expect("CSSNestedDeclarations should update parent rule style");

    assert_eq!(
        result,
        "function,true,true,& .child { background: blue; },,20px,red|green,.parent {\n  color: red;\n  & .child { background: blue; }\n  font-size: 20px; color: green;\n},rgb(0, 128, 0)"
    );
}

#[test]
fn css_nested_declarations_cssom_preserves_rule_order_and_group_blocks() {
    let mut vm = new_storage_test_vm("https://css-nested-declarations-cssom.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    .a {
      --a:1;
      --b:1;
      & { --c:1; }
      --d:1;
      --e:1;
      color:hover {}
      @media (width > 100px) {
        --x:1;
        --y:1;
        .b { }
        --z:1;
      }
      --w:1;
    }
  `);
  const outer = sheet.cssRules[0];
  const media = outer.cssRules[3];
  const iterated = [];
  for (const rule of outer.cssRules) {
    iterated.push(rule.cssText);
  }
  return [
    outer.cssRules.length,
    iterated.join('|'),
    media instanceof CSSMediaRule,
    media.cssRules.length,
    media.cssRules[0] instanceof CSSNestedDeclarations,
    media.cssRules[0].cssText,
    media.cssRules[1].cssText,
    media.cssRules[2] instanceof CSSNestedDeclarations,
    media.cssRules[2].cssText,
  ].join('||');
})()
"#,
        )
        .expect("nested declaration CSSOM order should evaluate");

    assert_eq!(
        result,
        "5||& { --c: 1; }|--d: 1; --e: 1;|& color:hover { }|@media (width > 100px) {\n  --x: 1; --y: 1;\n  & .b { }\n  --z: 1;\n}|--w: 1;||true||3||true||--x: 1; --y: 1;||& .b { }||true||--z: 1;"
    );
}

#[test]
fn css_nested_declarations_parent_rule_uses_pdb_serialization() {
    let mut vm = new_storage_test_vm("https://css-nested-declarations-pdb-serialization.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.a { & .b { } }');
  const rule = sheet.cssRules[0];
  const index = rule.insertRule(
    'color: rgb(0 128 0 / 50%); width: 0; opacity: 1 !important;',
    1
  );
  const nested = rule.cssRules[index];
  return [
    index,
    nested instanceof CSSNestedDeclarations,
    nested.cssText,
    nested.style.cssText,
    rule.cssText
  ].join('|');
})()
"#,
        )
        .expect("nested declaration runs should serialize parent rule from PDB");

    assert_eq!(
        result,
        "1|true|color: rgba(0, 128, 0, 0.5); width: 0px; opacity: 1 !important;|color: rgba(0, 128, 0, 0.5); width: 0px; opacity: 1 !important;|.a {\n  & .b { }\n  color: rgba(0, 128, 0, 0.5); width: 0px; opacity: 1 !important;\n}"
    );
}

#[test]
fn css_nested_grouping_declarations_local_mutation_syncs_live_parent_rule() {
    let mut vm = new_storage_test_vm("https://css-nested-grouping-declarations-sync.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    .a {
      @media (width > 100px) {
        --x: 1;
        .b {}
        --z: 1;
      }
    }
  `);
  const outer = sheet.cssRules[0];
  const media = outer.cssRules[0];
  const leading = media.cssRules[0];
  leading.style.setProperty('--x', '2');
  return [
    leading.cssText,
    media.cssText,
    outer.cssText
  ].join('||');
})()
"#,
        )
        .expect("nested grouping declaration mutation should sync parent rule text");

    assert_eq!(
        result,
        "--x: 2;||@media (width > 100px) {\n  --x: 2;\n  & .b { }\n  --z: 1;\n}||.a {\n  @media (width > 100px) {\n  --x: 2;\n  & .b { }\n  --z: 1;\n}\n}"
    );
}

#[test]
fn css_scope_nested_declarations_are_exposed_in_cssom() {
    let mut vm = new_storage_test_vm("https://css-scope-nested-declarations-cssom.test/");

    let result = vm
        .eval(
            r#"
(() => {
  function run(prelude) {
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(`
      @scope ${prelude} {
        color: red;
        width: 1px;
        .b {}
        left: 2px;
        right: 3px;
        .c {}
        top: 4px;
        bottom: 5px;
      }
    `);
    const scopeRule = sheet.cssRules[0];
    return [
      scopeRule.cssRules.length,
      scopeRule.cssRules[0] instanceof CSSNestedDeclarations,
      scopeRule.cssRules[0].cssText,
      scopeRule.cssRules[2] instanceof CSSNestedDeclarations,
      scopeRule.cssRules[2].cssText,
      scopeRule.cssRules[4] instanceof CSSNestedDeclarations,
      scopeRule.cssRules[4].cssText,
    ].join('|');
  }
  return [run('(.a)'), run('')].join('||');
})()
"#,
        )
        .expect("@scope nested declarations CSSOM should evaluate");

    assert_eq!(
        result,
        "5|true|color: red; width: 1px;|true|left: 2px; right: 3px;|true|top: 4px; bottom: 5px;||5|true|color: red; width: 1px;|true|left: 2px; right: 3px;|true|top: 4px; bottom: 5px;"
    );
}

#[test]
fn css_nested_style_assignment_preserves_existing_nested_rules() {
    let mut vm = new_storage_test_vm("https://css-nested-style-assignment.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.a { color: red; & .b { color: green; } & .c { color: blue; } }');
  const rule = sheet.cssRules[0];
  rule.insertRule('@supports selector(&) { & div { font-size: 10px; }}', 1);
  rule.style = 'color: olivedrab; &.d { color: peru; }';
  return [
    rule.cssRules.length,
    rule.cssRules[0].cssText,
    rule.cssRules[1].cssText,
    rule.cssRules[2].cssText,
    rule.cssText,
  ].join('|');
})()
"#,
        )
        .expect("style assignment should preserve existing nested rules");

    assert_eq!(
        result,
        "3|& .b { color: green; }|@supports selector(&) {\n  & div { font-size: 10px; }\n}|& .c { color: blue; }|.a {\n  color: olivedrab;\n  & .b { color: green; }\n  @supports selector(&) {\n  & div { font-size: 10px; }\n}\n  & .c { color: blue; }\n}"
    );
}

#[test]
fn css_document_stylesheet_insert_rule_preserves_nested_rule_tree() {
    let mut vm = new_storage_test_vm("https://css-document-sheet-nesting.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  (document.head || document.documentElement || document).appendChild(style);
  const sheet = style.sheet;
  sheet.insertRule('div { @media screen { color: red; background-color: green; } }');
  const inserted = sheet.cssRules[0].cssText;

  sheet.deleteRule(0);
  sheet.insertRule('.a { color: red; & .b { color: green; } & .c { color: blue; } }');
  const rule = sheet.cssRules[0];
  rule.style = 'color: olivedrab; &.d { color: peru; }';
  const assigned = rule.cssText;

  return [inserted, assigned].join('|');
})()
"#,
        )
        .expect("document stylesheet nesting mutations should evaluate");

    assert_eq!(
        result,
        "div {\n  @media screen {\n  color: red; background-color: green;\n}\n}|.a {\n  color: olivedrab;\n  & .b { color: green; }\n  & .c { color: blue; }\n}"
    );
}

#[test]
fn css_scope_relative_nested_style_rules_are_exposed_in_cssom() {
    let mut vm = new_storage_test_vm("https://css-scope-relative-nesting.test/");

    let result = vm
        .eval(
            r#"
(() => {
  function createRuleString(prelude, inner) {
    if (prelude.length === 0) {
      return `${inner} {}`;
    }
    const outermost = prelude[0];
    const rest = createRuleString(prelude.slice(1), inner);
    return `${outermost} { ${rest} }`;
  }
  function createByString(style, prelude, inner) {
    style.textContent = createRuleString(prelude, inner);
  }
  function createByInsertion(style, prelude, inner) {
    let current = style.sheet;
    for (const p of prelude) {
      const idx = current.insertRule(`${p} {}`);
      current = current.cssRules[idx];
    }
    current.insertRule(`${inner} {}`);
  }
  function innermostSelector(depth, rules) {
    let current = rules;
    for (let d = depth + 1; d !== 0; d--) {
      if (current.cssRules.length !== 1) {
        return `len:${current.cssRules.length}`;
      }
      current = current.cssRules[0];
    }
    return current.selectorText;
  }
  function run(prelude, method) {
    try {
      const style = document.createElement('style');
      (document.head || document.documentElement || document).appendChild(style);
      method(style, prelude, '> .foo');
      const selector = innermostSelector(prelude.length, style.sheet);
      style.remove();
      return selector;
    } catch (error) {
      return `error:${error && (error.name || error.message)}`;
    }
  }
  const cases = [
    [['@scope', '.nest'], createByString],
    [['.nest', '@scope'], createByString],
    [['@scope', '.nest', '@media screen'], createByString],
    [['.nest', '@scope', '@media screen'], createByString],
    [['@scope', '.nest'], createByInsertion],
    [['.nest', '@scope'], createByInsertion],
    [['@scope', '.nest', '@media screen'], createByInsertion],
    [['.nest', '@scope', '@media screen'], createByInsertion],
  ];
  return cases.map(([prelude, method]) => run(prelude, method)).join('|');
})()
"#,
        )
        .expect("CSS @scope relative nested selector CSSOM should evaluate");

    assert_eq!(
        result,
        "& > .foo|> .foo|& > .foo|> .foo|& > .foo|> .foo|& > .foo|> .foo"
    );
}

#[test]
fn computed_css_style_declaration_is_read_only() {
    let mut vm = new_storage_test_vm("https://computed-style-readonly.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  (document.body || document.documentElement || document).appendChild(target);
  const style = getComputedStyle(target);
  const probe = callback => {
    try {
      callback();
      return 'no-throw';
    } catch (error) {
      return `${error.name}:${error.code}`;
    }
  };
  return [
    style.parentRule === null,
    probe(() => { style.cssText = 'color: blue'; }),
    probe(() => { style.setProperty('color', 'blue'); }),
    probe(() => { style.color = 'blue'; }),
    probe(() => { style.webkitTransition = ''; }),
    probe(() => { style.removeProperty('color'); })
  ].join('|');
})()
"#,
        )
        .expect("computed CSSStyleDeclaration should reject mutations");

    assert_eq!(
        result,
        "true|NoModificationAllowedError:7|NoModificationAllowedError:7|NoModificationAllowedError:7|NoModificationAllowedError:7|NoModificationAllowedError:7"
    );
}
#[test]
fn css_media_rule_exposes_mutable_media_list() {
    let mut vm = new_storage_test_vm("https://css-media-list.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule('@media screen and (min-width: 480px), print, projection { body { color: red; } }');
  const rule = sheet.cssRules[0];
  const media = rule.media;
  const before = [
    media.length,
    media.mediaText,
    media.toString(),
    media[0],
    media[1],
    media[2],
    media[3] === undefined,
    media.item(0),
    media.item(3) === null
  ].join('|');
  media.deleteMedium('print');
  const afterDelete = [media.length, media.mediaText, media[1], media[2] === undefined, media.item(2) === null].join('|');
  media.appendMedium('speech');
  const afterAppend = [media.length, media.mediaText, media[2], media[3] === undefined, media.item(3) === null].join('|');
  media.mediaText = null;
  const afterNull = [media.length, media.mediaText, media.toString()].join('|');
  rule.media = 'speech';
  const afterRuleSetter = [rule.media === media, rule.media.mediaText, rule.conditionText].join('|');
  return [before, afterDelete, afterAppend, afterNull, afterRuleSetter].join('||');
})()
"#,
        )
        .expect("CSSMediaRule MediaList surface should evaluate");

    assert_eq!(
        result,
        "3|screen and (min-width: 480px), print, projection|screen and (min-width: 480px), print, projection|screen and (min-width: 480px)|print|projection|true|screen and (min-width: 480px)|true||2|screen and (min-width: 480px), projection|projection|true|true||3|screen and (min-width: 480px), projection, speech|speech|true|true||0||||true|speech|speech"
    );
}

#[test]
fn css_conditional_rule_idl_surface_matches_wpt_shape() {
    let mut vm = new_storage_test_vm("https://css-conditional-idl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      return String(callback());
    } catch (error) {
      return error && error.name;
    }
  };
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    @media screen { }
    @media print { }
    @supports (display: block) { }
    @supports (does-not-exist: nope) { }
    @supports (color : red) or ( color:blue ) { }
  `);
  const screen = sheet.cssRules[0];
  const print = sheet.cssRules[1];
  const supported = sheet.cssRules[2];
  const unsupported = sheet.cssRules[3];
  const spaced = sheet.cssRules[4];
  return [
    CSS.supports.length,
    screen.matches,
    print.matches,
    supported.matches,
    unsupported.matches,
    spaced.conditionText,
    probe(() => CSSConditionRule.prototype.conditionText),
    probe(() => CSSMediaRule.prototype.media),
    probe(() => CSSMediaRule.prototype.matches),
    probe(() => CSSSupportsRule.prototype.matches),
    typeof Object.getOwnPropertyDescriptor(CSSMediaRule.prototype, 'matches').get,
    typeof Object.getOwnPropertyDescriptor(CSSSupportsRule.prototype, 'matches').get
  ].join('|');
})()
"#,
        )
        .expect("CSS conditional rule IDL surface should evaluate");

    assert_eq!(
        result,
        "1|true|false|true|false|(color : red) or ( color:blue )|TypeError|TypeError|TypeError|TypeError|function|function"
    );
}

#[test]
fn css_media_rules_and_stylesheet_media_use_renderer_viewport_surface() {
    let mut vm = new_storage_test_vm("https://css-media-rule-viewport-surface.test/");
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

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = `
    #target { color: rgb(10, 20, 30); }
    @media (width: 800px) and (device-width: 1920px) {
      #target { color: rgb(1, 2, 3); }
    }
    @media (width: 800px) and (device-width: 800px) {
      #target { color: rgb(4, 5, 6); }
    }
  `;
  const target = document.createElement('div');
  target.id = 'target';
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  head.appendChild(style);
  body.appendChild(target);
  const mediaRules = Array.from(style.sheet.cssRules)
    .filter(rule => rule instanceof CSSMediaRule)
    .map(rule => rule.matches)
    .join('|');
  return JSON.stringify({
    mediaRules,
    color: getComputedStyle(target).color
  });
})()
"#,
        )
        .expect("CSS media viewport surface probe should evaluate");

    assert_eq!(
        result,
        r#"{"mediaRules":"true|false","color":"rgb(1, 2, 3)"}"#
    );
}

#[test]
fn css_media_rule_serializes_normalized_query_and_block() {
    let mut vm = new_storage_test_vm("https://css-media-rule-serialization.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule('@media {}', 0);
  const emptyList = sheet.cssRules[0].cssText;
  sheet.insertRule('@media spEech {}', 0);
  const empty = sheet.cssRules[0].cssText;
  sheet.insertRule('@media all and (WiDtH) {}', 0);
  const feature = sheet.cssRules[0].cssText;
  sheet.cssRules[0].insertRule('#foo { z-index: 23; float: left; }', 0);
  const nested = sheet.cssRules[0].cssText;
  sheet.insertRule('@media all and (not-a-real-feature) {}', 0);
  const unknownFeature = sheet.cssRules[0].cssText;
  return [emptyList, empty, feature, nested, unknownFeature].join('||');
})()
"#,
        )
        .expect("CSSMediaRule serialization should evaluate");

    assert_eq!(
        result,
        "@media  {\n}||@media speech {\n}||@media (width) {\n}||@media (width) {\n  #foo { z-index: 23; float: left; }\n}||@media (not-a-real-feature) {\n}"
    );
}

#[test]
fn css_media_rule_media_mutation_uses_native_rule_serialization() {
    let mut vm = new_storage_test_vm("https://css-media-rule-media-mutation-stylo.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@supports (display: grid) { @media screen { .new { margin: 0; } } }');
  const supports = sheet.cssRules[0];
  const media = supports.cssRules[0];
  media.media.mediaText = 'all and (WiDtH)';
  return [
    media.media.mediaText,
    media.cssText,
    supports.cssText,
    sheet.cssRules[0].cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSMediaRule media mutation should use Stylo rule serialization");

    assert_eq!(
        result,
        "(width)|@media (width) {\n  .new { margin: 0px; }\n}|@supports (display: grid) {\n  @media (width) {\n  .new { margin: 0px; }\n}\n}|@supports (display: grid) {\n  @media (width) {\n  .new { margin: 0px; }\n}\n}"
    );
}

#[test]
fn css_media_rule_media_live_stylesheet_mutation_preserves_stylesheet_path() {
    let mut vm = new_storage_test_vm("https://css-media-rule-media-live-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@supports (display: grid) { @media screen { .one { margin: 0; } } }');
  const supports = sheet.cssRules[0];
  const supportsRules = supports.cssRules;
  const media = supportsRules[0];
  const mediaList = media.media;

  media.media.mediaText = 'print and (min-width: 10px)';
  const inserted = sheet.insertRule('.after { color: blue; }', 1);
  const deleted = sheet.deleteRule(1);

  return [
    inserted,
    deleted === undefined,
    sheet.cssRules.length,
    sheet.cssRules[0] === supports,
    supports.cssRules === supportsRules,
    supports.cssRules[0] === media,
    media.media === mediaList,
    media.media.mediaText,
    media.cssText,
    supports.cssText,
    sheet.cssRules[0].cssText,
  ].join('|');
})()
"#,
        )
        .expect("live CSSMediaRule media mutation path should evaluate");

    assert_eq!(
        result,
        "1|true|1|true|true|true|true|print and (min-width: 10px)|@media print and (min-width: 10px) {\n  .one { margin: 0px; }\n}|@supports (display: grid) {\n  @media print and (min-width: 10px) {\n  .one { margin: 0px; }\n}\n}|@supports (display: grid) {\n  @media print and (min-width: 10px) {\n  .one { margin: 0px; }\n}\n}"
    );
}

#[test]
fn css_media_rule_lazy_css_rules_use_live_stylesheet_after_media_mutation() {
    let mut vm = new_storage_test_vm("https://css-media-rule-lazy-rules-live-source.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@media screen { .one { color: red; } } .after { color: black; }');
  const media = sheet.cssRules[0];
  const mediaList = media.media;

  media.media.mediaText = 'print and (min-width: 10px)';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  const rules = media.cssRules;
  const child = rules[0];
  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === media,
    media.media === mediaList,
    media.media.mediaText,
    rules.length,
    child.selectorText,
    child.style.color,
    media.cssText.includes('print and (min-width: 10px)'),
    media.cssText.includes('.one'),
    sheet.cssRules[0].cssText === media.cssText,
    sheet.cssRules[1].cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSMediaRule lazy cssRules should use live Stylo source");

    assert_eq!(
        result,
        "2|true|true|print and (min-width: 10px)|1|.one|red|true|true|true|.after { color: black; }"
    );
}

#[test]
fn css_style_rule_style_live_stylesheet_mutation_preserves_stylesheet_path() {
    let mut vm = new_storage_test_vm("https://css-style-rule-style-live-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@supports (display: grid) { .one { color: red; & .child { color: blue; } } }');
  const supports = sheet.cssRules[0];
  const rule = supports.cssRules[0];
  const nestedRules = rule.cssRules;
  const nested = nestedRules[0];
  const style = rule.style;

  rule.style.color = 'green';
  sheet.insertRule('.after { color: blue; }', 1);
  sheet.deleteRule(1);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === supports,
    supports.cssRules[0] === rule,
    rule.cssRules === nestedRules,
    rule.cssRules[0] === nested,
    rule.style === style,
    rule.style.color,
    rule.cssText.includes('color: green'),
    rule.cssText.includes('& .child { color: blue; }'),
    supports.cssText.includes('color: green'),
    sheet.cssRules[0].cssText === supports.cssText,
  ].join('|');
})()
"#,
        )
        .expect("live CSSStyleRule style mutation path should evaluate");

    assert_eq!(
        result,
        "1|true|true|true|true|true|green|true|true|true|true"
    );
}

#[test]
fn css_nested_declarations_style_live_stylesheet_mutation_preserves_stylesheet_path() {
    let mut vm = new_storage_test_vm("https://css-nested-declarations-style-live-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@supports (display: grid) { .one { & .child { color: blue; } color: red; margin: 0; } }');
  const supports = sheet.cssRules[0];
  const rule = supports.cssRules[0];
  const nestedRules = rule.cssRules;
  const child = nestedRules[0];
  const declarations = nestedRules[1];
  const style = declarations.style;

  declarations.style.color = 'green';
  sheet.insertRule('.after { color: blue; }', 1);
  sheet.deleteRule(1);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === supports,
    supports.cssRules[0] === rule,
    rule.cssRules === nestedRules,
    rule.cssRules[0] === child,
    rule.cssRules[1] === declarations,
    declarations.style === style,
    declarations.style.color,
    declarations.cssText.includes('color: green'),
    rule.cssText.includes('& .child { color: blue; }'),
    rule.cssText.includes('color: green'),
    supports.cssText.includes('color: green'),
  ].join('|');
})()
"#,
        )
        .expect("live CSSNestedDeclarations style mutation path should evaluate");

    assert_eq!(
        result,
        "1|true|true|true|true|true|true|green|true|true|true|true"
    );
}

#[test]
fn css_keyframe_rule_style_live_stylesheet_mutation_preserves_stylesheet_path() {
    let mut vm = new_storage_test_vm("https://css-keyframe-rule-style-live-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes fade { from { opacity: 0; } to { opacity: 1; } }');
  const keyframes = sheet.cssRules[0];
  const keyframeRules = keyframes.cssRules;
  const keyframe = keyframeRules[1];
  const style = keyframe.style;

  keyframe.style.opacity = '.5';
  sheet.insertRule('.after { color: blue; }', 1);
  sheet.deleteRule(1);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === keyframes,
    keyframes.cssRules === keyframeRules,
    keyframes.cssRules[1] === keyframe,
    keyframe.style === style,
    keyframe.style.opacity,
    keyframe.cssText.includes('opacity: 0.5'),
    keyframes.cssText.includes('opacity: 0.5'),
    sheet.cssRules[0].cssText === keyframes.cssText,
  ].join('|');
})()
"#,
        )
        .expect("live CSSKeyframeRule style mutation path should evaluate");

    assert_eq!(result, "1|true|true|true|true|0.5|true|true|true");
}

#[test]
fn css_style_rule_css_text_reset_live_stylesheet_mutation_preserves_stylesheet_path() {
    let mut vm = new_storage_test_vm("https://css-style-rule-css-text-live-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@supports (display: grid) { .one { color: red; margin: 0; } }');
  const supports = sheet.cssRules[0];
  const rule = supports.cssRules[0];

  rule.cssText = '.one { color: green; padding: 1px; }';
  const style = rule.style;
  sheet.insertRule('.after { color: blue; }', 1);
  sheet.deleteRule(1);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === supports,
    supports.cssRules[0] === rule,
    rule.style === style,
    rule.style.color,
    rule.style.paddingTop,
    rule.cssText.includes('color: green'),
    rule.cssText.includes('padding: 1px'),
    supports.cssText.includes('color: green'),
    sheet.cssRules[0].cssText === supports.cssText,
  ].join('|');
})()
"#,
        )
        .expect("live CSSStyleRule cssText reset path should evaluate");

    assert_eq!(result, "1|true|true|true|green|1px|true|true|true|true");
}

#[test]
fn css_style_rule_css_text_reset_with_style_wrapper_updates_selector() {
    let mut vm = new_storage_test_vm("https://css-style-rule-css-text-selector-wrapper.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.one { color: red; }');
  const rule = sheet.cssRules[0];
  const style = rule.style;

  rule.cssText = '.two { color: green; padding: 1px; }';

  return [
    sheet.cssRules[0] === rule,
    rule.style === style,
    rule.selectorText,
    rule.style.color,
    rule.style.paddingTop,
    rule.cssText.includes('.two'),
    sheet.cssRules[0].cssText.includes('.two'),
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleRule cssText reset with existing style wrapper should evaluate");

    assert_eq!(result, "true|true|.two|green|1px|true|true");
}

#[test]
fn css_style_rule_css_text_replace_selector_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-style-rule-css-text-replace-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@namespace svg url("http://www.w3.org/2000/svg"); .one { color: red; } .after { color: black; }');
  const rule = sheet.cssRules[1];
  const style = rule.style;

  rule.cssText = 'svg|path { color: blue; & > .icon { opacity: .5; } }';
  const nestedRules = rule.cssRules;
  const nested = nestedRules[0];
  sheet.insertRule('.temp { color: green; }', 3);
  sheet.deleteRule(3);

  return [
    sheet.cssRules.length,
    sheet.cssRules[1] === rule,
    rule.style === style,
    rule.selectorText,
    rule.style.color,
    rule.cssRules === nestedRules,
    nestedRules[0] === nested,
    nested.selectorText,
    nested.style.opacity,
    sheet.cssRules[1].cssText === rule.cssText,
    sheet.cssRules[1].cssText.includes('svg|path'),
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleRule cssText replace should preserve live tree");

    assert_eq!(
        result,
        "3|true|true|svg|path|blue|true|true|& > .icon|0.5|true|true"
    );
}

#[test]
fn css_style_rule_css_text_invalid_multi_rule_reset_restores_native_rule() {
    let mut vm =
        new_storage_test_vm("https://css-style-rule-css-text-invalid-reset-rollback.test/");

    let initial_length = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  const children = Array.from(
    { length: 1000 },
    (_, index) => `& .child-${index} { opacity: .5; }`
  ).join('\n');
  sheet.replaceSync(`.host { color: red; ${children} } .after { color: black; }`);
  const rule = sheet.cssRules[0];
  const style = rule.style;
  const rules = rule.cssRules;
  const child = rules[0];
  const after = sheet.cssRules[1];
  const before = rule.cssText;

  globalThis.__invalidResetSheet = sheet;
  globalThis.__invalidResetRule = rule;
  globalThis.__invalidResetStyle = style;
  globalThis.__invalidResetRules = rules;
  globalThis.__invalidResetChild = child;
  globalThis.__invalidResetAfter = after;
  globalThis.__invalidResetBefore = before;
  return rules.length;
})()
"#,
        )
        .expect("invalid CSSStyleRule rollback fixture should initialize");
    assert_eq!(initial_length, "1000");

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_wrapper_construction_count_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = globalThis.__invalidResetSheet;
  const rule = globalThis.__invalidResetRule;
  const style = globalThis.__invalidResetStyle;
  const rules = globalThis.__invalidResetRules;
  const child = globalThis.__invalidResetChild;
  const after = globalThis.__invalidResetAfter;
  const before = globalThis.__invalidResetBefore;

  rule.cssText = '.host { color: green; } .extra { color: purple; }';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === rule,
    rule.style === style,
    rule.cssRules === rules,
    rules[0] === child,
    rules.length,
    rule.style.color,
    child.selectorText,
    child.style.opacity,
    rule.cssText === before,
    !rule.cssText.includes('.extra'),
    sheet.cssRules[0].cssText === rule.cssText,
    after.cssText,
  ].join('|');
})()
"#,
        )
        .expect("invalid CSSStyleRule cssText reset should restore attached native state");

    assert_eq!(
        result,
        "2|true|true|true|true|1000|red|& .child-0|0.5|true|true|true|.after { color: black; }"
    );
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        0,
        "failed attached cssText replacement must restore only the root wrapper"
    );
}

#[test]
fn css_style_rule_selector_text_mutation_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-style-rule-selector-text-live-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@namespace svg url("http://www.w3.org/2000/svg"); .one { color: red; & > .icon { opacity: .5; } } .after { color: black; }');
  const rule = sheet.cssRules[1];
  const style = rule.style;
  const nestedRules = rule.cssRules;
  const nested = nestedRules[0];

  rule.selectorText = 'svg|path';
  sheet.insertRule('.temp { color: green; }', 3);
  sheet.deleteRule(3);

  return [
    sheet.cssRules.length,
    sheet.cssRules[1] === rule,
    rule.style === style,
    rule.cssRules === nestedRules,
    nestedRules[0] === nested,
    rule.selectorText,
    rule.style.color,
    nested.selectorText,
    nested.style.opacity,
    sheet.cssRules[1].cssText === rule.cssText,
    sheet.cssRules[1].cssText.includes('svg|path'),
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleRule selectorText mutation should preserve live tree");

    assert_eq!(
        result,
        "3|true|true|true|true|svg|path|red|& > .icon|0.5|true|true"
    );
}

#[test]
fn css_style_rule_public_reads_use_attached_native_rule() {
    let mut vm = new_storage_test_vm("https://css-style-rule-public-read-stylo-view.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@namespace svg url("http://www.w3.org/2000/svg"); .host { color: red; } .after { color: black; }');
  const rule = sheet.cssRules[1];

  rule.cssText = 'svg|path { color: blue; & > .icon { opacity: .5; } }';
  const rules = rule.cssRules;
  const nested = rules[0];

  return [
    sheet.cssRules[1] === rule,
    rule.selectorText,
    rules.length,
    nested.selectorText,
    nested.cssText,
    nested.parentRule === rule,
    sheet.cssRules[1].cssText === rule.cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleRule public reads should use the attached native rule");

    assert_eq!(
        result,
        "true|svg|path|1|& > .icon|& > .icon { opacity: 0.5; }|true|true"
    );
}

#[test]
fn css_nested_style_rule_css_text_replace_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-nested-style-rule-css-text-replace-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.host { color: red; & .old { color: blue; } font-size: 12px; }');
  const parent = sheet.cssRules[0];
  const rules = parent.cssRules;
  const child = rules[0];
  const childStyle = child.style;

  child.cssText = '> .new { color: green; padding: 1px; }';
  parent.insertRule('.later { color: purple; }', 2);
  parent.deleteRule(2);

  return [
    sheet.cssRules[0] === parent,
    parent.cssRules === rules,
    rules[0] === child,
    child.style === childStyle,
    child.selectorText,
    child.style.color,
    child.style.paddingTop,
    parent.cssText.includes('& > .new'),
    parent.cssText.includes('font-size: 12px'),
    sheet.cssRules[0].cssText === parent.cssText,
  ].join('|');
})()
"#,
        )
        .expect("nested CSSStyleRule cssText replace should preserve live tree");

    assert_eq!(
        result,
        "true|true|true|true|& > .new|green|1px|true|true|true"
    );
}

#[test]
fn css_nested_style_rule_selector_text_mutation_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-nested-style-rule-selector-text-live-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.host { color: red; & .old { color: blue; } font-size: 12px; }');
  const parent = sheet.cssRules[0];
  const rules = parent.cssRules;
  const child = rules[0];
  const childStyle = child.style;

  child.selectorText = '> .new';
  parent.insertRule('.later { color: purple; }', 2);
  parent.deleteRule(2);

  return [
    sheet.cssRules[0] === parent,
    parent.cssRules === rules,
    rules[0] === child,
    child.style === childStyle,
    child.selectorText,
    child.style.color,
    parent.cssText.includes('& > .new'),
    parent.cssText.includes('font-size: 12px'),
    sheet.cssRules[0].cssText === parent.cssText,
  ].join('|');
})()
"#,
        )
        .expect("nested CSSStyleRule selectorText mutation should preserve live tree");

    assert_eq!(result, "true|true|true|true|& > .new|blue|true|true|true");
}

#[test]
fn css_keyframe_rule_css_text_reset_live_stylesheet_mutation_preserves_stylesheet_path() {
    let mut vm = new_storage_test_vm("https://css-keyframe-rule-css-text-live-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes fade { from { opacity: 0; } to { opacity: 1; } }');
  const keyframes = sheet.cssRules[0];
  const keyframeRules = keyframes.cssRules;
  const keyframe = keyframeRules[1];

  keyframe.cssText = 'to { opacity: .25; transform: translateX(1px); }';
  const style = keyframe.style;
  sheet.insertRule('.after { color: blue; }', 1);
  sheet.deleteRule(1);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === keyframes,
    keyframes.cssRules === keyframeRules,
    keyframes.cssRules[1] === keyframe,
    keyframe.style === style,
    keyframe.style.opacity,
    keyframe.style.transform,
    keyframe.cssText.includes('opacity: 0.25'),
    keyframes.cssText.includes('translateX(1px)'),
    sheet.cssRules[0].cssText === keyframes.cssText,
  ].join('|');
})()
"#,
        )
        .expect("live CSSKeyframeRule cssText reset path should evaluate");

    assert_eq!(
        result,
        "1|true|true|true|true|0.25|translateX(1px)|true|true|true"
    );
}

#[test]
fn css_keyframe_rule_css_text_reset_with_style_wrapper_updates_key_text() {
    let mut vm = new_storage_test_vm("https://css-keyframe-rule-css-text-key-wrapper.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes fade { from { opacity: 0; } to { opacity: 1; } }');
  const keyframes = sheet.cssRules[0];
  const keyframe = keyframes.cssRules[1];
  const style = keyframe.style;

  keyframe.cssText = '80% { opacity: .8; transform: scale(1); }';

  return [
    keyframe.keyText,
    keyframe.cssText,
    keyframes.findRule('80%') === keyframe,
    keyframes.findRule('100%') === null,
    keyframe.style === style,
    keyframe.style.opacity,
    keyframe.style.transform,
    keyframes.cssText.includes('80%'),
  ].join('|');
})()
"#,
        )
        .expect("CSSKeyframeRule cssText reset with existing style wrapper should evaluate");

    assert_eq!(
        result,
        "80%|80% { opacity: 0.8; transform: scale(1); }|true|true|true|0.8|scale(1)|true"
    );
}

#[test]
fn css_keyframe_rule_css_text_replace_key_text_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-keyframe-rule-css-text-replace-tree.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes fade { from { opacity: 0; } to { opacity: 1; } }');
  const keyframes = sheet.cssRules[0];
  const keyframeRules = keyframes.cssRules;
  const keyframe = keyframeRules[1];
  const style = keyframe.style;

  keyframe.cssText = '80% { opacity: .8; transform: scale(1); }';
  keyframes.appendRule('50% { opacity: .5; }');
  keyframes.deleteRule('50%');

  return [
    sheet.cssRules[0] === keyframes,
    keyframes.cssRules === keyframeRules,
    keyframes.cssRules[1] === keyframe,
    keyframe.style === style,
    keyframe.keyText,
    keyframe.style.opacity,
    keyframe.style.transform,
    keyframes.findRule('80%') === keyframe,
    keyframes.findRule('100%') === null,
    sheet.cssRules[0].cssText === keyframes.cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSKeyframeRule cssText replace should preserve live tree");

    assert_eq!(
        result,
        "true|true|true|true|80%|0.8|scale(1)|true|true|true"
    );
}

#[test]
fn css_import_rule_media_mutation_preserves_conditions_with_stylo_serialization() {
    let mut vm = new_storage_test_vm("https://css-import-rule-media-mutation-stylo.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = '@import url("support/c.css") layer(A.B) supports((display: flex) or (foo: bar)); body { color: red; }';
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  head.appendChild(style);

  const rule = style.sheet.cssRules[0];
  const media = rule.media;
  media.mediaText = 'print and (WiDtH)';
  style.sheet.insertRule('.temp { color: green; }', 2);
  style.sheet.deleteRule(2);
  return [
    style.sheet.cssRules.length,
    style.sheet.cssRules[0] === rule,
    rule.media === media,
    media.mediaText,
    rule.layerName,
    rule.supportsText,
    rule.cssText,
    style.sheet.cssRules[0].cssText,
    style.sheet.cssRules[1].cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSImportRule media mutation should preserve import conditions");

    assert_eq!(
        result,
        "2|true|true|print and (width)|A.B|(display: flex) or (foo: bar)|@import url(\"support/c.css\") layer(A.B) supports((display: flex) or (foo: bar)) print and (width);|@import url(\"support/c.css\") layer(A.B) supports((display: flex) or (foo: bar)) print and (width);|body { color: red; }"
    );
}

#[test]
fn css_import_rule_media_mutation_retains_loaded_child_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-import-rule-loaded-media-mutation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = '@import url("data:text/css,.child%7Bcolor:green%7D") screen;';
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  head.appendChild(style);

  const rule = style.sheet.cssRules[0];
  const media = rule.media;
  const child = rule.styleSheet;
  const childRule = child.cssRules[0];
  media.mediaText = 'print';

  return [
    style.sheet.cssRules[0] === rule,
    rule.media === media,
    rule.styleSheet === child,
    child.cssRules[0] === childRule,
    media.mediaText,
    child.media.mediaText,
    childRule.cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSImportRule media mutation should retain the loaded child stylesheet");

    assert_eq!(
        result,
        "true|true|true|true|print||.child { color: green; }"
    );
}

#[test]
fn imported_stylesheet_runtime_state_is_independent_from_import_cascade_state() {
    let mut vm = new_storage_test_vm("https://css-import-runtime-state.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const probe = document.createElement('div');
  probe.className = 'child';
  body.appendChild(probe);

  const style = document.createElement('style');
  style.textContent = '@import url("data:text/css,.child%7Bcolor:green%7D") screen;';
  head.appendChild(style);

  const importRule = style.sheet.cssRules[0];
  const child = importRule.styleSheet;
  const initialColor = getComputedStyle(probe).color;

  child.media.mediaText = 'print';
  child.disabled = true;
  const childStateIsIndependent =
    child.media.mediaText === 'print' &&
    child.disabled === true &&
    getComputedStyle(probe).color === initialColor;

  importRule.media.mediaText = 'print';
  const importMediaControlsCascade = getComputedStyle(probe).color !== initialColor;

  importRule.media.mediaText = 'screen';
  const childDisabledDoesNotControlCascade =
    child.disabled === true && getComputedStyle(probe).color === initialColor;

  return [
    childStateIsIndependent,
    importMediaControlsCascade,
    childDisabledDoesNotControlCascade,
    importRule.styleSheet === child,
  ].join('|');
})()
"#,
        )
        .expect("imported stylesheet runtime state should remain CSSOM-local");

    assert_eq!(result, "true|true|true|true");
}

#[test]
fn css_stylesheet_and_import_rule_expose_media_lists() {
    let mut vm = new_storage_test_vm("https://css-media-list-owners.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.media = 'all';
  style.textContent = '@import url("support/a.css") screen; @import url("support/b.css") supports((display: flex) or (display: block)); @import url("support/c.css") layer(A.B) supports((display: flex) or (foo: bar)); @page { background-color: red; @top-left { content: "x"; } } body { color: red; }';
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  head.appendChild(style);
  const sheet = style.sheet;
  const sheetMedia = sheet.media;
  sheetMedia.appendMedium('screen');
  sheetMedia.deleteMedium('all');
  const sheetPart = [
    sheetMedia instanceof MediaList,
    sheet.media === sheetMedia,
    sheetMedia.length,
    sheetMedia.mediaText,
    sheetMedia.item(0),
    style.getAttribute('media')
  ].join('|');

  const rule = sheet.cssRules[0];
  const supportsRule = sheet.cssRules[1];
  const layeredRule = sheet.cssRules[2];
  const pageRule = sheet.cssRules[3];
  const importMedia = rule.media;
  const importSheet = rule.styleSheet;
  let deleteMissing = 'no-throw';
  try {
    importMedia.deleteMedium('print');
  } catch (error) {
    deleteMissing = error.name;
  }
  rule.media = 'print';
  const importPart = [
    importMedia instanceof MediaList,
    rule.media === importMedia,
    importMedia.mediaText,
    rule.cssText,
    importSheet === null,
    rule.styleSheet === null,
    deleteMissing,
    supportsRule.supportsText
  ].join('|');

  const layeredImportPart = [
    layeredRule.cssText,
    layeredRule.media.length,
    layeredRule.media.mediaText,
    layeredRule.supportsText
  ].join('|');

  const marginRule = pageRule.cssRules[0];
  pageRule.style = 'margin-top: 10px;';
    marginRule.style.cssText = 'content: "y"; color: red;';
  const pagePart = [
    pageRule instanceof CSSPageRule,
    pageRule instanceof CSSGroupingRule,
    pageRule.type === CSSRule.PAGE_RULE,
    marginRule instanceof CSSMarginRule,
    marginRule instanceof CSSRule,
    marginRule.type === CSSRule.MARGIN_RULE,
    marginRule.name,
    marginRule.style === marginRule.style,
    marginRule.style.getPropertyValue('content'),
    marginRule.parentRule === pageRule,
    marginRule.parentStyleSheet === sheet,
    Object.getPrototypeOf(CSSPageRule) === CSSGroupingRule,
    Object.getPrototypeOf(CSSPageRule.prototype) === CSSGroupingRule.prototype,
    'cssRules' in pageRule,
    pageRule.cssRules.length,
    pageRule.style.cssText,
    pageRule.cssText
  ].join('|');

  sheet.media = 'speech';
  const setterPart = [
    sheet.media === sheetMedia,
    sheetMedia.mediaText,
    style.getAttribute('media')
  ].join('|');

  return [sheetPart, importPart, layeredImportPart, pagePart, setterPart].join('||');
})()
"#,
        )
        .expect("StyleSheet and CSSImportRule MediaList surfaces should evaluate");
    assert_eq!(
        result,
        "true|true|1|screen|screen|all||true|true|print|@import url(\"support/a.css\") print;|true|true|NotFoundError|(display: flex) or (display: block)||@import url(\"support/c.css\") layer(A.B) supports((display: flex) or (foo: bar));|0||(display: flex) or (foo: bar)||true|true|true|true|true|true|top-left|true|\"y\"|true|true|true|true|true|1|margin-top: 10px;|@page { margin-top: 10px; @top-left { content: \"y\"; color: red; } }||true|speech|all"
    );
}
#[test]
fn child_window_exposes_cssom_rule_constructors_for_inserted_rules() {
    let mut vm = new_storage_test_vm("https://css-rule-child-realm.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const iframe = document.createElement('iframe');
  body.appendChild(iframe);
  const doc = iframe.contentDocument;
  const head = doc.head || doc.documentElement.appendChild(doc.createElement('head'));
  const style = doc.createElement('style');
  head.appendChild(style);
  const sheet = style.sheet;
  style.remove();
  sheet.insertRule('.kaoru {}');
  const constructed = new iframe.contentWindow.CSSStyleSheet();
  constructed.insertRule('.kaoru {}');
  return [
    typeof iframe.contentWindow.CSSStyleRule,
    sheet.cssRules[0].constructor === iframe.contentWindow.CSSStyleRule,
    constructed.cssRules[0].constructor === iframe.contentWindow.CSSStyleRule
  ].join('|');
})()
"#,
        )
        .expect("child CSSOM rule constructors should evaluate");

    assert_eq!(result, "function|true|true");
}

#[tokio::test]
async fn committed_child_xml_inline_style_has_associated_sheet() {
    let mut vm = new_storage_test_vm("https://child-xml-inline-style.test/page.html");

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.id = 'child-xml-inline-style';
  const markup = `<html xmlns="http://www.w3.org/1999/xhtml">
    <head><style></style></head>
    <body><div id="target" foo="BAR"></div></body>
  </html>`;
  frame.src = 'data:application/xhtml+xml,' + encodeURIComponent(markup);
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child XML inline style setup should evaluate");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "child XML inline style document should commit",
    )
    .await;

    let child_handle = vm
        ._context_host
        .borrow()
        .child_browsing_context_handles_in_document_order()[0];
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .find(|realm| {
            vm.child_frame_realm_store
                .get(&realm.context_id)
                .is_some_and(|record| record.child_handle == child_handle)
        })
        .map(|realm| realm.context_id)
        .expect("committed child XML document should have a default realm");
    assert_eq!(
        vm.eval_in_child_default_context(
            child_context_id,
            r#"
(() => {
  const style = document.getElementsByTagName('style')[0];
  const target = document.getElementById('target');
  const initialSheet = style.sheet;
  const initialRuleCount = initialSheet === null ? -1 : initialSheet.cssRules.length;
  style.textContent = "[foo='bar' i] { visibility: hidden; }";
  return [
    document.contentType,
    initialSheet !== null,
    initialRuleCount,
    style.sheet === initialSheet,
    initialSheet.ownerNode === null,
    style.sheet === null ? -1 : style.sheet.cssRules.length,
    getComputedStyle(target).visibility
  ].join('|');
})()
"#,
        )
        .expect("child XML inline stylesheet should evaluate"),
        "application/xhtml+xml|true|0|false|true|1|hidden"
    );
}

#[test]
fn css_stylesheet_exposes_legacy_surface_and_metadata() {
    let mut vm = new_storage_test_vm("https://css-stylesheet-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const style = document.createElement('style');
  style.title = 'main';
  style.media = 'all';
  style.textContent = '@import url("support/a.css"); body { width: 50%; } #foo { height: 100px; }';
  head.appendChild(style);
  const sheet = style.sheet;
  const importRule = sheet.cssRules[0];
  const importSheet = importRule.styleSheet;
  const first = sheet.cssRules[1];
  const second = sheet.cssRules[2];
  first.randomProperty = 1;
  second.randomProperty = 2;
  sheet.insertRule('#bar { margin: 10px; }', 2);
  const afterInsert = [
    sheet.rules === sheet.cssRules,
    sheet.cssRules[1].randomProperty,
    sheet.cssRules[3].randomProperty,
    sheet.cssRules[2].cssText
  ].join('|');
  sheet.deleteRule(2);
  const afterDelete = [
    sheet.cssRules[1].randomProperty,
    sheet.cssRules[2].randomProperty
  ].join('|');
  const importMetaBeforeRemove = [
    importSheet === null,
    importRule.styleSheet === null,
    importRule.parentStyleSheet === sheet,
    importRule.href
  ].join('|');
  sheet.removeRule();
  const afterRemove = [
    sheet.cssRules[0].cssText,
    sheet.cssRules[0].selectorText,
    sheet.cssRules[0] === first,
    sheet.cssRules[0].randomProperty
  ].join('|');
  const addReturn = sheet.addRule('#foo', 'color: red', 1);
  const afterAdd = [addReturn, sheet.cssRules[1].cssText].join('|');
  sheet.addRule();
  const afterDefaultAdd = sheet.cssRules[sheet.cssRules.length - 1].cssText;
  const empty = document.createElement('style');
  head.appendChild(empty);
  let removeEmpty = 'no-throw';
  try {
    empty.sheet.removeRule(0);
  } catch (error) {
    removeEmpty = error.name;
  }
  const disabledBefore = style.disabled;
  style.disabled = true;
  const disabledAfterStyle = [style.disabled, sheet.disabled, style.hasAttribute('disabled')].join('|');
  sheet.disabled = false;
  const disabledAfterSheet = [style.disabled, sheet.disabled, style.hasAttribute('disabled')].join('|');
  sheet.disabled = true;
  style.removeAttribute('disabled');
  const disabledAfterSheetOnly = [style.disabled, sheet.disabled, style.hasAttribute('disabled')].join('|');
  sheet.disabled = false;
  const meta = [
    sheet.type,
    sheet.ownerNode === style,
    sheet.parentStyleSheet === null,
    sheet.href === null,
    sheet.title,
    sheet.media.mediaText
  ].join('|');
  sheet.insertRule('@import url("support/b.css");', 0);
  const removableImportSheet = sheet.cssRules[0].styleSheet;
  sheet.deleteRule(0);
  const importMetaAfterRemove = [
    removableImportSheet === null,
    sheet.cssRules[0] !== importRule,
    importRule.styleSheet === null,
    importRule.parentStyleSheet === null
  ].join('|');
  const importMeta = [
    importSheet === null,
    importRule.parentStyleSheet === null,
    importRule.styleSheet === null,
    importRule.href === 'support/a.css'
  ].join('|');
  return [
    afterInsert,
    afterDelete,
    afterRemove,
    afterAdd,
    afterDefaultAdd,
    removeEmpty,
    disabledBefore,
    disabledAfterStyle,
    disabledAfterSheet,
    disabledAfterSheetOnly,
    meta,
    importMetaBeforeRemove,
    importMetaAfterRemove,
    importMeta
  ].join('||');
})()
"#,
        )
        .expect("CSSStyleSheet legacy surface and metadata should evaluate");

    assert_eq!(
        result,
        "true|1|2|#bar { margin: 10px; }||1|2||body { width: 50%; }|body|true|1||-1|#foo { color: red; }||undefined { }||IndexSizeError||false||true|true|false||false|false|false||true|true|false||text/css|true|true|true|main|all||true|true|true|support/a.css||true|true|true|true||true|true|true|true"
    );
}

#[test]
fn css_stylesheet_add_rule_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-stylesheet-add-rule-live.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.before { color: black; } .after { color: blue; }');
  const first = sheet.cssRules[0];
  const second = sheet.cssRules[1];
  first.marker = 'first';
  second.marker = 'second';

  const addReturn = sheet.addRule('.inserted', 'margin: 1px', 1);
  sheet.insertRule('.temp { color: green; }', 3);
  sheet.deleteRule(3);

  return [
    addReturn,
    sheet.cssRules.length,
    sheet.cssRules[0] === first,
    sheet.cssRules[2] === second,
    sheet.cssRules[0].marker,
    sheet.cssRules[2].marker,
    sheet.cssRules[1].selectorText,
    sheet.cssRules[1].style.margin,
    sheet.cssRules[1].cssText,
    sheet.cssRules[0].cssText,
    sheet.cssRules[2].cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleSheet.addRule should preserve Stylo rule tree");

    assert_eq!(
        result,
        "-1|3|true|true|first|second|.inserted|1px|.inserted { margin: 1px; }|.before { color: black; }|.after { color: blue; }"
    );
}

#[test]
fn html_style_disabled_waits_for_associated_stylesheet() {
    let mut vm = new_storage_test_vm("https://style-disabled-cssom.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const style = document.createElement('style');
  style.disabled = true;
  const beforeAppend = [
    style.disabled,
    style.hasAttribute('disabled'),
    style.sheet === null
  ].join('|');
  head.append(style);
  const sheet = style.sheet;
  const afterAppend = [
    style.disabled,
    sheet.disabled,
    style.hasAttribute('disabled')
  ].join('|');
  style.disabled = true;
  const afterStyleSet = [
    style.disabled,
    sheet.disabled,
    style.hasAttribute('disabled')
  ].join('|');
  style.disabled = false;
  sheet.disabled = true;
  const afterSheetSet = [
    style.disabled,
    sheet.disabled,
    style.hasAttribute('disabled')
  ].join('|');

  const attributed = document.createElement('style');
  attributed.setAttribute('disabled', '');
  head.append(attributed);
  const attributedSheet = attributed.sheet;
  const contentAttributeInitial = [
    attributed.disabled,
    attributedSheet.disabled,
    attributed.hasAttribute('disabled')
  ].join('|');
  attributed.media = 'screen';
  const contentAttributeAfterMedia = [
    attributed.disabled,
    attributedSheet.disabled,
    attributed.hasAttribute('disabled')
  ].join('|');
  return [
    beforeAppend,
    afterAppend,
    afterStyleSet,
    afterSheetSet,
    contentAttributeInitial,
    contentAttributeAfterMedia
  ].join('||');
})()
"#,
        )
        .expect("HTMLStyleElement.disabled should follow the associated stylesheet");

    assert_eq!(
        result,
        "false|false|true||false|false|false||true|true|false||true|true|false||false|false|true||false|false|true"
    );
}
#[test]
fn preferred_stylesheet_title_filters_cascade_only() {
    let mut vm = new_storage_test_vm("https://stylesheet-title.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const target = document.createElement('p');
  target.id = 'test-element';
  body.appendChild(target);
  const rules = [
    [null, ''],
    ['', ''],
    ['Preferred', 'p { color: green; }'],
    ['Not preferred', 'p { color: red; }'],
  ];
  for (const [title, text] of rules) {
    const style = document.createElement('style');
    if (title !== null) {
      style.setAttribute('title', title);
    }
    style.textContent = text;
    head.appendChild(style);
  }
  const titles = Array.from(document.styleSheets).map(sheet => sheet.title === null ? 'null' : sheet.title);
  return [getComputedStyle(target).color, titles.join(',')].join('|');
})()
"#,
        )
        .expect("preferred stylesheet title should filter cascade");

    assert_eq!(result, "rgb(0, 128, 0)|null,null,Preferred,Not preferred");
}
#[test]
fn stylesheet_list_intrinsic_iterator_ignores_item_tampering() {
    let mut vm = new_storage_test_vm("https://stylesheet-list-intrinsic-iterator.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const style = document.createElement('style');
  style.textContent = 'body { color: black; }';
  head.appendChild(style);
  const list = document.styleSheets;
  const first = list[0];
  Object.defineProperty(list, 'item', {
    configurable: true,
    value() {
      throw new Error('item boom');
    },
  });
  const iterated = Array.from(list);
  return [iterated.length, iterated[0] === first].join('|');
})()
"#,
        )
        .expect("StyleSheetList intrinsic iterator should ignore item() tampering");

    assert_eq!(result, "1|true");
}

#[test]
fn retained_stylesheet_list_tracks_nested_candidate_tree_scope_transitions() {
    let mut vm = new_storage_test_vm("https://retained-stylesheet-list.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const list = document.styleSheets;
  const fragment = document.createDocumentFragment();
  const wrapper = document.createElement('section');
  const style = document.createElement('style');
  style.textContent = 'body { color: rgb(1, 2, 3); }';
  wrapper.appendChild(style);
  fragment.appendChild(wrapper);
  const initial = list.length;
  head.appendChild(fragment);
  const inserted = list.length;
  wrapper.remove();
  const removed = list.length;
  head.appendChild(wrapper);
  const reconnected = list.length;
  const shadowHost = document.createElement('div');
  body.appendChild(shadowHost);
  shadowHost.attachShadow({ mode: 'open' }).appendChild(wrapper);
  const movedToShadow = list.length;
  return [initial, inserted, removed, reconnected, movedToShadow].join('|');
})()
"#,
        )
        .expect("retained StyleSheetList should follow typed TreeScope mutation effects");

    assert_eq!(result, "0|1|0|1|0");
}

#[test]
fn preferred_stylesheet_title_uses_add_order_not_tree_order() {
    let mut vm = new_storage_test_vm("https://stylesheet-title-reversed.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const target = document.createElement('div');
  target.id = 't1';
  body.appendChild(target);
  function createStyleElement(text, title) {
    const element = document.createElement('style');
    element.setAttribute('title', title);
    element.appendChild(document.createTextNode(text));
    return element;
  }
  head.insertBefore(createStyleElement('#t1 { color: green; }', 'preferred'), head.firstChild);
  head.insertBefore(createStyleElement('#t1 { color: red; }', 'notpreferred'), head.firstChild);
  return getComputedStyle(target).color;
})()
"#,
        )
        .expect("preferred stylesheet title should use stylesheet add order");

    assert_eq!(result, "rgb(0, 128, 0)");
}
#[test]
fn css_stylesheet_insert_rule_enforces_import_namespace_ordering() {
    let mut vm = new_storage_test_vm("https://css-insert-rule-order.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const importStyle = document.createElement('style');
  importStyle.textContent = '@import url("support/a.css");';
  head.appendChild(importStyle);
  const importSheet = importStyle.sheet;
  let styleBeforeImport = 'no-throw';
  try {
    importSheet.insertRule('p { color: green; }');
  } catch (error) {
    styleBeforeImport = error.name;
  }
  const layerStatementIndex = importSheet.insertRule('@layer first, second;', 0);
  let layerBlockBeforeImport = 'no-throw';
  try {
    importSheet.insertRule('@layer third {}', 0);
  } catch (error) {
    layerBlockBeforeImport = error.name;
  }
  let undefinedStyleBeforeImport = 'no-throw';
  try {
    importSheet.insertRule('p { color: yellow; }', undefined);
  } catch (error) {
    undefinedStyleBeforeImport = error.name;
  }

  const namespaceStyle = document.createElement('style');
  namespaceStyle.textContent = '@namespace svg url("http://servo"); @namespace url("http://servo1");';
  head.appendChild(namespaceStyle);
  const namespaceSheet = namespaceStyle.sheet;
  let styleBeforeNamespace = 'no-throw';
  try {
    namespaceSheet.insertRule('p { color: green; }');
  } catch (error) {
    styleBeforeNamespace = error.name;
  }
  namespaceSheet.insertRule('@import url("support/b.css");');

  return [
    importSheet.cssRules.length,
    styleBeforeImport,
    layerStatementIndex,
    importSheet.cssRules.item(0).cssText,
    layerBlockBeforeImport,
    undefinedStyleBeforeImport,
    importSheet.cssRules.item(1).cssText,
    namespaceSheet.cssRules.length,
    styleBeforeNamespace,
    namespaceSheet.cssRules.item(0).cssText
  ].join('|');
})()
"#,
        )
        .expect("insertRule ordering should evaluate");

    assert_eq!(
        result,
        "2|HierarchyRequestError|0|@layer first, second;|HierarchyRequestError|HierarchyRequestError|@import url(\"support/a.css\");|3|HierarchyRequestError|@import url(\"support/b.css\");"
    );
}
#[test]
fn link_style_sheet_attribute_lives_on_prototype() {
    let mut vm = new_storage_test_vm("https://link-style-sheet-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  function prototypeOwns(value, name) {
    for (let proto = Object.getPrototypeOf(value); proto; proto = Object.getPrototypeOf(proto)) {
      if (Object.prototype.hasOwnProperty.call(proto, name)) {
        return true;
      }
    }
    return false;
  }

  const style = document.createElement('style');
  const link = document.createElement('link');
  link.rel = 'stylesheet';

  const placement = [
    Object.prototype.hasOwnProperty.call(style, 'sheet'),
    'sheet' in style,
    prototypeOwns(style, 'sheet'),
    Object.prototype.hasOwnProperty.call(link, 'sheet'),
    'sheet' in link,
    prototypeOwns(link, 'sheet')
  ].join('|');

  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  head.append(style, link);
  style.textContent = 'body { color: red; }';
  const styleSheet = style.sheet;
  const linkSheet = link.sheet;
  return [
    placement,
    styleSheet instanceof CSSStyleSheet,
    styleSheet.ownerNode === style,
    styleSheet.cssRules.length,
    linkSheet === null
  ].join('||');
})()
"#,
        )
        .expect("LinkStyle sheet prototype placement should evaluate");

    assert_eq!(
        result,
        "false|true|true|false|true|true||true||true||1||true"
    );
}

#[test]
fn disconnected_style_owners_do_not_expose_unbound_stylesheets() {
    let mut vm = new_storage_test_vm("https://disconnected-stylesheet-owner.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const SVG_NS = 'http://www.w3.org/2000/svg';
  const style = document.createElement('style');
  style.textContent = 'body { color: red; }';
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  const svg = document.createElementNS(SVG_NS, 'svg');
  const svgStyle = document.createElementNS(SVG_NS, 'style');
  svgStyle.textContent = 'body { color: green; }';
  svg.appendChild(svgStyle);
  const before = [style.sheet, link.sheet, svgStyle.sheet].map(sheet => sheet === null);

  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  head.appendChild(style);
  body.appendChild(svg);
  const styleSheet = style.sheet;
  const svgSheet = svgStyle.sheet;
  style.remove();
  svg.remove();

  return [
    ...before,
    styleSheet instanceof CSSStyleSheet,
    svgSheet instanceof CSSStyleSheet,
    style.sheet === null,
    svgStyle.sheet === null,
    styleSheet.ownerNode === null,
    svgSheet.ownerNode === null,
  ].join('|');
})()
"#,
        )
        .expect("disconnected stylesheet owner behavior should evaluate");

    assert_eq!(result, "true|true|true|true|true|true|true|true|true");
}

#[test]
fn link_disabled_controls_stylesheet_exposure_and_explicit_enable_state() {
    let mut vm = new_storage_test_vm("https://link-disabled-stylesheet.test/");
    let linked_url =
        url::Url::parse("data:text/css,html%20%7B%20background:%20green%20%7D").unwrap();
    let alternate_url =
        url::Url::parse("data:text/css,html%20%7B%20background:%20rgb(1,%202,%203)%20%7D").unwrap();

    let initial = vm
        .eval(
            r#"
(() => {
  function background() {
    return getComputedStyle(document.documentElement).backgroundColor;
  }
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const link = document.createElement('link');
  link.id = 'disabled-link';
  link.rel = 'stylesheet';
  link.type = 'text/css; charset=utf-8';
  link.href = 'data:text/css,html%20%7B%20background:%20green%20%7D';
  link.disabled = true;
  head.appendChild(link);

  return [
    link.disabled,
    link.hasAttribute('disabled'),
    document.styleSheets.length,
    link.sheet === null,
    background()
  ].join(',');
})()
"#,
        )
        .expect("disabled linked stylesheet setup should evaluate");

    vm.eval("document.getElementById('disabled-link').disabled = false")
        .expect("enabling linked stylesheet should evaluate");
    let link = cssom_element_handle_by_id(&vm, "disabled-link");
    install_linked_stylesheet_for_test(
        &mut vm,
        link,
        linked_url.clone(),
        crate::style_engine::StyloStylesheetSource::new(
            "html { background: green }".to_owned(),
            linked_url.clone(),
        )
        .with_sheet_url(linked_url),
    );

    let enabled_and_disabled_again = vm
        .eval(
            r#"
(() => {
  function background() {
    return getComputedStyle(document.documentElement).backgroundColor;
  }
  const link = document.getElementById('disabled-link');

  const sheet = document.styleSheets[0];
  const enabled = [
    link.disabled,
    link.hasAttribute('disabled'),
    document.styleSheets.length,
    sheet.ownerNode === link,
    sheet.cssRules.length,
    background()
  ].join(',');

  link.disabled = true;
  const disabledAgain = [
    link.disabled,
    link.hasAttribute('disabled'),
    document.styleSheets.length,
    sheet.ownerNode === null,
    sheet.disabled,
    background()
  ].join(',');
  link.remove();

  const alternate = document.createElement('link');
  alternate.id = 'alternate-link';
  alternate.rel = 'alternate stylesheet';
  alternate.title = 'alt';
  alternate.href = 'data:text/css,html%20%7B%20background:%20rgb(1,%202,%203)%20%7D';
  alternate.disabled = true;
  alternate.disabled = false;
  globalThis.__alternateLink = alternate;
  const clone = alternate.cloneNode(false);
  clone.id = 'alternate-clone';
  document.head.appendChild(clone);
  return [enabled, disabledAgain].join('|');
})()
"#,
        )
        .expect("link disabled stylesheet state should evaluate");

    let clone = cssom_element_handle_by_id(&vm, "alternate-clone");
    install_linked_stylesheet_for_test(
        &mut vm,
        clone,
        alternate_url.clone(),
        crate::style_engine::StyloStylesheetSource::new(
            "html { background: rgb(1, 2, 3) }".to_owned(),
            alternate_url.clone(),
        )
        .with_sheet_url(alternate_url.clone()),
    );
    let clone_background = vm
        .eval(
            r#"
(() => {
  const result = getComputedStyle(document.documentElement).backgroundColor;
  document.getElementById('alternate-clone').remove();
  document.head.appendChild(globalThis.__alternateLink);
  delete globalThis.__alternateLink;
  return result;
})()
"#,
        )
        .expect("alternate clone stylesheet should evaluate");

    let alternate = cssom_element_handle_by_id(&vm, "alternate-link");
    install_linked_stylesheet_for_test(
        &mut vm,
        alternate,
        alternate_url.clone(),
        crate::style_engine::StyloStylesheetSource::new(
            "html { background: rgb(1, 2, 3) }".to_owned(),
            alternate_url.clone(),
        )
        .with_sheet_url(alternate_url),
    );
    let alternate_background = vm
        .eval("getComputedStyle(document.documentElement).backgroundColor")
        .expect("alternate stylesheet should evaluate");

    let result =
        format!("{initial}|{enabled_and_disabled_again}|{clone_background}|{alternate_background}");

    assert_eq!(
        result,
        "true,true,0,true,rgba(0, 0, 0, 0)|false,false,1,true,1,rgb(0, 128, 0)|true,true,0,true,false,rgba(0, 0, 0, 0)|rgba(0, 0, 0, 0)|rgb(1, 2, 3)"
    );
}

#[test]
fn cached_linked_stylesheet_rebinds_synchronously_across_rel_and_shadow_scope() {
    let mut vm = new_storage_test_vm("https://link-rel-cache.test/");
    let stylesheet_url = url::Url::parse("https://link-rel-cache.test/shared.css").unwrap();

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const link = document.createElement('link');
  link.id = 'light-link';
  link.rel = 'stylesheet';
  link.href = '/shared.css';
  head.appendChild(link);
  const target = document.createElement('div');
  target.id = 'light-target';
  target.className = 'green';
  body.appendChild(target);
})()
"#,
    )
    .expect("linked stylesheet cache fixture should evaluate");
    let link = cssom_element_handle_by_id(&vm, "light-link");
    install_linked_stylesheet_for_test(
        &mut vm,
        link,
        stylesheet_url.clone(),
        crate::style_engine::StyloStylesheetSource::new(
            ".green { color: green }".to_owned(),
            stylesheet_url.clone(),
        )
        .with_sheet_url(stylesheet_url),
    );

    let result = vm
        .eval(
            r#"
(() => {
  const color = element => getComputedStyle(element).color;
  const lightLink = document.getElementById('light-link');
  const lightTarget = document.getElementById('light-target');
  const initialLightSheet = lightLink.sheet;
  const values = [color(lightTarget)];
  lightLink.rel = 'no-stylesheet';
  values.push(color(lightTarget));
  values.push(initialLightSheet.ownerNode === null);
  lightLink.rel = 'stylesheet';
  values.push(lightLink.sheet !== initialLightSheet && lightLink.sheet.ownerNode === lightLink);
  values.push(color(lightTarget));

  const host = document.body.appendChild(document.createElement('div'));
  const shadow = host.attachShadow({mode: 'open'});
  const shadowLink = document.createElement('link');
  shadowLink.rel = 'stylesheet';
  shadowLink.href = '/shared.css';
  const shadowTarget = document.createElement('div');
  shadowTarget.className = 'green';
  shadow.append(shadowLink, shadowTarget);
  values.push(color(shadowTarget));
  shadowLink.rel = 'no-stylesheet';
  values.push(color(shadowTarget));
  shadowLink.rel = 'stylesheet';
  values.push(color(shadowTarget));
  shadowLink.removeAttribute('rel');
  values.push(color(shadowTarget));
  return values.join('|');
})()
"#,
        )
        .expect("cached linked stylesheet rel transitions should evaluate");

    assert_eq!(
        result,
        "rgb(0, 128, 0)|rgb(0, 0, 0)|true|true|rgb(0, 128, 0)|rgb(0, 128, 0)|rgb(0, 0, 0)|rgb(0, 128, 0)|rgb(0, 0, 0)"
    );
}

#[test]
fn cached_import_bearing_link_creates_a_fresh_native_graph_for_each_owner() {
    let mut vm = new_storage_test_vm("https://link-import-cache.test/");
    let stylesheet_url = url::Url::parse("https://link-import-cache.test/shared.css").unwrap();

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  for (const id of ['first-import-link', 'cached-import-link']) {
    const link = document.createElement('link');
    link.id = id;
    link.rel = 'stylesheet';
    link.href = '/shared.css';
    head.appendChild(link);
  }
})()
"#,
    )
    .expect("import-bearing linked stylesheet cache fixture should evaluate");
    let first = cssom_element_handle_by_id(&vm, "first-import-link");
    let cached = cssom_element_handle_by_id(&vm, "cached-import-link");
    install_linked_stylesheet_for_test(
        &mut vm,
        first,
        stylesheet_url.clone(),
        crate::style_engine::StyloStylesheetSource::new(
            "@import './child.css'; .root { color: green; }".to_owned(),
            stylesheet_url.clone(),
        )
        .with_sheet_url(stylesheet_url.clone()),
    );
    assert!(
        vm._context_host
            .borrow_mut()
            .install_cached_linked_stylesheet_for_owner(cached, &stylesheet_url),
        "the second owner should rebind from the URL resource cache"
    );
    vm.apply_pending_stylesheet_source_css_projections();

    let result = vm
        .eval(
            r#"
(() => {
  const first = document.querySelector('#first-import-link').sheet;
  const cached = document.querySelector('#cached-import-link').sheet;
  return [
    first !== null,
    cached !== null,
    first !== cached,
    first.cssRules.length,
    cached.cssRules.length,
    first.cssRules[0] instanceof CSSImportRule,
    cached.cssRules[0] instanceof CSSImportRule,
    first.cssRules[0] !== cached.cssRules[0],
    first.cssRules[0].styleSheet === null,
    cached.cssRules[0].styleSheet === null
  ].join('|');
})()
"#,
        )
        .expect("cached import-bearing stylesheet should retain its native rules");

    assert_eq!(result, "true|true|true|2|2|true|true|true|true|true");
}

#[test]
fn inline_style_attribute_lives_on_prototype() {
    let mut vm = new_storage_test_vm("https://inline-style-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  function prototypeOwns(value, name) {
    for (let proto = Object.getPrototypeOf(value); proto; proto = Object.getPrototypeOf(proto)) {
      if (Object.prototype.hasOwnProperty.call(proto, name)) {
        return true;
      }
    }
    return false;
  }

  const element = document.createElement('div');
  element.setAttribute('style', 'margin-left: 5px;');
  const declaration = element.style;
  declaration.cssText = 'margin-left: 10px; padding-left: 10px;';
  element.style = 'margin-left: 15px;';
  const detachedDocument = new DOMParser().parseFromString('<div></div>', 'text/html');
  const detached = detachedDocument.querySelector('div');
  const detachedStyle = detached.style;
  detachedStyle.cssText = 'color: red;';

  return [
    Object.prototype.hasOwnProperty.call(element, 'style'),
    'style' in element,
    prototypeOwns(element, 'style'),
    declaration === element.style,
    element.style instanceof CSSStyleDeclaration,
    element.style.cssText,
    element.getAttribute('style'),
    Object.prototype.hasOwnProperty.call(detached, 'style'),
    'style' in detached,
    prototypeOwns(detached, 'style'),
    Object.prototype.toString.call(detachedStyle),
    detachedStyle.cssText
  ].join('|');
})()
"#,
        )
        .expect("inline style prototype placement should evaluate");

    assert_eq!(
        result,
        "false|true|true|true|true|margin-left: 15px;|margin-left: 15px;|false|true|true|[object CSSStyleProperties]|color: red;"
    );
}

#[test]
fn element_style_put_forwards_uses_ordinary_get_and_set() {
    let mut vm = new_storage_test_vm("https://element-style-put-forwards.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const descriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "style");
  const element = document.createElement("div");
  const originalStyle = element.style;
  let getterCalls = 0;
  Object.defineProperty(element, "style", {
    configurable: true,
    get() { getterCalls++; return originalStyle; },
    set: descriptor.set
  });
  element.style = "color: green";

  let setterCalls = 0;
  const cssText = Object.getOwnPropertyDescriptor(CSSStyleDeclaration.prototype, "cssText");
  Object.defineProperty(originalStyle, "cssText", {
    configurable: true,
    get: cssText.get,
    set(value) { setterCalls++; cssText.set.call(this, value); }
  });
  element.style = "color: blue";

  const fakeStyle = { cssText: "original" };
  Object.defineProperty(element, "style", {
    configurable: true,
    get() { return fakeStyle; },
    set: descriptor.set
  });
  element.style = "color: red";

  const outcome = callback => {
    try { callback(); return "return"; }
    catch (error) { return error && error.name; }
  };
  Object.defineProperty(element, "style", {
    configurable: true,
    get() { throw new SyntaxError(); },
    set: descriptor.set
  });
  const getterError = outcome(() => { element.style = "x"; });
  Object.defineProperty(element, "style", {
    configurable: true,
    get() { return null; },
    set: descriptor.set
  });
  const nonObject = outcome(() => { element.style = "x"; });

  return [
    getterCalls,
    setterCalls,
    originalStyle.color,
    fakeStyle.cssText,
    getterError,
    nonObject
  ].join("|");
})()
"#,
        )
        .expect("Element.style PutForwards probe should evaluate");

    assert_eq!(result, "2|1|blue|color: red|SyntaxError|TypeError");
}

#[test]
fn css_style_declaration_property_attributes_are_not_own_properties() {
    let mut vm = new_storage_test_vm("https://css-style-property-attributes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('div').style;
  style.color = 'red';
  return [
    style instanceof CSSStyleDeclaration,
    'color' in style,
    style.color,
    Object.prototype.hasOwnProperty.call(style, 'color'),
    Object.getOwnPropertyNames(style).includes('color'),
    Object.keys(style).includes('color')
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleDeclaration property attribute probe should evaluate");

    assert_eq!(result, "true|true|red|false|false|false");
}
#[test]
fn css_namespace_rule_serializes_css_text_with_quoted_url() {
    let mut vm = new_storage_test_vm("https://css-namespace-rule.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule('@namespace svg url(http://servo);', 0);
  sheet.insertRule('@namespace url("http://servo1");', 1);
  const rules = sheet.cssRules;
  return [
    rules[0] instanceof CSSNamespaceRule,
    rules[0].prefix,
    rules[0].namespaceURI,
    rules[0].cssText,
    rules[1].prefix,
    rules[1].namespaceURI,
    rules[1].cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSNamespaceRule serialization should evaluate");

    assert_eq!(
        result,
        r#"true|svg|http://servo|@namespace svg url("http://servo");||http://servo1|@namespace url("http://servo1");"#
    );
}
#[test]
fn css_font_face_rule_style_exposes_font_descriptors() {
    let mut vm = new_storage_test_vm("https://css-font-face-rule-style.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule('@font-face { src: url(http://foo/bar/font.ttf); }', 0);
  sheet.insertRule('@font-face { font-family: STIXGeneral; src: local(STIXGeneral), url(/stixfonts/STIXGeneral.otf); }', 1);
  const first = sheet.cssRules[0];
  const second = sheet.cssRules[1];
  return [
    first instanceof CSSFontFaceRule,
    first.style.src,
    second.style.fontFamily,
    second.style.src,
    second.cssText,
    second.style.cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSFontFaceRule style descriptors should evaluate");

    assert_eq!(
        result,
        r#"true|url("http://foo/bar/font.ttf")|STIXGeneral|local(STIXGeneral), url("/stixfonts/STIXGeneral.otf")|@font-face { font-family: STIXGeneral; src: local(STIXGeneral), url("/stixfonts/STIXGeneral.otf"); }|font-family: STIXGeneral; src: local(STIXGeneral), url("/stixfonts/STIXGeneral.otf");"#
    );
}

#[test]
fn css_font_face_descriptor_setter_uses_cssom_value_fragment_eof() {
    let mut vm = new_storage_test_vm("https://css-font-face-descriptor-fragment-eof.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@font-face { font-family: Foo; src: local(Foo); } .after { color: black; }');
  const rule = sheet.cssRules[0];
  const style = rule.style;

  style.src = 'local(Bar';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  return [
    style.src,
    rule.cssText.includes('src: local(Bar);'),
    !rule.cssText.includes('local(Foo)'),
    sheet.cssRules[0].cssText === rule.cssText,
    sheet.cssRules[1].cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSFontFaceRule descriptor setter should parse CSSOM value fragments at EOF");

    assert_eq!(
        result,
        r#"local(Bar)|true|true|true|.after { color: black; }"#
    );
}

#[test]
fn css_font_face_descriptor_dot_accessors_cover_stylo_descriptors() {
    let mut vm = new_storage_test_vm("https://css-font-face-descriptor-dot-accessors.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const prototype = CSSFontFaceDescriptors.prototype;
  for (const name of [
    'fontFamily',
    'src',
    'fontStyle',
    'fontWeight',
    'fontStretch',
    'fontDisplay',
    'unicodeRange',
    'fontFeatureSettings',
    'fontVariationSettings',
    'fontLanguageOverride',
    'ascentOverride',
    'descentOverride',
    'lineGapOverride',
    'sizeAdjust'
  ]) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    ok(`${name}-prototype-accessor`, !!descriptor && typeof descriptor.get === 'function' && typeof descriptor.set === 'function');
  }

  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`@font-face {
    font-family: Foo;
    src: local(Foo);
    font-display: swap;
    ascent-override: 90%;
    size-adjust: 110%;
    unicode-range: U+20-7E;
  }`);
  const rule = sheet.cssRules[0];
  const style = rule.style;

  eq('source-font-display', style.fontDisplay, 'swap');
  eq('source-ascent-override', style.ascentOverride, '90%');
  eq('source-size-adjust', style.sizeAdjust, '110%');
  eq('source-unicode-range', style.unicodeRange, 'U+20-7E');

  style.fontDisplay = 'fallback';
  style.ascentOverride = '80%';
  style.descentOverride = '25%';
  style.lineGapOverride = 'normal';
  style.sizeAdjust = '120%';
  style.fontFeatureSettings = '"liga" 0';
  style.fontVariationSettings = '"wght" 500';
  style.fontLanguageOverride = '"ENG"';
  style.unicodeRange = 'U+30-39';
  style.fontStretch = 'expanded';
  style.src = 'local(Bar)';

  eq('mutated-font-display', style.getPropertyValue('font-display'), 'fallback');
  eq('mutated-ascent-override', style.getPropertyValue('ascent-override'), '80%');
  eq('mutated-descent-override', style.getPropertyValue('descent-override'), '25%');
  eq('mutated-line-gap-override', style.getPropertyValue('line-gap-override'), 'normal');
  eq('mutated-size-adjust', style.getPropertyValue('size-adjust'), '120%');
  eq('mutated-feature-settings', style.getPropertyValue('font-feature-settings'), '"liga" 0');
  eq('mutated-variation-settings', style.getPropertyValue('font-variation-settings'), '"wght" 500');
  eq('mutated-language-override', style.getPropertyValue('font-language-override'), '"ENG"');
  eq('mutated-unicode-range', style.getPropertyValue('unicode-range'), 'U+30-39');
  eq('mutated-font-stretch', style.getPropertyValue('font-stretch'), 'expanded');
  eq('mutated-src', style.getPropertyValue('src'), 'local(Bar)');

  style.sizeAdjust = '-1%';
  eq('invalid-size-adjust-preserves-old-value', style.sizeAdjust, '120%');
  ok('style-has-font-display-accessor', 'fontDisplay' in style);
  ok('style-does-not-create-own-font-display-data-property', !Object.prototype.hasOwnProperty.call(style, 'fontDisplay'));
  ok('rule-css-text-updated', rule.cssText.includes('font-display: fallback') && rule.cssText.includes('size-adjust: 120%'));

  style.setProperty('font-display', 'optional', 'important');
  eq('priority-font-display-value', style.fontDisplay, 'optional');
  eq('priority-font-display-priority', style.getPropertyPriority('font-display'), 'important');
  ok('priority-rule-css-text-updated', rule.cssText.includes('font-display: optional !important'));
  const removedDisplay = style.removeProperty('font-display');
  eq('removed-font-display-value', removedDisplay, 'optional');
  eq('removed-font-display-current', style.fontDisplay, '');
  eq('removed-font-display-priority', style.getPropertyPriority('font-display'), '');
  ok('removed-rule-css-text-updated', !rule.cssText.includes('font-display'));
  return failures.join('\n');
})()
"#,
        )
        .expect("CSSFontFaceDescriptors dot accessors should evaluate");

    assert_eq!(result, "");
}

#[test]
fn css_page_and_margin_rule_styles_match_chromium_serialization() {
    let mut vm = new_storage_test_vm("https://css-page-rule-style-view.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@page :first { margin-top: 1px; @top-left { content: "x"; color: red; } }');
  const page = sheet.cssRules[0];
  const margin = page.cssRules[0];
  return [
    page instanceof CSSPageRule,
    page.selectorText,
    page.style.cssText,
    page.cssRules.length,
    margin instanceof CSSMarginRule,
    margin.name,
    margin.style.cssText,
    page.cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSPageRule and CSSMarginRule style views should evaluate");

    assert_eq!(
        result,
        "true|:first|margin-top: 1px;|1|true|top-left|content: \"x\"; color: red;|@page :first { margin-top: 1px; @top-left { content: \"x\"; color: red; } }"
    );
}

#[test]
fn css_descriptor_rule_style_accessors_validate_receiver_type() {
    let mut vm = new_storage_test_vm("https://css-descriptor-style-receiver.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@font-face { font-family: a; src: local(a); } @page { margin-top: 1px; @top-left { content: "x"; } }');
  const font = sheet.cssRules[0];
  const page = sheet.cssRules[1];
  const margin = page.cssRules[0];
  const probe = callback => {
    try {
      callback();
      return 'ok';
    } catch (error) {
      return error && error.name;
    }
  };
  const fontStyle = Object.getOwnPropertyDescriptor(CSSFontFaceRule.prototype, 'style');
  const pageStyle = Object.getOwnPropertyDescriptor(CSSPageRule.prototype, 'style');
  const marginStyle = Object.getOwnPropertyDescriptor(CSSMarginRule.prototype, 'style');
  return [
    probe(() => fontStyle.get.call(font)),
    probe(() => fontStyle.get.call(page)),
    probe(() => pageStyle.get.call(page)),
    probe(() => pageStyle.get.call(font)),
    probe(() => marginStyle.get.call(margin)),
    probe(() => marginStyle.get.call(page)),
    font.style.fontFamily,
    page.style.marginTop,
    margin.style.getPropertyValue('content'),
  ].join('|');
})()
"#,
        )
        .expect("descriptor rule style accessors should validate receiver type");

    assert_eq!(
        result,
        r#"ok|TypeError|ok|TypeError|ok|TypeError|a|1px|"x""#
    );
}

#[test]
fn css_font_feature_values_rule_exposes_feature_maps() {
    let mut vm = new_storage_test_vm("https://css-font-feature-values-rule.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule(`@font-feature-values test_family {
    @annotation { the_first: 6; }
    @styleset {
      yo: 7;
      di: 10 9 4 5;
    }
  }`, 0);
  const styleIndex = sheet.insertRule('.after { font-variant-alternates: annotation(the_first); }', 1);
  sheet.deleteRule(styleIndex);
  const rule = sheet.cssRules[0];
  rule.fontFamily = 'changed_family';
  rule.styleset.set('di', 43);
  rule.annotation.set('the_first', [1, 2]);
  return [
    sheet.cssRules.length,
    rule instanceof CSSFontFeatureValuesRule,
    rule.type,
    rule.fontFamily,
    rule.annotation.size,
    rule.styleset.size,
    rule.styleset.get('yo').join(','),
    rule.styleset.get('di').join(','),
    rule.annotation.get('the_first').join(','),
    rule.swash.size
  ].join('|');
})()
"#,
        )
        .expect("CSSFontFeatureValuesRule feature maps should evaluate");

    assert_eq!(result, "1|true|14|changed_family|1|2|7|43|1,2|0");
}

#[test]
fn css_font_feature_values_map_uses_declared_maplike_surface_and_intrinsics() {
    let mut vm = new_storage_test_vm("https://css-font-feature-values-map-intrinsics.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule(`@font-feature-values test_family {
    @annotation { first: 6; }
  }`, 0);
  const rule = sheet.cssRules[0];
  const getPrototypeOf = Reflect.getPrototypeOf;
  const MapConstructor = Map;
  const mapIteratorPrototype = getPrototypeOf(new MapConstructor().entries());
  const originalObjectGetPrototypeOf = Object.getPrototypeOf;
  const originals = [
    ["entries", MapConstructor.prototype.entries],
    ["forEach", MapConstructor.prototype.forEach],
    ["keys", MapConstructor.prototype.keys],
    ["values", MapConstructor.prototype.values]
  ];
  const poisoned = function poisonedMapBuiltin() {
    throw new Error("public Map builtin was observed");
  };
  for (const [name] of originals) {
    MapConstructor.prototype[name] = poisoned;
  }
  Object.getPrototypeOf = function poisonedGetPrototypeOf() {
    throw new Error("public Object.getPrototypeOf was observed");
  };
  globalThis.Map = undefined;

  const failures = [];
  try {
    const map = rule.annotation;
    if (!(map instanceof CSSFontFeatureValuesMap) || map instanceof MapConstructor) {
      failures.push("brand");
    }
    try {
      new CSSFontFeatureValuesMap();
      failures.push("constructor");
    } catch (error) {
      if (!(error instanceof TypeError)) failures.push("constructor-error");
    }
    if (map.set("second", 7) !== undefined || map.set(3, [8]) !== undefined) {
      failures.push("set-return");
    }
    if (
      map.size !== 3 ||
      map.get("first").join(",") !== "6" ||
      map.get("second").join(",") !== "7" ||
      map.get("3").join(",") !== "8" ||
      !map.has("second")
    ) {
      failures.push("maplike");
    }
    const iterator = map.entries();
    const prototype = getPrototypeOf(iterator);
    const next = Object.getOwnPropertyDescriptor(prototype, "next");
    const tag = Object.getOwnPropertyDescriptor(prototype, Symbol.toStringTag);
    if (
      getPrototypeOf(prototype) !== mapIteratorPrototype ||
      iterator[Symbol.iterator]() !== iterator ||
      Object.hasOwn(iterator, "next") ||
      Object.hasOwn(iterator, Symbol.iterator) ||
      Object.hasOwn(prototype, "constructor") ||
      next?.enumerable !== true ||
      next?.writable !== true ||
      next?.configurable !== true ||
      tag?.value !== "CSSFontFeatureValuesMap Iterator" ||
      tag?.enumerable !== false ||
      tag?.writable !== false ||
      tag?.configurable !== true
    ) {
      failures.push("iterator-shape");
    }
    const first = iterator.next();
    map.set("late", 10);
    const remaining = [...iterator].map(([key, value]) => [key, value.join(",")]);
    if (
      first.value[0] !== "first" ||
      first.value[1].join(",") !== "6" ||
      remaining.at(-1)?.[0] !== "3" ||
      remaining.some(([key]) => key === "late")
    ) {
      failures.push("snapshot-iterator");
    }
    const seen = [];
    map.forEach((value, key, owner) => {
      seen.push([key, value.join(","), owner === map]);
    });
    if (seen.length !== 4 || seen.at(-1)?.join("|") !== "late|10|true") {
      failures.push("forEach");
    }
    if (
      CSSFontFeatureValuesMap.prototype[Symbol.iterator] !==
      CSSFontFeatureValuesMap.prototype.entries
    ) {
      failures.push("alias");
    }
  } finally {
    for (const [name, original] of originals) {
      MapConstructor.prototype[name] = original;
    }
    Object.getPrototypeOf = originalObjectGetPrototypeOf;
    globalThis.Map = MapConstructor;
  }
  return failures.join(",") || "ok";
})()
"#,
        )
        .expect("CSSFontFeatureValuesMap intrinsic maplike probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn css_font_feature_values_font_family_mutation_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-font-feature-values-family-live-mutation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`@font-feature-values old_family {
    @annotation { the_first: 6; }
    @styleset { yo: 7; }
  } .after { color: black; }`);
  const rule = sheet.cssRules[0];
  const annotation = rule.annotation;
  const styleset = rule.styleset;

  rule.cssText = `@font-feature-values tree_family {
    @annotation { tree_mark: 11; }
    @styleset { tree_set: 12 13; }
  }`;
  rule.fontFamily = 'changed_family';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === rule,
    rule.annotation === annotation,
    rule.styleset === styleset,
    rule.fontFamily,
    rule.annotation.has('the_first'),
    rule.annotation.get('tree_mark').join(','),
    rule.styleset.has('yo'),
    rule.styleset.get('tree_set').join(','),
    rule.cssText.includes('@font-feature-values changed_family'),
    rule.cssText.includes('tree_mark: 11'),
    rule.cssText.includes('tree_set: 12 13'),
    !rule.cssText.includes('the_first'),
    !rule.cssText.includes('yo: 7'),
    sheet.cssRules[0].cssText === rule.cssText,
    sheet.cssRules[1].cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSFontFeatureValuesRule fontFamily mutation should preserve Stylo rule tree");

    assert_eq!(
        result,
        "2|true|true|true|changed_family|false|11|false|12,13|true|true|true|true|true|true|.after { color: black; }"
    );
}

#[test]
fn css_font_feature_values_map_mutation_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-font-feature-values-map-live-mutation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`@font-feature-values test_family {
    @annotation { the_first: 6; }
    @styleset { yo: 7; }
  } .after { color: black; }`);
  const rule = sheet.cssRules[0];
  const annotation = rule.annotation;
  const styleset = rule.styleset;

  styleset.set('yo', [8, 9]);
  annotation.set('new_mark', 3);
  const beforeInvalidMapMutation = rule.cssText;
  annotation.set('bad_multi', [1, 2]);
  styleset.set('empty', []);
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === rule,
    rule.annotation === annotation,
    rule.styleset === styleset,
    rule.annotation.get('the_first').join(','),
    rule.annotation.get('new_mark').join(','),
    rule.styleset.get('yo').join(','),
    rule.cssText.includes('the_first: 6'),
    rule.cssText.includes('new_mark: 3'),
    rule.cssText.includes('yo: 8 9'),
    rule.annotation.get('bad_multi').join(','),
    rule.styleset.get('empty').join(','),
    rule.cssText === beforeInvalidMapMutation,
    !rule.cssText.includes('bad_multi'),
    !rule.cssText.includes('empty'),
    sheet.cssRules[0].cssText === rule.cssText,
    sheet.cssRules[1].cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSFontFeatureValuesRule map mutation should preserve Stylo rule tree");

    assert_eq!(
        result,
        "2|true|true|true|6|3|8,9|true|true|true|1,2||true|true|true|true|.after { color: black; }"
    );
}

#[test]
fn css_font_feature_values_native_delete_clear_and_family_serialization() {
    let mut vm = new_storage_test_vm("https://css-font-feature-values-native-delete-clear.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`@font-feature-values old_family {
    @annotation { first: 1; }
    @styleset { old_set: 2 3; }
  }`);
  const rule = sheet.cssRules[0];
  const annotation = rule.annotation;
  const styleset = rule.styleset;

  rule.fontFamily = 'serif, foo bar, changed_family,,';
  annotation.set('late', 4);
  const deleted = annotation.delete('first');
  const deletedMissing = annotation.delete('missing');
  styleset.set('replacement', [8, 9]);
  const clearResult = styleset.clear();

  return [
    sheet.cssRules[0] === rule,
    rule.annotation === annotation,
    rule.styleset === styleset,
    rule.fontFamily === '\"serif\", \"foo bar\", changed_family',
    deleted,
    deletedMissing,
    clearResult === undefined,
    annotation.size,
    annotation.get('late').join(','),
    styleset.size,
    rule.cssText.includes('@font-feature-values \"serif\", \"foo bar\", changed_family'),
    rule.cssText.includes('late: 4'),
    !rule.cssText.includes('first: 1'),
    !rule.cssText.includes('@styleset'),
    sheet.cssRules[0].cssText === rule.cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSFontFeatureValuesRule native delete and clear should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|true|false|true|1|4|0|true|true|true|true|true"
    );
}

#[test]
fn css_font_feature_values_family_setter_matches_chromium_raw_string_contract() {
    let mut vm = new_storage_test_vm("https://css-font-feature-values-family-contract.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@font-feature-values seed { }');
  const rule = sheet.cssRules[0];
  const cases = [
    ['serif, foo bar, changed_family,,', '"serif", "foo bar", changed_family'],
    ['SERIF, System-UI, math, default, initial, revert-layer', 'SERIF, System-UI, "math", "default", "initial", "revert-layer"'],
    ['foo\\ bar, --custom, -valid, 1bad, _ok, 日本語', '"foo\\\\ bar", "--custom", -valid, "1bad", _ok, 日本語'],
    ['"foo,bar", baz', '"\\"foo", "bar\\"", baz'],
    ['"serif"', '"\\"serif\\""'],
    ['foo/*x*/bar', '"foo/*x*/bar"'],
    ['--, -, café, \\66 oo', '"--", "-", café, "\\\\66 oo"'],
  ];
  return cases.map(([input, expected]) => {
    rule.fontFamily = input;
    const prelude = `@font-feature-values ${expected} {`;
    return rule.fontFamily === expected && rule.cssText.startsWith(prelude)
      ? 'ok'
      : `${input} => ${rule.fontFamily} | ${rule.cssText}`;
  }).join('\n');
})()
"#,
        )
        .expect("CSSFontFeatureValuesRule raw family contract should evaluate");

    assert_eq!(result, "ok\nok\nok\nok\nok\nok\nok");
}

#[test]
fn detached_css_font_feature_values_maps_mutate_the_retained_snapshot() {
    let mut vm = new_storage_test_vm("https://detached-css-font-feature-values-map.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`@font-feature-values detached_family {
    @annotation { first: 1; }
    @styleset { old_set: 2 3; }
  }`);
  const rule = sheet.cssRules[0];
  const annotation = rule.annotation;
  const styleset = rule.styleset;
  sheet.deleteRule(0);

  const deleted = annotation.delete('first');
  styleset.clear();
  annotation.set('late', 7);

  return [
    sheet.cssRules.length,
    rule.annotation === annotation,
    rule.styleset === styleset,
    deleted,
    annotation.size,
    annotation.get('late').join(','),
    styleset.size,
    rule.cssText.includes('late: 7'),
    !rule.cssText.includes('first: 1'),
    !rule.cssText.includes('@styleset'),
  ].join('|');
})()
"#,
        )
        .expect("detached CSSFontFeatureValuesRule maps should mutate their retained snapshot");

    assert_eq!(result, "0|true|true|true|1|7|0|true|true|true");
}

#[test]
fn css_font_feature_values_rule_css_text_reset_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-font-feature-values-css-text-live-reset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`@font-feature-values old_family {
    @annotation { the_first: 6; }
    @styleset { yo: 7; }
  } .after { color: black; }`);
  const rule = sheet.cssRules[0];
  const annotation = rule.annotation;
  const styleset = rule.styleset;

  rule.cssText = `@font-feature-values newer {
    @annotation { second: 2; }
    @character-variant { cv: 5 6; }
    @styleset { replacement: 3 4; }
  }`;
  const familyAfterReset = rule.fontFamily;
  const characterVariant = rule.characterVariant;
  characterVariant.set('cv', [7, 8]);
  const beforeInvalidReset = rule.cssText;
  rule.cssText = `@font-feature-values serif {
    @annotation { invalid: 1; }
  }`;
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === rule,
    rule.annotation === annotation,
    rule.styleset === styleset,
    familyAfterReset,
    rule.fontFamily,
    annotation.size,
    annotation.has('the_first'),
    annotation.get('second').join(','),
    rule.characterVariant === characterVariant,
    characterVariant.get('cv').join(','),
    styleset.size,
    styleset.has('yo'),
    styleset.get('replacement').join(','),
    rule.cssText === beforeInvalidReset,
    rule.cssText.includes('@font-feature-values newer'),
    sheet.cssRules[0].cssText === rule.cssText,
    sheet.cssRules[1].cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSFontFeatureValuesRule cssText reset should preserve Stylo rule tree");

    assert_eq!(
        result,
        "2|true|true|true|newer|newer|1|false|2|true|7,8|1|false|3,4|true|true|true|.after { color: black; }"
    );
}

#[test]
fn css_descriptor_at_rules_serialize_css_text_without_newlines() {
    let mut vm = new_storage_test_vm("https://css-at-rule-newline-serialization.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule(`@font-face {
    src: local("foo");
    font-family: foo;
    font-weight: bold;
  }`, 0);
  sheet.insertRule(`@counter-style foo {
    system: cyclic;
    symbols: "*";
    suffix: " ";
  }`, 1);
  return [
    sheet.cssRules[0] instanceof CSSFontFaceRule,
    sheet.cssRules[0].cssText.includes('\n'),
    sheet.cssRules[0].cssText,
    sheet.cssRules[1] instanceof CSSCounterStyleRule,
    sheet.cssRules[1].cssText.includes('\n'),
    sheet.cssRules[1].cssText
  ].join('|');
})()
"#,
        )
        .expect("descriptor at-rule cssText newline serialization should evaluate");

    assert_eq!(
        result,
        r#"true|false|@font-face { font-family: foo; src: local("foo"); font-weight: bold; }|true|false|@counter-style foo { system: cyclic; suffix: " "; symbols: "*"; }"#
    );
}

#[test]
fn css_font_face_survives_stylo_stylesheet_mutations() {
    let mut vm = new_storage_test_vm("https://css-font-face-stylo-mutation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  const fontIndex = sheet.insertRule('@font-face { font-family: Foo; src: local(Foo); }', 0);
  const styleIndex = sheet.insertRule('.after { margin: 0; }', 1);
  sheet.deleteRule(styleIndex);
  const font = sheet.cssRules[fontIndex];
  return [
    sheet.cssRules.length,
    font instanceof CSSFontFaceRule,
    font.style.fontFamily,
    font.style.src,
    font.cssText
  ].join('|');
})()
"#,
        )
        .expect("font-face rule should survive Stylo stylesheet mutations");

    assert_eq!(
        result,
        r#"1|true|Foo|local(Foo)|@font-face { font-family: Foo; src: local(Foo); }"#
    );
}

#[test]
fn css_font_face_style_mutation_preserves_stylo_stylesheet_mutations() {
    let mut vm = new_storage_test_vm("https://css-font-face-style-live-mutation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@font-face { font-family: Foo; src: local(Foo); } .after { color: black; }');
  const font = sheet.cssRules[0];
  const style = font.style;

  font.style.cssText = 'font-family: Bar; src: local(Bar); font-style: italic;';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === font,
    font.style === style,
    font.style.fontFamily,
    font.style.src,
    font.style.fontStyle,
    font.cssText,
    sheet.cssRules[0].cssText === font.cssText
  ].join('|');
})()
"#,
        )
        .expect("font-face style mutation should preserve Stylo stylesheet mutation path");

    assert_eq!(
        result,
        r#"2|true|true|Bar|local(Bar)|italic|@font-face { font-family: Bar; src: local(Bar); font-style: italic; }|true"#
    );
}

#[test]
fn css_font_face_style_priority_mutation_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-font-face-style-priority-mutation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@font-face { font-family: Foo; src: local(Foo); } .after { color: black; }');
  const font = sheet.cssRules[0];
  const style = font.style;

  style.setProperty('font-family', 'Bar', 'important');
  style.setProperty('src', 'local(Bar)', 'important');
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === font,
    font.style === style,
    style.getPropertyValue('font-family'),
    style.getPropertyPriority('font-family'),
    style.getPropertyValue('src'),
    style.getPropertyPriority('src'),
    font.cssText,
    sheet.cssRules[0].cssText === font.cssText,
    sheet.cssRules[1].cssText
  ].join('|');
})()
"#,
        )
        .expect("font-face priority mutation should preserve Stylo rule tree");

    assert_eq!(
        result,
        r#"2|true|true|Bar|important|local(Bar)|important|@font-face { font-family: Bar !important; src: local(Bar) !important; }|true|.after { color: black; }"#
    );
}

#[test]
fn css_font_face_rule_css_text_reset_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-font-face-css-text-live-reset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@font-face { font-family: Foo; src: local(Foo); } .after { color: black; }');
  const font = sheet.cssRules[0];
  const style = font.style;

  font.cssText = '@font-face { font-family: Bar; src: local(Bar); font-style: italic; }';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === font,
    font.style === style,
    font.style.fontFamily,
    font.style.src,
    font.style.fontStyle,
    font.cssText.includes('font-family: Bar'),
    sheet.cssRules[0].cssText === font.cssText,
    sheet.cssRules[1].cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSFontFaceRule cssText reset should preserve Stylo rule tree");

    assert_eq!(
        result,
        "2|true|true|Bar|local(Bar)|italic|true|true|.after { color: black; }"
    );
}

#[test]
fn css_font_face_invalid_style_mutation_stays_stylo_canonical() {
    let mut vm = new_storage_test_vm("https://css-font-face-invalid-style-mutation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@font-face { font-family: Foo; src: local(Foo); font-weight: 400; } .after { color: black; }');
  const font = sheet.cssRules[0];
  const style = font.style;
  const before = font.cssText;

  style.fontWeight = 'definitely-invalid';
  style.setProperty('src', 'url("a.woff2"); font-family: Injected');
  style.cssText = 'font-family: Bar; src: local(Bar); font-weight: bad-weight;';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === font,
    font.style === style,
    font.style.fontFamily,
    font.style.src,
    font.style.fontWeight,
    font.cssText.includes('font-family: Bar'),
    font.cssText.includes('src: local(Bar)'),
    !font.cssText.includes('bad-weight'),
    !font.cssText.includes('Injected'),
    sheet.cssRules[0].cssText === font.cssText,
    sheet.cssRules[1].cssText
  ].join('|');
})()
"#,
        )
        .expect("invalid CSSFontFaceRule style mutation should stay Stylo-canonical");

    assert_eq!(
        result,
        "2|true|true|Bar|local(Bar)||true|true|true|true|true|.after { color: black; }"
    );
}

#[test]
fn css_page_and_margin_style_mutation_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-page-margin-style-live-mutation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@page :first { margin-top: 1px; @top-left { content: "x"; } } .after { color: black; }');
  const page = sheet.cssRules[0];
  const pageStyle = page.style;
  const marginRules = page.cssRules;
  const margin = marginRules[0];
  const marginStyle = margin.style;

  page.style.cssText = 'margin-top: 10px;';
  page.style.margin = '1px 2px 3px 4px';
  page.style.setProperty('margin-left', '5px');
  page.style.marginTop = '1px; margin-bottom: 2px';
  page.style.setProperty('margin-right', '1px !important');
  page.style.size = 'jis-b5 landscape';
  page.style.pageOrientation = 'rotate-left';
  page.style.size = 'notarealsize';
  page.style.setProperty('page-orientation', 'rotate-right !important');
  margin.style.cssText = 'content: "y"; color: red;';
  page.insertRule('@bottom-right { content: "z"; }', 1);
  page.deleteRule(1);
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === page,
    page.style === pageStyle,
    page.cssRules === marginRules,
    marginRules[0] === margin,
    margin.style === marginStyle,
    page.style.marginTop,
    page.style.marginRight,
    page.style.marginBottom,
    page.style.marginLeft,
    page.style.size,
    page.style.pageOrientation,
    page.cssText.includes('5px'),
    !page.cssText.includes('!important'),
    margin.style.getPropertyValue('content'),
    margin.style.color,
    page.cssRules.length,
    page.cssText.includes('margin'),
    page.cssText.includes('content: "y"; color: red;'),
    sheet.cssRules[0].cssText === page.cssText
  ].join('|');
})()
"#,
        )
        .expect("page and margin style mutations should preserve Stylo rule tree");

    assert_eq!(
        result,
        r#"2|true|true|true|true|true|1px|2px|3px|5px|jis-b5 landscape|rotate-left|true|true|"y"|red|1|true|true|true"#
    );
}

#[test]
fn page_descriptors_do_not_leak_into_element_style_queries() {
    let mut vm = new_storage_test_vm("https://css-page-descriptor-query-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const element = document.createElement('div');
  (document.body || document.documentElement || document).appendChild(element);
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@page { size: portrait; }');
  return [
    getComputedStyle(element).getPropertyValue('size'),
    element.style.getPropertyValue('size'),
    CSS.supports('size', 'portrait'),
    sheet.cssRules[0].style.getPropertyValue('size')
  ].join('|');
})()
"#,
        )
        .expect("page descriptors should stay scoped to CSSPageDescriptors");

    assert_eq!(result, "||false|portrait");
}

#[test]
fn css_page_descriptor_setters_use_stylo_descriptor_entries() {
    let mut vm = new_storage_test_vm("https://css-page-descriptor-entry-setter.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@page { margin-top: 1px; size: portrait; }');
  const page = sheet.cssRules[0];
  const style = page.style;

  style.setProperty('margin', '2px 4px', 'important');
  const marginPart = [
    style.length,
    style.item(0),
    style.item(1),
    style.item(2),
    style.item(3),
    style.item(4),
    style.margin,
    style.marginTop,
    style.marginRight,
    style.marginBottom,
    style.marginLeft,
    style.getPropertyPriority('margin'),
    style.getPropertyPriority('margin-left'),
    style.cssText.includes('margin: 2px 4px !important'),
    page.cssText.includes('margin: 2px 4px !important')
  ].join(',');

  style.size = 'landscape';
  style.pageOrientation = 'rotate-left';
  style.size = 'definitely-invalid';
  style.setProperty('page-orientation', 'rotate-right !important');
  style.marks = 'crop';
  const descriptorPart = [
    style.size,
    style.pageOrientation,
    style.marks,
    style.cssText.includes('size: landscape'),
    style.cssText.includes('page-orientation: rotate-left'),
    page.cssText.includes('size: landscape'),
    page.cssText.includes('page-orientation: rotate-left')
  ].join(',');

  const removed = style.removeProperty('margin');
  const removePart = [
    removed,
    style.length,
    style.margin,
    style.marginTop,
    style.marginRight,
    style.marginBottom,
    style.marginLeft,
    style.getPropertyPriority('margin'),
    style.cssText.includes('margin-')
  ].join(',');

  return [marginPart, descriptorPart, removePart].join('|');
})()
"#,
        )
        .expect("CSSPageDescriptors setters should use Stylo descriptor entries");

    assert_eq!(
        result,
        "5,size,margin-top,margin-right,margin-bottom,margin-left,2px 4px,2px,4px,2px,4px,important,important,true,true|landscape,rotate-left,,true,true,true,true|2px 4px,2,,,,,,,false"
    );
}

#[test]
fn css_page_and_margin_invalid_style_mutation_stays_stylo_canonical() {
    let mut vm = new_storage_test_vm("https://css-page-margin-invalid-style-mutation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@page :first { margin-top: 1px; @bottom-right { content: "x"; color: red; } } .after { color: black; }');
  const page = sheet.cssRules[0];
  const pageStyle = page.style;
  const margin = page.cssRules[0];
  const marginStyle = margin.style;
  const beforePage = page.cssText;
  const beforeMargin = margin.cssText;

  pageStyle.marginTop = '1px; margin-bottom: 2px';
  pageStyle.setProperty('size', 'notarealsize');
  pageStyle.cssText = 'margin-top: 10px; size: definitely-invalid;';
  marginStyle.setProperty('content', '"y"; color: blue');
  marginStyle.cssText = 'content: "y"; color: notacolor;';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === page,
    page.style === pageStyle,
    page.cssRules[0] === margin,
    margin.style === marginStyle,
    page.style.marginTop,
    margin.name,
    margin.style.getPropertyValue('content'),
    margin.style.color,
    page.cssText.includes('@bottom-right'),
    margin.cssText.startsWith('@bottom-right'),
    !page.cssText.includes('@top-left'),
    page.cssText.includes('margin-top: 10px'),
    !page.cssText.includes('definitely-invalid'),
    !page.cssText.includes('notarealsize'),
    !page.cssText.includes('color: notacolor'),
    !page.cssText.includes('color: blue'),
    !page.cssText.includes('margin-bottom: 2px'),
    page.cssText !== beforePage,
    margin.cssText !== beforeMargin,
    sheet.cssRules[0].cssText === page.cssText,
    sheet.cssRules[1].cssText
  ].join('|');
})()
"#,
        )
        .expect("invalid CSSPageRule/CSSMarginRule style mutation should stay Stylo-canonical");

    assert_eq!(
        result,
        r#"2|true|true|true|true|10px|bottom-right|"y"||true|true|true|true|true|true|true|true|true|true|true|true|.after { color: black; }"#
    );
}

#[test]
fn css_page_rule_selector_text_mutation_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-page-selector-live-mutation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@page :first { margin-top: 1px; @top-left { content: "x"; } } .after { color: black; }');
  const page = sheet.cssRules[0];
  const pageStyle = page.style;
  const marginRules = page.cssRules;
  const margin = marginRules[0];
  const marginStyle = margin.style;

  page.selectorText = ':first, named:left';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === page,
    page.style === pageStyle,
    page.cssRules === marginRules,
    marginRules[0] === margin,
    margin.style === marginStyle,
    page.selectorText,
    page.style.marginTop,
    margin.style.getPropertyValue('content'),
    page.cssText.includes('@page :first, named:left'),
    page.cssText.includes('@top-left'),
    sheet.cssRules[0].cssText === page.cssText,
    sheet.cssRules[1].cssText
  ].join('|');
})()
"#,
        )
        .expect("page selectorText mutation should preserve Stylo rule tree");

    assert_eq!(
        result,
        r#"2|true|true|true|true|true|:first, named:left|1px|"x"|true|true|true|.after { color: black; }"#
    );
}

#[test]
fn css_page_rule_public_mutations_use_attached_native_read_before_child_materialization() {
    let mut vm = new_storage_test_vm("https://css-page-public-mutation-stylo-source.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@page :first { margin-top: 1px; @top-left { content: "x"; color: red; } } .after { color: black; }');
  const page = sheet.cssRules[0];
  const pageStyle = page.style;

  pageStyle.marginTop = '10px';
  page.selectorText = ':left';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  const rules = page.cssRules;
  const margin = rules[0];
  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === page,
    page.style === pageStyle,
    page.selectorText,
    pageStyle.marginTop,
    rules.length,
    margin instanceof CSSMarginRule,
    margin.name,
    margin.style.getPropertyValue('content'),
    margin.style.color,
    page.cssText.includes('@top-left'),
    sheet.cssRules[0].cssText === page.cssText,
    sheet.cssRules[1].cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSPageRule public mutations should use Stylo source before child materialization");

    assert_eq!(
        result,
        r#"2|true|true|:left|10px|1|true|top-left|"x"|red|true|true|.after { color: black; }"#
    );
}

#[test]
fn detached_css_page_selector_mutation_preserves_unmaterialized_margin_rules() {
    let mut vm =
        new_storage_test_vm("https://detached-css-page-selector-unmaterialized-margin.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@page :first { margin-top: 1px; @top-left { content: "x"; color: red; } } .after { color: black; }');
  const page = sheet.cssRules[0];

  // Keep the margin CSSRuleList unmaterialized until after the page rule has
  // detached and its selector has changed.
  sheet.deleteRule(0);
  page.selectorText = ':left';

  const marginRules = page.cssRules;
  const margin = marginRules[0];
  return [
    sheet.cssRules.length,
    page.parentRule === null,
    page.parentStyleSheet === null,
    page.selectorText,
    page.style.marginTop,
    marginRules.length,
    margin instanceof CSSMarginRule,
    margin.name,
    margin.style.getPropertyValue('content'),
    margin.style.color,
    page.cssText.includes('@top-left')
  ].join('|');
})()
"#,
        )
        .expect("detached CSSPageRule selector mutation should preserve margin snapshots");

    assert_eq!(
        result,
        r#"1|true|true|:left|1px|1|true|top-left|"x"|red|true"#
    );
}

#[test]
fn css_page_rule_css_text_reset_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-page-css-text-live-reset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@page :first { margin-top: 1px; @top-left { content: "x"; } } .after { color: black; }');
  const page = sheet.cssRules[0];
  const pageStyle = page.style;
  const marginRules = page.cssRules;

  page.cssText = '@page :left { margin-top: 10px; @top-left { content: "y"; color: red; } }';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  const margin = page.cssRules[0];
  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === page,
    page.style === pageStyle,
    page.cssRules === marginRules,
    margin instanceof CSSMarginRule,
    margin.name,
    margin.style.getPropertyValue('content'),
    margin.style.color,
    page.selectorText,
    page.style.marginTop,
    page.cssText.includes('@page :left'),
    sheet.cssRules[0].cssText === page.cssText,
    sheet.cssRules[1].cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSPageRule cssText reset should preserve Stylo rule tree");

    assert_eq!(
        result,
        r#"2|true|true|true|true|top-left|"y"|red|:left|10px|true|true|.after { color: black; }"#
    );
}

#[test]
fn css_page_rule_selector_mutation_uses_synced_wrapper_after_css_text_reset() {
    let mut vm = new_storage_test_vm("https://css-page-wrapper-sync-after-reset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@page :first { margin-top: 1px; @top-left { content: "x"; color: blue; } } .after { color: black; }');
  const page = sheet.cssRules[0];
  const pageStyle = page.style;
  const marginRules = page.cssRules;
  marginRules[0].style;

  page.cssText = '@page :left { margin-top: 10px; @top-left { content: "y"; color: red; } }';
  page.selectorText = ':right';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  const margin = marginRules[0];
  const text = page.cssText;
  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === page,
    page.style === pageStyle,
    page.cssRules === marginRules,
    page.selectorText,
    page.style.marginTop,
    marginRules.length,
    margin.name,
    margin.style.getPropertyValue('content'),
    margin.style.color,
    text.includes('@page :right'),
    !text.includes('margin-top: 1px'),
    !text.includes('"x"'),
    sheet.cssRules[0].cssText === page.cssText,
    sheet.cssRules[1].cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSPageRule selector mutation should use synced wrappers after cssText reset");

    assert_eq!(
        result,
        r#"2|true|true|true|:right|10px|1|top-left|"y"|red|true|true|true|true|.after { color: black; }"#
    );
}

#[test]
fn css_page_rule_invalid_css_text_reset_keeps_live_stylesheet_owner() {
    let mut vm = new_storage_test_vm("https://css-page-invalid-reset-live-owner.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@page :first { margin-top: 1px; @top-left { content: "x"; } } .after { color: black; }');
  const page = sheet.cssRules[0];
  const pageStyle = page.style;
  const marginRules = page.cssRules;
  const margin = marginRules[0];
  const marginStyle = margin.style;
  const before = page.cssText;

  page.cssText = '@page :left { margin-top: 10px; } @page :right { margin-top: 20px; }';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === page,
    page.style === pageStyle,
    page.cssRules === marginRules,
    marginRules[0] === margin,
    margin.style === marginStyle,
    page.style.marginTop,
    margin.style.getPropertyValue('content'),
    page.cssText === before,
    !page.cssText.includes(':left'),
    sheet.cssRules[0].cssText === page.cssText,
    sheet.cssRules[1].cssText
  ].join('|');
})()
"#,
        )
        .expect("invalid CSSPageRule cssText reset should keep live Stylo owner");

    assert_eq!(
        result,
        r#"2|true|true|true|true|true|1px|"x"|true|true|true|.after { color: black; }"#
    );
}

#[test]
fn css_page_rule_lazy_fields_use_attached_native_rule() {
    let mut vm = new_storage_test_vm("https://css-page-lazy-views-stylo-view.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@page :first { margin-top: 1px; @top-left { content: "x"; } } .after { color: black; }');
  const page = sheet.cssRules[0];

  page.cssText = '@page :left { margin-top: 10px; @top-left { content: "y"; color: red; } }';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  const rules = page.cssRules;
  const margin = rules[0];
  const marginStyle = margin.style;
  marginStyle.marginTop = '4px';
  return [
    page.selectorText,
    page.style.cssText,
    rules.length,
    margin.name,
    marginStyle.cssText,
    page.cssText.includes('@top-left'),
    sheet.cssRules[0].cssText === page.cssText,
  ].join('|');
})()
"#,
        )
        .expect("CSSPageRule lazy fields should use the attached native rule");

    assert_eq!(
        result,
        r#":left|margin-top: 10px;|1|top-left|content: "y"; color: red; margin-top: 4px;|true|true"#
    );
}

#[test]
fn css_property_rule_survives_stylo_stylesheet_mutations() {
    let mut vm = new_storage_test_vm("https://css-property-stylo-mutation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  const propertyIndex = sheet.insertRule('@property --accent { syntax: "<color>"; inherits: false; initial-value: red; }', 0);
  const styleIndex = sheet.insertRule('.after { color: var(--accent); }', 1);
  sheet.deleteRule(styleIndex);
  const property = sheet.cssRules[propertyIndex];
  return [
    sheet.cssRules.length,
    property instanceof CSSPropertyRule,
    property.name,
    property.syntax,
    property.inherits,
    property.initialValue,
    property.cssText.includes('\n')
  ].join('|');
})()
"#,
        )
        .expect("property rule should survive Stylo stylesheet mutations");

    assert_eq!(result, "1|true|--accent|<color>|false|red|false");
}

#[test]
fn css_property_rule_css_text_reset_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-property-css-text-live-reset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@property --old { syntax: "<color>"; inherits: false; initial-value: red; } .after { color: black; }');
  const property = sheet.cssRules[0];

  property.cssText = '@property --new { syntax: "*"; inherits: true; }';
  const beforeInvalidReset = property.cssText;
  property.cssText = '@property --bad { syntax: "<color>"; inherits: false; initial-value: 10px; }';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);
  const getterSnapshot = [
    property.name,
    property.syntax,
    property.inherits,
    property.initialValue === null
  ].join(',');

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === property,
    property instanceof CSSPropertyRule,
    property.name,
    property.syntax,
    property.inherits,
    property.initialValue === null,
    getterSnapshot,
    property.cssText.includes('@property --new'),
    property.cssText === beforeInvalidReset,
    sheet.cssRules[0].cssText === property.cssText,
    sheet.cssRules[1].cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSPropertyRule cssText reset should preserve Stylo rule tree");

    assert_eq!(
        result,
        "2|true|true|--new|*|true|true|--new,*,true,true|true|true|true|.after { color: black; }"
    );
}

#[test]
fn css_property_rule_materialization_uses_stylo_validation() {
    let mut vm = new_storage_test_vm("https://css-property-stylo-validation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const invalidProperty = '@property --bad { syntax: "<color>"; inherits: false; initial-value: 10px; }';
  const sheet = new CSSStyleSheet();
  let insertError = 'none';
  try {
    sheet.insertRule(invalidProperty, 0);
  } catch (error) {
    insertError = error.name;
  }

  sheet.replaceSync(`${invalidProperty} .ok { color: green; }`);
  return [
    insertError,
    sheet.cssRules.length,
    sheet.cssRules[0] instanceof CSSStyleRule,
    sheet.cssRules[0].cssText
  ].join('|');
})()
"#,
        )
        .expect("invalid property rules should be validated by Stylo");

    assert_eq!(result, "SyntaxError|1|true|.ok { color: green; }");
}

#[test]
fn css_counter_style_rule_survives_stylo_stylesheet_mutations() {
    let mut vm = new_storage_test_vm("https://css-counter-style-stylo-mutation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  const counterIndex = sheet.insertRule('@counter-style thumbs { system: cyclic; symbols: "*"; suffix: " "; }', 0);
  const styleIndex = sheet.insertRule('.after { list-style: thumbs; }', 1);
  sheet.deleteRule(styleIndex);
  const counter = sheet.cssRules[counterIndex];
  return [
    sheet.cssRules.length,
    counter instanceof CSSCounterStyleRule,
    counter.type,
    counter.cssText.includes('\n'),
    counter.cssText
  ].join('|');
})()
"#,
        )
        .expect("counter-style rule should survive Stylo stylesheet mutations");

    assert_eq!(
        result,
        r#"1|true|11|false|@counter-style thumbs { system: cyclic; suffix: " "; symbols: "*"; }"#
    );
}

#[test]
fn css_counter_style_rule_css_text_reset_preserves_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://css-counter-style-css-text-live-reset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@counter-style thumbs { system: cyclic; symbols: "*"; suffix: " "; } .after { color: black; }');
  const counter = sheet.cssRules[0];

  counter.cssText = '@counter-style dots { system: cyclic; symbols: "."; suffix: " "; }';
  const beforeInvalidReset = counter.cssText;
  counter.cssText = '@counter-style bad { system: cyclic; suffix: " "; }';
  sheet.insertRule('.temp { color: green; }', 2);
  sheet.deleteRule(2);

  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === counter,
    counter instanceof CSSCounterStyleRule,
    counter.name,
    counter.cssText.includes('@counter-style dots'),
    counter.cssText.includes('symbols: "."'),
    counter.cssText === beforeInvalidReset,
    sheet.cssRules[0].cssText === counter.cssText,
    sheet.cssRules[1].cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSCounterStyleRule cssText reset should preserve Stylo rule tree");

    assert_eq!(
        result,
        r#"2|true|true|dots|true|true|true|true|.after { color: black; }"#
    );
}
#[test]
fn css_namespace_rules_obey_stylesheet_ordering_boundaries() {
    let mut vm = new_storage_test_vm("https://css-namespace-rule-ordering.test/");

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
  const inserted = new CSSStyleSheet();
  inserted.insertRule('div { color: green; }', 0);
  const insertResult = probe(() => {
    inserted.insertRule('@namespace myhtml url("http://www.w3.org/1999/xhtml")', 0);
  });
  const namespaceSelectorResult = probe(() => {
    inserted.insertRule('myhtml|div { color: red !important; }', 0);
  });

  const deleted = new CSSStyleSheet();
  deleted.insertRule('@namespace a url();', 0);
  deleted.insertRule('b {}', 1);
  const deleteResult = probe(() => deleted.deleteRule(0));

  return [
    insertResult,
    inserted.cssRules.length,
    inserted.cssRules[0].cssText,
    namespaceSelectorResult,
    inserted.cssRules.length,
    deleteResult,
    deleted.cssRules.length,
    deleted.cssRules[0].cssText
  ].join('|');
})()
"#,
        )
        .expect("CSS namespace rule ordering boundaries should evaluate");

    assert_eq!(
        result,
        "InvalidStateError|1|div { color: green; }|SyntaxError|1|InvalidStateError|2|@namespace a url(\"\");"
    );
}
#[test]
fn cssom_rule_list_keeps_namespaced_style_rules() {
    let mut vm = new_storage_test_vm("https://cssom-rule-list-namespaces.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule('@namespace svg "http://www.w3.org/2000/svg";', 0);
  sheet.insertRule('svg|a { color: white; }', 1);
  sheet.insertRule('@media screen {}', 2);
  sheet.cssRules[2].insertRule('svg|circle { color: blue; }', 0);
  return [
    sheet.cssRules.length,
    sheet.cssRules[0].type,
    sheet.cssRules[1].type,
    sheet.cssRules[1].selectorText,
    sheet.cssRules[1].cssText,
    sheet.cssRules[2].cssRules.length,
    sheet.cssRules[2].cssRules[0].selectorText
  ].join('|');
})()
"#,
        )
        .expect("namespaced CSSOM style rules should evaluate");

    assert_eq!(result, "3|10|1|svg|a|svg|a { color: white; }|1|svg|circle");
}

#[test]
fn cssom_attribute_case_flags_reject_namespace_like_trailing_tokens() {
    let mut vm = new_storage_test_vm("https://cssom-attribute-case-flags.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      callback();
      return "ok";
    } catch (error) {
      return error && error.name;
    }
  };
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const head = document.head || root.appendChild(document.createElement("head"));
  const style = document.createElement("style");
  head.append(style);
  const invalid = [
    "[foo='bar' |i]",
    "[foo='bar' *|i]",
    "[foo='bar' \\*|i]",
  ];
  const textCounts = invalid.map(selector => {
    style.textContent = `${selector} { color: red; }`;
    return style.sheet.cssRules.length;
  });
  const insertResults = invalid.map(selector => {
    const sheet = new CSSStyleSheet();
    return probe(() => sheet.insertRule(`${selector} { color: red; }`, 0));
  });
  const valid = new CSSStyleSheet();
  valid.insertRule("[|foo='bar' i] { color: green; }", 0);
  valid.insertRule("[*|foo='bar' i] { color: blue; }", 1);
  return [
    textCounts.join(","),
    insertResults.join(","),
    valid.cssRules.length,
    valid.cssRules[0].selectorText,
    valid.cssRules[1].selectorText
  ].join("|");
})()
"#,
        )
        .expect("attribute case flag namespace-like selector validation should evaluate");

    assert_eq!(
        result,
        "0,0,0|SyntaxError,SyntaxError,SyntaxError|2|[foo=\"bar\" i]|[*|foo=\"bar\" i]"
    );
}

#[test]
fn cssom_rejects_terminal_pseudo_element_chains() {
    let mut vm = new_storage_test_vm("https://cssom-terminal-pseudo-chain.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      callback();
      return "ok";
    } catch (error) {
      return error && error.name;
    }
  };
  const sheet = new CSSStyleSheet();
  const beforeHighlight = probe(() => {
    sheet.insertRule("::before::highlight(foo) { color: red; }", 0);
  });
  const highlightAfter = probe(() => {
    sheet.insertRule("::highlight(foo)::after { color: red; }", 0);
  });
  sheet.insertRule("::part(label)::highlight(foo) { color: green; }", 0);
  const rule = sheet.cssRules[0];
  rule.selectorText = "::highlight(foo)::after";
  return [
    beforeHighlight,
    highlightAfter,
    sheet.cssRules.length,
    rule.selectorText
  ].join("|");
})()
"#,
        )
        .expect("terminal pseudo-element chain validation should evaluate");

    assert_eq!(
        result,
        "SyntaxError|SyntaxError|1|::part(label)::highlight(foo)"
    );
}

#[test]
fn cssom_stylesheet_text_uses_prior_namespace_rules_for_attribute_case_flags() {
    let mut vm = new_storage_test_vm("https://cssom-attribute-case-namespace.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const head = document.head || root.appendChild(document.createElement("head"));
  const style = document.createElement("style");
  style.textContent = `
    @namespace xml url("http://www.w3.org/XML/1998/namespace");
    [xml|lang='A' i] { color: red; }
  `;
  head.append(style);
  const rules = style.sheet.cssRules;
  return [
    rules.length,
    rules[0].cssText,
    rules[1].selectorText
  ].join("|");
})()
"#,
        )
        .expect("stylesheet namespace context should apply to later attribute selectors");

    assert_eq!(
        result,
        "2|@namespace xml url(\"http://www.w3.org/XML/1998/namespace\");|[xml|lang=\"A\" i]"
    );
}

#[test]
fn cssom_namespaced_selector_text_updates_style_matching() {
    let mut vm = new_storage_test_vm("https://cssom-selector-text-namespace-matching.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = `
    @namespace url("http://www.w3.org/1999/xhtml");
    @namespace svg url("http://www.w3.org/2000/svg");
    svg|*.style0 { background-color: rgb(0, 0, 255) !important; }
    svg|*.style1 { background-color: rgb(255, 0, 255); }
  `;
  head.append(style);
  const target = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  target.setAttribute("class", "style1");
  body.append(target);
  const sheet = style.sheet;
  const rule = sheet.cssRules[2];
  const original = rule.selectorText;
  const color = () => getComputedStyle(target).backgroundColor;
  const probe = selector => {
    rule.selectorText = selector;
    const value = [rule.selectorText, color()].join("=>");
    rule.selectorText = original;
    return value;
  };
  return [
    original,
    color(),
    probe(".style1"),
    probe("svg|*.style1  "),
    probe("*|*.style1  "),
    probe(" *.style1  "),
    probe("p")
  ].join("|");
})()
"#,
        )
        .expect("namespaced selectorText style matching should evaluate");

    assert_eq!(
        result,
        "svg|*.style0|rgb(255, 0, 255)|.style1=>rgb(255, 0, 255)|svg|*.style1=>rgb(0, 0, 255)|*|*.style1=>rgb(0, 0, 255)|.style1=>rgb(255, 0, 255)|p=>rgb(255, 0, 255)"
    );
}

#[test]
fn cssom_namespaced_css_text_setter_uses_parent_stylesheet_context() {
    let mut vm = new_storage_test_vm("https://cssom-csstext-namespace-context.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const target = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  target.setAttribute("class", "target");
  body.append(target);

  const sheet = new CSSStyleSheet();
  sheet.insertRule('@namespace svg "http://www.w3.org/2000/svg";', 0);
  sheet.insertRule('.target { background-color: rgb(255, 0, 0); }', 1);
  document.adoptedStyleSheets = [sheet];

  const rule = sheet.cssRules[1];
  const before = [rule.selectorText, getComputedStyle(target).backgroundColor].join("=>");
  rule.cssText = "svg|*.target { background-color: rgb(0, 128, 0); }";
  const after = [
    rule.selectorText,
    rule.cssText,
    getComputedStyle(target).backgroundColor
  ].join("=>");
  return [before, after].join("|");
})()
"#,
        )
        .expect("namespaced CSSRule.cssText setter should evaluate");

    assert_eq!(
        result,
        ".target=>rgb(255, 0, 0)|svg|*.target=>svg|*.target { background-color: rgb(0, 128, 0); }=>rgb(0, 128, 0)"
    );
}

#[test]
fn css_container_rule_exposes_condition_surface() {
    let mut vm = new_storage_test_vm("https://css-container-rule.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const style = document.createElement('style');
  head.append(style);
  const sheet = style.sheet;
  sheet.insertRule('@container name (min-width: 100px) {}', 0);
  sheet.insertRule('@container (min-width: 100px) {}', 1);
  const named = sheet.cssRules[0];
  const anonymous = sheet.cssRules[1];
  return [
    named instanceof CSSContainerRule,
    named instanceof CSSConditionRule,
    named.containerName,
    named.containerQuery,
    named.conditionText,
    anonymous.containerName,
    anonymous.containerQuery,
    anonymous.conditionText,
    CSSRule.CONTAINER_RULE
  ].join('|');
})()
"#,
        )
        .expect("CSSContainerRule surface should evaluate");

    assert_eq!(
        result,
        "true|true|name|(min-width: 100px)|name (min-width: 100px)||(min-width: 100px)|(min-width: 100px)|17"
    );
}

#[test]
fn css_named_container_rule_without_query_exposes_empty_query_string() {
    let mut vm = new_storage_test_vm("https://css-container-name-only.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@container sidebar {}');
  const rule = sheet.cssRules[0];
  return [
    rule instanceof CSSContainerRule,
    rule.containerName,
    rule.containerQuery,
    typeof rule.containerQuery,
    rule.conditionText
  ].join('|');
})()
"#,
        )
        .expect("name-only CSSContainerRule getters should evaluate");

    assert_eq!(result, "true|sidebar||string|sidebar");
}

#[test]
fn css_layer_rules_expose_cssom_surface() {
    let mut vm = new_storage_test_vm("https://css-layer-rules.test/");

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
  const head = document.head || root.appendChild(document.createElement('head'));
  const style = document.createElement('style');
  style.textContent = `
    @layer foo, bar;
    @import url("data:text/css,") layer(qux);
    @import url("data:text/css,");
    @layer outer { @layer inner {} }
  `;
  head.append(style);
  const statement = style.sheet.cssRules[0];
  const layeredImport = style.sheet.cssRules[1];
  const plainImport = style.sheet.cssRules[2];
  const block = style.sheet.cssRules[3];
  const nested = block.cssRules[0];
  return [
    typeof CSSLayerBlockRule,
    typeof CSSLayerStatementRule,
    block instanceof CSSLayerBlockRule,
    block instanceof CSSGroupingRule,
    !(block instanceof CSSConditionRule),
    statement instanceof CSSLayerStatementRule,
    !(statement instanceof CSSGroupingRule),
    block.name,
    nested.name,
    statement.nameList.join(','),
    Object.isFrozen(statement.nameList),
    layeredImport.layerName,
    plainImport.layerName === null,
    probe(() => CSSLayerBlockRule.prototype.name),
    probe(() => CSSLayerStatementRule.prototype.nameList)
  ].join('|');
})()
"#,
        )
        .expect("CSS layer rule surface should evaluate");

    assert_eq!(
        result,
        "function|function|true|true|true|true|true|outer|inner|foo,bar|true|qux|true|TypeError|TypeError"
    );
}

#[test]
fn css_grouping_rule_insert_rule_rejects_invalid_or_disallowed_rules() {
    let mut vm = new_storage_test_vm("https://css-grouping-insert-rule.test/");

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
  const sheet = new CSSStyleSheet();
  sheet.insertRule('@media all { * { color: red; } }', 0);
  const grouping = sheet.cssRules[0];
  const first = grouping.cssRules[0].cssText;
  const syntax = probe(() => grouping.insertRule('???', 0));
  const importResult = probe(() => grouping.insertRule('@import url("foo.css");', 0));
  const namespaceResult = probe(() => grouping.insertRule('@namespace url("http://www.w3.org/1999/xhtml");', 0));
  const inserted = grouping.insertRule('.foo {}');

  return [
    syntax,
    importResult,
    namespaceResult,
    inserted,
    grouping.cssRules.length,
    grouping.cssRules[1].cssText === first
  ].join('|');
})()
"#,
        )
        .expect("CSSGroupingRule insertRule validation should evaluate");

    assert_eq!(
        result,
        "SyntaxError|HierarchyRequestError|HierarchyRequestError|0|2|true"
    );
}
#[test]
fn css_stylesheet_insert_rule_rejects_trailing_garbage() {
    let mut vm = new_storage_test_vm("https://css-insert-rule-trailing-garbage.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule('p { color: green; }', 0);
  let errorName = 'ok';
  try {
    sheet.insertRule('p { color: red; } garbage', 1);
  } catch (error) {
    errorName = error && error.name;
  }
  return [
    errorName,
    sheet.cssRules.length,
    sheet.cssRules[0].cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleSheet.insertRule trailing garbage rejection should evaluate");

    assert_eq!(result, "SyntaxError|1|p { color: green; }");
}
#[test]
fn live_inline_style_css_text_serializes_named_right_property() {
    let mut vm = new_storage_test_vm("https://inline-style-right-css-text.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('div').style;
  style.left = '10px';
  const afterLeft = style.cssText;
  style.right = '20px';
  return [afterLeft, style.cssText, style.getPropertyValue('right')].join('|');
})()
"#,
        )
        .expect("live inline style right property should serialize");

    assert_eq!(result, "left: 10px;|left: 10px; right: 20px;|20px");
}
#[test]
fn css_style_set_property_undefined_value_is_noop() {
    let mut vm = new_storage_test_vm("https://css-set-property-undefined.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const element = document.createElement('div');
  document.appendChild(element);
  const live = element.style;
  live.color = 'white';
  live.setProperty('color', undefined);

  const detached = new DOMParser()
    .parseFromString('<html><body></body></html>', 'text/html')
    .createElement('div')
    .style;
  detached.color = 'white';
  detached.setProperty('color', undefined);

  live.setProperty('background-color', 'red', undefined);
  return [
    live.color,
    detached.color,
    live.backgroundColor,
    live.getPropertyPriority('background-color')
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleDeclaration.setProperty undefined value should evaluate");

    assert_eq!(result, "white|white|red|");
}
#[test]
fn live_inline_style_css_text_getter_serializes_declaration_block() {
    let mut vm = new_storage_test_vm("https://inline-style-csstext-serialization.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const element = document.createElement('div');
  element.setAttribute(
    'style',
    'background-color: blue !important; color: red ! important; broken'
  );
  return element.style.cssText;
})()
"#,
        )
        .expect("live inline style cssText getter should serialize");

    assert_eq!(
        result,
        "background-color: blue !important; color: red !important;"
    );
}

#[test]
fn live_inline_css_text_setter_uses_stylo_declaration_block_for_plain_properties() {
    let mut vm = new_storage_test_vm("https://inline-style-pdb-csstext.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  target.style.cssText = [
    'display: invalid',
    'display: block',
    'visibility: nope',
    'visibility: hidden',
    'table-layout: nonsense',
    'table-layout: fixed'
  ].join('; ');
  return [
    target.style.display,
    target.style.visibility,
    target.style.tableLayout,
    target.style.cssText
  ].join('|');
})()
"#,
        )
        .expect("live inline cssText setter should use Stylo declaration block");

    assert_eq!(
        result,
        "block|hidden|fixed|display: block; visibility: hidden; table-layout: fixed;"
    );
}

#[test]
fn live_inline_css_text_reset_builds_pdb_storage_for_plain_properties() {
    let mut vm = new_storage_test_vm("https://inline-style-pdb-csstext-storage.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const plain = document.createElement('div');
  plain.style.cssText = 'display: block; visibility: hidden;';
  plain.style.opacity = '0.5';
  const removed = plain.style.removeProperty('display');
  plain.style.setProperty('visibility', 'collapse');
  const plainState = [
    removed,
    plain.style.display,
    plain.style.visibility,
    plain.style.opacity,
    Array.from({ length: plain.style.length }, (_, index) => plain.style.item(index)).join(','),
    plain.style.cssText,
    plain.getAttribute('style')
  ].join('|');

  const mixed = document.createElement('div');
  mixed.style.cssText = '--token: value; display: block;';
  mixed.style.opacity = '0.25';
  const mixedState = [
    mixed.style.getPropertyValue('--token'),
    mixed.style.display,
    mixed.style.opacity,
    mixed.style.cssText
  ].join('|');

  const mixedShorthand = document.createElement('div');
  mixedShorthand.style.cssText = '--before: one; place-content: center start; --after: two;';
  const mixedShorthandState = [
    mixedShorthand.style.getPropertyValue('--before'),
    mixedShorthand.style.getPropertyValue('--after'),
    mixedShorthand.style.getPropertyValue('place-content'),
    mixedShorthand.style.getPropertyValue('align-content'),
    mixedShorthand.style.getPropertyValue('justify-content'),
    mixedShorthand.style.cssText,
    mixedShorthand.getAttribute('style')
  ].join('|');

  return [plainState, mixedState, mixedShorthandState].join('/');
})()
"#,
        )
        .expect("live inline cssText reset should build PDB storage for plain properties");

    assert_eq!(
        result,
        "block||collapse|0.5|opacity,visibility|opacity: 0.5; visibility: collapse;|opacity: 0.5; visibility: collapse;/value|block|0.25|--token: value; display: block; opacity: 0.25;/one|two|center start|center|start|--before: one; place-content: center start; --after: two;|--before: one; place-content: center start; --after: two;"
    );
}

#[test]
fn live_inline_mixed_pdb_indexed_names_use_stored_block() {
    let mut vm = new_storage_test_vm("https://inline-style-pdb-indexed-names.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const duplicate = document.createElement('div');
  duplicate.style.cssText = 'display: block; --token: a; display: flex;';
  const duplicateNames = Array.from(
    { length: duplicate.style.length },
    (_, index) => duplicate.style.item(index)
  ).join(',');

  const interleaved = document.createElement('div');
  interleaved.style.cssText = 'width: 0; --token: a; height: 0;';
  const interleavedNames = Array.from(
    { length: interleaved.style.length },
    (_, index) => interleaved.style.item(index)
  ).join(',');

  return [
    duplicate.style.length,
    duplicateNames,
    duplicate.style.cssText,
    interleaved.style.length,
    interleavedNames,
    interleaved.style.cssText
  ].join('|');
})()
"#,
        )
        .expect("live inline mixed PDB indexed names should use stored block");

    assert_eq!(
        result,
        "2|--token,display|--token: a; display: flex;|3|width,--token,height|width: 0px; --token: a; height: 0px;"
    );
}

#[test]
fn live_inline_style_writes_use_stylo_declaration_block_for_plain_properties() {
    let mut vm = new_storage_test_vm("https://inline-style-pdb-write.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  const style = target.style;
  style.setProperty('display', 'invalid');
  style.setProperty('display', 'block');
  style.setProperty('visibility', 'nope');
  style.setProperty('visibility', 'hidden');
  style.tableLayout = 'nonsense';
  style.tableLayout = 'fixed';
  style.setProperty('display', 'inline; visibility: collapse');
  style.setProperty('visibility', 'visible !important');

  const detached = new DOMParser().parseFromString('<html></html>', 'text/html')
    .createElement('div').style;
  detached.setProperty('display', 'block; visibility: hidden');

  return [
    style.display,
    style.visibility,
    style.tableLayout,
    style.getPropertyPriority('visibility'),
    style.cssText,
    detached.display,
    detached.visibility,
    detached.cssText
  ].join('|');
})()
"#,
        )
        .expect("live inline style writes should use Stylo declaration block");

    assert_eq!(
        result,
        "block|hidden|fixed||display: block; visibility: hidden; table-layout: fixed;|||"
    );
}

#[test]
fn background_image_image_set_uses_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://background-image-image-set-pdb.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const value = 'image-set(url("") calc(1x * NaN))';
  const serialized = 'image-set(url("") calc(NaN * 1dppx))';

  function exercise(style, label, ruleText) {
    const beforeNames = names(style);
    style.setProperty('background-image', value, 'important');
    eq(`${label}-value`, style.getPropertyValue('background-image'), serialized);
    eq(`${label}-priority`, style.getPropertyPriority('background-image'), 'important');
    const afterNames = names(style);
    eq(`${label}-length`, String(style.length), String(beforeNames.length + 1));
    ok(`${label}-names`, afterNames.includes('background-image'));
    ok(`${label}-cssText`, style.cssText.includes(`background-image: ${serialized} !important;`));
    if (ruleText) {
      ok(`${label}-rule-cssText`, ruleText().includes(`background-image: ${serialized} !important;`));
    }

    style.setProperty('--token', 'value');
    const removed = style.removeProperty('background-image');
    eq(`${label}-removed`, removed, serialized);
    eq(`${label}-after-remove`, style.getPropertyValue('background-image'), '');
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
  }

  exercise(document.createElement('div').style, 'inline');

  const detached = new DOMParser().parseFromString('<html><body></body></html>', 'text/html')
    .createElement('div').style;
  exercise(detached, 'detached');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; }');
  const rule = sheet.cssRules[0];
  exercise(rule.style, 'rule', () => rule.cssText);

  return failures.length ? failures.slice(0, 8).join('|') : 'PASS';
})()
"#,
        )
        .expect("background-image image-set PDB backing should evaluate");

    assert_eq!(result, "PASS");
}

#[test]
fn live_inline_css_text_getter_uses_stylo_declaration_block_for_plain_properties() {
    let mut vm = new_storage_test_vm("https://inline-style-pdb-csstext-getter.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  const style = target.style;
    style.setProperty('place-content', 'center start', 'important');
  return [
    style.length,
    Array.from({ length: style.length }, (_, index) => style.item(index)).join(','),
    style.getPropertyValue('place-content'),
    style.getPropertyPriority('place-content'),
    style.getPropertyValue('align-content'),
    style.getPropertyPriority('align-content'),
    style.getPropertyValue('justify-content'),
    style.getPropertyPriority('justify-content'),
    style.cssText
  ].join('|');
})()
"#,
        )
        .expect("live inline cssText getter should use Stylo declaration block");

    assert_eq!(
        result,
        "2|align-content,justify-content|center start|important|center|important|start|important|place-content: center start !important;"
    );
}

#[test]
fn live_inline_shorthand_queries_use_stylo_declaration_block_for_supported_shorthands() {
    let mut vm = new_storage_test_vm("https://inline-style-pdb-shorthand-query.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('div').style;
  style.setProperty('grid-column-start', '1', 'important');
  style.setProperty('grid-column-end', '3', 'important');
  const gridColumn = [
    style.getPropertyValue('grid-column'),
    style.getPropertyPriority('grid-column')
  ].join(',');

  style.cssText = 'margin-inline-start: 1px; margin-inline-end: 2px;';
  const marginInline = [
    style.getPropertyValue('margin-inline'),
    style.cssText
  ].join(',');

  style.cssText = '';
  style.setProperty('flex', '1 2 3px', 'important');
  const flex = [
    style.getPropertyValue('flex'),
    style.getPropertyPriority('flex'),
    style.getPropertyValue('flex-grow'),
    style.getPropertyValue('flex-shrink'),
    style.getPropertyValue('flex-basis'),
    style.cssText
  ].join(',');

  return [gridColumn, marginInline, flex].join('|');
})()
"#,
        )
        .expect("live inline shorthand queries should use Stylo declarations");

    assert_eq!(
        result,
        "1 / 3,important|1px 2px,margin-inline: 1px 2px;|1 2 3px,important,1,2,3px,flex: 1 2 3px !important;"
    );
}

#[test]
fn live_inline_pdb_queries_ignore_unrelated_supplemental_entries() {
    let mut vm = new_storage_test_vm("https://inline-style-pdb-mixed-query.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('div').style;
  style.setProperty('place-content', 'center start', 'important');
  style.setProperty('user-select', 'none');
  style.setProperty('--token', 'value');
  style.setProperty('-webkit-text-fill-color', 'red');

  return [
    style.getPropertyValue('place-content'),
    style.getPropertyPriority('place-content'),
    style.getPropertyValue('align-content'),
    style.getPropertyPriority('justify-content'),
    style.getPropertyValue('user-select'),
    style.getPropertyValue('--token'),
    style.getPropertyValue('-webkit-text-fill-color')
  ].join('|');
})()
"#,
        )
        .expect("live inline PDB queries should ignore unrelated supplemental entries");

    assert_eq!(
        result,
        "center start|important|center|important|none|value|red"
    );
}

#[test]
fn live_inline_pdb_mutations_replace_target_side_entries() {
    let mut vm = new_storage_test_vm("https://inline-style-pdb-target-side-entry.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  const style = target.style;
  style.setProperty('--token', 'value');
  style.setProperty('width', 'var(--w)');
  style.setProperty('height', '1px');
  const before = [
    style.getPropertyValue('width'),
    Array.from({ length: style.length }, (_, index) => style.item(index)).join(','),
    style.cssText
  ].join('|');

  style.setProperty('width', '10px');
  const afterSet = [
    style.width,
    style.getPropertyValue('--token'),
    style.height,
    Array.from({ length: style.length }, (_, index) => style.item(index)).join(','),
    style.cssText,
    target.getAttribute('style')
  ].join('|');

  const removed = style.removeProperty('width');
  const afterRemove = [
    removed,
    style.width,
    style.getPropertyValue('--token'),
    style.height,
    Array.from({ length: style.length }, (_, index) => style.item(index)).join(','),
    style.cssText,
    target.getAttribute('style')
  ].join('|');

  const namedTarget = document.createElement('div');
  const named = namedTarget.style;
  named.width = 'var(--named)';
  named.height = '2px';
  named.width = '12px';
  const namedSet = [
    named.width,
    Array.from({ length: named.length }, (_, index) => named.item(index)).join(','),
    named.cssText,
    namedTarget.getAttribute('style')
  ].join('|');
  named.width = '';
  const namedRemove = [
    named.width,
    Array.from({ length: named.length }, (_, index) => named.item(index)).join(','),
    named.cssText,
    namedTarget.getAttribute('style')
  ].join('|');

  return [before, afterSet, afterRemove, namedSet, namedRemove].join('/');
})()
"#,
        )
        .expect("live inline PDB mutations should replace target side entries");

    assert_eq!(
        result,
        "var(--w)|--token,width,height|--token: value; width: var(--w); height: 1px;/10px|value|1px|--token,height,width|--token: value; height: 1px; width: 10px;|--token: value; height: 1px; width: 10px;/10px||value|1px|--token,height|--token: value; height: 1px;|--token: value; height: 1px;/12px|height,width|height: 2px; width: 12px;|height: 2px; width: 12px;/|height|height: 2px;|height: 2px;"
    );
}

#[test]
fn live_inline_pdb_mutations_replace_fully_covered_side_entries() {
    let mut vm = new_storage_test_vm("https://inline-style-pdb-covered-side-entry.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  const style = target.style;
  style.setProperty('--token', 'value');
  style.paddingLeft = 'var(--pad)';
  style.setProperty('-webkit-text-fill-color', 'red');

  style.setProperty('padding', 'calc(calc(1px)) 2px', 'important');
  const afterShorthand = [
    style.getPropertyValue('--token'),
    style.getPropertyValue('-webkit-text-fill-color'),
    style.getPropertyValue('padding'),
    style.getPropertyPriority('padding'),
    style.paddingLeft,
    style.getPropertyPriority('padding-left'),
    Array.from({ length: style.length }, (_, index) => style.item(index)).join(','),
    style.cssText,
    target.getAttribute('style')
  ].join('|');

  style.setProperty('padding', '');
  const afterRemoveShorthand = [
    style.paddingLeft,
    style.getPropertyValue('--token'),
    style.getPropertyValue('-webkit-text-fill-color'),
    Array.from({ length: style.length }, (_, index) => style.item(index)).join(','),
    style.cssText,
    target.getAttribute('style')
  ].join('|');

  const allTarget = document.createElement('div');
  const allStyle = allTarget.style;
  allStyle.width = 'var(--w)';
  allStyle.setProperty('--token', 'value');
  allStyle.setProperty('all', 'inherit');
  const afterAll = [
    allStyle.width,
    allStyle.getPropertyValue('--token'),
    Array.from({ length: allStyle.length }, (_, index) => allStyle.item(index)).join(','),
    allStyle.cssText,
    allTarget.getAttribute('style')
  ].join('|');

  return [afterShorthand, afterRemoveShorthand, afterAll].join('/');
})()
"#,
        )
        .expect("live inline PDB mutations should replace fully covered side entries");

    assert_eq!(
        result,
        "value|red|calc(1px) 2px|important|2px|important|--token,-webkit-text-fill-color,padding-top,padding-right,padding-bottom,padding-left|--token: value; -webkit-text-fill-color: red; padding: calc(1px) 2px !important;|--token: value; -webkit-text-fill-color: red; padding: calc(1px) 2px !important;/|value|red|--token,-webkit-text-fill-color|--token: value; -webkit-text-fill-color: red;|--token: value; -webkit-text-fill-color: red;/inherit|value|--token,all|--token: value; all: inherit;|--token: value; all: inherit;"
    );
}

#[test]
fn live_inline_pdb_mutations_preserve_partially_covered_side_entries() {
    let mut vm = new_storage_test_vm("https://inline-style-pdb-partial-side-entry.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  const style = target.style;
  style.padding = 'var(--pad)';
  const beforeLonghand = [
    style.getPropertyValue('padding'),
    Array.from({ length: style.length }, (_, index) => style.item(index)).join(','),
    style.cssText,
    target.getAttribute('style')
  ].join('|');
  style.paddingLeft = 'calc(calc(1px))';
  const afterLonghand = [
    style.getPropertyValue('padding'),
    style.paddingLeft,
    style.getPropertyPriority('padding-left'),
    Array.from({ length: style.length }, (_, index) => style.item(index)).join(','),
    style.cssText,
    target.getAttribute('style')
  ].join('|');

  style.setProperty('padding-left', '2px', 'important');
  const afterImportantLonghand = [
    style.getPropertyValue('padding'),
    style.paddingLeft,
    style.getPropertyPriority('padding-left'),
    Array.from({ length: style.length }, (_, index) => style.item(index)).join(','),
    style.cssText,
    target.getAttribute('style')
  ].join('|');

  const removed = style.removeProperty('padding-left');
  const afterRemoveLonghand = [
    removed,
    style.getPropertyValue('padding'),
    style.paddingLeft,
    Array.from({ length: style.length }, (_, index) => style.item(index)).join(','),
    style.cssText,
    target.getAttribute('style')
  ].join('|');

  const importantTarget = document.createElement('div');
  const important = importantTarget.style;
  important.setProperty('padding', 'var(--pad)', 'important');
  important.paddingLeft = '1px';
  const importantSide = [
    important.paddingLeft,
    important.getPropertyPriority('padding-left'),
    Array.from({ length: important.length }, (_, index) => important.item(index)).join(','),
    important.cssText,
    importantTarget.getAttribute('style')
  ].join('|');

  return [beforeLonghand, afterLonghand, afterImportantLonghand, afterRemoveLonghand, importantSide].join('/');
})()
"#,
        )
        .expect("live inline PDB mutations should preserve partially covered side entries");

    assert_eq!(
        result,
        "var(--pad)|padding-top,padding-right,padding-bottom,padding-left|padding: var(--pad);|padding: var(--pad);/|calc(1px)||padding-top,padding-right,padding-bottom,padding-left|padding-top: ; padding-right: ; padding-bottom: ; padding-left: calc(1px);|padding-top: ; padding-right: ; padding-bottom: ; padding-left: calc(1px);/|2px|important|padding-top,padding-right,padding-bottom,padding-left|padding-top: ; padding-right: ; padding-bottom: ; padding-left: 2px !important;|padding-top: ; padding-right: ; padding-bottom: ; padding-left: 2px !important;/2px|||padding-top,padding-right,padding-bottom|padding-top: ; padding-right: ; padding-bottom: ;|padding-top: ; padding-right: ; padding-bottom: ;/1px||padding-top,padding-right,padding-bottom,padding-left|padding-top:  !important; padding-right:  !important; padding-bottom:  !important; padding-left: 1px;|padding-top:  !important; padding-right:  !important; padding-bottom:  !important; padding-left: 1px;"
    );
}

#[test]
fn live_inline_pdb_state_tracks_style_attribute_mutations() {
    let mut vm = new_storage_test_vm("https://inline-style-pdb-attribute-sync.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  const style = target.style;
  style.setProperty('display', 'block');
  style.setProperty('visibility', 'hidden');
  const before = [style.cssText, style.display, style.visibility].join(',');

  target.setAttribute('style', 'display: inline; color: red;');
  const afterSet = [style.cssText, style.display, style.visibility, style.color].join(',');

  target.removeAttribute('style');
  const afterRemove = [style.cssText, style.display, style.color, style.length].join(',');

  return [before, afterSet, afterRemove].join('|');
})()
"#,
        )
        .expect("live inline PDB state should track style attribute mutations");

    assert_eq!(
        result,
        "display: block; visibility: hidden;,block,hidden|display: inline; color: red;,inline,,red|,,,0"
    );
}

#[test]
fn live_inline_pdb_storage_directly_mutates_plain_properties() {
    let mut vm = new_storage_test_vm("https://inline-style-pdb-direct-storage.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  target.setAttribute('style', 'display: block; visibility: hidden;');
  const style = target.style;

  style.opacity = '0.5';
  const removed = style.removeProperty('display');
  style.setProperty('visibility', 'collapse');

  return [
    removed,
    style.display,
    style.visibility,
    style.opacity,
    style.length,
    Array.from({ length: style.length }, (_, index) => style.item(index)).join(','),
    style.cssText,
    target.getAttribute('style')
  ].join('|');
})()
"#,
        )
        .expect("live inline PDB storage should directly mutate plain properties");

    assert_eq!(
        result,
        "block||collapse|0.5|2|opacity,visibility|opacity: 0.5; visibility: collapse;|opacity: 0.5; visibility: collapse;"
    );
}

#[test]
fn live_inline_css_text_reset_preserves_all_adapter_with_pdb_storage() {
    let mut vm = new_storage_test_vm("https://inline-style-pdb-all-csstext.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  const style = target.style;
  style.cssText = 'display: block; all: inherit; padding-left: 1px;';
  const afterReset = [
    style.length,
    Array.from({ length: style.length }, (_, index) => style.item(index)).join(','),
    style.display,
    style.paddingLeft,
    style.cssText,
    target.getAttribute('style')
  ].join('|');

  style.display = 'inline';
  const afterLonghand = [
    style.getPropertyValue('all'),
    style.display,
    style.paddingLeft,
    Array.from({ length: style.length }, (_, index) => style.item(index)).join(','),
    style.cssText,
    target.getAttribute('style')
  ].join('|');

  const removedAll = style.removeProperty('all');
  const afterRemoveAll = [
    removedAll,
    style.display,
    style.paddingLeft,
    style.length,
    style.cssText,
    target.getAttribute('style')
  ].join('|');

  return [afterReset, afterLonghand, afterRemoveAll].join('/');
})()
"#,
        )
        .expect("live inline cssText reset should preserve all adapter with PDB storage");

    assert_eq!(
        result,
        "2|all,padding-left|inherit|1px|all: inherit; padding-left: 1px;|all: inherit; padding-left: 1px;/|inline|1px|all,padding-left,display|all: inherit; padding-left: 1px; display: inline;|all: inherit; padding-left: 1px; display: inline;/|||0||"
    );
}

#[test]
fn live_inline_all_mutations_update_pdb_storage_without_losing_cssom_order() {
    let mut vm = new_storage_test_vm("https://inline-style-pdb-all.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  const style = target.style;

  style.display = 'block';
  style.setProperty('all', 'inherit');
  const afterAll = [
    style.length,
    Array.from({ length: style.length }, (_, index) => style.item(index)).join(','),
    style.display,
    style.getPropertyValue('padding-left'),
    style.cssText,
    target.getAttribute('style')
  ].join('|');

  style.paddingLeft = '1px';
  const afterLonghand = [
    style.getPropertyValue('all'),
    style.display,
    style.paddingLeft,
    style.cssText,
    target.getAttribute('style')
  ].join('|');

  const overridden = document.createElement('div').style;
  overridden.setProperty('all', 'inherit');
  overridden.paddingLeft = '1px';
  const afterRemoveOverriddenAll = [
    overridden.removeProperty('all'),
    overridden.cssText
  ].join('|');

  style.setProperty('--token', 'value');
  style.setProperty('all', 'unset');
  const afterMixedReset = [
    style.getPropertyValue('--token'),
    style.display,
    style.cssText,
    target.getAttribute('style')
  ].join('|');

  const removedAll = style.removeProperty('all');
  const afterRemoveAll = [
    removedAll,
    style.display,
    style.getPropertyValue('--token'),
    style.cssText,
    target.getAttribute('style')
  ].join('|');

  return [
    afterAll,
    afterLonghand,
    afterRemoveOverriddenAll,
    afterMixedReset,
    afterRemoveAll
  ].join('/');
})()
"#,
        )
        .expect("live inline all mutations should update PDB storage");

    assert_eq!(
        result,
        "1|all|inherit|inherit|all: inherit;|all: inherit;/|inherit|1px|all: inherit; padding-left: 1px;|all: inherit; padding-left: 1px;/|/value|unset|--token: value; all: unset;|--token: value; all: unset;/unset||value|--token: value;|--token: value;"
    );
}

#[test]
fn css_rule_styles_use_stylo_declaration_block_for_plain_properties() {
    let mut vm = new_storage_test_vm("https://rule-style-pdb.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0];
  const style = rule.style;
  style.setProperty('display', 'invalid');
  style.setProperty('place-content', 'center start', 'important');
  const ruleWrite = [
    style.length,
    Array.from({ length: style.length }, (_, index) => style.item(index)).join(','),
    style.getPropertyValue('place-content'),
    style.getPropertyPriority('place-content'),
    style.cssText,
    rule.cssText
  ].join(',');

  style.cssText = 'color: invalid; color: red; padding: 1px 2px;';
  const cssTextWrite = [
    style.getPropertyValue('color'),
    style.getPropertyValue('padding'),
    style.cssText,
    rule.cssText
  ].join(',');

  const nestedSheet = new CSSStyleSheet();
  nestedSheet.replaceSync('.parent { color: red; .child { color: blue; } font-size: 12px; }');
  const nestedRule = nestedSheet.cssRules[0];
  const nested = nestedRule.cssRules[1];
  nested.style.setProperty('overflow', 'hidden visible', 'important');
  const nestedWrite = [
    nested.style.getPropertyValue('overflow'),
    nested.style.getPropertyPriority('overflow'),
    nested.style.cssText,
    nested.cssText,
    nestedRule.cssText.includes('overflow: hidden visible !important;')
  ].join(',');

  return [ruleWrite, cssTextWrite, nestedWrite].join('|');
})()
"#,
        )
        .expect("CSS rule styles should use Stylo declaration block");

    assert_eq!(
        result,
        "2,align-content,justify-content,center start,important,place-content: center start !important;,div { place-content: center start !important; }|red,1px 2px,color: red; padding: 1px 2px;,div { color: red; padding: 1px 2px; }|hidden visible,important,font-size: 12px; overflow: hidden visible !important;,font-size: 12px; overflow: hidden visible !important;,true"
    );
}

#[test]
fn css_rule_style_sync_uses_internal_declaration_state_not_css_text_property() {
    let mut vm = new_storage_test_vm("https://rule-style-internal-sync.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    div {}
    @keyframes fade { from {} }
    .parent { .child { color: blue; } font-size: 12px; }
  `);

  const rule = sheet.cssRules[0];
  Object.defineProperty(rule.style, 'cssText', {
    get() { return 'opacity: 0;'; },
    configurable: true
  });
  rule.style.setProperty('color', 'red');

  const keyframe = sheet.cssRules[1].cssRules[0];
  Object.defineProperty(keyframe.style, 'cssText', {
    get() { return 'opacity: 0;'; },
    configurable: true
  });
  keyframe.style.setProperty('opacity', '0.5');

  const parent = sheet.cssRules[2];
  const nested = parent.cssRules[1];
  Object.defineProperty(nested.style, 'cssText', {
    get() { return 'font-size: 1px;'; },
    configurable: true
  });
  nested.style.setProperty('color', 'green');

  return [
    rule.cssText,
    keyframe.cssText,
    nested.cssText,
    parent.cssText.includes('font-size: 12px; color: green;')
  ].join('|');
})()
"#,
        )
        .expect("CSS rule style sync should use internal declaration state");

    assert_eq!(
        result,
        "div { color: red; }|0% { opacity: 0.5; }|font-size: 12px; color: green;|true"
    );
}

#[test]
fn css_rule_style_assignment_uses_internal_declaration_setter() {
    let mut vm = new_storage_test_vm("https://rule-style-internal-assignment.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    div {}
    @keyframes fade { from {} }
    .parent { .child { color: blue; } font-size: 12px; }
    @page :first { margin-top: 1px; @top-left { content: "x"; } }
  `);

  const rule = sheet.cssRules[0];
  Object.defineProperty(rule.style, 'cssText', {
    value: 'opacity: 0;',
    writable: true,
    configurable: true
  });
  rule.style = 'color: rgb(0 128 0 / 50%); width: 0;';

  const keyframe = sheet.cssRules[1].cssRules[0];
  Object.defineProperty(keyframe.style, 'cssText', {
    value: 'opacity: 0;',
    writable: true,
    configurable: true
  });
  keyframe.style = 'background-color: rgb(0 128 0 / 50%); opacity: 1;';

  const parent = sheet.cssRules[2];
  const nested = parent.cssRules[1];
  Object.defineProperty(nested.style, 'cssText', {
    value: 'font-size: 1px;',
    writable: true,
    configurable: true
  });
  nested.style = 'color: rgb(0 128 0 / 50%); width: 0;';

  const page = sheet.cssRules[3];
  const margin = page.cssRules[0];
  Object.defineProperty(page.style, 'cssText', {
    value: 'margin-top: 99px;',
    writable: true,
    configurable: true
  });
  page.style = 'margin-top: 10px;';

  Object.defineProperty(margin.style, 'cssText', {
    value: 'content: "bad";',
    writable: true,
    configurable: true
  });
  margin.style = 'content: "y"; color: red;';

  return [
    rule.style.getPropertyValue('color'),
    rule.cssText,
    keyframe.style.getPropertyValue('background-color'),
    keyframe.cssText,
    nested.style.getPropertyValue('width'),
    nested.cssText,
    parent.cssText.includes('color: rgba(0, 128, 0, 0.5); width: 0px;'),
    page.style.getPropertyValue('margin-top'),
    margin.style.getPropertyValue('content'),
    margin.style.getPropertyValue('color'),
    page.cssText.includes('margin-top: 10px;'),
    page.cssText.includes('content: "y"; color: red;')
  ].join('|');
})()
"#,
        )
        .expect("CSS rule style assignment should use internal declaration setter");

    assert_eq!(
        result,
        "rgba(0, 128, 0, 0.5)|div { color: rgba(0, 128, 0, 0.5); width: 0px; }|rgba(0, 128, 0, 0.5)|0% { background-color: rgba(0, 128, 0, 0.5); opacity: 1; }|0px|color: rgba(0, 128, 0, 0.5); width: 0px;|true|10px|\"y\"|red|true|true"
    );
}

#[test]
fn css_rule_css_text_uses_seeded_pdb_block_for_safe_rules_before_style_wrapper() {
    let mut vm = new_storage_test_vm("https://rule-style-seeded-pdb.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    div { display: block; opacity: 0.5; }
    span { --x: 1; display: block; }
    @keyframes fade {
      from { opacity: 0.5; }
      to { animation-name: spin; opacity: 1; }
    }
  `);
  const pure = sheet.cssRules[0];
  const mixed = sheet.cssRules[1];
  const keyframePure = sheet.cssRules[2].cssRules[0];
  const keyframeMixed = sheet.cssRules[2].cssRules[1];

  const beforeMaterializingStyle = [
    pure.cssText,
    keyframePure.cssText,
    mixed.cssText,
    keyframeMixed.cssText
  ].join('|');

  return [
    beforeMaterializingStyle,
    pure.style.cssText,
    keyframePure.style.cssText
  ].join('||');
})()
"#,
        )
        .expect("CSS rule cssText should use seeded PDB state when safe");

    assert_eq!(
        result,
        "div { display: block; opacity: 0.5; }|0% { opacity: 0.5; }|span { --x: 1; display: block; }|100% { opacity: 1; }||display: block; opacity: 0.5;||opacity: 0.5;"
    );
}

#[test]
fn css_rule_css_text_reads_do_not_project_the_entire_live_stylesheet() {
    let mut vm = new_storage_test_vm("https://rule-css-text-local-projection.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    .first { color: red; }
    @media screen { .nested { color: blue; } }
    .last { color: green; }
  `);
  globalThis.__cssTextReadRules = [sheet.cssRules[0], sheet.cssRules[1]];
})()
"#,
    )
    .expect("CSS rule wrappers should materialize");

    crate::live_stylesheet::reset_live_stylesheet_css_text_projection_count_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const [style, media] = globalThis.__cssTextReadRules;
  return [style.cssText, media.cssText, style.cssText, media.cssText].join('|');
})()
"#,
        )
        .expect("cached CSS rule cssText reads should evaluate");

    assert_eq!(
        result,
        ".first { color: red; }|@media screen {\n  .nested { color: blue; }\n}|.first { color: red; }|@media screen {\n  .nested { color: blue; }\n}"
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_css_text_projection_count_for_test(),
        0,
        "read-only cssText access must not serialize the containing stylesheet"
    );
}

#[test]
fn css_rule_wrappers_are_counted_at_the_materialization_boundary() {
    let mut vm = new_storage_test_vm("https://css-rule-wrapper-materialization-count.test/");

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_wrapper_construction_count_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const style = document.createElement('style');
  style.textContent = Array.from(
    { length: 1000 },
    (_, index) => `.rule-${index} { --index: ${index}; }`
  ).join('\n');
  head.appendChild(style);
  const rules = style.sheet.cssRules;
  const length = rules.length;
  globalThis.__largeRuleList = rules;
  return length;
})()
"#,
        )
        .expect("large stylesheet should expose its rule count");

    assert_eq!(result, "1000");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        0,
        "reading CSSRuleList.length must not materialize any rule wrappers"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const rules = globalThis.__largeRuleList;
  const first = rules[0];
  const middle = rules.item(500);
  const last = Object.getOwnPropertyDescriptor(rules, "999").value;
  return [
    first.cssText,
    middle.cssText,
    last.cssText,
    rules[500] === middle,
    rules[0] === first,
  ].join('|');
})()
"#,
        )
        .expect("indexed CSSRuleList reads should materialize individual wrappers");

    assert_eq!(
        result,
        ".rule-0 { --index: 0; }|.rule-500 { --index: 500; }|.rule-999 { --index: 999; }|true|true"
    );
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        3,
        "repeated reads must preserve wrapper identity without materializing siblings"
    );
}

#[test]
fn css_rule_list_sparse_mutations_visit_only_materialized_entries() {
    let mut vm = new_storage_test_vm("https://css-rule-list-sparse-traversal.test/");

    for rule_count in [1_000_u32, 10_000_u32] {
        vm.eval(&format!(
            "globalThis.__sparseTraversalRuleCount = {rule_count};"
        ))
        .expect("sparse traversal rule count should initialize");
        vm.eval(
            r#"
(() => {
  const count = globalThis.__sparseTraversalRuleCount;
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(Array.from(
    { length: count },
    (_, index) => `.rule-${index} { --index: ${index}; }`
  ).join('\n'));
  const rules = sheet.cssRules;
  const middle = Math.floor(count / 2);
  globalThis.__sparseTraversalSheet = sheet;
  globalThis.__sparseTraversalRules = rules;
  globalThis.__sparseTraversalRetained = [rules[0], rules[middle], rules[count - 1]];
})()
"#,
        )
        .expect("sparse CSSRuleList traversal fixture should initialize");

        crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_list_materialized_traversal_metrics_for_test();
        let result = vm
            .eval(
                r#"
(() => {
  const count = globalThis.__sparseTraversalRuleCount;
  const sheet = globalThis.__sparseTraversalSheet;
  const rules = globalThis.__sparseTraversalRules;
  const retained = globalThis.__sparseTraversalRetained;
  const middle = Math.floor(count / 2);
  sheet.insertRule('.inserted { margin: 0; }', middle);
  return [
    rules.length,
    rules[0] === retained[0],
    rules[middle + 1] === retained[1],
    rules[count] === retained[2],
    rules[middle].cssText,
  ].join('|');
})()
"#,
            )
            .expect("middle insert should shift only materialized CSSRuleList entries");
        assert_eq!(
            result,
            format!(
                "{}|true|true|true|.inserted {{ margin: 0px; }}",
                rule_count + 1
            )
        );
        assert_eq!(
            crate::context_bootstrap::css_stylesheet_runtime::css_rule_list_materialized_traversal_metrics_for_test(),
            crate::context_bootstrap::css_stylesheet_runtime::CssRuleListMaterializedTraversalMetrics {
                traversals: 1,
                entries: 3,
            },
            "middle insert must not scan the logical CSSRuleList length"
        );

        crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_list_materialized_traversal_metrics_for_test();
        let result = vm
            .eval(
                r#"
(() => {
  const count = globalThis.__sparseTraversalRuleCount;
  const sheet = globalThis.__sparseTraversalSheet;
  const rules = globalThis.__sparseTraversalRules;
  const retained = globalThis.__sparseTraversalRetained;
  const middle = Math.floor(count / 2);
  sheet.deleteRule(middle);
  return [
    rules.length,
    rules[0] === retained[0],
    rules[middle] === retained[1],
    rules[count - 1] === retained[2],
  ].join('|');
})()
"#,
            )
            .expect("middle delete should shift only materialized CSSRuleList entries");
        assert_eq!(result, format!("{rule_count}|true|true|true"));
        assert_eq!(
            crate::context_bootstrap::css_stylesheet_runtime::css_rule_list_materialized_traversal_metrics_for_test(),
            crate::context_bootstrap::css_stylesheet_runtime::CssRuleListMaterializedTraversalMetrics {
                traversals: 1,
                entries: 4,
            },
            "middle delete must not scan the logical CSSRuleList length"
        );

        crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_list_materialized_traversal_metrics_for_test();
        let result = vm
            .eval(
                r#"
(() => {
  const sheet = globalThis.__sparseTraversalSheet;
  const rules = globalThis.__sparseTraversalRules;
  const retained = globalThis.__sparseTraversalRetained;
  sheet.replaceSync('.replacement { color: green; }');
  return [
    sheet.cssRules === rules,
    rules.length,
    retained[0].cssText,
    retained[1].cssText,
    retained[2].cssText,
  ].join('|');
})()
"#,
            )
            .expect("whole-sheet replacement should retire only materialized entries");
        assert_eq!(
            result,
            format!(
                "true|1|.rule-0 {{ --index: 0; }}|.rule-{} {{ --index: {}; }}|.rule-{} {{ --index: {}; }}",
                rule_count / 2,
                rule_count / 2,
                rule_count - 1,
                rule_count - 1,
            )
        );
        assert_eq!(
            crate::context_bootstrap::css_stylesheet_runtime::css_rule_list_materialized_traversal_metrics_for_test(),
            crate::context_bootstrap::css_stylesheet_runtime::CssRuleListMaterializedTraversalMetrics {
                traversals: 1,
                entries: 3,
            },
            "whole-sheet replacement must not scan the logical CSSRuleList length"
        );
    }
}

#[test]
fn css_rule_pdb_probe_visits_only_materialized_descendants() {
    let mut vm = new_storage_test_vm("https://css-rule-pdb-sparse-traversal.test/");

    for rule_count in [1_000_u32, 10_000_u32] {
        vm.eval(&format!(
            "globalThis.__pdbSparseTraversalRuleCount = {rule_count};"
        ))
        .expect("PDB sparse traversal rule count should initialize");
        vm.eval(
            r#"
(() => {
  const count = globalThis.__pdbSparseTraversalRuleCount;
  const sheet = new CSSStyleSheet();
  const children = Array.from(
    { length: count },
    (_, index) => `.rule-${index} { --index: ${index}; }`
  ).join('\n');
  sheet.replaceSync(`@media screen { ${children} }`);
  const media = sheet.cssRules[0];
  const rules = media.cssRules;
  const middle = Math.floor(count / 2);
  globalThis.__pdbSparseTraversalMedia = media;
  globalThis.__pdbSparseTraversalRules = rules;
  globalThis.__pdbSparseTraversalRetained = [rules[0], rules[middle], rules[count - 1]];
})()
"#,
        )
        .expect("PDB sparse traversal fixture should initialize");

        crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_list_materialized_traversal_metrics_for_test();
        let result = vm
            .eval(
                r#"
(() => {
  const count = globalThis.__pdbSparseTraversalRuleCount;
  const media = globalThis.__pdbSparseTraversalMedia;
  const rules = globalThis.__pdbSparseTraversalRules;
  const retained = globalThis.__pdbSparseTraversalRetained;
  const middle = Math.floor(count / 2);
  const text = media.cssText;
  return [
    text.startsWith('@media screen'),
    text.includes(`.rule-${count - 1} { --index: ${count - 1}; }`),
    rules[0] === retained[0],
    rules[middle] === retained[1],
    rules[count - 1] === retained[2],
  ].join('|');
})()
"#,
            )
            .expect("grouping cssText should probe only materialized PDB descendants");

        assert_eq!(result, "true|true|true|true|true");
        assert_eq!(
            crate::context_bootstrap::css_stylesheet_runtime::css_rule_list_materialized_traversal_metrics_for_test(),
            crate::context_bootstrap::css_stylesheet_runtime::CssRuleListMaterializedTraversalMetrics {
                traversals: 1,
                entries: 3,
            },
            "PDB side-entry detection must not scan the logical child-rule count"
        );
    }
}

#[test]
fn css_grouping_rule_children_materialize_on_indexed_access() {
    let mut vm = new_storage_test_vm("https://css-grouping-child-materialization.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  const children = Array.from(
    { length: 1000 },
    (_, index) => `.rule-${index} { --index: ${index}; }`
  ).join('\n');
  sheet.replaceSync(`@media screen { ${children} }`);
  globalThis.__largeGroupingSheet = sheet;
})()
"#,
    )
    .expect("large grouping stylesheet should parse");

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_wrapper_construction_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = globalThis.__largeGroupingSheet;
  const media = sheet.cssRules[0];
  const rules = media.cssRules;
  globalThis.__largeGroupingRule = media;
  globalThis.__largeGroupingRuleList = rules;
  return [
    sheet.cssRules.length,
    media.conditionText,
    media.media.mediaText,
    rules.length,
  ].join('|');
})()
"#,
        )
        .expect("large grouping rule should expose native child count");

    assert_eq!(result, "1|screen|screen|1000");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        1,
        "reading a grouping rule child-list length must materialize only the parent wrapper"
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test()
            .recursive_rule_snapshots,
        0,
        "materializing a grouping parent must not recursively project its child rules"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const rules = globalThis.__largeGroupingRuleList;
  const first = rules[0];
  const middle = rules.item(500);
  const last = Object.getOwnPropertyDescriptor(rules, '999').value;
  return [
    first.cssText,
    middle.cssText,
    last.cssText,
    rules[0] === first,
    rules[500] === middle,
  ].join('|');
})()
"#,
        )
        .expect("grouping child indexed reads should materialize individual wrappers");

    assert_eq!(
        result,
        ".rule-0 { --index: 0; }|.rule-500 { --index: 500; }|.rule-999 { --index: 999; }|true|true"
    );
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        4,
        "grouping child reads must preserve identity without materializing siblings"
    );
}

#[test]
fn css_page_rule_fields_do_not_project_large_margin_rule_subtree() {
    let mut vm = new_storage_test_vm("https://css-page-rule-shallow-read.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  const children = Array.from(
    { length: 1000 },
    (_, index) => `@top-left { --index: ${index}; }`
  ).join('\n');
  sheet.replaceSync(`@page :first { margin-top: 1px; ${children} }`);
  globalThis.__largePageRuleSheet = sheet;
})()
"#,
    )
    .expect("large page-rule stylesheet should parse");

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_wrapper_construction_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const page = globalThis.__largePageRuleSheet.cssRules[0];
  return [
    page.selectorText,
    page.style.marginTop,
    page.cssRules.length,
  ].join('|');
})()
"#,
        )
        .expect("page rule fields should use shallow native reads");

    assert_eq!(result, ":first|1px|1000");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        1,
        "reading page fields and child-list length must materialize only the page wrapper"
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test()
            .recursive_rule_snapshots,
        0,
        "page selector and declaration reads must not project margin-rule children"
    );
}

#[test]
fn css_nested_style_rule_children_materialize_without_recursive_projection() {
    let mut vm = new_storage_test_vm("https://css-style-rule-child-materialization.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  const children = Array.from(
    { length: 1000 },
    (_, index) => `& .rule-${index} { --index: ${index}; }`
  ).join('\n');
  sheet.replaceSync(`.host { color: red; ${children} }`);
  globalThis.__largeNestedStyleSheet = sheet;
})()
"#,
    )
    .expect("large nested style rule should parse");

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_wrapper_construction_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = globalThis.__largeNestedStyleSheet;
  const parent = sheet.cssRules[0];
  const rules = parent.cssRules;
  globalThis.__largeNestedStyleRuleList = rules;
  return [
    parent.selectorText,
    parent.style.getPropertyValue('color'),
    rules.length,
  ].join('|');
})()
"#,
        )
        .expect("nested style parent should expose native declarations and child count");

    assert_eq!(result, ".host|red|1000");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        1,
        "reading a nested style child-list length must materialize only the parent wrapper"
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test()
            .recursive_rule_snapshots,
        0,
        "materializing a nested style parent must not recursively project its child rules"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const rules = globalThis.__largeNestedStyleRuleList;
  const first = rules[0];
  const middle = rules.item(500);
  const last = Object.getOwnPropertyDescriptor(rules, '999').value;
  return [
    first.cssText,
    middle.cssText,
    last.cssText,
    rules[0] === first,
    rules[500] === middle,
  ].join('|');
})()
"#,
        )
        .expect("nested style child indexed reads should materialize individual wrappers");

    assert_eq!(
        result,
        "& .rule-0 { --index: 0; }|& .rule-500 { --index: 500; }|& .rule-999 { --index: 999; }|true|true"
    );
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        4,
        "nested style child reads must preserve identity without materializing siblings"
    );
}

#[test]
fn css_nested_declarations_materialize_without_rule_projection() {
    let mut vm = new_storage_test_vm("https://css-nested-declarations-materialization.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    @supports (display: grid) {
      .host {
        & .child { color: blue; }
        color: red;
        margin: 0;
      }
    }
  `);
  globalThis.__nestedDeclarationsSheet = sheet;
})()
"#,
    )
    .expect("nested declarations stylesheet should parse");

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_wrapper_construction_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = globalThis.__nestedDeclarationsSheet;
  const supports = sheet.cssRules[0];
  const style = supports.cssRules[0];
  const declarations = style.cssRules[1];
  return [
    declarations instanceof CSSNestedDeclarations,
    declarations.cssText,
    declarations.style.color,
    declarations.style.margin,
    style.cssRules[1] === declarations,
  ].join('|');
})()
"#,
        )
        .expect("nested declarations wrapper should materialize from its native seed");

    assert_eq!(result, "true|color: red; margin: 0px;|red|0px|true");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        3,
        "only the explicitly read supports, style, and nested declarations wrappers should exist"
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test()
            .recursive_rule_snapshots,
        0,
        "nested declarations materialization must not build a recursive rule snapshot"
    );
}

#[test]
fn css_keyframes_rule_children_materialize_on_indexed_access() {
    let mut vm = new_storage_test_vm("https://css-keyframes-child-materialization.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  const children = Array.from(
    { length: 1000 },
    (_, index) => `${index / 10}% { --index: ${index}; }`
  ).join('\n');
  sheet.replaceSync(`@keyframes dense { ${children} }`);
  globalThis.__largeKeyframesSheet = sheet;
})()
"#,
    )
    .expect("large keyframes stylesheet should parse");

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_wrapper_construction_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = globalThis.__largeKeyframesSheet;
  const keyframes = sheet.cssRules[0];
  const rules = keyframes.cssRules;
  globalThis.__largeKeyframesRule = keyframes;
  globalThis.__largeKeyframesRuleList = rules;
  return [keyframes.name, keyframes.length, rules.length].join('|');
})()
"#,
        )
        .expect("large keyframes rule should expose native child count");

    assert_eq!(result, "dense|1000|1000");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        1,
        "reading keyframe counts must materialize only the CSSKeyframesRule wrapper"
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test()
            .recursive_rule_snapshots,
        0,
        "materializing a keyframes parent must not recursively project its keyframe rules"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const keyframes = globalThis.__largeKeyframesRule;
  const rules = globalThis.__largeKeyframesRuleList;
  const found = keyframes.findRule('50%');
  return [found.cssText, found === rules[500]].join('|');
})()
"#,
        )
        .expect("attached keyframe lookup should search the native rule list");

    assert_eq!(result, "50% { --index: 500; }|true");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        2,
        "findRule must materialize only the matching keyframe"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const keyframes = globalThis.__largeKeyframesRule;
  const rules = globalThis.__largeKeyframesRuleList;
  const first = keyframes[0];
  const middle = rules.item(500);
  const last = Object.getOwnPropertyDescriptor(keyframes, '999').value;
  return [
    first.cssText,
    middle.cssText,
    last.cssText,
    keyframes[0] === rules[0],
    keyframes[500] === middle,
  ].join('|');
})()
"#,
        )
        .expect("keyframe indexed reads should share lazy CSSRuleList wrappers");

    assert_eq!(
        result,
        "0% { --index: 0; }|50% { --index: 500; }|99.9% { --index: 999; }|true|true"
    );
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        4,
        "CSSKeyframesRule and cssRules must share sparse wrapper identity"
    );

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_wrapper_construction_count_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const keyframes = globalThis.__largeKeyframesRule;
  keyframes.deleteRule('25%');
  return [keyframes.length, keyframes.findRule('25%') === null].join('|');
})()
"#,
        )
        .expect("attached keyframe deletion should search the native rule list");

    assert_eq!(result, "999|true");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        0,
        "deleteRule must not materialize keyframes while locating the native match"
    );
}

#[test]
fn css_style_sheet_insert_rule_does_not_reproject_existing_rule_wrappers() {
    let mut vm = new_storage_test_vm("https://css-insert-rule-local-projection.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(Array.from(
    { length: 128 },
    (_, index) => `.rule-${index} { color: rgb(${index % 255}, 0, 0); }`
  ).join('\n'));
  globalThis.__insertRuleProjectionSheet = sheet;
  globalThis.__insertRuleProjectionFirst = sheet.cssRules[0];
  globalThis.__insertRuleProjectionMiddle = sheet.cssRules[64];
})()
"#,
    )
    .expect("CSS rule wrappers should materialize");

    crate::live_stylesheet::reset_live_stylesheet_css_text_projection_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = globalThis.__insertRuleProjectionSheet;
  sheet.insertRule('.inserted { margin: 0; }', 64);
  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === globalThis.__insertRuleProjectionFirst,
    sheet.cssRules[65] === globalThis.__insertRuleProjectionMiddle,
    sheet.cssRules[64].cssText,
  ].join('|');
})()
"#,
        )
        .expect("incremental CSS rule insertion should evaluate");

    assert_eq!(result, "129|true|true|.inserted { margin: 0px; }");
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_css_text_projection_count_for_test(),
        0,
        "insertRule must not reproject the live stylesheet after the mutation result"
    );
    let metrics = crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test();
    assert_eq!(metrics.native_top_level_mutations, 1);
    assert_eq!(
        metrics.recursive_rule_snapshots, 0,
        "materializing the inserted style rule must use its shallow native seed"
    );
}

#[test]
fn css_style_sheet_dense_append_mutations_stay_on_the_native_single_rule_path() {
    let mut vm = new_storage_test_vm("https://css-insert-rule-native-density.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('');
  globalThis.__denseNativeMutationSheet = sheet;
  globalThis.__denseNativeMutationRules = sheet.cssRules;
})()
"#,
    )
    .expect("dense native CSSStyleSheet fixture should initialize");

    crate::live_stylesheet::reset_live_stylesheet_css_text_projection_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = globalThis.__denseNativeMutationSheet;
  const rules = globalThis.__denseNativeMutationRules;
  for (let index = 0; index < 1000; index++) {
    sheet.insertRule(`.rule-${index} { --index: ${index}; }`, rules.length);
  }
  return [
    rules.length,
    rules[0].cssText,
    rules[999].cssText,
  ].join('|');
})()
"#,
        )
        .expect("dense native CSSStyleSheet insertion should evaluate");

    assert_eq!(
        result,
        "1000|.rule-0 { --index: 0; }|.rule-999 { --index: 999; }"
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_css_text_projection_count_for_test(),
        0
    );
    let metrics = crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test();
    assert_eq!(metrics.native_top_level_mutations, 1000);
    assert_eq!(
        metrics.recursive_rule_snapshots, 0,
        "append-only insertRule must seed read wrappers without full rule projection"
    );
}

#[test]
fn css_grouping_rule_dense_append_avoids_full_stylesheet_projection() {
    let mut vm = new_storage_test_vm("https://css-grouping-rule-native-density.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@media screen { }');
  globalThis.__denseNestedMutationSheet = sheet;
  globalThis.__denseNestedMutationParent = sheet.cssRules[0];
  globalThis.__denseNestedMutationRules = sheet.cssRules[0].cssRules;
})()
"#,
    )
    .expect("dense native grouping-rule fixture should initialize");

    crate::context_bootstrap::css_stylesheet_runtime::reset_detached_rule_mutation_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_css_text_projection_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    let mutation_result = vm
        .eval(
            r#"
(() => {
  const parent = globalThis.__denseNestedMutationParent;
  const rules = globalThis.__denseNestedMutationRules;
  for (let index = 0; index < 1000; index++) {
    parent.insertRule(`.rule-${index} { --index: ${index}; }`, rules.length);
  }
  return rules.length;
})()
"#,
        )
        .expect("dense native grouping-rule insertion should evaluate");

    assert_eq!(mutation_result, "1000");
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_css_text_projection_count_for_test(),
        0
    );
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::detached_rule_mutation_count_for_test(),
        0
    );
    let metrics = crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test();
    assert_eq!(metrics.native_top_level_mutations, 0);
    assert_eq!(metrics.native_nested_mutations, 1000);
    assert_eq!(metrics.native_keyframe_mutations, 0);
    assert_eq!(
        metrics.recursive_rule_snapshots, 0,
        "nested insertRule must seed read style wrappers without recursive projection"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const parent = globalThis.__denseNestedMutationParent;
  const rules = globalThis.__denseNestedMutationRules;
  const parentText = parent.cssText;
  const parentTextAgain = parent.cssText;
  return [
    rules.length,
    rules[0].cssText,
    rules[999].cssText,
    parentText.includes('.rule-0 { --index: 0; }'),
    parentText.includes('.rule-999 { --index: 999; }'),
    parentTextAgain === parentText,
  ].join('|');
})()
"#,
        )
        .expect("dense native grouping-rule insertion should evaluate");

    assert_eq!(
        result,
        "1000|.rule-0 { --index: 0; }|.rule-999 { --index: 999; }|true|true|true"
    );
}

#[test]
fn css_keyframes_rule_dense_append_avoids_full_stylesheet_projection() {
    let mut vm = new_storage_test_vm("https://css-keyframes-rule-native-density.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@keyframes dense { }');
  globalThis.__denseKeyframeMutationSheet = sheet;
  globalThis.__denseKeyframeMutationParent = sheet.cssRules[0];
  globalThis.__denseKeyframeMutationRules = sheet.cssRules[0].cssRules;
})()
"#,
    )
    .expect("dense native keyframes fixture should initialize");

    crate::context_bootstrap::css_stylesheet_runtime::reset_detached_rule_mutation_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_css_text_projection_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    let mutation_result = vm
        .eval(
            r#"
(() => {
  const parent = globalThis.__denseKeyframeMutationParent;
  const rules = globalThis.__denseKeyframeMutationRules;
  for (let index = 0; index < 1000; index++) {
    parent.appendRule(`${index / 10}% { --index: ${index}; }`);
  }
  return rules.length;
})()
"#,
        )
        .expect("dense native keyframe insertion should evaluate");

    assert_eq!(mutation_result, "1000");
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_css_text_projection_count_for_test(),
        0
    );
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::detached_rule_mutation_count_for_test(),
        0
    );
    let metrics = crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test();
    assert_eq!(metrics.native_top_level_mutations, 0);
    assert_eq!(metrics.native_nested_mutations, 0);
    assert_eq!(metrics.native_keyframe_mutations, 1000);
    assert_eq!(
        metrics.recursive_rule_snapshots, 0,
        "appendRule must seed keyframe wrappers without full rule projection"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const parent = globalThis.__denseKeyframeMutationParent;
  const rules = globalThis.__denseKeyframeMutationRules;
  const parentText = parent.cssText;
  const parentTextAgain = parent.cssText;
  return [
    rules.length,
    rules[0].cssText,
    rules[999].cssText,
    parentText.includes('0% { --index: 0; }'),
    parentText.includes('99.9% { --index: 999; }'),
    parentTextAgain === parentText,
  ].join('|');
})()
"#,
        )
        .expect("dense native keyframe insertion should evaluate");

    assert_eq!(
        result,
        "1000|0% { --index: 0; }|99.9% { --index: 999; }|true|true|true"
    );
}

#[test]
fn css_style_rule_dense_value_mutation_stays_on_the_native_single_rule_path() {
    let mut vm = new_storage_test_vm("https://css-style-rule-native-value-density.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.subject { color: red; } .sibling { color: blue; }');
  globalThis.__denseValueMutationSheet = sheet;
  globalThis.__denseValueMutationRule = sheet.cssRules[0];
  globalThis.__denseValueMutationSibling = sheet.cssRules[1];
})()
"#,
    )
    .expect("dense native rule-value fixture should initialize");

    crate::live_stylesheet::reset_live_stylesheet_css_text_projection_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = globalThis.__denseValueMutationSheet;
  const rule = globalThis.__denseValueMutationRule;
  const sibling = globalThis.__denseValueMutationSibling;
  for (let index = 0; index < 1000; index++) {
    rule.style.setProperty('--iteration', String(index));
  }
  return [
    sheet.cssRules[0] === rule,
    sheet.cssRules[1] === sibling,
    rule.style.getPropertyValue('--iteration'),
    rule.cssText,
    sibling.cssText,
  ].join('|');
})()
"#,
        )
        .expect("dense native rule-value mutation should evaluate");

    assert_eq!(
        result,
        "true|true|999|.subject { color: red; --iteration: 999; }|.sibling { color: blue; }"
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_css_text_projection_count_for_test(),
        0
    );
    let metrics = crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test();
    assert_eq!(metrics.native_rule_value_mutations, 1000);
    assert_eq!(metrics.recursive_rule_snapshots, 0);
}

#[test]
fn css_grouping_rule_replacement_keeps_new_children_sparse() {
    let mut vm = new_storage_test_vm("https://css-grouping-replacement-sparse.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@media screen { .old { color: red; } .unmaterialized { color: blue; } }');
  const media = sheet.cssRules[0];
  const rules = media.cssRules;
  globalThis.__sparseReplacementMedia = media;
  globalThis.__sparseReplacementRules = rules;
  globalThis.__sparseReplacementOld = rules[0];
})()
"#,
    )
    .expect("sparse replacement fixture should initialize");

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_rule_wrapper_construction_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const media = globalThis.__sparseReplacementMedia;
  const rules = globalThis.__sparseReplacementRules;
  const old = globalThis.__sparseReplacementOld;
  const children = Array.from(
    { length: 1000 },
    (_, index) => `.new-${index} { --index: ${index}; }`
  ).join('\n');
  media.cssText = `@media print { ${children} }`;
  return [
    media.cssRules === rules,
    rules.length,
    old.parentRule === null,
    old.parentStyleSheet === null,
    old.cssText,
  ].join('|');
})()
"#,
        )
        .expect("grouping replacement should reset its existing child list");

    assert_eq!(result, "true|1000|true|true|.old { color: red; }");
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        0,
        "replacement must not materialize any rule from the new subtree"
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test()
            .recursive_rule_snapshots,
        1,
        "replacement may snapshot only the retained old wrapper, not the new native subtree"
    );

    let result = vm
        .eval(
            r#"
(() => {
  const rules = globalThis.__sparseReplacementRules;
  const first = rules[0];
  const middle = rules[500];
  const last = rules[999];
  return [
    first.cssText,
    middle.cssText,
    last.cssText,
    rules[0] === first,
    rules[500] === middle,
  ].join('|');
})()
"#,
        )
        .expect("replacement children should materialize on indexed access");

    assert_eq!(
        result,
        ".new-0 { --index: 0; }|.new-500 { --index: 500; }|.new-999 { --index: 999; }|true|true"
    );
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_rule_wrapper_construction_count_for_test(),
        3
    );
}

#[test]
fn css_rule_side_entry_mutation_preserves_live_stylesheet_authority() {
    let mut vm = new_storage_test_vm("https://css-rule-side-entry-native-authority.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const target = body.appendChild(document.createElement('div'));
  target.className = 'subject';
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    .subject {
      color: rgb(1, 2, 3);
      & .child { color: blue; }
      font-size: 11px;
    }
    @keyframes pulse { from { opacity: 0; } }
  `);
  document.adoptedStyleSheets = [sheet];
  globalThis.__sideEntrySheet = sheet;
  globalThis.__sideEntryRule = sheet.cssRules[0];
  globalThis.__sideEntryNestedDeclarations = sheet.cssRules[0].cssRules[1];
  globalThis.__sideEntryKeyframe = sheet.cssRules[1].cssRules[0];
  globalThis.__sideEntryTarget = target;
})()
"#,
    )
    .expect("side-entry native authority fixture should initialize");

    crate::live_stylesheet::reset_live_stylesheet_parse_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_css_text_projection_count_for_test();
    crate::style_engine::reset_author_source_text_parse_count_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const sheet = globalThis.__sideEntrySheet;
  const rule = globalThis.__sideEntryRule;
  const nestedDeclarations = globalThis.__sideEntryNestedDeclarations;
  const keyframe = globalThis.__sideEntryKeyframe;
  rule.style.setProperty('-webkit-text-fill-color', 'red');
  rule.style.setProperty('margin-left', '7px');
  nestedDeclarations.style.setProperty('-webkit-text-fill-color', 'green');
  nestedDeclarations.style.setProperty('margin-right', '9px');
  keyframe.style.setProperty('-webkit-text-fill-color', 'blue');
  keyframe.style.setProperty('opacity', '0.5');
  const computed = getComputedStyle(globalThis.__sideEntryTarget);
  return [
    document.adoptedStyleSheets[0] === sheet,
    sheet.cssRules[0] === rule,
    rule.style.getPropertyValue('-webkit-text-fill-color'),
    rule.style.marginLeft,
    nestedDeclarations.style.getPropertyValue('-webkit-text-fill-color'),
    nestedDeclarations.style.marginRight,
    keyframe.style.getPropertyValue('-webkit-text-fill-color'),
    keyframe.style.opacity,
    computed.color,
    computed.marginLeft,
    computed.marginRight,
  ].join('|');
})()
"#,
        )
        .expect("side-entry mutation should evaluate");

    assert_eq!(
        result,
        "true|true|red|7px|green|9px|blue|0.5|rgb(1, 2, 3)|7px|9px"
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_parse_count_for_test(),
        0,
        "a supplemental CSSOM side entry must not replace the native stylesheet"
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_css_text_projection_count_for_test(),
        0,
        "a supplemental CSSOM side entry must not serialize the whole stylesheet"
    );
    assert_eq!(
        crate::style_engine::author_source_text_parse_count_for_test(),
        0,
        "the adopted Stylist must keep consuming the same parsed stylesheet"
    );
}

#[test]
fn inline_style_sheet_getter_does_not_reparse_unchanged_cssom_source() {
    let mut vm = new_storage_test_vm("https://inline-sheet-getter-cache.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const style = document.createElement('style');
  style.textContent = Array.from(
    { length: 128 },
    (_, index) => `.initial-${index} { color: rgb(${index % 255}, 0, 0); }`
  ).join('\n');
  head.appendChild(style);

  const sheet = style.sheet;
  globalThis.__inlineGetterStyle = style;
  globalThis.__inlineGetterSheet = sheet;
  globalThis.__inlineGetterOwnerText = style.textContent;
  globalThis.__inlineGetterFirstRule = sheet.cssRules[0];
  globalThis.__inlineGetterMiddleRule = sheet.cssRules[64];
})()
"#,
    )
    .expect("inline stylesheet should materialize");

    crate::context_bootstrap::css_stylesheet_runtime::reset_css_style_sheet_rule_sync_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_css_text_projection_count_for_test();
    crate::style_engine::reset_author_source_text_parse_count_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const style = globalThis.__inlineGetterStyle;
  const sheet = globalThis.__inlineGetterSheet;
  for (let index = 0; index < 64; index += 1) {
    const current = style.sheet;
    if (current !== sheet) throw new Error('sheet identity changed');
    current.insertRule(`.inserted-${index} { margin: ${index}px; }`, current.cssRules.length);
  }
  const body = document.body || document.documentElement.appendChild(document.createElement('body'));
  const target = body.appendChild(document.createElement('div'));
  target.className = 'inserted-63';
  return [
    sheet.cssRules.length,
    sheet.cssRules[0] === globalThis.__inlineGetterFirstRule,
    sheet.cssRules[64] === globalThis.__inlineGetterMiddleRule,
    sheet.cssRules[191].cssText,
    style.textContent === globalThis.__inlineGetterOwnerText,
    getComputedStyle(target).marginLeft,
  ].join('|');
})()
"#,
        )
        .expect("repeated inline sheet access and insertion should evaluate");

    assert_eq!(
        result,
        "192|true|true|.inserted-63 { margin: 63px; }|true|63px"
    );
    assert_eq!(
        crate::context_bootstrap::css_stylesheet_runtime::css_style_sheet_rule_sync_count_for_test(
        ),
        0,
        "reading the cached inline sheet must not reparse its unchanged CSSOM source"
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_css_text_projection_count_for_test(),
        0,
        "inline CSSOM mutation must not serialize the live sheet for the Stylist"
    );
    assert_eq!(
        crate::style_engine::author_source_text_parse_count_for_test(),
        0,
        "the Stylist must consume the owner CSSOM's live parsed stylesheet"
    );
}

#[test]
fn svg_style_uses_the_shared_owner_live_stylesheet_pipeline() {
    let mut vm = new_storage_test_vm("https://svg-live-stylesheet.test/");

    let initial = vm
        .eval(
            r#"
(() => {
  const SVG_NS = 'http://www.w3.org/2000/svg';
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const target = body.appendChild(document.createElement('div'));
  target.className = 'svg-owned-target';
  const svg = document.createElementNS(SVG_NS, 'svg');
  const style = document.createElementNS(SVG_NS, 'style');
  style.textContent = '.svg-owned-target { margin-left: 13px; }';
  svg.appendChild(style);
  body.appendChild(svg);
  globalThis.__svgLiveStyle = style;
  globalThis.__svgLiveFirstSheet = style.sheet;
  globalThis.__svgLiveTarget = target;
  return [
    style.sheet instanceof CSSStyleSheet,
    style.sheet === globalThis.__svgLiveFirstSheet,
    getComputedStyle(target).marginLeft,
  ].join('|');
})()
"#,
        )
        .expect("SVG style should install through the owner stylesheet pipeline");
    assert_eq!(initial, "true|true|13px");

    crate::live_stylesheet::reset_live_stylesheet_parse_count_for_test();
    crate::live_stylesheet::reset_live_stylesheet_css_text_projection_count_for_test();
    crate::style_engine::reset_author_source_text_parse_count_for_test();
    let mutated = vm
        .eval(
            r#"
(() => {
  const style = globalThis.__svgLiveStyle;
  const firstSheet = globalThis.__svgLiveFirstSheet;
  const target = globalThis.__svgLiveTarget;
  style.textContent = '.svg-owned-target { margin-left: 17px; }';
  const replacement = style.sheet;
  replacement.insertRule(
    '.svg-owned-target { margin-right: 19px; }',
    replacement.cssRules.length,
  );
  return [
    replacement instanceof CSSStyleSheet,
    replacement !== firstSheet,
    replacement === style.sheet,
    replacement.cssRules.length,
    getComputedStyle(target).marginLeft,
    getComputedStyle(target).marginRight,
  ].join('|');
})()
"#,
        )
        .expect("SVG style text and CSSOM mutation should remain live");

    assert_eq!(mutated, "true|true|true|2|17px|19px");
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_parse_count_for_test(),
        1,
        "changing SVG style text should parse one replacement live stylesheet"
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_css_text_projection_count_for_test(),
        0,
        "SVG CSSOM mutation must not serialize the live stylesheet"
    );
    assert_eq!(
        crate::style_engine::author_source_text_parse_count_for_test(),
        0,
        "the Stylist must consume the SVG owner's parsed live stylesheet"
    );
}

#[test]
fn identical_inline_stylesheets_copy_on_write_without_losing_cssom_identity() {
    let mut vm = new_storage_test_vm("https://inline-sheet-copy-on-write.test/");

    crate::live_stylesheet::reset_live_stylesheet_parse_count_for_test();
    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const target = body.appendChild(document.createElement('div'));
  target.className = 'target nested';
  const css = [
    '.target { color: rgb(1, 2, 3); }',
    '@media screen { .nested { margin-left: 2px; } }',
  ].join('\n');

  const firstOwner = document.createElement('style');
  const secondOwner = document.createElement('style');
  firstOwner.textContent = css;
  secondOwner.textContent = css;
  head.append(firstOwner, secondOwner);

  const firstSheet = firstOwner.sheet;
  const secondSheet = secondOwner.sheet;
  const firstStyleRule = firstSheet.cssRules[0];
  const firstNestedRule = firstSheet.cssRules[1].cssRules[0];
  const secondStyleRule = secondSheet.cssRules[0];
  const secondNestedRule = secondSheet.cssRules[1].cssRules[0];
  firstStyleRule.marker = 'first-style';
  firstNestedRule.marker = 'first-nested';
  secondStyleRule.marker = 'second-style';
  secondNestedRule.marker = 'second-nested';

  firstStyleRule.style.color = 'rgb(4, 5, 6)';
  firstNestedRule.style.marginLeft = '7px';
  firstSheet.insertRule('.first-only { padding-left: 9px; }', firstSheet.cssRules.length);

  secondSheet.disabled = true;
  const firstComputed = getComputedStyle(target);
  const firstResult = [firstComputed.color, firstComputed.marginLeft].join('|');
  firstSheet.disabled = true;
  secondSheet.disabled = false;
  const secondComputed = getComputedStyle(target);
  const secondResult = [secondComputed.color, secondComputed.marginLeft].join('|');

  return JSON.stringify({
    firstResult,
    secondResult,
    firstRuleIdentity:
      firstSheet.cssRules[0] === firstStyleRule &&
      firstStyleRule.marker === 'first-style' &&
      firstSheet.cssRules[1].cssRules[0] === firstNestedRule &&
      firstNestedRule.marker === 'first-nested',
    secondRuleIdentity:
      secondSheet.cssRules[0] === secondStyleRule &&
      secondStyleRule.marker === 'second-style' &&
      secondSheet.cssRules[1].cssRules[0] === secondNestedRule &&
      secondNestedRule.marker === 'second-nested',
    firstLength: firstSheet.cssRules.length,
    secondLength: secondSheet.cssRules.length,
    secondCssText: Array.from(secondSheet.cssRules, rule => rule.cssText).join(' '),
    ownerInputsUnchanged:
      firstOwner.textContent === css && secondOwner.textContent === css,
  });
})()
"#,
        )
        .expect("identical inline stylesheets should copy on first CSSOM mutation");

    assert_eq!(
        result,
        r#"{"firstResult":"rgb(4, 5, 6)|7px","secondResult":"rgb(1, 2, 3)|2px","firstRuleIdentity":true,"secondRuleIdentity":true,"firstLength":3,"secondLength":2,"secondCssText":".target { color: rgb(1, 2, 3); } @media screen {\n  .nested { margin-left: 2px; }\n}","ownerInputsUnchanged":true}"#
    );
    assert_eq!(
        crate::live_stylesheet::live_stylesheet_parse_count_for_test(),
        1,
        "identical inline owners in one author-lock domain should share their initial parse"
    );
}

#[test]
fn inline_owner_text_replacement_creates_a_new_sheet_and_detaches_the_old_wrapper() {
    let mut vm = new_storage_test_vm("https://inline-sheet-owner-replacement.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const target = body.appendChild(document.createElement('div'));
  target.id = 'target';
  const style = document.createElement('style');
  style.textContent = '#target { color: rgb(1, 2, 3); }';
  head.appendChild(style);

  const oldSheet = style.sheet;
  const oldRule = oldSheet.cssRules[0];
  oldRule.expando = 'retained';
  style.textContent = '#target { color: rgb(4, 5, 6); }';
  const currentSheet = style.sheet;
  oldSheet.insertRule('.detached-only { color: red; }', oldSheet.cssRules.length);

  return [
    currentSheet !== oldSheet,
    oldSheet.ownerNode === null,
    currentSheet.ownerNode === style,
    oldSheet.cssRules[0] === oldRule,
    oldSheet.cssRules[0].expando,
    oldSheet.cssRules.length,
    currentSheet.cssRules.length,
    style.textContent,
    getComputedStyle(target).color,
  ].join('|');
})()
"#,
        )
        .expect("inline owner text replacement should create a new stylesheet");

    assert_eq!(
        result,
        "true|true|true|true|retained|2|1|#target { color: rgb(4, 5, 6); }|rgb(4, 5, 6)"
    );
}

#[test]
fn inline_cssom_mutation_keeps_owner_text_as_input_only() {
    let mut vm = new_storage_test_vm("https://inline-sheet-owner-input.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const target = body.appendChild(document.createElement('div'));
  target.id = 'target';
  const style = document.createElement('style');
  style.id = 'owner-input';
  style.textContent = '#target { color: rgb(1, 2, 3); }';
  head.appendChild(style);
  const sheet = style.sheet;
  sheet.cssRules[0].style.color = 'rgb(4, 5, 6)';
  sheet.insertRule('#target { background-color: rgb(7, 8, 9); }');
  return [
    style.textContent,
    sheet.cssRules.length,
    getComputedStyle(target).color,
    getComputedStyle(target).backgroundColor,
  ].join('|');
})()
"#,
        )
        .expect("inline CSSOM mutation should preserve owner input text");

    assert_eq!(
        result,
        "#target { color: rgb(1, 2, 3); }|2|rgb(4, 5, 6)|rgb(7, 8, 9)"
    );
    let owner = vm
        .document_runtime
        .dom_host()
        .element_handle_by_id("owner-input")
        .expect("style owner");
    assert_eq!(
        vm._context_host
            .borrow()
            .owner_style_sheet_text(owner)
            .as_deref(),
        Some("#target { color: rgb(1, 2, 3); }")
    );
    assert!(
        vm._context_host
            .borrow()
            .owner_live_stylesheet(owner)
            .is_some_and(|stylesheet| {
                crate::style_engine::StyloStylesheetSource::from_live_stylesheet(&stylesheet)
                    .serialized_css_text()
                    .contains("background-color")
            })
    );
}

#[test]
fn css_rule_list_serialization_uses_rule_owned_pdb_block() {
    let mut vm = new_storage_test_vm("https://rule-list-pdb-serialization.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const style = document.createElement('style');
  style.id = 'pdb-sheet';
  head.appendChild(style);
  const sheet = style.sheet;
  sheet.insertRule('div { place-content: center start; }', 0);

  return sheet.cssRules[0].cssText;
})()
"#,
        )
        .expect("CSS rule list serialization should use rule-owned PDB block");

    assert_eq!(result, "div { place-content: center start; }");

    let style = vm
        .document_runtime
        .dom_host()
        .element_handle_by_id("pdb-sheet")
        .expect("style handle");
    let cached = vm._context_host.borrow().owner_style_sheet_text(style);
    assert_eq!(cached.as_deref(), Some(""));
    assert!(
        vm._context_host
            .borrow()
            .owner_live_stylesheet(style)
            .is_some_and(|stylesheet| {
                crate::style_engine::StyloStylesheetSource::from_live_stylesheet(&stylesheet)
                    .serialized_css_text()
                    .contains("place-content: center start")
            })
    );
}

#[test]
fn css_rule_mixed_style_serialization_uses_rule_owned_pdb_segment() {
    let mut vm = new_storage_test_vm("https://rule-mixed-pdb-serialization.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const style = document.createElement('style');
  style.id = 'mixed-pdb-sheet';
  head.appendChild(style);
  const sheet = style.sheet;
  sheet.insertRule('div {}', 0);
  const rule = sheet.cssRules[0];
  rule.style.cssText = '--before: one; width: 0; --middle: two; height: 0; --after: three;';

  return [
    rule.style.cssText,
    rule.cssText
  ].join('|');
})()
"#,
        )
        .expect("mixed CSS rule style serialization should use rule-owned PDB block segment");

    assert_eq!(
        result,
        "--before: one; width: 0px; --middle: two; height: 0px; --after: three;|div { --before: one; width: 0px; --middle: two; height: 0px; --after: three; }"
    );

    let style = vm
        .document_runtime
        .dom_host()
        .element_handle_by_id("mixed-pdb-sheet")
        .expect("style handle");
    let cached = vm._context_host.borrow().owner_style_sheet_text(style);
    assert_eq!(cached.as_deref(), Some(""));
    assert!(
        vm._context_host
            .borrow()
            .owner_live_stylesheet(style)
            .is_some_and(|stylesheet| {
                crate::style_engine::StyloStylesheetSource::from_live_stylesheet(&stylesheet)
                    .serialized_css_text()
                    .contains("--after: three")
            })
    );
}

#[test]
fn css_rule_mixed_style_mutation_snapshots_side_entries_once() {
    let mut vm = new_storage_test_vm("https://rule-mixed-pdb-snapshot.test/");

    vm.eval(
        r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule('.subject {}');
  const rule = sheet.cssRules[0];
  for (let index = 0; index < 64; index += 1) {
    rule.style.setProperty(`--token-${index}`, String(index));
  }
  rule.style.setProperty('padding', '1px 2px');
  globalThis.__mixedSnapshotRule = rule;
})()
"#,
    )
    .expect("mixed CSS rule snapshot fixture should initialize");

    crate::detached_css_style::reset_raw_style_entries_snapshot_count_for_test();
    vm.eval("globalThis.__mixedSnapshotRule.style.setProperty('margin-left', '3px')")
        .expect("mixed CSS rule mutation should evaluate");
    let snapshot_count = crate::detached_css_style::raw_style_entries_snapshot_count_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const rule = globalThis.__mixedSnapshotRule;
  return [
    rule.style.getPropertyValue('--token-63'),
    rule.style.getPropertyValue('padding'),
    rule.style.getPropertyValue('margin-left'),
    rule.cssText.includes('--token-63: 63;')
  ].join('|');
})()
"#,
        )
        .expect("mixed CSS rule mutation should evaluate");

    assert_eq!(result, "63|1px 2px|3px|true");
    assert!(
        snapshot_count <= 3,
        "a mixed CSS rule mutation must reuse one side-entry snapshot per serialization; got {snapshot_count} snapshots"
    );
}

#[test]
fn css_style_declaration_pdb_shorthand_removal_clears_materialized_longhands() {
    let mut vm = new_storage_test_vm("https://style-pdb-shorthand-removal.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  live.setProperty('place-content', 'center start', 'important');
  const liveRemoved = live.removeProperty('place-content');
  const liveRemove = [
    liveRemoved,
    live.length,
    live.getPropertyValue('align-content'),
    live.getPropertyPriority('align-content'),
    live.cssText
  ].join(',');

  live.setProperty('overflow', 'hidden visible', 'important');
  live.setProperty('overflow', '');
  const liveSetEmpty = [
    live.length,
    live.getPropertyValue('overflow'),
    live.getPropertyValue('overflow-x'),
    live.cssText
  ].join(',');

  live.setProperty('place-content', 'center start', 'important');
  live.placeContent = '';
  const liveNamedEmpty = [
    live.length,
    live.getPropertyValue('place-content'),
    live.getPropertyValue('align-content'),
    live.cssText
  ].join(',');

  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = doc.createElement('div').style;
  detached.setProperty('overflow', 'hidden visible', 'important');
  const detachedRemoved = detached.removeProperty('overflow');
  const detachedRemove = [
    detachedRemoved,
    detached.length,
    detached.getPropertyValue('overflow-x'),
    detached.getPropertyPriority('overflow-y'),
    detached.cssText
  ].join(',');

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0];
  rule.style.setProperty('place-content', 'center start', 'important');
  const ruleRemoved = rule.style.removeProperty('place-content');
  const ruleRemove = [
    ruleRemoved,
    rule.style.length,
    rule.style.cssText,
    rule.cssText.includes('place-content')
  ].join(',');

  const nestedSheet = new CSSStyleSheet();
  nestedSheet.replaceSync('.parent { color: red; .child { color: blue; } font-size: 12px; }');
  const nestedRule = nestedSheet.cssRules[0];
  const nested = nestedRule.cssRules[1];
  nested.style.setProperty('overflow', 'hidden visible', 'important');
  const nestedRemoved = nested.style.removeProperty('overflow');
  const nestedRemove = [
    nestedRemoved,
    nested.style.getPropertyValue('overflow'),
    nested.style.getPropertyValue('overflow-x'),
    nested.style.cssText,
    nested.cssText,
    nestedRule.cssText.includes('overflow')
  ].join(',');

  return [liveRemove, liveSetEmpty, liveNamedEmpty, detachedRemove, ruleRemove, nestedRemove].join('|');
})()
"#,
        )
        .expect("PDB shorthand removal should clear materialized longhands");

    assert_eq!(
        result,
        "center start,0,,,|0,,,|0,,,|hidden visible,0,,,|center start,0,,false|hidden visible,,,font-size: 12px;,font-size: 12px;,false"
    );
}

#[test]
fn overflow_overlay_uses_pdb_supplemental_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://overflow-overlay-pdb.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index)).join(',');
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  function check(style, label) {
    style.setProperty('overflow', 'overlay hidden', 'important');
    eq(`${label}-length`, String(style.length), '2');
    eq(`${label}-names`, names(style), 'overflow-x,overflow-y');
    eq(`${label}-overflow`, style.getPropertyValue('overflow'), 'overlay hidden');
    eq(`${label}-x`, style.getPropertyValue('overflow-x'), 'overlay');
    eq(`${label}-y`, style.getPropertyValue('overflow-y'), 'hidden');
    eq(`${label}-priority`, style.getPropertyPriority('overflow'), 'important');
    eq(`${label}-x-priority`, style.getPropertyPriority('overflow-x'), 'important');
    eq(`${label}-cssText`, style.cssText, 'overflow: overlay hidden !important;');
  }

  check(document.createElement('div').style, 'inline');

  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  check(doc.createElement('div').style, 'detached');

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0];
  check(rule.style, 'rule');
  eq('rule-cssText', rule.cssText, 'div { overflow: overlay hidden !important; }');

  return failures.length ? failures.slice(0, 8).join('|') : 'PASS';
})()
"#,
        )
        .expect("overflow overlay PDB supplemental surface should evaluate");

    assert_eq!(result, "PASS");
}

#[test]
fn live_inline_style_preserves_outline_color_invert() {
    let mut vm = new_storage_test_vm("https://outline-color-invert.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  target.style.outlineColor = 'invert';
  const propertySetter = [target.style.outlineColor, target.style.cssText].join('|');
  target.style.cssText = 'color: invert; outline-color: invert;';
  const cssTextSetter = [
    target.style.getPropertyValue('color'),
    target.style.getPropertyValue('outline-color'),
    target.style.cssText
  ].join('|');
  return `${propertySetter}|${cssTextSetter}`;
})()
"#,
        )
        .expect("inline outline-color invert should serialize");

    assert_eq!(
        result,
        "invert|outline-color: invert;||invert|outline-color: invert;"
    );
}
#[test]
fn live_inline_style_serializes_border_shorthands() {
    let mut vm = new_storage_test_vm("https://border-shorthand-serialization.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  const cases = [
    'border: 1px; border-top: 1px;',
    'border-top: 1px; border-right: 1px; border-bottom: 1px; border-left: 1px; border-image: none;',
    'border: 1px; border-top-color: red;'
  ];
  return cases.map((cssText) => {
    target.style.cssText = cssText;
    return target.style.cssText;
  }).join('|');
})()
"#,
        )
        .expect("inline border shorthands should serialize");

    assert_eq!(
        result,
        "border: 1px;|border: 1px;|border-width: 1px; border-style: none; border-color: red currentcolor currentcolor; border-image: none;"
    );
}
#[test]
fn live_inline_style_important_shorthand_overrides_later_normal_longhand() {
    let mut vm = new_storage_test_vm("https://important-shorthand-cascade.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  target.style.cssText = 'padding: 10px !important; padding-left: 20px;';
  const importantShorthand = target.style.getPropertyValue('padding-left');
  target.style.cssText = 'margin-left: 2px !important; margin: 4px;';
  const importantLonghand = target.style.getPropertyValue('margin-left');
  target.style.cssText = 'padding-left: 2px; padding: 4px;';
  const laterNormalShorthand = target.style.getPropertyValue('padding-left');
  return [importantShorthand, importantLonghand, laterNormalShorthand].join('|');
})()
"#,
        )
        .expect("inline shorthand cascade should evaluate");

    assert_eq!(result, "10px|2px|4px");
}
#[test]
fn detached_css_style_item_uses_webidl_index_conversion() {
    let mut vm = new_storage_test_vm("https://detached-style-item-conversion.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const style = doc.createElement('div').style;
  style.cssText = 'color: red; background-color: blue;';
  function probe(callback) {
    try {
      return callback();
    } catch (error) {
      return 'throw:' + error.name;
    }
  }
  return [
    probe(() => style.item()),
    probe(() => style.item(undefined)),
    probe(() => style.item(null)),
    probe(() => style.item(NaN)),
    probe(() => style.item('1')),
    probe(() => style.item(-1)),
    probe(() => style.item(99)),
    probe(() => style.item(Symbol()))
  ].join('|');
})()
"#,
        )
        .expect("detached CSSStyleDeclaration item conversion should evaluate");

    assert_eq!(
        result,
        "throw:TypeError|color|color|color|background-color|||throw:TypeError"
    );
}
#[test]
fn detached_css_style_methods_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://detached-style-webidl-args.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const style = doc.createElement('div').style;
  function probe(callback) {
    try {
      return callback();
    } catch (error) {
      return 'throw:' + error.name;
    }
  }
  style.setProperty('color', 'red');
  const removed = style.setProperty('color', null);
  style.setProperty('margin', '1px', null);
  style.setProperty('display', 'none', 'IMPORTANT');
  return [
    removed,
    style.getPropertyValue('color'),
    style.getPropertyValue('margin'),
    style.getPropertyPriority('margin'),
    style.getPropertyPriority('display'),
    probe(() => style.setProperty('opacity', '0.5', Symbol())),
    probe(() => style.getPropertyValue(Symbol())),
    style.getPropertyValue('opacity')
  ].join('|');
})()
"#,
        )
        .expect("detached CSSStyleDeclaration WebIDL args should evaluate");

    assert_eq!(result, "||1px||important|throw:TypeError|throw:TypeError|");
}

#[test]
fn css_style_declaration_methods_reject_non_style_receivers() {
    let mut vm = new_storage_test_vm("https://css-style-receiver-validation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const proto = Object.getPrototypeOf(document.createElement('div').style);
  function probe(callback) {
    try {
      return callback();
    } catch (error) {
      return 'throw:' + error.name;
    }
  }
  return [
    probe(() => proto.getPropertyValue.call({}, 'color')),
    probe(() => proto.removeProperty.call({}, 'color')),
    probe(() => proto.getPropertyPriority.call({}, 'color')),
    probe(() => proto.item.call({}, 0))
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleDeclaration receiver validation should evaluate");

    assert_eq!(
        result,
        "throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError"
    );
}

#[test]
fn css_style_set_property_reappends_existing_declaration_order() {
    let mut vm = new_storage_test_vm("https://css-style-set-property-order.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const element = document.createElement('div');
  element.setAttribute('style', [
    'padding-top: 0px;',
    'padding-right: 1px;',
    'padding-bottom: 2px;',
    'padding-left: 3px;',
    'padding-block-start: 4px;',
    'padding-block-end: 5px;',
    'padding-inline-start: 6px;',
    'padding-inline-end: 7px;'
  ].join(' '));
  document.appendChild(element);
  const style = element.style;
  style.setProperty('padding-top', '0px');
  const afterLonghand = Array.from({ length: style.length }, (_, index) => style.item(index)).join(',');
  style.setProperty('padding', '0px 1px 2px 3px');
  const afterShorthand = Array.from({ length: style.length }, (_, index) => style.item(index)).join(',');

  const detached = new DOMParser().parseFromString('<html></html>', 'text/html')
    .createElement('div').style;
  detached.setProperty('color', 'red');
  detached.setProperty('opacity', '0.5');
  detached.setProperty('color', 'blue');
  const detachedOrder = Array.from({ length: detached.length }, (_, index) => detached.item(index)).join(',');

  return [afterLonghand, afterShorthand, detachedOrder].join('|');
})()
"#,
        )
        .expect("CSSStyleDeclaration setProperty order should evaluate");

    assert_eq!(
        result,
        "padding-right,padding-bottom,padding-left,padding-block-start,padding-block-end,padding-inline-start,padding-inline-end,padding-top|padding-block-start,padding-block-end,padding-inline-start,padding-inline-end,padding-top,padding-right,padding-bottom,padding-left|opacity,color"
    );
}

#[test]
fn css_style_declaration_rejects_invalid_animation_times() {
    let mut vm = new_storage_test_vm("https://css-style-animation-invalid.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  live.animationDelay = '0';
  live.setProperty('animation-duration', '-3s');

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0].style;
  rule.animationDelay = 'infinite';
  rule.setProperty('animation-duration', '1s 2s');

  return [
    live.getPropertyValue('animation-delay'),
    live.getPropertyValue('animation-duration'),
    rule.getPropertyValue('animation-delay'),
    rule.getPropertyValue('animation-duration')
  ].join('|');
})()
"#,
        )
        .expect("invalid animation CSSOM values should evaluate");

    assert_eq!(result, "|||");
}

#[test]
fn css_style_declaration_validates_env_function_syntax() {
    let mut vm = new_storage_test_vm("https://css-style-env-syntax.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  live.width = 'env(safe-area-inset-top, )';
  const liveValid = live.getPropertyValue('width');
  live.width = 'env(safe-area-inset-top ())';
  const liveAfterInvalid = live.getPropertyValue('width');
  live.top = 'env(test 0 1, green)';
  const liveIndexed = live.getPropertyValue('top');
  live.top = 'env(test1 test2, green)';
  const liveAfterInvalidIndex = live.getPropertyValue('top');

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0].style;
  rule.setProperty('width', 'env(safe-area-inset-top,)');
  const ruleValid = rule.getPropertyValue('width');
  rule.setProperty('width', 'env(safe-area-inset-top(),)');
  const ruleAfterInvalid = rule.getPropertyValue('width');

  return [
    liveValid,
    liveAfterInvalid,
    liveIndexed,
    liveAfterInvalidIndex,
    ruleValid,
    ruleAfterInvalid,
    CSS.supports('background', 'env(test)'),
    CSS.supports('background', 'env(test, )'),
    CSS.supports('background', 'env()'),
    CSS.supports('top', 'env(test 0.1, green)')
  ].join('|');
})()
"#,
        )
        .expect("env() CSSOM syntax validation should evaluate");

    assert_eq!(
        result,
        "env(safe-area-inset-top, )|env(safe-area-inset-top, )|env(test 0 1, green)|env(test 0 1, green)|env(safe-area-inset-top,)|env(safe-area-inset-top,)|true|true|false|false"
    );
}

#[test]
fn css_style_declaration_supports_compositing_longhands() {
    let mut vm = new_storage_test_vm("https://css-style-compositing-longhands.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  live.backgroundBlendMode = 'normal, luminosity';
  live.mixBlendMode = 'multiply';
  live.isolation = 'isolate';
  live.mixBlendMode = 'normal, luminosity';
  live.isolation = 'none';

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0].style;
  rule.setProperty('background-blend-mode', 'screen, overlay');
  rule.setProperty('mix-blend-mode', 'screen');
  rule.setProperty('isolation', 'auto');
  rule.setProperty('background-blend-mode', 'normal luminosity');

  return [
    CSS.supports('background-blend-mode', 'normal, luminosity'),
    CSS.supports('background-blend-mode', 'normal luminosity'),
    CSS.supports('mix-blend-mode', 'multiply'),
    CSS.supports('mix-blend-mode', 'normal, luminosity'),
    CSS.supports('isolation', 'isolate'),
    CSS.supports('isolation', 'auto isolate'),
    live.getPropertyValue('background-blend-mode'),
    live.getPropertyValue('mix-blend-mode'),
    live.getPropertyValue('isolation'),
    rule.getPropertyValue('background-blend-mode'),
    rule.getPropertyValue('mix-blend-mode'),
    rule.getPropertyValue('isolation')
  ].join('|');
})()
"#,
        )
        .expect("compositing CSSOM longhands should evaluate");

    assert_eq!(
        result,
        "true|false|true|false|true|false|normal, luminosity|multiply|isolate|screen, overlay|screen|auto"
    );
}

#[test]
fn css_style_declaration_color_longhands_use_pdb_entries() {
    let mut vm = new_storage_test_vm("https://css-style-color-pdb-longhands.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  live.accentColor = 'rgb(0 128 0 / 50%)';
  live.caretColor = 'auto';
  live.setProperty('caret-color', 'rgb(clamp(10, none, 20) 0 0)');

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0].style;
  rule.accentColor = 'rgb(0 128 0 / 50%)';
  rule.setProperty('accent-color', rule.accentColor, 'important');
  rule.caretColor = 'auto';
  rule.setProperty('accent-color', 'rgb(clamp(10, none, 20) 0 0)');

  return [
    CSS.supports('accent-color', 'auto'),
    CSS.supports('accent-color', 'rgb(0 128 0 / 50%)'),
    CSS.supports('accent-color', 'rgb(clamp(10, none, 20) 0 0)'),
    CSS.supports('caret-color', 'auto'),
    'accentColor' in live,
    'caretColor' in live,
    live.length,
    live.item(0),
    live.item(1),
    live.getPropertyValue('accent-color'),
    live.accentColor,
    live.getPropertyValue('caret-color'),
    live.caretColor,
    live.cssText.includes('accent-color: rgba(0, 128, 0, 0.5);'),
    live.cssText.includes('caret-color: auto;'),
    rule.length,
    rule.item(0),
    rule.item(1),
    rule.getPropertyValue('accent-color'),
    rule.accentColor,
    rule.getPropertyPriority('accent-color'),
    rule.getPropertyValue('caret-color'),
    rule.caretColor,
    sheet.cssRules[0].cssText.includes('accent-color: rgba(0, 128, 0, 0.5) !important;'),
    sheet.cssRules[0].cssText.includes('caret-color: auto;')
  ].join('|');
})()
"#,
        )
        .expect("color CSSOM longhands should evaluate");

    assert_eq!(
        result,
        "true|true|false|true|true|true|2|accent-color|caret-color|rgba(0, 128, 0, 0.5)|rgba(0, 128, 0, 0.5)|auto|auto|true|true|2|accent-color|caret-color|rgba(0, 128, 0, 0.5)|rgba(0, 128, 0, 0.5)|important|auto|auto|true|true"
    );
}

#[test]
fn css_style_declaration_supports_color_adjust_longhands() {
    let mut vm = new_storage_test_vm("https://css-style-color-adjust-longhands.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  live['color-scheme'] = 'only light';
  live.colorAdjust = 'exact';
  live.forcedColorAdjust = 'preserve-parent-color';
  live.colorScheme = 'light inherit';
  live.printColorAdjust = 'bad';
  live.forcedColorAdjust = 'preserve parent color';

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0].style;
  rule.setProperty('color-scheme', 'only none');
  rule.setProperty('color-adjust', 'economy');
  rule.setProperty('forced-color-adjust', 'none');
  rule.setProperty('color-scheme', 'default');
  rule.setProperty('print-color-adjust', 'economy exact');

  return [
    CSS.supports('color-scheme', 'only light dark'),
    CSS.supports('color-scheme', 'light inherit'),
    CSS.supports('color-scheme', 'default'),
    CSS.supports('color-adjust', 'exact'),
    CSS.supports('print-color-adjust', 'economy exact'),
    CSS.supports('forced-color-adjust', 'preserve-parent-color'),
    CSS.supports('forced-color-adjust', 'preserve parent color'),
    'colorAdjust' in live,
    'color-adjust' in live,
    live.getPropertyValue('color-scheme'),
    live.getPropertyValue('color-adjust'),
    live.getPropertyValue('print-color-adjust'),
    live.getPropertyValue('forced-color-adjust'),
    rule.getPropertyValue('color-scheme'),
    rule.getPropertyValue('color-adjust'),
    rule.getPropertyValue('print-color-adjust'),
    rule.getPropertyValue('forced-color-adjust')
  ].join('|');
})()
"#,
        )
        .expect("color-adjust CSSOM longhands should evaluate");

    assert_eq!(
        result,
        "true|false|false|true|false|true|false|true|true|light only|exact|exact|preserve-parent-color|none only|economy|economy|none"
    );
}

#[test]
fn css_style_declaration_supports_scrollbar_longhands() {
    let mut vm = new_storage_test_vm("https://css-style-scrollbar-longhands.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  live.scrollbarColor = 'red green';
  live.scrollbarWidth = 'thin';
  live.scrollbarColor = '#FF0000 #00FF00';
  live.scrollbarWidth = 'auto none';

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0].style;
  rule.setProperty('scrollbar-color', 'currentcolor currentcolor');
  rule.setProperty('scrollbar-width', 'none');
  rule.setProperty('scrollbar-color', 'auto auto');
  rule.setProperty('scrollbar-width', '12px');

  return [
    CSS.supports('scrollbar-color', 'auto'),
    CSS.supports('scrollbar-color', 'red green'),
    CSS.supports('scrollbar-color', '#FF0000 #00FF00'),
    CSS.supports('scrollbar-color', 'rgb(bad) green'),
    CSS.supports('scrollbar-color', 'red'),
    CSS.supports('scrollbar-color', 'auto currentcolor'),
    CSS.supports('scrollbar-width', 'thin'),
    CSS.supports('scrollbar-width', 'auto none'),
    'scrollbarColor' in live,
    'scrollbar-color' in live,
    live.getPropertyValue('scrollbar-color'),
    live.getPropertyValue('scrollbar-width'),
    rule.getPropertyValue('scrollbar-color'),
    rule.getPropertyValue('scrollbar-width')
  ].join('|');
})()
"#,
        )
        .expect("scrollbar CSSOM longhands should evaluate");

    assert_eq!(
        result,
        "true|true|true|false|false|false|true|false|true|true|rgb(255, 0, 0) rgb(0, 255, 0)|thin|currentcolor currentcolor|none"
    );
}

#[test]
fn css_style_declaration_supports_text_size_adjust() {
    let mut vm = new_storage_test_vm("https://css-style-text-size-adjust.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  live.textSizeAdjust = 'auto';
  live.textSizeAdjust = 'calc(10% + 5%)';
  live.textSizeAdjust = '-100%';
  const liveAfterInvalid = live.getPropertyValue('text-size-adjust');
  live.textSizeAdjust = 'initial';
  const liveInitial = live.getPropertyValue('text-size-adjust');

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0].style;
  rule.setProperty('text-size-adjust', 'none');
  rule.setProperty('text-size-adjust', 'calc(10% * sibling-index())');
  rule.setProperty('text-size-adjust', '10px');

  return [
    CSS.supports('text-size-adjust', 'auto'),
    CSS.supports('text-size-adjust', 'none'),
    CSS.supports('text-size-adjust', '200%'),
    CSS.supports('text-size-adjust', 'calc(10% + 5%)'),
    CSS.supports('text-size-adjust', 'calc(10% * sibling-index())'),
    CSS.supports('text-size-adjust', '-100%'),
    CSS.supports('text-size-adjust', '10px'),
    'textSizeAdjust' in live,
    'text-size-adjust' in live,
    liveAfterInvalid,
    liveInitial,
    rule.getPropertyValue('text-size-adjust')
  ].join('|');
})()
"#,
        )
        .expect("text-size-adjust CSSOM longhand should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|true|false|false|true|true|calc(15%)|initial|calc(10% * sibling-index())"
    );
}

#[test]
fn css_style_declaration_supports_link_parameters() {
    let mut vm = new_storage_test_vm("https://css-style-link-parameters.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  live.linkParameters = 'none';
  live.linkParameters = 'param(--a, orange), param(--b, blue)';
  live.linkParameters = 'param(--a red)';
  const liveAfterInvalid = live.getPropertyValue('link-parameters');
  live.linkParameters = 'param(--a, )';
  const liveEmptyFallback = live.getPropertyValue('link-parameters');
  live.linkParameters = 'param(--a';
  const liveUnclosedFunction = live.getPropertyValue('link-parameters');
  live.linkParameters = 'initial';
  const liveInitial = live.getPropertyValue('link-parameters');

  const sheet = new CSSStyleSheet();
  sheet.insertRule('a {}');
  const rule = sheet.cssRules[0].style;
  rule.setProperty('link-parameters', 'param(--)');
  rule.setProperty('link-parameters', 'param(--, --)');
  rule.setProperty('link-parameters', 'param(-a)');

  return [
    CSS.supports('link-parameters', 'none'),
    CSS.supports('link-parameters', 'param(--a, orange)'),
    CSS.supports('link-parameters', 'param(--a, orange), param(--b, blue)'),
    CSS.supports('link-parameters', 'param(--a, )'),
    CSS.supports('link-parameters', 'param(--a)'),
    CSS.supports('link-parameters', 'param(--a'),
    CSS.supports('link-parameters', 'param(--)'),
    CSS.supports('link-parameters', 'param(--, --)'),
    CSS.supports('link-parameters', 'param(-a)'),
    CSS.supports('link-parameters', 'param(--a red)'),
    CSS.supports('link-parameters', 'param(--a, red) param(--b, blue)'),
    'linkParameters' in live,
    'link-parameters' in live,
    liveAfterInvalid,
    liveEmptyFallback,
    liveUnclosedFunction,
    liveInitial,
    rule.getPropertyValue('link-parameters')
  ].join('|');
})()
"#,
        )
        .expect("link-parameters CSSOM longhand should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|true|true|true|true|false|false|false|true|true|param(--a, orange), param(--b, blue)|param(--a, )|param(--a)|initial|param(--, --)"
    );
}

#[test]
fn css_style_declaration_parses_content_and_bookmark_properties() {
    let mut vm = new_storage_test_vm("https://css-style-content.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  live.content = 'counter(counter-name, dECiMaL)';
  const decimalCounter = live.getPropertyValue('content');
  live.content = 'counter(counter-name, DECIMAL) / "alt text"';
  const decimalCounterWithAlt = live.getPropertyValue('content');
  live.content = 'attr()';
  const afterInvalidAttr = live.getPropertyValue('content');
  live.content = 'open-quote / url("https://www.example.com/picture.svg")';
  const afterInvalidAlt = live.getPropertyValue('content');
  live.content = '"hello" "world"';
  const stringList = live.getPropertyValue('content');

  live.bookmarkLevel = '1';
  live.bookmarkLevel = '0';
  const bookmarkLevelAfterInvalid = live.getPropertyValue('bookmark-level');
  live.bookmarkState = 'closed';
  live.bookmarkState = 'none';
  const bookmarkStateAfterInvalid = live.getPropertyValue('bookmark-state');

  return [
    CSS.supports('content', 'counter(counter-name, decimal)'),
    CSS.supports('content', 'attr()'),
    CSS.supports('content', 'open-quote / url("https://www.example.com/picture.svg")'),
    CSS.supports('quotes', 'none'),
    CSS.supports('bookmark-level', '1'),
    CSS.supports('bookmark-level', '0'),
    CSS.supports('bookmark-state', 'closed'),
    CSS.supports('bookmark-state', 'none'),
    'content' in live,
    'quotes' in live,
    'bookmarkLevel' in live,
    'bookmarkState' in live,
    decimalCounter,
    decimalCounterWithAlt,
    afterInvalidAttr,
    afterInvalidAlt,
    stringList,
    bookmarkLevelAfterInvalid,
    bookmarkStateAfterInvalid
  ].join('|');
})()
"#,
        )
        .expect("content CSSOM longhands should evaluate");

    assert_eq!(
        result,
        "true|false|false|true|true|false|true|false|true|true|true|true|counter(counter-name)|counter(counter-name) / \"alt text\"|counter(counter-name) / \"alt text\"|counter(counter-name) / \"alt text\"|\"hello\" \"world\"|1|closed"
    );
}

#[test]
fn css_style_declaration_supports_will_change() {
    let mut vm = new_storage_test_vm("https://css-style-will-change.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  live.willChange = 'scroll-position, TRANSFORM';
  const liveValid = live.getPropertyValue('will-change');
  live.willChange = 'auto, transform';
  const liveAfterInvalidAutoList = live.getPropertyValue('will-change');
  live.willChange = 'will-change';
  const liveAfterInvalidReserved = live.getPropertyValue('will-change');

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0].style;
  rule.setProperty('will-change', 'Not-A-Property, --var');
  const ruleValid = rule.getPropertyValue('will-change');
  rule.setProperty('will-change', 'transform, all');
  const ruleAfterInvalid = rule.getPropertyValue('will-change');

  return [
    CSS.supports('will-change', 'auto'),
    CSS.supports('will-change', 'scroll-position, transform'),
    CSS.supports('will-change', 'Not-A-Property, --var'),
    CSS.supports('will-change', 'auto, transform'),
    CSS.supports('will-change', 'transform, all'),
    'willChange' in live,
    'will-change' in live,
    liveValid,
    liveAfterInvalidAutoList,
    liveAfterInvalidReserved,
    ruleValid,
    ruleAfterInvalid
  ].join('|');
})()
"#,
        )
        .expect("will-change CSSOM longhand should evaluate");

    assert_eq!(
        result,
        "true|true|true|false|false|true|true|scroll-position, TRANSFORM|scroll-position, TRANSFORM|scroll-position, TRANSFORM|Not-A-Property, --var|Not-A-Property, --var"
    );
}

#[test]
fn css_style_declaration_supports_zoom() {
    let mut vm = new_storage_test_vm("https://css-style-zoom.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  live.zoom = 'normal';
  const liveNormal = live.getPropertyValue('zoom');
  live.zoom = '100%';
  const livePercent = live.getPropertyValue('zoom');
  live.zoom = '-1';
  const liveAfterInvalidNegative = live.getPropertyValue('zoom');

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0].style;
  rule.setProperty('zoom', '0%');
  const ruleZeroPercent = rule.getPropertyValue('zoom');
  rule.setProperty('zoom', 'auto');
  const ruleAfterInvalidAuto = rule.getPropertyValue('zoom');

  return [
    CSS.supports('zoom', 'normal'),
    CSS.supports('zoom', '1.5'),
    CSS.supports('zoom', '150%'),
    CSS.supports('zoom', 'calc(sign(1em - 1px) * 2%)'),
    CSS.supports('zoom', 'auto'),
    CSS.supports('zoom', '-100%'),
    'zoom' in live,
    liveNormal,
    livePercent,
    liveAfterInvalidNegative,
    ruleZeroPercent,
    ruleAfterInvalidAuto
  ].join('|');
})()
"#,
        )
        .expect("zoom CSSOM longhand should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|false|false|true|normal|100%|100%|0%|0%"
    );
}

#[test]
fn css_style_declaration_expands_overscroll_behavior_shorthand() {
    let mut vm = new_storage_test_vm("https://css-style-overscroll-shorthand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  live.overscrollBehavior = 'contain none';
  const livePair = [
    live.overscrollBehavior,
    live.getPropertyValue('overscroll-behavior'),
    live.overscrollBehaviorX,
    live.overscrollBehaviorY,
    live.cssText
  ].join(',');
  live.overscrollBehavior = 'chain chain';
  const liveCompressed = [
    live.overscrollBehavior,
    live.overscrollBehaviorX,
    live.overscrollBehaviorY,
    live.cssText
  ].join(',');
  live.overscrollBehavior = 'bad';
  const liveInvalid = live.overscrollBehavior;
  live.overscrollBehavior = '';
  const liveCleared = [
    live.overscrollBehavior,
    live.overscrollBehaviorX,
    live.overscrollBehaviorY,
    live.length
  ].join(',');
  live.overscrollBehaviorBlock = 'contain';
  live.overscrollBehaviorInline = 'none';
  const liveLogical = [
    live.overscrollBehaviorBlock,
    live.getPropertyValue('overscroll-behavior-block'),
    live.overscrollBehaviorInline,
    live.getPropertyValue('overscroll-behavior-inline'),
    live.cssText
  ].join(',');
  live.overscrollBehaviorBlock = 'bad';
  const liveLogicalInvalid = live.overscrollBehaviorBlock;
  live.removeProperty('overscroll-behavior-block');
  const liveLogicalRemoved = [
    live.overscrollBehaviorBlock,
    live.overscrollBehaviorInline,
    live.cssText
  ].join(',');

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0].style;
  rule.setProperty('overscroll-behavior', 'chain auto');
  const rulePair = [
    rule.overscrollBehavior,
    rule.getPropertyValue('overscroll-behavior'),
    rule.overscrollBehaviorX,
    rule.overscrollBehaviorY,
    rule.cssText
  ].join(',');
  rule.overscrollBehavior = 'contain contain';
  const ruleCompressed = [
    rule.overscrollBehavior,
    rule.overscrollBehaviorX,
    rule.overscrollBehaviorY,
    rule.cssText
  ].join(',');
  rule.removeProperty('overscroll-behavior');
  const ruleCleared = [
    rule.overscrollBehavior,
    rule.overscrollBehaviorX,
    rule.overscrollBehaviorY,
    rule.length
  ].join(',');
  rule.overscrollBehaviorBlock = 'contain';
  rule.overscrollBehaviorInline = 'none';
  const ruleLogical = [
    rule.overscrollBehaviorBlock,
    rule.getPropertyValue('overscroll-behavior-block'),
    rule.overscrollBehaviorInline,
    rule.getPropertyValue('overscroll-behavior-inline'),
    rule.cssText
  ].join(',');
  rule.removeProperty('overscroll-behavior-inline');
  const ruleLogicalRemoved = [
    rule.overscrollBehaviorBlock,
    rule.overscrollBehaviorInline,
    rule.cssText
  ].join(',');

  return [
    livePair,
    liveCompressed,
    liveInvalid,
    liveCleared,
    liveLogical,
    liveLogicalInvalid,
    liveLogicalRemoved,
    rulePair,
    ruleCompressed,
    ruleCleared,
    ruleLogical,
    ruleLogicalRemoved
  ].join('|');
})()
"#,
        )
        .expect("overscroll-behavior shorthand CSSOM expansion should evaluate");

    assert_eq!(
        result,
        "contain none,contain none,contain,none,overscroll-behavior: contain none;|chain,chain,chain,overscroll-behavior: chain;|chain|,,,0|contain,contain,none,none,overscroll-behavior-block: contain; overscroll-behavior-inline: none;|contain|,none,overscroll-behavior-inline: none;|chain auto,chain auto,chain,auto,overscroll-behavior: chain auto;|contain,contain,contain,overscroll-behavior: contain;|,,,0|contain,contain,none,none,overscroll-behavior-block: contain; overscroll-behavior-inline: none;|contain,,overscroll-behavior-block: contain;"
    );
}

#[test]
fn css_style_declaration_expands_animation_shorthand() {
    let mut vm = new_storage_test_vm("https://css-style-animation-shorthand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  live.animation = 'anim paused both reverse 4 1s -3s cubic-bezier(0, -2, 1, 3)';

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0].style;
  rule.animation = 'anim paused both reverse, 4 1s -3s cubic-bezier(0, -2, 1, 3)';

    return [
    live.animationDuration,
    live.animationTimingFunction,
    live.animationDelay,
    live.animationIterationCount,
    live.animationDirection,
    live.animationFillMode,
    live.animationPlayState,
    live.animationName,
    live.animationTimeline,
    live.animationRangeStart,
    live.animationRangeEnd,
    rule.animationDuration,
    rule.animationTimingFunction,
    rule.animationDelay,
    rule.animationIterationCount,
    rule.animationDirection,
    rule.animationFillMode,
    rule.animationPlayState,
    rule.animationName,
    rule.animationTimeline,
    rule.animationRangeStart,
    rule.animationRangeEnd
  ].join('|');
})()
"#,
        )
        .expect("animation shorthand CSSOM expansion should evaluate");

    assert_eq!(
        result,
        "1s|cubic-bezier(0, -2, 1, 3)|-3s|4|reverse|both|paused|anim|auto|normal|normal|auto, 1s|ease, cubic-bezier(0, -2, 1, 3)|0s, -3s|1, 4|reverse, normal|both, none|paused, running|anim, none|auto|normal|normal"
    );
}

#[test]
fn css_style_declaration_serializes_animation_shorthand_from_longhands() {
    let mut vm = new_storage_test_vm("https://css-style-animation-serialize.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  live.animation = 'anim paused both reverse 4 1s -3s cubic-bezier(0, -2, 1, 3)';

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0].style;
  rule.animation = 'anim paused both reverse, 4 1s -3s cubic-bezier(0, -2, 1, 3)';

  return [
    live.animation,
    live.getPropertyValue('animation'),
    rule.animation,
    rule.getPropertyValue('animation')
  ].join('|');
})()
"#,
        )
        .expect("animation shorthand serialization should evaluate");

    assert_eq!(
        result,
        "1s cubic-bezier(0, -2, 1, 3) -3s 4 reverse both paused anim|1s cubic-bezier(0, -2, 1, 3) -3s 4 reverse both paused anim|reverse both paused anim, 1s cubic-bezier(0, -2, 1, 3) -3s 4|reverse both paused anim, 1s cubic-bezier(0, -2, 1, 3) -3s 4"
    );
}

#[test]
fn css_style_declaration_shorthand_common_serialization_checks() {
    let mut vm = new_storage_test_vm("https://css-style-shorthand-common.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('div').style;

  style.setProperty('animation', 'initial');
  const animationInitial = style.getPropertyValue('animation');
  style.setProperty('animation-duration', 'inherit');
  const animationMixedCssWide = style.getPropertyValue('animation');

  style.cssText = '';
  style.setProperty('animation', 'initial');
  style.removeProperty('animation-timeline');
  const animationMissingResetOnly = style.getPropertyValue('animation');

  style.cssText = '';
  style.setProperty('animation-range', 'initial');
  const animationRangeInitial = style.getPropertyValue('animation-range');
  style.setProperty('animation-range-start', 'inherit');
  const animationRangeMixedCssWide = style.getPropertyValue('animation-range');

  style.cssText = '';
  style.setProperty('transition', 'initial');
  const transitionInitial = style.getPropertyValue('transition');
  style.setProperty('transition-duration', 'initial', 'important');
  const transitionMixedPriority = style.getPropertyValue('transition');

  const element = document.createElement('div');
  element.setAttribute('style', 'animation: initial; animation-duration: inherit;');
  const attributeAnimationMixedCssWide = element.style.getPropertyValue('animation');

  const liveResults = [
    animationInitial,
    animationMixedCssWide,
    animationMissingResetOnly,
    animationRangeInitial,
    animationRangeMixedCssWide,
    transitionInitial,
    transitionMixedPriority,
    attributeAnimationMixedCssWide
  ].join('|');

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0].style;

  rule.setProperty('animation', 'initial');
  const ruleAnimationInitial = rule.getPropertyValue('animation');
  rule.setProperty('animation-duration', 'inherit');
  const ruleAnimationMixedCssWide = rule.getPropertyValue('animation');

  rule.cssText = '';
  rule.setProperty('animation', 'initial');
  rule.removeProperty('animation-timeline');
  const ruleAnimationMissingResetOnly = rule.getPropertyValue('animation');

  rule.cssText = '';
  rule.setProperty('animation-range', 'initial');
  const ruleAnimationRangeInitial = rule.getPropertyValue('animation-range');
  rule.setProperty('animation-range-start', 'inherit');
  const ruleAnimationRangeMixedCssWide = rule.getPropertyValue('animation-range');

  rule.cssText = '';
  rule.setProperty('transition', 'initial');
  const ruleTransitionInitial = rule.getPropertyValue('transition');
  rule.setProperty('transition-duration', 'inherit');
  const ruleTransitionMixedCssWide = rule.getPropertyValue('transition');

  rule.cssText = '';
  rule.setProperty('transition', 'initial');
  rule.setProperty('transition-duration', 'initial', 'important');
  const ruleTransitionMixedPriority = rule.getPropertyValue('transition');

  const ruleResults = [
    ruleAnimationInitial,
    ruleAnimationMixedCssWide,
    ruleAnimationMissingResetOnly,
    ruleAnimationRangeInitial,
    ruleAnimationRangeMixedCssWide,
    ruleTransitionInitial,
    ruleTransitionMixedCssWide,
    ruleTransitionMixedPriority
  ].join('|');

  return [liveResults, ruleResults].join('/');
})()
"#,
        )
        .expect("common shorthand CSSOM serialization checks should evaluate");

    let common_results = "initial|||initial||initial||";
    assert_eq!(result, format!("{common_results}/{common_results}"));
}

#[test]
fn computed_style_initial_values_match_exposed_css_supports_longhands() {
    let mut vm = new_storage_test_vm("https://css-computed-initial-supports.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const element = document.createElement('div');
  (document.body || document.documentElement || document).appendChild(element);
  const { style } = element;
  const computedStyle = getComputedStyle(element);
  const cssProperties = new Set();
  const computedLonghandNames = new Set(
    Array.from({ length: computedStyle.length }, (_, index) => computedStyle.item(index))
  );

  for (let obj = style; obj; obj = Reflect.getPrototypeOf(obj)) {
    for (let name of Object.getOwnPropertyNames(obj)) {
      const property = name.replace(/[A-Z]/g, c => "-" + c.toLowerCase());
      if (CSS.supports(property, "initial")) {
        cssProperties.add(property);
      }
    }
  }

  const cssLonghands = new Set(
    Array.from(cssProperties).filter(property => computedLonghandNames.has(property))
  );

  for (let longhand of cssLonghands) {
    element.style.setProperty(longhand, "initial");
  }

  const bad = [];
  for (let property of cssLonghands) {
    const result = computedStyle.getPropertyValue(property);
    if (!CSS.supports(property, result) && property !== "all") {
      bad.push([property, result]);
    }
  }
  return JSON.stringify(bad);
})()
"#,
        )
        .expect("computed initial CSS.supports surface should evaluate");

    assert_eq!(result, "[]");
}

#[test]
fn css_style_declaration_expands_transition_shorthand() {
    let mut vm = new_storage_test_vm("https://css-style-transition-shorthand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  live.transition = 'allow-discrete display 3s ease-in-out 1s, normal opacity';

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0].style;
  rule.transition = '1s -3s cubic-bezier(0, -2, 1, 3) top';

  return [
    live.transition,
    live.transitionProperty,
    live.transitionDuration,
    live.transitionTimingFunction,
    live.transitionDelay,
    live.transitionBehavior,
    rule.transition,
    rule.transitionProperty,
    rule.transitionDuration,
    rule.transitionTimingFunction,
    rule.transitionDelay,
    rule.transitionBehavior
  ].join('|');
})()
"#,
        )
        .expect("transition shorthand CSSOM expansion should evaluate");

    assert_eq!(
        result,
        "display 3s ease-in-out 1s allow-discrete, opacity|display, opacity|3s, 0s|ease-in-out, ease|1s, 0s|allow-discrete, normal|top 1s cubic-bezier(0, -2, 1, 3) -3s|top|1s|cubic-bezier(0, -2, 1, 3)|-3s|normal"
    );
}

#[test]
fn css_style_transition_property_accepts_css_ident_tokens() {
    let mut vm = new_storage_test_vm("https://css-style-transition-property-ident.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('div').style;
  style.transitionProperty = 'ALL, INVALID, SYNTAX, SRC';
  const upper = style.transitionProperty;
  style.transitionProperty = 'foo\\ bar, --custom-prop, --\\30 0, \\E9';
  const escaped = style.transitionProperty;
  style.transition = 'foo\\ bar 1s';
  const shorthand = [
    style.transition,
    style.transitionProperty,
    style.transitionDuration
  ].join('/');
  style.transitionProperty = 'none, width';
  const invalidNoneList = style.transitionProperty;
  return [upper, escaped, shorthand, invalidNoneList].join('|');
})()
"#,
        )
        .expect("transition-property CSS ident parsing should evaluate");

    assert_eq!(
        result,
        "all, INVALID, SYNTAX, SRC|foo\\ bar, --custom-prop, --00, é|foo\\ bar 1s/foo\\ bar/1s|foo\\ bar"
    );
}

#[test]
fn css_style_declaration_expands_animation_range_shorthand() {
    let mut vm = new_storage_test_vm("https://css-style-animation-range-shorthand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  live.animationRange = 'entry 10% exit 20%';

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0].style;
  rule.animationRange = 'entry, exit';

  return [
    live.animationRange,
    live.animationRangeStart,
    live.animationRangeEnd,
    rule.animationRange,
    rule.animationRangeStart,
    rule.animationRangeEnd,
  ].join('|');
})()
"#,
        )
        .expect("animation-range shorthand CSSOM expansion should evaluate");

    assert_eq!(
        result,
        "entry 10% exit 20%|entry 10%|exit 20%|entry, exit|entry, exit|entry, exit"
    );
}

#[test]
fn css_style_declaration_exposes_animation_reset_longhands_by_kebab_name() {
    let mut vm = new_storage_test_vm("https://css-style-animation-reset-longhands.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('div').style;
  style.animation = 'anim paused both reverse 4 1s -3s cubic-bezier(0, -2, 1, 3)';
  const names = Array.from({ length: style.length }, (_, index) => style.item(index));
  const beforeClear = [
    style['animation-timeline'],
    style['animation-range-start'],
    style['animation-range-end']
  ];
  for (const longhand of [
    'animation-delay',
    'animation-direction',
    'animation-duration',
    'animation-fill-mode',
    'animation-iteration-count',
    'animation-name',
    'animation-play-state',
    'animation-range-end',
    'animation-range-start',
    'animation-timeline',
    'animation-timing-function'
  ]) {
    style[longhand] = '';
  }
  const afterNames = Array.from({ length: style.length }, (_, index) => style.item(index));
  return [
    ...beforeClear,
    names.includes('animation-timeline'),
    names.includes('animation-range-start'),
    names.includes('animation-range-end'),
    style.length,
    afterNames.join(',')
  ].join('|');
})()
"#,
        )
        .expect("animation reset longhands kebab access should evaluate");

    assert_eq!(result, "auto|normal|normal|true|true|true|0|");
}

#[test]
fn css_style_declaration_animation_shorthand_does_not_leave_unrelated_longhands() {
    let mut vm = new_storage_test_vm("https://css-style-animation-shorthand-cleanup.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const longhands = {
    'animation-duration': '1s',
    'animation-timing-function': 'cubic-bezier(0, -2, 1, 3)',
    'animation-delay': '-3s',
    'animation-iteration-count': '4',
    'animation-direction': 'reverse',
    'animation-fill-mode': 'both',
    'animation-play-state': 'paused',
    'animation-name': 'anim',
    'animation-timeline': 'auto',
    'animation-range-start': 'normal',
    'animation-range-end': 'normal',
  };
  const style = document.createElement('div').style;
  style['animation'] = '';
  const expectedLength = style.length;
  style['animation'] = 'anim paused both reverse 4 1s -3s cubic-bezier(0, -2, 1, 3)';
  for (let longhand of Object.keys(longhands).sort()) {
    style[longhand] = '';
  }
  return [
    expectedLength,
    style.length,
    Array.from({ length: style.length }, (_, index) => style.item(index)).join(',')
  ].join('|');
})()
"#,
        )
        .expect("animation shorthand cleanup should evaluate");

    assert_eq!(result, "0|0|");
}

#[test]
fn cssom_animation_timing_function_accepts_css_easing_math() {
    let mut vm = new_storage_test_vm("https://css-easing-cssom.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('div').style;
  const values = [];

  values.push(CSS.supports('animation-timing-function', 'cubic-bezier(calc(-2), calc(0.7 / 2), calc(1.5), calc(0))'));
  values.push(CSS.supports('animation-timing-function', 'cubic-bezier(-0.1, 0.1, 0.5, 0.9)'));
  values.push(CSS.supports('animation-timing-function', 'steps(calc(1), jump-none)'));
  values.push(CSS.supports('animation-timing-function', 'steps(calc(0/0), jump-none)'));
  values.push(CSS.supports('animation-timing-function', 'linear(0, 1)'));
  values.push(CSS.supports('animation-timing-function', 'linear(0 calc(50px - 50%), 0 calc(50em + 50em))'));

  style.animationTimingFunction = 'cubic-bezier(calc(-2), calc(0.7 / 2), calc(1.5), calc(0))';
  values.push(style.animationTimingFunction);

  style.animationTimingFunction = 'cubic-bezier(0, sibling-index(), 1, sign(2em - 20px))';
  values.push(style.animationTimingFunction);

  style.animationTimingFunction = 'steps(calc(-10), start)';
  values.push(style.animationTimingFunction);

  style.animationTimingFunction = 'steps(calc(1), jump-none)';
  values.push(style.animationTimingFunction);

  style.animationTimingFunction = 'linear(0, 1)';
  values.push(style.animationTimingFunction);

  style.animationTimingFunction = 'linear(0 calc(50% - 50%), 0 calc(50% + 50%))';
  values.push(style.animationTimingFunction);

  style.animationTimingFunction = 'linear(calc(0/0), 1)';
  values.push(style.animationTimingFunction);

  return values.join('|');
})()
"#,
        )
        .expect("animation timing CSSOM math should evaluate");

    assert_eq!(
        result,
        "true|false|true|false|true|false|cubic-bezier(calc(-2), calc(0.35), calc(1.5), calc(0))|cubic-bezier(0, sibling-index(), 1, sign(2em - 20px))|steps(calc(-10), start)|steps(calc(1), jump-none)|linear(0, 1)|linear(0 calc(0%), 0 calc(100%))|linear(0 0%, 1 100%)"
    );
}

#[test]
fn css_stylesheet_rejects_invalid_keyframes_names() {
    let mut vm = new_storage_test_vm("https://css-keyframes-name-validation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  const probe = (name) => {
    try {
      sheet.insertRule(`@keyframes ${name} {}`);
      const length = sheet.cssRules.length;
      while (sheet.cssRules.length) {
        sheet.deleteRule(0);
      }
      return length;
    } catch (e) {
      return 'throw';
    }
  };
  return [
    probe('none'),
    probe('initial'),
    probe('revert-rule'),
    probe('default'),
    probe('12foo'),
    probe('one two'),
    probe('""'),
    probe('"none"'),
    probe('"default"'),
    probe('normal')
  ].join('|');
})()
"#,
        )
        .expect("keyframes name validation should evaluate");

    assert_eq!(result, "throw|throw|throw|throw|throw|throw|throw|1|1|1");
}

#[test]
fn css_keyframes_rule_name_setter_serializes_reserved_names_as_strings() {
    let mut vm = new_storage_test_vm("https://css-keyframes-name-setter.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule('@keyframes spin {}');
  const rule = sheet.cssRules[0];
  const probe = (name) => {
    rule.name = name;
    const cssText = rule.cssText;
    sheet.insertRule(cssText, 1);
    sheet.deleteRule(1);
    return `${rule.name}:${cssText.replace(/\s/g, '')}`;
  };
  return [
    probe('default'),
    probe('revert-rule'),
    probe('initial')
  ].join('|');
})()
"#,
        )
        .expect("CSSKeyframesRule name setter serialization should evaluate");

    assert_eq!(
        result,
        "default:@keyframes\"default\"{}|revert-rule:@keyframes\"revert-rule\"{}|initial:@keyframes\"initial\"{}"
    );
}

#[test]
fn detached_css_style_named_properties_sync_with_declaration_store() {
    let mut vm = new_storage_test_vm("https://detached-style-named-properties.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const style = doc.createElement('div').style;
  style.backgroundColor = 'rgb(0 128 0 / 50%)';
  style.marginTop = '4px';
  style.cssFloat = 'left';
  style['border-top'] = '1px solid red';
  const afterNamedSet = [
    style.length,
    style.item(0),
    style.item(1),
    style.item(2),
    style.item(3),
    style.item(4),
    style.item(5),
    style.getPropertyValue('background-color'),
    style.getPropertyValue('margin-top'),
    style.getPropertyValue('float'),
    style.getPropertyValue('border-top'),
    style.backgroundColor,
    style.marginTop,
    style.cssFloat,
    style.borderTop,
    style.cssText
  ].join(',');
  style.setProperty('background-color', 'blue');
  style.cssText = 'padding-left: 2px; z-index: 10;';
  const afterCssText = [
    style.length,
    style.item(0),
    style.item(1),
    style.paddingLeft,
    style.zIndex,
    style.getPropertyValue('padding-left'),
    style.getPropertyValue('z-index'),
    style.backgroundColor
  ].join(',');
  return [afterNamedSet, afterCssText].join('|');
})()
"#,
        )
        .expect("detached CSSStyleDeclaration named properties should sync");

    assert_eq!(
        result,
        "6,background-color,margin-top,float,border-top-width,border-top-style,border-top-color,rgba(0, 128, 0, 0.5),4px,left,1px solid red,rgba(0, 128, 0, 0.5),4px,left,1px solid red,background-color: rgba(0, 128, 0, 0.5); margin-top: 4px; float: left; border-top: 1px solid red;|2,padding-left,z-index,2px,10,2px,10,"
    );
}

#[test]
fn detached_css_text_setter_uses_stylo_declaration_block_semantics() {
    let mut vm = new_storage_test_vm("https://detached-style-pdb-csstext.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const style = doc.createElement('div').style;
  style.cssText = 'color: invalid; color: red !important; padding: 1px 2px; width: 0;';
  const canonical = [
    style.length,
    style.item(0),
    style.item(1),
    style.getPropertyValue('color'),
    style.getPropertyPriority('color'),
    style.getPropertyValue('padding-left'),
    style.getPropertyValue('padding'),
    style.getPropertyValue('width'),
    style.cssText
  ].join('|');

  style.cssText = 'display: block; width: 0; display: flex;';
  const duplicate = [
    style.length,
    Array.from({ length: style.length }, (_, index) => style.item(index)).join(','),
    style.display,
    style.width,
    style.cssText
  ].join('|');

  style.cssText = '--token: value; width: calc(7px * up); -webkit-text-fill-color: red; padding: calc(calc(1px)) 2px;';
  const pdbFirst = [
    style.length,
    Array.from({ length: style.length }, (_, index) => style.item(index)).join(','),
    style.getPropertyValue('--token'),
    style.getPropertyValue('width'),
    style.getPropertyValue('-webkit-text-fill-color'),
    style.getPropertyValue('padding'),
    style.cssText
  ].join('|');

  return [canonical, duplicate, pdbFirst].join('/');
})()
"#,
        )
        .expect("detached CSSStyleDeclaration cssText setter should use Stylo declarations");

    assert_eq!(
        result,
        "6|color|padding-top|red|important|2px|1px 2px|0px|color: red !important; padding: 1px 2px; width: 0px;/2|width,display|flex|0px|width: 0px; display: flex;/6|--token,-webkit-text-fill-color,padding-top,padding-right,padding-bottom,padding-left|value||red|calc(1px) 2px|--token: value; -webkit-text-fill-color: red; padding: calc(1px) 2px;"
    );
}

#[test]
fn inline_css_text_preserves_cssom_shorthand_entry_shape_after_stylo_validation() {
    let mut vm = new_storage_test_vm("https://inline-style-pdb-preserved-shorthand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('div').style;
  style.cssText = 'color: red; background: blue';
  const background = [
    style.length,
    style.item(0),
    style.item(1),
    style.getPropertyValue('background'),
    style.getPropertyValue('background-color'),
    style.cssText
  ].join(',');

  style.cssText = 'background: url(http://example.com/image.png)';
  const url = style.getPropertyValue('background');

  style.cssText = '';
  style.gap = '10px 10px';
  const namedGap = [
    style.length,
    style.item(0),
    style.item(1),
    style.getPropertyValue('gap'),
    style.gap,
    style.cssText
  ].join(',');
  style.cssText = '';
  style.setProperty('gap', '10px 10px');
  const methodGap = [
    style.length,
    style.item(0),
    style.item(1),
    style.getPropertyValue('gap'),
    style.gap,
    style.cssText
  ].join(',');

  style.cssText = '';
  style.rowGap = '567px';
  style.columnGap = '567px';
  style.rowGap = '1234';
  style.setProperty('column-gap', '1234');
  const gapUnitlessRejection = [
    style.length,
    style.item(0),
    style.item(1),
    style.rowGap,
    style.columnGap,
    style.getPropertyValue('row-gap'),
    style.getPropertyValue('column-gap'),
    style.cssText
  ].join(',');

  style.cssText = '';
  style.scrollMarginTop = '0';
  style.scrollPaddingBottom = '0';
  style.columnWidth = '0';
  style.columnRuleWidth = '0';
  style.shapeMargin = '0';
  const structuredCompatSerializers = [
    style.length,
    Array.from({ length: style.length }, (_, index) => style.item(index)).join('/'),
    style.scrollMarginTop,
    style.scrollPaddingBottom,
    style.columnWidth,
    style.columnRuleWidth,
    style.shapeMargin,
    style.getPropertyValue('scroll-margin-top'),
    style.getPropertyValue('scroll-padding-bottom'),
    style.getPropertyValue('column-width'),
    style.getPropertyValue('column-rule-width'),
    style.getPropertyValue('shape-margin'),
    style.cssText
  ].join(',');

  style.cssText = '';
  style.scrollSnapAlign = 'start start';
  const scrollSnapAlign = [
    style.length,
    style.item(0),
    style.scrollSnapAlign,
    style.getPropertyValue('scroll-snap-align'),
    style.cssText
  ].join(',');

  style.cssText = '';
  style.overflowX = 'overlay';
  style.overflowY = 'hidden';
  const overflowOverlay = [
    style.overflow,
    style.overflowX,
    style.overflowY
  ].join(',');

  return [
    background,
    url,
    namedGap,
    methodGap,
    gapUnitlessRejection,
    [
      CSS.supports('scroll-margin-top', '0'),
      CSS.supports('scroll-padding-bottom', '0'),
      CSS.supports('column-width', '0'),
      CSS.supports('column-rule-width', '0'),
      CSS.supports('shape-margin', '0'),
      CSS.supports('scroll-snap-align', 'start start'),
      CSS.supports('scroll-snap-align', 'start invalid'),
      structuredCompatSerializers
    ].join(','),
    scrollSnapAlign,
    overflowOverlay
  ].join('|');
})()
"#,
        )
        .expect("inline preserved shorthand style probe should evaluate");

    assert_eq!(
        result,
        r#"2,color,background,blue,blue,color: red; background: blue;|url("http://example.com/image.png")|2,row-gap,column-gap,10px,10px,gap: 10px;|2,row-gap,column-gap,10px,10px,gap: 10px;|2,row-gap,column-gap,567px,567px,567px,567px,gap: 567px;|true,true,true,true,true,true,false,5,scroll-margin-top/scroll-padding-bottom/column-width/column-rule-width/shape-margin,0px,0px,0px,0px,0px,0px,0px,0px,0px,0px,scroll-margin-top: 0px; scroll-padding-bottom: 0px; column-width: 0px; column-rule-width: 0px; shape-margin: 0px;|1,scroll-snap-align,start,start,scroll-snap-align: start;|overlay hidden,overlay,hidden"#
    );
}

#[test]
fn detached_css_property_writes_use_stylo_declaration_block_semantics() {
    let mut vm = new_storage_test_vm("https://detached-style-pdb-property-writes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const style = doc.createElement('div').style;
  style.width = '10px';
  style.width = 'bad';
  const invalidWidthPreservesOldValue = style.width;
  style.setProperty('width', '0');
  style.backgroundColor = 'not-a-color';
  const invalidColor = style.backgroundColor;
  style.backgroundColor = 'rgb(0 128 0 / 50%)';
  style.setProperty('padding', '1px 2px');
  return [
    invalidWidthPreservesOldValue,
    style.width,
    invalidColor,
    style.backgroundColor,
    style.getPropertyValue('padding-left'),
    style.getPropertyValue('padding'),
    style.cssText
  ].join('|');
})()
"#,
        )
        .expect("detached CSSStyleDeclaration property writes should use Stylo declarations");

    assert_eq!(
        result,
        "10px|0px||rgba(0, 128, 0, 0.5)|2px|1px 2px|width: 0px; background-color: rgba(0, 128, 0, 0.5); padding: 1px 2px;"
    );
}

#[test]
fn detached_css_property_queries_use_stylo_declaration_block_lookup() {
    let mut vm = new_storage_test_vm("https://detached-style-pdb-property-query.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const style = doc.createElement('div').style;
  style.setProperty('overflow', 'hidden visible', 'important');
  return [
    style.length,
    Array.from({ length: style.length }, (_, index) => style.item(index)).join(','),
    style.getPropertyValue('overflow'),
    style.getPropertyPriority('overflow'),
    style.getPropertyValue('overflow-x'),
    style.getPropertyPriority('overflow-x'),
    style.getPropertyValue('overflow-y'),
    style.getPropertyPriority('overflow-y'),
    style.cssText
  ].join('|');
})()
"#,
        )
        .expect("detached CSSStyleDeclaration property queries should use Stylo declarations");

    assert_eq!(
        result,
        "2|overflow-x,overflow-y|hidden visible|important|hidden|important|visible|important|overflow: hidden visible !important;"
    );
}

#[test]
fn detached_css_pdb_backing_preserves_supplemental_side_table_semantics() {
    let mut vm = new_storage_test_vm("https://detached-style-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const style = doc.createElement('div').style;
  style.setProperty('padding', '1px 2px', 'important');
  style.setProperty('--token', 'value');
  style.setProperty('-webkit-text-fill-color', 'red');
  style.marginLeft = '4px';

  const names = Array.from({ length: style.length }, (_, index) => style.item(index));
  const cssText = style.cssText;
  const before = [
    style.length,
    names.slice(0, 4).join('/'),
    names.includes('--token'),
    names.includes('-webkit-text-fill-color'),
    names.at(-1),
    style.getPropertyValue('padding'),
    style.getPropertyPriority('padding'),
    style.getPropertyValue('--token'),
    style.getPropertyValue('-webkit-text-fill-color'),
    style.marginLeft,
    cssText.includes('padding: 1px 2px !important;'),
    cssText.includes('--token: value;'),
    cssText.includes('-webkit-text-fill-color: red;')
  ].join(',');

  const removed = style.removeProperty('padding');
  const afterNames = Array.from({ length: style.length }, (_, index) => style.item(index));
  const after = [
    removed,
    style.length,
    afterNames.includes('padding-left'),
    style.getPropertyValue('padding'),
    style.marginLeft,
    style.getPropertyValue('--token'),
    style.getPropertyValue('-webkit-text-fill-color')
  ].join(',');

  style.cssText = '';
  style.paddingLeft = '1px';
  style.setProperty('all', 'inherit');
  const allAfterPdbProperty = [
    style.paddingLeft,
    style.getPropertyPriority('padding-left')
  ].join('/');

  style.cssText = '';
  style.setProperty('all', 'inherit');
  style.setProperty('padding-left', '1px', 'important');
  const pdbPropertyAfterAll = [
    style.paddingLeft,
    style.getPropertyPriority('padding-left')
  ].join('/');

  return [before, after, allAfterPdbProperty, pdbPropertyAfterAll].join('|');
})()
"#,
        )
        .expect("detached PDB backing should preserve supplemental side table semantics");

    assert_eq!(
        result,
        "7,padding-top/padding-right/padding-bottom/padding-left,true,true,margin-left,1px 2px,important,value,red,4px,true,true,true|1px 2px,3,false,,4px,value,red|inherit/|1px/important"
    );
}

#[test]
fn box_shorthand_cssom_writes_use_pdb_boundary() {
    let mut vm = new_storage_test_vm("https://box-shorthand-cssom-pdb-boundary.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, value) => {
    if (!value) failures.push(`${label}:${value}`);
  };
  const check = (style, label, property, valid, invalid, longhand, longhandValue, shorthandValue) => {
    style[property] = invalid;
    eq(`${label}-${property}-invalid-empty-length`, style.length, 0);
    eq(`${label}-${property}-invalid-empty-own`, Object.prototype.hasOwnProperty.call(style, property), false);
    eq(`${label}-${property}-invalid-empty-query`, style.getPropertyValue(property), '');

    style[property] = valid;
    eq(`${label}-${property}-shorthand`, style.getPropertyValue(property), shorthandValue);
    eq(`${label}-${property}-longhand`, style.getPropertyValue(longhand), longhandValue);
    ok(`${label}-${property}-cssText`, style.cssText.includes(`${property}: ${shorthandValue};`));

    style[property] = invalid;
    eq(`${label}-${property}-invalid-preserves-shorthand`, style.getPropertyValue(property), shorthandValue);
    eq(`${label}-${property}-invalid-preserves-longhand`, style.getPropertyValue(longhand), longhandValue);
    eq(`${label}-${property}-invalid-preserves-own`, Object.prototype.hasOwnProperty.call(style, property), false);
  };

  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = doc.createElement('div').style;
  check(detached, 'detached', 'margin', '1px 2px', 'banana', 'margin-left', '2px', '1px 2px');

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const ruleStyle = sheet.cssRules[0].style;
  check(ruleStyle, 'rule', 'padding', 'calc(calc(12px)) 2px', 'banana', 'padding-top', 'calc(12px)', 'calc(12px) 2px');

  return failures.length ? failures.join('|') : 'PASS';
})()
"#,
        )
        .expect("box shorthand CSSOM writes should use the PDB boundary");

    assert_eq!(result, "PASS");
}

#[test]
fn logical_box_cssom_writes_use_pdb_boundary() {
    let mut vm = new_storage_test_vm("https://logical-box-cssom-pdb-boundary.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, value) => {
    if (!value) failures.push(`${label}:${value}`);
  };
  const check = (style, label, idl, property, valid, invalid, longhand, longhandValue, queryValue) => {
    ok(`${label}-${property}-idl`, idl in style);
    ok(`${label}-${property}-kebab`, property in style);

    style[idl] = invalid;
    eq(`${label}-${property}-invalid-empty-length`, style.length, 0);
    eq(`${label}-${property}-invalid-empty-own`, Object.prototype.hasOwnProperty.call(style, idl), false);
    eq(`${label}-${property}-invalid-empty-query`, style.getPropertyValue(property), '');

    style[idl] = valid;
    eq(`${label}-${property}-query`, style.getPropertyValue(property), queryValue);
    eq(`${label}-${property}-idl-get`, style[idl], queryValue);
    eq(`${label}-${property}-longhand`, style.getPropertyValue(longhand), longhandValue);
    ok(`${label}-${property}-cssText`, style.cssText.includes(`${property}: ${queryValue};`));

    style[idl] = invalid;
    eq(`${label}-${property}-invalid-preserves-query`, style.getPropertyValue(property), queryValue);
    eq(`${label}-${property}-invalid-preserves-longhand`, style.getPropertyValue(longhand), longhandValue);
    eq(`${label}-${property}-invalid-preserves-own`, Object.prototype.hasOwnProperty.call(style, idl), false);
  };

  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  check(
    doc.createElement('div').style,
    'detached-shorthand',
    'marginBlock',
    'margin-block',
    '1px 2px',
    'banana',
    'margin-block-end',
    '2px',
    '1px 2px'
  );
  check(
    doc.createElement('div').style,
    'detached-longhand',
    'paddingBlockStart',
    'padding-block-start',
    'calc(calc(12px))',
    'banana',
    'padding-block-start',
    'calc(12px)',
    'calc(12px)'
  );

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  check(
    sheet.cssRules[0].style,
    'rule-shorthand',
    'marginInline',
    'margin-inline',
    '3px 4px',
    'banana',
    'margin-inline-start',
    '3px',
    '3px 4px'
  );
  const sheet2 = new CSSStyleSheet();
  sheet2.insertRule('div {}');
  check(
    sheet2.cssRules[0].style,
    'rule-longhand',
    'paddingInlineEnd',
    'padding-inline-end',
    'calc(calc(8px))',
    'banana',
    'padding-inline-end',
    'calc(8px)',
    'calc(8px)'
  );

  return failures.length ? failures.join('|') : 'PASS';
})()
"#,
        )
        .expect("logical box CSSOM writes should use the PDB boundary");

    assert_eq!(result, "PASS");
}

#[test]
fn numeric_standard_cssom_writes_use_pdb_boundary() {
    let mut vm = new_storage_test_vm("https://numeric-standard-cssom-pdb-boundary.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, value) => {
    if (!value) failures.push(`${label}:${value}`);
  };
  const cases = [
    ['backgroundSize', 'background-size', '10px 20px', '1px 2px 3px', '10px 20px'],
    ['blockSize', 'block-size', 'calc(calc(12px))', 'banana', 'calc(12px)'],
    ['letterSpacing', 'letter-spacing', 'clamp(1px,2px,3px)', '1px 2px', 'calc(2px)'],
    ['opacity', 'opacity', '0.5', 'banana', '0.5'],
    ['rotate', 'rotate', '45deg', '1px', '45deg'],
    ['scale', 'scale', '2', 'banana', '2'],
    ['tabSize', 'tab-size', '4', '-1', '4'],
    ['textIndent', 'text-indent', 'calc(calc(12px))', 'banana', 'calc(12px)'],
    ['zIndex', 'z-index', '3', '1.5', '3']
  ];

  const check = (style, label, idl, property, valid, invalid, expected) => {
    ok(`${label}-${property}-idl`, idl in style);
    ok(`${label}-${property}-kebab`, property in style);

    style[idl] = invalid;
    eq(`${label}-${property}-invalid-empty-length`, style.length, 0);
    eq(`${label}-${property}-invalid-empty-query`, style.getPropertyValue(property), '');
    eq(`${label}-${property}-invalid-empty-own`, Object.prototype.hasOwnProperty.call(style, idl), false);

    style[idl] = valid;
    eq(`${label}-${property}-query`, style.getPropertyValue(property), expected);
    eq(`${label}-${property}-idl-get`, style[idl], expected);
    ok(`${label}-${property}-cssText`, style.cssText.includes(`${property}: ${expected};`));

    style[idl] = invalid;
    eq(`${label}-${property}-invalid-preserves-query`, style.getPropertyValue(property), expected);
    eq(`${label}-${property}-invalid-preserves-own`, Object.prototype.hasOwnProperty.call(style, idl), false);
  };

  for (const [idl, property, valid, invalid, expected] of cases) {
    const live = document.createElement('div');
    (document.body || document.documentElement || document).appendChild(live);
    check(live.style, 'live', idl, property, valid, invalid, expected);

    const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
    check(doc.createElement('div').style, 'detached', idl, property, valid, invalid, expected);

    const sheet = new CSSStyleSheet();
    sheet.insertRule('div {}');
    check(sheet.cssRules[0].style, 'rule', idl, property, valid, invalid, expected);
  }

  return failures.length ? failures.slice(0, 30).join('|') : 'PASS';
})()
"#,
        )
        .expect("numeric standard CSSOM writes should use the PDB boundary");

    assert_eq!(result, "PASS");
}

#[test]
fn rule_css_entries_expand_unresolved_box_shorthand_before_longhand_mutation() {
    let mut vm = new_storage_test_vm("https://rule-style-unresolved-box-shorthand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const margin = sheet.cssRules[0].style;
  margin.setProperty('margin', 'var(--m)');
  margin.setProperty('margin-top', '10px');
  const setPropertyPath = [
    margin.cssText,
    margin.getPropertyValue('margin'),
    margin.getPropertyValue('margin-right'),
    margin.getPropertyValue('margin-top'),
    Array.from({ length: margin.length }, (_, index) => margin.item(index)).join('/')
  ].join(',');

  const sheet2 = new CSSStyleSheet();
  sheet2.insertRule('div {}');
  const padding = sheet2.cssRules[0].style;
  padding.padding = 'var(--p)';
  padding.paddingLeft = 'calc(calc(1px))';
  const namedSetterPath = [
    padding.cssText,
    padding.getPropertyValue('padding'),
    padding.getPropertyValue('padding-right'),
    padding.paddingLeft,
    Array.from({ length: padding.length }, (_, index) => padding.item(index)).join('/')
  ].join(',');

  return [setPropertyPath, namedSetterPath].join('|');
})()
"#,
        )
        .expect(
            "rule style entries should expand unresolved box shorthand before longhand mutation",
        );

    assert_eq!(
        result,
        "margin-right: ; margin-bottom: ; margin-left: ; margin-top: 10px;,,,10px,margin-right/margin-bottom/margin-left/margin-top|padding-top: ; padding-right: ; padding-bottom: ; padding-left: calc(1px);,,,calc(1px),padding-top/padding-right/padding-bottom/padding-left"
    );
}

#[test]
fn border_image_shorthand_uses_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://border-image-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const borderImageLonghands = [
    'border-image-outset',
    'border-image-repeat',
    'border-image-slice',
    'border-image-source',
    'border-image-width'
  ];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };
  const lacksAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (actual.includes(name)) failures.push(`${label}:unexpected:${name}:${actual.join(',')}`);
    }
  };

  function exerciseBorderImage(style, label, ruleText) {
    style.setProperty('border-image', 'url("img.png") 30 / 2 / 1 round', 'important');
    eq(`${label}-value`, style.getPropertyValue('border-image'), 'url("img.png") 30 / 2 / 1 round');
    eq(`${label}-priority`, style.getPropertyPriority('border-image'), 'important');
    eq(`${label}-source`, style.getPropertyValue('border-image-source'), 'url("img.png")');
    eq(`${label}-slice`, style.getPropertyValue('border-image-slice'), '30');
    eq(`${label}-width`, style.getPropertyValue('border-image-width'), '2');
    eq(`${label}-outset`, style.getPropertyValue('border-image-outset'), '1');
    eq(`${label}-repeat`, style.getPropertyValue('border-image-repeat'), 'round');
    hasAll(`${label}-names`, style, borderImageLonghands);
    ok(`${label}-length`, style.length >= borderImageLonghands.length);
    ok(`${label}-cssText`, style.cssText.includes('border-image: url("img.png") 30 / 2 / 1 round !important;'));
    if (ruleText) {
      ok(`${label}-rule-cssText`, ruleText().includes('border-image: url("img.png") 30 / 2 / 1 round !important;'));
    }

    style.setProperty('--token', 'value');
    style.setProperty('-webkit-text-fill-color', 'red');
    const removed = style.removeProperty('border-image');
    eq(`${label}-removed`, removed, 'url("img.png") 30 / 2 / 1 round');
    eq(`${label}-after-remove`, style.getPropertyValue('border-image'), '');
    eq(`${label}-source-after-remove`, style.getPropertyValue('border-image-source'), '');
    lacksAll(`${label}-names-after-remove`, style, borderImageLonghands);
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    style.setProperty('border-image', 'url("img.png") 30 / 2 / 1 round');
    style.setProperty('border', '1px solid red');
    eq(`${label}-border-reset`, style.getPropertyValue('border-image'), 'none');
    eq(`${label}-border-source-reset`, style.getPropertyValue('border-image-source'), 'none');
    eq(`${label}-border-slice-reset`, style.getPropertyValue('border-image-slice'), '100%');
    eq(`${label}-border-width-reset`, style.getPropertyValue('border-image-width'), '1');
    eq(`${label}-border-outset-reset`, style.getPropertyValue('border-image-outset'), '0');
    eq(`${label}-border-repeat-reset`, style.getPropertyValue('border-image-repeat'), 'stretch');
  }

  const inline = document.createElement('div').style;
  exerciseBorderImage(inline, 'inline');

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = detachedDoc.createElement('div').style;
  exerciseBorderImage(detached, 'detached');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  exerciseBorderImage(rule.style, 'rule', () => rule.cssText);

  const keyframe = sheet.cssRules[1].cssRules[0];
  exerciseBorderImage(keyframe.style, 'keyframe', () => keyframe.cssText);

  return failures.length ? failures.slice(0, 16).join('|') : 'PASS';
})()
"#,
        )
        .expect("border-image shorthand PDB backing should evaluate");

    assert_eq!(result, "PASS");
}

#[test]
fn border_shorthand_uses_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://border-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const borderLonghands = [
    'border-top-width',
    'border-right-width',
    'border-bottom-width',
    'border-left-width',
    'border-top-style',
    'border-right-style',
    'border-bottom-style',
    'border-left-style',
    'border-top-color',
    'border-right-color',
    'border-bottom-color',
    'border-left-color'
  ];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };
  const lacks = (label, style, name) => {
    const actual = names(style);
    if (actual.includes(name)) failures.push(`${label}:unexpected:${name}:${actual.join(',')}`);
  };

  function exerciseMixedStyle(style, label) {
    style.setProperty('border-image', 'url("img.png") 30');
    style.setProperty('border', '1px solid red', 'important');
    eq(`${label}-border`, style.getPropertyValue('border'), '1px solid red');
    eq(`${label}-border-priority`, style.getPropertyPriority('border'), 'important');
    eq(`${label}-top-width`, style.getPropertyValue('border-top-width'), '1px');
    eq(`${label}-right-style`, style.getPropertyValue('border-right-style'), 'solid');
    eq(`${label}-bottom-color`, style.getPropertyValue('border-bottom-color'), 'red');
    eq(`${label}-border-image-reset`, style.getPropertyValue('border-image'), 'none');
    hasAll(`${label}-border-names`, style, borderLonghands);
    ok(`${label}-border-image-name`, names(style).includes('border-image-source'));
    ok(`${label}-border-cssText`, style.cssText.includes('border: 1px solid red !important;'));

    style.setProperty('--token', 'value');
    const borderBeforeRemove = style.getPropertyValue('border');
    style.setProperty('-webkit-text-fill-color', 'red');
    const removedBorder = style.removeProperty('border');
    eq(`${label}-border-before-remove`, borderBeforeRemove, '1px solid red');
    eq(`${label}-removed-border`, removedBorder, '1px solid red');
    eq(`${label}-border-after-remove`, style.getPropertyValue('border'), '');
    eq(`${label}-top-width-after-remove`, style.getPropertyValue('border-top-width'), '');
    eq(`${label}-border-image-after-remove`, style.getPropertyValue('border-image'), '');
    lacks(`${label}-border-image-name-after-remove`, style, 'border-image-source');
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');
  }

  const inline = document.createElement('div').style;
  exerciseMixedStyle(inline, 'inline');

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = detachedDoc.createElement('div').style;
  detached.cssText = 'border-image: url("img.png") 30; border: 2px dashed blue !important; --token: value; -webkit-text-fill-color: red;';
  eq('detached-border', detached.getPropertyValue('border'), '2px dashed blue');
  eq('detached-border-priority', detached.getPropertyPriority('border'), 'important');
  eq('detached-right-style', detached.getPropertyValue('border-right-style'), 'dashed');
  eq('detached-left-color', detached.getPropertyValue('border-left-color'), 'blue');
  eq('detached-border-image-reset', detached.getPropertyValue('border-image'), 'none');
  hasAll('detached-border-names', detached, borderLonghands);
  eq('detached-token', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  const removedDetached = detached.removeProperty('border');
  eq('detached-removed-border', removedDetached, '2px dashed blue');
  eq('detached-border-after-remove', detached.getPropertyValue('border'), '');
  eq('detached-border-image-after-remove', detached.getPropertyValue('border-image'), '');
  eq('detached-token-after-remove', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit-after-remove', detached.getPropertyValue('-webkit-text-fill-color'), 'red');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  rule.style.setProperty('border-image', 'url("img.png") 30');
  rule.style.setProperty('border', '3px dotted green', 'important');
  eq('rule-border', rule.style.getPropertyValue('border'), '3px dotted green');
  eq('rule-border-priority', rule.style.getPropertyPriority('border'), 'important');
  eq('rule-border-image-reset', rule.style.getPropertyValue('border-image'), 'none');
  hasAll('rule-border-names', rule.style, borderLonghands);
  ok('rule-cssText-border', rule.cssText.includes('border: 3px dotted green !important;'));
  rule.style.setProperty('--token', 'value');
  const removedRuleBorder = rule.style.removeProperty('border');
  eq('rule-removed-border', removedRuleBorder, '3px dotted green');
  eq('rule-border-after-remove', rule.style.getPropertyValue('border'), '');
  eq('rule-border-image-after-remove', rule.style.getPropertyValue('border-image'), '');
  ok('rule-cssText-token', rule.cssText.includes('--token: value;'));
  ok('rule-cssText-border-removed', !rule.cssText.includes('border: 3px dotted green'));

  const keyframe = sheet.cssRules[1].cssRules[0];
  keyframe.style.setProperty('border-image', 'url("img.png") 30');
  keyframe.style.setProperty('border', '4px double purple', 'important');
  eq('keyframe-border', keyframe.style.getPropertyValue('border'), '4px double purple');
  eq('keyframe-border-priority', keyframe.style.getPropertyPriority('border'), 'important');
  eq('keyframe-border-image-reset', keyframe.style.getPropertyValue('border-image'), 'none');
  hasAll('keyframe-border-names', keyframe.style, borderLonghands);
  ok('keyframe-cssText-border', keyframe.cssText.includes('border: 4px double purple !important;'));

  return failures.length ? failures.slice(0, 12).join('|') : 'PASS';
})()
"#,
        )
        .expect("border shorthand PDB backing should evaluate");

    assert_eq!(result, "PASS");
}

#[test]
fn border_component_shorthands_use_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://border-component-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const colorLonghands = [
    'border-top-color',
    'border-right-color',
    'border-bottom-color',
    'border-left-color'
  ];
  const styleLonghands = [
    'border-top-style',
    'border-right-style',
    'border-bottom-style',
    'border-left-style'
  ];
  const widthLonghands = [
    'border-top-width',
    'border-right-width',
    'border-bottom-width',
    'border-left-width'
  ];
  const sideLonghands = side => [
    `border-${side}-width`,
    `border-${side}-style`,
    `border-${side}-color`
  ];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };

  function exerciseMixedStyle(style, label) {
    style.setProperty('border-color', 'red blue', 'important');
    eq(`${label}-border-color`, style.getPropertyValue('border-color'), 'red blue');
    eq(`${label}-border-color-priority`, style.getPropertyPriority('border-color'), 'important');
    eq(`${label}-border-top-color`, style.getPropertyValue('border-top-color'), 'red');
    eq(`${label}-border-right-color`, style.getPropertyValue('border-right-color'), 'blue');
    hasAll(`${label}-color-names`, style, colorLonghands);
    ok(`${label}-color-cssText`, style.cssText.includes('border-color: red blue !important;'));

    style.setProperty('--token', 'value');
    style.setProperty('-webkit-text-fill-color', 'red');
    style.setProperty('border-width', '1px 2px 3px', 'important');
    eq(`${label}-border-width`, style.getPropertyValue('border-width'), '1px 2px 3px');
    eq(`${label}-border-bottom-width`, style.getPropertyValue('border-bottom-width'), '3px');
    eq(`${label}-border-left-width`, style.getPropertyValue('border-left-width'), '2px');
    eq(`${label}-border-width-priority`, style.getPropertyPriority('border-left-width'), 'important');
    hasAll(`${label}-width-names`, style, widthLonghands);
    ok(`${label}-side-token-name`, names(style).includes('--token'));
    ok(`${label}-side-webkit-name`, names(style).includes('-webkit-text-fill-color'));
    eq(`${label}-side-token`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-side-webkit`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    const removedWidth = style.removeProperty('border-width');
    eq(`${label}-removed-width`, removedWidth, '1px 2px 3px');
    eq(`${label}-border-width-after-remove`, style.getPropertyValue('border-width'), '');
    eq(`${label}-border-color-after-remove`, style.getPropertyValue('border-color'), 'red blue');
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    style.setProperty('border-top', '4px dashed green', 'important');
    eq(`${label}-border-side-top`, style.getPropertyValue('border-top'), '4px dashed green');
    eq(`${label}-border-side-top-width`, style.getPropertyValue('border-top-width'), '4px');
    eq(`${label}-border-side-top-style`, style.getPropertyValue('border-top-style'), 'dashed');
    eq(`${label}-border-side-top-color`, style.getPropertyValue('border-top-color'), 'green');
    eq(`${label}-border-side-top-priority`, style.getPropertyPriority('border-top'), 'important');
    hasAll(`${label}-border-top-names`, style, sideLonghands('top'));
    ok(`${label}-border-top-cssText`, style.cssText.includes('border-top: 4px dashed green !important;'));
    const removedTop = style.removeProperty('border-top');
    eq(`${label}-removed-top`, removedTop, '4px dashed green');
    eq(`${label}-border-top-after-remove`, style.getPropertyValue('border-top'), '');
    eq(`${label}-token-after-top-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-top-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');
  }

  const inline = document.createElement('div').style;
  exerciseMixedStyle(inline, 'inline');

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = detachedDoc.createElement('div').style;
  detached.cssText = 'border-style: solid dotted !important; --token: value; -webkit-text-fill-color: red;';
  eq('detached-border-style', detached.getPropertyValue('border-style'), 'solid dotted');
  eq('detached-border-style-priority', detached.getPropertyPriority('border-style'), 'important');
  eq('detached-border-right-style', detached.getPropertyValue('border-right-style'), 'dotted');
  hasAll('detached-style-names', detached, styleLonghands);
  eq('detached-token', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  const removedStyle = detached.removeProperty('border-style');
  eq('detached-removed-style', removedStyle, 'solid dotted');
  eq('detached-border-style-after-remove', detached.getPropertyValue('border-style'), '');
  eq('detached-token-after-remove', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit-after-remove', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  detached.setProperty('border-right', '2px dashed green', 'important');
  eq('detached-border-right', detached.getPropertyValue('border-right'), '2px dashed green');
  eq('detached-border-right-style', detached.getPropertyValue('border-right-style'), 'dashed');
  hasAll('detached-right-names', detached, sideLonghands('right'));
  ok('detached-right-cssText', detached.cssText.includes('border-right: 2px dashed green !important;'));

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  rule.style.setProperty('border-style', 'solid dotted', 'important');
  eq('rule-border-style', rule.style.getPropertyValue('border-style'), 'solid dotted');
  eq('rule-border-style-priority', rule.style.getPropertyPriority('border-style'), 'important');
  hasAll('rule-style-names', rule.style, styleLonghands);
  ok('rule-cssText-style', rule.cssText.includes('border-style: solid dotted !important;'));
  rule.style.setProperty('--token', 'value');
  rule.style.setProperty('border-color', 'green');
  eq('rule-border-color', rule.style.getPropertyValue('border-color'), 'green');
  eq('rule-token', rule.style.getPropertyValue('--token'), 'value');
  const removedRuleStyle = rule.style.removeProperty('border-style');
  eq('rule-removed-style', removedRuleStyle, 'solid dotted');
  eq('rule-border-style-after-remove', rule.style.getPropertyValue('border-style'), '');
  eq('rule-border-color-after-remove', rule.style.getPropertyValue('border-color'), 'green');
  ok('rule-cssText-color', rule.cssText.includes('border-color: green;'));
  ok('rule-cssText-token', rule.cssText.includes('--token: value;'));
  ok('rule-cssText-style-removed', !rule.cssText.includes('border-style'));
  rule.style.setProperty('border-bottom', '3px double blue', 'important');
  eq('rule-border-bottom', rule.style.getPropertyValue('border-bottom'), '3px double blue');
  eq('rule-border-bottom-style', rule.style.getPropertyValue('border-bottom-style'), 'double');
  hasAll('rule-bottom-names', rule.style, sideLonghands('bottom'));
  ok('rule-cssText-bottom', rule.cssText.includes('border-bottom: 3px double blue !important;'));

  const keyframe = sheet.cssRules[1].cssRules[0];
  keyframe.style.setProperty('border-width', '4px 5px', 'important');
  eq('keyframe-border-width', keyframe.style.getPropertyValue('border-width'), '4px 5px');
  eq('keyframe-border-left-width', keyframe.style.getPropertyValue('border-left-width'), '5px');
  eq('keyframe-border-width-priority', keyframe.style.getPropertyPriority('border-width'), 'important');
  hasAll('keyframe-width-names', keyframe.style, widthLonghands);
  ok('keyframe-cssText-width', keyframe.cssText.includes('border-width: 4px 5px !important;'));
  keyframe.style.setProperty('border-left', '6px solid purple', 'important');
  eq('keyframe-border-left', keyframe.style.getPropertyValue('border-left'), '6px solid purple');
  eq('keyframe-border-left-width', keyframe.style.getPropertyValue('border-left-width'), '6px');
  hasAll('keyframe-left-names', keyframe.style, sideLonghands('left'));
  ok('keyframe-cssText-left', keyframe.cssText.includes('border-left: 6px solid purple !important;'));

  return failures.length ? failures.slice(0, 8).join('|') : 'PASS';
})()
"#,
        )
        .expect("border component shorthand PDB backing should evaluate");

    assert_eq!(result, "PASS");
}

#[test]
fn border_radius_shorthand_uses_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://border-radius-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const radiusLonghands = [
    'border-top-left-radius',
    'border-top-right-radius',
    'border-bottom-right-radius',
    'border-bottom-left-radius'
  ];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };

  function exerciseMixedStyle(style, label) {
    style.setProperty('border-radius', '1px 2px', 'important');
    eq(`${label}-radius`, style.getPropertyValue('border-radius'), '1px 2px');
    eq(`${label}-radius-priority`, style.getPropertyPriority('border-radius'), 'important');
    eq(`${label}-top-left`, style.getPropertyValue('border-top-left-radius'), '1px');
    eq(`${label}-top-right`, style.getPropertyValue('border-top-right-radius'), '2px');
    hasAll(`${label}-radius-names`, style, radiusLonghands);
    ok(`${label}-radius-cssText`, style.cssText.includes('border-radius: 1px 2px !important;'));

    style.setProperty('--token', 'value');
    style.setProperty('-webkit-text-fill-color', 'red');
    const removedRadius = style.removeProperty('border-radius');
    eq(`${label}-removed-radius`, removedRadius, '1px 2px');
    eq(`${label}-radius-after-remove`, style.getPropertyValue('border-radius'), '');
    eq(`${label}-top-left-after-remove`, style.getPropertyValue('border-top-left-radius'), '');
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');
  }

  const inline = document.createElement('div').style;
  exerciseMixedStyle(inline, 'inline');

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = detachedDoc.createElement('div').style;
  detached.cssText = 'border-radius: 3px 4px !important; --token: value; -webkit-text-fill-color: red;';
  eq('detached-radius', detached.getPropertyValue('border-radius'), '3px 4px');
  eq('detached-radius-priority', detached.getPropertyPriority('border-radius'), 'important');
  eq('detached-top-right', detached.getPropertyValue('border-top-right-radius'), '4px');
  hasAll('detached-radius-names', detached, radiusLonghands);
  ok('detached-radius-cssText', detached.cssText.includes('border-radius: 3px 4px !important;'));
  eq('detached-token', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  const removedDetached = detached.removeProperty('border-radius');
  eq('detached-removed-radius', removedDetached, '3px 4px');
  eq('detached-radius-after-remove', detached.getPropertyValue('border-radius'), '');
  eq('detached-token-after-remove', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit-after-remove', detached.getPropertyValue('-webkit-text-fill-color'), 'red');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  rule.style.setProperty('border-radius', '5px 6px', 'important');
  eq('rule-radius', rule.style.getPropertyValue('border-radius'), '5px 6px');
  eq('rule-radius-priority', rule.style.getPropertyPriority('border-radius'), 'important');
  eq('rule-top-left', rule.style.getPropertyValue('border-top-left-radius'), '5px');
  hasAll('rule-radius-names', rule.style, radiusLonghands);
  ok('rule-cssText-radius', rule.cssText.includes('border-radius: 5px 6px !important;'));
  const removedRuleRadius = rule.style.removeProperty('border-radius');
  eq('rule-removed-radius', removedRuleRadius, '5px 6px');
  eq('rule-radius-after-remove', rule.style.getPropertyValue('border-radius'), '');
  ok('rule-cssText-radius-removed', !rule.cssText.includes('border-radius'));

  const keyframe = sheet.cssRules[1].cssRules[0];
  keyframe.style.setProperty('border-radius', '7px 8px', 'important');
  eq('keyframe-radius', keyframe.style.getPropertyValue('border-radius'), '7px 8px');
  eq('keyframe-radius-priority', keyframe.style.getPropertyPriority('border-radius'), 'important');
  eq('keyframe-bottom-left', keyframe.style.getPropertyValue('border-bottom-left-radius'), '8px');
  hasAll('keyframe-radius-names', keyframe.style, radiusLonghands);
  ok('keyframe-cssText-radius', keyframe.cssText.includes('border-radius: 7px 8px !important;'));

  return failures.length ? failures.slice(0, 10).join('|') : 'PASS';
})()
"#,
        )
        .expect("border-radius shorthand PDB backing should evaluate");

    assert_eq!(result, "PASS");
}

#[test]
fn text_decoration_shorthand_uses_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://text-decoration-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const decorationLonghands = [
    'text-decoration-line',
    'text-decoration-thickness',
    'text-decoration-style',
    'text-decoration-color'
  ];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };

  function exerciseMixedStyle(style, label) {
    style.setProperty('text-decoration', 'overline from-font dotted green', 'important');
    eq(`${label}-decoration`, style.getPropertyValue('text-decoration'), 'overline from-font dotted green');
    eq(`${label}-decoration-priority`, style.getPropertyPriority('text-decoration'), 'important');
    eq(`${label}-line`, style.getPropertyValue('text-decoration-line'), 'overline');
    eq(`${label}-thickness`, style.getPropertyValue('text-decoration-thickness'), 'from-font');
    eq(`${label}-style`, style.getPropertyValue('text-decoration-style'), 'dotted');
    eq(`${label}-color`, style.getPropertyValue('text-decoration-color'), 'green');
    hasAll(`${label}-decoration-names`, style, decorationLonghands);
    ok(`${label}-decoration-cssText`, style.cssText.includes('text-decoration: overline from-font dotted green !important;'));

    style.setProperty('--token', 'value');
    style.setProperty('-webkit-text-fill-color', 'red');
    style.setProperty('text-decoration-skip-ink', 'all', 'important');
    eq(`${label}-skip-ink`, style.getPropertyValue('text-decoration-skip-ink'), 'all');
    eq(`${label}-skip-ink-priority`, style.getPropertyPriority('text-decoration-skip-ink'), 'important');
    ok(`${label}-skip-ink-name`, names(style).includes('text-decoration-skip-ink'));
    ok(`${label}-side-token-name`, names(style).includes('--token'));
    ok(`${label}-side-webkit-name`, names(style).includes('-webkit-text-fill-color'));
    eq(`${label}-side-token`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-side-webkit`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    style.setProperty('text-decoration-line', 'spelling-error', 'important');
    eq(`${label}-compat-line`, style.getPropertyValue('text-decoration-line'), 'spelling-error');
    eq(`${label}-compat-line-priority`, style.getPropertyPriority('text-decoration-line'), 'important');
    eq(`${label}-compat-decoration`, style.getPropertyValue('text-decoration'), 'spelling-error from-font dotted green');
    eq(`${label}-skip-ink-after-compat-line`, style.getPropertyValue('text-decoration-skip-ink'), '');
    ok(`${label}-compat-line-name`, names(style).includes('text-decoration-line'));
    ok(`${label}-compat-cssText`, style.cssText.includes('text-decoration-line: spelling-error !important;'));
    style.textDecorationStyle = 'solid wavy';
    eq(`${label}-compat-style-after-invalid-idl`, style.getPropertyValue('text-decoration-style'), 'dotted');
    eq(`${label}-compat-decoration-after-invalid-idl`, style.getPropertyValue('text-decoration'), 'spelling-error from-font dotted green');

    const removedDecoration = style.removeProperty('text-decoration');
    eq(`${label}-removed-decoration`, removedDecoration, '');
    eq(`${label}-decoration-after-remove`, style.getPropertyValue('text-decoration'), '');
    eq(`${label}-line-after-remove`, style.getPropertyValue('text-decoration-line'), '');
    eq(`${label}-skip-ink-after-remove`, style.getPropertyValue('text-decoration-skip-ink'), '');
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');
  }

  const inline = document.createElement('div').style;
  exerciseMixedStyle(inline, 'inline');

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = detachedDoc.createElement('div').style;
  detached.cssText = 'text-decoration: overline from-font dotted green !important; text-decoration-skip-ink: all; --token: value; -webkit-text-fill-color: red;';
  eq('detached-decoration', detached.getPropertyValue('text-decoration'), 'overline from-font dotted green');
  eq('detached-decoration-priority', detached.getPropertyPriority('text-decoration'), 'important');
  eq('detached-skip-ink', detached.getPropertyValue('text-decoration-skip-ink'), 'all');
  hasAll('detached-decoration-names', detached, decorationLonghands);
  ok('detached-decoration-cssText', detached.cssText.includes('text-decoration: overline from-font dotted green !important;'));
  eq('detached-token', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  detached.setProperty('text-decoration-line', 'grammar-error', 'important');
  eq('detached-compat-line', detached.getPropertyValue('text-decoration-line'), 'grammar-error');
  eq('detached-compat-decoration', detached.getPropertyValue('text-decoration'), 'grammar-error from-font dotted green');
  eq('detached-skip-ink-after-compat-line', detached.getPropertyValue('text-decoration-skip-ink'), '');
  ok('detached-compat-cssText', detached.cssText.includes('text-decoration-line: grammar-error !important;'));
  detached.textDecorationStyle = 'solid wavy';
  eq('detached-compat-style-after-invalid-idl', detached.getPropertyValue('text-decoration-style'), 'dotted');
  eq('detached-compat-decoration-after-invalid-idl', detached.getPropertyValue('text-decoration'), 'grammar-error from-font dotted green');
  const removedDetached = detached.removeProperty('text-decoration');
  eq('detached-removed-decoration', removedDetached, '');
  eq('detached-decoration-after-remove', detached.getPropertyValue('text-decoration'), '');
  eq('detached-compat-line-after-remove', detached.getPropertyValue('text-decoration-line'), '');
  eq('detached-skip-ink-after-remove', detached.getPropertyValue('text-decoration-skip-ink'), '');
  eq('detached-token-after-remove', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit-after-remove', detached.getPropertyValue('-webkit-text-fill-color'), 'red');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  rule.style.setProperty('text-decoration', 'underline auto dashed blue', 'important');
  eq('rule-decoration', rule.style.getPropertyValue('text-decoration'), 'underline dashed blue');
  eq('rule-decoration-priority', rule.style.getPropertyPriority('text-decoration'), 'important');
  eq('rule-line', rule.style.getPropertyValue('text-decoration-line'), 'underline');
  eq('rule-style', rule.style.getPropertyValue('text-decoration-style'), 'dashed');
  eq('rule-color', rule.style.getPropertyValue('text-decoration-color'), 'blue');
  hasAll('rule-decoration-names', rule.style, decorationLonghands);
  ok('rule-cssText-decoration', rule.cssText.includes('text-decoration: underline dashed blue !important;'));
  rule.style.setProperty('text-decoration-line', 'spelling-error', 'important');
  eq('rule-compat-line', rule.style.getPropertyValue('text-decoration-line'), 'spelling-error');
  eq('rule-compat-decoration', rule.style.getPropertyValue('text-decoration'), 'spelling-error dashed blue');
  ok('rule-cssText-compat-line', rule.cssText.includes('text-decoration: spelling-error dashed blue !important;'));
  rule.style.setProperty('--token', 'value');
  const removedRuleDecoration = rule.style.removeProperty('text-decoration');
  eq('rule-removed-decoration', removedRuleDecoration, '');
  eq('rule-decoration-after-remove', rule.style.getPropertyValue('text-decoration'), '');
  eq('rule-compat-line-after-remove', rule.style.getPropertyValue('text-decoration-line'), '');
  ok('rule-cssText-decoration-removed', !rule.cssText.includes('text-decoration: underline dashed blue'));
  ok('rule-cssText-token', rule.cssText.includes('--token: value;'));

  const keyframe = sheet.cssRules[1].cssRules[0];
  keyframe.style.setProperty('text-decoration', 'line-through 2px wavy red', 'important');
  eq('keyframe-decoration', keyframe.style.getPropertyValue('text-decoration'), 'line-through 2px wavy red');
  eq('keyframe-decoration-priority', keyframe.style.getPropertyPriority('text-decoration'), 'important');
  eq('keyframe-line', keyframe.style.getPropertyValue('text-decoration-line'), 'line-through');
  eq('keyframe-thickness', keyframe.style.getPropertyValue('text-decoration-thickness'), '2px');
  eq('keyframe-style', keyframe.style.getPropertyValue('text-decoration-style'), 'wavy');
  eq('keyframe-color', keyframe.style.getPropertyValue('text-decoration-color'), 'red');
  hasAll('keyframe-decoration-names', keyframe.style, decorationLonghands);
  ok('keyframe-cssText-decoration', keyframe.cssText.includes('text-decoration: line-through 2px wavy red !important;'));

  return failures.length ? failures.slice(0, 10).join('|') : 'PASS';
})()
"#,
        )
        .expect("text-decoration shorthand PDB backing should evaluate");

    assert_eq!(result, "PASS");
}

#[test]
fn text_emphasis_shorthand_uses_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://text-emphasis-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const emphasisLonghands = ['text-emphasis-style', 'text-emphasis-color'];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };

  function exerciseMixedStyle(style, label) {
    style.setProperty('text-emphasis', 'dot red', 'important');
    eq(`${label}-emphasis`, style.getPropertyValue('text-emphasis'), 'dot red');
    eq(`${label}-emphasis-priority`, style.getPropertyPriority('text-emphasis'), 'important');
    eq(`${label}-style`, style.getPropertyValue('text-emphasis-style'), 'dot');
    eq(`${label}-color`, style.getPropertyValue('text-emphasis-color'), 'red');
    hasAll(`${label}-emphasis-names`, style, emphasisLonghands);
    ok(`${label}-emphasis-cssText`, style.cssText.includes('text-emphasis: dot red !important;'));

    style.setProperty('--token', 'value');
    style.setProperty('-webkit-text-fill-color', 'red');
    style.setProperty('text-emphasis-position', 'over left', 'important');
    eq(`${label}-position`, style.getPropertyValue('text-emphasis-position'), 'over left');
    eq(`${label}-position-priority`, style.getPropertyPriority('text-emphasis-position'), 'important');
    ok(`${label}-position-name`, names(style).includes('text-emphasis-position'));
    ok(`${label}-side-token-name`, names(style).includes('--token'));
    ok(`${label}-side-webkit-name`, names(style).includes('-webkit-text-fill-color'));
    eq(`${label}-side-token`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-side-webkit`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    const removedEmphasis = style.removeProperty('text-emphasis');
    eq(`${label}-removed-emphasis`, removedEmphasis, 'dot red');
    eq(`${label}-emphasis-after-remove`, style.getPropertyValue('text-emphasis'), '');
    eq(`${label}-style-after-remove`, style.getPropertyValue('text-emphasis-style'), '');
    eq(`${label}-position-after-remove`, style.getPropertyValue('text-emphasis-position'), 'over left');
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');
  }

  const inline = document.createElement('div').style;
  exerciseMixedStyle(inline, 'inline');

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = detachedDoc.createElement('div').style;
  detached.cssText = 'text-emphasis: dot red !important; text-emphasis-position: over left; --token: value; -webkit-text-fill-color: red;';
  eq('detached-emphasis', detached.getPropertyValue('text-emphasis'), 'dot red');
  eq('detached-emphasis-priority', detached.getPropertyPriority('text-emphasis'), 'important');
  eq('detached-position', detached.getPropertyValue('text-emphasis-position'), 'over left');
  hasAll('detached-emphasis-names', detached, emphasisLonghands);
  ok('detached-emphasis-cssText', detached.cssText.includes('text-emphasis: dot red !important;'));
  eq('detached-token', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  const removedDetached = detached.removeProperty('text-emphasis');
  eq('detached-removed-emphasis', removedDetached, 'dot red');
  eq('detached-emphasis-after-remove', detached.getPropertyValue('text-emphasis'), '');
  eq('detached-position-after-remove', detached.getPropertyValue('text-emphasis-position'), 'over left');
  eq('detached-token-after-remove', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit-after-remove', detached.getPropertyValue('-webkit-text-fill-color'), 'red');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  rule.style.setProperty('text-emphasis', 'circle blue', 'important');
  eq('rule-emphasis', rule.style.getPropertyValue('text-emphasis'), 'circle blue');
  eq('rule-emphasis-priority', rule.style.getPropertyPriority('text-emphasis'), 'important');
  eq('rule-style', rule.style.getPropertyValue('text-emphasis-style'), 'circle');
  eq('rule-color', rule.style.getPropertyValue('text-emphasis-color'), 'blue');
  hasAll('rule-emphasis-names', rule.style, emphasisLonghands);
  ok('rule-cssText-emphasis', rule.cssText.includes('text-emphasis: circle blue !important;'));
  rule.style.setProperty('--token', 'value');
  const removedRuleEmphasis = rule.style.removeProperty('text-emphasis');
  eq('rule-removed-emphasis', removedRuleEmphasis, 'circle blue');
  eq('rule-emphasis-after-remove', rule.style.getPropertyValue('text-emphasis'), '');
  ok('rule-cssText-emphasis-removed', !rule.cssText.includes('text-emphasis: circle blue'));
  ok('rule-cssText-token', rule.cssText.includes('--token: value;'));

  const keyframe = sheet.cssRules[1].cssRules[0];
  keyframe.style.setProperty('text-emphasis', 'sesame green', 'important');
  eq('keyframe-emphasis', keyframe.style.getPropertyValue('text-emphasis'), 'sesame green');
  eq('keyframe-emphasis-priority', keyframe.style.getPropertyPriority('text-emphasis'), 'important');
  eq('keyframe-style', keyframe.style.getPropertyValue('text-emphasis-style'), 'sesame');
  eq('keyframe-color', keyframe.style.getPropertyValue('text-emphasis-color'), 'green');
  hasAll('keyframe-emphasis-names', keyframe.style, emphasisLonghands);
  ok('keyframe-cssText-emphasis', keyframe.cssText.includes('text-emphasis: sesame green !important;'));

  return failures.length ? failures.slice(0, 10).join('|') : 'PASS';
})()
"#,
        )
        .expect("text-emphasis shorthand PDB backing should evaluate");

    assert_eq!(result, "PASS");
}

#[test]
fn transition_shorthand_uses_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://transition-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const transitionLonghands = [
    'transition-property',
    'transition-duration',
    'transition-timing-function',
    'transition-delay',
    'transition-behavior'
  ];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };

  function exerciseMixedStyle(style, label) {
    style.setProperty('transition', 'display 3s ease-in-out 1s allow-discrete, opacity', 'important');
    eq(`${label}-transition`, style.getPropertyValue('transition'), 'display 3s ease-in-out 1s allow-discrete, opacity');
    eq(`${label}-transition-priority`, style.getPropertyPriority('transition'), 'important');
    eq(`${label}-property`, style.getPropertyValue('transition-property'), 'display, opacity');
    eq(`${label}-duration`, style.getPropertyValue('transition-duration'), '3s, 0s');
    eq(`${label}-timing`, style.getPropertyValue('transition-timing-function'), 'ease-in-out, ease');
    eq(`${label}-delay`, style.getPropertyValue('transition-delay'), '1s, 0s');
    eq(`${label}-behavior`, style.getPropertyValue('transition-behavior'), 'allow-discrete, normal');
    eq(`${label}-duration-priority`, style.getPropertyPriority('transition-duration'), 'important');
    hasAll(`${label}-transition-names`, style, transitionLonghands);
    ok(`${label}-transition-cssText`, style.cssText.includes('transition: display 3s ease-in-out 1s allow-discrete, opacity !important;'));

    style.setProperty('--token', 'value');
    style.setProperty('-webkit-text-fill-color', 'red');
    style.setProperty('transition-duration', '4s, 5s', 'important');
    eq(`${label}-transition-after-duration`, style.getPropertyValue('transition'), 'display 4s ease-in-out 1s allow-discrete, opacity 5s');
    ok(`${label}-cssText-after-duration`, style.cssText.includes('transition: display 4s ease-in-out 1s allow-discrete, opacity 5s !important;'));
    ok(`${label}-cssText-side-after-duration`, style.cssText.includes('-webkit-text-fill-color: red;'));
    eq(`${label}-duration-after-duration`, style.getPropertyValue('transition-duration'), '4s, 5s');
    eq(`${label}-duration-priority-after-duration`, style.getPropertyPriority('transition-duration'), 'important');
    ok(`${label}-side-token-name`, names(style).includes('--token'));
    ok(`${label}-side-webkit-name`, names(style).includes('-webkit-text-fill-color'));
    eq(`${label}-side-token`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-side-webkit`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    const removedTransition = style.removeProperty('transition');
    eq(`${label}-removed-transition`, removedTransition, 'display 4s ease-in-out 1s allow-discrete, opacity 5s');
    eq(`${label}-transition-after-remove`, style.getPropertyValue('transition'), '');
    eq(`${label}-property-after-remove`, style.getPropertyValue('transition-property'), '');
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    style.setProperty('transition-duration', 'calc(10s + (sign(2cqw - 10px) * 5s))');
    style.setProperty('transition-timing-function', 'steps(calc(2 * sibling-index()), jump-none)');
    eq(`${label}-dynamic-duration`, style.getPropertyValue('transition-duration'), 'calc(10s + (5s * sign(2cqw - 10px)))');
    eq(`${label}-dynamic-timing`, style.getPropertyValue('transition-timing-function'), 'steps(calc(2 * sibling-index()), jump-none)');
    ok(`${label}-dynamic-duration-name`, names(style).includes('transition-duration'));
    ok(`${label}-dynamic-timing-name`, names(style).includes('transition-timing-function'));
  }

  const inline = document.createElement('div').style;
  exerciseMixedStyle(inline, 'inline');

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = detachedDoc.createElement('div').style;
  detached.cssText = 'transition: display 3s ease-in-out 1s allow-discrete, opacity !important; --token: value; -webkit-text-fill-color: red;';
  eq('detached-transition', detached.getPropertyValue('transition'), 'display 3s ease-in-out 1s allow-discrete, opacity');
  eq('detached-transition-priority', detached.getPropertyPriority('transition'), 'important');
  eq('detached-property', detached.getPropertyValue('transition-property'), 'display, opacity');
  eq('detached-duration', detached.getPropertyValue('transition-duration'), '3s, 0s');
  hasAll('detached-transition-names', detached, transitionLonghands);
  ok('detached-transition-cssText', detached.cssText.includes('transition: display 3s ease-in-out 1s allow-discrete, opacity !important;'));
  eq('detached-token', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit-kebab', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  eq('detached-webkit-lower', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  eq('detached-webkit', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  const removedDetached = detached.removeProperty('transition-duration');
  eq('detached-removed-duration', removedDetached, '3s, 0s');
  eq('detached-transition-after-duration-remove', detached.getPropertyValue('transition'), '');
  eq('detached-token-after-remove', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit-after-remove', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  const dynamicDetached = detachedDoc.createElement('div').style;
  dynamicDetached.setProperty('transition-duration', 'calc(10s + (sign(2cqw - 10px) * 5s))');
  dynamicDetached.setProperty('transition-timing-function', 'steps(calc(2 * sibling-index()), jump-none)');
  eq('detached-dynamic-duration', dynamicDetached.getPropertyValue('transition-duration'), 'calc(10s + (5s * sign(2cqw - 10px)))');
  eq('detached-dynamic-timing', dynamicDetached.getPropertyValue('transition-timing-function'), 'steps(calc(2 * sibling-index()), jump-none)');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  rule.style.setProperty('transition', 'top 1s cubic-bezier(0, -2, 1, 3) -3s', 'important');
  eq('rule-transition', rule.style.getPropertyValue('transition'), 'top 1s cubic-bezier(0, -2, 1, 3) -3s');
  eq('rule-transition-priority', rule.style.getPropertyPriority('transition'), 'important');
  eq('rule-property', rule.style.getPropertyValue('transition-property'), 'top');
  eq('rule-delay', rule.style.getPropertyValue('transition-delay'), '-3s');
  hasAll('rule-transition-names', rule.style, transitionLonghands);
  ok('rule-cssText-transition', rule.cssText.includes('transition: top 1s cubic-bezier(0, -2, 1, 3) -3s !important;'));
  rule.style.setProperty('--token', 'value');
  const removedRuleTransition = rule.style.removeProperty('transition');
  eq('rule-removed-transition', removedRuleTransition, 'top 1s cubic-bezier(0, -2, 1, 3) -3s');
  eq('rule-transition-after-remove', rule.style.getPropertyValue('transition'), '');
  ok('rule-cssText-transition-removed', !rule.cssText.includes('transition: top 1s'));
  ok('rule-cssText-token', rule.cssText.includes('--token: value;'));
  rule.style.transitionTimingFunction = 'steps(calc(2 * sibling-index()), jump-none)';
  eq('rule-dynamic-timing', rule.style.getPropertyValue('transition-timing-function'), 'steps(calc(2 * sibling-index()), jump-none)');

  const keyframe = sheet.cssRules[1].cssRules[0];
  keyframe.style.setProperty('transition', 'opacity 2s steps(2) 1s', 'important');
  eq('keyframe-transition', keyframe.style.getPropertyValue('transition'), 'opacity 2s steps(2) 1s');
  eq('keyframe-transition-priority', keyframe.style.getPropertyPriority('transition'), 'important');
  eq('keyframe-property', keyframe.style.getPropertyValue('transition-property'), 'opacity');
  eq('keyframe-timing', keyframe.style.getPropertyValue('transition-timing-function'), 'steps(2)');
  hasAll('keyframe-transition-names', keyframe.style, transitionLonghands);
  ok('keyframe-cssText-transition', keyframe.cssText.includes('transition: opacity 2s steps(2) 1s !important;'));
  const dynamicKeyframe = sheet.cssRules[1].cssRules[0];
  dynamicKeyframe.style.removeProperty('transition');
  dynamicKeyframe.style.transitionDuration = 'calc(10s + (sign(2cqw - 10px) * 5s))';
  eq('keyframe-dynamic-duration', dynamicKeyframe.style.getPropertyValue('transition-duration'), 'calc(10s + (5s * sign(2cqw - 10px)))');

  const invalid = document.createElement('div').style;
  invalid.transitionDuration = '1s';
  invalid.transitionDuration = '-2s';
  eq('invalid-duration-preserves-old', invalid.transitionDuration, '1s');

  return failures.length ? failures.slice(0, 10).join('|') : 'PASS';
})()
"#,
        )
        .expect("transition shorthand PDB backing should evaluate");

    assert_eq!(result, "PASS");
}

#[test]
fn animation_shorthand_uses_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://animation-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const animationLonghands = [
    'animation-duration',
    'animation-timing-function',
    'animation-delay',
    'animation-iteration-count',
    'animation-direction',
    'animation-fill-mode',
    'animation-play-state',
    'animation-name',
    'animation-timeline',
    'animation-range-start',
    'animation-range-end'
  ];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };

  function exerciseMixedStyle(style, label) {
    style.setProperty('animation', 'fade paused both reverse 3 1s 2s linear', 'important');
    eq(`${label}-animation`, style.getPropertyValue('animation'), '1s linear 2s 3 reverse both paused fade');
    eq(`${label}-animation-priority`, style.getPropertyPriority('animation'), 'important');
    eq(`${label}-duration`, style.getPropertyValue('animation-duration'), '1s');
    eq(`${label}-timing`, style.getPropertyValue('animation-timing-function'), 'linear');
    eq(`${label}-delay`, style.getPropertyValue('animation-delay'), '2s');
    eq(`${label}-iteration`, style.getPropertyValue('animation-iteration-count'), '3');
    eq(`${label}-direction`, style.getPropertyValue('animation-direction'), 'reverse');
    eq(`${label}-fill`, style.getPropertyValue('animation-fill-mode'), 'both');
    eq(`${label}-play`, style.getPropertyValue('animation-play-state'), 'paused');
    eq(`${label}-name`, style.getPropertyValue('animation-name'), 'fade');
    eq(`${label}-timeline`, style.getPropertyValue('animation-timeline'), 'auto');
    eq(`${label}-range-start`, style.getPropertyValue('animation-range-start'), 'normal');
    eq(`${label}-range-end`, style.getPropertyValue('animation-range-end'), 'normal');
    eq(`${label}-duration-priority`, style.getPropertyPriority('animation-duration'), 'important');
    hasAll(`${label}-animation-names`, style, animationLonghands);

    style.setProperty('--token', 'value');
    style.setProperty('-webkit-text-fill-color', 'red');
    style.setProperty('animation-timing-function', 'linear(0, 1)', 'important');
    eq(`${label}-animation-after-timing`, style.getPropertyValue('animation'), '1s linear(0, 1) 2s 3 reverse both paused fade');
    eq(`${label}-timing-after-timing`, style.getPropertyValue('animation-timing-function'), 'linear(0, 1)');
    eq(`${label}-timing-priority-after-timing`, style.getPropertyPriority('animation-timing-function'), 'important');
    ok(`${label}-side-token-name`, names(style).includes('--token'));
    ok(`${label}-side-webkit-name`, names(style).includes('-webkit-text-fill-color'));
    eq(`${label}-side-token`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-side-webkit`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    style.setProperty('animation-timing-function', 'ease-in-out', 'important');
    eq(`${label}-animation-after-ordinary-timing`, style.getPropertyValue('animation'), '1s ease-in-out 2s 3 reverse both paused fade');
    eq(`${label}-ordinary-timing`, style.getPropertyValue('animation-timing-function'), 'ease-in-out');
    eq(`${label}-ordinary-timing-priority`, style.getPropertyPriority('animation-timing-function'), 'important');
    ok(`${label}-ordinary-timing-name`, names(style).includes('animation-timing-function'));

    const removedAnimation = style.removeProperty('animation');
    eq(`${label}-removed-animation`, removedAnimation, '1s ease-in-out 2s 3 reverse both paused fade');
    eq(`${label}-animation-after-remove`, style.getPropertyValue('animation'), '');
    eq(`${label}-duration-after-remove`, style.getPropertyValue('animation-duration'), '');
    eq(`${label}-timing-after-remove`, style.getPropertyValue('animation-timing-function'), '');
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    style.setProperty('animation-range', 'entry 10% exit 20%');
    eq(`${label}-range`, style.getPropertyValue('animation-range'), 'entry 10% exit 20%');
    eq(`${label}-range-start-after-range`, style.getPropertyValue('animation-range-start'), 'entry 10%');
    eq(`${label}-range-end-after-range`, style.getPropertyValue('animation-range-end'), 'exit 20%');
    const removedRange = style.removeProperty('animation-range');
    eq(`${label}-removed-range`, removedRange, 'entry 10% exit 20%');
    eq(`${label}-range-after-remove`, style.getPropertyValue('animation-range'), '');

    style.setProperty('animation-duration', 'calc(10s + (sign(2cqw - 10px) * 5s))');
    style.setProperty('animation-timing-function', 'steps(calc(2 * sibling-index()), jump-none)');
    eq(`${label}-dynamic-duration`, style.getPropertyValue('animation-duration'), 'calc(10s + (5s * sign(2cqw - 10px)))');
    eq(`${label}-dynamic-timing`, style.getPropertyValue('animation-timing-function'), 'steps(calc(2 * sibling-index()), jump-none)');
    ok(`${label}-dynamic-duration-name`, names(style).includes('animation-duration'));
    ok(`${label}-dynamic-timing-name`, names(style).includes('animation-timing-function'));
  }

  const inline = document.createElement('div').style;
  exerciseMixedStyle(inline, 'inline');

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = detachedDoc.createElement('div').style;
  detached.cssText = 'animation: fade paused both reverse 3 1s 2s linear !important; --token: value; -webkit-text-fill-color: red;';
  eq('detached-animation', detached.getPropertyValue('animation'), '1s linear 2s 3 reverse both paused fade');
  eq('detached-animation-priority', detached.getPropertyPriority('animation'), 'important');
  eq('detached-duration', detached.getPropertyValue('animation-duration'), '1s');
  eq('detached-timeline', detached.getPropertyValue('animation-timeline'), 'auto');
  hasAll('detached-animation-names', detached, animationLonghands);
  eq('detached-token', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  detached.setProperty('animation-timing-function', 'linear(0, 1)', 'important');
  eq('detached-animation-after-timing', detached.getPropertyValue('animation'), '1s linear(0, 1) 2s 3 reverse both paused fade');
  eq('detached-timing-after-timing', detached.getPropertyValue('animation-timing-function'), 'linear(0, 1)');
  const removedDetached = detached.removeProperty('animation-duration');
  eq('detached-removed-duration', removedDetached, '1s');
  eq('detached-animation-after-duration-remove', detached.getPropertyValue('animation'), '');
  eq('detached-token-after-remove', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit-after-remove', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  const dynamicDetached = detachedDoc.createElement('div').style;
  dynamicDetached.setProperty('animation-duration', 'calc(10s + (sign(2cqw - 10px) * 5s))');
  dynamicDetached.setProperty('animation-timing-function', 'steps(calc(2 * sibling-index()), jump-none)');
  eq('detached-dynamic-duration', dynamicDetached.getPropertyValue('animation-duration'), 'calc(10s + (5s * sign(2cqw - 10px)))');
  eq('detached-dynamic-timing', dynamicDetached.getPropertyValue('animation-timing-function'), 'steps(calc(2 * sibling-index()), jump-none)');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  rule.style.setProperty('animation', 'slide running forwards alternate 2 4s -1s ease-in', 'important');
  eq('rule-animation', rule.style.getPropertyValue('animation'), '4s ease-in -1s 2 alternate forwards slide');
  eq('rule-animation-priority', rule.style.getPropertyPriority('animation'), 'important');
  eq('rule-duration', rule.style.getPropertyValue('animation-duration'), '4s');
  eq('rule-delay', rule.style.getPropertyValue('animation-delay'), '-1s');
  eq('rule-timeline', rule.style.getPropertyValue('animation-timeline'), 'auto');
  hasAll('rule-animation-names', rule.style, animationLonghands);
  ok('rule-cssText-animation', rule.cssText.includes('animation: 4s ease-in -1s 2 alternate forwards slide !important;'));
  rule.style.setProperty('--token', 'value');
  rule.style.animationTimingFunction = 'linear(0, 1)';
  eq('rule-animation-after-timing', rule.style.getPropertyValue('animation'), '');
  eq('rule-timing-after-timing', rule.style.getPropertyValue('animation-timing-function'), 'linear(0, 1)');
  const removedRuleAnimation = rule.style.removeProperty('animation');
  eq('rule-removed-animation', removedRuleAnimation, '');
  eq('rule-animation-after-remove', rule.style.getPropertyValue('animation'), '');
  ok('rule-cssText-token', rule.cssText.includes('--token: value;'));

  const keyframe = sheet.cssRules[1].cssRules[0];
  keyframe.style.setProperty('animation', 'fade 1s linear', 'important');
  keyframe.style.animationDuration = '1s';
  eq('keyframe-animation-ignored', keyframe.style.getPropertyValue('animation'), '');
  eq('keyframe-duration-ignored', keyframe.style.getPropertyValue('animation-duration'), '');
  keyframe.style.animationTimingFunction = 'linear(0, 1)';
  eq('keyframe-timing', keyframe.style.getPropertyValue('animation-timing-function'), 'linear(0, 1)');
  ok('keyframe-timing-name', names(keyframe.style).includes('animation-timing-function'));
  ok('keyframe-cssText-timing', keyframe.cssText.includes('animation-timing-function: linear(0, 1);'));

  const invalid = document.createElement('div').style;
  invalid.animationDuration = '1s';
  invalid.animationDuration = '-2s';
  eq('invalid-duration-preserves-old', invalid.animationDuration, '1s');

  return failures.length ? failures.slice(0, 10).join('|') : 'PASS';
})()
"#,
        )
        .expect("animation shorthand PDB backing should evaluate");

    assert_eq!(result, "PASS");
}

#[test]
fn detached_animation_shorthand_queries_use_pdb_backing() {
    let mut vm = new_storage_test_vm("https://detached-animation-pdb-query.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));

  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const style = doc.createElement('div').style;

  style.cssText = 'animation: fade paused both reverse 3 1s 2s linear !important; --token: value; -webkit-text-fill-color: red;';
  eq('animation', style.getPropertyValue('animation'), '1s linear 2s 3 reverse both paused fade');
  eq('animation-priority', style.getPropertyPriority('animation'), 'important');
  eq('duration', style.getPropertyValue('animation-duration'), '1s');
  eq('range', style.getPropertyValue('animation-range'), 'normal');
  eq('token', style.getPropertyValue('--token'), 'value');
  eq('webkit-transition', style.getPropertyValue('-webkit-text-fill-color'), 'red');

  style.setProperty('animation-range', 'entry 10% exit 20%', 'important');
  eq('range-after-set', style.getPropertyValue('animation-range'), 'entry 10% exit 20%');
  eq('range-priority', style.getPropertyPriority('animation-range'), 'important');
  eq('range-start', style.getPropertyValue('animation-range-start'), 'entry 10%');
  eq('range-end', style.getPropertyValue('animation-range-end'), 'exit 20%');
  ok('range-start-name', names(style).includes('animation-range-start'));
  ok('range-end-name', names(style).includes('animation-range-end'));

  const removedRange = style.removeProperty('animation-range');
  eq('removed-range', removedRange, 'entry 10% exit 20%');
  eq('range-after-remove', style.getPropertyValue('animation-range'), '');
  eq('token-after-remove', style.getPropertyValue('--token'), 'value');
  eq('webkit-after-remove', style.getPropertyValue('-webkit-text-fill-color'), 'red');

  return failures.length ? failures.join('|') : 'PASS';
})()
"#,
        )
        .expect("detached animation PDB query should evaluate");

    assert_eq!(result, "PASS");
}

#[test]
fn outline_shorthand_uses_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://outline-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const outlineLonghands = ['outline-color', 'outline-style', 'outline-width'];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };

  function exerciseMixedStyle(style, label) {
    style.setProperty('outline', '1px solid red', 'important');
    eq(`${label}-outline`, style.getPropertyValue('outline'), 'red solid 1px');
    eq(`${label}-outline-priority`, style.getPropertyPriority('outline'), 'important');
    eq(`${label}-outline-width`, style.getPropertyValue('outline-width'), '1px');
    eq(`${label}-outline-style`, style.getPropertyValue('outline-style'), 'solid');
    eq(`${label}-outline-color`, style.getPropertyValue('outline-color'), 'red');
    eq(`${label}-outline-color-priority`, style.getPropertyPriority('outline-color'), 'important');
    hasAll(`${label}-outline-names`, style, outlineLonghands);
    ok(`${label}-outline-cssText`, style.cssText.includes('outline: red solid 1px !important;'));

    style.setProperty('--token', 'value');
    style.setProperty('-webkit-text-fill-color', 'red');
    style.setProperty('outline-width', '2px', 'important');
    eq(`${label}-outline-after-width`, style.getPropertyValue('outline'), 'red solid 2px');
    eq(`${label}-outline-width-after-width`, style.getPropertyValue('outline-width'), '2px');
    eq(`${label}-outline-width-priority`, style.getPropertyPriority('outline-width'), 'important');
    ok(`${label}-side-token-name`, names(style).includes('--token'));
    ok(`${label}-side-webkit-name`, names(style).includes('-webkit-text-fill-color'));
    eq(`${label}-side-token`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-side-webkit`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    const removedOutline = style.removeProperty('outline');
    eq(`${label}-removed-outline`, removedOutline, 'red solid 2px');
    eq(`${label}-outline-after-remove`, style.getPropertyValue('outline'), '');
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');
  }

  const inline = document.createElement('div').style;
  exerciseMixedStyle(inline, 'inline');
  inline.setProperty('outline-color', 'invert');
  eq('inline-invert-value', inline.getPropertyValue('outline-color'), 'invert');
  ok('inline-invert-cssText', inline.cssText.includes('outline-color: invert;'));

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = detachedDoc.createElement('div').style;
  detached.cssText = 'outline: 3px dotted blue !important; --token: value; -webkit-text-fill-color: red;';
  eq('detached-outline', detached.getPropertyValue('outline'), 'blue dotted 3px');
  eq('detached-outline-priority', detached.getPropertyPriority('outline'), 'important');
  eq('detached-outline-style', detached.getPropertyValue('outline-style'), 'dotted');
  hasAll('detached-outline-names', detached, outlineLonghands);
  eq('detached-token', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  const removedStyle = detached.removeProperty('outline-style');
  eq('detached-removed-style', removedStyle, 'dotted');
  eq('detached-outline-after-style-remove', detached.getPropertyValue('outline'), '');
  eq('detached-token-after-remove', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit-after-remove', detached.getPropertyValue('-webkit-text-fill-color'), 'red');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  rule.style.setProperty('outline', '1px dashed green', 'important');
  eq('rule-outline', rule.style.getPropertyValue('outline'), 'green dashed 1px');
  eq('rule-outline-priority', rule.style.getPropertyPriority('outline'), 'important');
  hasAll('rule-outline-names', rule.style, outlineLonghands);
  ok('rule-cssText-outline', rule.cssText.includes('outline: green dashed 1px !important;'));
  rule.style.setProperty('--token', 'value');
  const removedRuleOutline = rule.style.removeProperty('outline');
  eq('rule-removed-outline', removedRuleOutline, 'green dashed 1px');
  eq('rule-outline-after-remove', rule.style.getPropertyValue('outline'), '');
  eq('rule-token-after-remove', rule.style.getPropertyValue('--token'), 'value');
  ok('rule-cssText-token', rule.cssText.includes('--token: value;'));
  ok('rule-cssText-outline-removed', !rule.cssText.includes('outline'));

  const keyframe = sheet.cssRules[1].cssRules[0];
  keyframe.style.setProperty('outline-width', '4px', 'important');
  eq('keyframe-outline-width', keyframe.style.getPropertyValue('outline-width'), '4px');
  eq('keyframe-outline-width-priority', keyframe.style.getPropertyPriority('outline-width'), 'important');
  ok('keyframe-outline-width-name', names(keyframe.style).includes('outline-width'));
  ok('keyframe-cssText-width', keyframe.cssText.includes('outline-width: 4px !important;'));

  return failures.length ? failures.slice(0, 8).join('|') : 'PASS';
})()
"#,
        )
        .expect("outline shorthand PDB backing should evaluate");

    assert_eq!(result, "PASS");
}

#[test]
fn webkit_text_stroke_shorthand_uses_pdb_backing_across_cssom_surfaces() {
    let mut vm = new_storage_test_vm("https://webkit-text-stroke-pdb-backing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const strokeLonghands = ['-webkit-text-stroke-width', '-webkit-text-stroke-color'];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, condition) => {
    if (!condition) failures.push(label);
  };
  const hasAll = (label, style, expectedNames) => {
    const actual = names(style);
    for (const name of expectedNames) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };

  function exerciseMixedStyle(style, label) {
    style.setProperty('-webkit-text-stroke', '1px red', 'important');
    eq(`${label}-stroke`, style.getPropertyValue('-webkit-text-stroke'), '1px red');
    eq(`${label}-stroke-priority`, style.getPropertyPriority('-webkit-text-stroke'), 'important');
    eq(`${label}-width`, style.getPropertyValue('-webkit-text-stroke-width'), '1px');
    eq(`${label}-color`, style.getPropertyValue('-webkit-text-stroke-color'), 'red');
    eq(`${label}-width-priority`, style.getPropertyPriority('-webkit-text-stroke-width'), 'important');
    hasAll(`${label}-stroke-names`, style, strokeLonghands);
    ok(`${label}-stroke-cssText`, style.cssText.includes('-webkit-text-stroke: 1px red !important;'));

    style.setProperty('--token', 'value');
    style.setProperty('-webkit-text-fill-color', 'red');
    style.setProperty('-webkit-text-stroke-width', '2px', 'important');
    eq(`${label}-stroke-after-width`, style.getPropertyValue('-webkit-text-stroke'), '2px red');
    eq(`${label}-width-after-width`, style.getPropertyValue('-webkit-text-stroke-width'), '2px');
    ok(`${label}-side-token-name`, names(style).includes('--token'));
    ok(`${label}-side-webkit-name`, names(style).includes('-webkit-text-fill-color'));
    eq(`${label}-side-token`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-side-webkit`, style.getPropertyValue('-webkit-text-fill-color'), 'red');

    const removedStroke = style.removeProperty('-webkit-text-stroke');
    eq(`${label}-removed-stroke`, removedStroke, '2px red');
    eq(`${label}-stroke-after-remove`, style.getPropertyValue('-webkit-text-stroke'), '');
    eq(`${label}-width-after-remove`, style.getPropertyValue('-webkit-text-stroke-width'), '');
    eq(`${label}-color-after-remove`, style.getPropertyValue('-webkit-text-stroke-color'), '');
    eq(`${label}-token-after-remove`, style.getPropertyValue('--token'), 'value');
    eq(`${label}-webkit-after-remove`, style.getPropertyValue('-webkit-text-fill-color'), 'red');
  }

  const inline = document.createElement('div').style;
  exerciseMixedStyle(inline, 'inline');

  const detachedDoc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = detachedDoc.createElement('div').style;
  detached.cssText = '-webkit-text-stroke: 3px green !important; --token: value; -webkit-text-fill-color: red;';
  eq('detached-stroke', detached.getPropertyValue('-webkit-text-stroke'), '3px green');
  eq('detached-stroke-priority', detached.getPropertyPriority('-webkit-text-stroke'), 'important');
  eq('detached-width', detached.webkitTextStrokeWidth, '3px');
  eq('detached-color', detached.webkitTextStrokeColor, 'green');
  hasAll('detached-stroke-names', detached, strokeLonghands);
  eq('detached-token', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit', detached.getPropertyValue('-webkit-text-fill-color'), 'red');
  const removedDetachedColor = detached.removeProperty('-webkit-text-stroke-color');
  eq('detached-removed-color', removedDetachedColor, 'green');
  eq('detached-stroke-after-color-remove', detached.getPropertyValue('-webkit-text-stroke'), '');
  eq('detached-token-after-remove', detached.getPropertyValue('--token'), 'value');
  eq('detached-webkit-after-remove', detached.getPropertyValue('-webkit-text-fill-color'), 'red');

  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: black; } @keyframes k { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  rule.style.setProperty('-webkit-text-stroke', '4px blue', 'important');
  eq('rule-stroke', rule.style.getPropertyValue('-webkit-text-stroke'), '4px blue');
  eq('rule-stroke-priority', rule.style.getPropertyPriority('-webkit-text-stroke'), 'important');
  hasAll('rule-stroke-names', rule.style, strokeLonghands);
  ok('rule-cssText-stroke', rule.cssText.includes('-webkit-text-stroke: 4px blue !important;'));
  rule.style.setProperty('--token', 'value');
  const removedRuleStroke = rule.style.removeProperty('-webkit-text-stroke');
  eq('rule-removed-stroke', removedRuleStroke, '4px blue');
  eq('rule-stroke-after-remove', rule.style.getPropertyValue('-webkit-text-stroke'), '');
  ok('rule-cssText-token', rule.cssText.includes('--token: value;'));
  ok('rule-cssText-stroke-removed', !rule.cssText.includes('-webkit-text-stroke'));

  const keyframe = sheet.cssRules[1].cssRules[0];
  keyframe.style.setProperty('-webkit-text-stroke', '5px purple', 'important');
  eq('keyframe-stroke', keyframe.style.getPropertyValue('-webkit-text-stroke'), '5px purple');
  eq('keyframe-stroke-priority', keyframe.style.getPropertyPriority('-webkit-text-stroke'), 'important');
  hasAll('keyframe-stroke-names', keyframe.style, strokeLonghands);
  ok('keyframe-cssText-stroke', keyframe.cssText.includes('-webkit-text-stroke: 5px purple !important;'));

  return failures.length ? failures.slice(0, 10).join('|') : 'PASS';
})()
"#,
        )
        .expect("-webkit-text-stroke shorthand PDB backing should evaluate");

    assert_eq!(result, "PASS");
}

#[test]
fn css_rule_style_pdb_priority_survives_css_text_reset() {
    let mut vm = new_storage_test_vm("https://rule-style-pdb-priority-reset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule('div { color: red; }');
  const rule = sheet.cssRules[0];
  rule.cssText = 'div { display: block; }';
  rule.style.setProperty('opacity', '0.5', 'important');
  return [
    rule.style.getPropertyValue('opacity'),
    rule.style.getPropertyPriority('opacity'),
    rule.style.cssText,
    rule.cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleRule PDB priority should survive cssText reset");

    assert_eq!(
        result,
        "0.5|important|display: block; opacity: 0.5 !important;|div { display: block; opacity: 0.5 !important; }"
    );
}

#[test]
fn css_rule_css_text_reset_uses_pdb_canonical_declarations() {
    let mut vm = new_storage_test_vm("https://rule-style-pdb-canonical-reset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('div { color: red; } @keyframes fade { from { opacity: 0; } }');
  const rule = sheet.cssRules[0];
  const keyframe = sheet.cssRules[1].cssRules[0];

  rule.cssText = 'div { color: rgb(0 128 0 / 50%); width: 0; }';
  keyframe.cssText = 'from { background-color: rgb(0 128 0 / 50%); opacity: 1; }';

  return [
    rule.cssText,
    rule.style.cssText,
    keyframe.cssText,
    keyframe.style.cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSRule cssText reset should canonicalize safe declarations with PDB");

    assert_eq!(
        result,
        "div { color: rgba(0, 128, 0, 0.5); width: 0px; }|color: rgba(0, 128, 0, 0.5); width: 0px;|0% { background-color: rgba(0, 128, 0, 0.5); opacity: 1; }|background-color: rgba(0, 128, 0, 0.5); opacity: 1;"
    );
}

#[test]
fn detached_css_pdb_queries_ignore_unrelated_supplemental_entries() {
    let mut vm = new_storage_test_vm("https://detached-style-pdb-mixed-query.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const style = doc.createElement('div').style;
  style.setProperty('grid-column-start', '1', 'important');
  style.setProperty('grid-column-end', '3', 'important');
  style.setProperty('user-select', 'none');
  style.setProperty('--token', 'value');
    style.setProperty('-webkit-text-fill-color', 'red');

  return [
    style.getPropertyValue('grid-column'),
    style.getPropertyPriority('grid-column'),
    style.getPropertyValue('grid-column-start'),
    style.getPropertyPriority('grid-column-end'),
    style.getPropertyValue('user-select'),
    style.getPropertyValue('--token'),
    style.getPropertyValue('-webkit-text-fill-color')
  ].join('|');
})()
"#,
        )
        .expect("detached PDB queries should ignore unrelated supplemental entries");

    assert_eq!(result, "1 / 3|important|1|important|none|value|red");
}

#[test]
fn detached_css_style_exposes_common_feature_detection_properties() {
    let mut vm = new_storage_test_vm("https://detached-style-feature-detection.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const style = doc.createElement('div').style;
  const probes = [
    'transition' in style,
    'transform' in style,
    'animationName' in style,
    'filter' in style,
    'userSelect' in style,
    'appearance' in style,
    'colorAdjust' in style,
    'WebkitTransition' in style,
    'WebkitTransform' in style,
    'WebkitUserSelect' in style
  ].join(',');
  style.WebkitTransition = 'opacity 1s';
  style.userSelect = 'none';
  style.appearance = 'auto';
  style.colorAdjust = 'exact';
  return [
    probes,
    style.getPropertyValue('transition'),
    style.getPropertyValue('-webkit-transition'),
    style['-webkit-transition'],
    style.WebkitTransition,
    style.getPropertyValue('user-select'),
    style.getPropertyValue('appearance'),
    style.getPropertyValue('color-adjust'),
    style.getPropertyValue('print-color-adjust'),
    style.cssText
  ].join('|');
})()
"#,
        )
        .expect("detached CSSStyleDeclaration should expose feature probes");

    assert_eq!(
        result,
        "true,true,true,true,true,true,true,true,true,true|opacity 1s|opacity 1s|opacity 1s|opacity 1s|none|auto|exact|exact|transition: opacity 1s; user-select: none; appearance: auto; print-color-adjust: exact;"
    );
}

#[test]
fn detached_css_style_standard_idl_batch_uses_pdb_projection() {
    let mut vm = new_storage_test_vm("https://detached-style-standard-idl-pdb.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, value) => {
    if (!value) failures.push(`${label}:${value}`);
  };
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const cases = [
    ['aspectRatio', 'aspect-ratio', '1 / 2', 'banana'],
    ['baselineShift', 'baseline-shift', 'super', 'banana'],
    ['backgroundPosition', 'background-position', 'left top', 'banana'],
    ['backgroundRepeat', 'background-repeat', 'repeat-x', 'banana'],
    ['borderBottomColor', 'border-bottom-color', 'red', 'not-a-color'],
    ['borderBottomStyle', 'border-bottom-style', 'dashed', 'banana'],
    ['borderLeftColor', 'border-left-color', 'red', 'not-a-color'],
    ['borderLeftStyle', 'border-left-style', 'dashed', 'banana'],
    ['borderRightColor', 'border-right-color', 'red', 'not-a-color'],
    ['borderRightStyle', 'border-right-style', 'dashed', 'banana'],
    ['borderTopColor', 'border-top-color', 'red', 'not-a-color'],
    ['borderTopStyle', 'border-top-style', 'dashed', 'banana'],
    ['borderBlockEndColor', 'border-block-end-color', 'red', 'not-a-color'],
    ['borderBlockStartColor', 'border-block-start-color', 'red', 'not-a-color'],
    ['borderInlineEndColor', 'border-inline-end-color', 'red', 'not-a-color'],
    ['borderInlineStartColor', 'border-inline-start-color', 'red', 'not-a-color'],
    ['direction', 'direction', 'rtl', 'sideways'],
    ['flexFlow', 'flex-flow', 'column wrap', 'banana'],
    ['gridColumnStart', 'grid-column-start', 'span 2', '1 2'],
    ['gridColumnEnd', 'grid-column-end', '3', '1 2'],
    ['justifySelf', 'justify-self', 'safe center', 'banana'],
    ['perspective', 'perspective', '12px', 'banana'],
    ['placeContent', 'place-content', 'center start', 'banana'],
    ['readingFlow', 'reading-flow', 'grid-order', 'auto'],
    ['readingOrder', 'reading-order', '-2', '1.5'],
    ['wordSpacing', 'word-spacing', '2px', 'banana'],
    ['writingMode', 'writing-mode', 'vertical-rl', 'horizontal']
  ];

  for (const [idl, property, valid, invalid] of cases) {
    const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
    const detached = doc.createElement('div').style;
    const sheet = new CSSStyleSheet();
    sheet.insertRule('div {}');
    const ruleStyle = sheet.cssRules[0].style;

    ok(`${property}-detached-idl`, idl in detached);
    ok(`${property}-detached-kebab`, property in detached);
    ok(`${property}-rule-idl`, idl in ruleStyle);
    ok(`${property}-rule-kebab`, property in ruleStyle);

    detached[idl] = invalid;
    ruleStyle[idl] = invalid;
    eq(`${property}-invalid-detached-length`, detached.length, 0);
    eq(`${property}-invalid-detached-own`, Object.prototype.hasOwnProperty.call(detached, idl), false);
    eq(`${property}-invalid-rule-length`, ruleStyle.length, 0);
    eq(`${property}-invalid-rule-own`, Object.prototype.hasOwnProperty.call(ruleStyle, idl), false);

    detached[idl] = valid;
    ruleStyle[idl] = valid;
    const detachedValue = detached.getPropertyValue(property);
    const ruleValue = ruleStyle.getPropertyValue(property);
    ok(`${property}-detached-value`, detachedValue.length > 0);
    ok(`${property}-rule-value`, ruleValue.length > 0);
    eq(`${property}-detached-idl-get`, detached[idl], detachedValue);
    eq(`${property}-rule-idl-get`, ruleStyle[idl], ruleValue);
    ok(`${property}-detached-name`, names(detached).length > 0);
    ok(`${property}-rule-name`, names(ruleStyle).length > 0);
    ok(`${property}-detached-cssText`, detached.cssText.includes(`${property}:`));
    ok(`${property}-rule-cssText`, ruleStyle.cssText.includes(`${property}:`));
  }
  return failures.length ? failures.slice(0, 30).join('|') : 'PASS';
})()
"#,
        )
        .expect("detached standard IDL properties should use PDB projection");

    assert_eq!(result, "PASS");
}

#[test]
fn css_style_declaration_exposes_compat_webkit_aliases() {
    let mut vm = new_storage_test_vm("https://css-style-webkit-aliases.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const aliases = [
    'webkitAlignContent',
    'webkitAlignItems',
    'webkitAlignSelf',
    'webkitAnimation',
    'webkitAnimationDelay',
    'webkitAnimationDirection',
    'webkitAnimationDuration',
    'webkitAnimationFillMode',
    'webkitAnimationIterationCount',
    'webkitAnimationName',
    'webkitAnimationPlayState',
    'webkitAnimationTimingFunction',
    'webkitBackfaceVisibility',
    'WebKitBackgroundClip',
    'webkitBackgroundOrigin',
    'webkitBackgroundSize',
    'webkitBorderBottomLeftRadius',
    'webkitBorderBottomRightRadius',
    'webkitBorderRadius',
    'webkitBorderTopLeftRadius',
    'webkitBorderTopRightRadius',
    'webkitBoxShadow',
    'webkitBoxSizing',
    'webkitFilter',
    'webkitFlex',
    'webkitFlexBasis',
    'webkitFlexDirection',
    'webkitFlexFlow',
    'webkitFlexGrow',
    'webkitFlexShrink',
    'webkitFlexWrap',
    'webkitJustifyContent',
    'webkitMask',
    'webkitMaskBoxImage',
    'webkitMaskBoxImageOutset',
    'webkitMaskBoxImageRepeat',
    'webkitMaskBoxImageSlice',
    'webkitMaskBoxImageSource',
    'webkitMaskBoxImageWidth',
    'webkitMaskClip',
    'webkitMaskComposite',
    'webkitMaskImage',
    'webkitMaskOrigin',
    'webkitMaskPosition',
    'webkitMaskRepeat',
    'webkitMaskSize',
    'webkitOrder',
    'webkitPerspective',
    'webkitPerspectiveOrigin',
    'webkitTransform',
    'webkitTransformOrigin',
    'webkitTransformStyle',
    'webkitTransition',
    'webkitTransitionDelay',
    'webkitTransitionDuration',
    'webkitTransitionProperty',
    'webkitTransitionTimingFunction'
  ];
  const live = document.createElement('div').style;
  const detached = new CSSStyleSheet();
  detached.insertRule('div {}');
  const ruleStyle = detached.cssRules[0].style;
  const missing = aliases.filter(name => !(name in live) || !(name in ruleStyle));
  live.webkitTransition = 'opacity 1s';
  ruleStyle.webkitFilter = 'blur(2px)';
  live.setProperty('-webkit-transform', 'rotate(45deg)');
  ruleStyle.webkitTransform = 'scale(2)';
  live.webkitBorderRadius = '5px';
  ruleStyle.setProperty('-webkit-border-radius', '6px 7px');
  live.webkitPerspective = '12px';
  ruleStyle.setProperty('-webkit-perspective', '13px');
  live.webkitPerspectiveOrigin = '20px 30px';
  ruleStyle.setProperty('-webkit-perspective-origin', 'left top');
  return [
    missing.join(','),
    live.getPropertyValue('transition'),
    live.getPropertyValue('-webkit-transition'),
    live.webkitTransition,
    ruleStyle.getPropertyValue('filter'),
    ruleStyle.webkitFilter,
    live.getPropertyValue('transform'),
    live.getPropertyValue('-webkit-transform'),
    live.webkitTransform,
    live.cssText,
    ruleStyle.getPropertyValue('transform'),
    ruleStyle.getPropertyValue('-webkit-transform'),
    ruleStyle.getPropertyValue('border-radius'),
    ruleStyle.getPropertyValue('-webkit-border-radius'),
    ruleStyle.webkitBorderRadius,
    live.getPropertyValue('perspective'),
    live.getPropertyValue('-webkit-perspective'),
    live.webkitPerspective,
    ruleStyle.getPropertyValue('perspective'),
    ruleStyle.getPropertyValue('-webkit-perspective'),
    ruleStyle.webkitPerspective,
    live.getPropertyValue('perspective-origin'),
    live.getPropertyValue('-webkit-perspective-origin'),
    live.webkitPerspectiveOrigin,
    ruleStyle.getPropertyValue('perspective-origin'),
    ruleStyle.getPropertyValue('-webkit-perspective-origin'),
    ruleStyle.webkitPerspectiveOrigin,
    ruleStyle.cssText
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleDeclaration webkit aliases should evaluate");

    assert_eq!(
        result,
        "|opacity 1s|opacity 1s|opacity 1s|blur(2px)|blur(2px)|rotate(45deg)|rotate(45deg)|rotate(45deg)|transition: opacity 1s; transform: rotate(45deg); border-radius: 5px; perspective: 12px; perspective-origin: 20px 30px;|scale(2)|scale(2)|6px 7px|6px 7px|6px 7px|12px|12px|12px|13px|13px|13px|20px 30px|20px 30px|20px 30px|left top|left top|left top|filter: blur(2px); transform: scale(2); border-radius: 6px 7px; perspective: 13px; perspective-origin: left top;"
    );
}

#[test]
fn css_style_declaration_webkit_transform_origin_compat_writes_use_pdb_gate() {
    let mut vm = new_storage_test_vm("https://css-style-webkit-transform-origin-gate.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, value) => {
    if (!value) failures.push(`${label}:${value}`);
  };
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const surfaces = () => {
    const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
    const sheet = new CSSStyleSheet();
    sheet.insertRule('div {}');
    return [
      ['live', document.createElement('div').style],
      ['detached', doc.createElement('div').style],
      ['rule', sheet.cssRules[0].style]
    ];
  };

  eq('supports-valid', CSS.supports('-webkit-transform-origin', '20px 30px'), true);
  eq('supports-css-wide', CSS.supports('-webkit-transform-origin', 'inherit'), true);
  eq('supports-invalid', CSS.supports('-webkit-transform-origin', 'banana'), false);

  for (const [label, style] of surfaces()) {
    ok(`${label}-idl`, 'webkitTransformOrigin' in style);
    ok(`${label}-kebab`, '-webkit-transform-origin' in style);

    style.webkitTransformOrigin = 'banana';
    eq(`${label}-invalid-empty`, style.getPropertyValue('-webkit-transform-origin'), '');
    eq(`${label}-invalid-length`, style.length, 0);
    eq(`${label}-invalid-own`, Object.prototype.hasOwnProperty.call(style, 'webkitTransformOrigin'), false);

    style.webkitTransformOrigin = '20px 30px';
    eq(`${label}-value`, style.getPropertyValue('-webkit-transform-origin'), '20px 30px');
    eq(`${label}-idl-get`, style.webkitTransformOrigin, '20px 30px');
    ok(`${label}-name`, names(style).includes('-webkit-transform-origin'));
    ok(`${label}-cssText`, style.cssText.includes('-webkit-transform-origin: 20px 30px;'));

    style.setProperty('-webkit-transform-origin', 'banana', 'important');
    eq(`${label}-invalid-preserves-value`, style.getPropertyValue('-webkit-transform-origin'), '20px 30px');
    eq(`${label}-invalid-preserves-priority`, style.getPropertyPriority('-webkit-transform-origin'), '');

    style.setProperty('-webkit-transform-origin', 'inherit', 'important');
    eq(`${label}-css-wide-value`, style.getPropertyValue('-webkit-transform-origin'), 'inherit');
    eq(`${label}-css-wide-priority`, style.getPropertyPriority('-webkit-transform-origin'), 'important');
    style.removeProperty('-webkit-transform-origin');
    eq(`${label}-removed`, style.getPropertyValue('-webkit-transform-origin'), '');
  }
  return failures.length ? failures.slice(0, 30).join('|') : 'PASS';
})()
"#,
        )
        .expect("-webkit-transform-origin compat CSSOM writes should evaluate");

    assert_eq!(result, "PASS");
}

#[test]
fn css_style_declaration_webkit_text_fill_color_writes_use_stylo_pdb() {
    let mut vm = new_storage_test_vm("https://css-style-webkit-text-fill-color-stylo.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, value) => {
    if (!value) failures.push(`${label}:${value}`);
  };
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const property = '-webkit-text-fill-color';
  const surfaces = () => {
    const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
    const sheet = new CSSStyleSheet();
    sheet.insertRule('div {}');
    return [
      ['live', document.createElement('div').style],
      ['detached', doc.createElement('div').style],
      ['rule', sheet.cssRules[0].style]
    ];
  };

  eq('supports-valid', CSS.supports(property, 'red'), true);
  eq('supports-css-wide', CSS.supports(property, 'inherit'), true);
  eq('supports-invalid', CSS.supports(property, 'not-a-color'), false);

  for (const [label, style] of surfaces()) {
    ok(`${label}-kebab`, property in style);

    style[property] = 'not-a-color';
    eq(`${label}-invalid-empty`, style.getPropertyValue(property), '');
    eq(`${label}-invalid-length`, style.length, 0);
    eq(`${label}-invalid-own`, Object.prototype.hasOwnProperty.call(style, property), false);

    style.setProperty(property, 'red');
    eq(`${label}-value`, style.getPropertyValue(property), 'red');
    eq(`${label}-property-get`, style[property], 'red');
    ok(`${label}-name`, names(style).includes(property));
    ok(`${label}-cssText`, style.cssText.includes(`${property}: red;`));

    style.setProperty(property, 'not-a-color', 'important');
    eq(`${label}-invalid-preserves-value`, style.getPropertyValue(property), 'red');
    eq(`${label}-invalid-preserves-priority`, style.getPropertyPriority(property), '');

    style.setProperty(property, 'inherit', 'important');
    eq(`${label}-css-wide-value`, style.getPropertyValue(property), 'inherit');
    eq(`${label}-css-wide-priority`, style.getPropertyPriority(property), 'important');
    style.removeProperty(property);
    eq(`${label}-removed`, style.getPropertyValue(property), '');
  }
  return failures.length ? failures.slice(0, 30).join('|') : 'PASS';
})()
"#,
        )
        .expect("Stylo-owned -webkit-text-fill-color CSSOM writes should evaluate");

    assert_eq!(result, "PASS");
}

#[test]
fn css_style_declaration_webkit_mask_compat_writes_use_narrow_gate() {
    let mut vm = new_storage_test_vm("https://css-style-webkit-mask-compat-gate.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const ok = (label, value) => {
    if (!value) failures.push(`${label}:${value}`);
  };
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const surfaces = () => {
    const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
    const sheet = new CSSStyleSheet();
    sheet.insertRule('div {}');
    return [
      ['live', document.createElement('div').style],
      ['detached', doc.createElement('div').style],
      ['rule', sheet.cssRules[0].style]
    ];
  };
  const cases = [
    ['webkitMask', '-webkit-mask', 'none'],
    ['webkitMaskBoxImage', '-webkit-mask-box-image', 'none'],
    ['webkitMaskBoxImageOutset', '-webkit-mask-box-image-outset', '0'],
    ['webkitMaskBoxImageRepeat', '-webkit-mask-box-image-repeat', 'stretch'],
    ['webkitMaskBoxImageSlice', '-webkit-mask-box-image-slice', '0'],
    ['webkitMaskBoxImageSource', '-webkit-mask-box-image-source', 'none'],
    ['webkitMaskBoxImageWidth', '-webkit-mask-box-image-width', 'auto'],
    ['webkitMaskClip', '-webkit-mask-clip', 'border-box'],
    ['webkitMaskComposite', '-webkit-mask-composite', 'source-over'],
    ['webkitMaskImage', '-webkit-mask-image', 'none'],
    ['webkitMaskOrigin', '-webkit-mask-origin', 'border-box'],
    ['webkitMaskPosition', '-webkit-mask-position', '0% 0%'],
    ['webkitMaskRepeat', '-webkit-mask-repeat', 'repeat'],
    ['webkitMaskSize', '-webkit-mask-size', 'auto']
  ];

  for (const [idl, property, valid] of cases) {
    eq(`${property}-supports-valid`, CSS.supports(property, valid), true);
    eq(`${property}-supports-invalid`, CSS.supports(property, 'banana'), false);
    for (const [label, style] of surfaces()) {
      ok(`${label}-${property}-idl`, idl in style);
      ok(`${label}-${property}-kebab`, property in style);
      style[idl] = 'banana';
      eq(`${label}-${property}-invalid-empty`, style.getPropertyValue(property), '');
      eq(`${label}-${property}-invalid-length`, style.length, 0);
      eq(`${label}-${property}-invalid-own`, Object.prototype.hasOwnProperty.call(style, idl), false);

      style[idl] = valid;
      eq(`${label}-${property}-value`, style.getPropertyValue(property), valid);
      eq(`${label}-${property}-idl-get`, style[idl], valid);
      ok(`${label}-${property}-name`, names(style).includes(property));
      ok(`${label}-${property}-cssText`, style.cssText.includes(`${property}: ${valid};`));

      style.setProperty(property, 'banana', 'important');
      eq(`${label}-${property}-invalid-preserves-value`, style.getPropertyValue(property), valid);
      eq(`${label}-${property}-invalid-preserves-priority`, style.getPropertyPriority(property), '');

      style.setProperty(property, 'inherit', 'important');
      eq(`${label}-${property}-css-wide-value`, style.getPropertyValue(property), 'inherit');
      eq(`${label}-${property}-css-wide-priority`, style.getPropertyPriority(property), 'important');
      style.removeProperty(property);
      eq(`${label}-${property}-removed`, style.getPropertyValue(property), '');
    }
  }
  return failures.length ? failures.slice(0, 30).join('|') : 'PASS';
})()
"#,
        )
        .expect("-webkit-mask compat CSSOM writes should evaluate");

    assert_eq!(result, "PASS");
}

#[test]
fn css_style_declaration_webkit_appearance_and_user_select_aliases_use_pdb_projection() {
    let mut vm = new_storage_test_vm("https://css-style-webkit-ui-aliases-pdb.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  live.setProperty('-webkit-appearance', 'none');
  live.webkitUserSelect = 'none';

  const doc = new DOMParser().parseFromString('<html><body></body></html>', 'text/html');
  const detached = doc.createElement('div').style;
  detached.cssText = '-webkit-appearance: auto; -webkit-user-select: text;';

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const ruleStyle = sheet.cssRules[0].style;
  ruleStyle.WebkitAppearance = 'none';
  ruleStyle.setProperty('-webkit-user-select', 'all');

  const invalid = document.createElement('div').style;
  invalid.setProperty('-webkit-appearance', 'banana');
  invalid.setProperty('-webkit-user-select', 'banana');

  return [
    CSS.supports('-webkit-appearance', 'none'),
    CSS.supports('-webkit-appearance', 'banana'),
    CSS.supports('-webkit-user-select', 'all'),
    CSS.supports('-webkit-user-select', 'banana'),
    'webkitAppearance' in live,
    'WebkitAppearance' in live,
    'WebKitAppearance' in live,
    'webkitUserSelect' in live,
    'WebkitUserSelect' in live,
    'WebKitUserSelect' in live,
    live.length,
    Array.from({ length: live.length }, (_, index) => live.item(index)).join(','),
    live.getPropertyValue('appearance'),
    live.getPropertyValue('-webkit-appearance'),
    live.webkitAppearance,
    live.WebkitAppearance,
    live.getPropertyValue('user-select'),
    live.getPropertyValue('-webkit-user-select'),
    live.webkitUserSelect,
    live.WebkitUserSelect,
    live.cssText,
    detached.length,
    Array.from({ length: detached.length }, (_, index) => detached.item(index)).join(','),
    detached.getPropertyValue('appearance'),
    detached.getPropertyValue('-webkit-appearance'),
    detached.getPropertyValue('user-select'),
    detached.getPropertyValue('-webkit-user-select'),
    detached.cssText,
    ruleStyle.length,
    Array.from({ length: ruleStyle.length }, (_, index) => ruleStyle.item(index)).join(','),
    ruleStyle.getPropertyValue('appearance'),
    ruleStyle.getPropertyValue('-webkit-appearance'),
    ruleStyle.WebkitAppearance,
    ruleStyle.getPropertyValue('user-select'),
    ruleStyle.getPropertyValue('-webkit-user-select'),
    ruleStyle.cssText,
    invalid.length,
    invalid.cssText
  ].join('|');
})()
"#,
        )
        .expect("-webkit-appearance and -webkit-user-select aliases should use PDB projection");

    assert_eq!(
        result,
        "true|false|true|false|true|true|false|true|true|false|2|appearance,user-select|none|none|none|none|none|none|none|none|appearance: none; user-select: none;|2|appearance,user-select|auto|auto|text|text|appearance: auto; user-select: text;|2|appearance,user-select|none|none|none|all|all|appearance: none; user-select: all;|0|"
    );
}

#[test]
fn css_style_declaration_webkit_filter_rejects_invalid_alias_value() {
    let mut vm = new_storage_test_vm("https://css-style-webkit-filter-invalid.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const ruleStyle = sheet.cssRules[0].style;
  live.setProperty('-webkit-filter', 'banana');
  ruleStyle.webkitFilter = 'banana';
  return [
    CSS.supports('-webkit-filter', 'banana'),
    live.length,
    live.getPropertyValue('-webkit-filter'),
    live.cssText,
    ruleStyle.length,
    ruleStyle.getPropertyValue('filter'),
    ruleStyle.webkitFilter,
    ruleStyle.cssText
  ].join('|');
})()
"#,
        )
        .expect("invalid -webkit-filter alias CSSOM write should evaluate");

    assert_eq!(result, "false|0|||0|||");
}

#[test]
fn css_style_declaration_webkit_filter_alias_uses_pdb_projection() {
    let mut vm = new_storage_test_vm("https://css-style-webkit-filter-pdb.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const ruleStyle = sheet.cssRules[0].style;
  live.setProperty('-webkit-filter', 'blur(2px)');
  ruleStyle.webkitFilter = 'grayscale(20%)';
  return [
    CSS.supports('-webkit-filter', 'blur(2px)'),
    'filter' in live,
    '-webkit-filter' in live,
    'webkitFilter' in live,
    live.length,
    live.getPropertyValue('filter'),
    live.getPropertyValue('-webkit-filter'),
    live.webkitFilter,
    live.cssText,
    ruleStyle.length,
    ruleStyle.getPropertyValue('filter'),
    ruleStyle.getPropertyValue('-webkit-filter'),
    ruleStyle.webkitFilter,
    ruleStyle.cssText
  ].join('|');
})()
"#,
        )
        .expect("-webkit-filter alias PDB projection should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|1|blur(2px)|blur(2px)|blur(2px)|filter: blur(2px);|1|grayscale(20%)|grayscale(20%)|grayscale(20%)|filter: grayscale(20%);"
    );
}

#[test]
fn css_style_declaration_webkit_transform_alias_uses_pdb_projection() {
    let mut vm = new_storage_test_vm("https://css-style-webkit-transform-pdb.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const ruleStyle = sheet.cssRules[0].style;
  live.setProperty('-webkit-transform', 'rotate(calc((0.25turn error)))');
  ruleStyle.webkitTransform = 'rotate(calc((0.25turn error)))';
  const invalid = [
    CSS.supports('-webkit-transform', 'rotate(calc((0.25turn error)))'),
    live.length,
    live.cssText,
    ruleStyle.length,
    ruleStyle.cssText
  ].join(',');
  live.setProperty('-webkit-transform', 'rotate(45deg)');
  ruleStyle.webkitTransform = 'scale(2)';
  const valid = [
    CSS.supports('-webkit-transform', 'rotate(45deg)'),
    'transform' in live,
    '-webkit-transform' in live,
    'webkitTransform' in live,
    live.length,
    live.getPropertyValue('transform'),
    live.getPropertyValue('-webkit-transform'),
    live.webkitTransform,
    live.cssText,
    ruleStyle.length,
    ruleStyle.getPropertyValue('transform'),
    ruleStyle.getPropertyValue('-webkit-transform'),
    ruleStyle.cssText
  ].join(',');
  return [invalid, valid].join('|');
})()
"#,
        )
        .expect("-webkit-transform alias PDB projection should evaluate");

    assert_eq!(
        result,
        "false,0,,0,|true,true,true,true,1,rotate(45deg),rotate(45deg),rotate(45deg),transform: rotate(45deg);,1,scale(2),scale(2),transform: scale(2);"
    );
}

#[test]
fn css_style_declaration_webkit_border_radius_alias_uses_pdb_projection() {
    let mut vm = new_storage_test_vm("https://css-style-webkit-border-radius-pdb.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const ruleStyle = sheet.cssRules[0].style;
  live.setProperty('-webkit-border-radius', 'banana');
  ruleStyle.webkitBorderRadius = 'banana';
  const invalid = [
    CSS.supports('-webkit-border-radius', 'banana'),
    live.length,
    live.cssText,
    ruleStyle.length,
    ruleStyle.cssText
  ].join(',');
  live.setProperty('-webkit-border-radius', '3px');
  ruleStyle.webkitBorderRadius = '4px 5px';
  const valid = [
    CSS.supports('-webkit-border-radius', '3px'),
    live.length,
    live.getPropertyValue('border-radius'),
    live.getPropertyValue('-webkit-border-radius'),
    live.webkitBorderRadius,
    live.cssText,
    ruleStyle.length,
    ruleStyle.getPropertyValue('border-radius'),
    ruleStyle.getPropertyValue('-webkit-border-radius'),
    ruleStyle.webkitBorderRadius,
    ruleStyle.cssText
  ].join(',');
  return [invalid, valid].join('|');
})()
"#,
        )
        .expect("-webkit-border-radius alias PDB projection should evaluate");

    assert_eq!(
        result,
        "false,0,,0,|true,4,3px,3px,3px,border-radius: 3px;,4,4px 5px,4px 5px,4px 5px,border-radius: 4px 5px;"
    );
}

#[test]
fn css_style_declaration_webkit_border_radius_longhand_aliases_use_pdb_projection() {
    let mut vm = new_storage_test_vm("https://css-style-webkit-radius-longhands-pdb.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const ruleStyle = sheet.cssRules[0].style;
  live.setProperty('-webkit-border-top-left-radius', 'banana');
  ruleStyle.webkitBorderTopRightRadius = 'banana';
  const invalid = [
    CSS.supports('-webkit-border-top-left-radius', 'banana'),
    live.length,
    live.cssText,
    ruleStyle.length,
    ruleStyle.cssText
  ].join(',');

  live.setProperty('-webkit-border-top-left-radius', '7px');
  live.webkitBorderTopRightRadius = '8px';
  live.setProperty('-webkit-border-bottom-right-radius', '9px');
  live.webkitBorderBottomLeftRadius = '10px';
  ruleStyle.webkitBorderTopLeftRadius = '11px';
  ruleStyle.setProperty('-webkit-border-bottom-right-radius', '12px');
  const valid = [
    CSS.supports('-webkit-border-bottom-left-radius', '10px'),
    'border-top-right-radius' in live,
    '-webkit-border-top-right-radius' in live,
    'webkitBorderTopRightRadius' in live,
    live.length,
    live.getPropertyValue('border-top-left-radius'),
    live.getPropertyValue('-webkit-border-top-left-radius'),
    live.webkitBorderTopLeftRadius,
    live.getPropertyValue('border-top-right-radius'),
    live.getPropertyValue('-webkit-border-top-right-radius'),
    live.webkitBorderTopRightRadius,
    live.getPropertyValue('border-bottom-right-radius'),
    live.getPropertyValue('-webkit-border-bottom-right-radius'),
    live.webkitBorderBottomRightRadius,
    live.getPropertyValue('border-bottom-left-radius'),
    live.getPropertyValue('-webkit-border-bottom-left-radius'),
    live.webkitBorderBottomLeftRadius,
    live.cssText,
    ruleStyle.length,
    ruleStyle.getPropertyValue('border-top-left-radius'),
    ruleStyle.getPropertyValue('-webkit-border-top-left-radius'),
    ruleStyle.webkitBorderTopLeftRadius,
    ruleStyle.getPropertyValue('border-bottom-right-radius'),
    ruleStyle.getPropertyValue('-webkit-border-bottom-right-radius'),
    ruleStyle.webkitBorderBottomRightRadius,
    ruleStyle.cssText
  ].join('|');
  return [invalid, valid].join('||');
})()
"#,
        )
        .expect("-webkit-border-*radius aliases should use PDB projection");

    assert_eq!(
        result,
        "false,0,,0,||true|true|true|true|4|7px|7px|7px|8px|8px|8px|9px|9px|9px|10px|10px|10px|border-radius: 7px 8px 9px 10px;|2|11px|11px|11px|12px|12px|12px|border-top-left-radius: 11px; border-bottom-right-radius: 12px;"
    );
}

#[test]
fn css_style_declaration_webkit_perspective_alias_uses_pdb_projection() {
    let mut vm = new_storage_test_vm("https://css-style-webkit-perspective-pdb.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const ruleStyle = sheet.cssRules[0].style;
  live.setProperty('-webkit-perspective', 'banana');
  ruleStyle.webkitPerspective = 'banana';
  const invalid = [
    CSS.supports('-webkit-perspective', 'banana'),
    live.length,
    live.cssText,
    ruleStyle.length,
    ruleStyle.cssText
  ].join(',');
  live.setProperty('-webkit-perspective', '12px');
  ruleStyle.webkitPerspective = 'none';
  const valid = [
    CSS.supports('-webkit-perspective', '12px'),
    live.length,
    live.getPropertyValue('perspective'),
    live.getPropertyValue('-webkit-perspective'),
    live.webkitPerspective,
    live.cssText,
    ruleStyle.length,
    ruleStyle.getPropertyValue('perspective'),
    ruleStyle.getPropertyValue('-webkit-perspective'),
    ruleStyle.webkitPerspective,
    ruleStyle.cssText
  ].join(',');
  return [invalid, valid].join('|');
})()
"#,
        )
        .expect("-webkit-perspective alias PDB projection should evaluate");

    assert_eq!(
        result,
        "false,0,,0,|true,1,12px,12px,12px,perspective: 12px;,1,none,none,none,perspective: none;"
    );
}

#[test]
fn css_style_declaration_webkit_perspective_origin_alias_uses_pdb_projection() {
    let mut vm = new_storage_test_vm("https://css-style-webkit-perspective-origin-pdb.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const ruleStyle = sheet.cssRules[0].style;
  live.setProperty('-webkit-perspective-origin', 'banana');
  ruleStyle.webkitPerspectiveOrigin = 'banana';
  const invalid = [
    CSS.supports('-webkit-perspective-origin', 'banana'),
    live.length,
    live.cssText,
    ruleStyle.length,
    ruleStyle.cssText
  ].join(',');
  live.setProperty('-webkit-perspective-origin', '20px 30px');
  ruleStyle.webkitPerspectiveOrigin = 'left top';
  const valid = [
    CSS.supports('-webkit-perspective-origin', '20px 30px'),
    'perspective-origin' in live,
    '-webkit-perspective-origin' in live,
    'perspectiveOrigin' in live,
    'webkitPerspectiveOrigin' in live,
    live.length,
    live.getPropertyValue('perspective-origin'),
    live.getPropertyValue('-webkit-perspective-origin'),
    live.perspectiveOrigin,
    live.webkitPerspectiveOrigin,
    live.cssText,
    ruleStyle.length,
    ruleStyle.getPropertyValue('perspective-origin'),
    ruleStyle.getPropertyValue('-webkit-perspective-origin'),
    ruleStyle.perspectiveOrigin,
    ruleStyle.webkitPerspectiveOrigin,
    ruleStyle.cssText
  ].join('|');
  return [invalid, valid].join('||');
})()
"#,
        )
        .expect("-webkit-perspective-origin alias PDB projection should evaluate");

    assert_eq!(
        result,
        "false,0,,0,||true|true|true|true|true|1|20px 30px|20px 30px|20px 30px|20px 30px|perspective-origin: 20px 30px;|1|left top|left top|left top|left top|perspective-origin: left top;"
    );
}

#[test]
fn css_style_declaration_webkit_standard_alias_batch_uses_pdb_projection() {
    let mut vm = new_storage_test_vm("https://css-style-webkit-standard-alias-pdb.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const cases = [
    ['-webkit-align-content', 'center', 'banana', 'align-content', 'webkitAlignContent', 'align-content', 'center'],
    ['-webkit-align-items', 'flex-end', 'banana', 'align-items', 'webkitAlignItems', 'align-items', 'flex-end'],
    ['-webkit-align-self', 'stretch', 'banana', 'align-self', 'webkitAlignSelf', 'align-self', 'stretch'],
    ['-webkit-backface-visibility', 'visible', 'banana', 'backface-visibility', 'webkitBackfaceVisibility', 'backface-visibility', 'visible'],
    ['-webkit-background-clip', 'text', 'banana', 'background-clip', 'webkitBackgroundClip', 'background-clip', 'text'],
    ['-webkit-background-origin', 'content-box', 'banana', 'background-origin', 'webkitBackgroundOrigin', 'background-origin', 'content-box'],
    ['-webkit-background-size', '10px 20px', 'banana', 'background-size', 'webkitBackgroundSize', 'background-size', '10px 20px'],
    ['-webkit-box-shadow', '1px 2px 3px red', 'banana', 'box-shadow', 'webkitBoxShadow', 'box-shadow', 'red 1px 2px 3px'],
    ['-webkit-box-sizing', 'border-box', 'banana', 'box-sizing', 'webkitBoxSizing', 'box-sizing', 'border-box'],
    ['-webkit-flex', '1 2 3px', 'banana ???', 'flex', 'webkitFlex', 'flex-grow', '1 2 3px'],
    ['-webkit-flex-basis', '12px', 'banana', 'flex-basis', 'webkitFlexBasis', 'flex-basis', '12px'],
    ['-webkit-flex-direction', 'column', 'banana', 'flex-direction', 'webkitFlexDirection', 'flex-direction', 'column'],
    ['-webkit-flex-flow', 'column wrap', 'banana', 'flex-flow', 'webkitFlexFlow', 'flex-direction', 'column wrap'],
    ['-webkit-flex-grow', '2', 'banana', 'flex-grow', 'webkitFlexGrow', 'flex-grow', '2'],
    ['-webkit-flex-shrink', '3', 'banana', 'flex-shrink', 'webkitFlexShrink', 'flex-shrink', '3'],
    ['-webkit-flex-wrap', 'wrap', 'banana', 'flex-wrap', 'webkitFlexWrap', 'flex-wrap', 'wrap'],
    ['-webkit-justify-content', 'center', 'banana', 'justify-content', 'webkitJustifyContent', 'justify-content', 'center'],
    ['-webkit-order', '2', 'banana', 'order', 'webkitOrder', 'order', '2'],
    ['-webkit-transform-style', 'preserve-3d', 'banana', 'transform-style', 'webkitTransformStyle', 'transform-style', 'preserve-3d'],
    ['-webkit-transition', 'opacity 1s', '1s 2s 3s', 'transition', 'webkitTransition', 'transition-property', 'opacity 1s'],
    ['-webkit-transition-delay', '2s', '1px', 'transition-delay', 'webkitTransitionDelay', 'transition-delay', '2s'],
    ['-webkit-transition-duration', '3s', '1px', 'transition-duration', 'webkitTransitionDuration', 'transition-duration', '3s'],
    ['-webkit-transition-property', 'opacity', '123', 'transition-property', 'webkitTransitionProperty', 'transition-property', 'opacity'],
    ['-webkit-transition-timing-function', 'ease-in-out', 'banana', 'transition-timing-function', 'webkitTransitionTimingFunction', 'transition-timing-function', 'ease-in-out']
  ];
  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const ruleStyle = sheet.cssRules[0].style;
  for (const [prefixed, valid, invalid, standard, idl, firstName, expectedValue] of cases) {
    const live = document.createElement('div').style;
    live.setProperty(prefixed, invalid);
    ruleStyle[idl] = invalid;
    eq(`${prefixed}-invalid-supports`, CSS.supports(prefixed, invalid), false);
    eq(`${prefixed}-invalid-live-length`, live.length, 0);
    eq(`${prefixed}-invalid-rule-standard`, ruleStyle.getPropertyValue(standard), '');
    eq(`${prefixed}-invalid-rule-cssText`, ruleStyle.cssText, '');

    live.setProperty(prefixed, valid);
    ruleStyle[idl] = valid;
    eq(`${prefixed}-valid-supports`, CSS.supports(prefixed, valid), true);
    eq(`${prefixed}-live-first`, live[0], firstName);
    eq(`${prefixed}-live-standard`, live.getPropertyValue(standard), expectedValue);
    eq(`${prefixed}-live-prefixed`, live.getPropertyValue(prefixed), expectedValue);
    eq(`${prefixed}-live-idl`, live[idl], expectedValue);
    eq(`${prefixed}-live-cssText`, live.cssText, `${standard}: ${expectedValue};`);
    eq(`${prefixed}-rule-standard`, ruleStyle.getPropertyValue(standard), expectedValue);
    eq(`${prefixed}-rule-prefixed`, ruleStyle.getPropertyValue(prefixed), expectedValue);
    eq(`${prefixed}-rule-idl`, ruleStyle[idl], expectedValue);
    eq(`${prefixed}-rule-cssText`, ruleStyle.cssText, `${standard}: ${expectedValue};`);
    ruleStyle.removeProperty(standard);
  }
  return failures.length ? failures.slice(0, 20).join('|') : 'PASS';
})()
"#,
        )
        .expect("WebKit standard aliases should use PDB projection");

    assert_eq!(result, "PASS");
}

#[test]
fn css_rule_source_webkit_standard_aliases_seed_pdb_block() {
    let mut vm = new_storage_test_vm("https://css-rule-webkit-source-alias-pdb.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.insertRule('div { -webkit-transition: opacity 1s; -webkit-box-shadow: 1px 2px 3px red; -webkit-backface-visibility: visible; -webkit-background-clip: text; -webkit-transform-style: preserve-3d; }');
  const rule = sheet.cssRules[0];
  const before = [
    rule.style.length,
    rule.style.item(0),
    rule.style.getPropertyValue('transition'),
    rule.style.getPropertyValue('-webkit-transition'),
    rule.style.item(5),
    rule.style.getPropertyValue('box-shadow'),
    rule.style.getPropertyValue('-webkit-box-shadow'),
    rule.style.item(6),
    rule.style.getPropertyValue('backface-visibility'),
    rule.style.getPropertyValue('-webkit-backface-visibility'),
    rule.style.item(7),
    rule.style.getPropertyValue('background-clip'),
    rule.style.getPropertyValue('-webkit-background-clip'),
    rule.style.item(8),
    rule.style.getPropertyValue('transform-style'),
    rule.style.getPropertyValue('-webkit-transform-style'),
    rule.cssText
  ].join('|');
  const removed = rule.style.removeProperty('-webkit-transition');
  const after = [
    removed,
    rule.style.length,
    rule.style.getPropertyValue('transition'),
    rule.cssText
  ].join('|');
  return [before, after].join('||');
})()
"#,
        )
        .expect("rule source WebKit standard aliases should seed PDB block");

    assert_eq!(
        result,
        "9|transition-property|opacity 1s|opacity 1s|box-shadow|red 1px 2px 3px|red 1px 2px 3px|backface-visibility|visible|visible|background-clip|text|text|transform-style|preserve-3d|preserve-3d|div { transition: opacity 1s; box-shadow: red 1px 2px 3px; backface-visibility: visible; background-clip: text; transform-style: preserve-3d; }||opacity 1s|4||div { box-shadow: red 1px 2px 3px; backface-visibility: visible; background-clip: text; transform-style: preserve-3d; }"
    );
}

#[test]
fn css_style_declaration_webkit_animation_alias_uses_pdb_projection() {
    let mut vm = new_storage_test_vm("https://css-style-webkit-animation-pdb.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const failures = [];
  const eq = (label, actual, expected) => {
    if (actual !== expected) failures.push(`${label}:${actual}!=${expected}`);
  };
  const animationLonghands = [
    'animation-duration',
    'animation-timing-function',
    'animation-delay',
    'animation-iteration-count',
    'animation-direction',
    'animation-fill-mode',
    'animation-play-state',
    'animation-name',
    'animation-timeline',
    'animation-range-start',
    'animation-range-end'
  ];
  const names = style => Array.from({ length: style.length }, (_, index) => style.item(index));
  const hasAll = (label, style) => {
    const actual = names(style);
    for (const name of animationLonghands) {
      if (!actual.includes(name)) failures.push(`${label}:missing:${name}:${actual.join(',')}`);
    }
  };

  const value = 'fade 1s linear 2s 3 reverse both paused';
  const expected = '1s linear 2s 3 reverse both paused fade';
  const invalid = document.createElement('div').style;
  invalid.setProperty('-webkit-animation', 'banana ???');
  eq('invalid-supports', CSS.supports('-webkit-animation', 'banana ???'), false);
  eq('invalid-length', invalid.length, 0);

  const live = document.createElement('div').style;
  live.setProperty('-webkit-animation', value, 'important');
  eq('live-supports', CSS.supports('-webkit-animation', value), true);
  eq('live-length', live.length, 11);
  eq('live-item0', live.item(0), 'animation-duration');
  eq('live-animation', live.getPropertyValue('animation'), expected);
  eq('live-webkit-animation', live.getPropertyValue('-webkit-animation'), expected);
  eq('live-idl', live.webkitAnimation, expected);
  eq('live-priority', live.getPropertyPriority('-webkit-animation'), 'important');
  hasAll('live-names', live);
  eq('live-cssText', live.cssText, `animation: ${expected} !important;`);
  const removed = live.removeProperty('-webkit-animation');
  eq('live-removed', removed, expected);
  eq('live-after-length', live.length, 0);

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div { -webkit-animation: fade 1s linear 2s 3 reverse both paused; }');
  const rule = sheet.cssRules[0];
  eq('rule-length', rule.style.length, 11);
  eq('rule-item0', rule.style.item(0), 'animation-duration');
  eq('rule-animation', rule.style.getPropertyValue('animation'), expected);
  eq('rule-webkit-animation', rule.style.getPropertyValue('-webkit-animation'), expected);
  eq('rule-idl', rule.style.webkitAnimation, expected);
  hasAll('rule-names', rule.style);
  eq('rule-cssText', rule.cssText, `div { animation: ${expected}; }`);
  const ruleRemoved = rule.style.removeProperty('-webkit-animation');
  eq('rule-removed', ruleRemoved, expected);
  eq('rule-after-length', rule.style.length, 0);
  eq('rule-after-cssText', rule.cssText, 'div { }');

  return failures.length ? failures.slice(0, 20).join('|') : 'PASS';
})()
"#,
        )
        .expect("-webkit-animation alias PDB projection should evaluate");

    assert_eq!(result, "PASS");
}

#[test]
fn css_style_declaration_webkit_aliases_match_css_supports_surface() {
    let mut vm = new_storage_test_vm("https://css-style-webkit-supports-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('div').style;
  const probes = [
    CSS.supports('-webkit-align-content', 'inherit'),
    '-webkit-align-content' in style,
    'WebkitAlignContent' in style,
    'webkitAlignContent' in style,
    CSS.supports('-webkit-background-clip', 'inherit'),
    '-webkit-background-clip' in style,
    'WebkitBackgroundClip' in style,
    'webkitBackgroundClip' in style,
    'WebKitBackgroundClip' in style
  ].join(',');
  style.WebkitAlignContent = 'center';
  const align = [
    style.getPropertyValue('-webkit-align-content'),
    style['-webkit-align-content'],
    style.WebkitAlignContent,
    style.webkitAlignContent
  ].join(',');
  return [probes, align].join('|');
})()
"#,
        )
        .expect("CSSStyleDeclaration webkit alias surface should match CSS.supports");

    assert_eq!(
        result,
        "true,true,true,true,true,true,true,true,true|center,center,center,center"
    );
}

#[test]
fn css_style_declaration_rejects_moz_user_select_compat_alias() {
    let mut vm = new_storage_test_vm("https://css-style-moz-user-select-unsupported.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const ruleStyle = sheet.cssRules[0].style;
  live.setProperty('-moz-user-select', 'none');
  ruleStyle.setProperty('-moz-user-select', 'none');
  return [
    CSS.supports('-moz-user-select', 'none'),
    '-moz-user-select' in live,
    live.getPropertyValue('-moz-user-select'),
    live.length,
    live.cssText,
    ruleStyle.getPropertyValue('-moz-user-select'),
    ruleStyle.length,
    ruleStyle.cssText
  ].join('|');
})()
"#,
        )
        .expect("-moz-user-select unsupported CSSOM surface should evaluate");

    assert_eq!(result, "false|false||0|||0|");
}

#[test]
fn css_namespace_to_string_tag_descriptor_matches_cssom() {
    let mut vm = new_storage_test_vm("https://css-namespace-tag.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const descriptor = Object.getOwnPropertyDescriptor(CSS, Symbol.toStringTag);
  const before = [
    Object.prototype.hasOwnProperty.call(CSS, Symbol.toStringTag),
    descriptor.value,
    descriptor.writable,
    descriptor.enumerable,
    descriptor.configurable,
    Object.prototype.toString.call(CSS)
  ].join(',');
  Object.defineProperty(CSS, Symbol.toStringTag, { value: 'Other' });
  const afterDefine = Object.prototype.toString.call(CSS);
  const deleted = delete CSS[Symbol.toStringTag];
  return [before, afterDefine, deleted, Symbol.toStringTag in CSS].join('|');
})()
"#,
        )
        .expect("CSS namespace toStringTag descriptor should evaluate");

    assert_eq!(
        result,
        "true,CSS,false,false,true,[object CSS]|[object Other]|true|false"
    );
}
#[test]
fn css_style_declaration_exposes_iterator() {
    let mut vm = new_storage_test_vm("https://css-style-iterator.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  live.cssText = 'color: red; margin-top: 1px;';
  const detached = new DOMParser()
    .parseFromString('<html><body></body></html>', 'text/html')
    .createElement('div')
    .style;
  detached.cssText = 'display: block; opacity: 0.5;';
  const descriptor = Object.getOwnPropertyDescriptor(
    CSSStyleDeclaration.prototype,
    Symbol.iterator
  );
  return [
    Symbol.iterator in CSSStyleDeclaration.prototype,
    typeof descriptor.value,
    descriptor.value.name,
    descriptor.value.length,
    Object.hasOwn(CSSStyleDeclaration.prototype, 'values'),
    descriptor.value === CSSStyleDeclaration.prototype[Symbol.iterator],
    descriptor.enumerable,
    descriptor.writable,
    descriptor.configurable,
    [...live].join(','),
    [...detached].join(','),
    detached[0],
    detached[1]
  ].join('|');
})()
"#,
        )
        .expect("CSSStyleDeclaration iterator should evaluate");

    assert_eq!(
        result,
        "true|function|values|0|false|true|false|true|true|color,margin-top|display,opacity|display|opacity"
    );
}
