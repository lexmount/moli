use super::*;

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
fn computed_font_variant_serializes_from_computed_longhands() {
    let mut vm = new_parsed_test_vm(
        "https://font-variant-computed-shorthand.test/",
        r#"<html><head><style>
          #target { font-variant: small-caps; }
        </style></head><body>
          <div id="target"></div>
          <svg><text id="svg-target" font-variant="small-caps"></text></svg>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.getElementById('target');
  const computed = getComputedStyle(target);
  const values = [computed.fontVariant];

  target.style.fontVariant = 'common-ligatures small-caps oldstyle-nums';
  values.push(computed.fontVariant);
  target.style.fontVariant = 'none';
  values.push(computed.fontVariant);
  target.style.fontVariant = 'normal';
  target.style.fontVariantNumeric = 'tabular-nums diagonal-fractions';
  values.push(computed.fontVariant);
  target.style.fontVariantNumeric = 'normal';
  values.push(computed.fontVariant);
  values.push(getComputedStyle(document.getElementById('svg-target')).fontVariant);
  return values.join('|');
})()
"#,
        )
        .expect("computed font-variant shorthand should evaluate");

    assert_eq!(
        result,
        "small-caps|common-ligatures small-caps oldstyle-nums|none|tabular-nums diagonal-fractions|normal|small-caps"
    );
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
fn svg_generic_presentation_attributes_apply_to_every_svg_element() {
    let mut vm = new_parsed_test_vm(
        "https://svg-generic-presentation-attributes.test/",
        r#"<!doctype html><svg id="svg"></svg>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const namespace = 'http://www.w3.org/2000/svg';
  const svg = document.getElementById('svg');
  const attributes = [
    ['font-size-adjust', '0.5'],
    ['text-overflow', 'ellipsis'],
    ['white-space', 'pre'],
  ];
  const values = [];

  for (const localName of ['text', 'rect', 'unknown']) {
    const element = document.createElementNS(namespace, localName);
    svg.append(element);
    const before = getComputedStyle(element);
    values.push(...attributes.map(([property]) => before.getPropertyValue(property)));
    for (const [attribute, value] of attributes)
      element.setAttribute(attribute, value);
    const after = getComputedStyle(element);
    values.push(...attributes.map(([property]) => after.getPropertyValue(property)));
    for (const [attribute] of attributes)
      element.removeAttribute(attribute);
    const removed = getComputedStyle(element);
    values.push(...attributes.map(([property]) => removed.getPropertyValue(property)));
  }

  return values.join('|');
})()
"#,
        )
        .expect("generic SVG presentation attributes should evaluate");

    assert_eq!(
        result,
        "none|clip|normal|0.5|ellipsis|pre|none|clip|normal|none|clip|normal|0.5|ellipsis|pre|none|clip|normal|none|clip|normal|0.5|ellipsis|pre|none|clip|normal"
    );
}

#[test]
fn computed_mask_serializes_from_computed_longhands() {
    let mut vm = new_parsed_test_vm(
        "https://svg-mask-computed-shorthand.test/path/page.html",
        r#"<!doctype html>
        <style>
          #stylesheet { mask: url(#stylesheet-mask); }
          #layered {
            mask: url(#layered-mask) center / contain no-repeat content-box exclude luminance;
          }
          #cascade { mask: url(#rule-mask); }
        </style>
        <svg id="svg">
          <g id="stylesheet"></g>
          <g id="attribute" mask="url(#attribute-mask)"></g>
          <g id="layered"></g>
          <g id="cascade" mask="url(#attribute-loses)"></g>
        </svg>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const namespace = 'http://www.w3.org/2000/svg';
  const svg = document.getElementById('svg');
  const value = element => getComputedStyle(element).getPropertyValue('mask');
  const dynamic = document.createElementNS(namespace, 'unknown');
  svg.append(dynamic);
  const values = [
    CSS.supports('mask', 'url(#supported)'),
    value(dynamic),
    value(document.getElementById('stylesheet')),
    value(document.getElementById('attribute')),
    value(document.getElementById('layered')),
    value(document.getElementById('cascade')),
  ];

  dynamic.setAttribute('mask', 'url(#dynamic-mask)');
  values.push(value(dynamic));
  dynamic.removeAttribute('mask');
  values.push(value(dynamic));

  dynamic.style.mask =
    'url(#inline-mask) center / contain no-repeat content-box exclude luminance';
  values.push(value(dynamic));
  dynamic.style.mask = 'none';
  dynamic.style.maskImage = 'url(#longhand-mask)';
  dynamic.style.maskMode = 'luminance';
  values.push(value(dynamic));

  return values.join('|');
})()
"#,
        )
        .expect("computed mask shorthand should evaluate");

    assert_eq!(
        result,
        concat!(
            "true|none|",
            "url(\"https://svg-mask-computed-shorthand.test/path/page.html#stylesheet-mask\")|",
            "url(\"https://svg-mask-computed-shorthand.test/path/page.html#attribute-mask\")|",
            "url(\"https://svg-mask-computed-shorthand.test/path/page.html#layered-mask\") ",
            "50% 50% / contain no-repeat content-box exclude luminance|",
            "url(\"https://svg-mask-computed-shorthand.test/path/page.html#rule-mask\")|",
            "url(\"https://svg-mask-computed-shorthand.test/path/page.html#dynamic-mask\")|none|",
            "url(\"https://svg-mask-computed-shorthand.test/path/page.html#inline-mask\") ",
            "50% 50% / contain no-repeat content-box exclude luminance|",
            "url(\"https://svg-mask-computed-shorthand.test/path/page.html#longhand-mask\") luminance",
        )
    );
}

