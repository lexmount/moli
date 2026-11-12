use super::*;

#[test]
fn monospace_default_size_distinguishes_generic_from_named_or_fallback_families() {
    let mut vm = new_parsed_test_vm(
        "https://monospace-default-size.test/",
        "<!doctype html><body></body>",
    );
    let actual = vm
        .eval(
            r#"JSON.stringify([
              'monospace', 'serif', 'sans-serif', 'system-ui',
              '"monospace"', '"Missing Font", monospace', 'monospace, serif'
            ].map(fontFamily => ['', 'medium', '16px', '20px'].map(fontSize => {
              const element = document.createElement('span');
              Object.assign(element.style, {fontFamily, fontSize});
              document.body.append(element);
              return parseFloat(getComputedStyle(element).fontSize);
            })))"#,
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
        serde_json::json!([
            [13, 13, 16, 20],
            [16, 16, 16, 20],
            [16, 16, 16, 20],
            [16, 16, 16, 20],
            [16, 16, 16, 20],
            [16, 16, 16, 20],
            [16, 16, 16, 20],
        ])
    );
}

#[test]
fn monospace_font_size_preserves_keyword_and_numeric_inheritance_distinction() {
    let mut vm = new_parsed_test_vm(
        "https://monospace-size-inheritance.test/",
        "<!doctype html><body></body>",
    );
    let actual = vm
        .eval(
            r#"JSON.stringify(['', '16px', '20px', 'medium', 'large', '125%']
              .map(parentSize => ['', 'inherit', 'medium', '100%', '1em', '1rem']
                .map(fontSize => {
                  const parent = document.createElement('div');
                  parent.style.fontSize = parentSize;
                  const child = document.createElement('span');
                  Object.assign(child.style, {fontFamily: 'monospace', fontSize});
                  parent.append(child);
                  document.body.append(parent);
                  return parseFloat(getComputedStyle(child).fontSize);
                })))"#,
        )
        .unwrap();
    // Chromium re-resolves an inherited keyword against the fixed-font table.
    // An explicit percentage/em instead scales the numeric parent size: a
    // `large` (18px) parent therefore yields 18 * 13 / 16 = 14.625px, not 16px.
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
        serde_json::json!([
            [13, 13, 13, 13, 13, 16],
            [16, 16, 13, 16, 16, 16],
            [20, 20, 13, 20, 20, 16],
            [13, 13, 13, 13, 13, 16],
            [16, 16, 13, 14.625, 14.625, 16],
            [16.25, 16.25, 13, 16.25, 16.25, 16],
        ])
    );
}

#[test]
fn monospace_font_size_keeps_pure_percentage_calc_relative_but_length_calc_absolute() {
    let mut vm = new_parsed_test_vm(
        "https://monospace-size-calc.test/",
        "<!doctype html><body></body>",
    );
    let actual = vm
        .eval(
            r#"JSON.stringify(['large', '20px'].map(parentSize => [
              '100%', 'calc(100%)', 'calc(1em)', 'calc(100% + 0px)',
              'min(100%, 125%)', 'max(50%, 100%)', 'calc(50% + 50%)',
              'calc(50% + .5em)', 'clamp(50%, 100%, 125%)', 'calc(0 * 1px + 100%)'
            ].map(fontSize => {
              const parent = document.createElement('div');
              Object.assign(parent.style, {fontFamily: 'serif', fontSize: parentSize});
              const child = document.createElement('span');
              Object.assign(child.style, {fontFamily: 'monospace', fontSize});
              parent.append(child);
              document.body.append(parent);
              return parseFloat(getComputedStyle(child).fontSize);
            })))"#,
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
        serde_json::json!([
            [
                14.625, 14.625, 18, 18, 14.625, 14.625, 14.625, 18, 14.625, 18
            ],
            [20, 20, 20, 20, 20, 20, 20, 20, 20, 20],
        ])
    );
}

#[test]
fn monospace_relative_size_survives_a_switch_back_to_proportional_and_back_again() {
    let mut vm = new_parsed_test_vm(
        "https://monospace-size-three-generations.test/",
        "<!doctype html><body></body>",
    );
    let actual = vm
        .eval(
            r#"JSON.stringify(['', 'medium', '125%'].map(parentSize =>
              ['100%', '1em', 'calc(100%)'].map(fontSize => {
                const parent = document.createElement('div');
                Object.assign(parent.style, {fontFamily: 'monospace', fontSize: parentSize});
                const child = document.createElement('span');
                Object.assign(child.style, {fontFamily: 'serif', fontSize});
                const descendant = document.createElement('span');
                descendant.style.fontFamily = 'monospace';
                child.append(descendant);
                parent.append(child);
                document.body.append(parent);
                return [parent, child, descendant].map(element =>
                  parseFloat(getComputedStyle(element).fontSize));
              })))"#,
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
        serde_json::json!([
            [[13, 16, 13], [13, 16, 13], [13, 16, 13]],
            [[13, 16, 13], [13, 16, 13], [13, 16, 13]],
            [[16.25, 20, 16.25], [16.25, 20, 16.25], [16.25, 20, 16.25]],
        ])
    );
}
