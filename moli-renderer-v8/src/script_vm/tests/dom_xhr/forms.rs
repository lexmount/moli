use super::*;

mod dialog;
mod event_activation;

#[test]
fn object_set_custom_validity_updates_validity_state() {
    let mut vm = new_storage_test_vm("https://object-custom-validity.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const object = document.createElement('object');
  const states = [];
  const capture = () => states.push(`${object.validity.customError}:${object.validity.valid}`);

  capture();
  object.setCustomValidity('custom error');
  capture();
  object.setCustomValidity('');
  capture();

  return states.join('|');
})()
"#,
        )
        .expect("object custom validity should evaluate");

    assert_eq!(result, "false:true|true:false|false:true");
}

#[test]
fn readonly_controls_match_validity_pseudo_without_will_validate() {
    let mut vm = new_storage_test_vm("https://readonly-validity-pseudo.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const input = document.createElement('input');
  input.type = 'number';
  input.min = '1';
  input.max = '10';
  input.value = '12';
  input.readOnly = true;
  body.appendChild(input);

  const textarea = document.createElement('textarea');
  textarea.required = true;
  textarea.readOnly = true;
  body.appendChild(textarea);

  return [
    input.willValidate,
    input.matches(':valid'),
    input.matches(':invalid'),
    input.matches(':in-range'),
    input.matches(':out-of-range'),
    textarea.willValidate,
    textarea.matches(':valid'),
    textarea.matches(':invalid')
  ].join('|');
})()
"#,
        )
        .expect("readonly validity pseudo should evaluate");

    assert_eq!(result, "false|true|false|false|false|false|true|false");
}

#[test]
fn form_scalar_setters_apply_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://form-scalar-setters-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      if (value === '') {
        return '<empty>';
      }
      return value === undefined ? 'undefined' : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };

  const input = document.createElement('input');
  const textarea = document.createElement('textarea');
  const form = document.createElement('form');
  const output = document.createElement('output');
  const button = document.createElement('button');
  const fieldset = document.createElement('fieldset');
  const label = document.createElement('label');
  const select = document.createElement('select');
  const option = document.createElement('option');
  option.value = 'a';
  option.text = 'A';
  select.append(option);

  const detached = new DOMParser().parseFromString(
    '<select id="s"><option id="o" value="a">A</option></select>',
    'text/html'
  );
  const detachedSelect = detached.getElementById('s');
  const detachedOption = detached.getElementById('o');

  return [
    probe(() => input.setCustomValidity()),
    probe(() => input.setCustomValidity(Symbol())),
    probe(() => input.setCustomValidity({ toString() { throw new RangeError('validity'); } })),
    probe(() => { input.setCustomValidity('bad\r\nmessage'); return input.validationMessage; }),
    probe(() => { option.value = Symbol(); }),
    probe(() => { option.text = { toString() { throw new RangeError('text'); } }; }),
    probe(() => { option.value = undefined; return option.value; }),
    probe(() => { option.text = null; return option.text; }),
    probe(() => { select.value = Symbol(); }),
    probe(() => { detachedOption.value = Symbol(); }),
    probe(() => { detachedSelect.value = { toString() { throw new RangeError('select'); } }; }),
    probe(() => { detachedOption.value = undefined; return detachedOption.value; }),
    probe(() => { input.value = Symbol(); }),
    probe(() => { input.value = null; return input.value; }),
    probe(() => { input.defaultValue = Symbol(); }),
    probe(() => { input.defaultValue = null; return input.defaultValue; }),
    probe(() => { input.type = { toString() { return 'EMAIL'; } }; return `${input.type}/${input.getAttribute('type')}`; }),
    probe(() => { input.type = Symbol(); }),
    probe(() => { textarea.value = null; return textarea.value; }),
    probe(() => { textarea.value = Symbol(); }),
    probe(() => { textarea.defaultValue = { toString() { throw new RangeError('textarea'); } }; }),
    probe(() => { form.method = Symbol(); }),
    probe(() => { form.method = { toString() { return 'POST'; } }; return `${form.method}/${form.getAttribute('method')}`; }),
    probe(() => { form.method = 'put'; return `${form.method}/${form.getAttribute('method')}`; }),
    probe(() => { form.enctype = 'TEXT/PLAIN'; return `${form.enctype}/${form.getAttribute('enctype')}`; }),
    probe(() => { form.encoding = 'MULTIPART/FORM-DATA'; return `${form.encoding}/${form.getAttribute('enctype')}`; }),
    probe(() => { output.value = Symbol(); }),
    probe(() => { output.value = undefined; return output.value; }),
    probe(() => { output.defaultValue = null; return output.defaultValue; }),
    probe(() => { output.defaultValue = { toString() { throw new RangeError('output'); } }; }),
    probe(() => { input.accept = Symbol(); }),
    probe(() => { input.placeholder = null; return input.placeholder; }),
    probe(() => { input.pattern = { toString() { throw new RangeError('pattern'); } }; }),
    probe(() => { input.formMethod = 'POST'; return `${input.formMethod}/${input.getAttribute('formmethod')}`; }),
    probe(() => { input.formMethod = 'DIALOG'; return `${input.formMethod}/${input.getAttribute('formmethod')}`; }),
    probe(() => { input.formEnctype = 'TEXT/PLAIN'; return `${input.formEnctype}/${input.getAttribute('formenctype')}`; }),
    probe(() => { form.name = null; return form.name; }),
    probe(() => { form.action = { toString() { throw new RangeError('action'); } }; }),
    probe(() => { select.name = Symbol(); }),
    probe(() => { option.name = undefined; return option.name; }),
    probe(() => { label.htmlFor = { toString() { throw new RangeError('label'); } }; }),
    probe(() => { label.htmlFor = undefined; return label.htmlFor; }),
    probe(() => { fieldset.name = Symbol(); }),
    probe(() => { button.type = Symbol(); }),
    probe(() => { button.formMethod = 'DIALOG'; return `${button.formMethod}/${button.getAttribute('formmethod')}`; }),
    probe(() => { button.formEnctype = 'MULTIPART/FORM-DATA'; return `${button.formEnctype}/${button.getAttribute('formenctype')}`; }),
    probe(() => { button.type = 'RESET'; return `${button.type}/${button.getAttribute('type')}`; })
  ].join('|');
})()
"#,
        )
        .expect("form scalar setters should apply WebIDL conversion");

    assert_eq!(
        result,
        "throw:TypeError|throw:TypeError|throw:RangeError|bad\nmessage|throw:TypeError|throw:RangeError|undefined|null|throw:TypeError|throw:TypeError|throw:RangeError|undefined|throw:TypeError|<empty>|throw:TypeError|null|email/EMAIL|throw:TypeError|<empty>|throw:TypeError|throw:RangeError|throw:TypeError|post/POST|get/put|text/plain/TEXT/PLAIN|multipart/form-data/MULTIPART/FORM-DATA|throw:TypeError|undefined|null|throw:RangeError|throw:TypeError|null|throw:RangeError|post/POST|get/DIALOG|text/plain/TEXT/PLAIN|null|throw:RangeError|throw:TypeError|undefined|throw:RangeError|undefined|throw:TypeError|throw:TypeError|dialog/DIALOG|multipart/form-data/MULTIPART/FORM-DATA|reset/RESET"
    );
}
#[test]
fn common_element_reflected_setters_apply_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://common-reflected-setters-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      if (value === '') {
        return '<empty>';
      }
      return value === undefined ? 'undefined' : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };

  const anchor = document.createElement('a');
  const script = document.createElement('script');
  const image = document.createElement('img');
  const quote = document.createElement('blockquote');
  const time = document.createElement('time');
  const track = document.createElement('track');

  return [
    probe(() => { anchor.target = Symbol(); }),
    probe(() => { anchor.type = null; return anchor.type; }),
    probe(() => { anchor.text = { toString() { throw new RangeError('anchor'); } }; }),
    probe(() => { anchor.text = undefined; return anchor.text; }),
    probe(() => { script.type = Symbol(); }),
    probe(() => { script.nonce = null; return script.nonce; }),
    probe(() => { script.text = { toString() { throw new RangeError('script'); } }; }),
    probe(() => { script.text = undefined; return script.text; }),
    probe(() => { image.alt = Symbol(); }),
    probe(() => { image.loading = null; return image.loading; }),
    probe(() => { quote.cite = { toString() { throw new RangeError('quote'); } }; }),
    probe(() => { quote.cite = undefined; return quote.cite; }),
    probe(() => { time.dateTime = Symbol(); }),
    probe(() => { time.dateTime = null; return time.dateTime; }),
    probe(() => { track.kind = Symbol(); }),
    probe(() => { track.kind = 'CAPTIONS'; return `${track.kind}/${track.getAttribute('kind')}`; })
  ].join('|');
})()
"#,
        )
        .expect("common reflected setters should apply WebIDL conversion");

    assert_eq!(
        result,
        "throw:TypeError|null|throw:RangeError|undefined|throw:TypeError|null|throw:RangeError|undefined|throw:TypeError|null|throw:RangeError|https://common-reflected-setters-webidl.test/undefined|throw:TypeError|null|throw:TypeError|captions/CAPTIONS"
    );
}
#[test]
fn table_row_section_index_uses_its_parent_row_collection() {
    let mut vm = new_storage_test_vm("https://table-section-row-index.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const table = document.createElement('table');
  const direct = document.createElement('tr');
  const body = document.createElement('tbody');
  const firstBody = document.createElement('tr');
  const secondBody = document.createElement('tr');
  table.append(direct, body);
  body.append(firstBody, secondBody);
  const html = document.appendChild(document.createElement('html'));
  html.appendChild(document.createElement('body')).appendChild(table);
  return [
    direct.rowIndex,
    direct.sectionRowIndex,
    firstBody.rowIndex,
    firstBody.sectionRowIndex,
    secondBody.rowIndex,
    secondBody.sectionRowIndex
  ].join('|');
})()
"#,
        )
        .expect("table row index semantics should evaluate");

    assert_eq!(result, "0|0|1|0|2|1");
}
#[test]
fn global_element_attribute_setters_apply_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://global-element-attrs-webidl.test/");

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

  const element = document.createElement('div');
  const namedControl = document.createElement('input');
  let idCalls = 0;
  let tabIndexCalls = 0;

  return [
    probe(() => { element.id = Symbol('id'); }),
    probe(() => {
      element.id = {
        toString() {
          idCalls += 1;
          return 'hero';
        }
      };
      return `${element.id}:${element.getAttribute('id')}:${idCalls}`;
    }),
    probe(() => { element.id = { toString() { throw new RangeError('id'); } }; }),
    probe(() => { element.className = null; return element.className; }),
    probe(() => { element.className = undefined; return element.getAttribute('class'); }),
    probe(() => { element.className = Symbol('class'); }),
    probe(() => { namedControl.name = { toString() { return 'field'; } }; return namedControl.name; }),
    probe(() => { namedControl.name = Symbol('name'); }),
    probe(() => { element.dir = null; return element.dir; }),
    probe(() => { element.dir = { toString() { throw new RangeError('dir'); } }; }),
    probe(() => {
      element.tabIndex = {
        valueOf() {
          tabIndexCalls += 1;
          return '4.9';
        }
      };
      return `${element.tabIndex}:${element.getAttribute('tabindex')}:${tabIndexCalls}`;
    }),
    probe(() => { element.tabIndex = null; return element.tabIndex; }),
    probe(() => { element.tabIndex = Symbol('tab'); }),
    probe(() => { element.tabIndex = { valueOf() { throw new RangeError('tab'); } }; })
  ].join('|');
})()
"#,
        )
        .expect("global element attribute setters should apply WebIDL conversion");

    // The `dir` IDL attribute is "limited to only known values" (HTML spec):
    // setter coerces `null` to the JS string "null" and writes it as the
    // content attribute, but the getter canonicalises any non-keyword value
    // back to the empty string. The eighth probe therefore reads "" rather
    // than "null" — getAttribute('dir') still returns "null".
    assert_eq!(
        result,
        "throw:TypeError|hero:hero:1|throw:RangeError|null|undefined|throw:TypeError|field|throw:TypeError||throw:RangeError|4:4:1|0|throw:TypeError|throw:RangeError"
    );
}
#[test]
fn url_reflected_setters_apply_usvstring_conversion() {
    let mut vm = new_storage_test_vm("https://url-reflected-setters-webidl.test/");

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

  const anchor = document.createElement('a');
  const iframe = document.createElement('iframe');
  const image = document.createElement('img');
  const script = document.createElement('script');
  const video = document.createElement('video');
  const codes = value => Array.from(value).map(ch => ch.codePointAt(0)).join(',');

  return [
    probe(() => { anchor.href = Symbol(); }),
    probe(() => { anchor.href = { toString() { throw new RangeError('anchor'); } }; }),
    probe(() => { anchor.href = null; return anchor.getAttribute('href'); }),
    probe(() => { iframe.src = Symbol(); }),
    probe(() => { iframe.src = undefined; return iframe.getAttribute('src'); }),
    probe(() => { iframe.srcdoc = Symbol(); }),
    probe(() => { iframe.srcdoc = null; return iframe.getAttribute('srcdoc'); }),
    probe(() => { image.src = { toString() { throw new RangeError('image'); } }; }),
    probe(() => { image.src = '\uD800x'; return codes(image.getAttribute('src')); }),
    probe(() => { script.src = Symbol(); }),
    probe(() => { script.src = null; return script.getAttribute('src'); }),
    probe(() => { script.src = undefined; return script.getAttribute('src'); }),
    probe(() => { script.src = ''; return `${script.hasAttribute('src')}:${script.getAttribute('src')}`; }),
    probe(() => { script.src = '\uD800script'; return codes(script.getAttribute('src')); }),
    probe(() => { video.poster = Symbol(); }),
    probe(() => { video.poster = '\uD800poster'; return codes(video.getAttribute('poster')); })
  ].join('|');
})()
"#,
        )
        .expect("URL reflected setters should apply WebIDL conversion");

    assert_eq!(
        result,
        "throw:TypeError|throw:RangeError|null|throw:TypeError|undefined|throw:TypeError|null|throw:RangeError|65533,120|throw:TypeError|null|undefined|true:|65533,115,99,114,105,112,116|throw:TypeError|65533,112,111,115,116,101,114"
    );
}
#[test]
fn live_input_pattern_uses_ecmascript_regexp_semantics() {
    let mut vm = new_storage_test_vm("https://live-pattern-regexp.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const same = document.createElement('input');
  same.pattern = '([a-z])\\1';
  same.value = 'aa';
  const samePass = `${same.validity.patternMismatch}/${same.checkValidity()}`;
  same.value = 'ab';
  const sameFail = `${same.validity.patternMismatch}/${same.checkValidity()}`;

  const lookahead = document.createElement('input');
  lookahead.pattern = '(?=.*[0-9]).+';
  lookahead.value = 'abc';
  const lookaheadFail = `${lookahead.validity.patternMismatch}/${lookahead.checkValidity()}`;
  lookahead.value = 'abc1';
  const lookaheadPass = `${lookahead.validity.patternMismatch}/${lookahead.checkValidity()}`;

  const substring = document.createElement('input');
  substring.pattern = 'abc';
  substring.value = 'xabc';
  const fullValue = `${substring.validity.patternMismatch}/${substring.checkValidity()}`;

  return [samePass, sameFail, lookaheadFail, lookaheadPass, fullValue].join('|');
})()
"#,
        )
        .expect("live input pattern should use ECMAScript regexp semantics");

    assert_eq!(
        result,
        "false/true|true/false|true/false|false/true|true/false"
    );
}
#[test]
fn live_input_pattern_uses_native_regexp_after_global_mutation() {
    let mut vm = new_storage_test_vm("https://live-pattern-native-regexp.test/");

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.RegExp = function () {
    return { test: () => false };
  };
  RegExp.prototype.test = function () {
    return false;
  };

  const input = document.createElement('input');
  input.pattern = 'abc';
  input.value = 'abc';
  const matchingValue = `${input.validity.patternMismatch}/${input.checkValidity()}`;

  input.value = 'xabc';
  const partialValue = `${input.validity.patternMismatch}/${input.checkValidity()}`;

  input.pattern = '[';
  input.value = 'anything';
  const invalidPattern = `${input.validity.patternMismatch}/${input.checkValidity()}`;

  return [matchingValue, partialValue, invalidPattern].join('|');
})()
"#,
        )
        .expect("live input pattern should use native v8 regexp");

    assert_eq!(result, "false/true|true/false|false/true");
}
#[test]
fn detached_select_and_option_track_selection_state() {
    let mut vm = new_storage_test_vm("https://detached-select-state.test/path/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body><select id="select">' +
      '<option id="a" value="a">A</option>' +
      '<option id="b" value="b" selected>B</option>' +
      '<option id="c">Three Text</option>' +
    '</select></body></html>',
    'text/html'
  );
  const select = doc.getElementById('select');
  const a = doc.getElementById('a');
  const b = doc.getElementById('b');
  const c = doc.getElementById('c');
  const out = [];
  out.push(select.selectedIndex + ':' + select.value + ':' + [a.selected, b.selected, c.selected].join(','));
  select.selectedIndex = 2;
  out.push(select.selectedIndex + ':' + select.value + ':' + [a.selected, b.selected, c.selected].join(','));
  a.selected = true;
  out.push(select.selectedIndex + ':' + select.value + ':' + [a.selected, b.selected, c.selected].join(','));
  a.selected = false;
  out.push(select.selectedIndex + ':' + select.value + ':' + [a.selected, b.selected, c.selected].join(','));
  select.value = 'b';
  b.defaultSelected = false;
  out.push(select.selectedIndex + ':' + select.value + ':' + [a.selected, b.selected, c.selected].join(',') + ':' + b.defaultSelected + ':' + (b.getAttribute('selected') === null));
  select.value = 'missing';
  out.push(select.selectedIndex + ':' + select.value + ':' + [a.selected, b.selected, c.selected].join(','));
  return out.join('|');
})()
"#,
        )
        .expect("detached select and option should track selection state");

    assert_eq!(
        result,
        "1:b:false,true,false|2:Three Text:false,false,true|0:a:true,false,false|0:a:true,false,false|1:b:false,true,false:false:true|-1::false,false,false"
    );
}

