use super::*;

#[test]
fn css_font_feature_rule_receivers_use_native_brands_before_conversion() {
    let mut vm = new_parsed_test_vm(
        "https://css-font-feature-receivers.test/",
        "<!doctype html><body><iframe id=child></iframe>",
    );
    assert_eq!(
        vm.eval(include_str!("css_font_feature_rule_receivers.js"))
            .unwrap(),
        "true"
    );
}

#[test]
fn css_font_feature_map_receivers_preserve_cross_realm_native_operations() {
    let mut vm = new_parsed_test_vm(
        "https://css-font-feature-map-receivers.test/",
        "<!doctype html><body><iframe id=child></iframe>",
    );
    assert_eq!(
        vm.eval(include_str!("css_font_feature_map_receivers.js"))
            .unwrap(),
        "true"
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
fn css_font_face_unicode_range_setter_accepts_comments_between_tokens() {
    let mut vm = new_storage_test_vm("https://css-font-face-unicode-range-comments.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('@font-face { font-family: Foo; src: local(Foo); unicode-range: U+1357; }');
  const style = sheet.cssRules[0].style;
  const values = [
    'u/**/+/**/a/**/?',
    'u/**/+0a/**/?',
    'u/**/+0/**/?',
    'u/**/+0/**/-0a',
    'u/**/+0/**/-1',
    'u/**/+/**/?'
  ].map(value => {
    style.setProperty('unicode-range', value);
    return style.getPropertyValue('unicode-range');
  });
  style.setProperty('unicode-range', 'u/**/+a/**/b');
  values.push(style.getPropertyValue('unicode-range'));
  return values.join('|');
})()
"#,
        )
        .expect("CSSFontFaceRule unicode-range setter should accept comments between tokens");

    assert_eq!(result, "U+A0-AF|U+A0-AF|U+0-F|U+0-A|U+0-1|U+0-F|U+0-F");
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
fn live_inline_font_family_getter_normalizes_quoted_family_names() {
    let mut vm = new_storage_test_vm("https://inline-font-family-serialization.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement('div');
  const read = value => {
    target.setAttribute('style', `font-family: ${value}`);
    return [target.style.fontFamily, target.style.getPropertyValue('font-family')];
  };
  return JSON.stringify([
    read("'Twisty Tie'"),
    read("'Veronica'"),
    read("'34J'"),
    read("'serif'")
  ]);
})()
"#,
        )
        .expect("inline font-family serialization should evaluate");

    assert_eq!(
        result,
        r#"[["\"Twisty Tie\"","\"Twisty Tie\""],["Veronica","Veronica"],["\"34J\"","\"34J\""],["\"serif\"","\"serif\""]]"#
    );
}