#[test]
fn svg_special_presentation_attributes_follow_element_scopes_and_cascade() {
    let mut vm = new_parsed_test_vm(
        "https://svg-special-presentation-attributes.test/",
        r#"<!doctype html>
        <style>#cascade { transform: scale(3); }</style>
        <svg id="root" width="40" height="20">
          <use id="positioned" x="1" y="2" width="3" height="4"></use>
          <g id="group" transform="translate(200, 0)"></g>
          <symbol id="symbol" transform="translate(100, 0)"></symbol>
          <g id="group-alias" transform="scale(2)" patternTransform="scale(9)"></g>
          <pattern id="pattern" patternTransform="scale(2)" transform="scale(9)"></pattern>
          <linearGradient id="linear" gradientTransform="scale(4)" transform="scale(9)"></linearGradient>
          <radialGradient id="radial" gradientTransform="scale(5)" transform="scale(9)"></radialGradient>
          <g id="irrelevant" x="9" y="10" width="11" height="12"></g>
          <g id="irrelevant-control"></g>
          <path id="path" d="M0,0 L1,1"></path>
          <path id="path-control"></path>
          <animate id="animation" fill="blue"></animate>
          <animate id="animation-control"></animate>
          <svg id="nested" width="1" height="2"></svg>
          <svg id="nested-control"></svg>
          <g id="cascade" transform="scale(2)"></g>
        </svg>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const get = id => document.getElementById(id);
  const value = (id, property) =>
    getComputedStyle(get(id)).getPropertyValue(property);
  const sameProperties = (left, right, properties) =>
    properties.every(property => value(left, property) === value(right, property));

  const values = [
    value('positioned', 'x'),
    value('positioned', 'y'),
    value('positioned', 'width'),
    value('positioned', 'height'),
    value('group', 'transform'),
    value('symbol', 'transform'),
    value('group-alias', 'transform'),
    value('pattern', 'transform'),
    value('linear', 'transform'),
    value('radial', 'transform'),
    sameProperties('irrelevant', 'irrelevant-control', ['x', 'y', 'width', 'height']),
    value('path', 'd') !== value('path-control', 'd'),
    value('animation', 'fill') === value('animation-control', 'fill'),
    sameProperties('nested', 'nested-control', ['width', 'height']),
    value('cascade', 'transform')
  ];

  get('group').setAttribute('transform', 'translate(12, 3)');
  values.push(value('group', 'transform'));
  return JSON.stringify(values);
})()
"#,
        )
        .expect("SVG presentation attributes should evaluate");

    assert_eq!(
        result,
        r#"["1px","2px","3px","4px","matrix(1, 0, 0, 1, 200, 0)","matrix(1, 0, 0, 1, 100, 0)","matrix(2, 0, 0, 2, 0, 0)","matrix(2, 0, 0, 2, 0, 0)","matrix(4, 0, 0, 4, 0, 0)","matrix(5, 0, 0, 5, 0, 0)",true,true,true,true,"matrix(3, 0, 0, 3, 0, 0)","matrix(1, 0, 0, 1, 12, 3)"]"#
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
    let mut vm = new_rendered_test_vm(
        "https://shadow-mouse-offset.test/",
        r#"<html><head></head><body></body></html>"#,
    );

    let result = eval_with_layout_publications(&mut vm,
            r#"
(function* () {
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
yield; // Publish this scene before reading its geometry.
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
yield; // Publish this scene before reading its geometry.
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
  yield; // Publish the slotted scene before dispatching geometry-dependent events.
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

#[test]
fn custom_function_provenance_probe_preserves_the_retained_style_world() {
    let mut vm = new_parsed_test_vm(
        "https://custom-function-retained-world.test/",
        r#"<!doctype html><style>
          @function --answer() { result: 42px; }
          #target { color: rgb(1, 2, 3); --resolved: --answer(); }
        </style><div id=target></div>"#,
    );
    assert_eq!(
        vm.eval("getComputedStyle(document.getElementById('target')).color")
            .expect("ordinary style should establish the retained world"),
        "rgb(1, 2, 3)"
    );
    let document = vm.document_handle_for_test();
    let stylist_identity = vm.retained_stylist_identity_for_document_for_test(document);
    let rebuilds = vm.retained_style_system_rebuild_count_for_document_for_test(document);
    let updates = vm.retained_style_system_update_count_for_document_for_test(document);

    assert_eq!(
        vm.eval(
            "getComputedStyle(document.getElementById('target')).getPropertyValue('--resolved')",
        )
        .expect("custom function should resolve through the isolated provenance probe"),
        "42px"
    );
    assert_eq!(
        vm.retained_stylist_identity_for_document_for_test(document),
        stylist_identity,
        "a compatibility probe must not replace the Document Stylist"
    );
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(document),
        rebuilds,
        "a compatibility probe must not rebuild the retained style world"
    );
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(document),
        updates,
        "a compatibility probe must not flush the retained style world"
    );
    assert_eq!(
        vm.eval("getComputedStyle(document.getElementById('target')).color")
            .expect("the retained world should remain readable after the probe"),
        "rgb(1, 2, 3)"
    );
}

#[test]
fn raw_custom_function_scanner_respects_owner_media_changes() {
    let mut vm = new_parsed_test_vm(
        "https://custom-function-owner-media.test/",
        r#"<!doctype html>
        <style>#target { --actual: --print-only(); }</style>
        <style media="print">
          @function --print-only() { result: 42px; }
        </style>
        <div id="target"></div>"#,
    );

    assert_eq!(
        vm.eval(
            "getComputedStyle(document.getElementById('target')).getPropertyValue('--actual')",
        )
        .expect("screen custom property should evaluate"),
        "--print-only()",
        "a function in a print-only source must not resolve in screen media",
    );

    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides {
        media: Some("print".to_owned()),
        ..Default::default()
    });
    assert_eq!(
        vm.eval(
            "getComputedStyle(document.getElementById('target')).getPropertyValue('--actual')",
        )
        .expect("print custom property should evaluate"),
        "42px",
        "the function must become visible when its owner media becomes effective",
    );

    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides::default());
    assert_eq!(
        vm.eval(
            "getComputedStyle(document.getElementById('target')).getPropertyValue('--actual')",
        )
        .expect("restored screen custom property should evaluate"),
        "--print-only()",
        "leaving print must hide the raw custom function source again",
    );
}

