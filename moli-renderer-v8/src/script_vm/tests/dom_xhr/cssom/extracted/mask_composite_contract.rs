use super::*;

#[test]
fn mask_composite_native_storage_retains_grammar_and_projects_only_property_names() {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let mut block = moli_css_parse::parse_declaration_block(
        "-webkit-mask-composite: var(--operator); --tokens: \"mask-composite: untouched;\";",
    );
    assert_eq!(block.item(0).as_deref(), Some("mask-composite"));
    assert!(
        block
            .css_text()
            .starts_with("mask-composite: var(--operator);")
    );
    let input = block.project_style_resolution_names(block.css_text());
    assert!(input.starts_with("-webkit-mask-composite: var(--operator);"));
    assert!(input.contains("--tokens: \"mask-composite: untouched;\";"));
    let selected = block.select_properties(["mask-composite"]).unwrap();
    assert_eq!(
        selected.project_style_resolution_names(selected.css_text()),
        "-webkit-mask-composite: var(--operator);"
    );
    let projection = block.set_property_with_projection("mask-composite", "var(--operator)", false);
    assert_eq!(
        projection.set_result,
        moli_css_parse::CssSetResult::ModifiedExisting
    );
    assert_eq!(projection.affected_names, ["mask-composite"]);
    assert_eq!(projection.stored_names, ["mask-composite"]);
    assert_eq!(
        block.project_style_resolution_names(block.css_text()),
        block.css_text()
    );
    for operator in [
        "clear",
        "copy",
        "source-over",
        "source-in",
        "source-out",
        "source-atop",
        "destination-over",
        "destination-in",
        "destination-out",
        "destination-atop",
        "xor",
        "plus-lighter",
    ] {
        assert_ne!(
            block.set_property("-webkit-mask-composite", operator, false),
            moli_css_parse::CssSetResult::ParseError
        );
        let inner = block.clone().into_inner();
        let declaration = inner.declaration_importance_iter().next().unwrap().0;
        // Removing and appending the composite puts the custom declaration first.
        assert!(
            inner
                .declaration_importance_iter()
                .any(|(declaration, _)| matches!(
                    declaration,
                    style::properties::PropertyDeclaration::MaskComposite(_)
                )),
            "{operator}: {declaration:?}"
        );
        assert_eq!(
            block.property_value("mask-composite").as_deref(),
            Some(operator)
        );
        assert!(
            block
                .project_style_resolution_names(block.css_text())
                .contains(&format!("-webkit-mask-composite: {operator};"))
        );
        assert_eq!(
            block.set_property("mask-composite", operator, false),
            moli_css_parse::CssSetResult::ParseError
        );
    }
}