#[test]
fn child_document_select_exposes_detached_mutation_surface() {
    let mut vm = new_storage_test_vm("https://child-select-options-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  const doc = frame.contentDocument;
  const select = doc.createElement('select');
  const option = doc.createElement('option');
  select.appendChild(option);
  doc.body.appendChild(select);

  const out = [];
  out.push(`${select.length}:${select.options.length}:${select[0] === option}`);

  select.length = 0;
  out.push(`${select.length}:${select.options.length}:${select.firstChild === null}:${doc.querySelector('select') === select}`);

  const indexed = doc.createElement('option');
  select[0] = indexed;
  out.push(`${select.length}:${select.options.length}:${select[0] === indexed}:${select.options[0] === indexed}`);

  select[0] = null;
  out.push(`${select.length}:${select.options.length}:${select.firstChild === null}`);

  const added = doc.createElement('option');
  select.add(added);
  out.push(`${select.length}:${select.options.length}:${select[0] === added}`);

  select.remove(0);
  out.push(`${select.length}:${select.options.length}:${select.firstChild === null}:${doc.querySelector('select') === select}`);

  const collectionAdded = doc.createElement('option');
  select.options.add(collectionAdded);
  out.push(`${select.length}:${select.options.length}:${select.options[0] === collectionAdded}`);

  select.options.length = 0;
  out.push(`${select.length}:${select.options.length}:${select.firstChild === null}`);

  return out.join('|');
})()
"#,
        )
        .expect("child detached select mutation surface should evaluate");

    assert_eq!(
        result,
        "1:1:true|0:0:true:true|1:1:true:true|0:0:true|1:1:true|0:0:true:true|1:1:true|0:0:true"
    );
}

#[test]
fn option_click_default_action_updates_select_selectedness_and_events() {
    let mut vm = new_storage_test_vm("https://option-click-default-action.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const option = (id, value, selected = false) => {
    const node = document.createElement('option');
    node.id = id;
    node.value = value;
    node.textContent = value.toUpperCase();
    node.defaultSelected = selected;
    node.selected = selected;
    return node;
  };
  const single = document.createElement('select');
  single.id = 'single';
  const singleA = option('singleA', 'a');
  const singleB = option('singleB', 'b');
  single.append(singleA, singleB);
  const multi = document.createElement('select');
  multi.id = 'multi';
  multi.multiple = true;
  const multiA = option('multiA', 'a', true);
  const multiB = option('multiB', 'b');
  multi.append(multiA, multiB);
  const log = [];
  const selectedValues = select => Array.from(select.selectedOptions).map(option => option.value).join('/');
  for (const [name, select] of [['single', single], ['multi', multi]]) {
    select.addEventListener('input', () => log.push(`${name}:input:${selectedValues(select)}`));
    select.addEventListener('change', () => log.push(`${name}:change:${selectedValues(select)}`));
  }
  singleB.click();
  multiB.click();
  multiA.click();
  singleB.click();
  return [
    single.selectedIndex,
    single.value,
    [singleA.selected, singleB.selected].join(','),
    selectedValues(multi),
    [multiA.selected, multiB.selected].join(','),
    log.join(',')
  ].join('|');
})()
"#,
        )
        .expect("option click default action should update select selectedness");

    assert_eq!(
        result,
        "1|b|false,true|b|false,true|single:input:b,single:change:b,multi:input:a/b,multi:change:a/b,multi:input:b,multi:change:b"
    );
}

#[test]
fn option_click_default_action_respects_disabled_optgroup_and_select() {
    let mut vm = new_storage_test_vm("https://option-click-disabled-optgroup.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const single = document.createElement('select');
  single.id = 'single';
  single.innerHTML = [
    '<option id="singleA" value="a" selected>A</option>',
    '<optgroup disabled><option id="singleB" value="b">B</option></optgroup>',
    '<option id="singleC" value="c">C</option>'
  ].join('');
  const disabledSelect = document.createElement('select');
  disabledSelect.id = 'disabledSelect';
  disabledSelect.disabled = true;
  disabledSelect.innerHTML = [
    '<option id="disabledA" value="d" selected>D</option>',
    '<option id="disabledB" value="e">E</option>'
  ].join('');
  body.append(single, disabledSelect);

  const log = [];
  for (const select of [single, disabledSelect]) {
    select.addEventListener('input', () => log.push(`${select.id}:input:${select.value}`));
    select.addEventListener('change', () => log.push(`${select.id}:change:${select.value}`));
  }

  document.getElementById('singleB').click();
  document.getElementById('singleC').click();
  document.getElementById('disabledB').click();

  return [
    single.value,
    Array.from(single.options).map(option => option.selected).join(','),
    disabledSelect.value,
    Array.from(disabledSelect.options).map(option => option.selected).join(','),
    log.join(',')
  ].join('|');
})()
"#,
        )
        .expect("disabled option click default action should evaluate");

    assert_eq!(
        result,
        "c|false,false,true|d|true,false|single:input:c,single:change:c"
    );
}

#[test]
fn detached_form_elements_collection_tracks_controls_and_owner_form() {
    let mut vm = new_storage_test_vm("https://detached-form-elements.test/path/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body>' +
      '<form id="f">' +
        '<input id="email" name="email" value="a">' +
        '<button id="go" name="go"></button>' +
        '<select id="choice" name="choice"><option value="x" selected>X</option></select>' +
        '<div><textarea id="bio" name="bio"></textarea></div>' +
      '</form>' +
      '<input id="loose" name="loose">' +
    '</body></html>',
    'text/html'
  );
  const form = doc.getElementById('f');
  const email = doc.getElementById('email');
  const go = doc.getElementById('go');
  const choice = doc.getElementById('choice');
  const bio = doc.getElementById('bio');
  const loose = doc.getElementById('loose');
  const out = [];
  out.push([
    form.length,
    form.elements.length,
    form.elements[0] === email,
    form.elements.item(1) === go,
    form.elements.namedItem('choice') === choice,
    form.elements.namedItem('bio') === bio,
    email.form === form,
    bio.form === form,
    loose.form === null
  ].join(':'));

  const dynamic = doc.createElement('input');
  dynamic.name = 'dynamic';
  form.appendChild(dynamic);
  out.push([
    form.length,
    form.elements.length,
    form.elements.namedItem('dynamic') === dynamic,
    dynamic.form === form
  ].join(':'));

  const nested = doc.createElement('form');
  const nestedInput = doc.createElement('input');
  nestedInput.name = 'nested';
  nested.appendChild(nestedInput);
  form.appendChild(nested);
  out.push([
    form.length,
    form.elements.length,
    form.elements.namedItem('nested') === null,
    nested.length,
    nested.elements.namedItem('nested') === nestedInput,
    nestedInput.form === nested
  ].join(':'));

  form.removeChild(email);
  out.push([
    form.length,
    form.elements.length,
    form.elements.namedItem('email') === null,
    email.form === null
  ].join(':'));
  return out.join('|');
})()
"#,
        )
        .expect("detached form elements should track controls and owner form");

    assert_eq!(
        result,
        "4:4:true:true:true:true:true:true:true|5:5:true:true|5:5:true:1:true:true|4:4:true:true"
    );
}
#[test]
fn shadow_form_control_form_attribute_tracks_shadow_tree_connection() {
    let mut vm = new_parsed_test_vm(
        "https://shadow-form-attribute.test/",
        r#"<html><body><div id="testcontent"><form id="form">form</form><input id="input" form="form"></div></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const out = [];
  const testcontent = document.getElementById('testcontent');
  const host = document.createElement('div');
  const sr = host.attachShadow({ mode: 'open' });
  sr.innerHTML = testcontent.innerHTML;
  const input = sr.getElementById('input');
  const form = sr.getElementById('form');

  out.push(input.form === null);
  testcontent.appendChild(host);
  out.push(input.form === form);
  host.remove();
  out.push(input.form === null);
  testcontent.appendChild(host);
  out.push(input.form === form);
  input.remove();
  out.push(input.form === null);
  sr.appendChild(input);
  out.push(input.form === form);
  form.id = 'foobar';
  out.push(input.form === null);
  form.id = 'form';
  out.push(input.form === form);
  form.remove();
  out.push(input.form === null);
  sr.appendChild(form);
  out.push(input.form === form);
  host.remove();

  const ancestorHost = document.createElement('div');
  const ancestorRoot = ancestorHost.attachShadow({ mode: 'open' });
  ancestorRoot.innerHTML = "<form id='form'><input id='input'></form>";
  const ancestorInput = ancestorRoot.getElementById('input');
  const ancestorForm = ancestorRoot.getElementById('form');

  out.push(ancestorInput.form === ancestorForm);
  ancestorInput.remove();
  out.push(ancestorInput.form === null);
  ancestorForm.appendChild(ancestorInput);
  out.push(ancestorInput.form === ancestorForm);
  ancestorForm.remove();
  out.push(ancestorInput.form === ancestorForm);

  return out.map((value, index) => value ? 'ok' : `fail${index}`).join('|');
})()
"#,
        )
        .expect("shadow form attribute ownership should evaluate");

    assert_eq!(result, "ok|ok|ok|ok|ok|ok|ok|ok|ok|ok|ok|ok|ok|ok");
}
#[test]
fn html_form_named_properties_include_image_fallback_and_new_builtins() {
    let mut vm = new_parsed_test_vm(
        "https://form-named-properties.test/",
        r#"
<!doctype html>
<form id="f">
  <img id="fallback" name="fallback">
  <img id="sharedImg" name="shared">
  <input id="sharedInput" name="shared">
  <input id="requestSubmitControl" name="requestSubmit">
  <input id="reportValidityControl" name="reportValidity">
</form>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const form = document.getElementById('f');
  return JSON.stringify({
    fallback: form.fallback === document.getElementById('fallback'),
    controlWins: form.shared === document.getElementById('sharedInput'),
    requestSubmit: form.requestSubmit === document.getElementById('requestSubmitControl'),
    reportValidity: form.reportValidity === document.getElementById('reportValidityControl')
  });
})()
"#,
        )
        .expect("HTMLFormElement named property probe should evaluate");

    assert_eq!(
        result,
        r#"{"fallback":true,"controlWins":true,"requestSubmit":true,"reportValidity":true}"#
    );
}

#[test]
fn html_form_indexed_properties_shadow_prototype_and_reject_delete() {
    let mut vm = new_parsed_test_vm(
        "https://form-indexed-properties.test/",
        r#"
<!doctype html>
<form id="form">
  <input type="radio" name="radio1" id="r1" value="1">
  <input type="radio" name="radio2" id="r2" value="2">
</form>
"#,
    );

    vm.exec(
        r#"
(() => {
  const form = document.getElementById("form");
  const oldItem = form[0];
  const oldDesc = Object.getOwnPropertyDescriptor(form, 0);
  Object.prototype[0] = 5;
  try {
    const deleteResult = delete form[0];
    let strictDelete = "missing";
    try {
      (function () {
        "use strict";
        delete form[0];
      })();
      strictDelete = "no-throw";
    } catch (error) {
      strictDelete = error.name;
    }
    globalThis.__moliFormIndexedDeleteProbe = JSON.stringify({
      initial: oldItem && oldItem.id,
      descriptorValue: oldDesc && oldDesc.value && oldDesc.value.id,
      enumerable: oldDesc && oldDesc.enumerable,
      configurable: oldDesc && oldDesc.configurable,
      writable: oldDesc && oldDesc.writable,
      afterProto: form[0] === oldItem,
      deleteResult,
      afterDelete: form[0] === oldItem,
      strictDelete,
      afterStrictDelete: form[0] === oldItem
    });
  } finally {
    delete Object.prototype[0];
  }
})()
"#,
        None,
    )
    .expect("HTMLFormElement indexed property deletion probe should execute");

    let result = vm
        .eval("globalThis.__moliFormIndexedDeleteProbe")
        .expect("HTMLFormElement indexed property deletion probe result should evaluate");

    assert_eq!(
        result,
        r#"{"initial":"r1","descriptorValue":"r1","enumerable":true,"configurable":true,"writable":false,"afterProto":true,"deleteResult":false,"afterDelete":true,"strictDelete":"TypeError","afterStrictDelete":true}"#
    );
}

#[test]
fn html_form_indexed_properties_resist_set_define_and_delete_with_proto_index() {
    let mut vm = new_parsed_test_vm(
        "https://form-indexed-delete.test/",
        r#"
<!doctype html>
<form id="form">
  <input id="r1" name="radio1" value="1">
  <input id="r2" name="radio2" value="2">
</form>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const form = document.getElementById('form');
  const first = document.getElementById('r1');
  const descriptor = Object.getOwnPropertyDescriptor(form, 0);
  Object.prototype[0] = 5;
  try {
    const objectKeysIncludeZero = Object.keys(form).includes('0');
    const beforeDelete = form[0] === first;
    form[0] = 5;
    const afterSloppySet = form[0] === first;
    let strictSet = 'missing-throw';
    try {
      Function('"use strict"; arguments[0][0] = 5;')(form);
    } catch (error) {
      strictSet = error && error.name;
    }
    const reflectDefine = Reflect.defineProperty(form, 0, { value: 5 });
    const afterReflectDefine = form[0] === first;
    let defineProperty = 'missing-throw';
    try {
      Object.defineProperty(form, 0, { value: 5 });
    } catch (error) {
      defineProperty = error && error.name;
    }
    const deleteResult = delete form[0];
    const afterDelete = form[0] === first;
    let strictDelete = 'missing-throw';
    try {
      Function('"use strict"; arguments[0] && delete arguments[0][0];')(form);
    } catch (error) {
      strictDelete = error && error.name;
    }
    return JSON.stringify({
      descriptorValue: descriptor.value === first,
      enumerable: descriptor.enumerable,
      configurable: descriptor.configurable,
      writable: descriptor.writable,
      objectKeysIncludeZero,
      beforeDelete,
      afterSloppySet,
      strictSet,
      reflectDefine,
      afterReflectDefine,
      defineProperty,
      deleteResult,
      afterDelete,
      strictDelete
    });
  } finally {
    delete Object.prototype[0];
  }
})()
"#,
        )
        .expect("HTMLFormElement indexed delete probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptorValue":true,"enumerable":true,"configurable":true,"writable":false,"objectKeysIncludeZero":true,"beforeDelete":true,"afterSloppySet":true,"strictSet":"TypeError","reflectDefine":false,"afterReflectDefine":true,"defineProperty":"TypeError","deleteResult":false,"afterDelete":true,"strictDelete":"TypeError"}"#
    );
}

#[test]
fn html_form_indexed_properties_match_wpt_sequence() {
    let mut vm = new_parsed_test_vm(
        "https://form-indexed-properties-wpt-sequence.test/",
        r#"
<!doctype html>
<div id="test">
<form id=form>
<input type="radio" name="radio1" id="r1" value=1>
<input type="radio" name="radio2" id="r2" value=2>
</form>
</div>
"#,
    );

    vm.exec(
        r#"
(() => {
  const out = [];
  function check(condition, label) {
    out.push(condition ? label + ":ok" : label + ":fail");
  }
  var form = document.getElementById("form");
  check(form[0] === document.getElementById("r1"), "first-index-0");
  check(form[1] === document.getElementById("r2"), "first-index-1");
  check(form[2] === undefined, "first-index-2");
  check(form[-1] === undefined, "first-index-negative");

  form = document.getElementById("form");
  var old_item = form[0];
  var old_desc = Object.getOwnPropertyDescriptor(form, 0);
  check(old_desc.value === old_item, "desc-value");
  check(old_desc.enumerable, "desc-enumerable");
  check(old_desc.configurable, "desc-configurable");
  check(!old_desc.writable, "desc-writable");

  Object.prototype[0] = 5;
  const harnessLikeThis = {
    cleanup_callbacks: [],
    _user_defined_cleanup_count: 0,
    _add_cleanup(callback) {
      this.cleanup_callbacks.push(callback);
    },
    add_cleanup(callback) {
      this._user_defined_cleanup_count += 1;
      this._add_cleanup(callback);
    }
  };
  try {
    harnessLikeThis.add_cleanup(function () { delete Object.prototype[0]; });
    check(form[0] === old_item, "after-prototype");
    delete form[0];
    check(form[0] === old_item, "after-delete");
    let strictDelete = "no-throw";
    try {
      (function() { "use strict"; delete form[0]; })();
    } catch (error) {
      strictDelete = error.name;
    }
    check(strictDelete === "TypeError", "strict-delete");
    check(form[0] === old_item, "after-strict-delete");
  } finally {
    delete Object.prototype[0];
  }
  globalThis.__moliFormIndexedWptSequence = out.join("|");
})()
"#,
        None,
    )
    .expect("HTMLFormElement WPT indexed sequence should execute");

    let result = vm
        .eval("globalThis.__moliFormIndexedWptSequence")
        .expect("HTMLFormElement WPT indexed sequence result should evaluate");

    assert_eq!(
        result,
        "first-index-0:ok|first-index-1:ok|first-index-2:ok|first-index-negative:ok|desc-value:ok|desc-enumerable:ok|desc-configurable:ok|desc-writable:ok|after-prototype:ok|after-delete:ok|strict-delete:ok|after-strict-delete:ok"
    );
}

