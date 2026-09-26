use super::*;

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