#[test]
fn cssom_mask_composite_alias_retains_native_operators_and_substitution_grammar() {
    let mut vm = new_storage_html_test_vm("https://mask-composite-contract.test/");
    let result = vm.eval(r#"
(() => {
  let checks = 0; const failures = [];
  const eq = (actual, expected, label) => { checks++; if (actual !== expected) failures.push({label, actual, expected}); };
  const standard = 'mask-composite', alias = '-webkit-mask-composite';
  const regular = ['add', 'subtract', 'intersect', 'exclude'];
  const extended = ['clear', 'copy', 'source-over', 'source-in', 'source-out', 'source-atop', 'destination-over', 'destination-in', 'destination-out', 'destination-atop', 'xor', 'plus-lighter'];
  const make = source => {
    const doc = source === 'detached' ? document.implementation.createHTMLDocument('') : document;
    const element = doc.createElement('div');
    const sheet = new CSSStyleSheet(); sheet.replaceSync('div {}');
    const style = source === 'rule' ? sheet.cssRules[0].style : element.style;
    return {style, seed(property, value, priority) {
      const text = property + ':' + value + (priority ? ' !important' : '') + ';';
      if (source === 'attribute') element.setAttribute('style', text);
      else if (source === 'cssText' || source === 'rule') style.cssText = text;
      else style.setProperty(property, value, priority);
    }};
  };
  for (const value of regular.concat(extended)) {
    eq(CSS.supports(alias, value), true, 'supports alias:' + value);
    eq(CSS.supports(standard, value), regular.includes(value), 'supports standard:' + value);
  }
  for (const source of ['method', 'cssText', 'attribute', 'detached', 'rule']) {
    for (const priority of ['', 'important']) {
      for (const property of [standard, alias]) {
        for (const value of property === alias ? regular.concat(extended) : regular) {
          const base = make(source); base.seed(property, value, priority);
          const style = base.style, label = [source, priority, property, value].join(':');
          eq(style.getPropertyValue(standard), value, label + ': standard value');
          eq(style.getPropertyValue(alias), value, label + ': alias value');
          eq(style.getPropertyPriority(standard), priority, label + ': standard priority');
          eq(style.getPropertyPriority(alias), priority, label + ': alias priority');
          eq(style.item(0), standard, label + ': canonical name');
          eq(style.length, 1, label + ': single declaration');
          eq(style.cssText, standard + ': ' + value + (priority ? ' !important' : '') + ';', label + ': canonical cssText');
          const before = style.cssText;
          style.setProperty(property, 'lighter', priority);
          eq(style.cssText, before, label + ': invalid write atomic');
          style.setProperty(standard, 'source-over');
          eq(style.cssText, before, label + ': standard grammar rejects extended operator');
          style.setProperty(standard, 'exclude');
          eq(style.getPropertyValue(alias), 'exclude', label + ': standard replaces alias');
          eq(style.getPropertyPriority(alias), '', label + ': priority replaced');
          eq(style.removeProperty(alias), 'exclude', label + ': alias removes standard');
          eq(style.length, 0, label + ': shared removal');
        }
      }
    }
    for (const value of ['initial', 'inherit', 'unset', 'revert', 'revert-layer', 'var(--operation)', 'env(test, xor)', 'source-over, xor']) {
      const base = make(source); base.seed(alias, value, 'important');
      const style = base.style, label = source + ': deferred/wide/list:' + value;
      eq(style.getPropertyValue(standard), value, label + ': shared value');
      eq(style.getPropertyValue(alias), value, label + ': alias value');
      eq(style.item(0), standard, label + ': shared name');
      style.setProperty('--tokens', '"mask-composite: untouched;"');
      style.setProperty('border', 'var(--edge)');
      eq(style.getPropertyValue(alias), value, label + ': unrelated mutations preserve node');
      eq(style.getPropertyPriority(alias), 'important', label + ': retained priority');
      eq(style.getPropertyValue('--tokens'), '"mask-composite: untouched;"', label + ': custom tokens');
    }
    const base = make(source); base.seed(alias, 'source-over', '');
    const copy = make(source).style; copy.cssText = base.style.cssText;
    eq(copy.getPropertyValue(standard), '', source + ': standard serialized grammar remains strict');
  }
  for (const keyframes of [false, true]) {
    const sheet = new CSSStyleSheet();
    const body = alias + ': var(--operation, destination-out); --tokens: "mask-composite: untouched;";';
    sheet.replaceSync(keyframes ? '@keyframes spin { from {' + body + '}}' : 'div {' + body + '}');
    const rule = keyframes ? sheet.cssRules[0].cssRules[0] : sheet.cssRules[0];
    eq(rule.cssText.includes('mask-composite: var(--operation, destination-out)'), true, 'unmaterialized rule cssText:' + keyframes);
    eq(rule.style.getPropertyValue(standard), 'var(--operation, destination-out)', 'rule shared variable:' + keyframes);
    eq(rule.style.getPropertyValue('--tokens'), '"mask-composite: untouched;"', 'rule tokens:' + keyframes);
    rule.style.setProperty('color', 'blue');
    eq(rule.style.getPropertyValue(alias), 'var(--operation, destination-out)', 'rule retains native node:' + keyframes);
  }
  for (const source of ['attribute', 'cssText', 'rule']) {
    const element = document.createElement('div');
    const sheet = new CSSStyleSheet(); sheet.replaceSync('div {}');
    const style = source === 'rule' ? sheet.cssRules[0].style : element.style;
    const text = alias + ': source-out !important; border: var(--edge); --tokens: "mask-composite: untouched;";';
    if (source === 'attribute') element.setAttribute('style', text); else style.cssText = text;
    eq(style.getPropertyValue(standard), 'source-out', source + ': raw mixed standard value');
    eq(style.getPropertyValue(alias), 'source-out', source + ': raw mixed alias value');
    eq(style.getPropertyPriority(alias), 'important', source + ': raw mixed priority');
    style.setProperty('--changed', 'yes');
    eq(style.getPropertyValue(alias), 'source-out', source + ': raw mixed mutation');
  }
  const node = document.createElement('div'); document.body.appendChild(node);
  const computed = () => getComputedStyle(node).getPropertyValue(standard);
  for (const value of regular.concat(extended)) {
    node.style.setProperty(alias, value);
    eq(computed(), value, 'computed operator:' + value);
    eq(getComputedStyle(node).getPropertyValue(alias), value, 'computed alias operator:' + value);
  }
  node.style.setProperty('--operation', 'source-out');
  node.style.setProperty(standard, 'var(--operation)');
  eq(computed(), 'add', 'standard substitution keeps standard grammar');
  node.style.setProperty(alias, 'var(--operation)');
  eq(computed(), 'source-out', 'alias substitution changes grammar despite identical cssText');
  node.style.setProperty('--operation', 'destination-in');
  node.style.setProperty('border', 'var(--edge)');
  node.style.setProperty('--tokens', '"mask-composite: untouched;"');
  eq(computed(), 'destination-in', 'alias substitution survives unrelated mutations');
  node.style.removeProperty('--operation');
  node.style.setProperty(alias, 'env(unknown-mask-operator, xor)');
  eq(computed(), 'xor', 'alias environment fallback uses extended grammar');
  node.style.setProperty(standard, 'env(unknown-mask-operator, xor)');
  eq(computed(), 'add', 'standard environment fallback keeps strict grammar');
  node.style.cssText = alias + ': source-atop; --tokens: "mask-composite: untouched;";';
  eq(computed(), 'source-atop', 'attribute path cascades extended operator');
  const styleElement = document.createElement('style');
  node.id = 'mask-composite-contract'; node.style.cssText = '--operation: destination-out';
  styleElement.textContent = '#mask-composite-contract { ' + alias + ': var(--operation) }';
  document.head.appendChild(styleElement);
  eq(computed(), 'destination-out', 'stylesheet variable alias cascades');
  styleElement.sheet.cssRules[0].style.setProperty('color', 'green');
  eq(computed(), 'destination-out', 'stylesheet mutation retains original value grammar');
  node.style.setProperty(alias, 'initial');
  eq(computed(), 'add', 'alias initial value');
  styleElement.remove(); node.remove();
  return JSON.stringify({checks, failures});
})()
"#).expect("mask composite contract should evaluate");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(result["checks"].as_u64().unwrap() > 2800, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}

#[test]
fn mask_composite_owner_initialization_preserves_other_attribute_shorthand_priorities() {
    let mut vm = new_storage_html_test_vm("https://mask-composite-owner.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const values = [
    'background: red;',
    'background-color: blue; background: red !important; background-color: green;',
    'background-color: blue; background: red; background-color: green !important;'
  ];
  return JSON.stringify(values.map(value => {
    const element = document.createElement('div');
    element.setAttribute('style', value);
    return element.style.cssText;
  }));
})()
"#,
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!([
            "background: red;",
            "background: red !important;",
            "background: red; background-color: green !important;"
        ])
    );
}