#[test]
fn detached_form_submit_and_reset_methods_use_local_events() {
    let mut vm = new_storage_test_vm("https://detached-form-methods.test/path/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString(
    '<html><body>' +
      '<form id="form">' +
        '<input id="text" type="text" name="q" value="test">' +
        '<input id="submitButton" type="submit" value="Go">' +
        '<input id="image" type="image" src="x.png">' +
        '<button id="buttonSubmit">Submit</button>' +
        '<button id="buttonReset" type="reset">Reset</button>' +
        '<button id="buttonButton" type="button">Button</button>' +
        '<input id="emptyFormSubmit" form="" type="submit" value="Empty">' +
        '<select id="choice">' +
          '<option id="a" value="a">A</option>' +
          '<option id="b" value="b" selected>B</option>' +
        '</select>' +
      '</form>' +
      '<form id="other"><input id="otherSubmit" type="submit" value="Other"></form>' +
    '</body></html>',
    'text/html'
  );
  const form = doc.getElementById('form');
  const choice = doc.getElementById('choice');
  const out = [];
  let submitCount = 0;
  let submitTargetMatches = true;
  form.addEventListener('submit', (event) => {
    submitCount++;
    submitTargetMatches &&= event.target === form && event.currentTarget === form && event.cancelable;
    event.preventDefault();
  });

  out.push(typeof form.submit + ':' + typeof form.requestSubmit + ':' + typeof form.reset);
  out.push([
    typeof HTMLFormElement.prototype.submit,
    typeof HTMLFormElement.prototype.requestSubmit,
    typeof HTMLFormElement.prototype.reset,
    typeof HTMLFormElement.prototype.checkValidity,
    typeof HTMLFormElement.prototype.reportValidity
  ].join(':'));
  form.submit();
  HTMLFormElement.prototype.submit.call(form);
  out.push(submitCount);
  form.requestSubmit();
  form.requestSubmit(doc.getElementById('submitButton'));
  form.requestSubmit(doc.getElementById('image'));
  form.requestSubmit(doc.getElementById('buttonSubmit'));
  out.push(submitCount + ':' + submitTargetMatches);

  for (const id of ['text', 'buttonReset', 'buttonButton']) {
    try {
      form.requestSubmit(doc.getElementById(id));
      out.push(id + ':missing');
    } catch (error) {
      out.push(id + ':' + error.name);
    }
  }
  try {
    form.requestSubmit(doc.getElementById('otherSubmit'));
    out.push('other:missing');
  } catch (error) {
    out.push('other:' + error.name);
  }
  const emptyFormSubmit = doc.getElementById('emptyFormSubmit');
  out.push('empty-form-owner:' + (emptyFormSubmit.form === null));
  try {
    form.requestSubmit(emptyFormSubmit);
    out.push('empty-form:missing');
  } catch (error) {
    out.push('empty-form:' + error.name);
  }

  let resetCount = 0;
  form.addEventListener('reset', (event) => {
    resetCount++;
    out.push('reset-event:' + (event.target === form) + ':' + event.cancelable);
  });
  choice.value = 'a';
  form.reset();
  out.push('reset:' + resetCount + ':' + choice.value + ':' + choice.selectedIndex);

  choice.value = 'a';
  form.addEventListener('reset', (event) => event.preventDefault(), { once: true });
  form.reset();
  out.push('prevented-reset:' + choice.value + ':' + choice.selectedIndex);

  return out.join('|');
})()
"#,
        )
        .expect("detached form methods should use local events");

    assert_eq!(
        result,
        "function:function:function|function:function:function:function:function|0|4:true|text:TypeError|buttonReset:TypeError|buttonButton:TypeError|other:NotFoundError|empty-form-owner:true|empty-form:NotFoundError|reset-event:true:true|reset:1:b:1|reset-event:true:true|prevented-reset:a:0"
    );
}

#[test]
fn form_reset_clears_option_selectedness_dirty_flag() {
    let mut vm = new_storage_test_vm("https://form-reset-option-dirty.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const form = document.createElement('form');
  const select = document.createElement('select');
  select.multiple = true;
  const option = document.createElement('option');
  form.append(select);
  select.append(option);

  option.defaultSelected = true;
  option.selected = false;
  form.reset();
  const afterReset = `${option.selected}:${select.selectedIndex}`;

  option.defaultSelected = false;
  const afterDefaultChange = `${option.selected}:${select.selectedIndex}`;
  return `${afterReset}|${afterDefaultChange}`;
})()
"#,
        )
        .expect("form reset option dirty-state probe should evaluate");

    assert_eq!(result, "true:0|false:-1");
}

#[test]
fn form_reset_event_uses_form_wrapper_realm_after_cross_frame_move() {
    let mut vm = new_storage_test_vm("https://form-reset-event-realm.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sourceFrame = document.createElement("iframe");
  const targetFrame = document.createElement("iframe");
  const parent = document.body || document.documentElement || document;
  parent.appendChild(sourceFrame);
  parent.appendChild(targetFrame);
  const source = sourceFrame.contentWindow;
  const target = targetFrame.contentWindow;
  const form = source.document.createElement("form");
  const input = source.document.createElement("input");
  input.name = "q";
  input.value = "old";
  form.appendChild(input);
  source.document.body.appendChild(form);
  target.document.body.appendChild(form);
  let seen = null;
  form.onreset = event => {
    seen = {
      sameConstructorRealm: event.constructor.constructor === form.constructor.constructor,
      differentFromCallerRealm: event.constructor.constructor !== Function,
      targetMatches: event.target === form,
      currentTargetMatches: event.currentTarget === form,
      cancelable: event.cancelable,
      bubbles: event.bubbles
    };
  };
  form.reset();
  return JSON.stringify(seen);
})()
"#,
        )
        .expect("cross-frame moved form reset event realm should evaluate");

    assert_eq!(
        result,
        r#"{"sameConstructorRealm":true,"differentFromCallerRealm":true,"targetMatches":true,"currentTargetMatches":true,"cancelable":true,"bubbles":true}"#
    );
}

#[test]
fn option_constructor_default_selected_only_keeps_selectedness_false() {
    let mut vm = new_storage_test_vm("https://option-constructor-selected.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const option = new Option('Label', 'value', true);
  return [
    option.defaultSelected,
    option.selected,
    option.getAttribute('selected') !== null
  ].join(':');
})()
"#,
        )
        .expect("Option constructor selectedness probe should evaluate");

    assert_eq!(result, "true:false:true");
}
#[test]
fn legacy_element_constructors_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://legacy-element-constructors.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const img = new Image(4, 5);
  const audio = new Audio('clip.mp3');
  const selected = new Option('Label', 'value', true, true);
  return [
    img.width,
    img.height,
    audio.getAttribute('preload'),
    audio.getAttribute('src'),
    selected.defaultSelected,
    selected.selected,
    selected.getAttribute('selected') !== null
  ].join('|');
})()
"#,
        )
        .expect("legacy element constructors should parse WebIDL arguments");

    assert_eq!(result, "4|5|auto|clip.mp3|true|true|true");
}
#[test]
fn select_add_before_optgroup_inserts_as_select_child() {
    let mut vm = new_storage_test_vm("https://select-add-optgroup.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const select = document.createElement('select');
  const group = document.createElement('optgroup');
  group.label = 'group';
  const grouped = new Option('grouped', 'grouped');
  group.appendChild(grouped);
  select.appendChild(group);
  const inserted = new Option('inserted', 'inserted');
  select.add(inserted, group);
  return [
    inserted.parentNode === select,
    select.firstChild === inserted,
    inserted.nextSibling === group,
    group.firstChild === grouped
  ].join(':');
})()
"#,
        )
        .expect("select.add optgroup insertion probe should evaluate");

    assert_eq!(result, "true:true:true:true");
}
#[test]
fn detached_duplicate_signal_listener_does_not_leave_extra_abort_cleanup() {
    let mut vm = new_storage_test_vm("https://detached-duplicate-signal.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<div id="target"></div>', 'text/html');
  const target = doc.getElementById('target');
  const controller = new AbortController();
  let calls = 0;
  const listener = () => { calls++; };
  target.addEventListener('x', listener, { signal: controller.signal });
  target.addEventListener('x', listener, { signal: controller.signal });
  target.removeEventListener('x', listener);
  target.addEventListener('x', listener);
  controller.abort();
  target.dispatchEvent(new Event('x'));
  return String(calls);
})()
"#,
        )
        .expect("detached duplicate signal listener probe should evaluate");

    assert_eq!(result, "1");
}
#[test]
fn detached_stale_abort_cleanup_does_not_remove_readded_listener() {
    let mut vm = new_storage_test_vm("https://detached-stale-abort-cleanup.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = new DOMParser().parseFromString('<div id="target"></div>', 'text/html');
  const target = doc.getElementById('target');
  const controller = new AbortController();
  let calls = 0;
  const listener = () => { calls++; };
  target.addEventListener('x', listener, { signal: controller.signal });
  target.removeEventListener('x', listener);
  target.addEventListener('x', listener);
  controller.abort();
  target.dispatchEvent(new Event('x'));
  return String(calls);
})()
"#,
        )
        .expect("detached stale abort cleanup probe should evaluate");

    assert_eq!(result, "1");
}
#[test]
fn media_query_list_signal_abort_removes_registered_listener() {
    let mut vm = new_storage_test_vm("https://mql-signal-abort-cleanup.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = matchMedia('(min-width: 0px)');
  const controller = new AbortController();
  let calls = 0;
  const listener = () => { calls++; };
  target.addEventListener('change', listener, { signal: controller.signal });
  controller.abort();
  target.dispatchEvent(new Event('change'));
  return String(calls);
})()
"#,
        )
        .expect("MediaQueryList signal abort cleanup probe should evaluate");

    assert_eq!(result, "0");
}
#[test]
fn simple_event_target_stale_abort_cleanup_does_not_remove_readded_listener() {
    let mut vm = new_storage_test_vm("https://simple-stale-abort-cleanup.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = matchMedia('(min-width: 0px)');
  const controller = new AbortController();
  let calls = 0;
  const listener = () => { calls++; };
  target.addEventListener('change', listener, { signal: controller.signal });
  target.removeEventListener('change', listener);
  target.addEventListener('change', listener);
  controller.abort();
  target.dispatchEvent(new Event('change'));
  return String(calls);
})()
"#,
        )
        .expect("simple stale abort cleanup probe should evaluate");

    assert_eq!(result, "1");
}
#[test]
fn abort_signal_listener_methods_apply_webidl_boundaries() {
    let mut vm = new_storage_test_vm("https://abort-signal-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const controller = new AbortController();
  const signal = controller.signal;
  const out = [];
  function listener() { out.push('listener'); }
  signal.addEventListener({ toString() { out.push('type'); return 'abort'; } }, listener);
  signal.dispatchEvent(new Event('abort'));
  signal.removeEventListener({ toString() { out.push('remove'); return 'abort'; } }, listener);
  signal.dispatchEvent(new Event('abort'));

  const checks = [];
  function record(label, callback) {
    try {
      callback();
      checks.push(`${label}:no`);
    } catch (error) {
      checks.push(`${label}:${error.name}`);
    }
  }
  record('add-missing', () => signal.addEventListener());
  record('add-symbol', () => signal.addEventListener(Symbol(), listener));
  record('add-listener-missing', () => signal.addEventListener('abort'));
  record('remove-missing', () => signal.removeEventListener());
  record('dispatch-missing', () => signal.dispatchEvent());
  signal.addEventListener('abort', null);
  return `${out.join(',')}|${checks.join(',')}`;
})()
"#,
        )
        .expect("AbortSignal WebIDL boundary probe should evaluate");

    assert_eq!(
        result,
        "type,listener,remove|add-missing:TypeError,add-symbol:TypeError,add-listener-missing:TypeError,remove-missing:TypeError,dispatch-missing:TypeError"
    );
}

#[test]
fn abort_signal_throw_if_aborted_throws_the_exact_reason_value() {
    let mut vm = new_storage_test_vm("https://abort-signal-throw-reason.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const reasons = ["reason string", 42, null, { marker: "object reason" }];
  const results = reasons.map(reason => {
    const controller = new AbortController();
    controller.abort(reason);
    try {
      controller.signal.throwIfAborted();
      return "not-thrown";
    } catch (thrown) {
      return `${thrown === reason}:${typeof thrown}:${String(thrown)}`;
    }
  });
  const live = new AbortController();
  results.push(`live:${String(live.signal.throwIfAborted())}`);
  return results.join("|");
})()
"#,
        )
        .expect("AbortSignal exact throw reason probe should evaluate");

    assert_eq!(
        result,
        "true:string:reason string|true:number:42|true:object:null|true:object:[object Object]|live:undefined"
    );
}

#[test]
fn abort_signal_listeners_use_event_listener_callback_interface_semantics() {
    let mut vm = new_storage_test_vm("https://abort-signal-callback-interface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const controller = new AbortController();
  const signal = controller.signal;

  let callableCalls = 0;
  let callableThis = null;
  let callableWindowEvent = false;
  let forbiddenHandleEventGets = 0;
  function callable(event) {
    "use strict";
    callableCalls++;
    callableThis = this;
    callableWindowEvent = window.event === event;
  }
  Object.defineProperty(callable, "handleEvent", {
    get() {
      forbiddenHandleEventGets++;
      throw new Error("callable listeners must not resolve handleEvent");
    }
  });
  signal.addEventListener("abort", callable);
  signal.addEventListener("abort", callable, { once: true });
  controller.abort();
  signal.dispatchEvent(new Event("abort"));

  let operationGets = 0;
  let objectCalls = 0;
  let objectReceiver = false;
  let windowEventDuringOperationLookup = null;
  let operation = function(event) {
    objectCalls++;
    objectReceiver = this === objectListener;
  };
  const objectListener = {
    get handleEvent() {
      operationGets++;
      windowEventDuringOperationLookup = window.event && window.event.type;
      return operation;
    }
  };
  signal.addEventListener("object", objectListener);
  signal.dispatchEvent(new Event("object"));
  operation = function(event) {
    objectCalls++;
    objectReceiver &&= this === objectListener;
  };
  signal.dispatchEvent(new Event("object"));
  signal.removeEventListener("object", objectListener);
  signal.dispatchEvent(new Event("object"));

  let captureCalls = 0;
  const captureListener = () => captureCalls++;
  signal.addEventListener("capture", captureListener, false);
  signal.addEventListener("capture", captureListener, true);
  signal.removeEventListener("capture", captureListener, false);
  signal.dispatchEvent(new Event("capture"));

  let nestedOnceCalls = 0;
  signal.addEventListener("nested-once", () => {
    nestedOnceCalls++;
    signal.dispatchEvent(new Event("nested-once"));
  }, { once: true });
  signal.dispatchEvent(new Event("nested-once"));

  let selfRemoveCalls = 0;
  function selfRemovingListener() {
    selfRemoveCalls++;
    signal.removeEventListener("self-remove", selfRemovingListener);
  }
  signal.addEventListener("self-remove", selfRemovingListener);
  signal.dispatchEvent(new Event("self-remove"));
  signal.dispatchEvent(new Event("self-remove"));

  const mutationCalls = [];
  const removedBeforeTurn = () => mutationCalls.push("removed");
  signal.addEventListener("mutation", () => {
    mutationCalls.push("first");
    signal.removeEventListener("mutation", removedBeforeTurn);
  });
  signal.addEventListener("mutation", removedBeforeTurn);
  signal.dispatchEvent(new Event("mutation"));

  let passiveDefaultPrevented = null;
  signal.addEventListener("passive", event => {
    event.preventDefault();
    passiveDefaultPrevented = event.defaultPrevented;
  }, { passive: true });
  const passiveResult = signal.dispatchEvent(
    new Event("passive", { cancelable: true })
  );

  const reported = [];
  const onError = event => {
    reported.push(event.error && event.error.name);
    event.preventDefault();
  };
  window.addEventListener("error", onError);
  const throwingObject = {
    get handleEvent() {
      throw new RangeError("operation");
    }
  };
  signal.addEventListener("throwing", throwingObject);
  const throwingResult = signal.dispatchEvent(new Event("throwing"));

  const revocable = Proxy.revocable(() => {}, {});
  signal.addEventListener("proxy", revocable.proxy);
  revocable.revoke();
  const revokedResult = signal.dispatchEvent(new Event("proxy"));
  window.removeEventListener("error", onError);

  const stopped = new AbortController();
  let stoppedCalls = 0;
  stopped.signal.addEventListener("abort", event => {
    stoppedCalls++;
    event.stopImmediatePropagation();
  });
  stopped.signal.addEventListener("abort", () => stoppedCalls++);
  stopped.signal.onabort = () => stoppedCalls++;
  stopped.abort();

  return JSON.stringify({
    callableCalls,
    callableThisIsSignal: callableThis === signal,
    callableWindowEvent,
    forbiddenHandleEventGets,
    operationGets,
    objectCalls,
    objectReceiver,
    windowEventDuringOperationLookup,
    captureCalls,
    nestedOnceCalls,
    selfRemoveCalls,
    mutationCalls,
    passiveDefaultPrevented,
    passiveResult,
    throwingResult,
    revokedResult,
    reported,
    stoppedCalls
  });
})()
"#,
        )
        .expect("AbortSignal callback-interface semantics should evaluate");

    assert_eq!(
        result,
        r#"{"callableCalls":2,"callableThisIsSignal":true,"callableWindowEvent":true,"forbiddenHandleEventGets":0,"operationGets":2,"objectCalls":2,"objectReceiver":true,"windowEventDuringOperationLookup":"object","captureCalls":1,"nestedOnceCalls":1,"selfRemoveCalls":1,"mutationCalls":["first"],"passiveDefaultPrevented":false,"passiveResult":true,"throwingResult":true,"revokedResult":true,"reported":["RangeError","TypeError"],"stoppedCalls":1}"#
    );
}

