use super::*;

#[test]
fn css_container_rule_conditions_preserve_queries_realms_and_frozen_snapshots() {
    let mut vm = new_storage_html_test_vm("https://container-conditions.test/page.html");
    let result = vm
        .eval(
            r#"
(() => {
  let checks = 0, traps = 0;
  const failures = [];
  const equal = (actual, expected, label) => {
    checks++;
    if (actual !== expected) failures.push({label, actual, expected});
  };
  const frame = document.createElement('iframe'); document.body.appendChild(frame);
  const other = frame.contentWindow;
  const realms = [window, other].map((realm, index) => ({
    realm, label: index ? 'child' : 'main',
    arrayPrototype: realm.Array.prototype, objectPrototype: realm.Object.prototype,
    typeErrorPrototype: realm.TypeError.prototype,
  }));
  const corpus = [
    ['@container card (width > 10px) {}', [{name: 'card', query: '(width > 10px)'}]],
    ['@container style(--x > 0) {}', [{name: '', query: 'style(--x > 0)'}]],
    ['@container sidebar {}', [{name: 'sidebar', query: ''}]],
    ['@container card style(--theme: rgb(1,2,3)), (width < 9px) {}',
      [{name: 'card', query: 'style(--theme: rgb(1,2,3))'}, {name: '', query: '(width < 9px)'}]],
    [String.raw`@container \31 card (width > 10px) {}`,
      [{name: String.raw`\31 card`, query: '(width > 10px)'}]],
    [String.raw`@container escaped\,name {}`,
      [{name: String.raw`escaped\,name`, query: ''}]],
    ['@container not ((width > 10px) and (height < 20px)) {}',
      [{name: '', query: 'not ((width > 10px) and (height < 20px))'}]],
    ['@container future-name future(1,2), tail (width > 4px) {}',
      [{name: 'future-name', query: 'future(1,2)'}, {name: 'tail', query: '(width > 4px)'}]],
  ];
  const sheets = [], rules = [], media = [];
  for (const {realm, label} of realms) {
    const sheet = new realm.CSSStyleSheet(); sheets.push(sheet);
    for (const [text, expected] of corpus) {
      const index = sheet.cssRules.length; sheet.insertRule(text, index);
      rules.push({rule: sheet.cssRules[index], expected, label: label + ':' + index});
    }
    sheet.insertRule('@container outer { @container nested style(--x: y) {} }', sheet.cssRules.length);
    rules.push({rule: sheet.cssRules[8].cssRules[0], expected: [{name: 'nested', query: 'style(--x: y)'}], label: label + ':nested'});
    equal(sheet.cssRules.length, 9, label + ': parsed all rules');
    const mediaSheet = new realm.CSSStyleSheet(); mediaSheet.replaceSync('@media all {}');
    media.push(mediaSheet.cssRules[0]);
  }
  const getters = realms.map(record => {
    const descriptor = Object.getOwnPropertyDescriptor(record.realm.CSSContainerRule.prototype, 'conditions');
    equal(typeof descriptor?.get, 'function', record.label + ': getter exists');
    equal(descriptor?.set, undefined, record.label + ': readonly getter');
    equal(!!descriptor && descriptor.enumerable && descriptor.configurable, true, record.label + ': descriptor');
    return {...record, getter: descriptor?.get};
  });
  const invoke = (record, rule) => typeof record.getter === 'function' ? Reflect.apply(record.getter, rule, []) : undefined;
  const serialize = value => Array.isArray(value) ? JSON.stringify(Array.from(value, entry => ({name: entry?.name, query: entry?.query}))) : null;
  const conditionText = expected => expected.map(({name, query}) => name + (name && query ? ' ' : '') + query).join(', ');
  for (const {rule, expected, label} of rules) {
    equal(rule.containerName, expected.length === 1 ? expected[0].name : '', label + ': legacy name');
    equal(rule.containerQuery, expected.length === 1 ? expected[0].query : '', label + ': legacy query');
    equal(rule.conditionText, conditionText(expected), label + ': complete condition text');
  }
  const read = phase => {
    for (const record of getters) {
      for (const {rule, expected, label} of rules) {
        let value;
        try { value = invoke(record, rule); } catch (error) { value = {error: String(error)}; }
        const array = Array.isArray(value);
        equal(serialize(value), JSON.stringify(expected), phase + ':' + record.label + ':' + label + ': values');
        equal(array && Object.isFrozen(value), true, phase + ':' + record.label + ':' + label + ': frozen');
        equal(array && Object.getPrototypeOf(value) === record.arrayPrototype, true, phase + ':' + record.label + ':' + label + ': array realm');
        equal(array && value.length === expected.length && expected.every((_, index) => {
          const entry = value[index];
          if (!entry || Object.getPrototypeOf(entry) !== record.objectPrototype || Object.isFrozen(entry)) return false;
          return Object.keys(entry).join(',') === 'name,query' && ['name', 'query'].every(name => {
            const descriptor = Object.getOwnPropertyDescriptor(entry, name);
            return descriptor && descriptor.enumerable && descriptor.configurable && descriptor.writable && typeof descriptor.value === 'string';
          });
        }), true, phase + ':' + record.label + ':' + label + ': dictionary shape and realm');
      }
    }
  };
  read('live');
  for (const record of getters) {
    const {rule, expected} = rules[0];
    const first = invoke(record, rule), second = invoke(record, rule);
    equal(Array.isArray(first) && first !== second, true, record.label + ': fresh arrays');
    equal(!!first?.[0] && first[0] !== second?.[0], true, record.label + ': fresh dictionaries');
    if (first?.[0]) { first[0].name = 'changed'; first[0].query = 'changed'; }
    equal(serialize(invoke(record, rule)), JSON.stringify(expected), record.label + ': snapshot mutation stays local');
    equal(rule.conditionText, conditionText(expected), record.label + ': snapshot does not mutate rule');
    equal(Array.isArray(first) && Reflect.set(first, 0, {}) === false, true, record.label + ': array index readonly');
    equal(Array.isArray(first) && Reflect.set(first, 'length', 0) === false, true, record.label + ': array length readonly');
    equal(Reflect.set(rule, 'conditions', []) === false, true, record.label + ': attribute readonly');
    if (Object.hasOwn(rule, 'conditions')) delete rule.conditions;
  }
  const handler = new Proxy({}, {get() { traps++; throw new Error('unexpected receiver trap'); }});
  for (const record of getters) {
    const revoked = Proxy.revocable(rules[0].rule, {}); revoked.revoke();
    const invalid = [null, undefined, {}, [], record.realm.CSSContainerRule.prototype,
      record.realm.CSSConditionRule.prototype, Object.create(rules[0].rule),
      ...media, new Proxy(rules[0].rule, handler), revoked.proxy,
      {get cssText() { traps++; throw new Error('unexpected cssText lookup'); }}];
    for (let index = 0; index < invalid.length; index++) {
      let error;
      try { invoke(record, invalid[index]); } catch (caught) { error = caught; }
      equal(!!error && Object.getPrototypeOf(error) === record.typeErrorPrototype, true, record.label + ': invalid receiver ' + index);
    }
  }
  equal(traps, 0, 'receiver checks do not invoke author traps');
  for (const record of getters) {
    const properties = [];
    for (const {realm} of realms) {
      properties.push([realm.Object.prototype, 'name'], [realm.Object.prototype, 'query'],
        [realm.Array.prototype, '0'], [realm.Object, 'freeze']);
    }
    const saved = properties.map(([object, name]) => [object, name, Object.getOwnPropertyDescriptor(object, name)]);
    const define = Object.defineProperty;
    let value, error;
    try {
      for (const [object, name] of properties) {
        if (name === 'freeze') define(object, name, {configurable: true, writable: true, value() { traps++; throw new Error('unexpected Object.freeze'); }});
        else define(object, name, {configurable: true, get() { traps++; throw new Error('unexpected prototype read'); }, set() { traps++; throw new Error('unexpected prototype setter'); }});
      }
      value = invoke(record, rules[0].rule);
    } catch (caught) { error = String(caught); }
    finally {
      for (const [object, name, descriptor] of saved) {
        if (descriptor) define(object, name, descriptor); else delete object[name];
      }
    }
    equal(error, undefined, record.label + ': author hooks not called');
    equal(serialize(value), JSON.stringify(rules[0].expected), record.label + ': dictionaries use data properties');
    equal(traps, 0, record.label + ': no prototype or freeze traps');
  }
  for (const sheet of sheets) while (sheet.cssRules.length) sheet.deleteRule(0);
  read('detached rules');
  for (const {rule} of rules) Object.setPrototypeOf(rule, null);
  read('reprototyped rules');
  frame.remove();
  read('detached frame');
  return JSON.stringify({checks, failures});
})()
"#,
        )
        .expect("container conditions snapshot probe should evaluate");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["checks"], 683, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}
