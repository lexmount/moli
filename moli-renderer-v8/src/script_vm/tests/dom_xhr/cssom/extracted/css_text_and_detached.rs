use super::*;

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
