use super::*;

#[test]
fn css_property_shells_keep_supports_and_declarations_consistent() {
    let mut vm = new_parsed_test_vm(
        "https://css-property-shells.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(
        vm.eval(include_str!("css_property_shells.js")).unwrap(),
        "true"
    );
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
