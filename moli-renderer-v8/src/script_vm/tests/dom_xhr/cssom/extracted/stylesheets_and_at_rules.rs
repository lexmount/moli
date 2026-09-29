use super::*;

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

        ..Default::default()
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
        2,
        "SVG style processes the empty contents after removal and the inserted replacement text"
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