#[test]
fn abort_signal_listener_uses_callback_relevant_realm_and_exact_window_lifetime() {
    let mut vm = new_parsed_test_vm(
        "https://abort-signal-callback-realm.test/",
        "<!doctype html><html><body></body></html>",
    );

    vm.eval(
        r#"
(() => {
  const iframe = document.createElement("iframe");
  iframe.srcdoc = "<!doctype html><html><body></body></html>";
  document.body.appendChild(iframe);
  globalThis.__abortListenerRealmFrame = iframe;
  return "ready";
})()
"#,
    )
    .expect("cross-Realm AbortSignal listener setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const iframe = globalThis.__abortListenerRealmFrame;
  const other = iframe.contentWindow;
  const controller = new AbortController();
  const signal = controller.signal;

  globalThis.__abortListenerCallFacts = [];
  const crossRealmCallable = other.Function(
    "event",
    `"use strict";
     parent.__abortListenerCallFacts = [
       this === parent.__abortListenerSignal,
       globalThis === parent.__abortListenerExpectedRealm,
       window.event === event,
       event.currentTarget === parent.__abortListenerSignal
     ];`
  );
  globalThis.__abortListenerSignal = signal;
  globalThis.__abortListenerExpectedRealm = other;
  signal.addEventListener("cross-realm", crossRealmCallable);
  signal.dispatchEvent(new Event("cross-realm"));

  let reported = null;
  const missingOperation = new other.Object();
  const onError = event => {
    reported = {
      relevantTypeError:
        event.error instanceof other.TypeError &&
        !(event.error instanceof TypeError),
      targetIsCallbackWindow: event.currentTarget === other
    };
    event.preventDefault();
  };
  other.addEventListener("error", onError);
  signal.addEventListener("missing", missingOperation);
  const missingResult = signal.dispatchEvent(new Event("missing"));
  other.removeEventListener("error", onError);

  globalThis.__retiredAbortListenerCalls = 0;
  const retiredCallback = other.Function(
    "parent.__retiredAbortListenerCalls++;"
  );
  signal.addEventListener("retired", retiredCallback);
  iframe.remove();
  const retiredResult = signal.dispatchEvent(new Event("retired"));

  return JSON.stringify({
    callFacts: globalThis.__abortListenerCallFacts,
    missingResult,
    reported,
    childDetached: iframe.contentWindow === null,
    retiredResult,
    retiredCalls: globalThis.__retiredAbortListenerCalls
  });
})()
"#,
        )
        .expect("cross-Realm AbortSignal listener invocation should evaluate");

    assert_eq!(
        result,
        r#"{"callFacts":[true,true,true,true],"missingResult":true,"reported":{"relevantTypeError":true,"targetIsCallbackWindow":true},"childDetached":true,"retiredResult":true,"retiredCalls":0}"#
    );
}

#[test]
fn abort_signal_internal_id_is_not_page_visible_or_forgeable() {
    let mut vm = new_storage_test_vm("https://abort-signal-private-brand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const methodDescriptor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      name,
      typeof descriptor?.value,
      descriptor?.value?.name,
      descriptor?.value?.length,
      descriptor?.enumerable,
      descriptor?.writable,
      descriptor?.configurable
    ].join(":");
  };
  const accessorDescriptor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      name,
      typeof descriptor?.get,
      descriptor?.get?.name,
      descriptor?.get?.length,
      descriptor?.enumerable,
      typeof descriptor?.set,
      descriptor?.configurable
    ].join(":");
  };
  const controller = new AbortController();
  const signal = controller.signal;
  const staticAbortSignal = AbortSignal.abort("static-reason");
  const forged = { __lmAbortSignalId: signal.__lmAbortSignalId ?? 1 };
  const abortedGetter = Object.getOwnPropertyDescriptor(AbortSignal.prototype, "aborted").get;
  const probe = callback => {
    try {
      callback();
      return "ok";
    } catch (error) {
      return error && error.name;
    }
  };
  return JSON.stringify({
    hasVisibleSlot: "__lmAbortSignalId" in signal,
    ownNames: Object.getOwnPropertyNames(signal),
    signalMethods: [
      methodDescriptor(AbortSignal.prototype, "addEventListener"),
      methodDescriptor(AbortSignal.prototype, "removeEventListener"),
      methodDescriptor(AbortSignal.prototype, "dispatchEvent"),
      methodDescriptor(AbortSignal.prototype, "throwIfAborted")
    ],
    signalAccessors: [
      accessorDescriptor(AbortSignal.prototype, "aborted"),
      accessorDescriptor(AbortSignal.prototype, "reason"),
      accessorDescriptor(AbortSignal.prototype, "onabort")
    ],
    controllerMethods: [
      methodDescriptor(AbortController.prototype, "abort")
    ],
    controllerAccessors: [
      accessorDescriptor(AbortController.prototype, "signal")
    ],
    constructorMethods: [
      methodDescriptor(AbortSignal, "abort"),
      methodDescriptor(AbortSignal, "timeout"),
      methodDescriptor(AbortSignal, "any")
    ],
    staticAbortAborted: staticAbortSignal.aborted,
    staticAbortReason: staticAbortSignal.reason,
    anyForged: probe(() => AbortSignal.any([forged])),
    getterForged: abortedGetter.call(forged)
  });
})()
"#,
        )
        .expect("AbortSignal private brand probe should evaluate");

    assert_eq!(
        result,
        r#"{"hasVisibleSlot":false,"ownNames":[],"signalMethods":["addEventListener:function:addEventListener:2:true:true:true","removeEventListener:function:removeEventListener:2:true:true:true","dispatchEvent:function:dispatchEvent:1:true:true:true","throwIfAborted:function:throwIfAborted:0:true:true:true"],"signalAccessors":["aborted:function:get aborted:0:true:undefined:true","reason:function:get reason:0:true:undefined:true","onabort:function:get onabort:0:true:function:true"],"controllerMethods":["abort:function:abort:0:true:true:true"],"controllerAccessors":["signal:function:get signal:0:true:undefined:true"],"constructorMethods":["abort:function:abort:1:true:true:true","timeout:function:timeout:1:true:true:true","any:function:any:1:true:true:true"],"staticAbortAborted":true,"staticAbortReason":"static-reason","anyForged":"TypeError","getterForged":false}"#
    );
}

#[test]
fn detached_form_post_submission_uses_request_body_not_query() {
    let mut vm = new_parsed_test_vm(
        "https://detached-form-post.test/page.html",
        r#"<iframe id="frame"></iframe>"#,
    );

    vm.eval(
        r#"
(() => {
  const frame = document.getElementById('frame');
  frame.srcdoc = '<body></body>';
  const doc = frame.contentDocument;
  const form = doc.createElement('form');
  form.setAttribute('method', 'post');
  form.setAttribute('action', 'https://detached-form-post.test/submit?existing=1');
  const input = doc.createElement('input');
  input.name = 'a b';
  input.value = 'c+d';
  form.appendChild(input);
  doc.body.appendChild(form);
  form.submit();
})()
"#,
    )
    .expect("detached POST form submit should evaluate");

    let handle = vm
        ._context_host
        .borrow()
        .child_browsing_context_handle_by_index(0)
        .expect("test iframe should have a child browsing context");
    let pending = vm
        ._context_host
        .borrow()
        .child_browsing_context_pending_live_navigation_for_test(handle)
        .expect("POST form submit should queue a pending child navigation");
    let crate::native_bridge::ChildBrowsingContextBootstrap::Request(request) = pending else {
        panic!("POST form submit should use request navigation, got {pending:?}");
    };

    assert_eq!(
        request.url.as_str(),
        "https://detached-form-post.test/submit?existing=1"
    );
    assert_eq!(request.method, "POST");
    assert_eq!(request.body.as_deref(), Some(&b"a+b=c%2Bd"[..]));
    assert_eq!(
        request.request_headers,
        vec![(
            "Content-Type".to_owned(),
            "application/x-www-form-urlencoded".to_owned()
        )]
    );
}

#[test]
fn top_level_form_post_submission_queues_navigation_request() {
    let mut vm = new_storage_test_vm("https://top-level-form-post.test/page.html");

    vm.eval(
        r#"
(() => {
  const form = document.createElement('form');
  form.method = 'post';
  form.action = '/submit?existing=1';
  const input = document.createElement('input');
  input.name = 'a b';
  input.value = 'c+d';
  form.appendChild(input);
  (document.body || document.documentElement || document).appendChild(form);
  form.submit();
})()
"#,
    )
    .expect("top-level POST form submit should evaluate");

    let pending = vm
        .take_pending_location_navigation_with_seed()
        .expect("top-level POST form submit should queue a pending navigation request");
    assert_eq!(
        pending.url.as_str(),
        "https://top-level-form-post.test/submit?existing=1"
    );
    assert_eq!(pending.request_method, "POST");
    assert_eq!(pending.request_body.as_deref(), Some(&b"a+b=c%2Bd"[..]));
    assert_eq!(
        pending.request_headers,
        vec![(
            "Content-Type".to_owned(),
            "application/x-www-form-urlencoded".to_owned()
        )]
    );
}

#[test]
fn prevented_top_level_form_post_does_not_queue_navigation_request() {
    let mut vm = new_storage_test_vm("https://prevented-form-post.test/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const form = document.createElement('form');
  form.method = 'post';
  form.action = '/submit';
  const input = document.createElement('input');
  input.name = 'token';
  input.value = 'secret';
  form.appendChild(input);
  (document.body || document.documentElement || document).appendChild(form);
  navigation.onnavigate = event => {
    event.preventDefault();
  };
  form.submit();
  return 'prevented';
})()
"#,
        )
        .expect("prevented top-level POST form submit should evaluate");

    assert_eq!(result, "prevented");
    assert!(
        vm.take_pending_location_navigation_with_seed().is_none(),
        "a canceled navigate event must not leak the POST body into a pending navigation"
    );
}

