use super::*;

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
fn css_style_declaration_accepts_counters_in_content_alt_text() {
    let mut vm = new_storage_test_vm("https://css-style-content-alt-counter.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const live = document.createElement('div').style;
  const values = [
    `"" / counter(cnt)`,
    `"regular text" / "alt text 1" counter(cnt) "alt text 2"`,
    `"regular text" / counter(cnt) "alt text"`,
    `"regular text" / counters(chapter, ".", DECIMAL)`
  ];
  const serialized = values.map(value => {
    live.content = value;
    const read = live.getPropertyValue('content');
    live.content = read;
    return live.getPropertyValue('content');
  });
  live.content = `"regular text" / counter()`;
  const afterInvalid = live.getPropertyValue('content');

  live.cssText = `color: red; content: "" / counter(css-text)`;
  const cssTextContent = live.getPropertyValue('content');
  const cssTextColor = live.getPropertyValue('color');

  const sheet = new CSSStyleSheet();
  sheet.insertRule('div {}');
  const rule = sheet.cssRules[0].style;
  rule.setProperty('content', `"rule" / counter(rule-counter) "alt"`, 'important');
  const ruleValue = rule.getPropertyValue('content');
  const rulePriority = rule.getPropertyPriority('content');
  rule.setProperty('content', `"rule" / url(alt.svg) counter(rule-counter)`);
  const ruleAfterInvalid = rule.getPropertyValue('content');

  return [
    CSS.supports('content', `"" / counter(cnt)`),
    CSS.supports('content', `"" / counters(cnt, ".")`),
    CSS.supports('content', `"" / counter()`),
    serialized.join('~'),
    afterInvalid,
    cssTextContent,
    cssTextColor,
    ruleValue,
    rulePriority,
    ruleAfterInvalid
  ].join('|');
})()
"#,
        )
        .expect("content alt counter CSSOM should evaluate");

    assert_eq!(
        result,
        "true|true|false|\"\" / counter(cnt)~\"regular text\" / \"alt text 1\" counter(cnt) \"alt text 2\"~\"regular text\" / counter(cnt) \"alt text\"~\"regular text\" / counters(chapter, \".\")|\"regular text\" / counters(chapter, \".\")|\"\" / counter(css-text)|red|\"rule\" / counter(rule-counter) \"alt\"|important|\"rule\" / counter(rule-counter) \"alt\""
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
fn css_style_declaration_serializes_white_space_shorthand() {
    let mut vm = new_storage_test_vm("https://css-style-white-space-shorthand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const element = document.createElement('div');
  const values = ['normal', 'pre', 'nowrap', 'pre-wrap', 'pre-line', 'inherit'];
  const attributeValues = values.map(value => {
    element.setAttribute('style', `white-space: ${value}`);
    return [element.style.whiteSpace, element.style.getPropertyValue('white-space')].join('|');
  });
  const propertyValues = values.map(value => {
    element.style.cssText = '';
    element.style.whiteSpace = value;
    return [element.style.whiteSpace, element.style.getPropertyValue('white-space')].join('|');
  });
  return [attributeValues.join(','), propertyValues.join(',')].join('/');
})()
"#,
        )
        .expect("white-space shorthand serialization should evaluate");

    assert_eq!(
        result,
        "normal|normal,pre|pre,nowrap|nowrap,pre-wrap|pre-wrap,pre-line|pre-line,inherit|inherit/normal|normal,pre|pre,nowrap|nowrap,pre-wrap|pre-wrap,pre-line|pre-line,inherit|inherit"
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