#[test]
fn raw_custom_function_scanner_filters_shadow_owner_media() {
    let mut vm = new_parsed_test_vm(
        "https://shadow-custom-function-owner-media.test/",
        "<!doctype html><body></body>",
    );
    vm.eval(
        r#"(() => {
          const host = document.createElement('section');
          document.body.append(host);
          const shadow = host.attachShadow({ mode: 'open' });
          shadow.innerHTML = `
            <style>#target { --actual: --shadow-print-only(); }</style>
            <style media="print">
              @function --shadow-print-only() { result: 23px; }
            </style>
            <span id="target"></span>`;
          globalThis.__shadowFunctionTarget = shadow.getElementById('target');
        })()"#,
    )
    .expect("shadow custom function fixture should initialize");
    let probe = "getComputedStyle(__shadowFunctionTarget).getPropertyValue('--actual')";

    assert_eq!(
        vm.eval(probe).expect("screen shadow function state"),
        "--shadow-print-only()",
    );
    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides {
        media: Some("print".to_owned()),
        ..Default::default()
    });
    assert_eq!(
        vm.eval(probe).expect("print shadow function state"),
        "23px",
        "a shadow-scoped function must become visible only in its effective owner media",
    );
    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides::default());
    assert_eq!(
        vm.eval(probe).expect("restored shadow function state"),
        "--shadow-print-only()",
    );
}