#[test]
fn detached_form_submission_uses_document_encoding_for_urlencoded_body() {
    let mut vm = new_parsed_test_vm(
        "https://detached-form-gbk-post.test/page.html",
        r#"<iframe id="frame"></iframe>"#,
    );
    vm.document_runtime.set_document_character_set("GBK");

    vm.eval(
        r#"
(() => {
  const frame = document.getElementById('frame');
  frame.srcdoc = '<body></body>';
  const doc = frame.contentDocument;
  const form = doc.createElement('form');
  form.setAttribute('method', 'post');
  form.setAttribute('action', 'https://detached-form-gbk-post.test/submit');

  const charset = doc.createElement('input');
  charset.name = '_charset_';
  form.appendChild(charset);

  const upperCharset = doc.createElement('input');
  upperCharset.name = '_CHARSET_';
  upperCharset.value = 'literal';
  form.appendChild(upperCharset);

  const input = doc.createElement('input');
  input.name = 'q';
  input.value = '家居';
  form.appendChild(input);

  doc.body.appendChild(form);
  form.submit();
})()
"#,
    )
    .expect("detached GBK POST form submit should evaluate");

    let handle = vm
        ._context_host
        .borrow()
        .child_browsing_context_handle_by_index(0)
        .expect("test iframe should have a child browsing context");
    let pending = vm
        ._context_host
        .borrow()
        .child_browsing_context_pending_live_navigation_for_test(handle)
        .expect("POST form submit should queue a pending child navigation");
    let crate::native_bridge::ChildBrowsingContextBootstrap::Request(request) = pending else {
        panic!("POST form submit should use request navigation, got {pending:?}");
    };

    assert_eq!(
        request.body.as_deref(),
        Some(&b"_charset_=GBK&_CHARSET_=GBK&q=%BC%D2%BE%D3"[..])
    );
}
#[test]
fn form_submission_rewrites_charset_control_from_accept_charset() {
    let mut vm = new_storage_test_vm("https://form-charset-submit.test/page.html");

    vm.eval(
        r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const host = document.body;
  const frame = document.createElement('iframe');
  frame.name = 'target';
  host.appendChild(frame);

  const form = document.createElement('form');
  form.action = '/common/blank.html';
  form.target = 'target';
  form.setAttribute('accept-charset', 'iso-8859-1');

  const input = document.createElement('input');
  input.type = 'hidden';
  input.name = '_charset_';
  form.appendChild(input);

  const upperInput = document.createElement('input');
  upperInput.type = 'hidden';
  upperInput.name = '_CHARSET_';
  upperInput.value = 'literal';
  form.appendChild(upperInput);

  host.appendChild(form);
  form.submit();
})()
"#,
    )
    .expect("charset form submission should evaluate");

    let handle = vm
        ._context_host
        .borrow()
        .child_browsing_context_handle_by_index(0)
        .expect("test iframe should have a child browsing context");
    let pending = vm
        ._context_host
        .borrow()
        .child_browsing_context_pending_live_navigation_for_test(handle)
        .expect("form submit should queue a pending child navigation");
    let crate::native_bridge::ChildBrowsingContextBootstrap::Url(url) = pending else {
        panic!("GET form submit should use URL navigation, got {pending:?}");
    };

    assert_eq!(
        url.as_str(),
        "https://form-charset-submit.test/common/blank.html?_charset_=windows-1252&_CHARSET_=windows-1252"
    );

    let form_data_value = vm
        .eval(
            r#"
(() => {
  const form = document.querySelector('form');
  return new FormData(form).get('_charset_');
})()
"#,
        )
        .expect("FormData charset value should evaluate");
    assert_eq!(form_data_value, "UTF-8");

    let upper_form_data_value = vm
        .eval(
            r#"
(() => {
  const form = document.querySelector('form');
  return new FormData(form).get('_CHARSET_');
})()
"#,
        )
        .expect("FormData uppercase charset value should evaluate");
    assert_eq!(upper_form_data_value, "UTF-8");
}
#[test]
fn form_get_submission_uses_document_encoding_for_query() {
    let mut vm = new_storage_test_vm("https://form-gbk-submit.test/page.html");
    vm.document_runtime.set_document_character_set("GBK");

    vm.eval(
        r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const frame = document.createElement('iframe');
  frame.name = 'target';
  document.body.appendChild(frame);

  const form = document.createElement('form');
  form.action = '/search';
  form.target = 'target';

  const charset = document.createElement('input');
  charset.type = 'hidden';
  charset.name = '_charset_';
  form.appendChild(charset);

  const input = document.createElement('input');
  input.name = 'q';
  input.value = '家居';
  form.appendChild(input);

  document.body.appendChild(form);
  form.submit();
})()
"#,
    )
    .expect("GBK GET form submission should evaluate");

    let handle = vm
        ._context_host
        .borrow()
        .child_browsing_context_handle_by_index(0)
        .expect("test iframe should have a child browsing context");
    let pending = vm
        ._context_host
        .borrow()
        .child_browsing_context_pending_live_navigation_for_test(handle)
        .expect("GET form submit should queue a pending child navigation");
    let crate::native_bridge::ChildBrowsingContextBootstrap::Url(url) = pending else {
        panic!("GET form submit should use URL navigation, got {pending:?}");
    };

    assert_eq!(
        url.as_str(),
        "https://form-gbk-submit.test/search?_charset_=GBK&q=%BC%D2%BE%D3"
    );
}
#[test]
fn form_post_submission_uses_document_encoding_for_urlencoded_body() {
    let mut vm = new_storage_test_vm("https://form-gbk-post.test/page.html");
    vm.document_runtime.set_document_character_set("GBK");

    vm.eval(
        r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const frame = document.createElement('iframe');
  frame.name = 'target';
  document.body.appendChild(frame);

  const form = document.createElement('form');
  form.method = 'post';
  form.action = '/submit';
  form.target = 'target';

  const input = document.createElement('input');
  input.name = 'q';
  input.value = '家居';
  form.appendChild(input);

  document.body.appendChild(form);
  form.submit();
})()
"#,
    )
    .expect("GBK POST form submission should evaluate");

    let handle = vm
        ._context_host
        .borrow()
        .child_browsing_context_handle_by_index(0)
        .expect("test iframe should have a child browsing context");
    let pending = vm
        ._context_host
        .borrow()
        .child_browsing_context_pending_live_navigation_for_test(handle)
        .expect("POST form submit should queue a pending child navigation");
    let crate::native_bridge::ChildBrowsingContextBootstrap::Request(request) = pending else {
        panic!("POST form submit should use request navigation, got {pending:?}");
    };

    assert_eq!(request.url.as_str(), "https://form-gbk-post.test/submit");
    assert_eq!(request.method, "POST");
    assert_eq!(request.body.as_deref(), Some(&b"q=%BC%D2%BE%D3"[..]));
}
#[tokio::test]
async fn iso_2022_jp_get_form_data_url_target_posts_stateful_values() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://iso2022-form-target.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  globalThis.__iso2022FormMessages = [];
  globalThis.__iso2022FormLoads = [];
  addEventListener('message', event => __iso2022FormMessages.push(event.data));

  const target = document.createElement('iframe');
  target.name = 'target';
  target.onload = () => __iso2022FormLoads.push('load');
  document.body.appendChild(target);

  const form = document.createElement('form');
  form.acceptCharset = 'iso-2022-jp';
  const body = `<body onload="parent.postMessage({
    utf16: document.body.innerText.split('=').pop(),
    iso2022jp: unescape(location.href.split('=').pop())
  }, '*')"><plaintext>`;
  form.action = 'data:text/html;charset=iso-2022-jp,' + escape(body);
  form.target = target.name;

  const input = document.createElement('input');
  input.name = 'utf16';
  input.value = 'ABC~¤•★星🌟星★•¤~XYZ';
  form.appendChild(input);

  document.body.appendChild(form);
  form.submit();
  return 'queued';
})()
"#,
    )
    .expect("ISO-2022-JP data URL form submission should queue");

    let handle = vm
        ._context_host
        .borrow()
        .child_browsing_context_handle_by_index(0)
        .expect("target iframe should have a child browsing context");
    let pending = vm
        ._context_host
        .borrow()
        .child_browsing_context_pending_live_navigation_for_test(handle)
        .expect("form submit should queue target navigation");
    let crate::native_bridge::ChildBrowsingContextBootstrap::Url(url) = pending else {
        panic!("GET form submit should use URL navigation, got {pending:?}");
    };
    assert!(
        url.as_str()
            .starts_with("data:text/html;charset=iso-2022-jp,"),
        "unexpected form target URL: {url}"
    );

    vm.drain_ready_child_frame_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("form target setup should use only child selected tasks");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__iso2022FormMessages.length) + ':' + String(globalThis.__iso2022FormLoads.length)",
        "1:2",
        "form target message work should advance",
    )
    .await;

    let result = vm
        .eval("JSON.stringify({loads: globalThis.__iso2022FormLoads, messages: globalThis.__iso2022FormMessages})")
        .expect("ISO-2022-JP form target messages should evaluate");
    assert_eq!(
        result,
        "{\"loads\":[\"load\",\"load\"],\"messages\":[{\"utf16\":\"ABC~&#164;&#8226;★星&#127775;星★&#8226;&#164;~XYZ\",\"iso2022jp\":\"ABC~&#164;&#8226;\\u001b$B!z@1\\u001b(B&#127775;\\u001b$B@1!z\\u001b(B&#8226;&#164;~XYZ\"}]}"
    );
}
#[test]
fn url_like_href_resolution_uses_document_encoding_for_query() {
    let mut vm = new_storage_test_vm("https://href-gbk.test/page.html");
    vm.document_runtime.set_document_character_set("GBK");

    let result = vm
        .eval(
            r#"
(() => {
  const anchor = document.createElement('a');
  anchor.href = '/search?q=家居';
  return anchor.href;
})()
"#,
        )
        .expect("GBK href resolution should evaluate");

    assert_eq!(result, "https://href-gbk.test/search?q=%BC%D2%BE%D3");
}
#[test]
fn url_like_href_resolution_does_not_legacy_encode_fragment_question_mark() {
    let mut vm = new_storage_test_vm("https://href-fragment-gbk.test/page.html");
    vm.document_runtime.set_document_character_set("GBK");

    let result = vm
        .eval(
            r#"
(() => {
  const anchor = document.createElement('a');
  anchor.href = '/search#frag?q=家居';
  return anchor.href;
})()
"#,
        )
        .expect("GBK href fragment resolution should evaluate");

    assert_eq!(
        result,
        "https://href-fragment-gbk.test/search#frag?q=%E5%AE%B6%E5%B1%85"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn formdata_event_appended_entries_are_submitted_to_named_iframe() {
    let server = StaticHttpServer::spawn(1).await;
    let base_url = server.base_url();
    let page_url = base_url.join("page.html").expect("formdata page URL");
    let expected_url = base_url
        .join("common/blank.html?n1=v1&h1=vh1")
        .expect("formdata target URL");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(page_url.as_str(), &loader);

    vm.eval(
        r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const host = document.body;
  globalThis.__formdataSubmitLoads = [];
  const frameName = 'target';
  host.insertAdjacentHTML(
    'afterbegin',
    '<iframe name="' + frameName + '"></iframe>' +
      '<form action="/common/blank.html" target="' + frameName + '">' +
      '<input name=n1 value=v1>' +
      '</form>');
  const form = document.getElementsByName(frameName)[0].nextSibling;
  form.addEventListener('formdata', event => {
    event.formData.append('h1', 'vh1');
  });
  const iframe = form.previousSibling;
  iframe.onload = () => {
    __formdataSubmitLoads.push(iframe.contentWindow.location.href);
  };
  form.submit();
})()
"#,
    )
    .expect("formdata submit setup should evaluate");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__formdataSubmitLoads.join('|')",
        expected_url.as_str(),
        "formdata target navigation should complete",
    )
    .await;

    assert_eq!(
        vm.eval("__formdataSubmitLoads.join('|')")
            .expect("formdata submit load log should evaluate"),
        expected_url.as_str()
    );
    assert_eq!(
        server.finish_targets().await,
        vec!["/common/blank.html?n1=v1&h1=vh1"]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn child_click_uses_pre_dispatch_button_activation_target_after_dom_removal() {
    let server = StaticHttpServer::spawn(1).await;
    let base_url = server.base_url();
    let page_url = base_url
        .join("path/index.html")
        .expect("button child-submit page URL");
    let expected_url = base_url
        .join("path/does_not_exist.html?")
        .expect("button child-submit target URL");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(page_url.as_str(), &loader);

    vm.eval(
        r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  document.body.innerHTML =
    '<iframe name="frame1" id="frame1"></iframe>' +
    '<form id="form1" target="frame1" action="does_not_exist.html">' +
      '<button id="submitbutton" type="submit">' +
        '<span id="outerchild"><span id="innerchild">submit</span></span>' +
      '</button>' +
    '</form>';
  const frame1 = document.getElementById('frame1');
  globalThis.__buttonChildSubmitLoads = [];
  frame1.addEventListener('load', () => {
    __buttonChildSubmitLoads.push(frame1.contentWindow.location.href);
  });
  document.getElementById('submitbutton').addEventListener('click', () => {
    document.getElementById('outerchild').remove();
    document.getElementById('form1').submit();
  });
  document.getElementById('innerchild').click();
})()
"#,
    )
    .expect("button child click submit setup should evaluate");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__buttonChildSubmitLoads.join('|')",
        expected_url.as_str(),
        "button child-submit navigation should complete",
    )
    .await;

    assert_eq!(
        vm.eval("__buttonChildSubmitLoads.join('|')")
            .expect("button child submit load log should evaluate"),
        expected_url.as_str()
    );
    assert_eq!(
        server.finish_targets().await,
        vec!["/path/does_not_exist.html"]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn submit_button_click_supersedes_programmatic_submit_after_target_change() {
    let server = StaticHttpServer::spawn(1).await;
    let base_url = server.base_url();
    let page_url = base_url
        .join("path/index.html")
        .expect("button double-submit page URL");
    let expected_url = base_url
        .join("path/does_not_exist.html?navigated=1")
        .expect("button double-submit target URL");
    let expected_log = format!("frame2:{expected_url}");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(page_url.as_str(), &loader);

    vm.eval(
        r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  document.body.innerHTML =
    '<iframe name="frame1" id="frame1"></iframe>' +
    '<iframe name="frame2" id="frame2"></iframe>' +
    '<form id="form1" target="frame1" action="does_not_exist.html">' +
      '<input type="hidden" name="navigated" value="1">' +
      '<button id="submitbutton" type="submit">submit</button>' +
    '</form>';
  globalThis.__doubleSubmitLoads = [];
  const frame1 = document.getElementById('frame1');
  const frame2 = document.getElementById('frame2');
  frame1.onload = () => {
    __doubleSubmitLoads.push('frame1:' + frame1.contentWindow.location.href);
  };
  frame2.onload = () => {
    __doubleSubmitLoads.push('frame2:' + frame2.contentWindow.location.href);
  };
  const form = document.getElementById('form1');
  form.addEventListener('click', () => {
    form.submit();
    form.target = 'frame2';
  });
  document.getElementById('submitbutton').click();
})()
"#,
    )
    .expect("button double-submit setup should evaluate");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__doubleSubmitLoads.join('|')",
        &expected_log,
        "button double-submit navigation should complete",
    )
    .await;

    assert_eq!(
        vm.eval("__doubleSubmitLoads.join('|')")
            .expect("button double-submit load log should evaluate"),
        expected_log
    );
    assert_eq!(
        server.finish_targets().await,
        vec!["/path/does_not_exist.html?navigated=1"]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn distinct_forms_keep_distinct_pending_child_target_submissions() {
    let server = StaticHttpServer::spawn(2).await;
    let base_url = server.base_url();
    let page_url = base_url
        .join("path/index.html")
        .expect("parallel form-submit page URL");
    let first_url = base_url
        .join("path/first.html?first=1")
        .expect("first form-submit target URL");
    let second_url = base_url
        .join("path/second.html?second=2")
        .expect("second form-submit target URL");
    let expected_log = format!("frame1:{first_url}|frame2:{second_url}");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(page_url.as_str(), &loader);

    vm.eval(
        r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  document.body.innerHTML =
    '<iframe name="frame1" id="frame1"></iframe>' +
    '<iframe name="frame2" id="frame2"></iframe>' +
    '<form id="form1" target="frame1" action="first.html">' +
      '<input name="first" value="1">' +
    '</form>' +
    '<form id="form2" target="frame2" action="second.html">' +
      '<input name="second" value="2">' +
    '</form>';
  globalThis.__parallelFormLoads = [];
  const frame1 = document.getElementById('frame1');
  const frame2 = document.getElementById('frame2');
  frame1.onload = () => {
    __parallelFormLoads.push('frame1:' + frame1.contentWindow.location.href);
  };
  frame2.onload = () => {
    __parallelFormLoads.push('frame2:' + frame2.contentWindow.location.href);
  };
  document.getElementById('form1').submit();
  document.getElementById('form2').submit();
})()
"#,
    )
    .expect("parallel form submit setup should evaluate");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__parallelFormLoads.slice().sort().join('|')",
        &expected_log,
        "parallel form submissions should complete",
    )
    .await;

    assert_eq!(
        vm.eval("__parallelFormLoads.slice().sort().join('|')")
            .expect("parallel form submit load log should evaluate"),
        expected_log
    );
    let mut request_targets = server.finish_targets().await;
    request_targets.sort();
    assert_eq!(
        request_targets,
        vec!["/path/first.html?first=1", "/path/second.html?second=2"]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn programmatic_form_submit_keeps_successive_distinct_child_targets() {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind multi-target form server");
    let address = listener
        .local_addr()
        .expect("multi-target form server address");
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for _ in 0..3 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept multi-target form request");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream
                    .read(&mut chunk)
                    .await
                    .expect("read multi-target form request");
                assert_ne!(read, 0, "multi-target form request closed early");
                request.extend_from_slice(&chunk[..read]);
            }
            let request_line = String::from_utf8_lossy(&request)
                .lines()
                .next()
                .expect("multi-target form request line")
                .to_owned();
            requests.push(request_line);
            let body = "<!doctype html><title>target</title>";
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .expect("write multi-target form response");
        }
        requests
    });

    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let document_url = format!("http://{address}/path/index.html");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document_url, &loader);

    vm.eval(
        r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  document.body.innerHTML =
    '<iframe name="frame1" id="frame1"></iframe>' +
    '<iframe name="frame2" id="frame2"></iframe>' +
    '<iframe name="frame3" id="frame3"></iframe>' +
    '<form id="form1" action="formaction.html"></form>';
  globalThis.__multiTargetFormLoads = [];
  for (const id of ['frame1', 'frame2', 'frame3']) {
    const frame = document.getElementById(id);
    frame.onload = () => {
      __multiTargetFormLoads.push(id + ':' + frame.contentWindow.location.href);
    };
  }
  const form = document.getElementById('form1');
  form.target = 'frame1';
  form.submit();
  form.target = 'frame2';
  form.submit();
  form.target = 'frame3';
  form.submit();
})()
"#,
    )
    .expect("multi-target form submit setup should evaluate");

    vm.drain_ready_child_frame_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("form target setup should use only child selected tasks");
    let expected = ["frame1", "frame2", "frame3"]
        .map(|frame| format!("{frame}:http://{address}/path/formaction.html?"))
        .join("|");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "globalThis.__multiTargetFormLoads.slice().sort().join('|')",
        &expected,
        "all distinct form targets should complete through the wait driver",
    )
    .await;

    assert_eq!(
        vm.eval("__multiTargetFormLoads.slice().sort().join('|')")
            .expect("multi-target form submit load log should evaluate"),
        expected
    );
    let requests = server
        .await
        .expect("multi-target form server should finish");
    assert_eq!(requests.len(), 3);
    assert!(
        requests
            .iter()
            .all(|request| request == "GET /path/formaction.html HTTP/1.1"),
        "unexpected multi-target form requests: {requests:?}"
    );
}

#[test]
fn submit_event_constructor_surface() {
    let mut vm = new_storage_test_vm("https://submit-event-constructor.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const button = document.createElement('button');
  button.type = 'submit';
  const first = new SubmitEvent('submit');
  const second = new SubmitEvent('submit', {
    bubbles: true,
    cancelable: true,
    submitter: button
  });
  const desc = Object.getOwnPropertyDescriptor(SubmitEvent.prototype, 'submitter');
  const created = document.createEvent('SubmitEvent');
  return [
    typeof SubmitEvent,
    SubmitEvent.length,
    SubmitEvent.prototype.constructor === SubmitEvent,
    first instanceof SubmitEvent,
    first instanceof Event,
    first.type,
    first.bubbles,
    first.cancelable,
    first.submitter === null,
    second.bubbles,
    second.cancelable,
    second.submitter === button,
    Object.prototype.toString.call(second),
    typeof desc.get,
    desc.set === undefined,
    created instanceof SubmitEvent,
    created.submitter === null
  ].join('|');
})()
"#,
        )
        .expect("SubmitEvent constructor surface should evaluate");

    assert_eq!(
        result,
        "function|1|true|true|true|submit|false|false|true|true|true|true|[object SubmitEvent]|function|true|true|true"
    );
}
#[test]
fn live_form_request_submit_dispatches_submit_event_with_submitter() {
    let mut vm = new_storage_test_vm("https://submit-event-requestsubmit.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.body || document.documentElement || document;
  const withSubmitter = document.createElement('form');
  const button = document.createElement('button');
  button.type = 'submit';
  withSubmitter.appendChild(button);
  parent.appendChild(withSubmitter);

  const withoutSubmitter = document.createElement('form');
  parent.appendChild(withoutSubmitter);

  const out = [];
  withSubmitter.addEventListener('submit', (event) => {
    out.push([
      event instanceof SubmitEvent,
      event instanceof Event,
      event.type,
      event.bubbles,
      event.cancelable,
      event.submitter === button
    ].join(':'));
    event.preventDefault();
  }, { once: true });
  withoutSubmitter.addEventListener('submit', (event) => {
    out.push([
      event instanceof SubmitEvent,
      event.submitter === null,
      event.target === withoutSubmitter
    ].join(':'));
    event.preventDefault();
  }, { once: true });

  withSubmitter.requestSubmit(button);
  withoutSubmitter.requestSubmit();
  return out.join('|');
})()
"#,
        )
        .expect("requestSubmit should dispatch SubmitEvent with the active submitter");

    assert_eq!(result, "true:true:submit:true:true:true|true:true:true");
}
#[test]
fn live_form_request_submit_parses_webidl_submitter_argument() {
    let mut vm = new_storage_test_vm("https://request-submit-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.body || document.documentElement || document;
  const form = document.createElement('form');
  const button = document.createElement('button');
  button.type = 'submit';
  const text = document.createElement('input');
  text.type = 'text';
  form.appendChild(button);
  form.appendChild(text);
  parent.appendChild(form);

  const events = [];
  form.addEventListener('submit', event => {
    events.push(event.submitter === button ? 'button' : event.submitter === null ? 'null' : 'other');
    event.preventDefault();
  });

  const probe = callback => {
    try {
      const value = callback();
      return value === undefined ? 'undefined' : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };

  return [
    probe(() => form.requestSubmit()),
    probe(() => form.requestSubmit(undefined)),
    probe(() => form.requestSubmit(null)),
    events.join(','),
    probe(() => form.requestSubmit(button)),
    events.join(','),
    probe(() => form.requestSubmit(Symbol('submitter'))),
    probe(() => form.requestSubmit(text))
  ].join('|');
})()
"#,
        )
        .expect("requestSubmit WebIDL submitter probe should run");

    assert_eq!(
        result,
        "undefined|undefined|undefined|null,null,null|undefined|null,null,null,button|throw:TypeError|throw:TypeError"
    );
}

#[test]
fn live_form_request_submit_does_not_reenter_submission() {
    let mut vm = new_storage_test_vm("https://request-submit-reentrant.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.body || document.documentElement || document;

  function run(start, reenter) {
    const form = document.createElement('form');
    const submitter = document.createElement('input');
    submitter.type = 'submit';
    form.appendChild(submitter);
    parent.appendChild(form);

    let submitCounter = 0;
    form.addEventListener('submit', event => {
      submitCounter++;
      if (submitCounter < 10) {
        if (reenter === 'requestSubmit') {
          form.requestSubmit();
        } else {
          submitter.click();
        }
      }
      event.preventDefault();
    });

    if (start === 'requestSubmit') {
      form.requestSubmit();
    } else {
      submitter.click();
    }
    form.remove();
    return String(submitCounter);
  }

  return [
    run('requestSubmit', 'requestSubmit'),
    run('requestSubmit', 'click'),
    run('click', 'requestSubmit')
  ].join('|');
})()
"#,
        )
        .expect("requestSubmit should not reenter form submission");

    assert_eq!(result, "1|1|1");
}

#[test]
fn live_form_request_submit_does_not_reenter_interactive_validation() {
    let mut vm = new_storage_test_vm("https://request-submit-validation-reentrant.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.body || document.documentElement || document;

  function run(start, reenter) {
    const form = document.createElement('form');
    const submitter = document.createElement('input');
    submitter.type = 'submit';
    const required = document.createElement('input');
    required.required = true;
    form.appendChild(submitter);
    form.appendChild(required);
    parent.appendChild(form);

    let invalidCounter = 0;
    required.addEventListener('invalid', () => {
      invalidCounter++;
      if (invalidCounter < 10) {
        if (reenter === 'requestSubmit') {
          form.requestSubmit();
        } else {
          submitter.click();
        }
      }
    });

    if (start === 'requestSubmit') {
      form.requestSubmit();
    } else {
      submitter.click();
    }
    form.remove();
    return String(invalidCounter);
  }

  return [
    run('requestSubmit', 'requestSubmit'),
    run('requestSubmit', 'click'),
    run('click', 'requestSubmit')
  ].join('|');
})()
"#,
        )
        .expect("requestSubmit should not reenter interactive validation");

    assert_eq!(result, "1|1|1");
}

#[test]
fn live_form_request_submit_ignores_disconnected_form() {
    let mut vm = new_storage_test_vm("https://request-submit-disconnected.test/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const form = document.createElement('form');
  const submitter = document.createElement('input');
  submitter.type = 'submit';
  form.appendChild(submitter);

  let submitCounter = 0;
  form.addEventListener('submit', event => {
    submitCounter++;
    event.preventDefault();
  });

  form.requestSubmit();
  submitter.click();
  return String(submitCounter);
})()
"#,
        )
        .expect("disconnected form requestSubmit should run");

    assert_eq!(result, "0");
    assert!(
        vm.take_pending_location_navigation_with_seed().is_none(),
        "disconnected form submission must not queue a navigation"
    );
}

#[test]
fn live_form_request_submit_rejects_empty_form_attribute_submitter() {
    let mut vm = new_storage_test_vm("https://request-submit-empty-form-attr.test/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.body || document.documentElement || document;
  const form = document.createElement('form');
  const submitter = document.createElement('input');
  submitter.type = 'submit';
  submitter.setAttribute('form', '');
  form.appendChild(submitter);
  parent.appendChild(form);

  let submitCounter = 0;
  form.addEventListener('submit', event => {
    submitCounter++;
    event.preventDefault();
  });

  let requestSubmitError = 'missing';
  try {
    form.requestSubmit(submitter);
  } catch (error) {
    requestSubmitError = error.name;
  }

  return [
    submitter.form === null,
    requestSubmitError,
    submitCounter
  ].join('|');
})()
"#,
        )
        .expect("empty form attribute submitter should be rejected");

    assert_eq!(result, "true|NotFoundError|0");
    assert!(
        vm.take_pending_location_navigation_with_seed().is_none(),
        "empty form attribute submitter must not queue a navigation"
    );
}

#[test]
fn live_submit_button_click_dispatches_submit_event_with_submitter_and_click_order() {
    let mut vm = new_storage_test_vm("https://submit-button-click.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.body || document.documentElement || document;

  function run(kind) {
    const form = document.createElement('form');
    const submitter = document.createElement(kind === 'button' ? 'button' : 'input');
    submitter.type = 'submit';
    if (kind === 'button') {
      submitter.textContent = 'go';
    } else {
      submitter.value = 'go';
    }
    form.appendChild(submitter);
    parent.appendChild(form);

    const out = [];
    submitter.addEventListener('click', (event) => {
      out.push([
        'click',
        event.target === submitter,
        event.currentTarget === submitter
      ].join(':'));
    }, { once: true });
    form.addEventListener('submit', (event) => {
      out.push([
        'submit',
        event instanceof SubmitEvent,
        event.submitter === submitter,
        event.target === form,
        event.currentTarget === form,
        event.cancelable,
        event.bubbles
      ].join(':'));
      event.preventDefault();
    }, { once: true });

    submitter.click();
    form.remove();
    return out.join('|');
  }

  return [run('button'), run('input')].join('||');
})()
"#,
        )
        .expect("submit button click should dispatch SubmitEvent with the active submitter");

    assert_eq!(
        result,
        "click:true:true|submit:true:true:true:true:true:true||click:true:true|submit:true:true:true:true:true:true"
    );
}
#[test]
fn live_child_click_bubbles_to_ancestor_button() {
    let mut vm = new_storage_test_vm("https://child-click-bubbles.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.body || document.documentElement || document;
  const button = document.createElement('button');
  const outer = document.createElement('span');
  const inner = document.createElement('span');
  outer.appendChild(inner);
  button.appendChild(outer);
  parent.appendChild(button);

  const out = [];
  button.addEventListener('click', event => {
    out.push([
      event.target === inner,
      event.currentTarget === button,
      event.bubbles
    ].join(':'));
  });

  inner.click();
  button.remove();
  return out.join('|');
})()
"#,
        )
        .expect("child click should bubble to ancestor button");

    assert_eq!(result, "true:true:true");
}
#[test]
fn parsed_child_click_bubbles_to_ancestor_button() {
    let mut vm = new_parsed_test_vm(
        "https://parsed-child-click-bubbles.test/",
        r#"
<!doctype html>
<form id=form1>
  <button id=submitbutton type=submit>
    <span id=outerchild>
      <span id=innerchild>submit</span>
    </span>
  </button>
</form>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const button = document.getElementById('submitbutton');
  const inner = document.getElementById('innerchild');
  const out = [];
  button.addEventListener('click', event => {
    out.push([
      event.target === inner,
      event.currentTarget === button,
      event.bubbles
    ].join(':'));
  });

  inner.click();
  return out.join('|');
})()
"#,
        )
        .expect("parsed child click should bubble to ancestor button");

    assert_eq!(result, "true:true:true");
}
#[test]
fn load_handler_child_click_bubbles_to_ancestor_button() {
    let mut vm = new_parsed_test_vm(
        "https://load-child-click-bubbles.test/",
        r#"
<!doctype html>
<iframe name=frame1 id=frame1></iframe>
<form id=form1 target=frame1 action="does_not_exist.html">
  <button id=submitbutton type=submit>
    <span id=outerchild>
      <span id=innerchild>submit</span>
    </span>
  </button>
</form>
"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const out = [];
  window.addEventListener('load', () => {
    const button = document.getElementById('submitbutton');
    const inner = document.getElementById('innerchild');
    button.addEventListener('click', event => {
      out.push([
        event.target === inner,
        event.currentTarget === button,
        event.bubbles
      ].join(':'));
      event.preventDefault();
    });
    inner.click();
  });
  window.dispatchEvent(new Event('load'));
  return out.join('|');
})()
"#,
        )
        .expect("child click in load handler should bubble to ancestor button");

    assert_eq!(result, "true:true:true");
}
#[test]
fn window_load_task_nested_child_click_invokes_ancestor_button_listener() {
    let mut vm = new_parsed_test_vm(
        "https://real-load-child-click-bubbles.test/",
        r#"
<!doctype html>
<form id=form1>
  <button id=submitbutton type=submit>
    <span id=outerchild>
      <span id=innerchild>submit</span>
    </span>
  </button>
</form>
"#,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__loadClickEvents = [];
  window.addEventListener('load', () => {
    const button = document.getElementById('submitbutton');
    const inner = document.getElementById('innerchild');
    button.onclick = () => {
      globalThis.__loadClickEvents.push('property');
    };
    button.addEventListener('click', event => {
      globalThis.__loadClickEvents.push([
        event.target === inner,
        event.currentTarget === button,
        event.bubbles
      ].join(':'));
      event.preventDefault();
    });
    inner.click();
  });
})()
"#,
    )
    .expect("load click setup should evaluate");

    vm.dispatch_window_load_event()
        .expect("window load should dispatch nested click");

    let result = vm
        .eval("globalThis.__loadClickEvents.join('|')")
        .expect("load click events should be readable");

    assert_eq!(result, "property|true:true:true");
}
#[test]
fn button_activation_respects_wpt_event_cancellation_slice() {
    let mut vm = new_storage_test_vm("https://button-events-slice.test/path/index.html");

    let result = vm
        .eval(
            r#"
(() => {
  const parent = document.body || document.documentElement || document;
  const form = document.createElement('form');
  form.name = 'fm1';
  form.action = '/submit';
  const button = document.createElement('button');
  button.id = 'btn';
  form.appendChild(button);
  parent.appendChild(form);
  const out = [];

  form.onsubmit = event => {
    out.push(`submit:${button.type}`);
    event.preventDefault();
  };
  button.type = 'submit';
  button.click();

  form.onreset = event => {
    out.push(`reset:${button.type}`);
    event.preventDefault();
  };
  button.type = 'reset';
  button.click();

  form.onsubmit = () => out.push('bad-submit');
  form.onreset = () => out.push('bad-reset');
  button.type = 'button';
  button.click();

  form.onsubmit = event => {
    out.push(`dynamic-submit:${button.type}`);
    event.preventDefault();
  };
  button.onclick = () => { button.type = 'submit'; };
  button.type = 'button';
  button.click();

  form.onreset = event => {
    out.push(`dynamic-reset:${button.type}`);
    event.preventDefault();
  };
  button.onclick = () => { button.type = 'reset'; };
  button.type = 'button';
  button.click();

  return out.join('|');
})()
"#,
        )
        .expect("button activation WPT slice should evaluate");

    assert_eq!(
        result,
        "submit:submit|reset:reset|dynamic-submit:submit|dynamic-reset:reset"
    );
    assert!(
        vm.take_pending_location_navigation_with_seed().is_none(),
        "canceled or non-submit button activation must not queue navigation"
    );
}

#[test]
fn nested_form_submit_and_reset_stop_before_ancestor_form_bubble_listeners() {
    let mut vm = new_storage_test_vm("https://nested-form-events.test/path/index.html");

    let result = vm
        .eval(
            r#"
(() => {
  const out = [];
  const root = document.body || document.documentElement || document;
  for (const type of ['submit', 'reset']) {
    const outer = document.createElement('form');
    outer.id = `${type}-outer`;
    const parentControl = document.createElement('input');
    parentControl.type = type;
    outer.append(parentControl);
    root.append(outer);

    const inner = document.createElement('form');
    inner.id = `${type}-inner`;
    const childControl = document.createElement('button');
    childControl.type = type;
    inner.append(childControl);
    parentControl.append(inner);

    document.addEventListener(type, () => out.push(`${type}:document-capture`), {
      capture: true,
      once: true
    });
    document.addEventListener(type, () => out.push(`${type}:document-bubble`), {
      once: true
    });
    outer.addEventListener(type, () => out.push(`${type}:outer-capture`), {
      capture: true
    });
    outer.addEventListener(type, () => out.push(`${type}:outer-bubble`));
    parentControl.addEventListener(type, () => out.push(`${type}:parent-bubble`));
    inner.addEventListener(type, event => {
      out.push(`${type}:inner`);
      event.preventDefault();
    });

    childControl.click();
    outer.remove();
  }
  return out.join('|');
})()
"#,
        )
        .expect("nested form event propagation should evaluate");

    assert_eq!(
        result,
        "submit:document-capture|submit:outer-capture|submit:inner|submit:parent-bubble|reset:document-capture|reset:outer-capture|reset:inner|reset:parent-bubble"
    );
}

#[test]
fn child_button_activation_self_submit_dispatches_iframe_load() {
    let mut vm = new_storage_test_vm("https://button-child-self-submit.test/path/page.html");

    vm.eval(
        r#"
(() => {
  globalThis.__childButtonSelfSubmitLoads = [];
  const iframe = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(iframe);
  iframe.contentDocument.body.innerHTML =
    '<form action="about:blank"><button id="submit">Submit</button></form>';
  iframe.contentDocument.querySelector("form").addEventListener("submit", () => {
    parent.__childButtonSelfSubmitLoads.push("submit");
  });
})()
"#,
    )
    .expect("child button self-submit setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    vm.eval(
        r#"
(() => {
  const iframe = document.querySelector("iframe");
  iframe.onload = () => {
    __childButtonSelfSubmitLoads.push("load");
  };
  iframe.contentDocument.getElementById("submit").click();
})()
"#,
    )
    .expect("child button self-submit click should evaluate");
    let handle = vm
        ._context_host
        .borrow()
        .child_browsing_context_handle_by_index(0)
        .expect("test iframe should have a child browsing context");
    let pending = vm
        ._context_host
        .borrow()
        .child_browsing_context_pending_live_navigation_for_test(handle)
        .expect("child self-submit should queue a pending child navigation");
    let crate::native_bridge::ChildBrowsingContextBootstrap::Url(url) = pending else {
        panic!("child self-submit should use URL navigation, got {pending:?}");
    };
    assert_eq!(url.as_str(), "about:blank?");
    vm.drain_pending_child_frame_work_for_test();

    assert_eq!(
        vm.eval("__childButtonSelfSubmitLoads.join('|')")
            .expect("child button self-submit load log should evaluate"),
        "submit|load"
    );
}
#[test]
fn anchor_click_queues_pending_top_level_location_navigation() {
    let mut vm = new_storage_test_vm("https://anchor-click-navigation.test/path/index.html");

    vm.eval(
        r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const anchor = document.createElement('a');
  anchor.href = '/next?from=anchor#frag';
  document.body.appendChild(anchor);
  anchor.click();
})()
"#,
    )
    .expect("anchor click should execute");

    let pending = vm
        .take_pending_location_navigation_with_seed()
        .expect("anchor click should queue a pending location navigation");
    assert_eq!(
        pending.url.as_str(),
        "https://anchor-click-navigation.test/next?from=anchor#frag"
    );
    assert!(
        pending.entry_seed.is_none(),
        "plain anchor click should not synthesize a history seed"
    );
}

#[test]
fn taking_protocol_handoff_location_navigation_restores_source_location() {
    let mut vm = new_storage_test_vm("https://anchor-click-navigation.test/path/index.html");

    let assigned_path = vm
        .eval(
            r#"
(() => {
  location.assign('/download');
  return location.pathname;
})()
"#,
        )
        .expect("location assignment should evaluate");
    assert_eq!(assigned_path, "/download");

    let pending_url = vm
        .take_pending_non_javascript_location_navigation()
        .map(|pending| pending.url)
        .expect("location assignment should queue a protocol-visible navigation");
    assert_eq!(
        pending_url.as_str(),
        "https://anchor-click-navigation.test/download"
    );

    let restored_path = vm
        .eval("location.pathname")
        .expect("restored location should evaluate");
    assert_eq!(restored_path, "/path/index.html");
}

#[test]
fn anchor_same_document_fragment_click_does_not_queue_top_level_navigation() {
    let mut vm = new_storage_test_vm("https://anchor-fragment.test/path/index.html");

    vm.eval(
        r##"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const target = document.createElement('section');
  target.id = 'target';
  const anchor = document.createElement('a');
  anchor.href = '#target';
  document.body.append(anchor, target);
  anchor.click();
})()
"##,
    )
    .expect("same-document fragment anchor click should execute");

    assert!(
        vm.take_pending_location_navigation_with_seed().is_none(),
        "same-document fragment anchor clicks must not queue a top-level reload"
    );
}
#[test]
fn anchor_top_and_parent_targets_queue_plain_top_level_navigation() {
    for target in ["_top", "_parent", "_ToP", "_PaReNt", "_SeLf"] {
        let mut vm = new_storage_test_vm("https://anchor-target-navigation.test/path/index.html");

        vm.eval(&format!(
            r#"
(() => {{
  if (!document.documentElement) {{
    document.appendChild(document.createElement('html'));
  }}
  if (!document.body) {{
    document.documentElement.appendChild(document.createElement('body'));
  }}
  const anchor = document.createElement('a');
  anchor.href = '/next?target={target}#frag';
  anchor.target = '{target}';
  document.body.appendChild(anchor);
  anchor.click();
}})()
"#
        ))
        .expect("anchor click should execute");

        let pending = vm
            .take_pending_location_navigation_with_seed()
            .expect("targeted anchor click should queue a pending location navigation");
        assert_eq!(
            pending.url.as_str(),
            format!("https://anchor-target-navigation.test/next?target={target}#frag")
        );
        assert!(
            pending.entry_seed.is_none(),
            "{target} anchor click should not synthesize a history seed"
        );
    }
}
#[test]
fn anchor_download_click_returns_pending_download_without_navigation() {
    let mut vm = new_parsed_test_vm(
        "https://anchor-download-navigation.test/path/index.html",
        r#"<html><body>
            <a href="/download.txt" download="saved.txt">download</a>
        </body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  document.querySelector('a').focus();
})()
"#,
    )
    .expect("anchor focus should succeed");

    let outcome = vm
        .dispatch_key_event("keydown", "Enter", "Enter", "", 0, false, false)
        .expect("keydown Enter should dispatch");
    assert!(outcome.handled);
    let pending_download = outcome
        .pending_download
        .expect("download anchor activation should return a pending download activation");
    assert_eq!(
        pending_download.url,
        "https://anchor-download-navigation.test/download.txt"
    );
    assert_eq!(
        pending_download.suggested_filename.as_deref(),
        Some("saved.txt")
    );
    assert!(
        vm.take_pending_location_navigation_with_seed().is_none(),
        "download anchor click should not queue a pending location navigation"
    );
}

#[test]
fn anchor_download_synthetic_click_records_pending_download_without_navigation() {
    let mut vm = new_parsed_test_vm(
        "https://anchor-download-synthetic-click.test/path/index.html",
        r#"<html><body>
            <a id="download_link" href="/download.txt" download="saved.txt">download</a>
        </body></html>"#,
    );

    vm.eval("download_link.click(); 'done'")
        .expect("synthetic anchor click should execute");

    let pending_downloads = vm.take_pending_download_activations();
    assert_eq!(pending_downloads.len(), 1);
    assert_eq!(
        pending_downloads[0].url,
        "https://anchor-download-synthetic-click.test/download.txt"
    );
    assert_eq!(
        pending_downloads[0].suggested_filename.as_deref(),
        Some("saved.txt")
    );
    assert!(
        vm.take_pending_location_navigation_with_seed().is_none(),
        "download anchor synthetic click should not queue a pending location navigation"
    );
}

#[test]
fn trusted_alt_click_downloads_an_anchor_without_a_download_attribute() {
    let mut vm = new_parsed_test_vm(
        "https://anchor-alt-download.test/path/index.html",
        r#"<!doctype html><html><head><style>
            body { margin: 0; }
            #download_link { display: block; width: 100px; height: 100px; }
        </style></head><body>
            <a id="download_link" href="/download.txt">download</a>
        </body></html>"#,
    );

    vm.dispatch_mouse_event_at_point_with_pointer_and_modifiers(
        20.0,
        20.0,
        "mousedown",
        0,
        None,
        1,
        0.0,
        0.0,
        crate::runtime::RendererPointerEventProperties::default(),
        1,
    )
    .expect("alt mousedown should dispatch");
    let outcome = vm
        .dispatch_mouse_event_at_point_with_pointer_and_modifiers(
            20.0,
            20.0,
            "mouseup",
            0,
            None,
            1,
            0.0,
            0.0,
            crate::runtime::RendererPointerEventProperties::default(),
            1,
        )
        .expect("alt mouseup should dispatch");

    let pending_download = outcome
        .pending_download
        .expect("trusted alt click should request a download");
    assert_eq!(
        pending_download.url,
        "https://anchor-alt-download.test/download.txt"
    );
    assert!(pending_download.suggested_filename.is_none());
    assert!(vm.take_pending_location_navigation_with_seed().is_none());
    assert!(vm.take_pending_popup_activations().is_empty());
}

#[test]
fn window_open_in_click_listener_uses_current_input_popup_disposition() {
    #[cfg(target_os = "macos")]
    let (platform_modifier, synthetic_modifier) = (4, "metaKey: true");
    #[cfg(not(target_os = "macos"))]
    let (platform_modifier, synthetic_modifier) = (2, "ctrlKey: true");

    let mut trusted = new_storage_test_vm("https://window-open-input-policy.test/path/index.html");
    trusted
        .eval(
            r#"
(() => {
  const html = document.createElement('html');
  const body = document.createElement('body');
  body.style.margin = '0';
  const button = document.createElement('button');
  button.id = 'go';
  button.type = 'button';
  button.style.cssText = 'display:block;width:100px;height:100px';
  button.onclick = () => window.open('/trusted', '_blank');
  body.appendChild(button);
  html.appendChild(body);
  document.appendChild(html);
})()
"#,
        )
        .expect("trusted window.open fixture should initialize");
    trusted
        .dispatch_mouse_event_at_point_with_pointer_and_modifiers(
            20.0,
            20.0,
            "mousedown",
            0,
            None,
            1,
            0.0,
            0.0,
            crate::runtime::RendererPointerEventProperties::default(),
            platform_modifier,
        )
        .expect("new-tab-modified mousedown should dispatch");
    trusted
        .dispatch_mouse_event_at_point_with_pointer_and_modifiers(
            20.0,
            20.0,
            "mouseup",
            0,
            None,
            1,
            0.0,
            0.0,
            crate::runtime::RendererPointerEventProperties::default(),
            platform_modifier,
        )
        .expect("new-tab-modified mouseup should dispatch");
    let trusted_popups = trusted.take_pending_popup_activations();
    assert_eq!(trusted_popups.len(), 1);
    assert_eq!(
        trusted_popups[0].disposition(),
        crate::RendererPopupDisposition::Background
    );

    let mut synthetic =
        new_storage_test_vm("https://window-open-synthetic-policy.test/path/index.html");
    synthetic
        .eval(
            r#"
(() => {
  const html = document.createElement('html');
  const body = document.createElement('body');
  const button = document.createElement('button');
  button.id = 'go';
  button.type = 'button';
  button.onclick = () => window.open('/synthetic', '_blank');
  body.appendChild(button);
  html.appendChild(body);
  document.appendChild(html);
})()
"#,
        )
        .expect("synthetic window.open fixture should initialize");
    synthetic
        .eval(&format!(
            "go.dispatchEvent(new MouseEvent('click', {{ button: 0, {synthetic_modifier} }})); 'done'"
        ))
        .expect("synthetic modified click should dispatch");
    let synthetic_popups = synthetic.take_pending_popup_activations();
    assert_eq!(synthetic_popups.len(), 1);
    assert_eq!(
        synthetic_popups[0].disposition(),
        crate::RendererPopupDisposition::Foreground,
        "a synthesized modifier cannot create a tab-under"
    );
}

#[test]
fn keyboard_activation_click_preserves_modifiers() {
    let mut vm = new_parsed_test_vm(
        "https://keyboard-activation-modifiers.test/path/index.html",
        r#"<html><body>
            <input id="box" type="checkbox">
            <button id="button" type="button">Run</button>
        </body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  window.__clickLog = [];
  for (const id of ['box', 'button']) {
    document.getElementById(id).addEventListener('click', event => {
      __clickLog.push(`${id}:${event.detail}:${event.ctrlKey}:${event.shiftKey}:${event.altKey}:${event.metaKey}`);
    });
  }
  document.getElementById('box').focus();
})()
"#,
    )
    .expect("keyboard activation setup should evaluate");

    const CTRL_SHIFT_MODIFIERS: u8 = 2 | 8;
    vm.dispatch_key_event(
        "keydown",
        " ",
        "Space",
        "",
        CTRL_SHIFT_MODIFIERS,
        false,
        false,
    )
    .expect("space keydown should dispatch");
    let checkbox_outcome = vm
        .dispatch_key_event(
            "keyup",
            " ",
            "Space",
            "",
            CTRL_SHIFT_MODIFIERS,
            false,
            false,
        )
        .expect("space keyup should dispatch");
    assert!(checkbox_outcome.handled);

    vm.eval("document.getElementById('button').focus()")
        .expect("button focus should evaluate");
    let button_outcome = vm
        .dispatch_key_event(
            "keydown",
            "Enter",
            "Enter",
            "",
            CTRL_SHIFT_MODIFIERS,
            false,
            false,
        )
        .expect("enter keydown should dispatch");
    assert!(button_outcome.handled);

    let result = vm
        .eval("`${document.getElementById('box').checked}|${__clickLog.join('|')}`")
        .expect("keyboard activation modifier probe should evaluate");

    assert_eq!(
        result,
        "true|box:1:true:true:false:false|button:1:true:true:false:false"
    );
}

#[test]
fn image_submit_click_coordinates_use_live_layout_geometry() {
    let mut vm = new_parsed_test_vm(
        "https://image-submit-coordinates.test/path/index.html",
        r#"<!doctype html><html><head><style>
            body { margin: 0; }
            #photo {
                position: absolute;
                left: 100px;
                top: 50px;
                width: 80px;
                height: 40px;
                border: 0;
                padding: 0;
            }
        </style></head><body>
            <form id="form"><input id="photo" type="image" name="photo"></form>
        </body></html>"#,
    );
    vm.eval(
        r#"
const form = document.getElementById('form');
form.addEventListener('submit', event => {
  event.preventDefault();
  globalThis.__imageSubmitEntries = JSON.stringify(
    Array.from(new FormData(form, event.submitter))
  );
});
"#,
    )
    .expect("image submit coordinate fixture should initialize");

    vm.dispatch_mouse_event_at_point(125.4, 68.6, "mousedown", 0, None, 0.0, 0.0)
        .expect("image submitter mousedown should dispatch");
    let outcome = vm
        .dispatch_mouse_event_at_point(125.4, 68.6, "mouseup", 0, None, 0.0, 0.0)
        .expect("image submitter mouseup should dispatch");
    assert!(outcome.handled);

    let entries = vm
        .eval("globalThis.__imageSubmitEntries")
        .expect("image submit coordinate entries should evaluate");
    assert_eq!(entries, r#"[["photo.x","25"],["photo.y","19"]]"#);
}

#[test]
fn button_click_listener_window_open_self_reports_top_level_navigation() {
    let mut vm = new_parsed_test_vm(
        "https://button-open-self-outcome.test/path/index.html",
        r#"<html><body>
            <button id="go" type="button" onclick="window.open('/next', '_self')">go</button>
        </body></html>"#,
    );

    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("button mousedown should dispatch");
    let outcome = vm
        .dispatch_mouse_event_at_point(20.0, 20.0, "mouseup", 0, None, 0.0, 0.0)
        .expect("button mouseup should dispatch");

    assert!(outcome.handled);
    assert!(
        outcome.triggered_top_level_navigation,
        "click listener window.open(_self) must be reported to protocol input completion"
    );
    let pending = vm
        .take_pending_location_navigation_with_seed()
        .expect("window.open(_self) should queue a top-level navigation");
    assert_eq!(
        pending.url.as_str(),
        "https://button-open-self-outcome.test/next"
    );
}

#[test]
fn cross_port_anchor_click_dispatches_non_interceptable_navigate_event() {
    let mut vm = new_storage_test_vm("http://127.0.0.1:8000/page.html");

    let result = vm
        .eval(
            r#"
if (!document.documentElement) {
  document.appendChild(document.createElement('html'));
}
if (!document.body) {
  document.documentElement.appendChild(document.createElement('body'));
}
const anchor = document.createElement('a');
anchor.href = 'http://127.0.0.1:8800/';
document.body.appendChild(anchor);
let seen = null;
navigation.onnavigate = event => {
  seen = [
    event.navigationType,
    event.canIntercept,
    event.destination.sameDocument,
    event.sourceElement === anchor
  ].join('|');
  event.preventDefault();
};
anchor.click();
seen
"#,
        )
        .expect("cross-port anchor navigate event probe should evaluate");

    assert_eq!(result, "push|false|false|true");
    assert!(
        vm.take_pending_location_navigation_with_seed().is_none(),
        "prevented cross-port anchor navigate event should not queue a location navigation"
    );
}
#[test]
fn iframe_anchor_download_click_dispatches_child_navigate_event() {
    let mut vm = new_storage_test_vm("https://iframe-download-navigation.test/page.html");

    let result = vm
        .eval(
            r#"
const frame = document.createElement('iframe');
if (!document.documentElement) {
  document.appendChild(document.createElement('html'));
}
if (!document.body) {
  document.documentElement.appendChild(document.createElement('body'));
}
document.body.appendChild(frame);
let topNavigate = false;
let childNavigate = false;
navigation.onnavigate = () => { topNavigate = true; };
frame.contentWindow.navigation.onnavigate = event => {
  childNavigate = [
    event.navigationType,
    event.downloadRequest
  ].join('|');
  event.preventDefault();
};
const anchor = frame.contentDocument.createElement('a');
anchor.href = '?download';
anchor.download = '';
frame.contentDocument.body.appendChild(anchor);
anchor.click();
`${topNavigate}:${childNavigate}`
"#,
        )
        .expect("iframe download anchor navigate event probe should evaluate");

    assert_eq!(result, "false:push|");
}
#[test]
fn file_input_click_returns_pending_file_chooser_activation() {
    const FILE_INPUT_HIT_X: f64 = 20.0;
    const FILE_INPUT_HIT_Y: f64 = 20.0;

    let mut vm = new_parsed_test_vm(
        "https://file-input-click.test/path/index.html",
        r#"<html><body>
            <input id="picker" type="file" multiple>
        </body></html>"#,
    );
    let lifecycle = crate::runtime::RendererDocumentLifecycleJournalHandle::new_initial(
        crate::runtime::PageId::new_for_testing(91),
    );
    let source_document = lifecycle.identity();
    vm.set_root_document_lifecycle(lifecycle);
    vm.dispatch_mouse_event_at_point(
        FILE_INPUT_HIT_X,
        FILE_INPUT_HIT_Y,
        "mousedown",
        0,
        None,
        0.0,
        0.0,
    )
    .expect("file input mousedown should dispatch");
    let outcome = vm
        .dispatch_mouse_event_at_point(
            FILE_INPUT_HIT_X,
            FILE_INPUT_HIT_Y,
            "mouseup",
            0,
            None,
            0.0,
            0.0,
        )
        .expect("file input mouseup should dispatch");
    assert!(outcome.handled);
    let file_chooser = outcome
        .pending_file_chooser
        .expect("file input activation should queue a pending file chooser activation");
    assert_eq!(file_chooser.source_document(), source_document);
    assert_eq!(file_chooser.source_frame_id(), None);
    assert_eq!(file_chooser.backend_node_id(), 0);
    assert!(file_chooser.node_id.is_some());
    assert!(file_chooser.allow_multiple());
}

#[test]
fn file_input_click_document_open_matches_chromium_activation_state() {
    let cases = [
        ("file", "", "file", false, Some(false)),
        ("file", "picker.type = 'text';", "text", false, None),
        ("file", "picker.multiple = true;", "file", true, Some(true)),
        ("text", "picker.type = 'file';", "file", false, Some(false)),
    ];

    for (index, (initial_type, mutation, expected_type, expected_multiple, expected_chooser)) in
        cases.into_iter().enumerate()
    {
        let mut vm = new_parsed_test_vm(
            "https://file-input-document-open.test/path/index.html",
            &format!(r#"<html><body><input id="picker" type="{initial_type}"></body></html>"#),
        );
        let lifecycle = crate::runtime::RendererDocumentLifecycleJournalHandle::new_initial(
            crate::runtime::PageId::new_for_testing(93 + index as u64),
        );
        let source_document = lifecycle.identity();
        vm.set_root_document_lifecycle(lifecycle.clone());
        let picker_handle = vm
            .document_runtime
            .get_element_by_id("picker")
            .expect("source file input should exist");

        let retained_state = vm
            .eval(&format!(
            r#"
const picker = document.getElementById('picker');
picker.addEventListener('click', () => {{
  {mutation}
  document.open();
  document.write('<!doctype html><input id="replacement" type="file">');
  document.close();
}});
picker.click();
`${{picker.type}}|${{picker.multiple}}|${{picker.isConnected}}|${{document.getElementById('replacement') instanceof HTMLInputElement}}`
"#
        ))
            .expect("file input listener should replace the Document during activation");

        assert_eq!(
            retained_state,
            format!("{expected_type}|{expected_multiple}|false|true"),
            "the source wrapper must still address the detached source element"
        );

        assert_ne!(
            lifecycle.identity(),
            source_document,
            "the listener must replace the Document before the default action finishes"
        );
        let retained_picker = vm
            .document_runtime
            .dom_host()
            .node(picker_handle)
            .and_then(crate::dom::native::Node::as_element)
            .expect("the detached source input should retain its native node identity");
        assert!(retained_picker.is_html_input());
        assert_eq!(retained_picker.input_type(), expected_type);
        assert_eq!(
            retained_picker.attribute("multiple").is_some(),
            expected_multiple
        );
        assert!(!vm.document_runtime.dom_host().is_connected(picker_handle));
        let replacement_handle = vm
            .document_runtime
            .get_element_by_id("replacement")
            .expect("replacement input should exist");
        assert_ne!(replacement_handle, picker_handle);
        assert!(replacement_handle.index() > picker_handle.index());

        let activations = vm.take_pending_file_chooser_activations();
        match expected_chooser {
            Some(allow_multiple) => {
                assert_eq!(activations.len(), 1);
                let activation = &activations[0];
                assert_eq!(activation.source_document(), source_document);
                assert_eq!(activation.source_frame_id(), None);
                assert_eq!(activation.node_id, Some(picker_handle));
                assert!(
                    activation.backend_node_id() >= moli_page_types::RENDERER_BACKEND_NODE_ID_START,
                    "the producer must freeze a renderer backend id before document.open() \
                     retires the source Document"
                );
                assert_eq!(activation.allow_multiple(), allow_multiple);
            }
            None => assert!(
                activations.is_empty(),
                "changing the activated element away from type=file must cancel the chooser"
            ),
        }
    }
}

#[test]
fn webdriver_bidi_file_prompt_handler_dispatches_trusted_cancel_without_pending_chooser() {
    let mut vm = new_parsed_test_vm(
        "https://file-input-bidi-cancel.test/path/index.html",
        r#"<html><body></body></html>"#,
    );
    let message = serde_json::json!({
        "id": 7,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"
(() => {
  const picker = document.createElement('input');
  picker.type = 'file';
  let trusted = 'missing';
  picker.addEventListener('cancel', event => {
    trusted = String(event.isTrusted);
  });
  document.body.appendChild(picker);
  picker.click();
  return trusted;
})()
"#,
            "awaitPromise": false,
            "returnByValue": true,
            "userGesture": true,
            "__moliWebDriverBidiFilePromptHandler": "accept"
        }
    });
    let messages = vm
        .dispatch_inspector_protocol_message(&message.to_string())
        .expect("Runtime.evaluate with BiDi file handler should dispatch");
    let response = messages
        .iter()
        .find(|message| message["id"] == serde_json::json!(7))
        .expect("Runtime.evaluate response");
    assert_eq!(
        response["result"]["result"]["value"],
        serde_json::json!("true")
    );
    assert!(
        vm.take_pending_file_chooser_activations().is_empty(),
        "auto-canceled BiDi file prompt must not leave a pending chooser"
    );
}

#[test]
fn label_click_for_file_input_returns_pending_file_chooser_activation() {
    const LABEL_HIT_X: f64 = 20.0;
    const LABEL_HIT_Y: f64 = 20.0;

    let mut vm = new_parsed_test_vm(
        "https://file-label-click.test/path/index.html",
        r#"<html><body>
            <label for="picker" style="display:block;width:120px;height:40px">Choose file</label>
            <input id="picker" type="file" multiple style="display:block;margin-top:20px">
        </body></html>"#,
    );
    let lifecycle = crate::runtime::RendererDocumentLifecycleJournalHandle::new_initial(
        crate::runtime::PageId::new_for_testing(92),
    );
    let source_document = lifecycle.identity();
    vm.set_root_document_lifecycle(lifecycle);
    vm.dispatch_mouse_event_at_point(LABEL_HIT_X, LABEL_HIT_Y, "mousedown", 0, None, 0.0, 0.0)
        .expect("label mousedown should dispatch");
    let outcome = vm
        .dispatch_mouse_event_at_point(LABEL_HIT_X, LABEL_HIT_Y, "mouseup", 0, None, 0.0, 0.0)
        .expect("label mouseup should dispatch");

    assert!(outcome.handled);
    let file_chooser = outcome
        .pending_file_chooser
        .expect("label activation should propagate the file input chooser");
    assert_eq!(file_chooser.source_document(), source_document);
    assert_eq!(file_chooser.source_frame_id(), None);
    assert_eq!(file_chooser.backend_node_id(), 0);
    assert!(file_chooser.node_id.is_some());
    assert!(file_chooser.allow_multiple());
}

#[test]
fn dispatched_label_click_preserves_modifiers_for_control_activation() {
    let mut vm = new_parsed_test_vm(
        "https://label-dispatched-click.test/path/index.html",
        r#"<html><body>
            <label id="label" for="box">Toggle</label>
            <input id="box" type="checkbox">
        </body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const label = document.getElementById('label');
  const box = document.getElementById('box');
  const clicks = [];
  box.addEventListener('click', event => {
    clicks.push(`${event.ctrlKey}:${event.shiftKey}:${event.altKey}:${event.metaKey}`);
  });
  const allowed = label.dispatchEvent(new MouseEvent('click', {
    bubbles: true,
    cancelable: true,
    ctrlKey: true,
    shiftKey: true
  }));
  return `${allowed}|${box.checked}|${clicks.join('|')}`;
})()
"#,
        )
        .expect("dispatched label click should evaluate");

    assert_eq!(result, "true|true|true:true:false:false");
}

#[test]
fn area_link_descendants_do_not_activate_label_controls() {
    let mut vm = new_parsed_test_vm(
        "https://label-area-click.test/path/index.html",
        r#"<html><body></body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const results = [];
  for (const controlName of ['checkbox', 'button']) {
    const label = document.createElement('label');
    const control = document.createElement('input');
    control.type = controlName === 'checkbox' ? 'checkbox' : 'button';
    let controlClicks = 0;
    control.addEventListener('click', () => controlClicks++);
    const area = document.createElement('area');
    area.href = `#${controlName}`;
    label.append(control, area);
    document.body.append(label);
    area.click();
    results.push(`${controlName}:${control.checked}:${controlClicks}`);
  }
  return results.join('|');
})()
"#,
        )
        .expect("label area activation probe should evaluate");

    assert_eq!(result, "checkbox:false:0|button:false:0");
}

#[tokio::test]
async fn summary_activation_toggles_details_and_queues_toggle_event() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://summary-activation.test/",
        &loader,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const details = document.createElement('details');
  const summary = document.createElement('summary');
  details.append(summary);
  body.append(details);
  globalThis.__detailsToggleEvents = [];
  details.addEventListener('toggle', event => {
    __detailsToggleEvents.push(
      `${event.oldState}:${event.newState}:${event.source === null}`
    );
  });
  summary.click();
  return `${details.open}:${__detailsToggleEvents.length}`;
})()
"#,
        )
        .expect("summary activation setup should evaluate");
    assert_eq!(result, "true:0");

    assert!(!vm.has_ready_timeout());
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ElementToggle,
            &loader,
        )
        .await
        .expect("details toggle event task should advance")
    );

    let result = vm
        .eval("globalThis.__detailsToggleEvents.join('|')")
        .expect("details toggle event result should evaluate");
    assert_eq!(result, "closed:open:true");
}

#[test]
fn pointer_click_enters_shadow_root_for_label_activation() {
    const LABEL_HIT_X: f64 = 10.0;
    // Chromium's checkbox UA margin places the following label at y=31 for
    // this fixture. Hit inside the label rather than the intervening margin.
    const LABEL_HIT_Y: f64 = 35.0;

    let mut vm = new_parsed_test_vm(
        "https://shadow-label-click.test/path/index.html",
        r#"<html><body><div id="host" class="h-screen"></div></body></html>"#,
    );
    vm.eval(
        r#"
const root = document.getElementById('host').attachShadow({ mode: 'open' });
const input = document.createElement('input');
input.id = 'target';
input.type = 'checkbox';
input.style.cssText = 'display:block;width:20px;height:20px';
root.appendChild(input);
const label = document.createElement('label');
label.htmlFor = 'target';
label.textContent = 'Shadow label';
label.style.cssText = 'display:block;width:120px;height:30px';
root.appendChild(label);
"#,
    )
    .expect("shadow label setup should evaluate");

    vm.dispatch_mouse_event_at_point(LABEL_HIT_X, LABEL_HIT_Y, "mousedown", 0, None, 0.0, 0.0)
        .expect("shadow label mousedown should dispatch");
    let outcome = vm
        .dispatch_mouse_event_at_point(LABEL_HIT_X, LABEL_HIT_Y, "mouseup", 0, None, 0.0, 0.0)
        .expect("shadow label mouseup should dispatch");

    assert!(outcome.handled);
    let checked = vm
        .eval("document.getElementById('host').shadowRoot.getElementById('target').checked")
        .expect("shadow checkbox state should evaluate");
    assert_eq!(checked, "true");
}

#[test]
fn submit_button_click_queues_pending_top_level_location_navigation() {
    let mut vm = new_storage_test_vm("https://form-submit-navigation.test/path/index.html");

    vm.eval(
        r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const form = document.createElement('form');
  form.action = '/submit?from=form';
  const input = document.createElement('input');
  input.type = 'hidden';
  input.name = 'from';
  input.value = 'form';
  const button = document.createElement('button');
  button.type = 'submit';
  form.appendChild(input);
  form.appendChild(button);
  document.body.appendChild(form);
  button.click();
})()
"#,
    )
    .expect("submit button click should execute");

    let pending = vm
        .take_pending_location_navigation_with_seed()
        .expect("submit button click should queue a pending location navigation");
    assert_eq!(
        pending.url.as_str(),
        "https://form-submit-navigation.test/submit?from=form"
    );
    assert!(
        pending.entry_seed.is_none(),
        "default form submission should not synthesize a history seed"
    );
}
#[test]
fn empty_get_form_submit_replaces_existing_action_query() {
    let mut vm = new_storage_test_vm("https://form-empty-get.test/path/index.html");

    vm.eval(
        r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const form = document.createElement('form');
  form.action = '/submit?debug=true';
  form.method = 'get';
  document.body.appendChild(form);
  form.submit();
})()
"#,
    )
    .expect("empty GET form submit should execute");

    let pending = vm
        .take_pending_location_navigation_with_seed()
        .expect("empty GET form submit should queue a pending location navigation");
    assert_eq!(pending.url.as_str(), "https://form-empty-get.test/submit?");
}
#[test]
fn get_form_submit_dispatches_cancelable_navigate_event_with_source_element() {
    let mut vm = new_storage_test_vm("https://form-get-navigate.test/path/index.html");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const form = document.createElement('form');
  form.action = '';
  document.body.appendChild(form);
  let seen = [];
  navigation.onnavigate = event => {
    seen.push([
      event.navigationType,
      event.cancelable,
      event.canIntercept,
      event.hashChange,
      event.formData === null,
      event.sourceElement === form,
      event.destination.url
    ].join('|'));
    event.preventDefault();
  };
  form.submit();
  return seen.join(',');
})()
"#,
        )
        .expect("GET form submit navigate event probe should evaluate");

    assert_eq!(
        result,
        "replace|true|true|false|true|true|https://form-get-navigate.test/path/index.html?"
    );
    assert!(
        vm.take_pending_location_navigation_with_seed().is_none(),
        "prevented GET form navigate event should not queue a location navigation"
    );
}
#[test]
fn form_top_and_parent_targets_queue_plain_top_level_navigation() {
    for target in ["_top", "_parent", "_ToP", "_PaReNt", "_SeLf"] {
        let mut vm = new_storage_test_vm("https://form-target-navigation.test/path/index.html");

        vm.eval(&format!(
            r#"
(() => {{
  if (!document.documentElement) {{
    document.appendChild(document.createElement('html'));
  }}
  if (!document.body) {{
    document.documentElement.appendChild(document.createElement('body'));
  }}
  const form = document.createElement('form');
  form.action = '/submit';
  form.target = '{target}';
  const input = document.createElement('input');
  input.type = 'hidden';
  input.name = 'target';
  input.value = '{target}';
  const button = document.createElement('button');
  button.type = 'submit';
  form.appendChild(input);
  form.appendChild(button);
  document.body.appendChild(form);
  button.click();
}})()
"#
        ))
        .expect("submit button click should execute");

        let pending = vm
            .take_pending_location_navigation_with_seed()
            .expect("targeted form submit should queue a pending location navigation");
        assert_eq!(
            pending.url.as_str(),
            format!("https://form-target-navigation.test/submit?target={target}")
        );
        assert!(
            pending.entry_seed.is_none(),
            "{target} form submission should not synthesize a history seed"
        );
    }
}
#[test]
fn form_elements_collection_exposes_named_item() {
    let mut vm = new_storage_test_vm("https://form-elements-named-item.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.body || document.documentElement || document;
  const form = document.createElement('form');
  form.id = 'f';
  const input = document.createElement('input');
  input.id = 'by-id';
  input.name = 'token';
  input.value = 'ok';
  form.appendChild(input);
  host.appendChild(form);
  const probe = callback => {
    try {
      return String(callback());
    } catch (error) {
      return 'throw:' + error.name;
    }
  };
  const elements = form.elements;
  const byName = elements.namedItem({ toString() { return 'token'; } });
  const byId = elements.namedItem('by-id');
  return [
    Object.prototype.toString.call(elements),
    typeof elements.namedItem,
    byName && byName.value,
    byId && byId.getAttribute('name'),
    probe(() => elements.namedItem()),
    probe(() => elements.namedItem(Symbol('name')))
  ].join('|');
})()
"#,
        )
        .expect("form elements collection namedItem should be available");

    assert_eq!(
        result,
        "[object HTMLFormControlsCollection]|function|ok|token|throw:TypeError|throw:TypeError"
    );
}

#[test]
fn collection_webidl_members_live_on_interface_prototypes() {
    let mut vm = new_storage_test_vm("https://collection-prototype-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.body || document.documentElement || document;
  const form = document.createElement('form');
  for (const [value, checked] of [['first', true], ['second', false]]) {
    const radio = document.createElement('input');
    radio.type = 'radio';
    radio.name = 'choice';
    radio.value = value;
    radio.checked = checked;
    form.appendChild(radio);
  }
  const select = document.createElement('select');
  select.innerHTML = '<option>one</option><option>two</option>';
  form.appendChild(select);
  host.appendChild(form);

  const options = select.options;
  const radios = form.elements.namedItem('choice');
  const htmlCollection = document.getElementsByTagName('form');
  const nodeList = document.querySelectorAll('form');
  const accessor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      descriptor.get.name,
      descriptor.get.length,
      descriptor.set && descriptor.set.name,
      descriptor.set && descriptor.set.length,
      descriptor.enumerable,
      descriptor.configurable
    ].join(':');
  };
  const method = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      descriptor.value.name,
      descriptor.value.length,
      descriptor.enumerable,
      descriptor.writable,
      descriptor.configurable
    ].join(':');
  };
  const optionsLength =
    Object.getOwnPropertyDescriptor(HTMLOptionsCollection.prototype, 'length');
  const htmlLength =
    Object.getOwnPropertyDescriptor(HTMLCollection.prototype, 'length');
  const nodeLength =
    Object.getOwnPropertyDescriptor(NodeList.prototype, 'length');
  const selectedIndex =
    Object.getOwnPropertyDescriptor(HTMLOptionsCollection.prototype, 'selectedIndex');
  const radioValue =
    Object.getOwnPropertyDescriptor(RadioNodeList.prototype, 'value');
  const probe = callback => {
    try {
      callback();
      return 'ok';
    } catch (error) {
      return error && error.name;
    }
  };
  selectedIndex.set.call(options, 1);
  radioValue.set.call(radios, 'second');

  return JSON.stringify({
    own: {
      htmlLength: Object.hasOwn(htmlCollection, 'length'),
      nodeLength: Object.hasOwn(nodeList, 'length'),
      optionsLength: Object.hasOwn(options, 'length'),
      optionsSelectedIndex: Object.hasOwn(options, 'selectedIndex'),
      optionsAdd: Object.hasOwn(options, 'add'),
      radioValue: Object.hasOwn(radios, 'value')
    },
    chains: [
      Object.getPrototypeOf(HTMLOptionsCollection.prototype) === HTMLCollection.prototype,
      Object.getPrototypeOf(RadioNodeList.prototype) === NodeList.prototype
    ],
    accessors: {
      htmlLength: accessor(HTMLCollection.prototype, 'length'),
      nodeLength: accessor(NodeList.prototype, 'length'),
      optionsLength: accessor(HTMLOptionsCollection.prototype, 'length'),
      selectedIndex: accessor(HTMLOptionsCollection.prototype, 'selectedIndex'),
      radioValue: accessor(RadioNodeList.prototype, 'value')
    },
    methods: {
      htmlItem: method(HTMLCollection.prototype, 'item'),
      namedItem: method(HTMLCollection.prototype, 'namedItem'),
      nodeItem: method(NodeList.prototype, 'item'),
      add: method(HTMLOptionsCollection.prototype, 'add'),
      remove: method(HTMLOptionsCollection.prototype, 'remove')
    },
    borrowed: [
      optionsLength.get.call(options),
      selectedIndex.get.call(options),
      radioValue.get.call(radios),
      radios[0].checked,
      radios[1].checked
    ],
    forged: [
      probe(() =>
        Object.getOwnPropertyDescriptor(HTMLCollection.prototype, 'length').get.call({})
      ),
      probe(() => selectedIndex.set.call({}, 0)),
      probe(() => radioValue.get.call({}))
    ],
    crossBrand: [
      probe(() => htmlLength.get.call(nodeList)),
      probe(() => nodeLength.get.call(htmlCollection)),
      probe(() => optionsLength.get.call(htmlCollection)),
      probe(() => radioValue.get.call(options)),
      probe(() => HTMLCollection.prototype.item.call(nodeList, 0)),
      probe(() => NodeList.prototype.item.call(htmlCollection, 0)),
      probe(() => HTMLCollection.prototype.namedItem.call(nodeList, 'choice')),
      probe(() => HTMLOptionsCollection.prototype.remove.call(htmlCollection, 0))
    ],
    tags: [
      Object.prototype.toString.call(options),
      Object.prototype.toString.call(radios)
    ]
  });
})()
"#,
        )
        .expect("collection prototype surface probe should evaluate");

    assert_eq!(
        result,
        r#"{"own":{"htmlLength":false,"nodeLength":false,"optionsLength":false,"optionsSelectedIndex":false,"optionsAdd":false,"radioValue":false},"chains":[true,true],"accessors":{"htmlLength":"get length:0:::true:true","nodeLength":"get length:0:::true:true","optionsLength":"get length:0:set length:1:true:true","selectedIndex":"get selectedIndex:0:set selectedIndex:1:true:true","radioValue":"get value:0:set value:1:true:true"},"methods":{"htmlItem":"item:1:true:true:true","namedItem":"namedItem:1:true:true:true","nodeItem":"item:1:false:true:true","add":"add:1:true:true:true","remove":"remove:1:true:true:true"},"borrowed":[2,1,"second",false,true],"forged":["TypeError","TypeError","TypeError"],"crossBrand":["TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError"],"tags":["[object HTMLOptionsCollection]","[object RadioNodeList]"]}"#
    );
}

#[test]
fn radio_node_list_value_uses_radio_state_and_preserves_it_on_a_miss() {
    let mut vm = new_storage_test_vm("https://radio-node-list-value.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.body || document.documentElement || document;
  const form = document.createElement('form');
  form.innerHTML = `
    <input type="checkbox" name="choice" value="checkbox" checked>
    <input id="first" type="radio" name="choice">
    <input id="second" type="radio" name="choice" value="second">
  `;
  host.appendChild(form);
  const list = form.elements.namedItem('choice');
  const first = document.getElementById('first');
  const second = document.getElementById('second');
  const values = [
    Object.prototype.toString.call(list),
    list.value
  ];
  second.checked = true;
  values.push(list.value);
  first.checked = true;
  values.push(list.value, String(second.checked));
  first.value = 'first';
  values.push(list.value, first.getAttribute('value'));
  list.value = 'second';
  values.push(list.value, String(first.checked), String(second.checked));
  list.value = 'missing';
  values.push(list.value, String(second.checked));
  first.removeAttribute('value');
  list.value = 'on';
  values.push(list.value, String(first.checked), String(second.checked));
  return values.join('|');
})()
"#,
        )
        .expect("RadioNodeList value semantics should evaluate");

    assert_eq!(
        result,
        "[object RadioNodeList]||second|on|false|first|first|second|false|true|second|true|on|true|false"
    );
}

#[test]
fn disconnected_radio_groups_follow_tree_roots_and_form_owners() {
    let mut vm = new_storage_test_vm("https://disconnected-radio-groups.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const makeRadio = (name, { checked = false, required = false } = {}) => {
    const radio = document.createElement('input');
    radio.type = 'radio';
    radio.name = name;
    radio.checked = checked;
    radio.required = required;
    return radio;
  };

  const container = document.createElement('div');
  const required = makeRadio('detached', { required: true });
  const checked = makeRadio('detached', { checked: true });
  container.append(required, checked);
  const detachedValid = !required.validity.valueMissing;
  required.checked = true;
  const detachedSelection = `${required.checked}:${checked.checked}`;

  const rootRadio = makeRadio('root', { required: true });
  const childRadio = makeRadio('root', { checked: true });
  rootRadio.appendChild(childRadio);
  const rootValid = !rootRadio.validity.valueMissing;
  rootRadio.checked = true;
  const rootSelection = `${rootRadio.checked}:${childRadio.checked}`;

  const fragment = document.createDocumentFragment();
  const fragmentRequired = makeRadio('fragment', { required: true });
  const fragmentChecked = makeRadio('fragment', { checked: true });
  fragment.append(fragmentRequired, fragmentChecked);
  const fragmentValid = !fragmentRequired.validity.valueMissing;

  const unnamed = makeRadio('', { required: true });
  const unnamedValid = !unnamed.validity.valueMissing;

  const loose = document.createElement('div');
  const looseFirst = makeRadio('loose', { checked: true });
  const looseSecond = makeRadio('loose', { checked: true });
  loose.append(looseFirst, looseSecond);

  const form = document.createElement('form');
  const formFirst = makeRadio('owned', { checked: true });
  const formSecond = makeRadio('owned', { checked: true });
  form.append(formFirst, formSecond);

  return [
    detachedValid,
    detachedSelection,
    rootValid,
    rootSelection,
    fragmentValid,
    unnamedValid,
    `${looseFirst.checked}:${looseSecond.checked}`,
    `${formFirst.checked}:${formSecond.checked}`
  ].join('|');
})()
"#,
        )
        .expect("disconnected radio group probe should evaluate");

    assert_eq!(
        result,
        "true|true:false|true|true:false|true|true|true:true|false:true"
    );
}
